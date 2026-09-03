//! Canonical in-progress token collection.
//!
//! `scan_toks` lists retain attempt-owned staging, while ordinary definitions
//! write into their selected semantic region through the same phase-tagged
//! in-progress owner. Macro arguments instead use the purpose-built
//! `ExecutionScratch` writer: their
//! brace, delimiter, range, and first-scan state never enters this generic
//! destination dispatch. Both paths reuse `ClassifiedToken` without decoding
//! a delivered command's spelling twice.

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingParameter {
    pub(crate) hash: TracedTokenWord,
    pub(crate) highest: u8,
    pub(crate) target: Option<Symbol>,
}

/// One authoritative in-progress collection destination.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum TokenCollectorDestination<G> {
    TokenBuffers {
        writer: AttemptTokenBufferId,
        replacement: AttemptTokenBufferId,
        parameter_result: Option<AttemptTokenListId>,
    },
    Definition {
        definition: tex_state::DefinitionBuildKey<G>,
    },
    AttemptDefinition {
        definition: AttemptDefinitionId,
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
    cursor: crate::scanner_kernel::ScannerCursor,
    pending_parameter: Option<PendingParameter>,
}

impl<G> TokenCollector<G> {
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
            cursor: crate::scanner_kernel::ScannerCursor::default(),
            pending_parameter: None,
        }
    }

    pub(crate) fn definition(definition: tex_state::DefinitionBuildKey<G>) -> Self {
        Self {
            destination: TokenCollectorDestination::Definition { definition },
            phase: TokenCollectorPhase::Parameter,
            cursor: crate::scanner_kernel::ScannerCursor::default(),
            pending_parameter: None,
        }
    }

    pub(crate) fn attempt_definition(definition: AttemptDefinitionId) -> Self {
        Self {
            destination: TokenCollectorDestination::AttemptDefinition { definition },
            phase: TokenCollectorPhase::Parameter,
            cursor: crate::scanner_kernel::ScannerCursor::default(),
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
            cursor: crate::scanner_kernel::ScannerCursor::default(),
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
        self.cursor.open_balanced_body();
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
            TokenCollectorDestination::TokenBuffers { .. }
            | TokenCollectorDestination::Definition { .. }
            | TokenCollectorDestination::AttemptDefinition { .. } => None,
        }
    }

    /// Applies TeX82 §477's balanced-body brace state. The closing token which
    /// returns the depth to zero is the collector boundary and is not stored.
    pub(crate) fn settle_balanced_brace(&mut self, token: ClassifiedToken) -> Result<bool, ()> {
        if self.phase != TokenCollectorPhase::Replacement {
            return Err(());
        }
        Ok(self.cursor.settle_balanced(token))
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
}
