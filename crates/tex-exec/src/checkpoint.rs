use std::fmt;

use tex_command::{
    CommandRestoreError, CommandState, CommandSummary, CommandSummaryError, PreparedCommandRestore,
};
use tex_state::{RuntimeCheckpoint, Universe, UniverseError};

use crate::{ExecError, MainControl, ModeNest, ModeNestSummary};

#[cfg(test)]
mod tests;

/// In-memory schema version for aggregate engine checkpoints.
///
/// Version 8 replaces the retired whole-Universe snapshot with one opaque
/// generation checkpoint plus command and mode roots.
pub const ENGINE_CHECKPOINT_SCHEMA_VERSION: u32 = 8;

/// A safe point at which the outer executor can publish restartable state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EngineBoundary {
    JobStart,
    OuterParagraphEnd,
    ShipoutComplete,
}

/// One restartable aggregate checkpoint in an admitted generation.
///
/// The runtime portion owns one coarse generation and opaque state roots. The
/// command portion owns its coarse generation/timeline owner and bounded
/// cursors. Mode roots are installed before the runtime checkpoint truncates
/// any page or durable suffix.
#[derive(Debug)]
pub struct EngineCheckpoint<G> {
    schema_version: u32,
    boundary: EngineBoundary,
    pub(crate) runtime: RuntimeCheckpoint<G>,
    pub(crate) command: Box<CommandSummary<G>>,
    pub(crate) modes: ModeNestSummary,
    mode_hash: u64,
    pub(crate) root_anchor: usize,
    effect_prefix: usize,
    artifact_prefix: usize,
    pub(crate) budget_counters: crate::ExecutionBudgetCounters,
}

impl<G> Clone for EngineCheckpoint<G> {
    fn clone(&self) -> Self {
        Self {
            schema_version: self.schema_version,
            boundary: self.boundary,
            runtime: self.runtime.clone(),
            command: Box::new(self.command.as_ref().clone()),
            modes: self.modes.clone(),
            mode_hash: self.mode_hash,
            root_anchor: self.root_anchor,
            effect_prefix: self.effect_prefix,
            artifact_prefix: self.artifact_prefix,
            budget_counters: self.budget_counters,
        }
    }
}

impl<G> EngineCheckpoint<G> {
    pub(crate) fn pdf_history_position(&self) -> (u64, u64) {
        self.runtime.pdf_history_position()
    }

    pub(crate) fn fork_state(
        &self,
        source: &mut Universe<G>,
    ) -> Result<(Universe<G>, MainControl<G>), CheckpointRestoreError> {
        let mut destination = source
            .fork_runtime_checkpoint(&self.runtime)
            .map_err(CheckpointRestoreError::Runtime)?;
        if !self.modes.font_roots_are_live(|font| {
            destination.runtime_checkpoint_retains_font(&self.runtime, font)
        }) {
            source.return_rejected_pdf_from(&mut destination);
            return Err(CheckpointRestoreError::Runtime(UniverseError::State(
                tex_state::StateError::InvalidCursor,
            )));
        }
        let command = match CommandState::fork_summary(self.command.as_ref(), source, &destination)
        {
            Ok(command) => command,
            Err(error) => {
                source.return_rejected_pdf_from(&mut destination);
                return Err(CheckpointRestoreError::Command(error));
            }
        };
        let modes = match ModeNest::from_summary(self.modes.clone()) {
            Ok(modes) => modes,
            Err(error) => {
                source.return_rejected_pdf_from(&mut destination);
                return Err(CheckpointRestoreError::Mode(error));
            }
        };
        Ok((
            destination,
            MainControl::from_checkpoint_fork(command, modes),
        ))
    }

