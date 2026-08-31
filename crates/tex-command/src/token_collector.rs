//! Canonical in-progress token collection.
//!
//! Macro arguments and `scan_toks` have different final lifetimes: an
//! argument remains in execution scratch for its live activation, while a
//! scanned list or definition remains attempt-owned until durable
//! publication. They nevertheless share one in-progress owner and one raw
//! command classification. This module holds that owner; the two backing
//! lanes remain with the semantic lifetime which can reclaim them exactly.

use core::marker::PhantomData;

use tex_state::interner::Symbol;
use tex_state::token::{Catcode, TokenWord, TracedTokenWord};

use crate::attempt::{AttemptDefinitionId, AttemptTokenBufferId, AttemptTokenListId};
use crate::input::ReplayInputBuilderId;

/// Per-word rewrite applied only while an escaping general-text collector
/// writes its final replay owner.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ReplayWordTransform {
    Identity,
    Uppercase,
    Lowercase,
}

impl<G> crate::CommandProcessor<'_, '_, G> {
    /// Classifies one raw command once at the shared collector boundary.
    #[inline]
    pub(crate) fn classify_collector_token(
        &mut self,
        command: &crate::CurrentCommand<G>,
        paragraph_token: Option<TokenWord>,
    ) -> ClassifiedToken {
        #[cfg(test)]
        {
            self.command
                .token_collector_path_counters
                .raw_classifications += 1;
        }
        ClassifiedToken::from_command(command, paragraph_token)
    }
}

/// One raw delivered command classified once for every collector decision.
///
/// Token equality, group balance, parameter-number matching, and leading-space
/// decisions use the immutable spelling. TeX82's collectors make these
/// decisions from `cur_tok`; a control sequence whose resolved `cur_cmd` is a
/// character command remains a control-sequence token in every grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ClassifiedToken {
    word: TracedTokenWord,
    paragraph: bool,
}

const _: () = assert!(core::mem::size_of::<ClassifiedToken>() == 16);

impl ClassifiedToken {
    pub(crate) fn from_command<G>(
        command: &crate::CurrentCommand<G>,
        paragraph_token: Option<TokenWord>,
    ) -> Self {
        let word = command.spelling();
        Self {
            word,
            paragraph: Some(word.token_word()) == paragraph_token,
        }
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn from_word(word: TracedTokenWord, paragraph_token: Option<TokenWord>) -> Self {
        Self {
            word,
            paragraph: Some(word.token_word()) == paragraph_token,
        }
    }

    pub(crate) const fn word(&self) -> TracedTokenWord {
        self.word
    }

    pub(crate) const fn spelling(&self) -> TokenWord {
        self.word.token_word()
    }

    pub(crate) const fn spelling_is_begin_group(&self) -> bool {
        matches!(self.spelling().literal_catcode(), Some(Catcode::BeginGroup))
    }

    pub(crate) const fn spelling_is_end_group(&self) -> bool {
        matches!(self.spelling().literal_catcode(), Some(Catcode::EndGroup))
    }

    pub(crate) const fn spelling_is_space(&self) -> bool {
        matches!(self.spelling().literal_catcode(), Some(Catcode::Space))
    }

    pub(crate) const fn spelling_is_parameter(&self) -> bool {
        matches!(self.spelling().literal_catcode(), Some(Catcode::Parameter))
    }

    pub(crate) const fn rejects_non_long_paragraph(&self, paragraph_checked: bool) -> bool {
        paragraph_checked && self.paragraph
    }
}

/// Exact TeX82 §394 facts established while one argument is collected.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MacroArgumentFacts {
    rejects_non_long_paragraph: bool,
    removable_outer_group: bool,
}

impl MacroArgumentFacts {
    #[cfg(test)]
    pub(crate) const fn rejects_non_long_paragraph(self) -> bool {
        self.rejects_non_long_paragraph
    }

