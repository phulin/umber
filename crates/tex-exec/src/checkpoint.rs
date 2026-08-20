use std::fmt;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use tex_command::{CommandProfileMismatch, CommandState, CommandSummary, CommandSummaryError};
use tex_state::{
    ContentHash, FragmentStore, GenerationForkError, GenerationSubstrate, Snapshot, Universe,
};

use crate::{ExecError, ModeNest, ModeNestSummary};

#[cfg(test)]
mod tests;

/// In-memory schema version for aggregate engine checkpoints.
///
/// Version 7 makes the command summary the only continuation representation.
pub const ENGINE_CHECKPOINT_SCHEMA_VERSION: u32 = 7;

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
    pub(crate) universe: Snapshot,
    pub(crate) command: Box<CommandSummary>,
    pub(crate) modes: ModeNestSummary,
    state_hash: u64,
    pub(crate) root_anchor: usize,
    pub(crate) root_content_hash: Option<tex_state::ContentHash>,
    effect_prefix: usize,
    artifact_prefix: usize,
    pub(crate) budget_counters: crate::ExecutionBudgetCounters,
}

/// Revision roots and their precomputed content identities for checkpoint
/// rehoming.
///
/// Construction binds each identity to the source bytes that produced it, so
/// checkpoint rewrite APIs cannot accidentally validate one revision while
/// installing another revision's identity. One context is intended to be
/// shared by every checkpoint rewritten for an incremental edit.
#[doc(hidden)]
pub struct RootRehomeContext<'a> {
    old_source: &'a str,
    new_source: &'a str,
    old_content_hash: ContentHash,
    new_content_hash: ContentHash,
}

impl<'a> RootRehomeContext<'a> {
    #[must_use]
    pub fn new(old_source: &'a str, new_source: &'a str) -> Self {
        Self {
            old_source,
            new_source,
            old_content_hash: ContentHash::from_bytes(old_source.as_bytes()),
            new_content_hash: ContentHash::from_bytes(new_source.as_bytes()),
        }
    }

    #[must_use]
    pub const fn new_content_hash(&self) -> ContentHash {
        self.new_content_hash
    }
}

impl EngineCheckpoint {
    /// Captures a named boundary.  Command publication proves that
    /// no scanner, macro matcher, or alignment delivery remains live.
    pub fn capture_checkpoint(
        boundary: EngineBoundary,
        command: &CommandState,
        nest: &mut ModeNest,
        universe: &mut Universe,
        budget_counters: crate::ExecutionBudgetCounters,
        exact_state_identity: bool,
    ) -> Result<Self, CommandSummaryError> {
        let command = command.publish_summary()?;
        let root_anchor = command.root_source_anchor().unwrap_or(0);
        let root_content_hash = universe.explicit_root_editor_content_hash();
        nest.freeze_node_sidecars(universe);
        let modes = nest.summary();
        let mode_hash = modes.semantic_fingerprint(universe);
        let effect_prefix = usize::try_from(universe.world().effect_pos().raw())
            .expect("effect log position must fit in memory address space");
        let artifact_prefix = universe.world().artifact_pos();
        let universe = if exact_state_identity {
            universe.snapshot_with_exact_identity()
        } else {
            universe.snapshot()
        };
        let state_hash = combine_mode_hash(universe.state_hash(), mode_hash);
        Ok(Self {
            schema_version: ENGINE_CHECKPOINT_SCHEMA_VERSION,
            boundary,
            universe,
            command: Box::new(command),
            modes,
            state_hash,
            root_anchor,
            root_content_hash,
            effect_prefix,
            artifact_prefix,
            budget_counters,
        })
    }

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
    pub const fn budget_counters(&self) -> crate::ExecutionBudgetCounters {
        self.budget_counters
    }

    /// Returns the validated command continuation published at this boundary.
    #[must_use]
    pub fn command_summary(&self) -> &CommandSummary {
        self.command.as_ref()
    }

    /// Restores the state roots.  Preparation validates the command
    /// profile and mode summary before it changes the live command, mode, or
    /// Universe roots.
    pub fn restore_state(
        &self,
        command: &mut CommandState,
        nest: &mut ModeNest,
        universe: &mut Universe,
    ) -> Result<(), CheckpointRestoreError> {
        let summary = self.command.as_ref().clone();
        let mut restored_command = command.clone();
        restored_command
            .restore_summary(summary)
            .map_err(CheckpointRestoreError::CommandProfile)?;
        let restored_modes =
            ModeNest::from_summary(self.modes.clone()).map_err(CheckpointRestoreError::Mode)?;
        // Command input and mode state contain copy-only runtime-value
        // coordinates. Install those consumers while the checkpoint still
        // owns its sealed RegionRootSet, then let Universe restore its own
        // consumers and discard the rejected suffix.
        *command = restored_command;
        *nest = restored_modes;
        universe.rollback(&self.universe);
        Ok(())
    }

