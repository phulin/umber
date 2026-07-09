use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::ExpansionState;
use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::{
    ExpandError, ExpandNext, ExpansionHooks, NoInputExpandNext, ReadRecorder, scan_int,
    semantic_token,
};

pub(crate) fn next_non_space_x_token_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    next_non_space_x_token_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
    )
}

pub(crate) fn next_non_space_x_token_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    loop {
        let Some(token) = expander.next_expanded_token(input, stores, recorder, hooks)? else {
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
pub(crate) fn scan_register_index<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    context: tex_state::token::TracedTokenWord,
) -> Result<u16, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    scan_register_index_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
        context,
    )
}

pub(crate) fn scan_register_index_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    context: tex_state::token::TracedTokenWord,
) -> Result<u16, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let scanned = scan_int::scan_int_with_expander_and_hooks(
        input, stores, recorder, hooks, expander, context,
    )?;
    let value = scanned.value();
    if !(0..=32_767).contains(&value) {
        return Err(scan_int::ScanIntError::RegisterNumberOutOfRange {
            value,
            context: scanned.context(),
        }
        .into());
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
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    keyword: &str,
) -> Result<bool, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    scan_optional_keyword_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
        keyword,
    )
}

pub fn scan_optional_keyword_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    keyword: &str,
) -> Result<bool, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let Some(first) =
        next_non_space_x_token_with_expander_and_hooks(input, stores, recorder, hooks, expander)?
    else {
        return Ok(false);
    };
    match scan_keyword_after_first_with_expander_and_hooks(
        input, stores, recorder, hooks, expander, first, keyword,
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
pub fn scan_keyword_after_first_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    first: TracedTokenWord,
    keyword: &str,
) -> Result<ExpandedKeywordMatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    scan_keyword_after_first_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
        first,
        keyword,
    )
}

pub fn scan_keyword_after_first_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    first: TracedTokenWord,
    keyword: &str,
) -> Result<ExpandedKeywordMatch, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
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
        let Some(token) = expander.next_expanded_token(input, stores, recorder, hooks)? else {
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
    let traced_tokens = tokens.into_iter().collect::<Vec<_>>();
    let tokens = traced_tokens
        .iter()
        .copied()
        .map(semantic_token)
        .collect::<Vec<_>>();
    let token_list = stores.intern_token_list(&tokens);
    let mut origins = stores.origin_list_builder();
    for token in traced_tokens {
        origins.push(token.origin());
    }
    let origin_list = stores.finish_origin_list(&mut origins);
    input.push_token_list_with_origins(token_list, origin_list, TokenListReplayKind::Inserted);
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
