//! Macro-call argument matching.
//!
//! This is the TeX gullet scanner for macro parameter text. It consumes the
//! call-site input, freezes matched arguments through `Universe`, and leaves body
//! replay/substitution to the expansion-frame work.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use tex_lex::{InputStack, LexError, MACRO_ARGUMENT_SLOTS, MacroArguments};
use tex_state::ExpansionState;
use tex_state::TracedTokenList;
use tex_state::ids::TokenListId;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::ExpansionContext;

/// Frozen arguments matched for one macro call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatchedArguments {
    arguments: Vec<TracedTokenList>,
}

impl MatchedArguments {
    #[must_use]
    pub fn len(&self) -> usize {
        self.arguments.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.arguments.is_empty()
    }

    #[must_use]
    pub fn get(&self, slot: u8) -> Option<TokenListId> {
        self.get_traced(slot).map(TracedTokenList::token_list)
    }

    #[must_use]
    pub fn get_traced(&self, slot: u8) -> Option<TracedTokenList> {
        slot.checked_sub(1)
            .and_then(|index| self.arguments.get(index as usize))
            .copied()
    }

    #[must_use]
    pub fn as_macro_arguments(&self) -> MacroArguments {
        assert!(
            self.arguments.len() <= MACRO_ARGUMENT_SLOTS,
            "macro calls support only #1 through #9"
        );
        let mut arguments = MacroArguments::new();
        for (index, &id) in self.arguments.iter().enumerate() {
            arguments.set_traced((index + 1) as u8, id);
        }
        arguments
    }

    fn push(&mut self, id: TracedTokenList) {
        self.arguments.push(id);
    }
}

/// Errors raised while matching a macro call.
#[derive(Debug)]
pub enum MacroCallError {
    Lex(LexError),
    EndOfInput {
        macro_name: String,
        context: TracedTokenWord,
    },
    DoesNotMatchDefinition {
        macro_name: String,
        context: TracedTokenWord,
    },
    ParagraphEndedBeforeComplete {
        macro_name: String,
        context: TracedTokenWord,
    },
    ForbiddenOuterToken {
        macro_name: String,
        context: TracedTokenWord,
    },
}

impl fmt::Display for MacroCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => write!(f, "{err}"),
            Self::EndOfInput { macro_name, .. } => {
                write!(f, "File ended while scanning use of {macro_name}")
            }
            Self::DoesNotMatchDefinition { macro_name, .. } => {
                write!(f, "Use of {macro_name} doesn't match its definition")
            }
            Self::ParagraphEndedBeforeComplete { macro_name, .. } => {
                write!(f, "Paragraph ended before {macro_name} was complete")
            }
            Self::ForbiddenOuterToken { macro_name, .. } => {
                write!(
                    f,
                    "Forbidden control sequence found while scanning use of {macro_name}"
                )
            }
        }
    }
}

impl std::error::Error for MacroCallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lex(err) => Some(err),
            _ => None,
        }
    }
}

