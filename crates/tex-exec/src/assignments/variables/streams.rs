use super::*;
use tex_expand::{NoopExpansionHooks, ReadRecorder, token_text};
use tex_lex::{MemoryInput, TokenListReplayKind};
use tex_state::ids::TokenListId;
use tex_state::macro_store::MacroMeaning;
use tex_state::node::{Node, Whatsit};
use tex_state::{PrintSink, StreamSlot};

use crate::diagnostics::print_text_with_newlinechar;
use crate::vertical::append_node_to_current_list;

pub(in crate::assignments) fn execute_stream_command<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let slot = scan_stream_slot(input, stores, hooks, context)?;
    match primitive {
        UnexpandablePrimitive::OpenIn => {
            skip_optional_equals_x(input, stores, hooks)?;
            let name = scan_file_name(input, stores, hooks, "\\openin")?;
            if stores.world_mut().open_in(slot, name).is_err() {
                stores.world_mut().close_in(slot);
            }
        }
        UnexpandablePrimitive::CloseIn => stores.world_mut().close_in(slot),
        UnexpandablePrimitive::OpenOut => {
            skip_optional_equals_x(input, stores, hooks)?;
            let name = scan_file_name(input, stores, hooks, "\\openout")?;
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

pub(in crate::assignments) fn execute_immediate_stream_command<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let slot = scan_stream_slot(input, stores, hooks, context)?;
    match primitive {
        UnexpandablePrimitive::OpenOut => {
            skip_optional_equals_x(input, stores, hooks)?;
            let name = scan_file_name(input, stores, hooks, "\\openout")?;
            stores.world_mut().open_out(slot, name);
        }
        UnexpandablePrimitive::CloseOut => stores.world_mut().close_out(slot),
        _ => unreachable!("caller restricts immediate stream primitive"),
    }
    Ok(())
}

pub(in crate::assignments) fn execute_read<S, H>(
    context: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let slot = scan_stream_slot(input, stores, hooks, context)?;
    if !scan_optional_keyword_x(input, stores, hooks, "to")? {
        return Err(ExecError::ReadNeedsTo);
    }
    let target = scan_definition_target(input, stores, "\\read")?;
    let tokens = scan_read_tokens(slot, target, stores)?;
    let replacement_text = stores.intern_token_list(&tokens);
    let parameter_text = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        target,
        MacroMeaning::new(MeaningFlags::EMPTY, parameter_text, replacement_text),
    );
    Ok(())
}

pub(in crate::assignments) fn execute_write<S, H>(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let sink = scan_write_sink(input, stores, hooks, context)?;
    let scanned = scan_toks(input, stores, MeaningFlags::EMPTY, context)?;
    append_node_to_current_list(
        nest,
        stores,
        Node::Whatsit(Whatsit::DeferredWrite {
            sink,
            tokens: scanned.meaning().replacement_text(),
        }),
    )
}

pub(in crate::assignments) fn execute_immediate_write<S, R, H>(
    context: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let sink = scan_write_sink(input, stores, hooks, context)?;
    let scanned = scan_toks(input, stores, MeaningFlags::EMPTY, context)?;
    let text = expand_write_tokens(stores, recorder, scanned.meaning().replacement_text())?;
    stores.world_mut().write_text(sink, &text);
    Ok(())
}

pub(in crate::assignments) fn execute_special<S, H>(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let tokens = scan_balanced_expanded_text(input, stores, hooks, context)?;
    let payload = tokens_text(stores, &tokens).into_bytes();
    append_node_to_current_list(
        nest,
        stores,
        Node::Whatsit(Whatsit::Special {
            class: "dvi".to_owned(),
            payload,
        }),
    )
}

fn expand_write_tokens<R>(
    stores: &mut Universe,
    recorder: &mut R,
    tokens: TokenListId,
) -> Result<String, ExecError>
where
    R: ReadRecorder,
{
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let mut hooks = NoopExpansionHooks;
    let mut text = String::new();
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(&mut input, stores, recorder, &mut hooks)?
            .map(tex_expand::semantic_token)
    {
        text.push_str(&token_text(stores, token));
    }
    Ok(print_text_with_newlinechar(stores, &text))
}

fn scan_read_tokens(
    slot: StreamSlot,
    target: Symbol,
    stores: &mut Universe,
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
    format!("\n{}=", tex_expand::token_text(stores, Token::Cs(target)))
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

fn scan_stream_slot<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<StreamSlot, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let value = scan_i32(input, stores, hooks, context)?;
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

fn scan_write_sink<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<PrintSink, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let value = scan_i32(input, stores, hooks, context)?;
    Ok(match value {
        0..=15 => PrintSink::Stream(StreamSlot::new(value as u8)),
        value if value < 0 => PrintSink::Log,
        _ => PrintSink::TerminalAndLog,
    })
}

fn scan_file_name<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: &'static str,
) -> Result<String, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut name = String::new();
    let Some(first) = next_non_space_x(input, stores, hooks)? else {
        return Err(ExecError::MissingToken { context });
    };
    append_file_name_token(&mut name, first, context)?;
    let mut recorder = NoopRecorder;
    while let Some(traced) =
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
    {
        match tex_expand::semantic_token(traced) {
            Token::Char {
                cat: Catcode::Space,
                ..
            } => break,
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

fn scan_balanced_expanded_text<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<Vec<Token>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let open =
        next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingTracedToken { context })?;
    if !is_begin_group(open) {
        return Err(ExecError::MissingTracedToken { context });
    }
    let mut recorder = NoopRecorder;
    let mut depth = 1usize;
    let mut tokens = Vec::new();
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
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