    /// Forks a retained checkpoint and substitutes an edited root source whose consumed prefix is unchanged.
    pub fn fork_editor(
        &self,
        control: &mut crate::MainControl,
        substrate: &GenerationSubstrate,
        old_source: &[u8],
        new_source: std::sync::Arc<[u8]>,
        fragments: &FragmentStore,
        layout: &tex_state::EditorLayout,
    ) -> Result<(Universe, Duration), EditorRestoreError> {
        if self.root_content_hash != Some(ContentHash::from_bytes(old_source)) {
            return Err(EditorRestoreError::RootRevisionMismatch);
        }
        if self.root_anchor > old_source.len()
            || self.root_anchor > new_source.len()
            || old_source[..self.root_anchor] != new_source[..self.root_anchor]
        {
            return Err(EditorRestoreError::ChangedRootPrefix);
        }
        let new_content_hash = ContentHash::from_bytes(&new_source);
        let fork_started = Timer::start();
        let summary = self.command_summary();
        let (mut universe, owned) = substrate
            .fork_at_prepared(&self.universe, |source| {
                tex_command::OwnedCommandContinuation::detach(summary, source)
            })
            .map_err(EditorRestoreError::Fork)?;
        let fork_latency = fork_started.elapsed();
        let mut rebound = self.clone();
        *rebound.command = owned
            .materialize(&mut universe)
            .map_err(EditorRestoreError::CommandContinuation)?;
        universe.begin_private_revision();
        universe
            .install_editor_fragments(fragments, layout)
            .map_err(EditorRestoreError::Layout)?;
        if !rebound.command.rebind_root_source(old_source, new_source) {
            return Err(EditorRestoreError::RootRevisionMismatch);
        }
        if let Some(source) = rebound.command.root_source_id() {
            universe.bind_rebound_editor_root_registration(source);
        }
        rebound.universe = universe.snapshot();
        control
            .restore_checkpoint(&rebound, &mut universe)
            .map_err(EditorRestoreError::Checkpoint)?;
        universe.set_root_editor_content_hash(new_content_hash);
        Ok((universe, fork_latency))
    }

