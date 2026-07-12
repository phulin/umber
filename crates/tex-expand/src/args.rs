//! Macro-call argument matching.
//!
//! This is the TeX gullet scanner for macro parameter text. It consumes the
//! call-site input, freezes matched arguments through `Universe`, and leaves body
//! replay/substitution to the expansion-frame work.

#[cfg(test)]
use std::collections::VecDeque;
use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, MACRO_ARGUMENT_SLOTS, MacroArguments};
use tex_state::ExpansionState;
use tex_state::TracedTokenList;
use tex_state::ids::MacroDefinitionId;
use tex_state::ids::TokenListId;
#[cfg(test)]
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

#[cfg(test)]
use crate::NoopRecorder;
use crate::ReadRecorder;

#[derive(Clone, Debug)]
pub(crate) struct ResumedMacroCall {
    pub definition: MacroDefinitionId,
    pub call_context: TracedTokenWord,
    pub arguments: MatchedArguments,
}

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

    #[cfg(test)]
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
struct ParameterPattern {
    leading: Vec<Token>,
    specs: Vec<ParameterSpec>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingArgumentToken {
    token: TracedTokenWord,
    allow_par: bool,
}

/// Matches one macro call and freezes each argument token list.
#[cfg(test)]
pub(crate) fn match_macro_call<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    call_token: TracedTokenWord,
    meaning: MacroMeaning,
) -> Result<MatchedArguments, MacroCallError>
where
    S: InputSource,
{
    match_macro_call_with_recorder(input, stores, &mut NoopRecorder, call_token, meaning)
}

#[cfg(test)]
fn match_macro_call_with_recorder<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    call_token: TracedTokenWord,
    meaning: MacroMeaning,
) -> Result<MatchedArguments, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let macro_name = macro_name(stores, traced_semantic_token(call_token));
    let pattern = parse_parameter_text(stores.tokens(meaning.parameter_text()));
    match_exact_tokens(
        input,
        stores,
        recorder,
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
                recorder,
                meaning.flags(),
                &macro_name,
                call_token,
            )?
        } else {
            scan_delimited_argument(
                input,
                stores,
                recorder,
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

pub(crate) fn match_rooted_macro_call<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    call_context: TracedTokenWord,
    definition: MacroDefinitionId,
) -> Result<ResumedMacroCall, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    input.push_gullet_continuation(tex_lex::GulletContinuationSummary::MacroCall(
        tex_state::MacroCallContinuationSummary {
            definition,
            call_context,
            matched: Vec::new(),
            phase: tex_state::MacroCallPhaseSummary::Leading { index: 0 },
        },
    ));
    let result = resume_macro_call(input, stores, recorder);
    if result.is_err() {
        let _ = input.pop_gullet_continuation();
    }
    result
}

