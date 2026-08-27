use std::fmt;

use tex_command::{
    CommandRestoreError, CommandState, CommandSummary, CommandSummaryError, PreparedCommandRestore,
};
use tex_state::{RuntimeCheckpoint, Universe, UniverseError};

use crate::mode::ModeCheckpoint;
use crate::{ExecError, MainControl, ModeNest, ModeNestSummary};

#[cfg(test)]
mod tests;

/// In-memory schema version for aggregate engine checkpoints.
///
/// Version 9 removes the mode-only hash and adds optional complete identity
/// plus explicit publication-time retention metadata.
pub const ENGINE_CHECKPOINT_SCHEMA_VERSION: u32 = 9;

/// Schema for the optional complete future-reachable semantic identity.
///
/// This version is independent of the process-local checkpoint schema. It is
/// included in the canonical field stream so a later component change cannot
/// silently compare identities produced under a different contract.
pub const REACHABLE_STATE_IDENTITY_SCHEMA_VERSION: u32 = 1;

/// Complete, versioned identity of every future-reachable semantic component.
///
/// The value is optional because ordinary checkpoints do not request it and a
/// requested projection remains unavailable until every component supplies a
/// journal-maintained root. Runtime coordinates and partial projections are
/// never substituted for a missing root.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ReachableStateIdentity {
    schema_version: u32,
    fingerprint: u64,
}

impl ReachableStateIdentity {
    #[must_use]
    pub const fn schema_version(self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub const fn fingerprint(self) -> u64 {
        self.fingerprint
    }
}

/// One authoritative coarse-owner family in the aggregate checkpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CheckpointOwnerFamily {
    Command,
    Mode,
    Page,
    World,
    Hyphenation,
    Pdf,
    Dependency,
    SourceFont,
    Core,
}

/// Opaque identity of one process-local owner participating in retention.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CheckpointOwnerKey(CheckpointOwnerKeyInner);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum CheckpointOwnerKeyInner {
    State(tex_state::CheckpointOwnerId),
    Executor(usize),
}

impl CheckpointOwnerKey {
    const fn state(owner: tex_state::CheckpointOwnerId) -> Self {
        Self(CheckpointOwnerKeyInner::State(owner))
    }

    const fn executor(owner: usize) -> Self {
        Self(CheckpointOwnerKeyInner::Executor(owner))
    }
}

/// Process-local retained-byte charge published by an authoritative owner.
///
/// The owner id is opaque and has no semantic meaning. Publication accounting
/// deduplicates the `(owner, family)` pair and keeps the largest observed
/// charge as an append/journal owner grows between named boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointOwnerCharge {
    owner: CheckpointOwnerKey,
    family: CheckpointOwnerFamily,
    bytes: usize,
}

impl CheckpointOwnerCharge {
    const fn new(owner: CheckpointOwnerKey, family: CheckpointOwnerFamily, bytes: usize) -> Self {
        Self {
            owner,
            family,
            bytes,
        }
    }

    #[must_use]
    pub const fn owner(self) -> CheckpointOwnerKey {
        self.owner
    }

    #[must_use]
    pub const fn family(self) -> CheckpointOwnerFamily {
        self.family
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }
}

/// Publication-time retained-byte charge for one aggregate checkpoint.
///
/// The fixed charge array is owner-produced and allocation-free. Consumers
/// deduplicate it by opaque owner plus family rather than assuming that all
/// components happen to share one pointer or taking a largest-size proxy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointRetention {
    shared_owners: [CheckpointOwnerCharge; 9],
    checkpoint_metadata_bytes: usize,
    execution_counter_bytes: usize,
}