    pub(crate) const fn removable_outer_group(self) -> bool {
        self.removable_outer_group
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PendingArgumentFacts {
    rejects_non_long_paragraph: bool,
    word_count: u32,
    outer_group_candidate: bool,
}

impl PendingArgumentFacts {
    fn settle(&mut self, token: ClassifiedToken, paragraph_checked: bool, brace_depth_before: u32) {
        self.rejects_non_long_paragraph |= token.rejects_non_long_paragraph(paragraph_checked);
        if self.word_count == 0 {
            self.outer_group_candidate = token.spelling_is_begin_group();
        } else if brace_depth_before == 0 {
            self.outer_group_candidate = false;
        }
        self.word_count = self.word_count.saturating_add(1);
    }

    pub(crate) const fn seal(self, brace_depth: u32) -> MacroArgumentFacts {
        MacroArgumentFacts {
            rejects_non_long_paragraph: self.rejects_non_long_paragraph,
            removable_outer_group: self.outer_group_candidate && brace_depth == 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingParameter {
    pub(crate) hash: TracedTokenWord,
    pub(crate) highest: u8,
    pub(crate) target: Option<Symbol>,
}

/// One authoritative in-progress collection destination.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum TokenCollectorDestination<G> {
    MacroArgument {
        slot: u8,
        start: u32,
        end: u32,
        facts: PendingArgumentFacts,
        end_trim: u8,
        delimiter_start: usize,
        delimiter_head: usize,
        _generation: PhantomData<fn(&G) -> &G>,
    },
    TokenBuffers {
        writer: AttemptTokenBufferId,
        replacement: AttemptTokenBufferId,
        parameter_result: Option<AttemptTokenListId>,
    },
    Definition {
        definition: AttemptDefinitionId,
        writing_replacement: bool,
    },
    /// Final generation-owned storage for a standalone escaping inserted list.
    ReplayInput {
        builder: ReplayInputBuilderId<G>,
        transform: ReplayWordTransform,
        /// Observation-only source spelling for a non-identity transform.
        /// The ordinary unobserved path keeps no parallel word owner.
        observed_source: Option<Vec<crate::observation::ObservedToken>>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TokenCollectorPhase {
    Parameter,
    Replacement,
    Complete,
}

/// One caller-owned collector reused by macro matching, macro definitions,
/// balanced token lists, writes, token assignments, and read-token lists.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct TokenCollector<G> {
    destination: TokenCollectorDestination<G>,
    phase: TokenCollectorPhase,
    brace_depth: u32,
    pending_parameter: Option<PendingParameter>,
}

impl<G> TokenCollector<G> {
    pub(crate) fn macro_argument(slot: u8, start: u32, delimiter_start: usize) -> Self {
        Self {
            destination: TokenCollectorDestination::MacroArgument {
                slot,
                start,
                end: start,
                facts: PendingArgumentFacts::default(),
                end_trim: 0,
                delimiter_start,
                delimiter_head: delimiter_start,
                _generation: PhantomData,
            },
            phase: TokenCollectorPhase::Replacement,
            brace_depth: 0,
            pending_parameter: None,
        }
    }

    pub(crate) fn token_buffers(
        parameter: AttemptTokenBufferId,
        replacement: AttemptTokenBufferId,
    ) -> Self {
        Self {
            destination: TokenCollectorDestination::TokenBuffers {
                writer: parameter,
                replacement,
                parameter_result: None,
            },
            phase: TokenCollectorPhase::Parameter,
            brace_depth: 0,
            pending_parameter: None,
        }
    }

    pub(crate) fn definition(definition: AttemptDefinitionId) -> Self {
        Self {
            destination: TokenCollectorDestination::Definition {
                definition,
                writing_replacement: false,
            },
            phase: TokenCollectorPhase::Parameter,
            brace_depth: 0,
            pending_parameter: None,
        }
    }

    pub(crate) fn replay_input(
        builder: ReplayInputBuilderId<G>,
        transform: ReplayWordTransform,
        observed: bool,
    ) -> Self {
        Self {
            destination: TokenCollectorDestination::ReplayInput {
                builder,
                transform,
                observed_source: (observed && transform != ReplayWordTransform::Identity)
                    .then(Vec::new),
            },
            phase: TokenCollectorPhase::Parameter,
            brace_depth: 0,
            pending_parameter: None,
        }
    }

    pub(crate) const fn phase(&self) -> TokenCollectorPhase {
        self.phase
    }

    pub(crate) fn begin_replacement(&mut self) -> Result<(), ()> {
        if self.phase != TokenCollectorPhase::Parameter {
            return Err(());
        }
        self.phase = TokenCollectorPhase::Replacement;
        self.brace_depth = 1;
        Ok(())
    }

    pub(crate) fn complete(&mut self) -> Result<(), ()> {
        if self.phase != TokenCollectorPhase::Replacement || self.pending_parameter.is_some() {
            return Err(());
        }
        self.phase = TokenCollectorPhase::Complete;
        Ok(())
    }

    pub(crate) const fn destination(&self) -> &TokenCollectorDestination<G> {
        &self.destination
    }

    pub(crate) const fn destination_mut(&mut self) -> &mut TokenCollectorDestination<G> {
        &mut self.destination
    }

    pub(crate) fn take_observed_source(
        &mut self,
    ) -> Option<Vec<crate::observation::ObservedToken>> {
        match &mut self.destination {
            TokenCollectorDestination::ReplayInput {
                observed_source, ..
            } => observed_source.take(),
            TokenCollectorDestination::MacroArgument { .. }
            | TokenCollectorDestination::TokenBuffers { .. }
            | TokenCollectorDestination::Definition { .. } => None,
        }
    }

    pub(crate) const fn brace_depth(&self) -> u32 {
        self.brace_depth
    }

    pub(crate) fn settle_argument_facts(
        &mut self,
        token: ClassifiedToken,
        paragraph_checked: bool,
    ) -> Result<u32, ()> {
        let TokenCollectorDestination::MacroArgument { facts, .. } = &mut self.destination else {
            return Err(());
        };
        facts.settle(token, paragraph_checked, self.brace_depth);
        self.advance_brace_depth(token);
        Ok(self.brace_depth)
    }

    /// Applies TeX82 §477's balanced-body brace state. The closing token which
    /// returns the depth to zero is the collector boundary and is not stored.
    pub(crate) fn settle_balanced_brace(&mut self, token: ClassifiedToken) -> Result<bool, ()> {
        if self.phase != TokenCollectorPhase::Replacement {
            return Err(());
        }
        if token.spelling_is_begin_group() {
            self.brace_depth = self.brace_depth.saturating_add(1);
        } else if token.spelling_is_end_group() && self.brace_depth != 0 {
            self.brace_depth -= 1;
        }
        Ok(token.spelling_is_end_group() && self.brace_depth == 0)
    }

    fn advance_brace_depth(&mut self, token: ClassifiedToken) {
        if token.spelling_is_begin_group() {
            self.brace_depth = self.brace_depth.saturating_add(1);
        } else if token.spelling_is_end_group() && self.brace_depth != 0 {
            self.brace_depth -= 1;
        }
    }

    pub(crate) fn set_pending_parameter(&mut self, pending: PendingParameter) -> Result<(), ()> {
        if self.pending_parameter.replace(pending).is_some() {
            return Err(());
        }
        Ok(())
    }

    pub(crate) fn take_pending_parameter(&mut self) -> Option<PendingParameter> {
        self.pending_parameter.take()
    }

    pub(crate) fn argument_facts(&self) -> Result<MacroArgumentFacts, ()> {
        let TokenCollectorDestination::MacroArgument { facts, .. } = self.destination else {
            return Err(());
        };
        Ok(facts.seal(self.brace_depth))
    }

    pub(crate) fn strip_argument_outer_group(&mut self) -> Result<(), ()> {
        let TokenCollectorDestination::MacroArgument {
            start,
            end,
            end_trim,
            ..
        } = &mut self.destination
        else {
            return Err(());
        };
        let collected = end
            .checked_sub(*start)
            .and_then(|len| len.checked_sub(u32::from(*end_trim)))
            .ok_or(())?;
        if collected < 2 {
            return Err(());
        }
        *start = start.checked_add(1).ok_or(())?;
        *end_trim = end_trim.checked_add(1).ok_or(())?;
        Ok(())
    }
}
