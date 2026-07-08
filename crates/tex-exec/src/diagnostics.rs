//! Diagnostic and log-writing primitives.

use tex_expand::{
    ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks, meaning_text,
    scan_the_text_with_hooks, token_text,
};
use tex_lex::{InputSource, InputStack};
use tex_state::env::banks::IntParam;
use tex_state::token::{Catcode, Token};
use tex_state::{PrintSink, Universe};

use crate::node_dump::{DumpConfig, dump_node_list};
use crate::{ExecError, push_tokens};

pub(crate) fn execute_show<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let token = input
        .next_token(stores)?
        .ok_or(ExecError::MissingToken { context: "\\show" })?;
    let text = match token {
        Token::Cs(_) => {
            format!(
                "\n> {}={}.\n",
                token_text(stores, token),
                show_meaning_text(stores, token)
            )
        }
        Token::Char { .. } | Token::Param(_) => {
            format!("\n> {}.\n", meaning_text(stores, token))
        }
    };
    write_diagnostic(stores, &text);
    Ok(())
}

pub(crate) fn execute_showthe<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let text = scan_the_text_with_hooks(input, stores, &mut recorder, hooks)?;
    write_diagnostic(stores, &format!("\n> {text}.\n"));
    Ok(())
}

pub(crate) fn execute_showtokens<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let tokens = scan_balanced_raw_text(input, stores, "\\showtokens")?;
    write_diagnostic(stores, &format!("\n> {}.\n", tokens_text(stores, &tokens)));
    Ok(())
}

pub(crate) fn execute_showbox(stores: &mut Universe, index: u16) {
    let mut text = format!("\n> \\box{index}=\n");
    if let Some(id) = stores.box_reg(index) {
        text.push_str(&dump_node_list(stores, id, DumpConfig::read(stores)));
    } else {
        text.push_str("void\n");
    }
    text.push_str("\n! OK.\n");
    write_diagnostic(stores, &text);
}

pub(crate) fn execute_message<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    error: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let tokens = scan_balanced_expanded_text(input, stores, hooks, "\\message")?;
    let text = tokens_text(stores, &tokens);
    if error {
        write_diagnostic(stores, &format!("\n! {text}.\n"));
    } else {
        let mut output = write_wrapped_message(&text);
        output.push(' ');
        write_diagnostic(stores, &output);
    }
    Ok(())
}

pub(crate) fn execute_showlists(stores: &mut Universe) {
    write_diagnostic(
        stores,
        "\n### vertical mode entered at line 0\nprevdepth ignored\n\n! OK.\n",
    );
}

pub(crate) fn execute_showhyphens<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let tokens = scan_balanced_expanded_text(input, stores, hooks, "\\showhyphens")?;
    let mut words = Vec::new();
    let mut current = String::new();
    for token in tokens {
        match token {
            Token::Char {
                ch: _,
                cat: Catcode::Space,
            } => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            Token::Char { ch, .. } => {
                if let Some(lower) = char::from_u32(stores.lccode(ch)).filter(|&ch| ch != '\0') {
                    current.push(lower);
                } else if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            Token::Cs(_) | Token::Param(_) => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
        }
    }
    if !current.is_empty() {
        words.push(current);
    }

    let left = stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(0) as usize;
    let right = stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(0) as usize;
    let mut lines = String::new();
    lines.push('\n');
    for word in words {
        let positions = stores.hyphen_positions(&word, left, right);
        lines.push_str(&hyphenated_word_text(&word, &positions));
        lines.push('\n');
    }
    lines.push_str("! OK.\n");
    write_diagnostic(stores, &lines);
    Ok(())
}

pub(crate) fn execute_change_case<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    uppercase: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let mut tokens = scan_balanced_raw_text(
        input,
        stores,
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

pub(crate) fn execute_ignorespaces<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    loop {
        let Some(token) = input.next_token(stores)? else {
            return Ok(());
        };
        if !is_space(token) {
            push_tokens(input, stores, [token]);
            return Ok(());
        }
    }
}

fn show_meaning_text(stores: &Universe, token: Token) -> String {
    let text = meaning_text(stores, token);
    if let Some(rest) = text.strip_prefix("macro:") {
        format!("macro:\n{rest}")
    } else if let Some(rest) = text.strip_prefix("protectedmacro:") {
        format!("protectedmacro:\n{rest}")
    } else {
        text
    }
}

fn scan_balanced_raw_text<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    context: &'static str,
) -> Result<Vec<Token>, ExecError>
where
    S: InputSource,
{
    let open = next_non_space_raw(input, stores)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(open) {
        return Err(ExecError::MissingToken { context });
    }
    let mut depth = 1usize;
    let mut tokens = Vec::new();
    while let Some(token) = input.next_token(stores)? {
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

fn scan_balanced_expanded_text<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: &'static str,
) -> Result<Vec<Token>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let open =
        next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(open) {
        return Err(ExecError::MissingToken { context });
    }
    let mut recorder = NoopRecorder;
    let mut depth = 1usize;
    let mut tokens = Vec::new();
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
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
    Err(ExecError::MissingToken { context })
}

fn next_non_space_raw<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<Option<Token>, ExecError>
where
    S: InputSource,
{
    while let Some(token) = input.next_token(stores)? {
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
    Ok(None)
}

fn next_non_space_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Option<Token>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
    {
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
    Ok(None)
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

fn hyphenated_word_text(word: &str, positions: &[usize]) -> String {
    let mut out = String::new();
    for (index, ch) in word.chars().enumerate() {
        if positions.contains(&index) {
            out.push('-');
        }
        out.push(ch);
    }
    if positions.contains(&word.chars().count()) {
        out.push('-');
    }
    out
}

fn write_wrapped_message(text: &str) -> String {
    let mut output = String::new();
    let mut column = 0usize;
    for ch in text.chars() {
        if column == 79 {
            output.push('\n');
            column = 0;
        }
        output.push(ch);
        column += 1;
    }
    output
}

fn write_diagnostic(stores: &mut Universe, text: &str) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, text);
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