pub(crate) fn resume_macro_call<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
) -> Result<ResumedMacroCall, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    loop {
        let call = current_macro_call(input);
        let meaning = stores.macro_definition(call.definition);
        let pattern = parse_parameter_text(stores.tokens(meaning.parameter_text()));
        let flags = meaning.flags();
        let macro_name = macro_name(stores, traced_semantic_token(call.call_context));

        match call.phase {
            tex_state::MacroCallPhaseSummary::Leading { index } => {
                if index == pattern.leading.len() {
                    set_macro_call_phase(
                        input,
                        tex_state::MacroCallPhaseSummary::ArgumentStart { spec_index: 0 },
                    );
                    continue;
                }
                let token = next_checked_token(
                    input,
                    stores,
                    recorder,
                    flags,
                    &macro_name,
                    call.call_context,
                )?;
                if traced_semantic_token(token) != pattern.leading[index] {
                    return Err(MacroCallError::DoesNotMatchDefinition {
                        macro_name,
                        context: token,
                    });
                }
                set_macro_call_phase(
                    input,
                    tex_state::MacroCallPhaseSummary::Leading { index: index + 1 },
                );
            }
            tex_state::MacroCallPhaseSummary::ArgumentStart { spec_index } => {
                if spec_index == pattern.specs.len() {
                    return Ok(finish_macro_call(input));
                }
                let phase = if pattern.specs[spec_index].delimiter.is_empty() {
                    tex_state::MacroCallPhaseSummary::UndelimitedSkip { spec_index }
                } else {
                    tex_state::MacroCallPhaseSummary::Delimited {
                        spec_index,
                        level: 0,
                        argument: Vec::new(),
                        pending: Vec::new(),
                    }
                };
                set_macro_call_phase(input, phase);
            }
            tex_state::MacroCallPhaseSummary::UndelimitedSkip { spec_index } => {
                let token = next_checked_token(
                    input,
                    stores,
                    recorder,
                    flags,
                    &macro_name,
                    call.call_context,
                )?;
                if is_space_token(traced_semantic_token(token)) {
                    continue;
                }
                if is_begin_group(traced_semantic_token(token)) {
                    set_macro_call_phase(
                        input,
                        tex_state::MacroCallPhaseSummary::UndelimitedGroup {
                            spec_index,
                            level: 1,
                            tokens: Vec::new(),
                        },
                    );
                } else {
                    push_rooted_argument(input, stores, &[token]);
                    set_macro_call_phase(
                        input,
                        tex_state::MacroCallPhaseSummary::ArgumentStart {
                            spec_index: spec_index + 1,
                        },
                    );
                }
            }
            tex_state::MacroCallPhaseSummary::UndelimitedGroup {
                spec_index,
                mut level,
                mut tokens,
            } => {
                let token = next_checked_token(
                    input,
                    stores,
                    recorder,
                    flags,
                    &macro_name,
                    call.call_context,
                )?;
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
                            push_rooted_argument(input, stores, &tokens);
                            set_macro_call_phase(
                                input,
                                tex_state::MacroCallPhaseSummary::ArgumentStart {
                                    spec_index: spec_index + 1,
                                },
                            );
                            continue;
                        }
                        tokens.push(token);
                    }
                    _ => tokens.push(token),
                }
                set_macro_call_phase(
                    input,
                    tex_state::MacroCallPhaseSummary::UndelimitedGroup {
                        spec_index,
                        level,
                        tokens,
                    },
                );
            }
            tex_state::MacroCallPhaseSummary::Delimited {
                spec_index,
                mut level,
                mut argument,
                mut pending,
            } => {
                let scanned = next_rooted_pending_or_raw(
                    input,
                    stores,
                    recorder,
                    &macro_name,
                    call.call_context,
                    &mut pending,
                )?;
                let token = traced_semantic_token(scanned.token);
                if level == 0 && token == pattern.specs[spec_index].delimiter[0] {
                    set_macro_call_phase(
                        input,
                        tex_state::MacroCallPhaseSummary::DelimiterCandidate {
                            spec_index,
                            level,
                            argument,
                            pending,
                            candidate: vec![scanned],
                            next_delimiter_index: 1,
                        },
                    );
                    continue;
                }
                check_rooted_argument_par(stores, flags, &macro_name, scanned)?;
                push_argument_token(&mut argument, &mut level, scanned.token);
                set_macro_call_phase(
                    input,
                    tex_state::MacroCallPhaseSummary::Delimited {
                        spec_index,
                        level,
                        argument,
                        pending,
                    },
                );
            }
            tex_state::MacroCallPhaseSummary::DelimiterCandidate {
                spec_index,
                mut level,
                mut argument,
                mut pending,
                mut candidate,
                next_delimiter_index,
            } => {
                let delimiter = &pattern.specs[spec_index].delimiter;
                if next_delimiter_index == delimiter.len() {
                    push_rooted_argument(input, stores, strip_outer_group(&argument));
                    set_macro_call_phase(
                        input,
                        tex_state::MacroCallPhaseSummary::ArgumentStart {
                            spec_index: spec_index + 1,
                        },
                    );
                    continue;
                }
                let next = next_rooted_pending_or_raw(
                    input,
                    stores,
                    recorder,
                    &macro_name,
                    call.call_context,
                    &mut pending,
                )?;
                candidate.push(next);
                if traced_semantic_token(next.token) == delimiter[next_delimiter_index] {
                    set_macro_call_phase(
                        input,
                        tex_state::MacroCallPhaseSummary::DelimiterCandidate {
                            spec_index,
                            level,
                            argument,
                            pending,
                            candidate,
                            next_delimiter_index: next_delimiter_index + 1,
                        },
                    );
                    continue;
                }

                push_argument_token(&mut argument, &mut level, candidate[0].token);
                let last_index = candidate.len() - 1;
                for (index, candidate_token) in candidate[1..].iter().enumerate().rev() {
                    let was_matched_prefix = index + 1 < last_index;
                    pending.insert(
                        0,
                        tex_state::PendingMacroTokenSummary {
                            token: candidate_token.token,
                            allow_par: candidate_token.allow_par || was_matched_prefix,
                        },
                    );
                }
                set_macro_call_phase(
                    input,
                    tex_state::MacroCallPhaseSummary::Delimited {
                        spec_index,
                        level,
                        argument,
                        pending,
                    },
                );
            }
        }
    }
}

fn current_macro_call<S>(input: &InputStack<S>) -> tex_state::MacroCallContinuationSummary {
    match input.current_gullet_continuation() {
        Some(tex_lex::GulletContinuationSummary::MacroCall(call)) => call.clone(),
        _ => panic!("macro-call resume requires a rooted continuation"),
    }
}

