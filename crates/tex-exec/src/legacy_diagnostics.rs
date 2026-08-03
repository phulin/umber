//! Retired `Executor` scanners for diagnostic and token-list primitives.

use tex_expand::{get_x_token_with_context, scan_the_text_with_context, token_text};
use tex_lex::InputStack;
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::token_show::meaning_text;
use tex_state::{PrintSink, Universe};

use crate::{ExecError, ExecutionContext, push_tokens, push_traced_tokens};

pub(crate) fn report_integer_diagnostic(
    stores: &mut Universe,
    diagnostic: tex_expand::scan_int::IntegerDiagnostic,
) {
    write_diagnostic(stores, &format!("\n! {diagnostic}.\n"));
}

pub(crate) fn report_undefined_control_sequence_in_input(
    input: &InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let context = crate::diagnostics::show_context(stores, &input.summary());
    crate::diagnostics::report_undefined_control_sequence(stores, Some(context))?;
    Ok(())
}

pub(crate) fn execute_show(input: &mut InputStack, stores: &mut Universe) -> Result<(), ExecError> {
    let token = crate::raw_delivery::next_semantic_raw_token(input, stores)?
        .ok_or(ExecError::MissingToken { context: "\\show" })?
        .semantic_token();
    let text = match token {
        Token::Cs(_)
        | Token::Char {
            cat: Catcode::Active,
            ..
        } => format!(
            "\n> {}={}.\n",
            token_text(stores, token),
            show_meaning_text(stores, token)
        ),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {
            format!("\n> {}.\n", meaning_text(stores, token))
        }
    };
    write_diagnostic(stores, &text);
    Ok(())
}

pub(crate) fn execute_showthe(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let text = match scan_the_text_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    ) {
        Ok(text) => text,
        Err(tex_expand::ExpandError::UnsupportedTheTarget { context }) => {
            let token = context.semantic_token();
            let rendered = match token {
                Token::Char { ch, cat } => format!("{} character {ch}", catcode_name(cat)),
                _ => meaning_text(stores, token),
            };
            crate::error_report::report_input_error(
                input,
                stores,
                &format!("You can't use `{rendered}' after \\the"),
                &["I'm forgetting what you said and using zero instead."],
            )?;
            "0".to_owned()
        }
        Err(error) => return Err(error.into()),
    };
    write_diagnostic(stores, &format!("\n> {text}.\n"));
    Ok(())
}

pub(crate) fn execute_showtokens(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let tokens = tex_expand::scan::scan_general_text_with_expanded_open_with_driver(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    )?;
    write_diagnostic(
        stores,
        &format!(
            "\n> {}.\n",
            tokens_text(stores, stores.tokens(tokens.token_list()))
        ),
    );
    Ok(())
}

pub(crate) fn execute_showifs(input: &InputStack, stores: &mut Universe) {
    let conditions = input.conditions().collect::<Vec<_>>();
    let mut text = String::new();
    text.push('\n');
    for (index, condition) in conditions.iter().enumerate().rev() {
        text.push_str("### level ");
        text.push_str(&(index + 1).to_string());
        text.push_str(": ");
        if condition.inverted() {
            text.push_str("\\unless");
        }
        text.push_str(if_type_text(condition.if_type()));
        text.push('\n');
    }
    text.push_str("\n! OK.\n");
    write_diagnostic(stores, &text);
}

pub(crate) fn execute_message(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    error: bool,
) -> Result<(), ExecError> {
    let tokens = scan_balanced_expanded_text(context, input, stores, execution)?;
    let text = crate::diagnostics::print_text_with_newlinechar(
        stores,
        &message_tokens_text(stores, &tokens),
    );
    if error {
        write_diagnostic(stores, &format!("\n! {text}.\n"));
    } else {
        let column = diagnostic_print_column(stores);
        let max_print_line = stores.printer().max_print_line();
        let mut output = String::new();
        if column + text.chars().count() > max_print_line - 2 {
            output.push('\n');
        } else if column > 0 {
            output.push(' ');
        }
        output.push_str(&text);
        write_diagnostic(stores, &output);
    }
    Ok(())
}

