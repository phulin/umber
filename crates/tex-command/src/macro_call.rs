//! Private canonical scalar macro-call state machine.
#![allow(dead_code)] // expansion dispatch is the next ordered integration slice
use std::sync::Arc;

use tex_state::ids::MacroDefinitionId;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::input::SharedTokenBuffer;
use crate::processor::status::{ArgumentBuilderId, MatchingContext, ScannerStatus, ScannerWarning};
use crate::{CommandError, CommandProcessor};

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, MacroRecord, TokenListRecord,
};

/// Persistent ownership of live macro-argument activations.
///
/// This is the sole owner of the activation chain. Macro-body input behavior
/// carries a typed activation identity, while parameter payloads retain shared
/// ownership of the one contiguous argument allocation.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ParameterState {
    pub(crate) activations: Vec<MacroActivation>,
    pub(crate) next_activation_identity: u64,
}

/// Typed identity of one live macro activation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct MacroActivationId(pub(crate) u64);

/// One live macro call and the materialized arguments it owns.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroActivation {
    pub(crate) identity: MacroActivationId,
    pub(crate) definition: MacroDefinitionId,
    pub(crate) arguments: MacroArguments,
    pub(crate) invocation: OriginId,
}

/// One contiguous macro-argument allocation and its at-most-nine ranges.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct MacroArguments {
    pub(crate) buffer: SharedTokenBuffer,
    pub(crate) ranges: [Option<MacroArgumentRange>; 9],
}

/// Incremental construction of one canonical macro activation.
///
/// The scalar matcher completes arguments in definition order. Each completed
/// argument is appended once to this one buffer; its slot records only a
/// half-open range, so parameter replay never duplicates argument tokens.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct MacroArgumentBuilder {
    tokens: Vec<TracedTokenWord>,
    ranges: [Option<MacroArgumentRange>; 9],
    next_slot: u8,
}

/// A malformed attempt to finish one macro argument.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MacroArgumentBuildError {
    InvalidSlot(u8),
    OutOfOrderSlot { expected: u8, actual: u8 },
}

/// Compact interpretation of a parameter-related replacement token.
///
/// `Token::Param` is the canonical compact out-parameter representation.
/// A literal parameter character reaches a replacement list only through
/// TeX's `##` escape, and remains an ordinary character during replay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MacroParameterEscape {
    OutParameter(u8),
    EscapedParameter,
}

impl MacroParameterEscape {
    pub(crate) const fn classify(token: Token) -> Option<Self> {
        match token {
            Token::Param(slot) => Some(Self::OutParameter(slot)),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            } => Some(Self::EscapedParameter),
            _ => None,
        }
    }
}

/// A half-open range within a macro activation's shared argument buffer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroArgumentRange {
    start: usize,
    end: usize,
}

impl MacroArgumentRange {
    pub(crate) const fn new(start: usize, end: usize) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    pub(crate) const fn start(self) -> usize {
        self.start
    }

    pub(crate) const fn end(self) -> usize {
        self.end
    }
}

impl MacroArgumentBuilder {
    /// Completes the next argument in canonical definition order.
    pub(crate) fn complete(
        &mut self,
        slot: u8,
        argument: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<(), MacroArgumentBuildError> {
        if !(1..=9).contains(&slot) {
            return Err(MacroArgumentBuildError::InvalidSlot(slot));
        }
        let expected = self.next_slot + 1;
        if slot != expected {
            return Err(MacroArgumentBuildError::OutOfOrderSlot {
                expected,
                actual: slot,
            });
        }
        let start = self.tokens.len();
        self.tokens.extend(argument);
        let end = self.tokens.len();
        self.ranges[usize::from(slot - 1)] = MacroArgumentRange::new(start, end);
        self.next_slot = slot;
        Ok(())
    }

    /// Freezes the single shared argument allocation for one activation.
    #[must_use]
    pub(crate) fn finish(self) -> MacroArguments {
        MacroArguments {
            buffer: SharedTokenBuffer::new(Arc::from(self.tokens)),
            ranges: self.ranges,
        }
    }
}

impl ParameterState {
    /// Installs the sole owner of a macro activation before its body level is
    /// pushed. The caller immediately associates the returned identity with
    /// `TokenBehavior::MacroBody`, making retirement an atomic ownership pair.
    pub(crate) fn push_activation(
        &mut self,
        definition: MacroDefinitionId,
        arguments: MacroArguments,
        invocation: OriginId,
    ) -> MacroActivationId {
        let identity = MacroActivationId(self.next_activation_identity);
        self.next_activation_identity = self.next_activation_identity.wrapping_add(1);
        self.activations.push(MacroActivation {
            identity,
            definition,
            arguments,
            invocation,
        });
        identity
    }

