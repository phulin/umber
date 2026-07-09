//! Macro-call argument matching.
//!
//! This is the TeX gullet scanner for macro parameter text. It consumes the
//! call-site input, freezes matched arguments through `Universe`, and leaves body
//! replay/substitution to the expansion-frame work.

use std::collections::VecDeque;
use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, MACRO_ARGUMENT_SLOTS, MacroArguments};
use tex_state::ExpansionState;
use tex_state::ids::TokenListId;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::token::{Catcode, Token};

use crate::{NoopRecorder, ReadRecorder};

/// Frozen arguments matched for one macro call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatchedArguments {
    arguments: Vec<TokenListId>,
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
            arguments.set((index + 1) as u8, id);
        }
        arguments
    }

    fn push(&mut self, id: TokenListId) {
        self.arguments.push(id);
    }
}

/// Errors raised while matching a macro call.
#[derive(Debug)]
pub enum MacroCallError {
    Lex(LexError),
    EndOfInput { macro_name: String },
    DoesNotMatchDefinition { macro_name: String },
    ParagraphEndedBeforeComplete { macro_name: String },
    ForbiddenOuterToken { macro_name: String },
}

impl fmt::Display for MacroCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => write!(f, "{err}"),
            Self::EndOfInput { macro_name } => {
                write!(f, "File ended while scanning use of {macro_name}")
            }
            Self::DoesNotMatchDefinition { macro_name } => {
                write!(f, "Use of {macro_name} doesn't match its definition")
            }
            Self::ParagraphEndedBeforeComplete { macro_name } => {
                write!(f, "Paragraph ended before {macro_name} was complete")
            }
            Self::ForbiddenOuterToken { macro_name } => {
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParameterSpec {
    delimiter: Vec<Token>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParameterPattern {
    leading: Vec<Token>,
    specs: Vec<ParameterSpec>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingArgumentToken {
    token: Token,
    allow_par: bool,
}

/// Matches one macro call and freezes each argument token list.
pub fn match_macro_call<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    call_token: Token,
    meaning: MacroMeaning,
) -> Result<MatchedArguments, MacroCallError>
where
    S: InputSource,
{
    match_macro_call_with_recorder(input, stores, &mut NoopRecorder, call_token, meaning)
}

pub(crate) fn match_macro_call_with_recorder<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    call_token: Token,
    meaning: MacroMeaning,
) -> Result<MatchedArguments, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let macro_name = macro_name(stores, call_token);
    let pattern = parse_parameter_text(stores.tokens(meaning.parameter_text()));
    match_exact_tokens(
        input,
        stores,
        recorder,
        meaning.flags(),
        &macro_name,
        &pattern.leading,
    )?;

    let mut matched = MatchedArguments::default();
    for spec in &pattern.specs {
        let id = if spec.delimiter.is_empty() {
            scan_undelimited_argument(input, stores, recorder, meaning.flags(), &macro_name)?
        } else {
            scan_delimited_argument(
                input,
                stores,
                recorder,
                meaning.flags(),
                &macro_name,
                &spec.delimiter,
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

fn match_exact_tokens<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
    expected: &[Token],
) -> Result<(), MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    for &expected_token in expected {
        let token = next_checked_token(input, stores, recorder, flags, macro_name)?;
        if token != expected_token {
            return Err(MacroCallError::DoesNotMatchDefinition {
                macro_name: macro_name.to_owned(),
            });
        }
    }
    Ok(())
}

fn scan_undelimited_argument<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
) -> Result<TokenListId, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let mut token = next_checked_token(input, stores, recorder, flags, macro_name)?;
    while is_space_token(token) {
        token = next_checked_token(input, stores, recorder, flags, macro_name)?;
    }

    let mut tokens = Vec::new();
    if is_begin_group(token) {
        scan_balanced_group(input, stores, recorder, flags, macro_name, &mut tokens)?;
    } else {
        tokens.push(token);
    }
    Ok(freeze_tokens(stores, &tokens))
}

fn scan_balanced_group<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
    tokens: &mut Vec<Token>,
) -> Result<(), MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let mut level = 1_u32;
    loop {
        let token = next_checked_token(input, stores, recorder, flags, macro_name)?;
        match token {
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

fn scan_delimited_argument<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    flags: MeaningFlags,
    macro_name: &str,
    delimiter: &[Token],
) -> Result<TokenListId, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let mut argument = Vec::new();
    let mut pending = VecDeque::new();
    let mut level = 0_u32;

    loop {
        let scanned = next_or_pending_token(input, stores, recorder, macro_name, &mut pending)?;
        let token = scanned.token;
        if level == 0 && token == delimiter[0] {
            let mut candidate = vec![scanned];
            let mut matched = true;
            for &expected in &delimiter[1..] {
                let next =
                    next_or_pending_token(input, stores, recorder, macro_name, &mut pending)?;
                candidate.push(next);
                if next.token != expected {
                    matched = false;
                    break;
                }
            }
            if matched {
                let stripped = strip_outer_group(&argument);
                return Ok(freeze_tokens(stores, stripped));
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
        push_argument_token(&mut argument, &mut level, token);
    }
}

fn next_or_pending_token<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    macro_name: &str,
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
            token: next_token_without_par_check(input, stores, recorder, macro_name)?,
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
        && is_par_token(stores, scanned.token)
        && !flags.contains(MeaningFlags::LONG)
    {
        return Err(MacroCallError::ParagraphEndedBeforeComplete {
            macro_name: macro_name.to_owned(),
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
) -> Result<Token, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let token = next_token_without_par_check(input, stores, recorder, macro_name)?;

    if is_par_token(stores, token) && !flags.contains(MeaningFlags::LONG) {
        return Err(MacroCallError::ParagraphEndedBeforeComplete {
            macro_name: macro_name.to_owned(),
        });
    }

    Ok(token)
}

fn next_token_without_par_check<S, R>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    macro_name: &str,
) -> Result<Token, MacroCallError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let token = input
        .next_token(stores)?
        .ok_or_else(|| MacroCallError::EndOfInput {
            macro_name: macro_name.to_owned(),
        })?;

    if let Token::Cs(symbol) = token {
        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);
        if let Meaning::Macro { flags, .. } = meaning
            && flags.contains(MeaningFlags::OUTER)
        {
            return Err(MacroCallError::ForbiddenOuterToken {
                macro_name: macro_name.to_owned(),
            });
        }
    }

    Ok(token)
}

fn push_argument_token(argument: &mut Vec<Token>, level: &mut u32, token: Token) {
    match token {
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

fn strip_outer_group(tokens: &[Token]) -> &[Token] {
    if tokens.len() < 2 || !is_begin_group(tokens[0]) || !is_end_group(tokens[tokens.len() - 1]) {
        return tokens;
    }

    let mut level = 0_u32;
    for (index, &token) in tokens.iter().enumerate() {
        match token {
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

fn freeze_tokens(stores: &mut impl ExpansionState, tokens: &[Token]) -> TokenListId {
    let mut builder = stores.token_list_builder();
    for &token in tokens {
        builder.push(token);
    }
    stores.finish_token_list(&mut builder)
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
        Token::Cs(symbol) => format!("\\{}", stores.resolve(symbol)),
        _ => format!("{token:?}"),
    }
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod tests;
