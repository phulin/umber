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
use tex_state::{ExpansionState, InputOpenState, InputReadState};

use crate::{
    Dispatch, ExpandError, ExpandableOpcode, ExpansionHooks, ExpansionReplayKind, NoopRecorder,
    dispatch_with_hooks,
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
    stores: &mut (impl ExpansionState + InputOpenState),
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
    stores: &mut (impl ExpansionState + InputOpenState),
    flags: MeaningFlags,
    hooks: &mut H,
) -> Result<ScannedMacro, ScanToksError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let scanned = scan_toks(input, stores, flags)?;
    let meaning = scanned.meaning();
    let replacement_text = expand_replacement_text(stores, meaning.replacement_text(), hooks)?;
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(flags, meaning.parameter_text(), replacement_text),
    })
}

fn expand_replacement_text<S, H>(
    stores: &mut (impl ExpansionState + InputOpenState),
    replacement_text: TokenListId,
    hooks: &mut H,
) -> Result<TokenListId, ScanToksError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
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

        match dispatch_with_hooks(
            token,
            &mut input,
            stores,
            &mut recorder,
            &mut hooks,
            meaning,
        )? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => builder.push(token),
            Dispatch::Push {
                replay_kind: ExpansionReplayKind::TheOutput,
                token_list,
                ..
            } => {
                for token in stores.tokens(token_list).iter().copied() {
                    builder.push(token);
                }
            }
            push @ Dispatch::Push { .. } => apply_edef_push(&mut input, push),
        }
    }
    Ok(stores.finish_token_list(&mut builder))
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

    fn input_stream_eof(&self, stores: &impl ExpansionState, stream: u8) -> bool {
        self.inner.input_stream_eof(stores, stream)
    }
}

fn apply_edef_push<S>(input: &mut InputStack<ReplacementSource<S>>, dispatch: Dispatch)
where
    S: InputSource,
{
    let Dispatch::Push {
        replay_kind,
        token_list,
        macro_arguments,
    } = dispatch
    else {
        return;
    };
    if replay_kind == ExpansionReplayKind::MacroBody {
        input.push_macro_body(token_list, macro_arguments);
    } else {
        input.push_token_list(token_list, replay_kind.as_lex_kind());
    }
}

fn scan_parameter_text<S>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
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
    stores: &mut (impl ExpansionState + InputOpenState),
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
mod tests {
    use super::{ScanToksError, scan_toks};
    use tex_lex::{InputStack, MemoryInput};
    use tex_state::Universe;
    use tex_state::meaning::MeaningFlags;
    use tex_state::token::{Catcode, Token};

    fn scan(input: &str) -> (Universe, Vec<Token>, Vec<Token>) {
        let mut stores = Universe::new();
        let mut input = InputStack::new(MemoryInput::new(input));
        let scanned =
            scan_toks(&mut input, &mut stores, MeaningFlags::EMPTY).expect("scan should succeed");
        let params = stores.tokens(scanned.parameter_text()).to_vec();
        let replacement = stores.tokens(scanned.replacement_text()).to_vec();
        (stores, params, replacement)
    }

    fn char_token(ch: char, cat: Catcode) -> Token {
        Token::Char { ch, cat }
    }

    #[test]
    fn scans_delimited_and_undelimited_parameters() {
        let (_stores, params, replacement) = scan("#1a#2{#2#1}");

        assert_eq!(
            params,
            vec![
                Token::param(1),
                char_token('a', Catcode::Letter),
                Token::param(2),
            ]
        );
        assert_eq!(replacement, vec![Token::param(2), Token::param(1)]);
    }

    #[test]
    fn scans_all_nine_parameters_in_order() {
        let (_stores, params, replacement) = scan("#1#2#3#4#5#6#7#8#9{#9#1}");

        assert_eq!(params, (1_u8..=9).map(Token::param).collect::<Vec<_>>());
        assert_eq!(replacement, vec![Token::param(9), Token::param(1)]);
    }

    #[test]
    fn rejects_out_of_order_parameter_numbers() {
        let mut stores = Universe::new();
        let mut input = InputStack::new(MemoryInput::new("#2{}"));

        let err = scan_toks(&mut input, &mut stores, MeaningFlags::EMPTY)
            .expect_err("scan should reject out-of-order parameter");

        assert!(matches!(
            err,
            ScanToksError::ParameterNumberOutOfOrder {
                expected: 1,
                found: 2
            }
        ));
    }

    #[test]
    fn scans_trailing_hash_brace_parameter_text() {
        let (_stores, params, replacement) = scan("#1#{#1}");

        assert_eq!(
            params,
            vec![Token::param(1), char_token('{', Catcode::BeginGroup)]
        );
        assert_eq!(replacement, vec![Token::param(1)]);
    }

    #[test]
    fn captures_nested_braces_in_replacement_text() {
        let (_stores, params, replacement) = scan("{a{b}c}");

        assert!(params.is_empty());
        assert_eq!(
            replacement,
            vec![
                char_token('a', Catcode::Letter),
                char_token('{', Catcode::BeginGroup),
                char_token('b', Catcode::Letter),
                char_token('}', Catcode::EndGroup),
                char_token('c', Catcode::Letter),
            ]
        );
    }

    #[test]
    fn scans_doubled_hash_as_literal_parameter_character_in_body() {
        let (_stores, params, replacement) = scan("{##}");

        assert!(params.is_empty());
        assert_eq!(replacement, vec![char_token('#', Catcode::Parameter)]);
    }
}