impl CheckpointRetention {
    fn capture<G>(
        command: &CommandSummary<G>,
        modes: &ModeCheckpoint,
        runtime: tex_state::RuntimeCheckpointRetention,
    ) -> Self {
        Self {
            shared_owners: [
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::state(command.checkpoint_owner_id()),
                    CheckpointOwnerFamily::Command,
                    command.retained_owner_bytes(),
                ),
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::executor(modes.retention_owner_address()),
                    CheckpointOwnerFamily::Mode,
                    modes.retained_owner_bytes(),
                ),
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::state(runtime.page_owner()),
                    CheckpointOwnerFamily::Page,
                    runtime.page_bytes(),
                ),
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::state(runtime.world_owner()),
                    CheckpointOwnerFamily::World,
                    runtime.world_bytes(),
                ),
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::state(runtime.hyphenation_owner()),
                    CheckpointOwnerFamily::Hyphenation,
                    runtime.hyphenation_bytes(),
                ),
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::state(runtime.pdf_owner()),
                    CheckpointOwnerFamily::Pdf,
                    runtime.pdf_bytes(),
                ),
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::state(runtime.dependency_owner()),
                    CheckpointOwnerFamily::Dependency,
                    runtime.dependency_bytes(),
                ),
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::state(runtime.source_font_owner()),
                    CheckpointOwnerFamily::SourceFont,
                    runtime.source_font_bytes(),
                ),
                CheckpointOwnerCharge::new(
                    CheckpointOwnerKey::state(runtime.core_owner()),
                    CheckpointOwnerFamily::Core,
                    runtime.core_bytes(),
                ),
            ],
            checkpoint_metadata_bytes: std::mem::size_of::<EngineCheckpoint<G>>(),
            execution_counter_bytes: std::mem::size_of::<crate::ExecutionBudgetCounters>(),
        }
    }

    #[must_use]
    pub const fn shared_owner_bytes(self) -> usize {
        let mut total = 0_usize;
        let mut index = 0;
        while index < self.shared_owners.len() {
            total = total.saturating_add(self.shared_owners[index].bytes);
            index += 1;
        }
        total
    }

    #[must_use]
    pub const fn shared_owners(&self) -> &[CheckpointOwnerCharge; 9] {
        &self.shared_owners
    }

    #[must_use]
    pub const fn checkpoint_metadata_bytes(self) -> usize {
        self.checkpoint_metadata_bytes
    }

    #[must_use]
    pub const fn command_bytes(self) -> usize {
        self.shared_owners[0].bytes
    }

    #[must_use]
    pub const fn mode_bytes(self) -> usize {
        self.shared_owners[1].bytes
    }

    #[must_use]
    pub const fn page_bytes(self) -> usize {
        self.shared_owners[2].bytes
    }

    #[must_use]
    pub const fn world_bytes(self) -> usize {
        self.shared_owners[3].bytes
    }

    #[must_use]
    pub const fn hyphenation_bytes(self) -> usize {
        self.shared_owners[4].bytes
    }

    #[must_use]
    pub const fn pdf_bytes(self) -> usize {
        self.shared_owners[5].bytes
    }

    #[must_use]
    pub const fn dependency_bytes(self) -> usize {
        self.shared_owners[6].bytes
    }

    #[must_use]
    pub const fn source_font_bytes(self) -> usize {
        self.shared_owners[7].bytes
    }

    #[must_use]
    pub const fn core_bytes(self) -> usize {
        self.shared_owners[8].bytes
    }

    #[must_use]
    pub const fn execution_counter_bytes(self) -> usize {
        self.execution_counter_bytes
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ReachableStateRoots {
    command: Option<u64>,
    mode: Option<u64>,
    page: Option<u64>,
    world: Option<u64>,
    hyphenation: Option<u64>,
    pdf: Option<u64>,
    dependency: Option<u64>,
    source: Option<u64>,
    font: Option<u64>,
    core: Option<u64>,
}

impl ReachableStateRoots {
    fn capture<G>(
        command: &CommandSummary<G>,
        modes: &ModeCheckpoint,
        runtime: &RuntimeCheckpoint<G>,
    ) -> Self {
        let runtime_roots = runtime.reachable_state_identity_roots();
        Self {
            command: command.reachable_state_identity_root(),
            mode: modes.reachable_state_identity_root(),
            page: runtime_roots.page(),
            world: runtime_roots.world(),
            hyphenation: runtime_roots.hyphenation(),
            pdf: runtime_roots.pdf(),
            dependency: runtime_roots.dependency(),
            source: runtime_roots.source(),
            font: runtime_roots.font(),
            core: runtime_roots.core(),
        }
    }

    fn complete(self) -> Option<ReachableStateIdentity> {
        let roots = [
            self.command?,
            self.mode?,
            self.page?,
            self.world?,
            self.hyphenation?,
            self.pdf?,
            self.dependency?,
            self.source?,
            self.font?,
            self.core?,
        ];
        // Fixed-seed, order-sensitive, domain-framed streaming identity. The
        // component roots are already canonical semantic identities; this
        // layer only composes them and never walks their payloads.
        let mut state =
            0x756d_6265_725f_7273_u64 ^ u64::from(REACHABLE_STATE_IDENTITY_SCHEMA_VERSION);
        for (tag, root) in (0_u64..).zip(roots) {
            state ^= tag.rotate_left(17) ^ root;
            state = state
                .rotate_left(27)
                .wrapping_mul(0x9e37_79b1_85eb_ca87)
                .wrapping_add(0x632b_e59b_d9b4_e019);
        }
        Some(ReachableStateIdentity {
            schema_version: REACHABLE_STATE_IDENTITY_SCHEMA_VERSION,
            fingerprint: avalanche(state),
        })
    }
}

const fn avalanche(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

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
    pub(crate) command: CommandSummary<G>,
    pub(crate) modes: ModeCheckpoint,
    reachable_state_identity: Option<ReachableStateIdentity>,
    retention: CheckpointRetention,
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
            command: self.command.clone(),
            modes: self.modes.clone(),
            reachable_state_identity: self.reachable_state_identity,
            retention: self.retention,
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
        // Capture pairs this opaque mode root with a later font watermark from
        // the same admitted generation.  A second walk here would inspect the
        // accepted owner's post-checkpoint suffix, not the bounded mode root.
        let command = match CommandState::fork_summary(&self.command, source, &destination) {
            Ok(command) => command,
            Err(error) => {
                source.return_rejected_pdf_from(&mut destination);
                return Err(CheckpointRestoreError::Command(error));
            }
        };
        let modes = match ModeNest::fork_checkpoint(&self.modes) {
            Ok(modes) => modes,
            Err(error) => {
                drop(command);
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

    /// Exercises only the mode/page owner fork, first mutation, and rejection
    /// seams.  The standalone gate uses this to distinguish their work from
    /// the independently owned command, core, and World fork families.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_mode_page_owner_cycle(
        &self,
        source: &mut Universe<G>,
    ) -> Result<[u64; 9], CheckpointRestoreError> {
        let mode_before = self.modes.replay_work();
        let mut modes =
            ModeNest::fork_checkpoint(&self.modes).map_err(CheckpointRestoreError::Mode)?;
        let mode_replace = u64::from(
            modes
                .current_list_mutation()
                .with_node_mut(0, |node| *node = tex_state::node::Node::Penalty(13))
                .is_some(),
        );
        modes.push_current_node(tex_state::node::Node::Penalty(17));
        let mode_private_pop = u64::from(modes.current_list_mutation().pop_last_node().is_some());
        let mode_root_pop = u64::from(modes.current_list_mutation().pop_last_node().is_some());
        drop(modes);
        let mode_work = self.modes.replay_work().saturating_sub(mode_before);
        let page_work = source
            .profile_page_owner_cycle(&self.runtime)
            .map_err(CheckpointRestoreError::Runtime)?;
        Ok([
            mode_work,
            mode_replace,
            mode_private_pop,
            mode_root_pop,
            page_work[0],
            page_work[1],
            page_work[2],
            page_work[3] + page_work[4],
            page_work[5],
        ])
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
        Self::capture_checkpoint_with_identity_demand(
            boundary,
            command,
            nest,
            universe,
            budget_counters,
            false,
        )
    }

    /// Runs the demanded-identity capture path for the standalone allocation
    /// gate. Production demand is selected through [`CheckpointSink`].
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_capture_checkpoint_with_identity_demand(
        boundary: EngineBoundary,
        command: &mut CommandState<G>,
        nest: &mut ModeNest,
        universe: &mut Universe<G>,
        budget_counters: crate::ExecutionBudgetCounters,
    ) -> Result<Self, CommandSummaryError> {
        Self::capture_checkpoint_with_identity_demand(
            boundary,
            command,
            nest,
            universe,
            budget_counters,
            true,
        )
    }

    pub(crate) fn capture_checkpoint_with_identity_demand(
        boundary: EngineBoundary,
        command: &mut CommandState<G>,
        nest: &mut ModeNest,
        universe: &mut Universe<G>,
        budget_counters: crate::ExecutionBudgetCounters,
        wants_reachable_state_identity: bool,
    ) -> Result<Self, CommandSummaryError> {
        let command = command.publish_summary(universe)?;
        let root_anchor = command
            .root_source_anchor()
            .and_then(|anchor| usize::try_from(anchor).ok())
            .unwrap_or(0);
        let modes = nest.checkpoint();
        let effect_prefix = usize::try_from(universe.world().effect_pos().raw())
            .expect("effect log position must fit in memory address space");
        let artifact_prefix = universe.world().artifact_pos();
        let runtime = universe
            .runtime_checkpoint_with_page_roots(modes.retains_page_node_handles())
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let reachable_state_identity = wants_reachable_state_identity
            .then(|| ReachableStateRoots::capture(&command, &modes, &runtime).complete())
            .flatten();
        let retention = CheckpointRetention::capture(&command, &modes, runtime.retention());
        Ok(Self {
            schema_version: ENGINE_CHECKPOINT_SCHEMA_VERSION,
            boundary,
            runtime,
            command,
            modes,
            reachable_state_identity,
            retention,
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

    #[must_use]
    pub const fn reachable_state_identity(&self) -> Option<ReachableStateIdentity> {
        self.reachable_state_identity
    }

    #[must_use]
    pub const fn retention(&self) -> CheckpointRetention {
        self.retention
    }

    #[must_use]
    pub const fn budget_counters(&self) -> crate::ExecutionBudgetCounters {
        self.budget_counters
    }

    /// Returns the live command summary selected for later cold detachment.
    #[must_use]
    pub fn command_summary(&self) -> &CommandSummary<G> {
        &self.command
    }

    #[must_use]
    pub fn mode_summary(&self) -> ModeNestSummary {
        self.modes.summary()
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
            .prepare_summary_restore(&self.command, universe)
            .map_err(CheckpointRestoreError::Command)?;
        let maximum_saved_depth = nest.maximum_saved_depth();
        restore_validated_roots(
            command,
            nest,
            universe,
            &self.runtime,
            prepared_command,
            &self.modes,
            maximum_saved_depth,
        )
    }
}

fn restore_validated_roots<G>(
    command: &mut CommandState<G>,
    nest: &mut ModeNest,
    universe: &mut Universe<G>,
    runtime: &RuntimeCheckpoint<G>,
    prepared_command: PreparedCommandRestore<G>,
    restored_modes: &ModeCheckpoint,
    maximum_saved_depth: usize,
) -> Result<(), CheckpointRestoreError> {
    universe
        .restore_runtime_checkpoint_with_roots(runtime, || {
            command
                .apply_prepared_restore(prepared_command)
                .expect("aggregate preflight retained its command destination");
            nest.restore_checkpoint(restored_modes)
                .expect("aggregate preflight retained its mode destination");
            // TeX's maxima are job-lifetime diagnostics, not semantic checkpoint
            // state. Rolling back live modes must not refund an observed high-water.
            nest.retain_maximum_saved_depth(maximum_saved_depth);
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

    /// Requests the optional complete future-reachable semantic identity.
    /// Ordinary sinks leave this false and pay no component-root work.
    fn wants_reachable_state_identity(&self, _boundary: EngineBoundary) -> bool {
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
