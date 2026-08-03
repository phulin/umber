use std::fmt;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use tex_command::{CommandProfileMismatch, CommandState, CommandSummary, CommandSummaryError};
#[cfg(any())]
use tex_state::SourceId;
#[cfg(any())]
use tex_state::source_map::SourceMapError;
use tex_state::{
    ContentHash, FragmentStore, GenerationForkError, GenerationSubstrate, InputSummary, Snapshot,
    Universe,
};

use crate::{ExecError, ModeNest, ModeNestSummary};
#[cfg(any())]
use tex_lex::InputStack;

/// In-memory schema version for aggregate engine checkpoints.
///
/// Version 6 makes canonical and retired input continuations disjoint.
pub const ENGINE_CHECKPOINT_SCHEMA_VERSION: u32 = 6;

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
    pub(crate) continuation: CheckpointContinuation,
    pub(crate) modes: ModeNestSummary,
    state_hash: u64,
    pub(crate) root_anchor: usize,
    pub(crate) root_content_hash: Option<tex_state::ContentHash>,
    effect_prefix: usize,
    artifact_prefix: usize,
    pub(crate) budget_counters: crate::ExecutionBudgetCounters,
}

/// The command machine and retired executor have different continuation
/// owners. Keeping the variants disjoint prevents canonical publication from
/// smuggling an empty legacy input summary through the checkpoint contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CheckpointContinuation {
    Canonical(Box<CommandSummary>),
    LegacyInput(InputSummary),
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
    /// Captures a canonical named boundary.  Command publication proves that
    /// no scanner, macro matcher, or alignment delivery remains live.
    pub fn capture_canonical(
        boundary: EngineBoundary,
        command: &CommandState,
        nest: &ModeNest,
        universe: &mut Universe,
        budget_counters: crate::ExecutionBudgetCounters,
        exact_state_identity: bool,
    ) -> Result<Self, CommandSummaryError> {
        let command = command.publish_summary()?;
        let root_anchor = command.root_source_anchor().unwrap_or(0);
        let root_content_hash = universe.explicit_root_editor_content_hash();
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
            continuation: CheckpointContinuation::Canonical(Box::new(command)),
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

    /// Returns the validated canonical continuation, when this checkpoint was
    /// published by the canonical command machine.
    #[must_use]
    pub fn command_summary(&self) -> Option<&CommandSummary> {
        match &self.continuation {
            CheckpointContinuation::Canonical(summary) => Some(summary.as_ref()),
            CheckpointContinuation::LegacyInput(_) => None,
        }
    }

    /// Restores the canonical state roots.  Preparation validates the command
    /// profile and mode summary before it changes the live command, mode, or
    /// Universe roots.
    pub fn restore_canonical_state(
        &self,
        command: &mut CommandState,
        nest: &mut ModeNest,
        universe: &mut Universe,
    ) -> Result<(), CanonicalCheckpointRestoreError> {
        let CheckpointContinuation::Canonical(summary) = &self.continuation else {
            return Err(CanonicalCheckpointRestoreError::MissingCommandSummary);
        };
        let summary = summary.as_ref().clone();
        let mut restored_command = command.clone();
        restored_command
            .restore_summary(summary)
            .map_err(CanonicalCheckpointRestoreError::CommandProfile)?;
        let restored_modes = ModeNest::from_summary(self.modes.clone())
            .map_err(CanonicalCheckpointRestoreError::Mode)?;
        universe.rollback(&self.universe);
        *command = restored_command;
        *nest = restored_modes;
        Ok(())
    }

    /// Forks a retained canonical checkpoint and substitutes an edited root
    /// source whose consumed prefix is unchanged.
    pub fn fork_canonical_editor(
        &self,
        control: &mut crate::CanonicalMainControl,
        substrate: &GenerationSubstrate,
        old_source: &[u8],
        new_source: std::sync::Arc<[u8]>,
        fragments: &FragmentStore,
        layout: &tex_state::EditorLayout,
    ) -> Result<(Universe, Duration), EditorRestoreError> {
        self.fork_canonical_editor_with_paragraphs(
            control,
            substrate,
            old_source,
            new_source,
            fragments,
            layout,
            &[],
        )
        .map(|(universe, latency, _)| (universe, latency))
    }

    /// Forks a checkpoint and atomically remaps its command continuation and
    /// retained paragraph endpoints before either can be restored.
    pub fn fork_canonical_editor_with_paragraphs(
        &self,
        control: &mut crate::CanonicalMainControl,
        substrate: &GenerationSubstrate,
        old_source: &[u8],
        new_source: std::sync::Arc<[u8]>,
        fragments: &FragmentStore,
        layout: &tex_state::EditorLayout,
        paragraphs: &[crate::CanonicalParagraphRegion],
    ) -> Result<(Universe, Duration, Vec<crate::CanonicalParagraphRegion>), EditorRestoreError>
    {
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
        let summary = self.command_summary().ok_or(EditorRestoreError::Canonical(
            CanonicalCheckpointRestoreError::MissingCommandSummary,
        ))?;
        let (mut universe, owned) = substrate
            .fork_at_prepared(&self.universe, |source| {
                tex_command::OwnedCommandContinuation::detach_with_paragraphs(
                    summary,
                    paragraphs
                        .iter()
                        .map(crate::CanonicalParagraphRegion::input),
                    source,
                )
            })
            .map_err(EditorRestoreError::Fork)?;
        let fork_latency = fork_started.elapsed();
        let mut rebound = self.clone();
        let CheckpointContinuation::Canonical(command) = &mut rebound.continuation else {
            return Err(EditorRestoreError::Canonical(
                CanonicalCheckpointRestoreError::MissingCommandSummary,
            ));
        };
        let (materialized, materialized_paragraphs) =
            owned.materialize_with_paragraphs(&mut universe);
        *command = Box::new(materialized);
        if !command.rebind_root_source(old_source, new_source) {
            return Err(EditorRestoreError::RootRevisionMismatch);
        }
        let mut paragraphs = paragraphs.to_vec();
        for (paragraph, input) in paragraphs.iter_mut().zip(materialized_paragraphs) {
            paragraph.replace_input(input);
        }
        // A retained graph is an optional replay candidate, not part of the
        // checkpoint continuation. Validate every graph before mounting any
        // of them, then conservatively discard candidates whose resource
        // closure is unavailable in the selected fork. This preserves the
        // fail-before-mutation contract while allowing ordinary cold delivery
        // to handle unsupported node forms and post-anchor font resources.
        let retained_before_validation = paragraphs.len();
        paragraphs.retain(|paragraph| paragraph.can_mount_finished_lines(&universe));
        let invalid_retained_paragraphs = retained_before_validation - paragraphs.len();
        if !paragraphs
            .iter()
            .all(|paragraph| paragraph.mount_finished_lines(&mut universe))
        {
            return Err(EditorRestoreError::Canonical(
                CanonicalCheckpointRestoreError::InvalidRetainedParagraph,
            ));
        }
        rebound.universe = universe.snapshot();
        control
            .restore_checkpoint(&rebound, &mut universe)
            .map_err(EditorRestoreError::Canonical)?;
        for _ in 0..invalid_retained_paragraphs {
            universe.record_pure_paragraph_validation_failure(
                tex_state::ParagraphValidationFailure::RetainedResult,
            );
        }
        universe
            .install_editor_fragments(fragments, layout)
            .map_err(EditorRestoreError::Layout)?;
        universe.set_root_editor_content_hash(new_content_hash);
        Ok((universe, fork_latency, paragraphs))
    }

    /// Checks the immutable prerequisites for an edited-root canonical fork.
    #[must_use]
    pub fn can_fork_canonical_editor(
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
            && self
                .command_summary()
                .is_some_and(|command| command.root_source_matches(old_source))
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

    /// Returns true when both checkpoints carry matching authoritative
    /// session-local 64-bit aHash projections and the remaining explicit roots
    /// compare exactly. The projection is probabilistic: a rare collision may
    /// cause incorrect suffix reuse.
    #[must_use]
    pub fn exact_future_state_matches(&self, other: &Self) -> bool {
        self.boundary == other.boundary
            && self.universe.exact_future_state_matches(&other.universe)
            && self.continuation == other.continuation
            && self.modes == other.modes
    }

    /// Returns whether this checkpoint already carries the optional
    /// probabilistic identity used by suffix-adoption comparisons.
    #[doc(hidden)]
    #[must_use]
    pub fn has_exact_state_identity(&self) -> bool {
        self.universe.has_exact_state_identity()
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
    pub(crate) fn with_mode_projection(
        sink: &'a mut C,
        mode_projection: Option<(ModeNestSummary, u64)>,
    ) -> Self {
        Self {
            sink,
            mode_projection,
        }
    }

    pub(crate) fn into_mode_projection(self) -> Option<(ModeNestSummary, u64)> {
        self.mode_projection
    }

    #[cfg(any())]
    pub(crate) fn publish(
        &mut self,
        boundary: EngineBoundary,
        nest: &ModeNest,
        input: &mut InputStack,
        universe: &mut Universe,
        budget_counters: crate::ExecutionBudgetCounters,
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
            continuation: CheckpointContinuation::LegacyInput(input_summary),
            modes,
            state_hash,
            root_anchor,
            root_content_hash,
            effect_prefix,
            artifact_prefix,
            budget_counters,
        });
    }

    pub(crate) fn stop_requested(&self) -> bool {
        self.sink.stop_requested()
    }
}

