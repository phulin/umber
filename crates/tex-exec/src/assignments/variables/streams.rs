use super::*;
use std::path::PathBuf;

use tex_expand::token_text;
use tex_lex::{MemoryInput, TokenListReplayKind};
use tex_state::env::banks::IntParam;
use tex_state::ids::TokenListId;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::node::PdfLiteralMode;
use tex_state::node::{Node, Whatsit};
use tex_state::{InputOpenState, PrintSink, StreamSlot};

use crate::diagnostics::print_text_with_newlinechar;
use crate::vertical::append_node_to_current_list;

pub(in crate::assignments) fn execute_stream_command(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let slot = scan_stream_slot(input, stores, execution, context)?;
    match primitive {
        UnexpandablePrimitive::OpenIn => {
            skip_optional_equals_x(input, stores, execution)?;
            let name = scan_file_name(input, stores, execution, "\\openin")?;
            let resolved = execution
                .open_stream_input(&mut stores.input_open_context(), &name)
                .ok()
                .flatten();
            match resolved {
                Some(content) if stores.world_mut().open_in_content(slot, &content).is_ok() => {}
                _ => stores.world_mut().close_in(slot),
            }
        }
        UnexpandablePrimitive::CloseIn => stores.world_mut().close_in(slot),
        UnexpandablePrimitive::OpenOut => {
            skip_optional_equals_x(input, stores, execution)?;
            let name = scan_file_name(input, stores, execution, "\\openout")?;
            append_node_to_current_list(
                nest,
                stores,
                Node::Whatsit(Whatsit::OpenOut { slot, path: name }),
            )?;
        }
        UnexpandablePrimitive::CloseOut => {
            append_node_to_current_list(nest, stores, Node::Whatsit(Whatsit::CloseOut { slot }))?;
        }
        _ => unreachable!("caller restricts stream primitive"),
    }
    Ok(())
}

pub(in crate::assignments) fn execute_immediate_stream_command(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let slot = scan_stream_slot(input, stores, execution, context)?;
    match primitive {
        UnexpandablePrimitive::OpenOut => {
            skip_optional_equals_x(input, stores, execution)?;
            let name = scan_file_name(input, stores, execution, "\\openout")?;
            stores.world_mut().open_out(slot, openout_target(name));
        }
        UnexpandablePrimitive::CloseOut => stores.world_mut().close_out(slot),
        _ => unreachable!("caller restricts immediate stream primitive"),
    }
    Ok(())
}

pub(in crate::assignments) fn openout_target(name: String) -> String {
    let mut path = PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension("tex");
    }
    path.to_string_lossy().into_owned()
}

pub(in crate::assignments) fn execute_read(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let slot = scan_stream_slot(input, stores, execution, context)?;
    if !scan_optional_keyword_x(input, stores, execution, "to")? {
        return Err(ExecError::ReadNeedsTo);
    }
    let primitive_name = if primitive == UnexpandablePrimitive::ReadLine {
        "\\readline"
    } else {
        "\\read"
    };
    let target = scan_definition_target(input, stores, primitive_name)?;
    let tokens = scan_read_tokens(
        slot,
        target,
        stores,
        primitive == UnexpandablePrimitive::ReadLine,
    )?;
    let replacement_text = stores.intern_token_list(&tokens);
    let parameter_text = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        target,
        MacroMeaning::new(MeaningFlags::EMPTY, parameter_text, replacement_text),
    );
    Ok(())
}

pub(in crate::assignments) fn execute_write(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let sink = scan_write_sink(input, stores, execution, context)?;
    let scanned = scan_toks(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        MeaningFlags::EMPTY,
        context,
    )?;
    append_node_to_current_list(
        nest,
        stores,
        Node::Whatsit(Whatsit::DeferredWrite {
            sink,
            tokens: scanned.meaning().replacement_text(),
        }),
    )
}

pub(in crate::assignments) fn execute_immediate_write(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let sink = scan_write_sink(input, stores, execution, context)?;
    let scanned = scan_toks(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        MeaningFlags::EMPTY,
        context,
    )?;
    let text = execution.with_nested(|expansion| {
        expand_write_tokens(stores, expansion, scanned.meaning().replacement_text())
    })?;
    stores.world_mut().write_text(sink, &text);
    Ok(())
}

