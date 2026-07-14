use std::fmt;

use tex_lex::{InputSource, InputStack};
use tex_state::{InputRecordId, InputSummary, Snapshot, SourceId, Universe};

use crate::{ExecError, ModeNest, ModeNestSummary};

/// In-memory schema version for aggregate engine checkpoints.
///
/// Version 2 names the component-framed state-hash schema explicitly.
pub const ENGINE_CHECKPOINT_SCHEMA_VERSION: u32 = 2;

/// A safe point at which the outer executor can publish restartable state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EngineBoundary {
    JobStart,
    OuterParagraphEnd,
    ShipoutComplete,
}

/// One restartable, aggregate engine checkpoint.
///
/// Checkpoints can only be constructed by an [`EngineSession`]. Their state
/// roots are intentionally private so a caller cannot forge a boundary.
#[derive(Clone, Debug)]
pub struct EngineCheckpoint {
    schema_version: u32,
    boundary: EngineBoundary,
    universe: Snapshot,
    input: InputSummary,
    modes: ModeNestSummary,
    state_hash: u64,
}

impl EngineCheckpoint {
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub const fn boundary(&self) -> EngineBoundary {
        self.boundary
    }

    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.state_hash
    }

    #[must_use]
    pub const fn input_summary(&self) -> &InputSummary {
        &self.input
    }

    #[must_use]
    pub const fn mode_summary(&self) -> &ModeNestSummary {
        &self.modes
    }
}

/// Receives checkpoints synchronously as the outer executor reaches boundaries.
pub trait CheckpointSink {
    /// Whether this sink wants a checkpoint captured at `boundary`.
    ///
    /// The default preserves checkpoint delivery for existing sinks. Sinks
    /// that decline a boundary avoid all input, mode, snapshot, and semantic
    /// hash construction for it.
    fn wants_checkpoint(&self, _boundary: EngineBoundary) -> bool {
        true
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint);
}

impl CheckpointSink for Vec<EngineCheckpoint> {
    fn checkpoint(&mut self, checkpoint: EngineCheckpoint) {
        self.push(checkpoint);
    }
}

#[derive(Debug, Default)]
pub(crate) struct NoopCheckpointSink;

impl CheckpointSink for NoopCheckpointSink {
    fn wants_checkpoint(&self, _boundary: EngineBoundary) -> bool {
        false
    }

    fn checkpoint(&mut self, _checkpoint: EngineCheckpoint) {}
}

/// Capability held by one outer executor run to publish named checkpoints.
///
/// Keeping capture here makes recursive scanners, alignments, box/math
/// builders, output routines, and nested shipouts structurally unable to
/// publish durable continuation state.
pub(crate) struct EngineSession<'a, C> {
    sink: &'a mut C,
    mode_projection: Option<(ModeNestSummary, u64)>,
}

impl<'a, C: CheckpointSink> EngineSession<'a, C> {
    pub(crate) fn new(sink: &'a mut C) -> Self {
        Self {
            sink,
            mode_projection: None,
        }
    }

    pub(crate) fn publish<S: InputSource>(
        &mut self,
        boundary: EngineBoundary,
        nest: &ModeNest,
        input: &mut InputStack<S>,
        universe: &mut Universe,
    ) {
        if !self.sink.wants_checkpoint(boundary) {
            return;
        }
        let input_summary = input.publication_summary(universe);
        universe.set_input_summary(input_summary.clone());
        let modes = nest.summary();
        let mode_hash = match &self.mode_projection {
            Some((cached, fingerprint)) if cached.shares_root_with(&modes) => *fingerprint,
            _ => {
                let fingerprint = modes.semantic_fingerprint(universe);
                self.mode_projection = Some((modes.clone(), fingerprint));
                fingerprint
            }
        };
        let universe = universe.snapshot();
        let state_hash = combine_mode_hash(universe.state_hash(), mode_hash);
        self.sink.checkpoint(EngineCheckpoint {
            schema_version: ENGINE_CHECKPOINT_SCHEMA_VERSION,
            boundary,
            universe,
            input: input_summary,
            modes,
            state_hash,
        });
    }
}

/// Failure to restore an engine checkpoint.
#[derive(Debug)]
pub enum EngineRestoreError<E> {
    Input(E),
    Mode(ExecError),
}

impl<E: fmt::Display> fmt::Display for EngineRestoreError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => write!(f, "could not reopen checkpoint input: {error}"),
            Self::Mode(error) => write!(f, "could not restore checkpoint mode nest: {error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for EngineRestoreError<E> {}

impl crate::Executor {
    /// Restores every engine-owned root from a published checkpoint.
    pub fn restore_checkpoint<S, E, F>(
        &mut self,
        input: &mut InputStack<S>,
        universe: &mut Universe,
        checkpoint: &EngineCheckpoint,
        reopen_source: F,
    ) -> Result<(), EngineRestoreError<E>>
    where
        S: InputSource,
        F: FnMut(SourceId, Option<InputRecordId>, &tex_state::SourceFrameSummary) -> Result<S, E>,
    {
        let restored_input = InputStack::from_summary(&checkpoint.input, reopen_source)
            .map_err(EngineRestoreError::Input)?;
        let restored_modes =
            ModeNest::from_summary(checkpoint.modes.clone()).map_err(EngineRestoreError::Mode)?;
        universe.rollback(&checkpoint.universe);
        *input = restored_input;
        self.nest = restored_modes;
        Ok(())
    }
}

fn combine_mode_hash(universe_hash: u64, mode_hash: u64) -> u64 {
    universe_hash.rotate_left(17) ^ mode_hash
}