/// Failure to restore a canonical command checkpoint.
#[derive(Debug)]
pub enum CanonicalCheckpointRestoreError {
    MissingCommandSummary,
    InvalidRetainedParagraph,
    CommandProfile(CommandProfileMismatch),
    Mode(ExecError),
}

impl fmt::Display for CanonicalCheckpointRestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCommandSummary => {
                f.write_str("checkpoint has no canonical command summary")
            }
            Self::InvalidRetainedParagraph => {
                f.write_str("retained paragraph graph cannot be mounted atomically")
            }
            Self::CommandProfile(error) => {
                write!(f, "could not restore checkpoint command profile: {error}")
            }
            Self::Mode(error) => write!(f, "could not restore checkpoint mode nest: {error}"),
        }
    }
}

impl std::error::Error for CanonicalCheckpointRestoreError {}

/// Failure to atomically restore and rebind an editor checkpoint.
#[derive(Debug)]
pub enum EditorRestoreError {
    Fork(GenerationForkError),
    Layout(tex_state::EditorLayoutError),
    #[cfg(any())]
    LayoutCursor(tex_lex::LayoutCursorError),
    RootRevisionMismatch,
    Canonical(CanonicalCheckpointRestoreError),
    #[cfg(any())]
    CanonicalContinuation,
    ChangedRootPrefix,
    #[cfg(any())]
    RootRebind(SourceMapError),
    #[cfg(any())]
    IncludedInputUnavailable(SourceId),
    Mode(ExecError),
}

