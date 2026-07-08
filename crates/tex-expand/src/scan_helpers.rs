use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionState, InputOpenState};

use crate::{
    ExpandError, ExpansionHooks, ReadRecorder, get_x_token_with_recorder_and_hooks, scan_int,
};

pub(crate) fn next_non_space_x_token_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
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

pub(crate) fn scan_register_index<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    hooks: &mut H,
) -> Result<u16, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let value = scan_int::scan_int_with_recorder_and_hooks(input, stores, recorder, hooks)?.value();
    if !(0..=32_767).contains(&value) {
        return Err(scan_int::ScanIntError::RegisterNumberOutOfRange(value).into());
    }
    Ok(value as u16)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpandedKeywordMatch {
    Matched,
    FirstTokenMismatch,
    PartialMismatch,
}

pub fn scan_optional_keyword_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    hooks: &mut H,
    keyword: &str,
) -> Result<bool, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(first) = next_non_space_x_token_with_hooks(input, stores, recorder, hooks)? else {
        return Ok(false);
    };
    match scan_keyword_after_first_with_hooks(input, stores, recorder, hooks, first, keyword)? {
        ExpandedKeywordMatch::Matched => Ok(true),
        ExpandedKeywordMatch::FirstTokenMismatch => {
            unread_token(input, stores, first);
            Ok(false)
        }
        ExpandedKeywordMatch::PartialMismatch => Ok(false),
    }
}

pub fn scan_keyword_after_first_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    hooks: &mut H,
    first: Token,
    keyword: &str,
) -> Result<ExpandedKeywordMatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    if keyword.is_empty() {
        return Ok(ExpandedKeywordMatch::Matched);
    }

    let mut consumed = Vec::with_capacity(keyword.len());
    consumed.push(first);

    if !token_matches_keyword_byte(first, keyword.as_bytes()[0]) {
        return Ok(ExpandedKeywordMatch::FirstTokenMismatch);
    }

    for &expected in &keyword.as_bytes()[1..] {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            unread_tokens(input, stores, consumed);
            return Ok(ExpandedKeywordMatch::PartialMismatch);
        };
        consumed.push(token);
        if !token_matches_keyword_byte(token, expected) {
            unread_tokens(input, stores, consumed);
            return Ok(ExpandedKeywordMatch::PartialMismatch);
        }
    }

    Ok(ExpandedKeywordMatch::Matched)
}

fn unread_token<S>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    token: Token,
) where
    S: InputSource,
{
    unread_tokens(input, stores, [token]);
}

fn unread_tokens<S, I>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    tokens: I,
) where
    S: InputSource,
    I: IntoIterator<Item = Token>,
{
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let token_list = stores.intern_token_list(&tokens);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
}

fn token_matches_keyword_byte(token: Token, expected: u8) -> bool {
    let Token::Char {
        ch,
        cat: Catcode::Letter | Catcode::Other,
    } = token
    else {
        return false;
    };
    ch.eq_ignore_ascii_case(&char::from(expected))
}
