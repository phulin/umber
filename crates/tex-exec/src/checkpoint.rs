use std::fmt;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use tex_lex::{InputSource, InputStack, LayoutCursor, MemoryInput, WorldInput};
use tex_state::source_map::SourceMapError;
use tex_state::{
    ContentHash, FragmentStore, GenerationForkError, GenerationSubstrate, InputRecordId,
    InputSummary, Snapshot, SourceId, Universe,
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
    root_content_hash: Option<tex_state::ContentHash>,
    effect_prefix: usize,
    artifact_prefix: usize,
}

impl EngineCheckpoint {
    /// Verifies that this checkpoint still names restorable roots in `substrate`.
    #[doc(hidden)]
    pub fn validate_retained_by(
        &self,
        substrate: &GenerationSubstrate,
    ) -> Result<(), GenerationForkError> {
        substrate.validate_checkpoint_snapshot(&self.universe)
    }

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

    /// Returns true only when both checkpoints carry a strong canonical store
    /// identity and all remaining future-relevant roots compare exactly.
    #[must_use]
    pub fn exact_future_state_matches(&self, other: &Self) -> bool {
        self.boundary == other.boundary
            && self.universe.exact_future_state_matches(&other.universe)
            && self.modes == other.modes
    }

    /// Returns whether this checkpoint already carries the optional strong
    /// identity used by exact suffix-adoption comparisons.
    #[doc(hidden)]
    #[must_use]
    pub fn has_exact_state_identity(&self) -> bool {
        self.universe.has_exact_state_identity()
    }

    /// Computes the optional strong identity for an already retained
    /// checkpoint without changing its restart roots.
    #[doc(hidden)]
    pub fn with_exact_state_identity(
        &self,
        substrate: &GenerationSubstrate,
    ) -> Result<Self, GenerationForkError> {
        let mut checkpoint = self.clone();
        checkpoint.universe = substrate.snapshot_with_exact_identity(&self.universe)?;
        Ok(checkpoint)
    }

    /// Rehomes revision-relative root metadata after a validated convergence
    /// match while adopting the owner-exact state snapshot by reference.
    pub fn rehome_converged_root(
        &self,
        substrate: &GenerationSubstrate,
        old_source: &str,
        new_source: &str,
        mapped_anchor: usize,
    ) -> Result<Self, GenerationForkError> {
        substrate.validate_checkpoint_snapshot(&self.universe)?;
        if self.root_content_hash != Some(tex_state::ContentHash::from_bytes(old_source.as_bytes()))
        {
            return Err(GenerationForkError::RootRevisionMismatch);
        }
        if mapped_anchor > new_source.len() || !new_source.is_char_boundary(mapped_anchor) {
            return Err(GenerationForkError::InvalidMappedAnchor);
        }
        if self.root_anchor > old_source.len()
            || old_source.as_bytes()[self.root_anchor..] != new_source.as_bytes()[mapped_anchor..]
        {
            return Err(GenerationForkError::ChangedRootInterval);
        }
        let mut checkpoint = self.clone();
        checkpoint.root_anchor = mapped_anchor;
        checkpoint.root_content_hash =
            Some(tex_state::ContentHash::from_bytes(new_source.as_bytes()));
        Ok(checkpoint)
    }

    pub fn rehome_unchanged_prefix(
        &self,
        substrate: &GenerationSubstrate,
        old_source: &str,
        new_source: &str,
    ) -> Result<Self, GenerationForkError> {
        substrate.validate_checkpoint_snapshot(&self.universe)?;
        if self.root_content_hash != Some(tex_state::ContentHash::from_bytes(old_source.as_bytes()))
        {
            return Err(GenerationForkError::RootRevisionMismatch);
        }
        if self.root_anchor > old_source.len()
            || self.root_anchor > new_source.len()
            || old_source.as_bytes()[..self.root_anchor]
                != new_source.as_bytes()[..self.root_anchor]
        {
            return Err(GenerationForkError::ChangedRootInterval);
        }
        let mut checkpoint = self.clone();
        checkpoint.root_content_hash =
            Some(tex_state::ContentHash::from_bytes(new_source.as_bytes()));
        Ok(checkpoint)
    }