impl fmt::Display for EditorRestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fork(error) => write!(f, "could not fork retained generation: {error}"),
            Self::Layout(error) => write!(f, "could not install editor layout: {error}"),
            #[cfg(any())]
            Self::LayoutCursor(error) => {
                write!(f, "could not bind editor layout to root input: {error}")
            }
            Self::RootRevisionMismatch => {
                f.write_str("checkpoint root revision does not match the accepted source")
            }
            Self::Canonical(error) => write!(f, "could not restore canonical checkpoint: {error}"),
            #[cfg(any())]
            Self::CanonicalContinuation => {
                f.write_str("canonical checkpoint cannot restore the retired input stack")
            }
            Self::ChangedRootPrefix => {
                f.write_str("edited source changed bytes before the restart anchor")
            }
            #[cfg(any())]
            Self::RootRebind(error) => write!(f, "could not rebind editor root: {error}"),
            #[cfg(any())]
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

#[cfg(any())]
impl crate::Executor {
    /// Restores a canonical checkpoint without consulting a host or legacy
    /// input stack.  All fallible reconstruction happens before any live root
    /// is changed, so a profile or mode mismatch is atomic.
    pub fn restore_canonical_checkpoint(
        &mut self,
        command: &mut CommandState,
        universe: &mut Universe,
        checkpoint: &EngineCheckpoint,
    ) -> Result<(), CanonicalCheckpointRestoreError> {
        checkpoint.restore_canonical_state(command, &mut self.nest, universe)?;
        self.budget_counters = checkpoint.budget_counters;
        Ok(())
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

#[cfg(any())]
mod tests {
    use super::{CanonicalCheckpointRestoreError, EngineBoundary, EngineCheckpoint};
    use crate::{ExecutionBudgetCounters, Executor, Mode};
    use tex_command::{CommandProfile, CommandState, RegisteredSourceKind, SourceRegistration};
    use tex_state::Universe;

    #[test]
    fn canonical_checkpoint_restores_command_mode_and_universe_atomically() {
        let mut universe = Universe::new();
        universe.set_count(3, 41);
        let mut command = CommandState::new(CommandProfile::TEX82);
        command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                b"x".to_vec(),
            ))
            .expect("source registers");
        let mut executor = Executor::new();
        let checkpoint = EngineCheckpoint::capture_canonical(
            EngineBoundary::JobStart,
            &command,
            executor.nest(),
            &mut universe,
            ExecutionBudgetCounters::default(),
            false,
        )
        .expect("quiescent command publishes");
        let expected_command = checkpoint.command_summary().cloned().expect("summary");

        universe.set_count(3, 99);
        executor
            .nest_mut()
            .push(Mode::Horizontal)
            .expect("test mode push");
        command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                b"y".to_vec(),
            ))
            .expect("second source registers");
        executor
            .restore_canonical_checkpoint(&mut command, &mut universe, &checkpoint)
            .expect("canonical checkpoint restores");

        assert_eq!(universe.count(3), 41);
        assert_eq!(executor.nest().current_mode(), Mode::Vertical);
        assert_eq!(
            command.publish_summary().expect("restored quiescent state"),
            expected_command
        );
    }

    #[test]
    fn canonical_checkpoint_rejects_profile_before_mutation() {
        let mut source_universe = Universe::new();
        let source = CommandState::new(CommandProfile::TEX82);
        let checkpoint = EngineCheckpoint::capture_canonical(
            EngineBoundary::JobStart,
            &source,
            Executor::new().nest(),
            &mut source_universe,
            ExecutionBudgetCounters::default(),
            false,
        )
        .expect("quiescent command publishes");
        let mut universe = Universe::new();
        universe.set_count(7, 19);
        let before = universe.snapshot().state_hash();
        let mut command = CommandState::new(CommandProfile::ETEX26);
        let mut executor = Executor::new();

        assert!(matches!(
            executor.restore_canonical_checkpoint(&mut command, &mut universe, &checkpoint),
            Err(CanonicalCheckpointRestoreError::CommandProfile(_))
        ));
        assert_eq!(universe.snapshot().state_hash(), before);
        assert_eq!(command.profile(), CommandProfile::ETEX26);
    }
}