    /// Checks the immutable prerequisites for an edited-root fork.
    #[must_use]
    pub fn can_fork_editor(
        &self,
        substrate: &GenerationSubstrate,
        old_source: &[u8],
        new_source: &[u8],
    ) -> bool {
        self.root_content_hash == Some(ContentHash::from_bytes(old_source))
            && self.root_anchor <= old_source.len()
            && self.root_anchor <= new_source.len()
            && old_source[..self.root_anchor] == new_source[..self.root_anchor]
            && substrate
                .validate_checkpoint_snapshot(&self.universe)
                .is_ok()
            && self.command_summary().root_source_matches(old_source)
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

    /// Compares the detached output/effect segment ending at two checkpoints.
    /// Future-state identity deliberately excludes these append-only logs, so
    /// convergence must validate the newly published segment separately.
    #[doc(hidden)]
    #[must_use]
    pub fn output_segment_matches(
        &self,
        self_effect_start: usize,
        self_artifact_start: usize,
        other: &Self,
        other_effect_start: usize,
        other_artifact_start: usize,
    ) -> bool {
        self.universe.output_segment_matches(
            self_effect_start..self.effect_prefix,
            self_artifact_start..self.artifact_prefix,
            &other.universe,
            other_effect_start..other.effect_prefix,
            other_artifact_start..other.artifact_prefix,
        )
    }

    #[must_use]
    pub const fn root_anchor(&self) -> usize {
        self.root_anchor
    }

    /// Returns true when both checkpoints carry matching authoritative
    /// session-local 64-bit aHash projections of reachable live state and the
    /// remaining explicit roots compare exactly. Physical handles,
    /// append-store lineages, provenance, and caches are excluded. The
    /// projection is probabilistic: a rare collision may cause incorrect
    /// suffix reuse.
    #[must_use]
    pub fn exact_future_state_matches(&self, other: &Self) -> bool {
        self.boundary == other.boundary
            && self.universe.exact_future_state_matches(&other.universe)
            && self.command.exact_future_state_matches(
                &other.command,
                self.root_anchor,
                other.root_anchor,
            )
            && self.modes == other.modes
    }

    /// Rehomes revision-relative root metadata after a validated convergence
    /// match while adopting the owner-owned state snapshot by reference.
    pub fn rehome_converged_root(
        &self,
        substrate: &GenerationSubstrate,
        roots: &RootRehomeContext<'_>,
        mapped_anchor: usize,
    ) -> Result<Self, GenerationForkError> {
        substrate.validate_checkpoint_snapshot(&self.universe)?;
        if self.root_content_hash != Some(roots.old_content_hash) {
            return Err(GenerationForkError::RootRevisionMismatch);
        }
        if mapped_anchor > roots.new_source.len()
            || !roots.new_source.is_char_boundary(mapped_anchor)
        {
            return Err(GenerationForkError::InvalidMappedAnchor);
        }
        if self.root_anchor > roots.old_source.len()
            || roots.old_source.as_bytes()[self.root_anchor..]
                != roots.new_source.as_bytes()[mapped_anchor..]
        {
            return Err(GenerationForkError::ChangedRootInterval);
        }
        let mut checkpoint = self.clone();
        let command = &mut checkpoint.command;
        if !command.rebind_root_source_at(
            roots.old_source.as_bytes(),
            std::sync::Arc::from(roots.new_source.as_bytes()),
            self.root_anchor,
            mapped_anchor,
        ) {
            return Err(GenerationForkError::RootRevisionMismatch);
        }
        checkpoint.root_anchor = mapped_anchor;
        checkpoint.root_content_hash = Some(roots.new_content_hash);
        Ok(checkpoint)
    }

    pub fn rehome_unchanged_prefix(
        &self,
        substrate: &GenerationSubstrate,
        roots: &RootRehomeContext<'_>,
    ) -> Result<Self, GenerationForkError> {
        substrate.validate_checkpoint_snapshot(&self.universe)?;
        if self.root_content_hash != Some(roots.old_content_hash) {
            return Err(GenerationForkError::RootRevisionMismatch);
        }
        if self.root_anchor > roots.old_source.len()
            || self.root_anchor > roots.new_source.len()
            || roots.old_source.as_bytes()[..self.root_anchor]
                != roots.new_source.as_bytes()[..self.root_anchor]
        {
            return Err(GenerationForkError::ChangedRootInterval);
        }
        let mut checkpoint = self.clone();
        if !checkpoint.command.rebind_root_source_at(
            roots.old_source.as_bytes(),
            std::sync::Arc::from(roots.new_source.as_bytes()),
            self.root_anchor,
            self.root_anchor,
        ) {
            return Err(GenerationForkError::RootRevisionMismatch);
        }
        checkpoint.root_content_hash = Some(roots.new_content_hash);
        Ok(checkpoint)
    }

    /// Retargets an inherited prefix checkpoint onto a promoted fork after
    /// the state layer proves it lies at or below that fork's exact anchor.
    pub fn retarget_prefix(
        &self,
        target: &GenerationSubstrate,
        source: &GenerationSubstrate,
        roots: &RootRehomeContext<'_>,
    ) -> Result<Self, GenerationForkError> {
        if self.root_content_hash != Some(roots.old_content_hash) {
            return Err(GenerationForkError::RootRevisionMismatch);
        }
        if self.root_anchor > roots.old_source.len()
            || self.root_anchor > roots.new_source.len()
            || roots.old_source.as_bytes()[..self.root_anchor]
                != roots.new_source.as_bytes()[..self.root_anchor]
        {
            return Err(GenerationForkError::ChangedRootInterval);
        }
        let mut checkpoint = self.clone();
        checkpoint.universe = target.retarget_prefix_from(source, &self.universe)?;
        checkpoint.root_content_hash = Some(roots.new_content_hash);
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

/// Failure to restore a command checkpoint.
#[derive(Debug)]
pub enum CheckpointRestoreError {
    CommandProfile(CommandProfileMismatch),
    Mode(ExecError),
}

impl fmt::Display for CheckpointRestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandProfile(error) => {
                write!(f, "could not restore checkpoint command profile: {error}")
            }
            Self::Mode(error) => write!(f, "could not restore checkpoint mode nest: {error}"),
        }
    }
}

impl std::error::Error for CheckpointRestoreError {}

/// Failure to atomically restore and rebind an editor checkpoint.
#[derive(Debug)]
pub enum EditorRestoreError {
    Fork(GenerationForkError),
    Layout(tex_state::EditorLayoutError),
    RootRevisionMismatch,
    Checkpoint(CheckpointRestoreError),
    ChangedRootPrefix,
    CommandContinuation(tex_command::CommandContinuationError),
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
            Self::Checkpoint(error) => write!(f, "could not restore checkpoint: {error}"),
            Self::ChangedRootPrefix => {
                f.write_str("edited source changed bytes before the restart anchor")
            }
            Self::CommandContinuation(error) => {
                write!(f, "could not materialize command continuation: {error}")
            }
            Self::Mode(error) => write!(f, "could not restore checkpoint mode nest: {error}"),
        }
    }
}

impl std::error::Error for EditorRestoreError {}

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