fn set_macro_call_phase<S>(input: &mut InputStack<S>, phase: tex_state::MacroCallPhaseSummary) {
    let Some(tex_lex::GulletContinuationSummary::MacroCall(call)) =
        input.current_gullet_continuation_mut()
    else {
        panic!("macro-call update requires a rooted continuation");
    };
    call.phase = phase;
}

fn push_rooted_argument<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    tokens: &[TracedTokenWord],
) {
    let argument = freeze_traced_tokens(stores, tokens);
    let Some(tex_lex::GulletContinuationSummary::MacroCall(call)) =
        input.current_gullet_continuation_mut()
    else {
        panic!("macro argument requires a rooted continuation");
    };
    call.matched.push(argument);
}

fn finish_macro_call<S>(input: &mut InputStack<S>) -> ResumedMacroCall {
    let Some(tex_lex::GulletContinuationSummary::MacroCall(call)) = input.pop_gullet_continuation()
    else {
        panic!("macro-call completion requires a rooted continuation");
    };
    ResumedMacroCall {
        definition: call.definition,
        call_context: call.call_context,
        arguments: MatchedArguments {
            arguments: call.matched,
        },
    }
}

fn next_rooted_pending_or_raw<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    macro_name: &str,
    call_context: TracedTokenWord,
    pending: &mut Vec<tex_state::PendingMacroTokenSummary>,
) -> Result<tex_state::PendingMacroTokenSummary, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    if !pending.is_empty() {
        return Ok(pending.remove(0));
    }
    Ok(tex_state::PendingMacroTokenSummary {
        token: next_token_without_par_check(input, stores, recorder, macro_name, call_context)?,
        allow_par: false,
    })
}

fn check_rooted_argument_par(
    stores: &impl ExpansionState,
    flags: MeaningFlags,
    macro_name: &str,
    scanned: tex_state::PendingMacroTokenSummary,
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

#[cfg(test)]
fn match_exact_tokens<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
    expected: &[Token],
    call_context: TracedTokenWord,
) -> Result<(), MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    for &expected_token in expected {
        let token = next_checked_token(input, stores, recorder, flags, macro_name, call_context)?;
        if traced_semantic_token(token) != expected_token {
            return Err(MacroCallError::DoesNotMatchDefinition {
                macro_name: macro_name.to_owned(),
                context: token,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
fn scan_undelimited_argument<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
    call_context: TracedTokenWord,
) -> Result<TracedTokenList, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let mut token = next_checked_token(input, stores, recorder, flags, macro_name, call_context)?;
    while is_space_token(traced_semantic_token(token)) {
        token = next_checked_token(input, stores, recorder, flags, macro_name, call_context)?;
    }

    let mut tokens = Vec::new();
    if is_begin_group(traced_semantic_token(token)) {
        scan_balanced_group(
            input,
            stores,
            recorder,
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

#[cfg(test)]
fn scan_balanced_group<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
    call_context: TracedTokenWord,
    tokens: &mut Vec<TracedTokenWord>,
) -> Result<(), MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let mut level = 1_u32;
    loop {
        let token = next_checked_token(input, stores, recorder, flags, macro_name, call_context)?;
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

#[cfg(test)]
fn scan_delimited_argument<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
    delimiter: &[Token],
    call_context: TracedTokenWord,
) -> Result<TracedTokenList, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let mut argument = Vec::new();
    let mut pending = VecDeque::new();
    let mut level = 0_u32;

    loop {
        let scanned = next_or_pending_token(
            input,
            stores,
            recorder,
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
                    recorder,
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

#[cfg(test)]
fn next_or_pending_token<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    macro_name: &str,
    call_context: TracedTokenWord,
    pending: &mut VecDeque<PendingArgumentToken>,
) -> Result<PendingArgumentToken, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    if let Some(token) = pending.pop_front() {
        Ok(token)
    } else {
        Ok(PendingArgumentToken {
            token: next_token_without_par_check(input, stores, recorder, macro_name, call_context)?,
            allow_par: false,
        })
    }
}

#[cfg(test)]
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

fn next_checked_token<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
    call_context: TracedTokenWord,
) -> Result<TracedTokenWord, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let token = next_token_without_par_check(input, stores, recorder, macro_name, call_context)?;

    if is_par_token(stores, traced_semantic_token(token)) && !flags.contains(MeaningFlags::LONG) {
        return Err(MacroCallError::ParagraphEndedBeforeComplete {
            macro_name: macro_name.to_owned(),
            context: token,
        });
    }

    Ok(token)
}

fn next_token_without_par_check<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    macro_name: &str,
    call_context: TracedTokenWord,
) -> Result<TracedTokenWord, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let token = crate::next_semantic_raw_token(input, stores)?.ok_or_else(|| {
        MacroCallError::EndOfInput {
            macro_name: macro_name.to_owned(),
            context: call_context,
        }
    })?;

    if let Token::Cs(symbol) = traced_semantic_token(token) {
        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);
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
    stores: &mut impl ExpansionState,
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
