use tex_lex::{InputSource, InputStack};
use tex_state::ExpansionState;
use tex_state::env::banks::IntParam;
use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::{
    ExpandError, ExpansionContext, ExpansionMode, RestrictedExpansionMode, scan_int, semantic_token,
};

pub(crate) fn next_non_space_x_token_with_context<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
{
    next_non_space_x_token_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
    )
}

pub(crate) fn next_non_space_x_token_with_mode_and_context<S, St>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    mode: &mut dyn ExpansionMode<S, St>,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
{
    loop {
        let Some(token) = mode.next_expanded_token(input, stores, expansion)? else {
            return Ok(None);
        };
        if !matches!(
            semantic_token(token),
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(token));
        }
    }
}

#[allow(dead_code)]
pub(crate) fn scan_register_index<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    context: tex_state::token::TracedTokenWord,
) -> Result<u16, ExpandError>
where
    S: InputSource,
{
    scan_register_index_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
        context,
    )
}

pub(crate) fn scan_register_index_with_mode_and_context<S, St>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    mode: &mut dyn ExpansionMode<S, St>,
    context: tex_state::token::TracedTokenWord,
) -> Result<u16, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
{
    let scanned =
        scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?;
    let value = scanned.value();
    let maximum = maximum_register_index(stores);
    if !(0..=i32::from(maximum)).contains(&value) {
        stores.report_bad_register_code(value, maximum);
        return Ok(0);
    }
    Ok(value as u16)
}

pub(crate) fn maximum_register_index(stores: &impl ExpansionState) -> u16 {
    if stores.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 {
        32_767
    } else {
        255
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpandedKeywordMatch {
    Matched,
    FirstTokenMismatch,
    PartialMismatch,
}

pub fn scan_optional_keyword_with_context<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    keyword: &str,
) -> Result<bool, ExpandError>
where
    S: InputSource,
{
    scan_optional_keyword_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
        keyword,
    )
}

pub fn scan_optional_keyword_with_mode_and_context<S, St>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    mode: &mut dyn ExpansionMode<S, St>,
    keyword: &str,
) -> Result<bool, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
{
    let Some(first) = next_non_space_x_token_with_mode_and_context(input, stores, expansion, mode)?
    else {
        return Ok(false);
    };
    match scan_keyword_after_first_with_mode_and_context(
        input, stores, expansion, mode, first, keyword,
    )? {
        ExpandedKeywordMatch::Matched => Ok(true),
        ExpandedKeywordMatch::FirstTokenMismatch => {
            unread_token(input, stores, first);
            Ok(false)
        }
        ExpandedKeywordMatch::PartialMismatch => Ok(false),
    }
}

#[allow(dead_code)]
pub fn scan_keyword_after_first_with_context<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    first: TracedTokenWord,
    keyword: &str,
) -> Result<ExpandedKeywordMatch, ExpandError>
where
    S: InputSource,
{
    scan_keyword_after_first_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
        first,
        keyword,
    )
}

pub fn scan_keyword_after_first_with_mode_and_context<S, St>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    mode: &mut dyn ExpansionMode<S, St>,
    first: TracedTokenWord,
    keyword: &str,
) -> Result<ExpandedKeywordMatch, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
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
        let Some(token) = mode.next_expanded_token(input, stores, expansion)? else {
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
    stores: &mut impl ExpansionState,
    token: TracedTokenWord,
) where
    S: InputSource,
{
    unread_tokens(input, stores, [token]);
}

fn unread_tokens<S, I>(input: &mut InputStack<S>, stores: &mut impl ExpansionState, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = TracedTokenWord>,
{
    crate::back_input(input, stores, tokens);
}

fn token_matches_keyword_byte(token: TracedTokenWord, expected: u8) -> bool {
    let Token::Char {
        ch,
        cat: Catcode::Letter | Catcode::Other,
    } = semantic_token(token)
    else {
        return false;
    };
    ch.eq_ignore_ascii_case(&char::from(expected))
}
