//! Temporary diagnostic sink and log-writing primitives.

use tex_expand::{
    ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks, meaning_text,
    scan_the_text_with_hooks, token_text,
};
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::token::{Catcode, Token};

use crate::{ExecError, push_tokens};

/// Minimal diagnostic sink kept local until the World effect log lands.
pub trait LogSink {
    fn write(&mut self, text: &str);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopLogSink;

impl LogSink for NoopLogSink {
    fn write(&mut self, _text: &str) {}
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StringLogSink {
    output: String,
}

impl StringLogSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.output
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.output
    }
}

impl LogSink for StringLogSink {
    fn write(&mut self, text: &str) {
        self.output.push_str(text);
    }
}

pub(crate) fn execute_show<S, L>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    log: &mut L,
) -> Result<(), ExecError>
where
    S: InputSource,
    L: LogSink,
{
    let token = input
        .next_token(stores)?
        .ok_or(ExecError::MissingToken { context: "\\show" })?;
    match token {
        Token::Cs(_) => {
            log.write("\n> ");
            log.write(&token_text(stores, token));
            log.write("=");
            log.write(&show_meaning_text(stores, token));
            log.write(".\n");
        }
        Token::Char { .. } | Token::Param(_) => {
            log.write("\n> ");
            log.write(&meaning_text(stores, token));
            log.write(".\n");
        }
    }
    Ok(())
}

pub(crate) fn execute_showthe<S, H, L>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    log: &mut L,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
    L: LogSink,
{
    let mut recorder = NoopRecorder;
    let text = scan_the_text_with_hooks(input, stores, &mut recorder, hooks)?;
    log.write("\n> ");
    log.write(&text);
    log.write(".\n");
    Ok(())
}

pub(crate) fn execute_showtokens<S, L>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    log: &mut L,
) -> Result<(), ExecError>
where
    S: InputSource,
    L: LogSink,
{
    let tokens = scan_balanced_raw_text(input, stores, "\\showtokens")?;
    log.write("\n> ");
    log.write(&tokens_text(stores, &tokens));
    log.write(".\n");
    Ok(())
}

pub(crate) fn execute_message<S, H, L>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    log: &mut L,
    error: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
    L: LogSink,
{
    let tokens = scan_balanced_expanded_text(input, stores, hooks, "\\message")?;
    if error {
        log.write("\n! ");
    }
    if error {
        log.write(&tokens_text(stores, &tokens));
        log.write(".\n");
    } else {
        write_wrapped_message(log, &tokens_text(stores, &tokens));
        log.write(" ");
    }
    Ok(())
}

pub(crate) fn execute_showlists<L>(log: &mut L)
where
    L: LogSink,
{
    log.write("\n### vertical mode entered at line 0\nprevdepth ignored\n\n! OK.\n");
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

fn write_wrapped_message<L>(log: &mut L, text: &str)
where
    L: LogSink,
{
    let mut column = 0usize;
    for ch in text.chars() {
        if column == 79 {
            log.write("\n");
            column = 0;
        }
        let mut buffer = [0; 4];
        log.write(ch.encode_utf8(&mut buffer));
        column += 1;
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
