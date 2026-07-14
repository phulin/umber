use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tex_lex::{InputSource, InputStack, MemoryInput, WorldInput};
use tex_state::source_map::SourceMapError;
use tex_state::{
    GenerationForkError, GenerationSubstrate, InputRecordId, InputSummary, Snapshot, SourceId,
    Universe,
};

use crate::{ExecError, ModeNest, ModeNestSummary};

/// In-memory schema version for aggregate engine checkpoints.
///
/// Version 3 uses revision-independent absolute coordinates for the editor root.
pub const ENGINE_CHECKPOINT_SCHEMA_VERSION: u32 = 3;

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
    root_anchor: usize,
    effect_prefix: usize,
    artifact_prefix: usize,
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

    #[must_use]
    pub const fn artifact_prefix_len(&self) -> usize {
        self.artifact_prefix
    }

    #[must_use]
    pub const fn effect_prefix_len(&self) -> usize {
        self.effect_prefix
    }

    #[must_use]
    pub const fn root_anchor(&self) -> usize {
        self.root_anchor
    }

    /// Rehomes revision-relative root metadata after a validated convergence
    /// match while adopting the owner-exact state snapshot by reference.
    pub fn rehome_converged_root(
        &self,
        substrate: &GenerationSubstrate,
        source: &str,
        mapped_anchor: usize,
    ) -> Result<Self, GenerationForkError> {
        substrate.validate_checkpoint_snapshot(&self.universe)?;
        if mapped_anchor > source.len() || !source.is_char_boundary(mapped_anchor) {
            return Err(GenerationForkError::InvalidMappedAnchor);
        }
        let mut checkpoint = self.clone();
        checkpoint.root_anchor = mapped_anchor;
        Ok(checkpoint)
    }

    /// Retargets an inherited prefix checkpoint onto a promoted fork after
    /// the state layer proves it lies at or below that fork's exact anchor.
    pub fn retarget_prefix(
        &self,
        target: &GenerationSubstrate,
        source: &GenerationSubstrate,
    ) -> Result<Self, GenerationForkError> {
        let mut checkpoint = self.clone();
        checkpoint.universe = target.retarget_prefix_from(source, &self.universe)?;
        Ok(checkpoint)
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

    /// Stops execution immediately after the last delivered checkpoint.
    fn stop_requested(&self) -> bool {
        false
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

    pub(crate) fn publish(
        &mut self,
        boundary: EngineBoundary,
        nest: &ModeNest,
        input: &mut InputStack,
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
        let effect_prefix = usize::try_from(universe.world().effect_pos().raw())
            .expect("effect log position must fit in memory address space");
        let artifact_prefix = universe.world().artifact_commits().len();
        let root_anchor = input_summary.conservative_root_position();
        let universe = universe.snapshot();
        let state_hash = combine_mode_hash(universe.state_hash(), mode_hash);
        self.sink.checkpoint(EngineCheckpoint {
            schema_version: ENGINE_CHECKPOINT_SCHEMA_VERSION,
            boundary,
            universe,
            input: input_summary,
            modes,
            state_hash,
            root_anchor,
            effect_prefix,
            artifact_prefix,
        });
    }

    pub(crate) fn stop_requested(&self) -> bool {
        self.sink.stop_requested()
    }
}

/// Failure to restore an engine checkpoint.
#[derive(Debug)]
pub enum EngineRestoreError<E> {
    Input(E),
    Mode(ExecError),
}

/// Failure to atomically restore and rebind an editor checkpoint.
#[derive(Debug)]
pub enum EditorRestoreError {
    Fork(GenerationForkError),
    RootRebind(SourceMapError),
    IncludedInputUnavailable(SourceId),
    Mode(ExecError),
}

impl fmt::Display for EditorRestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fork(error) => write!(f, "could not fork retained generation: {error}"),
            Self::RootRebind(error) => write!(f, "could not rebind editor root: {error}"),
            Self::IncludedInputUnavailable(source) => write!(
                f,
                "included generated source {} cannot be reopened",
                source.raw()
            ),
            Self::Mode(error) => write!(f, "could not restore checkpoint mode nest: {error}"),
        }
    }
}

impl std::error::Error for EditorRestoreError {}

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
    pub fn restore_checkpoint<E, F, T>(
        &mut self,
        input: &mut InputStack,
        universe: &mut Universe,
        checkpoint: &EngineCheckpoint,
        reopen_source: F,
    ) -> Result<(), EngineRestoreError<E>>
    where
        F: FnMut(SourceId, Option<InputRecordId>, &tex_state::SourceFrameSummary) -> Result<T, E>,
        T: InputSource + 'static,
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

    /// Restores a retained checkpoint while substituting only its root editor
    /// buffer. Preparation happens on a fork, so failure cannot partially
    /// mutate the live executor, input stack, or Universe.
    #[allow(clippy::disallowed_methods)] // Diagnostic latency; no engine fact observes it.
    pub fn restore_editor_checkpoint(
        &mut self,
        input: &mut InputStack,
        universe: &mut Universe,
        substrate: &GenerationSubstrate,
        checkpoint: &EngineCheckpoint,
        source: &str,
    ) -> Result<Duration, EditorRestoreError> {
        let fork_started = Instant::now();
        let mut restored_universe = substrate
            .fork_at(&checkpoint.universe)
            .map_err(EditorRestoreError::Fork)?;
        let fork_latency = fork_started.elapsed();
        let (summary, root_source) = restored_universe
            .rebind_root_editor_input(
                &checkpoint.input,
                Arc::from(source.as_bytes()),
                checkpoint.root_anchor,
            )
            .map_err(EditorRestoreError::RootRebind)?;
        let restored_modes =
            ModeNest::from_summary(checkpoint.modes.clone()).map_err(EditorRestoreError::Mode)?;
        let restored_input = InputStack::from_summary(&summary, |source_id, record, frame| {
            if source_id == root_source {
                return Ok::<Box<dyn InputSource>, EditorRestoreError>(Box::new(
                    MemoryInput::from_offset(source, checkpoint.root_anchor),
                ));
            }
            let Some(record) = record else {
                return Err(EditorRestoreError::IncludedInputUnavailable(source_id));
            };
            let content = restored_universe
                .world()
                .recorded_input_content(record)
                .ok_or(EditorRestoreError::IncludedInputUnavailable(source_id))?;
            Ok(Box::new(WorldInput::from_content_at_offset(
                content,
                frame.next_source_offset(),
            )))
        })?;
        *universe = restored_universe;
        *input = restored_input;
        self.nest = restored_modes;
        Ok(fork_latency)
    }
}

fn combine_mode_hash(universe_hash: u64, mode_hash: u64) -> u64 {
    universe_hash.rotate_left(17) ^ mode_hash
}