pub(in crate::assignments) fn execute_special(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let tokens = scan_balanced_expanded_text(input, stores, execution, context)?;
    let payload = tex_byte_text(&tokens_text(stores, &tokens));
    append_node_to_current_list(
        nest,
        stores,
        Node::Whatsit(Whatsit::Special {
            class: "dvi".to_owned(),
            payload,
        }),
    )
}

pub(in crate::assignments) fn execute_pdf_graphics(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
        return Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: tex_expand::semantic_token(context),
            origin: context.origin(),
            operation: "PDF graphics primitive while PDF output is disabled",
        });
    }
    let node = match primitive {
        UnexpandablePrimitive::PdfLiteral => {
            let deferred = scan_optional_keyword_x(input, stores, execution, "shipout")?;
            let mode = if scan_optional_keyword_x(input, stores, execution, "direct")? {
                PdfLiteralMode::Direct
            } else if scan_optional_keyword_x(input, stores, execution, "page")? {
                PdfLiteralMode::Page
            } else {
                PdfLiteralMode::Origin
            };
            if deferred {
                let scanned = scan_toks(
                    input,
                    &mut tex_state::ExpansionContext::new(stores),
                    MeaningFlags::EMPTY,
                    context,
                )?;
                Node::Whatsit(Whatsit::DeferredPdfLiteral {
                    mode,
                    tokens: scanned.meaning().replacement_text(),
                })
            } else {
                let tokens = scan_balanced_expanded_text(input, stores, execution, context)?;
                Node::Whatsit(Whatsit::PdfLiteral {
                    mode,
                    payload: tex_byte_text(&tokens_text(stores, &tokens)),
                })
            }
        }
        UnexpandablePrimitive::PdfSetMatrix => {
            let tokens = scan_balanced_expanded_text(input, stores, execution, context)?;
            Node::Whatsit(Whatsit::PdfSetMatrix {
                payload: tex_byte_text(&tokens_text(stores, &tokens)),
            })
        }
        UnexpandablePrimitive::PdfSave => Node::Whatsit(Whatsit::PdfSave),
        UnexpandablePrimitive::PdfRestore => Node::Whatsit(Whatsit::PdfRestore),
        UnexpandablePrimitive::PdfColorStack => {
            let scanned_id = scan_i32(input, stores, execution, context)?;
            let id = if scanned_id < 0 {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    "Invalid negative color stack number\n",
                );
                0
            } else if !stores.has_pdf_color_stack(scanned_id as u32) {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    &format!("Unknown color stack number {scanned_id}\n"),
                );
                0
            } else {
                scanned_id as u32
            };
            let action = if scan_optional_keyword_x(input, stores, execution, "set")? {
                let tokens = scan_balanced_expanded_text(input, stores, execution, context)?;
                tex_state::PdfColorStackAction::Set(tex_byte_text(&tokens_text(stores, &tokens)))
            } else if scan_optional_keyword_x(input, stores, execution, "push")? {
                let tokens = scan_balanced_expanded_text(input, stores, execution, context)?;
                tex_state::PdfColorStackAction::Push(tex_byte_text(&tokens_text(stores, &tokens)))
            } else if scan_optional_keyword_x(input, stores, execution, "pop")? {
                tex_state::PdfColorStackAction::Pop
            } else if scan_optional_keyword_x(input, stores, execution, "current")? {
                tex_state::PdfColorStackAction::Current
            } else {
                stores
                    .world_mut()
                    .write_text(PrintSink::TerminalAndLog, "Color stack action is missing\n");
                return Ok(());
            };
            Node::Whatsit(Whatsit::PdfColorStack { id, action })
        }
        _ => unreachable!("caller restricts PDF graphics primitive"),
    };
    append_node_to_current_list(nest, stores, node)
}

fn tex_byte_text(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if let Ok(byte) = u8::try_from(ch as u32) {
            bytes.push(byte);
        } else {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    bytes
}

fn expand_write_tokens(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    tokens: TokenListId,
) -> Result<String, ExecError> {
    let mut input = InputStack::empty();
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let mut text = String::new();
    while let Some(token) = next_write_expansion_token(&mut input, stores, expansion)? {
        crate::diagnostics::append_token_show_text(stores, token, &mut text);
    }
    let mut text = print_text_with_newlinechar(stores, &text);
    text.push('\n');
    Ok(text)
}

fn next_write_expansion_token(
    input: &mut InputStack,
    stores: &mut Universe,
    context: &mut tex_expand::ExpansionContext<'_>,
) -> Result<Option<Token>, ExecError> {
    let Some(read) = input.next_traced_expansion_token(stores)? else {
        return Ok(None);
    };
    let traced = read.traced_token();
    let token = read.token();
    if read.suppress_expansion() {
        return Ok(Some(token));
    }
    let symbol = match token {
        Token::Cs(symbol) => Some(symbol),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => tex_state::ExpansionState::active_character_symbol(stores, ch),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => None,
    };
    if let Some(symbol) = symbol {
        let meaning = stores.meaning(symbol);
        context.record_meaning(symbol, meaning);
        if matches!(meaning, Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED))
        {
            return Ok(Some(token));
        }
    }
    tex_expand::back_input(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        [traced],
    );
    Ok(get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        context,
    )?
    .map(tex_expand::semantic_token))
}