pub(crate) fn execute_change_case(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    uppercase: bool,
) -> Result<(), ExecError> {
    let mut tokens = scan_balanced_raw_text(
        input,
        stores,
        execution,
        if uppercase {
            "\\uppercase"
        } else {
            "\\lowercase"
        },
    )?;
    for token in &mut tokens {
        let Token::Char { ch, .. } = token else {
            continue;
        };
        let mapped = if uppercase {
            stores.uccode(*ch)
        } else {
            stores.lccode(*ch)
        };
        if let Some(mapped) = char::from_u32(mapped).filter(|&mapped| mapped != '\0') {
            *ch = mapped;
        }
    }
    push_tokens(input, stores, tokens);
    Ok(())
}

pub(crate) fn execute_ignorespaces(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Result<(), ExecError> {
    loop {
        let Some(token) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Ok(());
        };
        if !is_space(token.semantic_token()) {
            push_traced_tokens(input, stores, [token]);
            return Ok(());
        }
    }
}

fn scan_balanced_raw_text(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    context: &'static str,
) -> Result<Vec<Token>, ExecError> {
    let open =
        next_non_space_x(input, stores, execution)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(open) {
        return Err(ExecError::MissingToken { context });
    }
    let mut depth = 1usize;
    let mut tokens = Vec::new();
    while let Some(traced) = crate::raw_delivery::next_semantic_raw_token(input, stores)? {
        let token = traced.semantic_token();
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
    Err(ExecError::MissingToken { context })
}

fn scan_balanced_expanded_text(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Result<Vec<Token>, ExecError> {
    let token_list = tex_expand::scan::scan_general_text_expanded_with_driver(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    )?;
    Ok(stores.tokens(token_list).to_vec())
}

fn next_non_space_x(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Result<Option<Token>, ExecError> {
    while let Some(token) = get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )?
    .map(|token| token.semantic_token())
    {
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
    Ok(None)
}

fn show_meaning_text(stores: &Universe, token: Token) -> String {
    let text = meaning_text(stores, token);
    if let Some((prefix, rest)) = text.split_once("macro:") {
        format!("{prefix}macro:\n{rest}")
    } else {
        text
    }
}

fn tokens_text(stores: &Universe, tokens: &[Token]) -> String {
    let mut text = String::new();
    for &token in tokens {
        crate::diagnostics::append_token_show_text(stores, token, &mut text);
    }
    text
}

fn message_tokens_text(stores: &Universe, tokens: &[Token]) -> String {
    let mut text = String::new();
    for &token in tokens {
        if let Token::Char { ch, .. } = token {
            text.push(ch);
        } else {
            tex_state::token_show::append_token_string_text(stores, token, &mut text);
        }
    }
    text
}

fn diagnostic_print_column(stores: &Universe) -> usize {
    stores
        .world()
        .stream_bufs()
        .terminal_partial_line()
        .chars()
        .count()
}

fn write_diagnostic(stores: &mut Universe, text: &str) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, text);
}

fn catcode_name(cat: Catcode) -> &'static str {
    match cat {
        Catcode::MathShift => "math shift",
        Catcode::BeginGroup => "begin-group",
        Catcode::EndGroup => "end-group",
        Catcode::AlignmentTab => "alignment tab",
        Catcode::Parameter => "macro parameter",
        Catcode::Superscript => "superscript",
        Catcode::Subscript => "subscript",
        Catcode::Space => "blank space",
        Catcode::Letter => "the letter",
        Catcode::Other => "the character",
        Catcode::Active => "active character",
        Catcode::Escape => "escape",
        Catcode::EndLine => "end of line",
        Catcode::Ignored => "ignored",
        Catcode::Comment => "comment",
        Catcode::Invalid => "invalid character",
    }
}

fn if_type_text(if_type: u8) -> &'static str {
    match if_type {
        1 => "\\if",
        2 => "\\ifcat",
        3 => "\\ifnum",
        4 => "\\ifdim",
        5 => "\\ifodd",
        6 => "\\ifvmode",
        7 => "\\ifhmode",
        8 => "\\ifmmode",
        9 => "\\ifinner",
        10 => "\\ifvoid",
        11 => "\\ifhbox",
        12 => "\\ifvbox",
        13 => "\\ifx",
        14 => "\\ifeof",
        15 => "\\iftrue",
        16 => "\\iffalse",
        17 => "\\ifcase",
        18 => "\\ifdefined",
        19 => "\\ifcsname",
        20 => "\\iffontchar",
        _ => "\\if",
    }
}

fn is_begin_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    )
}

fn is_end_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    )
}

fn is_space(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    )
}