    /// Runs the production aggregate fork path for the standalone profiling
    /// gate. The returned generation and control remain generation-branded;
    /// this feature-only seam exposes no checkpoint field or raw cursor.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_fork_state(
        &self,
        source: &mut Universe<G>,
    ) -> Result<(Universe<G>, MainControl<G>), CheckpointRestoreError> {
        self.fork_state(source)
    }

    /// Captures a named boundary. Command publication proves that no scanner,
    /// macro matcher, alignment delivery, or attempt arena remains live.
    pub fn capture_checkpoint(
        boundary: EngineBoundary,
        command: &mut CommandState<G>,
        nest: &mut ModeNest,
        universe: &mut Universe<G>,
        budget_counters: crate::ExecutionBudgetCounters,
    ) -> Result<Self, CommandSummaryError> {
        let command = command.publish_summary(universe)?;
        let root_anchor = command
            .root_source_anchor()
            .and_then(|anchor| usize::try_from(anchor).ok())
            .unwrap_or(0);
        let modes = nest.summary();
        let mode_hash = modes.semantic_fingerprint(universe);
        let effect_prefix = usize::try_from(universe.world().effect_pos().raw())
            .expect("effect log position must fit in memory address space");
        let artifact_prefix = universe.world().artifact_pos();
        let runtime = universe
            .runtime_checkpoint_with_page_roots(modes.retains_page_node_handles())
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        Ok(Self {
            schema_version: ENGINE_CHECKPOINT_SCHEMA_VERSION,
            boundary,
            runtime,
            command: Box::new(command),
            modes,
            mode_hash,
            root_anchor,
            effect_prefix,
            artifact_prefix,
            budget_counters,
        })
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub const fn boundary(&self) -> EngineBoundary {
        self.boundary
    }

    /// Returns the allocation-independent mode-root projection captured at
    /// this boundary. Whole-state convergence identity belongs to the cold
    /// incremental checkpoint layer and is not synthesized from runtime ids.
    #[must_use]
    pub const fn mode_hash(&self) -> u64 {
        self.mode_hash
    }

    #[must_use]
    pub const fn budget_counters(&self) -> crate::ExecutionBudgetCounters {
        self.budget_counters
    }

    /// Returns the live command summary selected for later cold detachment.
    #[must_use]
    pub fn command_summary(&self) -> &CommandSummary<G> {
        self.command.as_ref()
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

    /// Restores every aggregate root atomically.
    ///
    /// Command, mode, and runtime validation finishes before mutation. During
    /// application, command and mode roots transfer after dense state but
    /// before durable, page, source, and font suffix truncation.
    pub fn restore_state(
        &self,
        command: &mut CommandState<G>,
        nest: &mut ModeNest,
        universe: &mut Universe<G>,
    ) -> Result<(), CheckpointRestoreError> {
        let prepared_command = command
            .prepare_summary_restore(self.command.as_ref(), universe)
            .map_err(CheckpointRestoreError::Command)?;
        if !self.modes.font_roots_are_live(|font| {
            universe.runtime_checkpoint_retains_font(&self.runtime, font)
        }) {
            return Err(CheckpointRestoreError::Runtime(UniverseError::State(
                tex_state::StateError::InvalidCursor,
            )));
        }
        let maximum_saved_depth = nest.maximum_saved_depth();
        let mut restored_modes =
            ModeNest::from_summary(self.modes.clone()).map_err(CheckpointRestoreError::Mode)?;
        // TeX's maxima are job-lifetime diagnostics, not semantic checkpoint
        // state. Rolling back live modes must not refund an already observed
        // §216 high-water mark.
        restored_modes.retain_maximum_saved_depth(maximum_saved_depth);
        restore_validated_roots(
            command,
            nest,
            universe,
            &self.runtime,
            prepared_command,
            restored_modes,
        )
    }
}

fn restore_validated_roots<G>(
    command: &mut CommandState<G>,
    nest: &mut ModeNest,
    universe: &mut Universe<G>,
    runtime: &RuntimeCheckpoint<G>,
    prepared_command: PreparedCommandRestore<G>,
    restored_modes: ModeNest,
) -> Result<(), CheckpointRestoreError> {
    universe
        .restore_runtime_checkpoint_with_roots(runtime, || {
            command
                .apply_prepared_restore(prepared_command)
                .expect("aggregate preflight retained its command destination");
            *nest = restored_modes;
        })
        .map_err(CheckpointRestoreError::Runtime)
}

/// Receives generation-typed checkpoints synchronously at named boundaries.
pub trait CheckpointSink<G> {
    fn wants_checkpoint(&self, _boundary: EngineBoundary) -> bool {
        true
    }

    fn stop_requested(&self) -> bool {
        false
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint<G>);
}

impl<G> CheckpointSink<G> for Vec<EngineCheckpoint<G>> {
    fn checkpoint(&mut self, checkpoint: EngineCheckpoint<G>) {
        self.push(checkpoint);
    }
}

/// Failure to restore an aggregate command checkpoint.
#[derive(Debug)]
pub enum CheckpointRestoreError {
    AttemptSuspended,
    Command(CommandRestoreError),
    Mode(ExecError),
    Runtime(UniverseError),
}

impl fmt::Display for CheckpointRestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AttemptSuspended => {
                formatter.write_str("the command attempt is owned by a suspension")
            }
            Self::Command(error) => write!(formatter, "could not restore command roots: {error}"),
            Self::Mode(error) => write!(formatter, "could not restore mode roots: {error}"),
            Self::Runtime(error) => {
                write!(formatter, "could not restore runtime roots: {error:?}")
            }
        }
    }
}

impl std::error::Error for CheckpointRestoreError {}