fn scan_read_tokens(
    slot: StreamSlot,
    target: Symbol,
    stores: &mut Universe,
    raw_catcodes: bool,
) -> Result<Vec<Token>, ExecError> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut terminal_prompt = Some(read_prompt(stores, target));
    loop {
        let had_open_stream = stores
            .world()
            .stream_bufs()
            .read_stream_target(slot)
            .is_some();
        let line = if had_open_stream {
            stores.world_mut().read_stream_line(slot)?
        } else {
            read_terminal_read_line(stores, terminal_prompt.take())?
        };
        let Some(line) = line else {
            return Err(ExecError::ReadNotImplemented);
        };
        if raw_catcodes {
            tokens.extend(tokenize_readline(&line, stores));
            return Ok(tokens);
        }
        if scan_read_line_tokens(&line, stores, &mut tokens, &mut depth)? {
            return Ok(tokens);
        }
        if depth == 0 {
            return Ok(tokens);
        }
        if had_open_stream
            && stores
                .world()
                .stream_bufs()
                .read_stream_target(slot)
                .is_none()
        {
            return Err(ExecError::FileEndedWithinRead);
        }
    }
}

fn tokenize_readline(line: &str, stores: &Universe) -> Vec<Token> {
    let mut tokens = line
        .chars()
        .map(|ch| Token::Char {
            ch,
            cat: if ch == ' ' {
                Catcode::Space
            } else {
                Catcode::Other
            },
        })
        .collect::<Vec<_>>();
    if let Ok(endline) = u32::try_from(stores.int_param(IntParam::END_LINE_CHAR))
        && let Some(ch) = char::from_u32(endline)
    {
        tokens.push(Token::Char {
            ch,
            cat: if ch == ' ' {
                Catcode::Space
            } else {
                Catcode::Other
            },
        });
    }
    tokens
}

fn read_terminal_read_line(
    stores: &mut Universe,
    prompt: Option<String>,
) -> Result<Option<String>, ExecError> {
    match stores.interaction_mode() {
        InteractionMode::Batch | InteractionMode::Nonstop => {
            return Err(ExecError::ReadNotImplemented);
        }
        InteractionMode::Scroll | InteractionMode::ErrorStop => {}
    }
    stores.world_mut().write_text(
        tex_state::PrintSink::TerminalAndLog,
        prompt.as_deref().unwrap_or(""),
    );
    stores
        .world_mut()
        .read_terminal_line()?
        .map_or(Err(ExecError::TerminalReadEof), |line| Ok(Some(line)))
}

fn read_prompt(stores: &Universe, target: Symbol) -> String {
    format!(
        "\n{}=",
        tex_expand::token_text(stores, Token::Cs(target.symbol()))
    )
}

fn scan_read_line_tokens(
    line: &str,
    stores: &mut Universe,
    tokens: &mut Vec<Token>,
    depth: &mut usize,
) -> Result<bool, ExecError> {
    for token in tokenize_read_line(line, stores)? {
        let meaning = match token {
            Token::Cs(symbol) => stores.meaning(symbol),
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = active_character_symbol(stores, ch);
                stores.meaning(symbol)
            }
            _ => Meaning::Undefined,
        };
        if matches!(
            meaning,
            Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::OUTER)
        ) {
            // TeX.web §336/§484 aborts a \read at an outer token. The
            // outer token is not replayed for file streams; inserted closing
            // braces finish the partial balanced definition.
            while *depth > 0 {
                tokens.push(Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                });
                *depth -= 1;
            }
            return Ok(true);
        }
        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                *depth += 1;
                tokens.push(token);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                if *depth == 0 {
                    return Ok(true);
                }
                *depth -= 1;
                tokens.push(token);
            }
            _ => tokens.push(token),
        }
    }
    Ok(false)
}

