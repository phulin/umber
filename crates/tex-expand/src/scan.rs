//! Macro definition token scanning.
//!
//! This module implements the reusable `scan_toks`-style part of `\def` and
//! `\edef`: scan the parameter text, then scan the brace-balanced replacement
//! text. It freezes the resulting token lists through `Stores`, but it does
//! not assign the macro meaning to `Env`.

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError};
use tex_state::ids::TokenListId;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::MeaningFlags;
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

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
            _ => None,
        }
    }
}

impl From<LexError> for ScanToksError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
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
    stores: &mut Stores,
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

fn scan_parameter_text<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
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
    stores: &mut Stores,
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
    use tex_state::meaning::MeaningFlags;
    use tex_state::stores::Stores;
    use tex_state::token::{Catcode, Token};

    fn scan(input: &str) -> (Stores, Vec<Token>, Vec<Token>) {
        let mut stores = Stores::new();
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
        let mut stores = Stores::new();
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
