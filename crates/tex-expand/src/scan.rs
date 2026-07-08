//! Macro definition token scanning.
//!
//! This module implements the reusable `scan_toks`-style part of `\def` and
//! `\edef`: scan the parameter text, then scan the brace-balanced replacement
//! text. It freezes the resulting token lists through `Universe`, but it does
//! not assign the macro meaning to `Env`.

use std::{fmt, marker::PhantomData};

use tex_lex::{InputSource, InputStack, LexError, MemoryInput, TokenListReplayKind};
use tex_state::ids::TokenListId;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionState, InputReadState};

use crate::{
    DriverExpandNext, ExpandError, ExpandNext, ExpandableOpcode, ExpansionHooks, NoInputExpandNext,
    NoopRecorder,
};

/// Result of scanning a macro definition without assigning it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMacro {
    meaning: MacroMeaning,
}

impl ScannedMacro {
    #[must_use]
    pub const fn meaning(self) -> MacroMeaning {
        self.meaning
    }

    #[must_use]
    pub const fn parameter_text(self) -> TokenListId {
        self.meaning.parameter_text()
    }

    #[must_use]
    pub const fn replacement_text(self) -> TokenListId {
        self.meaning.replacement_text()
    }
}

/// Errors raised while scanning a macro definition.
#[derive(Debug)]
pub enum ScanToksError {
    Lex(LexError),
    Expand(ExpandError),
    EndOfInputInParameterText,
    EndOfInputInReplacementText,
    ParameterNumberOutOfOrder { expected: u8, found: u8 },
    TooManyParameters,
    InvalidParameterTokenInParameterText(Token),
    InvalidParameterTokenInReplacementText(Token),
    MissingGeneralTextBeginGroup(Token),
}

impl fmt::Display for ScanToksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => write!(f, "{err}"),
            Self::Expand(err) => write!(f, "{err}"),
            Self::EndOfInputInParameterText => {
                write!(f, "end of input while scanning macro parameter text")
            }
            Self::EndOfInputInReplacementText => {
                write!(f, "end of input while scanning macro replacement text")
            }
            Self::ParameterNumberOutOfOrder { expected, found } => write!(
                f,
                "macro parameter number out of order: expected #{expected}, found #{found}"
            ),
            Self::TooManyParameters => write!(f, "macro definitions support only #1 through #9"),
            Self::InvalidParameterTokenInParameterText(token) => {
                write!(
                    f,
                    "invalid parameter token {token:?} in macro parameter text"
                )
            }
            Self::InvalidParameterTokenInReplacementText(token) => {
                write!(
                    f,
                    "invalid parameter token {token:?} in macro replacement text"
                )
            }
            Self::MissingGeneralTextBeginGroup(token) => {
                write!(
                    f,
                    "expected begin-group token before general text, got {token:?}"
                )
            }
        }
    }
}

impl std::error::Error for ScanToksError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lex(err) => Some(err),
            Self::Expand(err) => Some(err),
            _ => None,
        }
    }
}

impl From<LexError> for ScanToksError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<ExpandError> for ScanToksError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

/// Scans a macro definition from the current input position.
///
/// The control sequence being defined is already consumed by the caller. This
/// scans tokens up to the opening replacement brace as parameter text, then
/// captures a balanced replacement body. Frozen token-list ids are returned in
/// a `MacroMeaning`; callers decide whether, where, and how to assign it.
pub fn scan_toks<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    flags: MeaningFlags,
) -> Result<ScannedMacro, ScanToksError>
where
    S: InputSource,
{
    let parameter_text = scan_parameter_text(input, stores)?;
    let replacement_text = scan_replacement_text(input, stores)?;
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(flags, parameter_text, replacement_text),
    })
}

/// Scans a macro definition and expands the replacement text as for `\edef`.
pub fn scan_toks_expanded<S, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    flags: MeaningFlags,
    hooks: &mut H,
) -> Result<ScannedMacro, ScanToksError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let scanned = scan_toks(input, stores, flags)?;
    let meaning = scanned.meaning();
    let replacement_text = expand_replacement_text(
        stores,
        meaning.replacement_text(),
        hooks,
        &mut NoInputExpandNext,
    )?;
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(flags, meaning.parameter_text(), replacement_text),
    })
}

pub fn scan_toks_expanded_with_driver<S, St, H>(
    input: &mut InputStack<S>,
    stores: &mut St,
    flags: MeaningFlags,
    hooks: &mut H,
) -> Result<ScannedMacro, ScanToksError>
where
    S: InputSource,
    St: ExpansionState + tex_state::InputOpenState,
    H: ExpansionHooks<S>,
{
    let scanned = scan_toks(input, stores, flags)?;
    let meaning = scanned.meaning();
    let replacement_text = expand_replacement_text(
        stores,
        meaning.replacement_text(),
        hooks,
        &mut DriverExpandNext,
    )?;
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(flags, meaning.parameter_text(), replacement_text),
    })
}