    pub(crate) fn parent_invocation(&self) -> OriginId {
        self.activations
            .last()
            .map(|activation| activation.invocation)
            .unwrap_or(OriginId::UNKNOWN)
    }
}

impl CommandProcessor<'_> {
    /// TeX.web's scalar `macro_call` path for compulsory parameter text,
    /// literal argument matching, and replacement activation.
    pub(crate) fn macro_call(
        &mut self,
        call: crate::CurrentCommand,
    ) -> Result<MacroArguments, CommandError> {
        let Meaning::Macro { definition, .. } = call.meaning() else {
            return Err(CommandError::InputInvariant);
        };
        let macro_name = call
            .control_sequence()
            .ok_or(CommandError::InputInvariant)?;
        let meaning = self.state.macro_definition(definition);
        let pattern = self.state.macro_definition_parameter_pattern(definition);
        let builder = ArgumentBuilderId(self.command.transient.next_builder_identity);
        self.command.transient.next_builder_identity =
            self.command.transient.next_builder_identity.wrapping_add(1);
        let status = ScannerStatus::Matching(MatchingContext {
            macro_name,
            builder,
            // The diagnostic/provenance bridge assigns stable warning ids in
            // its own ordered slice; matching nevertheless owns a typed live
            // warning slot now so outer recovery has one canonical path.
            warning: ScannerWarning(0),
        });
        let prior = self.command.begin_scanner_status(status);
        self.observe_scanner_status(true);
        self.outer_recovered_while_matching = false;
        let arguments = match self.macro_call_scalar(definition, meaning.flags(), &pattern) {
            Ok(arguments) => arguments,
            Err(error) => {
                self.observe_scanner_status(false);
                self.command.restore_scanner_status(prior);
                return Err(error);
            }
        };

        // TeX.web §§391--400 freezes the completed ranges before replacing
        // the input. The activation owns that one shared buffer; its body
        // replays the canonical immutable replacement list and resolves
        // compact `OutParameter` tokens through that owner.
        let provenance = self.state.macro_definition_provenance(definition);
        let _level = self.push_macro_activation(
            definition,
            call.spelling().origin(),
            arguments.clone(),
            meaning.replacement_text(),
            provenance.replacement_origins(),
        );
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Push,
            reason: InputReason::Macro,
            level: _level.0,
            position: 0,
        }));
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Macro(MacroRecord {
            activation: true,
            definition: u64::from(definition.raw()),
            control_sequence: Some(self.state.resolve(macro_name).to_owned()),
            argument: Some(pattern.parameter_count() as u8),
            token_count: arguments.buffer.len() as u64,
            tokens: Vec::new(),
        }));
        self.observe_scanner_status(false);
        self.command.restore_scanner_status(prior);
        Ok(arguments)
    }

    fn macro_call_scalar(
        &mut self,
        _definition: MacroDefinitionId,
        flags: MeaningFlags,
        pattern: &tex_state::macro_store::MacroParameterPattern,
    ) -> Result<MacroArguments, CommandError> {
        for expected in pattern.leading() {
            let actual = self.get_token()?.ok_or(CommandError::MacroPrefixMismatch)?;
            if self.outer_recovered_while_matching
                || actual.spelling().semantic_token() != *expected
            {
                return Err(if self.outer_recovered_while_matching {
                    CommandError::OuterInMacroArgument
                } else {
                    CommandError::MacroPrefixMismatch
                });
            }
        }

        let mut arguments = MacroArgumentBuilder::default();
        for parameter in 0..pattern.parameter_count() {
            let delimiter = pattern.delimiter(parameter);
            let argument = if delimiter.is_empty() {
                self.scan_undelimited_argument(flags)?
            } else {
                self.scan_delimited_argument(flags, delimiter)?
            };
            #[cfg(any(test, feature = "instrumentation"))]
            self.observe(CommandObservation::TokenList(TokenListRecord {
                transition: "splice",
                purpose: "macro_delimiter_match",
                tokens: delimiter
                    .iter()
                    .copied()
                    .map(|token| {
                        self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN))
                    })
                    .collect(),
            }));
            #[cfg(any(test, feature = "instrumentation"))]
            self.observe(CommandObservation::Macro(MacroRecord {
                activation: false,
                definition: u64::from(_definition.raw()),
                control_sequence: None,
                argument: Some((parameter + 1) as u8),
                token_count: argument.len() as u64,
                tokens: argument
                    .iter()
                    .copied()
                    .map(|token| self.observed_token(token))
                    .collect(),
            }));
            arguments
                .complete((parameter + 1) as u8, argument)
                .map_err(|_| CommandError::InputInvariant)?;
        }
        Ok(arguments.finish())
    }

    fn scan_undelimited_argument(
        &mut self,
        flags: MeaningFlags,
    ) -> Result<Vec<TracedTokenWord>, CommandError> {
        let first = loop {
            let command = self
                .get_token()?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            if self.outer_recovered_while_matching {
                return Err(CommandError::OuterInMacroArgument);
            }
            if matches!(
                command.spelling().semantic_token(),
                Token::Char {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                continue;
            }
            break command;
        };
        self.check_argument_paragraph(&first, flags)?;
        if !matches!(
            first.spelling().semantic_token(),
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            }
        ) {
            return Ok(vec![first.spelling()]);
        }

        let mut depth = 1_u32;
        let mut tokens = Vec::new();
        loop {
            let command = self
                .get_token()?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            if self.outer_recovered_while_matching {
                return Err(CommandError::OuterInMacroArgument);
            }
            self.check_argument_paragraph(&command, flags)?;
            match command.spelling().semantic_token() {
                Token::Char {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    depth += 1;
                    tokens.push(command.spelling());
                }
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                } => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(tokens);
                    }
                    tokens.push(command.spelling());
                }
                _ => tokens.push(command.spelling()),
            }
        }
    }

    /// TeX.web §394's literal, scalar delimiter matcher. A failed delimiter
    /// prefix is committed to the argument one token at a time, then the
    /// mismatching token is considered again as a possible new prefix. This
    /// is intentionally not a compiled string matcher: token catcodes and
    /// brace depth are semantic here.
    fn scan_delimited_argument(
        &mut self,
        flags: MeaningFlags,
        delimiter: &[Token],
    ) -> Result<Vec<TracedTokenWord>, CommandError> {
        debug_assert!(!delimiter.is_empty());
        let mut tokens = Vec::new();
        let mut prefix = Vec::with_capacity(delimiter.len());
        let mut depth = 0_u32;
        let mut current = None;

        loop {
            let command = match current.take() {
                Some(command) => command,
                None => self
                    .get_token()?
                    .ok_or(CommandError::ParagraphInMacroArgument)?,
            };
            if self.outer_recovered_while_matching {
                return Err(CommandError::OuterInMacroArgument);
            }
            let token = command.spelling().semantic_token();

            if depth == 0 && token == delimiter[prefix.len()] {
                prefix.push(command.spelling());
                if prefix.len() == delimiter.len() {
                    // `#{` consumes the opening brace as parameter text. Raw
                    // delivery has accounted for it, but no replacement-body
                    // replay exists yet to provide the balancing delivery.
                    if is_begin_group(token) {
                        self.undo_delimiter_begin_group_delivery();
                    }
                    return Ok(strip_one_outer_group(tokens));
                }
                continue;
            }

            if !prefix.is_empty() {
                for prefix_token in prefix.drain(..) {
                    push_delimited_argument_token(&mut tokens, &mut depth, prefix_token);
                }
                // A mismatching token can itself start an overlapping prefix.
                current = Some(command);
                continue;
            }

            self.check_argument_paragraph(&command, flags)?;
            push_delimited_argument_token(&mut tokens, &mut depth, command.spelling());
        }
    }

    fn check_argument_paragraph(
        &self,
        command: &crate::CurrentCommand,
        flags: MeaningFlags,
    ) -> Result<(), CommandError> {
        if matches!(
            command.meaning(),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)
        ) && !flags.contains(MeaningFlags::LONG)
        {
            return Err(CommandError::ParagraphInMacroArgument);
        }
        Ok(())
    }
}

fn push_delimited_argument_token(
    tokens: &mut Vec<TracedTokenWord>,
    depth: &mut u32,
    token: TracedTokenWord,
) {
    match token.semantic_token() {
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => *depth = depth.saturating_add(1),
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } if *depth > 0 => *depth -= 1,
        _ => {}
    }
    tokens.push(token);
}

fn strip_one_outer_group(mut tokens: Vec<TracedTokenWord>) -> Vec<TracedTokenWord> {
    if tokens.len() < 2
        || !is_begin_group(tokens[0].semantic_token())
        || !is_end_group(tokens[tokens.len() - 1].semantic_token())
    {
        return tokens;
    }

    let mut depth = 0_u32;
    for (index, token) in tokens.iter().enumerate() {
        match token.semantic_token() {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => depth = depth.saturating_add(1),
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } if depth > 0 => {
                depth -= 1;
                if depth == 0 && index + 1 != tokens.len() {
                    return tokens;
                }
            }
            _ => {}
        }
    }
    if depth == 0 {
        tokens.pop();
        tokens.remove(0);
    }
    tokens
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

#[cfg(test)]
mod tests;
