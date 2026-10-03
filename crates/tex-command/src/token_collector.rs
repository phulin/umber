//! Canonical in-progress token collection.
//!
//! `scan_toks` lists retain attempt-owned staging, while ordinary definitions
//! write into their selected semantic region through the same phase-tagged
//! in-progress owner. Macro arguments instead use their final invocation
//! region and stack-local cursor: their
//! brace, delimiter, range, and first-scan state never enters this generic
//! destination dispatch. Both consume the actual traced spelling; no classified
//! token facade or cached paragraph column is constructed.

use tex_state::interner::Symbol;
use tex_state::token::TracedTokenWord;

use crate::attempt::AttemptTokenBufferId;

/// Per-word rewrite applied only while an escaping general-text collector
/// writes its final replay owner.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ReplayWordTransform {
    Identity,
    Uppercase,
    Lowercase,
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
    },
    Definition {
        definition: tex_state::DefinitionBuildKey<G>,
    },
    /// A standalone escaping inserted list, written into the pending-insertion
    /// words that its input level later adopts.
    ReplayInput {
        /// Pending-insertion length when collection began.
        start: u32,
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
    /// Destination admission is separate from the collector's stack-local
    /// initialization. The production scanner initializes this owner once,
    /// then prepares this small field after fallible destination admission;
    /// no by-value collector result needs to cross that boundary.
    destination: Option<TokenCollectorDestination<G>>,
    phase: TokenCollectorPhase,
    cursor: crate::scanner_kernel::ScannerCursor,
    pending_parameter: Option<PendingParameter>,
    /// Episode-local direct writer for an ordinary definition. It is borrowed
    /// only for this synchronous scanner call; a resource miss unwinds the
    /// call and the host replays the full checkpoint instead of retaining the
    /// writer as retry state.
    definition_writer: Option<tex_state::DefinitionBuildWriter<G>>,
}

impl<G> Default for TokenCollector<G> {
    fn default() -> Self {
        Self {
            destination: None,
            phase: TokenCollectorPhase::Parameter,
            cursor: crate::scanner_kernel::ScannerCursor::default(),
            pending_parameter: None,
            definition_writer: None,
        }
    }
}

impl<G> TokenCollector<G> {
    fn prepare_destination(&mut self, destination: TokenCollectorDestination<G>) -> Result<(), ()> {
        if self.destination.is_some() {
            return Err(());
        }
        self.destination = Some(destination);
        Ok(())
    }

    pub(crate) fn prepare_token_buffers(&mut self, writer: AttemptTokenBufferId) -> Result<(), ()> {
        self.prepare_destination(TokenCollectorDestination::TokenBuffers { writer })
    }

    pub(crate) fn prepare_definition(
        &mut self,
        definition: tex_state::DefinitionBuildKey<G>,
    ) -> Result<(), ()> {
        self.prepare_destination(TokenCollectorDestination::Definition { definition })
    }

    pub(crate) fn prepare_replay_input(
        &mut self,
        start: u32,
        transform: ReplayWordTransform,
        observed: bool,
    ) -> Result<(), ()> {
        self.prepare_destination(TokenCollectorDestination::ReplayInput {
            start,
            transform,
            observed_source: (observed && transform != ReplayWordTransform::Identity)
                .then(Vec::new),
        })
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

    pub(crate) const fn prepared_destination(&self) -> Option<&TokenCollectorDestination<G>> {
        self.destination.as_ref()
    }

    pub(crate) const fn destination(&self) -> &TokenCollectorDestination<G> {
        match &self.destination {
            Some(destination) => destination,
            None => panic!("token collector destination is not prepared"),
        }
    }

    pub(crate) const fn destination_mut(&mut self) -> &mut TokenCollectorDestination<G> {
        match &mut self.destination {
            Some(destination) => destination,
            None => panic!("token collector destination is not prepared"),
        }
    }

    pub(crate) fn install_definition_writer(
        &mut self,
        writer: tex_state::DefinitionBuildWriter<G>,
    ) -> Result<(), ()> {
        if !matches!(
            self.destination(),
            TokenCollectorDestination::Definition { .. }
        ) || self.definition_writer.replace(writer).is_some()
        {
            return Err(());
        }
        Ok(())
    }

    pub(crate) fn definition_writer(&self) -> Option<&tex_state::DefinitionBuildWriter<G>> {
        self.definition_writer.as_ref()
    }

    pub(crate) fn definition_writer_mut(
        &mut self,
    ) -> Option<&mut tex_state::DefinitionBuildWriter<G>> {
        self.definition_writer.as_mut()
    }

    /// Lends the definition writer to a batched replacement run together with
    /// the balanced-body depth the run starts from. Runs are only possible
    /// while no `#` awaits its §479 continuation.
    pub(crate) fn replacement_run(
        &mut self,
    ) -> Option<(
        &mut tex_state::DefinitionBuildWriter<G>,
        &mut crate::scanner_kernel::ScannerCursor,
    )> {
        if self.phase != TokenCollectorPhase::Replacement || self.pending_parameter.is_some() {
            return None;
        }
        Some((self.definition_writer.as_mut()?, &mut self.cursor))
    }

    pub(crate) fn take_definition_writer(&mut self) -> Option<tex_state::DefinitionBuildWriter<G>> {
        self.definition_writer.take()
    }

    pub(crate) fn take_observed_source(
        &mut self,
    ) -> Option<Vec<crate::observation::ObservedToken>> {
        match self.destination_mut() {
            TokenCollectorDestination::ReplayInput {
                observed_source, ..
            } => observed_source.take(),
            TokenCollectorDestination::TokenBuffers { .. }
            | TokenCollectorDestination::Definition { .. } => None,
        }
    }

    /// Applies TeX82 §477's balanced-body brace state. The closing token which
    /// returns the depth to zero is the collector boundary and is not stored.
    pub(crate) fn settle_balanced_brace(&mut self, token: TracedTokenWord) -> Result<bool, ()> {
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