/// Scans TeX general text as a raw balanced group, then expands it.
///
/// This matches `scan_toks(macro_def = false, xpand = true)` callers such as
/// TeX82 `\mark`: parameter tokens are ordinary tokens while scanning the
/// balanced text, and expansion happens over the frozen raw text.
pub fn scan_general_text_expanded_with_driver<S, St, H>(
    input: &mut InputStack<S>,
    stores: &mut St,
    hooks: &mut H,
) -> Result<TokenListId, ScanToksError>
where
    S: InputSource,
    St: ExpansionState + tex_state::InputOpenState,
    H: ExpansionHooks<S>,
{
    let raw_text = scan_general_text(input, stores)?;
    expand_replacement_text(stores, raw_text, hooks, &mut DriverExpandNext)
}

fn expand_replacement_text<'a, S, St, H, E>(
    stores: &mut St,
    replacement_text: TokenListId,
    hooks: &'a mut H,
    expander: &mut E,
) -> Result<TokenListId, ScanToksError>
where
    S: InputSource,
    St: ExpansionState,
    H: ExpansionHooks<S> + 'a,
    E: ExpandNext<ReplacementSource<S>, St, NoopRecorder, ReplacementHooks<'a, S, H>>,
{
    let mut input = InputStack::new(ReplacementSource::<S>::empty());
    input.push_token_list(replacement_text, TokenListReplayKind::Inserted);
    let mut builder = stores.token_list_builder();
    let mut recorder = NoopRecorder;
    let mut hooks = ReplacementHooks::new(hooks);

    loop {
        let Some(read) = input.next_expansion_token(stores)? else {
            break;
        };
        let token = read.token();
        if read.suppress_expansion() {
            builder.push(token);
            continue;
        }

        let Token::Cs(symbol) = token else {
            builder.push(token);
            continue;
        };
        let meaning = stores.meaning(symbol);
        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) {
            let Some(suppressed) = input.next_token(stores)? else {
                return Err(
                    ExpandError::MissingTokenAfterPrimitive(ExpandableOpcode::NoExpand).into(),
                );
            };
            builder.push(suppressed);
            continue;
        }

        unread_token(&mut input, stores, token);
        if let Some(expanded) =
            expander.next_expanded_token(&mut input, stores, &mut recorder, &mut hooks)?
        {
            builder.push(expanded);
        }
    }
    Ok(stores.finish_token_list(&mut builder))
}

fn unread_token<S>(input: &mut InputStack<S>, stores: &mut impl ExpansionState, token: Token)
where
    S: InputSource,
{
    let token_list = stores.intern_token_list(&[token]);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
}

enum ReplacementSource<S> {
    Empty(MemoryInput),
    Driver(S),
}

impl<S> ReplacementSource<S> {
    fn empty() -> Self {
        Self::Empty(MemoryInput::new(""))
    }
}

impl<S> InputSource for ReplacementSource<S>
where
    S: InputSource,
{
    fn read_line(&mut self) -> Result<Option<String>, tex_state::WorldError> {
        match self {
            Self::Empty(source) => source.read_line(),
            Self::Driver(source) => source.read_line(),
        }
    }
}

struct ReplacementHooks<'a, S, H> {
    inner: &'a mut H,
    _source: PhantomData<fn() -> S>,
}

impl<'a, S, H> ReplacementHooks<'a, S, H> {
    fn new(inner: &'a mut H) -> Self {
        Self {
            inner,
            _source: PhantomData,
        }
    }
}

impl<S, H> ExpansionHooks<ReplacementSource<S>> for ReplacementHooks<'_, S, H>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    fn open_input<C: InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<ReplacementSource<S>, String> {
        self.inner
            .open_input(input, name)
            .map(ReplacementSource::Driver)
    }

    fn job_name(&self) -> &str {
        self.inner.job_name()
    }

    fn mode(&self) -> crate::EngineMode {
        self.inner.mode()
    }

    fn is_inner_mode(&self) -> bool {
        self.inner.is_inner_mode()
    }

    fn space_factor(&self) -> i32 {
        self.inner.space_factor()
    }

    fn prev_depth(&self) -> tex_state::scaled::Scaled {
        self.inner.prev_depth()
    }

    fn prev_graf(&self) -> i32 {
        self.inner.prev_graf()
    }

    fn last_penalty(&self) -> i32 {
        self.inner.last_penalty()
    }

    fn last_kern(&self) -> tex_state::scaled::Scaled {
        self.inner.last_kern()
    }

    fn last_skip(&self) -> tex_state::glue::GlueSpec {
        self.inner.last_skip()
    }

    fn input_stream_eof(&self, stores: &impl ExpansionState, stream: u8) -> bool {
        self.inner.input_stream_eof(stores, stream)
    }

    fn set_engine_state(&mut self, state: crate::EngineStateSnapshot) {
        self.inner.set_engine_state(state);
    }
}