fn scan_stream_slot(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<StreamSlot, ExecError> {
    let value = scan_i32(input, stores, execution, context)?;
    let value = if (0..tex_state::world::STREAM_SLOT_COUNT as i32).contains(&value) {
        value
    } else {
        // TeX.web `scan_four_bit_int` section 435 substitutes stream zero
        // after reporting an out-of-range open/close stream number.
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            &format!(
                "\n! Bad number ({value}).\nSince I expected to read a number between 0 and 15,\nI changed this one to zero.\n"
            ),
        );
        0
    };
    Ok(StreamSlot::new(value as u8))
}

fn scan_write_sink(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<PrintSink, ExecError> {
    let value = scan_i32(input, stores, execution, context)?;
    Ok(match value {
        0..=15 => PrintSink::Stream(StreamSlot::new(value as u8)),
        value if value < 0 => PrintSink::Log,
        _ => PrintSink::TerminalAndLog,
    })
}

fn scan_file_name(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: &'static str,
) -> Result<String, ExecError> {
    let mut name = String::new();
    let Some(first) = next_non_space_x(input, stores, execution)? else {
        return Err(ExecError::MissingToken { context });
    };

    if is_begin_group(first) {
        let mut name = String::new();
        let mut quoted = false;
        loop {
            let Some(traced) = get_x_token_with_context(
                input,
                &mut tex_state::ExpansionContext::new(stores),
                execution,
            )?
            else {
                return Err(ExecError::MissingToken { context });
            };
            let token = tex_expand::semantic_token(traced);
            if is_end_group(token) && !quoted {
                return if name.is_empty() {
                    Err(ExecError::MissingToken { context })
                } else {
                    Ok(name)
                };
            }
            if matches!(token, Token::Char { ch: '"', .. }) {
                quoted = !quoted;
                continue;
            }
            append_file_name_token(&mut name, token, context)?;
        }
    }

    let quoted = matches!(first, Token::Char { ch: '"', .. });
    if !quoted {
        append_file_name_token(&mut name, first, context)?;
    }
    while let Some(traced) = get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )? {
        match tex_expand::semantic_token(traced) {
            Token::Char { ch: '"', .. } if quoted => break,
            Token::Char {
                cat: Catcode::Space,
                ..
            } if !quoted => break,
            token @ Token::Char { .. } => append_file_name_token(&mut name, token, context)?,
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
                push_traced_tokens(input, stores, [traced]);
                break;
            }
        }
    }
    Ok(name)
}

fn append_file_name_token(
    name: &mut String,
    token: Token,
    context: &'static str,
) -> Result<(), ExecError> {
    match token {
        Token::Char { ch, .. } => {
            name.push(ch);
            Ok(())
        }
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
            Err(ExecError::MissingToken { context })
        }
    }
}

fn tokenize_read_line(line: &str, stores: &mut Universe) -> Result<Vec<Token>, ExecError> {
    let mut input = InputStack::new(tex_lex::MemoryInput::new(format!("{line}\n")));
    let mut tokens = Vec::new();
    loop {
        match input.next_token(stores) {
            Ok(Some(token)) => tokens.push(token),
            Ok(None) => break,
            Err(tex_lex::LexError::InvalidCharacter { .. }) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(tokens)
}

fn scan_balanced_expanded_text(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<Vec<Token>, ExecError> {
    let open = next_non_space_x(input, stores, execution)?
        .ok_or(ExecError::MissingTracedToken { context })?;
    if !is_begin_group(open) {
        return Err(ExecError::MissingTracedToken { context });
    }
    let mut depth = 1usize;
    let mut tokens = Vec::new();
    while let Some(token) = get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )?
    .map(tex_expand::semantic_token)
    {
        if is_begin_group(token) {
            depth += 1;
            tokens.push(token);
        } else if is_end_group(token) {
            depth -= 1;
            if depth == 0 {
                return Ok(tokens);
            }
            tokens.push(token);
        } else {
            tokens.push(token);
        }
    }
    Err(ExecError::MissingTracedToken { context })
}

fn tokens_text(stores: &Universe, tokens: &[Token]) -> String {
    let mut text = String::new();
    for &token in tokens {
        text.push_str(&token_text(stores, token));
        if let Token::Cs(symbol) = token {
            let name = stores.resolve(symbol);
            if name.chars().all(|ch| ch.is_ascii_alphabetic()) {
                text.push(' ');
            }
        }
    }
    text
}