impl From<LexError> for MacroCallError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl MacroCallError {
    #[must_use]
    pub fn primary_origin(&self) -> Option<OriginId> {
        match self {
            Self::Lex(_) => None,
            Self::EndOfInput { context, .. }
            | Self::DoesNotMatchDefinition { context, .. }
            | Self::ParagraphEndedBeforeComplete { context, .. }
            | Self::ForbiddenOuterToken { context, .. } => Some(context.origin()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParameterSpec {
    delimiter: Vec<Token>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParameterPattern {
    leading: Vec<Token>,
    specs: Vec<ParameterSpec>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingArgumentToken {
    token: TracedTokenWord,
    allow_par: bool,
}

/// Matches one macro call and freezes each argument token list.
pub fn match_macro_call(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    call_token: TracedTokenWord,
    meaning: MacroMeaning,
) -> Result<MatchedArguments, MacroCallError> {
    let mut expansion = ExpansionContext::new("texput");
    match_macro_call_with_context(input, stores, &mut expansion, call_token, meaning)
}

pub(crate) fn match_macro_call_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    call_token: TracedTokenWord,
    meaning: MacroMeaning,
) -> Result<MatchedArguments, MacroCallError> {
    let macro_name = macro_name(stores, traced_semantic_token(call_token));
    let parameter_text = meaning.parameter_text();
    let pattern = match expansion.parameter_pattern_cache.get(&parameter_text) {
        Some(pattern) => Arc::clone(pattern),
        None => {
            let pattern = Arc::new(parse_parameter_text(stores.tokens(parameter_text)));
            expansion
                .parameter_pattern_cache
                .insert(parameter_text, Arc::clone(&pattern));
            pattern
        }
    };
    match_exact_tokens(
        input,
        stores,
        expansion,
        meaning.flags(),
        &macro_name,
        &pattern.leading,
        call_token,
    )?;

    let mut matched = MatchedArguments::default();
    for spec in &pattern.specs {
        let id = if spec.delimiter.is_empty() {
            scan_undelimited_argument(
                input,
                stores,
                expansion,
                meaning.flags(),
                &macro_name,
                call_token,
            )?
        } else {
            scan_delimited_argument(
                input,
                stores,
                expansion,
                meaning.flags(),
                &macro_name,
                &spec.delimiter,
                call_token,
            )?
        };
        matched.push(id);
    }
    Ok(matched)
}

fn parse_parameter_text(tokens: &[Token]) -> ParameterPattern {
    let mut leading = Vec::new();
    let mut specs = Vec::new();
    let mut current: Option<ParameterSpec> = None;

    for &token in tokens {
        match token {
            Token::Param(_slot) => {
                if let Some(spec) = current.take() {
                    specs.push(spec);
                }
                current = Some(ParameterSpec {
                    delimiter: Vec::new(),
                });
            }
            _ => {
                if let Some(spec) = current.as_mut() {
                    spec.delimiter.push(token);
                } else {
                    leading.push(token);
                }
            }
        }
    }

    if let Some(spec) = current {
        specs.push(spec);
    }

    ParameterPattern { leading, specs }
}

fn match_exact_tokens(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    flags: MeaningFlags,
    macro_name: &str,
    expected: &[Token],
    call_context: TracedTokenWord,
) -> Result<(), MacroCallError> {
    for &expected_token in expected {
        let token = next_checked_token(input, stores, expansion, flags, macro_name, call_context)?;
        if traced_semantic_token(token) != expected_token {
            return Err(MacroCallError::DoesNotMatchDefinition {
                macro_name: macro_name.to_owned(),
                context: token,
            });
        }
    }
    Ok(())
}

fn scan_undelimited_argument(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    flags: MeaningFlags,
    macro_name: &str,
    call_context: TracedTokenWord,
) -> Result<TracedTokenList, MacroCallError> {
    let mut token = next_checked_token(input, stores, expansion, flags, macro_name, call_context)?;
    while is_space_token(traced_semantic_token(token)) {
        token = next_checked_token(input, stores, expansion, flags, macro_name, call_context)?;
    }

    let mut tokens = Vec::new();
    if is_begin_group(traced_semantic_token(token)) {
        scan_balanced_group(
            input,
            stores,
            expansion,
            flags,
            macro_name,
            call_context,
            &mut tokens,
        )?;
    } else {
        tokens.push(token);
    }
    Ok(freeze_traced_tokens(stores, &tokens))
}

fn scan_balanced_group(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    flags: MeaningFlags,
    macro_name: &str,
    call_context: TracedTokenWord,
    tokens: &mut Vec<TracedTokenWord>,
) -> Result<(), MacroCallError> {
    let mut level = 1_u32;
    loop {
        let token = next_checked_token(input, stores, expansion, flags, macro_name, call_context)?;
        match traced_semantic_token(token) {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                level += 1;
                tokens.push(token);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                level -= 1;
                if level == 0 {
                    return Ok(());
                }
                tokens.push(token);
            }
            _ => tokens.push(token),
        }
    }
}

fn scan_delimited_argument(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    flags: MeaningFlags,
    macro_name: &str,
    delimiter: &[Token],
    call_context: TracedTokenWord,
) -> Result<TracedTokenList, MacroCallError> {
    let mut argument = Vec::new();
    let mut pending = VecDeque::new();
    let mut level = 0_u32;

    loop {
        let scanned = next_or_pending_token(
            input,
            stores,
            expansion,
            macro_name,
            call_context,
            &mut pending,
        )?;
        let token = traced_semantic_token(scanned.token);
        if level == 0 && token == delimiter[0] {
            let mut candidate = vec![scanned];
            let mut matched = true;
            for &expected in &delimiter[1..] {
                let next = next_or_pending_token(
                    input,
                    stores,
                    expansion,
                    macro_name,
                    call_context,
                    &mut pending,
                )?;
                candidate.push(next);
                if traced_semantic_token(next.token) != expected {
                    matched = false;
                    break;
                }
            }
            if matched {
                let stripped = strip_outer_group(&argument);
                return Ok(freeze_traced_tokens(stores, stripped));
            }
            push_argument_token(&mut argument, &mut level, candidate[0].token);
            let last_index = candidate.len() - 1;
            for (index, candidate_token) in candidate[1..].iter().enumerate().rev() {
                let was_matched_prefix = index + 1 < last_index;
                pending.push_front(PendingArgumentToken {
                    token: candidate_token.token,
                    allow_par: candidate_token.allow_par || was_matched_prefix,
                });
            }
            continue;
        }

        check_argument_par(stores, flags, macro_name, scanned)?;
        push_argument_token(&mut argument, &mut level, scanned.token);
    }
}

fn next_or_pending_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    macro_name: &str,
    call_context: TracedTokenWord,
    pending: &mut VecDeque<PendingArgumentToken>,
) -> Result<PendingArgumentToken, MacroCallError> {
    if let Some(token) = pending.pop_front() {
        Ok(token)
    } else {
        Ok(PendingArgumentToken {
            token: next_token_without_par_check(
                input,
                stores,
                expansion,
                macro_name,
                call_context,
            )?,
            allow_par: false,
        })
    }
}

fn check_argument_par(
    stores: &impl ExpansionState,
    flags: MeaningFlags,
    macro_name: &str,
    scanned: PendingArgumentToken,
) -> Result<(), MacroCallError> {
    if !scanned.allow_par
        && is_par_token(stores, traced_semantic_token(scanned.token))
        && !flags.contains(MeaningFlags::LONG)
    {
        return Err(MacroCallError::ParagraphEndedBeforeComplete {
            macro_name: macro_name.to_owned(),
            context: scanned.token,
        });
    }
    Ok(())
}

fn next_checked_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    flags: MeaningFlags,
    macro_name: &str,
    call_context: TracedTokenWord,
) -> Result<TracedTokenWord, MacroCallError> {
    let token = next_token_without_par_check(input, stores, expansion, macro_name, call_context)?;

    if is_par_token(stores, traced_semantic_token(token)) && !flags.contains(MeaningFlags::LONG) {
        return Err(MacroCallError::ParagraphEndedBeforeComplete {
            macro_name: macro_name.to_owned(),
            context: token,
        });
    }

    Ok(token)
}

fn next_token_without_par_check(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    macro_name: &str,
    call_context: TracedTokenWord,
) -> Result<TracedTokenWord, MacroCallError> {
    let token = crate::next_semantic_raw_token(input, stores)?.ok_or_else(|| {
        MacroCallError::EndOfInput {
            macro_name: macro_name.to_owned(),
            context: call_context,
        }
    })?;

    if let Token::Cs(symbol) = traced_semantic_token(token) {
        let meaning = stores.meaning(symbol);
        expansion.record_meaning(symbol, meaning);
        if let Meaning::Macro { flags, .. } = meaning
            && flags.contains(MeaningFlags::OUTER)
        {
            return Err(MacroCallError::ForbiddenOuterToken {
                macro_name: macro_name.to_owned(),
                context: token,
            });
        }
    }

    Ok(token)
}

fn push_argument_token(
    argument: &mut Vec<TracedTokenWord>,
    level: &mut u32,
    token: TracedTokenWord,
) {
    match traced_semantic_token(token) {
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => {
            *level += 1;
            argument.push(token);
        }
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } if *level > 0 => {
            *level -= 1;
            argument.push(token);
        }
        _ => argument.push(token),
    }
}

fn strip_outer_group(tokens: &[TracedTokenWord]) -> &[TracedTokenWord] {
    if tokens.len() < 2
        || !is_begin_group(traced_semantic_token(tokens[0]))
        || !is_end_group(traced_semantic_token(tokens[tokens.len() - 1]))
    {
        return tokens;
    }

    let mut level = 0_u32;
    for (index, &token) in tokens.iter().enumerate() {
        match traced_semantic_token(token) {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => level += 1,
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                level -= 1;
                if level == 0 && index != tokens.len() - 1 {
                    return tokens;
                }
            }
            _ => {}
        }
    }

    &tokens[1..tokens.len() - 1]
}

fn freeze_traced_tokens(
    stores: &mut tex_state::ExpansionContext<'_>,
    tokens: &[TracedTokenWord],
) -> TracedTokenList {
    stores.finish_traced_token_list(tokens)
}

fn traced_semantic_token(token: TracedTokenWord) -> Token {
    token
        .token()
        .expect("macro argument scanner received invalid traced token")
}

fn is_space_token(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space
        }
    )
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

fn is_par_token(stores: &impl ExpansionState, token: Token) -> bool {
    matches!(token, Token::Cs(symbol) if stores.symbol("par") == Some(symbol))
}

fn macro_name(stores: &impl ExpansionState, token: Token) -> String {
    match token {
        Token::Cs(_) => crate::values::token_text(stores, token),
        _ => format!("{token:?}"),
    }
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod tests;