    /// Retargets an inherited prefix checkpoint onto a promoted fork after
    /// the state layer proves it lies at or below that fork's exact anchor.
    pub fn retarget_prefix(
        &self,
        target: &GenerationSubstrate,
        source: &GenerationSubstrate,
        old_source: &str,
        new_source: &str,
    ) -> Result<Self, GenerationForkError> {
        if self.root_content_hash != Some(tex_state::ContentHash::from_bytes(old_source.as_bytes()))
        {
            return Err(GenerationForkError::RootRevisionMismatch);
        }
        if self.root_anchor > old_source.len()
            || self.root_anchor > new_source.len()
            || old_source.as_bytes()[..self.root_anchor]
                != new_source.as_bytes()[..self.root_anchor]
        {
            return Err(GenerationForkError::ChangedRootInterval);
        }
        let mut checkpoint = self.clone();
        checkpoint.universe = target.retarget_prefix_from(source, &self.universe)?;
        checkpoint.root_content_hash =
            Some(tex_state::ContentHash::from_bytes(new_source.as_bytes()));
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

    /// Whether this sink needs strong canonical identities for optional exact
    /// suffix adoption. Ordinary checkpoint consumers leave this false and
    /// retain O(1) state snapshots.
    fn wants_exact_state_identity(&self, _boundary: EngineBoundary, _root_anchor: usize) -> bool {
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
        let artifact_prefix = universe.world().artifact_pos();
        let root_anchor = input_summary.conservative_root_position();
        let root_content_hash = universe.root_editor_content_hash(&input_summary);
        let universe = if self.sink.wants_exact_state_identity(boundary, root_anchor) {
            universe.snapshot_with_exact_identity()
        } else {
            universe.snapshot()
        };
        let state_hash = combine_mode_hash(universe.state_hash(), mode_hash);
        self.sink.checkpoint(EngineCheckpoint {
            schema_version: ENGINE_CHECKPOINT_SCHEMA_VERSION,
            boundary,
            universe,
            input: input_summary,
            modes,
            state_hash,
            root_anchor,
            root_content_hash,
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
    Layout(tex_state::EditorLayoutError),
    RootRevisionMismatch,
    ChangedRootPrefix,
    RootRebind(SourceMapError),
    IncludedInputUnavailable(SourceId),
    Mode(ExecError),
}

impl fmt::Display for EditorRestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fork(error) => write!(f, "could not fork retained generation: {error}"),
            Self::Layout(error) => write!(f, "could not install editor layout: {error}"),
            Self::RootRevisionMismatch => {
                f.write_str("checkpoint root revision does not match the accepted source")
            }
            Self::ChangedRootPrefix => {
                f.write_str("edited source changed bytes before the restart anchor")
            }
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
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::disallowed_methods)] // Diagnostic latency; no engine fact observes it.
    pub fn restore_editor_checkpoint(
        &mut self,
        input: &mut InputStack,
        universe: &mut Universe,
        substrate: &GenerationSubstrate,
        checkpoint: &EngineCheckpoint,
        old_source: &str,
        source: &str,
        fragments: &FragmentStore,
        layout: &tex_state::EditorLayout,
        layout_cursor: LayoutCursor,
    ) -> Result<Duration, EditorRestoreError> {
        if checkpoint.root_content_hash
            != Some(tex_state::ContentHash::from_bytes(old_source.as_bytes()))
        {
            return Err(EditorRestoreError::RootRevisionMismatch);
        }
        if checkpoint.root_anchor > old_source.len()
            || checkpoint.root_anchor > source.len()
            || old_source.as_bytes()[..checkpoint.root_anchor]
                != source.as_bytes()[..checkpoint.root_anchor]
        {
            return Err(EditorRestoreError::ChangedRootPrefix);
        }
        let fork_started = Timer::start();
        let mut restored_universe = substrate
            .fork_at(&checkpoint.universe)
            .map_err(EditorRestoreError::Fork)?;
        let fork_latency = fork_started.elapsed();
        restored_universe
            .install_editor_fragments(fragments, layout)
            .map_err(EditorRestoreError::Layout)?;
        let (summary, root_source) = restored_universe
            .rebind_root_editor_layout(&checkpoint.input, source.as_bytes(), checkpoint.root_anchor)
            .map_err(EditorRestoreError::RootRebind)?;
        let restored_modes =
            ModeNest::from_summary(checkpoint.modes.clone()).map_err(EditorRestoreError::Mode)?;
        let mut restored_input = InputStack::from_summary(&summary, |source_id, record, frame| {
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
        let installed_root = restored_input
            .install_root_layout_cursor(layout_cursor)
            .ok_or(EditorRestoreError::RootRevisionMismatch)?;
        debug_assert_eq!(installed_root, root_source);
        restored_universe.set_root_editor_content_hash(ContentHash::from_bytes(source.as_bytes()));
        restored_universe.set_input_summary(restored_input.summary());
        *universe = restored_universe;
        *input = restored_input;
        self.nest = restored_modes;
        Ok(fork_latency)
    }
}

struct Timer {
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
}

impl Timer {
    #[allow(clippy::disallowed_methods)] // Diagnostic latency; no TeX state observes it.
    fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            started: Instant::now(),
        }
    }

    fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Duration::ZERO
        }
    }
}

fn combine_mode_hash(universe_hash: u64, mode_hash: u64) -> u64 {
    universe_hash.rotate_left(17) ^ mode_hash
}
