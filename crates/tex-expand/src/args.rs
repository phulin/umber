//! Macro-call argument matching.
//!
//! This is the TeX gullet scanner for macro parameter text. It consumes the
//! call-site input into transient packed buffers and leaves body replay and
//! substitution to the expansion-frame work.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use tex_lex::{InputStack, LexError, MACRO_ARGUMENT_SLOTS, MacroArguments};
use tex_state::ExpansionState;
use tex_state::MacroArgumentRange;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::ExpansionContext;

/// Packed transient arguments matched for one macro call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatchedArguments {
    tokens: Vec<TracedTokenWord>,
    slots: [Option<MacroArgumentRange>; MACRO_ARGUMENT_SLOTS],
    len: usize,
}

impl MatchedArguments {
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn get(&self, slot: u8) -> Option<&[TracedTokenWord]> {
        let index = usize::from(slot.checked_sub(1)?);
        let range = self.slots.get(index).copied().flatten()?;
        Some(&self.tokens[range.start()..range.start() + range.len()])
    }

    #[must_use]
    pub fn into_macro_arguments(self) -> MacroArguments {
        assert!(
            self.len <= MACRO_ARGUMENT_SLOTS,
            "macro calls support only #1 through #9"
        );
        MacroArguments::from_parts(self.tokens, self.slots)
    }

    fn push(&mut self, range: MacroArgumentRange) {
        assert!(self.len < MACRO_ARGUMENT_SLOTS);
        self.slots[self.len] = Some(range);
        self.len += 1;
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

#[derive(Clone, Copy)]
struct MacroCallContext {
    flags: MeaningFlags,
    call_token: TracedTokenWord,
}

impl MacroCallContext {
    fn macro_name(self, stores: &impl ExpansionState) -> String {
        macro_name(stores, traced_semantic_token(self.call_token))
    }
}

/// Matches one macro call into a single transient packed-token buffer.
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
    let context = MacroCallContext {
        flags: meaning.flags(),
        call_token,
    };
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
    match_exact_tokens(input, stores, expansion, context, &pattern.leading)?;

    let mut matched = MatchedArguments {
        tokens: input.take_transient_token_buffer(),
        ..MatchedArguments::default()
    };
    let result = (|| {
        for spec in &pattern.specs {
            let range = if spec.delimiter.is_empty() {
                scan_undelimited_argument(input, stores, expansion, context, &mut matched.tokens)?
            } else {
                scan_delimited_argument(
                    input,
                    stores,
                    expansion,
                    context,
                    spec,
                    &mut matched.tokens,
                )?
            };
            matched.push(range);
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(matched),
        Err(error) => {
            input.recycle_transient_token_buffer(std::mem::take(&mut matched.tokens));
            Err(error)
        }
    }
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
    context: MacroCallContext,
    expected: &[Token],
) -> Result<(), MacroCallError> {
    for &expected_token in expected {
        let token = next_checked_token(input, stores, expansion, context)?;
        if traced_semantic_token(token) != expected_token {
            return Err(MacroCallError::DoesNotMatchDefinition {
                macro_name: context.macro_name(stores),
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
    context: MacroCallContext,
    tokens: &mut Vec<TracedTokenWord>,
) -> Result<MacroArgumentRange, MacroCallError> {
    let mut token = next_checked_token(input, stores, expansion, context)?;
    while is_space_token(traced_semantic_token(token)) {
        token = next_checked_token(input, stores, expansion, context)?;
    }

    let start = tokens.len();
    if is_begin_group(traced_semantic_token(token)) {
        scan_balanced_group(input, stores, expansion, context, tokens)?;
    } else {
        tokens.push(token);
    }
    Ok(MacroArgumentRange::new(start, tokens.len() - start))
}

fn scan_balanced_group(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: MacroCallContext,
    tokens: &mut Vec<TracedTokenWord>,
) -> Result<(), MacroCallError> {
    let mut level = 1_u32;
    loop {
        let token = next_checked_token(input, stores, expansion, context)?;
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
    context: MacroCallContext,
    spec: &ParameterSpec,
    argument: &mut Vec<TracedTokenWord>,
) -> Result<MacroArgumentRange, MacroCallError> {
    let delimiter = &spec.delimiter;
    let start = argument.len();
    let mut pending = VecDeque::new();
    let mut level = 0_u32;

    loop {
        let scanned = next_or_pending_token(input, stores, expansion, context, &mut pending)?;
        let token = traced_semantic_token(scanned.token);
        if level == 0 && token == delimiter[0] {
            let mut candidate = vec![scanned];
            let mut matched = true;
            for &expected in &delimiter[1..] {
                let next = next_or_pending_token(input, stores, expansion, context, &mut pending)?;
                candidate.push(next);
                if traced_semantic_token(next.token) != expected {
                    matched = false;
                    break;
                }
            }
            if matched {
                let len = argument.len() - start;
                let stripped_len = strip_outer_group(&argument[start..]).len();
                if stripped_len != len {
                    argument.remove(start);
                    argument.pop();
                }
                return Ok(MacroArgumentRange::new(start, argument.len() - start));
            }
            push_argument_token(argument, &mut level, candidate[0].token);
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

        check_argument_par(stores, context, scanned)?;
        push_argument_token(argument, &mut level, scanned.token);
    }
}

fn next_or_pending_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: MacroCallContext,
    pending: &mut VecDeque<PendingArgumentToken>,
) -> Result<PendingArgumentToken, MacroCallError> {
    if let Some(token) = pending.pop_front() {
        Ok(token)
    } else {
        Ok(PendingArgumentToken {
            token: next_token_without_par_check(input, stores, expansion, context)?,
            allow_par: false,
        })
    }
}

fn check_argument_par(
    stores: &impl ExpansionState,
    context: MacroCallContext,
    scanned: PendingArgumentToken,
) -> Result<(), MacroCallError> {
    if !scanned.allow_par
        && is_par_token(stores, traced_semantic_token(scanned.token))
        && !context.flags.contains(MeaningFlags::LONG)
    {
        return Err(MacroCallError::ParagraphEndedBeforeComplete {
            macro_name: context.macro_name(stores),
            context: scanned.token,
        });
    }
    Ok(())
}

fn next_checked_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: MacroCallContext,
) -> Result<TracedTokenWord, MacroCallError> {
    let token = next_token_without_par_check(input, stores, expansion, context)?;

    if is_par_token(stores, traced_semantic_token(token))
        && !context.flags.contains(MeaningFlags::LONG)
    {
        return Err(MacroCallError::ParagraphEndedBeforeComplete {
            macro_name: context.macro_name(stores),
            context: token,
        });
    }

    Ok(token)
}

fn next_token_without_par_check(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: MacroCallContext,
) -> Result<TracedTokenWord, MacroCallError> {
    let token = crate::next_semantic_raw_token(input, stores)?.ok_or_else(|| {
        MacroCallError::EndOfInput {
            macro_name: context.macro_name(stores),
            context: context.call_token,
        }
    })?;

    if let Token::Cs(symbol) = traced_semantic_token(token) {
        let meaning = stores.meaning(symbol);
        expansion.record_meaning(symbol, meaning);
        if let Meaning::Macro { flags, .. } = meaning
            && flags.contains(MeaningFlags::OUTER)
        {
            return Err(MacroCallError::ForbiddenOuterToken {
                macro_name: context.macro_name(stores),
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