fn scan_parameter_text<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<TokenListId, ScanToksError>
where
    S: InputSource,
{
    let mut builder = stores.token_list_builder();
    let mut next_parameter = 1;
    let mut pending_parameter = false;

    loop {
        let token = input
            .next_token(stores)?
            .ok_or(ScanToksError::EndOfInputInParameterText)?;

        if pending_parameter {
            pending_parameter = false;
            match token {
                Token::Char {
                    ch: '1'..='9',
                    cat: Catcode::Other,
                } => {
                    let found = token_digit(token).expect("digit token was matched");
                    if found != next_parameter {
                        return Err(ScanToksError::ParameterNumberOutOfOrder {
                            expected: next_parameter,
                            found,
                        });
                    }
                    builder.push(Token::param(found));
                    next_parameter = next_parameter
                        .checked_add(1)
                        .filter(|value| *value <= 10)
                        .ok_or(ScanToksError::TooManyParameters)?;
                }
                Token::Char {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    builder.push(token);
                    return Ok(stores.finish_token_list(&mut builder));
                }
                _ => return Err(ScanToksError::InvalidParameterTokenInParameterText(token)),
            }
            continue;
        }

        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => return Ok(stores.finish_token_list(&mut builder)),
            Token::Char {
                cat: Catcode::Parameter,
                ..
            } => pending_parameter = true,
            _ => builder.push(token),
        }
    }
}

fn scan_replacement_text<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<TokenListId, ScanToksError>
where
    S: InputSource,
{
    let mut builder = stores.token_list_builder();
    let mut brace_level = 1_u32;
    let mut pending_parameter = false;

    loop {
        let token = input
            .next_token(stores)?
            .ok_or(ScanToksError::EndOfInputInReplacementText)?;

        if pending_parameter {
            pending_parameter = false;
            match token {
                Token::Char {
                    cat: Catcode::Parameter,
                    ..
                } => builder.push(token),
                Token::Char {
                    ch: '1'..='9',
                    cat: Catcode::Other,
                } => builder.push(Token::param(
                    token_digit(token).expect("digit token was matched"),
                )),
                _ => return Err(ScanToksError::InvalidParameterTokenInReplacementText(token)),
            }
            continue;
        }

        match token {
            Token::Char {
                cat: Catcode::Parameter,
                ..
            } => pending_parameter = true,
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                brace_level += 1;
                builder.push(token);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                brace_level -= 1;
                if brace_level == 0 {
                    return Ok(stores.finish_token_list(&mut builder));
                }
                builder.push(token);
            }
            _ => builder.push(token),
        }
    }
}

fn scan_general_text<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<TokenListId, ScanToksError>
where
    S: InputSource,
{
    let open =
        next_non_space_token(input, stores)?.ok_or(ScanToksError::EndOfInputInReplacementText)?;
    if !matches!(
        open,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    ) {
        return Err(ScanToksError::MissingGeneralTextBeginGroup(open));
    }

    let mut builder = stores.token_list_builder();
    let mut brace_level = 1_u32;
    loop {
        let token = input
            .next_token(stores)?
            .ok_or(ScanToksError::EndOfInputInReplacementText)?;
        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                brace_level += 1;
                builder.push(token);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                brace_level -= 1;
                if brace_level == 0 {
                    return Ok(stores.finish_token_list(&mut builder));
                }
                builder.push(token);
            }
            _ => builder.push(token),
        }
    }
}

fn next_non_space_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<Option<Token>, ScanToksError>
where
    S: InputSource,
{
    loop {
        let Some(token) = input.next_token(stores)? else {
            return Ok(None);
        };
        if !matches!(
            token,
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(token));
        }
    }
}

fn token_digit(token: Token) -> Option<u8> {
    let Token::Char {
        ch: '1'..='9',
        cat: Catcode::Other,
    } = token
    else {
        return None;
    };
    Some(match token {
        Token::Char { ch, .. } => ch as u8 - b'0',
        _ => unreachable!("matched token is a char"),
    })
}

#[cfg(test)]
mod tests;
