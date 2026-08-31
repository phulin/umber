//! Opaque retained executor generations and owner-relative checkpoint keys.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use std::collections::VecDeque;
use tex_state::{
    DetachedFormatImage, FormatError, ReachabilityStore, RetainedAttachmentKey,
    RetainedStateAccessError, RetainedStateAdmission, RetainedStateCandidateOperation,
    RetainedStateForkBuild, RetainedStateForkError, RetainedStateForkOperation,
    RetainedStateGeneration, RetainedStateOperation, RetainedStateRetirement, SessionEpochError,
    Universe, UniverseError, World,
};

use crate::EngineCheckpoint;

static NEXT_ENGINE_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_BOUNDARY_LANE_OWNER: AtomicU64 = AtomicU64::new(1);

/// One operation admitted against an opaque retained executor generation.
pub trait RetainedEngineOperation {
    type Output;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output;
}

/// Generation-branded engine episode. The generic coordinate cannot appear in
/// the operation's fixed output type.
pub struct AdmittedEngineGeneration<'a, G> {
    generation: u64,
    universe: &'a mut Universe<G>,
    sidecars: &'a mut EngineGenerationSidecars<G>,
}

/// Temporarily detached runtime and main-control owners for one admitted
/// candidate episode.
///
/// The generation sidecar slots stay exclusively borrowed for the guard's
/// lifetime. Normal completion calls [`Self::park`]; unwinding runs the same
/// non-allocating restoration from `Drop`, so aggregate generation rejection
/// can always reach command, mode, boundary, ledger, state, page, and PDF
/// ownership in dependency order.
#[doc(hidden)]
pub struct AttachedCheckpointControl<'a, G> {
    generation: u64,
    universe: &'a mut Universe<G>,
    sidecars: &'a mut EngineGenerationSidecars<G>,
    attachment: Option<Box<dyn Any>>,
    control: Option<crate::MainControl<G>>,
}

impl<'a, G> AttachedCheckpointControl<'a, G> {
    /// Mutable control slot used while constructing a fresh candidate runtime.
    /// Keeping the slot inside this guard makes every initialization unwind
    /// restore the exact rooted owner before aggregate rejection begins.
    pub fn control_slot(&mut self) -> &mut Option<crate::MainControl<G>> {
        &mut self.control
    }

    pub fn replace_attachment<T: 'static>(&mut self, attachment: T) {
        self.attachment = Some(Box::new(attachment));
    }

    pub fn initialization_parts(
        &mut self,
    ) -> (
        &mut Universe<G>,
        &mut crate::OutputLedger,
        RetainedCheckpointStore<'_, G>,
        &mut Option<crate::MainControl<G>>,
    ) {
        let sidecars = &mut *self.sidecars;
        (
            &mut *self.universe,
            sidecars
                .ledger
                .as_mut()
                .expect("the admitted generation owns its output ledger"),
            RetainedCheckpointStore {
                boundaries: sidecars
                    .boundaries
                    .as_mut()
                    .expect("the admitted generation owns its boundary lane"),
            },
            &mut self.control,
        )
    }

    pub fn parts<T: 'static>(
        &mut self,
    ) -> (
        &mut Universe<G>,
        &mut crate::OutputLedger,
        RetainedCheckpointStore<'_, G>,
        &mut crate::MainControl<G>,
        &mut T,
    ) {
        let sidecars = &mut *self.sidecars;
        (
            &mut *self.universe,
            sidecars
                .ledger
                .as_mut()
                .expect("the admitted generation owns its output ledger"),
            RetainedCheckpointStore {
                boundaries: sidecars
                    .boundaries
                    .as_mut()
                    .expect("the admitted generation owns its boundary lane"),
            },
            self.control
                .as_mut()
                .expect("the attached episode owns main control"),
            self.attachment
                .as_deref_mut()
                .expect("the attached episode owns its runtime")
                .downcast_mut::<T>()
                .expect("the attached episode runtime type was validated"),
        )
    }

    pub fn park(mut self) -> RetainedEngineAttachmentKey {
        self.restore();
        RetainedEngineAttachmentKey {
            generation: self.generation,
        }
    }

    /// Consumes a rooted live episode into the production aggregate
    /// settlement shape. Any panic while cancelling a retained attempt occurs
    /// before control leaves this guard, so `Drop` still parks both owners.
    pub fn prepare_live_settlement(
        mut self,
    ) -> Result<crate::PreparedCheckpointControl, RetainedEngineAccessError> {
        if self.sidecars.command.is_some() {
            return Err(RetainedEngineAccessError::LiveAttachment);
        }
        self.control
            .as_mut()
            .expect("the attached episode owns main control")
            .cancel_external_attempt_for_checkpoint_settlement(self.universe);
        let control = self
            .control
            .take()
            .expect("the attached episode owns main control");
        let (command, prepared) = control.into_checkpoint_candidate_parts();
        self.sidecars.command = Some(command);
        drop(self.attachment.take());
        Ok(prepared)
    }

    /// Consumes an independently materialized JobStart episode into its
    /// command-only parked shape. No rooted mode disposition exists.
    pub fn park_independent_settlement(mut self) -> Result<(), RetainedEngineAccessError> {
        if self.sidecars.command.is_some() {
            return Err(RetainedEngineAccessError::LiveAttachment);
        }
        let control = self
            .control
            .take()
            .expect("the attached episode owns main control");
        self.sidecars.command = Some(control.into_independent_parked_command());
        drop(self.attachment.take());
        Ok(())
    }

    fn restore(&mut self) {
        // The guard exclusively borrows both slots, so neither can have been
        // repopulated behind it. Direct assignment keeps unwind restoration
        // infallible and avoids a second panic while the original panic is in
        // flight.
        if let Some(control) = self.control.take() {
            self.sidecars.control = Some(control);
        }
        if let Some(attachment) = self.attachment.take() {
            self.sidecars.attachment = Some(attachment);
        }
    }
}

impl<G> Drop for AttachedCheckpointControl<'_, G> {
    fn drop(&mut self) {
        self.restore();
    }
}

impl<G> AdmittedEngineGeneration<'_, G> {
    pub fn universe(&mut self) -> &mut Universe<G> {
        self.universe
    }

    /// Reads the sole parked or live command owner's checkpoint work without
    /// traversing command payloads or changing candidate state.
    #[doc(hidden)]
    pub fn command_timeline_counters(
        &self,
    ) -> Result<tex_command::CommandTimelineCounters, RetainedEngineAccessError> {
        if let Some(command) = self.sidecars.command.as_ref() {
            return Ok(command.profile_timeline_counters());
        }
        self.sidecars
            .control
            .as_ref()
            .map(crate::MainControl::command_timeline_counters)
            .ok_or(RetainedEngineAccessError::StaleAttachment)
    }

    /// Splits the aggregate state and checkpoint store for an executor loop.
    pub fn parts(
        &mut self,
    ) -> (
        &mut Universe<G>,
        &mut crate::OutputLedger,
        RetainedCheckpointStore<'_, G>,
    ) {
        (
            self.universe,
            self.sidecars
                .ledger
                .as_mut()
                .expect("the admitted generation owns its output ledger"),
            RetainedCheckpointStore {
                boundaries: self
                    .sidecars
                    .boundaries
                    .as_mut()
                    .expect("the admitted generation owns its boundary lane"),
            },
        )
    }

    pub fn retain_checkpoint(&mut self, checkpoint: EngineCheckpoint<G>) -> RetainedCheckpointKey {
        let mut checkpoint = checkpoint;
        if !checkpoint.has_output_ledger() {
            let output = self
                .sidecars
                .ledger
                .as_mut()
                .expect("the admitted generation owns its output ledger")
                .checkpoint();
            checkpoint.set_output_ledger(output);
        }
        let evidence = RetainedBoundaryEvidence::from_checkpoint(0, 0, &checkpoint);
        self.sidecars
            .boundaries
            .as_mut()
            .expect("the admitted generation owns its boundary lane")
            .append(checkpoint, evidence)
    }

    pub fn checkpoint(
        &self,
        key: &RetainedCheckpointKey,
    ) -> Result<&EngineCheckpoint<G>, RetainedEngineAccessError> {
        self.sidecars
            .boundaries
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .get(key)
    }

    pub fn attach<T: 'static>(&mut self, attachment: T) -> RetainedEngineAttachmentKey {
        assert!(
            self.sidecars.attachment.is_none(),
            "one retained engine generation accepts one suspended runtime"
        );
        self.sidecars.attachment = Some(Box::new(attachment));
        RetainedEngineAttachmentKey {
            generation: self.generation,
        }
    }

    /// Attaches erased host/runtime metadata while keeping its typed live
    /// control separately reachable by aggregate settlement.
    pub fn attach_with_checkpoint_control<T: 'static>(
        &mut self,
        attachment: T,
        control: crate::MainControl<G>,
    ) -> Result<RetainedEngineAttachmentKey, RetainedEngineAccessError> {
        if self.sidecars.control.is_some() || self.sidecars.command.is_some() {
            return Err(RetainedEngineAccessError::LiveAttachment);
        }
        self.sidecars.control = Some(control);
        Ok(self.attach(attachment))
    }

    pub fn take_checkpoint_control(
        &mut self,
    ) -> Result<crate::MainControl<G>, RetainedEngineAccessError> {
        self.sidecars
            .control
            .take()
            .ok_or(RetainedEngineAccessError::StaleAttachment)
    }

    /// Borrows the complete live episode through an unwind-safe prepared
    /// guard. The attachment type is checked before either sidecar slot moves.
    pub fn prepare_attached_checkpoint_control<T: 'static>(
        &mut self,
        key: RetainedEngineAttachmentKey,
    ) -> Result<AttachedCheckpointControl<'_, G>, RetainedEngineAccessError> {
        validate_attachment_key(self.generation, &key)?;
        let attachment = self
            .sidecars
            .attachment
            .as_deref()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?;
        if !attachment.is::<T>() {
            return Err(RetainedEngineAccessError::AttachmentTypeMismatch);
        }
        if self.sidecars.control.is_none() {
            return Err(RetainedEngineAccessError::StaleAttachment);
        }
        Ok(AttachedCheckpointControl {
            generation: self.generation,
            universe: self.universe,
            attachment: self.sidecars.attachment.take(),
            control: self.sidecars.control.take(),
            sidecars: self.sidecars,
        })
    }

    /// Parks the sole quiescent command owner in the generation sidecar while
    /// returning the non-generic mode settlement receipt to the caller.
    pub fn prepare_checkpoint_control(
        &mut self,
        control: crate::MainControl<G>,
    ) -> Result<crate::PreparedCheckpointControl, RetainedEngineAccessError> {
        if self.sidecars.command.is_some() {
            return Err(RetainedEngineAccessError::LiveAttachment);
        }
        let (command, prepared) = control.into_checkpoint_candidate_parts();
        self.sidecars.command = Some(command);
        Ok(prepared)
    }

    pub fn prepare_live_checkpoint_control(
        &mut self,
    ) -> Result<crate::PreparedCheckpointControl, RetainedEngineAccessError> {
        let mut control = self.take_checkpoint_control()?;
        control.cancel_external_attempt_for_checkpoint_settlement(self.universe);
        self.prepare_checkpoint_control(control)
    }

    /// Parks a completed independently materialized JobStart command owner.
    /// Unlike [`Self::prepare_live_checkpoint_control`], this returns no
    /// source-settlement receipt because no command or mode owner was forked.
    pub fn park_independent_checkpoint_control(&mut self) -> Result<(), RetainedEngineAccessError> {
        if self.sidecars.command.is_some() {
            return Err(RetainedEngineAccessError::LiveAttachment);
        }
        let control = self.take_checkpoint_control()?;
        self.sidecars.command = Some(control.into_independent_parked_command());
        Ok(())
    }

    pub fn attachment_mut<T: 'static>(
        &mut self,
        key: &RetainedEngineAttachmentKey,
    ) -> Result<&mut T, RetainedEngineAccessError> {
        validate_attachment_key(self.generation, key)?;
        self.sidecars
            .attachment
            .as_deref_mut()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .downcast_mut::<T>()
            .ok_or(RetainedEngineAccessError::AttachmentTypeMismatch)
    }

    pub fn take_attachment<T: 'static>(
        &mut self,
        key: RetainedEngineAttachmentKey,
    ) -> Result<T, RetainedEngineAccessError> {
        validate_attachment_key(self.generation, &key)?;
        self.sidecars
            .attachment
            .take()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .downcast::<T>()
            .map(|attachment| *attachment)
            .map_err(|_| RetainedEngineAccessError::AttachmentTypeMismatch)
    }
}

/// Restricted checkpoint-store borrow used by a synchronous sink.
pub struct RetainedCheckpointStore<'a, G> {
    boundaries: &'a mut BoundaryLane<G>,
}

impl<G> RetainedCheckpointStore<'_, G> {
    pub fn retain(&mut self, checkpoint: EngineCheckpoint<G>) -> RetainedCheckpointKey {
        let evidence = RetainedBoundaryEvidence::from_checkpoint(0, 0, &checkpoint);
        self.boundaries.append(checkpoint, evidence)
    }

    pub fn retain_boundary(
        &mut self,
        checkpoint: EngineCheckpoint<G>,
        evidence: RetainedBoundaryEvidence,
    ) -> RetainedCheckpointKey {
        self.boundaries.append(checkpoint, evidence)
    }

    /// Retains schedule/comparison evidence without a live restart root.
    /// Frozen JobStart uses this because restart ownership lives in its
    /// immutable anchor rather than in the accepted journal lineage.
    pub fn retain_evidence(&mut self, evidence: RetainedBoundaryEvidence) {
        self.boundaries.append_evidence(evidence);
    }

    /// Releases one restart root during publication-time budget enforcement.
    /// Detached boundary evidence is owned by the incremental layer and is
    /// intentionally unaffected.
    pub fn release(
        &mut self,
        key: RetainedCheckpointKey,
    ) -> Result<crate::EngineCheckpointRelease<G>, RetainedEngineAccessError> {
        self.boundaries.release_key(key)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn storage(&self) -> BoundaryLaneStorage {
        self.boundaries.storage()
    }
}

/// Fixed detached evidence stored in the same move-only cell as its optional
/// restart root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedBoundaryEvidence {
    revision: u64,
    position: usize,
    boundary: crate::EngineBoundary,
    ordinal: u32,
    effect_prefix: usize,
    artifact_prefix: usize,
    reachable_state_identity: Option<crate::ReachableStateIdentity>,
}

impl RetainedBoundaryEvidence {
    #[must_use]
    pub const fn new(
        revision: u64,
        position: usize,
        boundary: crate::EngineBoundary,
        ordinal: u32,
        effect_prefix: usize,
        artifact_prefix: usize,
        reachable_state_identity: Option<crate::ReachableStateIdentity>,
    ) -> Self {
        Self {
            revision,
            position,
            boundary,
            ordinal,
            effect_prefix,
            artifact_prefix,
            reachable_state_identity,
        }
    }

    fn from_checkpoint(
        revision: u64,
        ordinal: u32,
        checkpoint: &EngineCheckpoint<impl Sized>,
    ) -> Self {
        Self::new(
            revision,
            checkpoint.root_anchor(),
            checkpoint.boundary(),
            ordinal,
            checkpoint.effect_prefix_len(),
            checkpoint.artifact_prefix_len(),
            checkpoint.reachable_state_identity(),
        )
    }

    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }
    #[must_use]
    pub const fn position(self) -> usize {
        self.position
    }
    #[must_use]
    pub const fn boundary(self) -> crate::EngineBoundary {
        self.boundary
    }
    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }
    #[must_use]
    pub const fn effect_prefix(self) -> usize {
        self.effect_prefix
    }
    #[must_use]
    pub const fn artifact_prefix(self) -> usize {
        self.artifact_prefix
    }
    #[must_use]
    pub const fn reachable_state_identity(self) -> Option<crate::ReachableStateIdentity> {
        self.reachable_state_identity
    }
}

/// Private owner-relative identity of one named retained checkpoint.
///
/// The key is non-`Copy`, non-`Clone`, and non-serializable. Detached boundary
/// identity remains in tex-incr's `BoundaryRecord` instead.
#[derive(Debug)]
pub struct RetainedCheckpointKey {
    owner: u64,
    slot: u32,
    generation: u32,
    record: u64,
}

impl PartialEq for RetainedCheckpointKey {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner && self.slot == other.slot && self.generation == other.generation
    }
}

impl Eq for RetainedCheckpointKey {}

/// Owner-relative key for one unpublished executor episode sidecar.
#[derive(Debug, Eq, PartialEq)]
pub struct RetainedEngineAttachmentKey {
    generation: u64,
}

/// Mutation-free retained executor admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedEngineAccessError {
    CandidateTransactionActive,
    ForeignGeneration,
    StaleCheckpoint,
    StaleAttachment,
    AttachmentTypeMismatch,
    LiveAttachment,
    ProtectedCheckpoint,
    State(RetainedStateAccessError),
}

#[derive(Debug)]
pub enum RetainedEngineForkError {
    Access(RetainedEngineAccessError),
    Restore(crate::CheckpointRestoreError),
    SlotsExhausted,
    IdentityExhausted,
}

impl core::fmt::Display for RetainedEngineForkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Access(error) => write!(formatter, "checkpoint fork access failed: {error:?}"),
            Self::Restore(error) => write!(formatter, "checkpoint fork restore failed: {error}"),
            Self::SlotsExhausted => formatter.write_str("both retained generation slots are live"),
            Self::IdentityExhausted => {
                formatter.write_str("retained generation identity space is exhausted")
            }
        }
    }
}

impl std::error::Error for RetainedEngineForkError {}

/// Scalar evidence that optional named roots were released without touching
/// immutable rows in their shared generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointPruningReceipt {
    released: usize,
    retained: usize,
    slots: usize,
}

/// Authoritative live/capacity gauges for the reusable retained-boundary row
/// pool. Capacity is physical row capacity, not an inferred byte estimate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BoundaryLaneStorage {
    pub live_rows: usize,
    pub row_capacity: usize,
    pub live_restart_roots: usize,
    pub rows_released: u64,
    pub rows_reused: u64,
}

impl CheckpointPruningReceipt {
    #[must_use]
    pub const fn released(self) -> usize {
        self.released
    }

    #[must_use]
    pub const fn retained(self) -> usize {
        self.retained
    }

    /// Physical checkpoint slots retained for exact O(1) reuse.
    #[must_use]
    pub const fn slots(self) -> usize {
        self.slots
    }
}

/// Once-only whole-generation retirement receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedEngineRetirement {
    state: RetainedStateRetirement,
}

impl RetainedEngineRetirement {
    #[must_use]
    pub const fn state(self) -> RetainedStateRetirement {
        self.state
    }
}

impl From<RetainedStateAccessError> for RetainedEngineAccessError {
    fn from(error: RetainedStateAccessError) -> Self {
        Self::State(error)
    }
}

/// Public non-generic aggregate owner of one revision generation.
///
/// The session reachability store owns physical storage. This move-only lease
/// names one of its two slots; main-control and checkpoint roots remain
/// generation-typed sidecars below that same store owner.
pub struct RetainedEngineGeneration<'store> {
    generation: u64,
    state: Option<RetainedStateGeneration<'store>>,
    sidecars: RetainedAttachmentKey,
    liveness: Arc<()>,
}

/// Complete move-only result of forking one retained named boundary.
///
/// Keeping the restored generation and its owner-relative checkpoint evidence
/// together prevents callers from accidentally reordering the parallel tuple
/// fields that make up one fork transaction.
pub struct RetainedBoundaryFork<'store> {
    pub generation: RetainedEngineGeneration<'store>,
    pub runtime: RetainedEngineAttachmentKey,
    pub budget_counters: crate::ExecutionBudgetCounters,
    pub selected: RetainedBoundaryEvidence,
    pub selected_key: RetainedCheckpointKey,
    pub retention: crate::CheckpointRetention,
}

/// Copy-only boundary and output-coordinate plan for one accepted revision
/// rehome. Source ownership is carried separately by
/// [`RetainedEditorRevisionRehome`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedEditorRevisionRehomePlan {
    pub revision: u64,
    pub old_start: usize,
    pub old_end: usize,
    pub new_end: usize,
    pub restart: (usize, crate::EngineBoundary, u32),
    pub convergence: (usize, crate::EngineBoundary, u32),
    pub new_effect_prefix: usize,
    pub new_artifact_prefix: usize,
}

/// Source owners and coordinate plan consumed by one atomic accepted revision
/// rehome.
pub struct RetainedEditorRevisionRehome<'a> {
    pub accepted: &'a [u8],
    pub bytes: Arc<[u8]>,
    pub plan: RetainedEditorRevisionRehomePlan,
}

/// Marker for the typed main-control owner restored together with one
/// aggregate checkpoint. The control itself remains in the generation's
/// typed sidecar so generic settlement can always reach it.
#[doc(hidden)]
pub struct RestoredCheckpointRuntime;

/// Weak coarse-owner witness used by lifecycle tests and host diagnostics.
/// It retains no runtime row, arena, checkpoint, or generation coordinate.
#[derive(Clone, Debug)]
pub struct RetainedEngineGenerationWitness(Weak<()>);

impl RetainedEngineGenerationWitness {
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.0.strong_count() != 0
    }
}

impl core::fmt::Debug for RetainedEngineGeneration<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RetainedEngineGeneration")
            .field(
                "checkpoints",
                &self
                    .state
                    .as_ref()
                    .map_or(0, RetainedStateGeneration::attachment_count),
            )
            .finish_non_exhaustive()
    }
}

impl<'store> RetainedEngineGeneration<'store> {
    pub fn new(store: &'store ReachabilityStore, world: World) -> Result<Self, SessionEpochError> {
        Self::new_owned(store.clone(), world)
    }

    #[doc(hidden)]
    pub fn new_owned(store: ReachabilityStore, world: World) -> Result<Self, SessionEpochError> {
        let generation = next_generation();
        let mut state = RetainedStateGeneration::new_owned(store, world)?;
        let sidecars = state.with_admitted(InitializeSidecars { generation });
        Ok(Self {
            generation,
            state: Some(state),
            sidecars,
            liveness: Arc::new(()),
        })
    }

    pub fn from_format(
        store: &'store ReachabilityStore,
        world: World,
        image: DetachedFormatImage,
    ) -> Result<Self, FormatError> {
        Self::from_format_owned(store.clone(), world, image)
    }

    #[doc(hidden)]
    pub fn from_format_owned(
        store: ReachabilityStore,
        world: World,
        image: DetachedFormatImage,
    ) -> Result<Self, FormatError> {
        Self::from_format_owned_with_page_node_identity_demand(store, world, image, false)
    }

    #[doc(hidden)]
    pub fn from_format_owned_with_page_node_identity_demand(
        store: ReachabilityStore,
        world: World,
        image: DetachedFormatImage,
        wants_page_node_semantic_identity: bool,
    ) -> Result<Self, FormatError> {
        let generation = next_generation();
        let mut state = RetainedStateGeneration::from_format_owned_with_page_node_identity_demand(
            store,
            world,
            image,
            wants_page_node_semantic_identity,
        )?;
        let sidecars = state.with_admitted(InitializeSidecars { generation });
        Ok(Self {
            generation,
            state: Some(state),
            sidecars,
            liveness: Arc::new(()),
        })
    }

    /// Materializes the session's frozen JobStart image into the sole current
    /// slot without rewinding or borrowing any owner from this accepted
    /// generation. The resulting pair still settles through the ordinary
    /// aggregate accept/reject barrier.
    #[doc(hidden)]
    pub fn materialize_job_start_candidate(
        &mut self,
        world: World,
        image: DetachedFormatImage,
        wants_page_node_semantic_identity: bool,
    ) -> Result<Self, FormatError> {
        let generation = next_generation();
        let mut state = self
            .state
            .as_mut()
            .expect("an accepted generation owns state")
            .materialize_independent_format_candidate(
                world,
                image,
                wants_page_node_semantic_identity,
            )?;
        let sidecars = state.with_admitted(InitializeSidecars { generation });
        Ok(Self {
            generation,
            state: Some(state),
            sidecars,
            liveness: Arc::new(()),
        })
    }

    #[doc(hidden)]
    #[must_use]
    pub fn is_independent_job_start_candidate(&self) -> bool {
        self.state
            .as_ref()
            .is_some_and(RetainedStateGeneration::is_independent_candidate_transaction_destination)
    }

    pub fn with_admitted<O: RetainedEngineOperation>(
        &mut self,
        operation: O,
    ) -> Result<O::Output, RetainedEngineAccessError> {
        let state = self.state.as_mut().expect("a live generation owns state");
        if state.has_candidate_transaction() {
            return Err(RetainedEngineAccessError::CandidateTransactionActive);
        }
        state.with_admitted(EngineOperationAdapter {
            generation: self.generation,
            sidecars: &self.sidecars,
            operation,
        })
    }

    /// Creates the sole current slot from one accepted named checkpoint.
    /// The returned attachment owns a restored [`crate::MainControl`]; a
    /// caller takes it inside the first destination admission episode.
    pub fn fork_checkpoint(
        &mut self,
        checkpoint: &RetainedCheckpointKey,
    ) -> Result<
        (
            Self,
            RetainedEngineAttachmentKey,
            crate::ExecutionBudgetCounters,
        ),
        RetainedEngineForkError,
    > {
        let state = self.state.as_mut().expect("a live generation owns state");
        if state.has_candidate_transaction() {
            return Err(RetainedEngineForkError::Access(
                RetainedEngineAccessError::CandidateTransactionActive,
            ));
        }
        let generation = next_generation();
        let result = state.try_fork_owned(ForkCheckpoint {
            generation,
            source_generation: self.generation,
            sidecars: &self.sidecars,
            checkpoint,
        });
        let (state, sidecars, budget_counters) = match result {
            Ok(result) => result,
            Err(RetainedStateForkError::Operation(error)) => return Err(error),
            Err(RetainedStateForkError::SlotsExhausted) => {
                return Err(RetainedEngineForkError::SlotsExhausted);
            }
            Err(RetainedStateForkError::IdentityExhausted) => {
                return Err(RetainedEngineForkError::IdentityExhausted);
            }
        };
        Ok((
            Self {
                generation,
                state: Some(state),
                sidecars,
                liveness: Arc::new(()),
            },
            RetainedEngineAttachmentKey { generation },
            budget_counters,
        ))
    }

    /// Selects an accepted boundary cell directly from the canonical record
    /// lane. Missing/stale detached evidence falls back explicitly to the
    /// surviving JobStart root rather than to an unrelated slot index.
    pub fn fork_boundary(
        &mut self,
        selected: Option<(usize, crate::EngineBoundary, u32)>,
    ) -> Result<RetainedBoundaryFork<'store>, RetainedEngineForkError> {
        let selected = self
            .with_admitted(SelectBoundary { selected })
            .map_err(RetainedEngineForkError::Access)?;
        let (key, evidence, retention) = selected.map_err(RetainedEngineForkError::Access)?;
        let (generation, runtime, counters) = self.fork_checkpoint(&key)?;
        Ok(RetainedBoundaryFork {
            generation,
            runtime,
            budget_counters: counters,
            selected: evidence,
            selected_key: key,
            retention,
        })
    }

    /// Forks the latest live restart root whose conservative root-source
    /// anchor does not exceed `position`. Detached evidence is deliberately
    /// ignored: `None` tells the session to select its explicit frozen
    /// JobStart path rather than disguising a cold materialization as reuse.
    pub fn fork_latest_boundary_at_or_before(
        &mut self,
        position: usize,
    ) -> Result<Option<RetainedBoundaryFork<'store>>, RetainedEngineForkError> {
        let selected = self
            .with_admitted(SelectLatestBoundary { position })
            .map_err(RetainedEngineForkError::Access)?
            .map_err(RetainedEngineForkError::Access)?;
        let Some((key, evidence, retention)) = selected else {
            return Ok(None);
        };
        let (generation, runtime, counters) = self.fork_checkpoint(&key)?;
        Ok(Some(RetainedBoundaryFork {
            generation,
            runtime,
            budget_counters: counters,
            selected: evidence,
            selected_key: key,
            retention,
        }))
    }

    /// Rehomes detached boundary metadata after a byte-identical revision
    /// converges and its candidate has been rejected. Runtime checkpoints and
    /// object graphs stay untouched because the root source bytes are equal.
    pub fn rehome_identical_revision_boundaries(
        &mut self,
        revision: u64,
    ) -> Result<(), RetainedEngineAccessError> {
        self.with_admitted(RehomeIdenticalRevision { revision })?
    }

    /// Atomically retargets the accepted editor backing and detached boundary
    /// revision metadata after the current generation has converged and been
    /// rejected.
    pub fn rehome_editor_revision(
        &mut self,
        rehome: RetainedEditorRevisionRehome<'_>,
    ) -> Result<usize, RetainedEngineAccessError> {
        self.with_admitted(RehomeEditorRevision(rehome))?
    }

    #[must_use]
    pub fn witness(&self) -> RetainedEngineGenerationWitness {
        RetainedEngineGenerationWitness(Arc::downgrade(&self.liveness))
    }

    /// Whether two generations reside in the same external session store.
    #[must_use]
    pub fn same_store(&self, other: &Self) -> bool {
        self.state
            .as_ref()
            .expect("a live generation owns state")
            .same_store(other.state.as_ref().expect("a live generation owns state"))
    }

    /// Explicitly settles the accepted/current owner transaction before the
    /// prior generation retires. No individual component may commit itself.
    #[doc(hidden)]
    pub fn prepare_candidate_accept(&mut self, candidate: &mut Self) {
        let independent = candidate.is_independent_job_start_candidate();
        if !independent {
            candidate
                .state
                .as_mut()
                .expect("a live candidate owns state")
                .with_candidate_source(SettleOutputLedger::Accept)
                .expect("the accepted/current pair settles one output owner")
                .expect("the accepted/current sidecars own the output ledger");
        }
        self.state
            .as_mut()
            .expect("an accepted generation owns state")
            .prepare_candidate_accept(
                candidate
                    .state
                    .as_mut()
                    .expect("a live candidate owns state"),
            );
    }

    #[doc(hidden)]
    pub fn finish_candidate_accept(&mut self, candidate: &mut Self) {
        self.state
            .as_mut()
            .expect("an accepted generation owns state")
            .finish_candidate_accept(
                candidate
                    .state
                    .as_mut()
                    .expect("a live candidate owns state"),
            );
    }

    /// Explicitly rejects every current owner before releasing the current
    /// physical slot. `Drop` remains only an unwind safety net.
    #[doc(hidden)]
    pub fn prepare_candidate_reject(&mut self) {
        let state = self.state.as_mut().expect("a live candidate owns state");
        if state.is_candidate_transaction_destination() {
            if !state.is_independent_candidate_transaction_destination() {
                state
                    .with_candidate_source(SettleOutputLedger::Reject)
                    .expect("the rejected current pair returns one output owner")
                    .expect("the accepted/current sidecars own the output ledger");
            }
            state.prepare_candidate_reject();
        }
    }

    #[doc(hidden)]
    pub fn finish_candidate_reject(mut self) {
        self.state
            .take()
            .expect("a live candidate owns state")
            .finish_candidate_reject();
    }

    /// Mutation-free terminal preflight. Every retained root is statically
    /// branded by this admitted generation; this validates the remaining
    /// owner-relative packed keys and proves no suspended episode survives.
    pub fn preflight_terminal(
        &mut self,
        retained: &[RetainedCheckpointKey],
    ) -> Result<(), RetainedEngineAccessError> {
        self.with_admitted(PreflightTerminal { retained })?
    }

    /// Drops optional checkpoint roots only after validating the complete
    /// retained key set. Their immutable semantic owners retire with them.
    pub fn prune_checkpoints(
        &mut self,
        retained: &[RetainedCheckpointKey],
    ) -> Result<CheckpointPruningReceipt, RetainedEngineAccessError> {
        self.with_admitted(PruneCheckpoints { retained })?
    }

    pub fn preflight_boundary_lane(&mut self) -> Result<(), RetainedEngineAccessError> {
        self.with_admitted(PreflightBoundaryLane)?
    }

    pub fn boundary_lane_checkpoint_count(&mut self) -> Result<usize, RetainedEngineAccessError> {
        self.with_admitted(BoundaryLaneCheckpointCount)
    }

    pub fn retire(mut self) -> Result<RetainedEngineRetirement, UniverseError> {
        Ok(RetainedEngineRetirement {
            state: self
                .state
                .take()
                .expect("a live generation owns state")
                .retire()?,
        })
    }
}

impl Drop for RetainedEngineGeneration<'_> {
    fn drop(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if !state.is_candidate_transaction_destination() {
            return;
        }
        if state.is_independent_candidate_transaction_destination() {
            state.prepare_candidate_reject();
            return;
        }
        let settled = state
            .with_candidate_source(SettleOutputLedger::Reject)
            .and_then(|result| result);
        if settled.is_ok() {
            state.prepare_candidate_reject();
        }
    }
}

enum SettleOutputLedger {
    Accept,
    Reject,
}

impl RetainedStateCandidateOperation for SettleOutputLedger {
    type Output = Result<(), RetainedStateAccessError>;

    fn run<G: 'static>(
        self,
        mut source: RetainedStateAdmission<'_, G>,
        mut candidate: RetainedStateAdmission<'_, G>,
    ) -> Self::Output {
        match self {
            Self::Accept => {
                let candidate = candidate.sole_attachment_mut::<EngineGenerationSidecars<G>>()?;
                let ledger = candidate
                    .ledger
                    .as_mut()
                    .ok_or(RetainedStateAccessError::StaleAttachment)?;
                if let Some(command) = candidate.command.as_mut() {
                    command.accept_checkpoint_candidate();
                } else {
                    let control = candidate
                        .control
                        .take()
                        .ok_or(RetainedStateAccessError::StaleAttachment)?;
                    candidate.control = Some(control.into_accepted_checkpoint_candidate());
                }
                candidate
                    .boundaries
                    .as_mut()
                    .ok_or(RetainedStateAccessError::StaleAttachment)?
                    .accept();
                ledger.accept_checkpoint_candidate();
            }
            Self::Reject => {
                let (parked_command, control) = {
                    let sidecars =
                        candidate.sole_attachment_mut::<EngineGenerationSidecars<G>>()?;
                    (sidecars.command.take(), sidecars.control.take())
                };
                let command = if let Some(mut command) = parked_command {
                    command.reject_checkpoint_candidate();
                    command
                } else {
                    control
                        .ok_or(RetainedStateAccessError::StaleAttachment)?
                        .into_rejected_checkpoint_command_with_state(candidate.universe())
                };
                let candidate = candidate.sole_attachment_mut::<EngineGenerationSidecars<G>>()?;
                let ledger = candidate
                    .ledger
                    .as_mut()
                    .ok_or(RetainedStateAccessError::StaleAttachment)?;
                candidate
                    .boundaries
                    .as_mut()
                    .ok_or(RetainedStateAccessError::StaleAttachment)?
                    .reject();
                ledger.reject_checkpoint_candidate();
                let source = source.sole_attachment_mut::<EngineGenerationSidecars<G>>()?;
                source.boundaries = candidate.boundaries.take();
                source.command = Some(command);
                source.ledger = candidate.ledger.take();
            }
        }
        Ok(())
    }
}

struct ForkCheckpoint<'a> {
    generation: u64,
    source_generation: u64,
    sidecars: &'a RetainedAttachmentKey,
    checkpoint: &'a RetainedCheckpointKey,
}

impl RetainedStateForkOperation for ForkCheckpoint<'_> {
    type Output = crate::ExecutionBudgetCounters;
    type Error = RetainedEngineForkError;

    fn run<G: 'static>(
        self,
        mut source: RetainedStateAdmission<'_, G>,
    ) -> Result<RetainedStateForkBuild<G, Self::Output>, Self::Error> {
        let (universe, sidecars) = source
            .universe_and_attachment_mut::<EngineGenerationSidecars<G>>(self.sidecars)
            .map_err(|error| RetainedEngineForkError::Access(error.into()))?;
        if sidecars.generation != self.source_generation {
            return Err(RetainedEngineForkError::Access(
                RetainedEngineAccessError::ForeignGeneration,
            ));
        }
        if sidecars.attachment.is_some() {
            return Err(RetainedEngineForkError::Access(
                RetainedEngineAccessError::LiveAttachment,
            ));
        }
        let boundaries = sidecars
            .boundaries
            .as_ref()
            .ok_or(RetainedEngineForkError::Access(
                RetainedEngineAccessError::StaleAttachment,
            ))?;
        if !boundaries.can_begin(self.checkpoint) {
            return Err(RetainedEngineForkError::Access(
                RetainedEngineAccessError::StaleCheckpoint,
            ));
        }
        let checkpoint = boundaries
            .get(self.checkpoint)
            .map_err(RetainedEngineForkError::Access)?;
        let budget_counters = checkpoint.budget_counters();
        let mut ledger = sidecars
            .ledger
            .take()
            .expect("the accepted generation owns its output ledger");
        let (universe, control) =
            match checkpoint.fork_state(universe, &mut sidecars.command, &mut ledger) {
                Ok(restored) => restored,
                Err(error) => {
                    sidecars.ledger = Some(ledger);
                    return Err(RetainedEngineForkError::Restore(error));
                }
            };
        let mut boundaries = sidecars
            .boundaries
            .take()
            .expect("validated accepted generation owns its boundary lane");
        boundaries.begin(self.checkpoint);
        Ok(RetainedStateForkBuild::new(
            universe,
            Box::new(EngineGenerationSidecars::<G> {
                generation: self.generation,
                attachment: Some(Box::new(RestoredCheckpointRuntime)),
                boundaries: Some(boundaries),
                command: None,
                control: Some(control),
                ledger: Some(ledger),
            }),
            budget_counters,
        ))
    }
}

struct PruneCheckpoints<'a> {
    retained: &'a [RetainedCheckpointKey],
}

struct SelectBoundary {
    selected: Option<(usize, crate::EngineBoundary, u32)>,
}

struct SelectLatestBoundary {
    position: usize,
}

struct RehomeIdenticalRevision {
    revision: u64,
}

struct RehomeEditorRevision<'a>(RetainedEditorRevisionRehome<'a>);

struct PreflightBoundaryLane;

impl RetainedEngineOperation for PreflightBoundaryLane {
    type Output = Result<(), RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        if admitted.sidecars.attachment.is_some() {
            return Err(RetainedEngineAccessError::LiveAttachment);
        }
        admitted
            .sidecars
            .boundaries
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .validate_all()
    }
}

struct BoundaryLaneCheckpointCount;

impl RetainedEngineOperation for BoundaryLaneCheckpointCount {
    type Output = usize;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        admitted
            .sidecars
            .boundaries
            .as_ref()
            .map_or(0, |boundaries| boundaries.live_roots)
    }
}

impl RetainedEngineOperation for SelectBoundary {
    type Output = Result<
        (
            RetainedCheckpointKey,
            RetainedBoundaryEvidence,
            crate::CheckpointRetention,
        ),
        RetainedEngineAccessError,
    >;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        let (key, evidence) = admitted
            .sidecars
            .boundaries
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .select(self.selected)?;
        let retention = admitted
            .sidecars
            .boundaries
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .get(&key)?
            .retention();
        Ok((key, evidence, retention))
    }
}

impl RetainedEngineOperation for SelectLatestBoundary {
    type Output = Result<
        Option<(
            RetainedCheckpointKey,
            RetainedBoundaryEvidence,
            crate::CheckpointRetention,
        )>,
        RetainedEngineAccessError,
    >;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        let boundaries = admitted
            .sidecars
            .boundaries
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?;
        let Some((key, evidence)) = boundaries.latest_restart_at_or_before(self.position)? else {
            return Ok(None);
        };
        let retention = boundaries.get(&key)?.retention();
        Ok(Some((key, evidence, retention)))
    }
}

impl RetainedEngineOperation for RehomeIdenticalRevision {
    type Output = Result<(), RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        admitted
            .sidecars
            .boundaries
            .as_mut()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .rehome_identical_revision(self.revision)
    }
}

impl RetainedEngineOperation for RehomeEditorRevision<'_> {
    type Output = Result<usize, RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        let RetainedEditorRevisionRehome {
            accepted,
            bytes,
            plan,
        } = self.0;
        let command = admitted
            .sidecars
            .command
            .as_mut()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?;
        let boundaries = admitted
            .sidecars
            .boundaries
            .as_mut()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?;
        boundaries.validate_revision_rehome(plan)?;
        command
            .rehome_generated_editor_source(
                accepted,
                bytes,
                plan.old_start,
                plan.old_end,
                plan.new_end,
            )
            .map_err(|_| RetainedEngineAccessError::StaleCheckpoint)?;
        boundaries.rehome_revision_suffix(plan)?;
        Ok(boundaries.live_roots)
    }
}

struct PreflightTerminal<'a> {
    retained: &'a [RetainedCheckpointKey],
}

impl RetainedEngineOperation for PreflightTerminal<'_> {
    type Output = Result<(), RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        if admitted.sidecars.attachment.is_some() {
            return Err(RetainedEngineAccessError::LiveAttachment);
        }
        for key in self.retained {
            admitted
                .sidecars
                .boundaries
                .as_ref()
                .ok_or(RetainedEngineAccessError::StaleAttachment)?
                .get(key)?;
        }
        Ok(())
    }
}

impl RetainedEngineOperation for PruneCheckpoints<'_> {
    type Output = Result<CheckpointPruningReceipt, RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        let receipt = admitted
            .sidecars
            .boundaries
            .as_mut()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .prune(self.retained)?;
        let low_water = admitted
            .sidecars
            .boundaries
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .pdf_history_low_water()
            .unwrap_or_else(|| admitted.universe.pdf_history_head());
        admitted.universe.prune_pdf_history(low_water);
        Ok(receipt)
    }
}

struct BoundaryCell<G> {
    evidence: RetainedBoundaryEvidence,
    checkpoint: Option<EngineCheckpoint<G>>,
}

const BOUNDARY_ROWS_PER_PAGE: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BoundarySlotKey {
    slot: u32,
    generation: u32,
    record: u64,
}

struct BoundarySlot<G> {
    generation: u32,
    cell: Option<BoundaryCell<G>>,
    free_next: Option<u32>,
}

impl<G> BoundarySlot<G> {
    const fn vacant(free_next: Option<u32>) -> Self {
        Self {
            generation: 1,
            cell: None,
            free_next,
        }
    }
}

struct BoundaryPage<G> {
    slots: Box<[BoundarySlot<G>]>,
}

enum BoundaryOwnership {
    Accepted(VecDeque<BoundarySlotKey>),
    Forked {
        prefix: VecDeque<BoundarySlotKey>,
        detached_prior: VecDeque<BoundarySlotKey>,
        current: VecDeque<BoundarySlotKey>,
    },
}

struct BoundaryLane<G> {
    owner: u64,
    next_record: u64,
    pages: Vec<BoundaryPage<G>>,
    free_head: Option<u32>,
    ownership: BoundaryOwnership,
    protected_record: Option<u64>,
    live_roots: usize,
    rows_released: u64,
    rows_reused: u64,
}

impl<G> Default for BoundaryLane<G> {
    fn default() -> Self {
        Self {
            owner: NEXT_BOUNDARY_LANE_OWNER.fetch_add(1, Ordering::Relaxed),
            next_record: 0,
            pages: Vec::new(),
            free_head: None,
            ownership: BoundaryOwnership::Accepted(VecDeque::new()),
            protected_record: None,
            live_roots: 0,
            rows_released: 0,
            rows_reused: 0,
        }
    }
}

impl<G> BoundaryLane<G> {
    fn storage(&self) -> BoundaryLaneStorage {
        BoundaryLaneStorage {
            live_rows: self.visible_len(),
            row_capacity: self.pages.len().saturating_mul(BOUNDARY_ROWS_PER_PAGE),
            live_restart_roots: self.live_roots,
            rows_released: self.rows_released,
            rows_reused: self.rows_reused,
        }
    }

    fn append(
        &mut self,
        checkpoint: EngineCheckpoint<G>,
        evidence: RetainedBoundaryEvidence,
    ) -> RetainedCheckpointKey {
        self.append_cell(Some(checkpoint), evidence)
    }

    fn append_evidence(&mut self, evidence: RetainedBoundaryEvidence) {
        let _ = self.append_cell(None, evidence);
    }

    fn append_cell(
        &mut self,
        checkpoint: Option<EngineCheckpoint<G>>,
        evidence: RetainedBoundaryEvidence,
    ) -> RetainedCheckpointKey {
        let has_restart = checkpoint.is_some();
        let protects_job_start = self.protected_record.is_none()
            && checkpoint
                .as_ref()
                .is_some_and(|checkpoint| checkpoint.boundary() == crate::EngineBoundary::JobStart);
        let record = self.next_record;
        self.next_record = self
            .next_record
            .checked_add(1)
            .expect("boundary record identity space exhausted");
        let key = self.allocate(BoundaryCell {
            evidence,
            checkpoint,
        });
        match &mut self.ownership {
            BoundaryOwnership::Accepted(accepted) => accepted.push_back(key),
            BoundaryOwnership::Forked { current, .. } => current.push_back(key),
        }
        if has_restart {
            self.live_roots = self.live_roots.saturating_add(1);
        }
        if protects_job_start {
            self.protected_record = Some(record);
        }
        RetainedCheckpointKey {
            owner: self.owner,
            slot: key.slot,
            generation: key.generation,
            record,
        }
    }

    fn add_page(&mut self) {
        let start = self.pages.len().saturating_mul(BOUNDARY_ROWS_PER_PAGE);
        let mut free = self.free_head;
        let slots = (0..BOUNDARY_ROWS_PER_PAGE)
            .map(|offset| {
                let slot = u32::try_from(start.saturating_add(offset))
                    .expect("boundary row pool fits u32");
                let row = BoundarySlot::vacant(free);
                free = Some(slot);
                row
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.pages.push(BoundaryPage { slots });
        self.free_head = free;
    }

    fn allocate(&mut self, cell: BoundaryCell<G>) -> BoundarySlotKey {
        if self.free_head.is_none() {
            self.add_page();
        }
        let slot = self.free_head.expect("boundary page supplied a free row");
        let reused = self.slot_by_index(slot).generation != 1;
        let free_next = self.slot_by_index(slot).free_next;
        self.free_head = free_next;
        let generation = {
            let row = self.slot_by_index_mut(slot);
            row.free_next = None;
            row.cell = Some(cell);
            row.generation
        };
        if reused {
            self.rows_reused = self.rows_reused.saturating_add(1);
        }
        BoundarySlotKey {
            slot,
            generation,
            record: self.next_record.saturating_sub(1),
        }
    }

    fn slot_by_index(&self, slot: u32) -> &BoundarySlot<G> {
        let slot = slot as usize;
        &self.pages[slot / BOUNDARY_ROWS_PER_PAGE].slots[slot % BOUNDARY_ROWS_PER_PAGE]
    }

    fn slot_by_index_mut(&mut self, slot: u32) -> &mut BoundarySlot<G> {
        let slot = slot as usize;
        &mut self.pages[slot / BOUNDARY_ROWS_PER_PAGE].slots[slot % BOUNDARY_ROWS_PER_PAGE]
    }

    fn raw_key(
        &self,
        key: &RetainedCheckpointKey,
    ) -> Result<BoundarySlotKey, RetainedEngineAccessError> {
        if key.owner != self.owner {
            return Err(RetainedEngineAccessError::ForeignGeneration);
        }
        Ok(BoundarySlotKey {
            slot: key.slot,
            generation: key.generation,
            record: key.record,
        })
    }

    fn cell(
        &self,
        key: &RetainedCheckpointKey,
    ) -> Result<&BoundaryCell<G>, RetainedEngineAccessError> {
        let raw = self.raw_key(key)?;
        let row = self
            .pages
            .get(raw.slot as usize / BOUNDARY_ROWS_PER_PAGE)
            .and_then(|page| page.slots.get(raw.slot as usize % BOUNDARY_ROWS_PER_PAGE))
            .filter(|row| row.generation == raw.generation)
            .ok_or(RetainedEngineAccessError::StaleCheckpoint)?;
        row.cell
            .as_ref()
            .filter(|_| self.visible_contains(raw))
            .ok_or(RetainedEngineAccessError::StaleCheckpoint)
    }

    fn get(
        &self,
        key: &RetainedCheckpointKey,
    ) -> Result<&EngineCheckpoint<G>, RetainedEngineAccessError> {
        self.cell(key)?
            .checkpoint
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleCheckpoint)
    }

    fn release_key(
        &mut self,
        key: RetainedCheckpointKey,
    ) -> Result<crate::EngineCheckpointRelease<G>, RetainedEngineAccessError> {
        self.get(&key)?;
        if self.protected_record == Some(key.record) {
            return Err(RetainedEngineAccessError::ProtectedCheckpoint);
        }
        let raw = self.raw_key(&key)?;
        if !self.remove_visible(raw) {
            return Err(RetainedEngineAccessError::StaleCheckpoint);
        }
        let cell = self.release_slot(raw)?;
        let removed = cell
            .checkpoint
            .expect("validated boundary row owns its restart root");
        self.live_roots = self.live_roots.saturating_sub(1);
        let mut oldest_retained = None;
        self.visit_visible_cells(|cell| {
            if oldest_retained.is_none()
                && let Some(checkpoint) = cell.checkpoint.as_ref()
                && checkpoint.boundary() != crate::EngineBoundary::JobStart
            {
                oldest_retained = Some(crate::checkpoint::CheckpointReleaseFloor::capture(
                    checkpoint,
                ));
            }
        })?;
        Ok(crate::EngineCheckpointRelease::new(
            removed,
            oldest_retained,
        ))
    }

    fn can_begin(&self, key: &RetainedCheckpointKey) -> bool {
        matches!(self.ownership, BoundaryOwnership::Accepted(_)) && self.get(key).is_ok()
    }

    fn select(
        &self,
        selected: Option<(usize, crate::EngineBoundary, u32)>,
    ) -> Result<(RetainedCheckpointKey, RetainedBoundaryEvidence), RetainedEngineAccessError> {
        let target = selected;
        let mut first_restart = None;
        let mut earlier_restart = None;
        let mut last_restart = None;
        let mut exact_seen = false;
        let mut exact_selection = None;
        self.visit_visible(|raw, cell| {
            let Some((position, boundary, ordinal)) = target else {
                if cell.checkpoint.is_some() {
                    first_restart.get_or_insert((raw, cell.evidence));
                }
                return;
            };
            if cell.evidence.position == position
                && cell.evidence.boundary == boundary
                && cell.evidence.ordinal == ordinal
            {
                exact_seen = true;
                exact_selection = cell
                    .checkpoint
                    .as_ref()
                    .map(|_| (raw, cell.evidence))
                    .or(last_restart);
            }
            if cell.checkpoint.is_some() {
                first_restart.get_or_insert((raw, cell.evidence));
                if cell.evidence.position < position {
                    earlier_restart = Some((raw, cell.evidence));
                }
                last_restart = Some((raw, cell.evidence));
            }
        })?;
        let (raw, evidence) = if target.is_none() {
            first_restart
        } else if exact_seen {
            exact_selection
        } else {
            earlier_restart.or(first_restart)
        }
        .ok_or(RetainedEngineAccessError::StaleCheckpoint)?;
        Ok((self.public_key(raw), evidence))
    }

    fn latest_restart_at_or_before(
        &self,
        position: usize,
    ) -> Result<Option<(RetainedCheckpointKey, RetainedBoundaryEvidence)>, RetainedEngineAccessError>
    {
        let mut selected = None;
        self.visit_visible(|raw, cell| {
            if cell.checkpoint.is_some() && cell.evidence.position <= position {
                selected = Some((raw, cell.evidence));
            }
        })?;
        Ok(selected.map(|(raw, evidence)| (self.public_key(raw), evidence)))
    }

    fn rehome_identical_revision(
        &mut self,
        revision: u64,
    ) -> Result<(), RetainedEngineAccessError> {
        let keys = match &self.ownership {
            BoundaryOwnership::Accepted(keys) => keys.iter().copied().collect::<Vec<_>>(),
            BoundaryOwnership::Forked { .. } => {
                return Err(RetainedEngineAccessError::LiveAttachment);
            }
        };
        for key in keys {
            let row = self.slot_by_index_mut(key.slot);
            if row.generation != key.generation {
                return Err(RetainedEngineAccessError::StaleCheckpoint);
            }
            row.cell
                .as_mut()
                .ok_or(RetainedEngineAccessError::StaleCheckpoint)?
                .evidence
                .revision = revision;
        }
        Ok(())
    }

    fn validate_revision_rehome(
        &self,
        plan: RetainedEditorRevisionRehomePlan,
    ) -> Result<(), RetainedEngineAccessError> {
        let RetainedEditorRevisionRehomePlan {
            old_start,
            old_end,
            new_end,
            restart,
            convergence,
            ..
        } = plan;
        let BoundaryOwnership::Accepted(keys) = &self.ownership else {
            return Err(RetainedEngineAccessError::LiveAttachment);
        };
        let mut restart_seen = false;
        let mut convergence_seen = false;
        for key in keys {
            let row = self.slot_by_index(key.slot);
            if row.generation != key.generation {
                return Err(RetainedEngineAccessError::StaleCheckpoint);
            }
            let evidence = row
                .cell
                .as_ref()
                .ok_or(RetainedEngineAccessError::StaleCheckpoint)?
                .evidence;
            let identity = (evidence.position, evidence.boundary, evidence.ordinal);
            restart_seen |= identity == restart;
            if identity == convergence {
                if !restart_seen {
                    return Err(RetainedEngineAccessError::StaleCheckpoint);
                }
                convergence_seen = true;
            }
            if convergence_seen
                && map_revision_offset(evidence.position, old_start, old_end, new_end).is_none()
            {
                return Err(RetainedEngineAccessError::StaleCheckpoint);
            }
        }
        if restart_seen && convergence_seen {
            Ok(())
        } else {
            Err(RetainedEngineAccessError::StaleCheckpoint)
        }
    }

    fn rehome_revision_suffix(
        &mut self,
        plan: RetainedEditorRevisionRehomePlan,
    ) -> Result<(), RetainedEngineAccessError> {
        let RetainedEditorRevisionRehomePlan {
            revision,
            old_start,
            old_end,
            new_end,
            restart,
            convergence,
            new_effect_prefix,
            new_artifact_prefix,
        } = plan;
        let BoundaryOwnership::Accepted(keys) = &mut self.ownership else {
            return Err(RetainedEngineAccessError::LiveAttachment);
        };
        let mut accepted = std::mem::take(keys);
        let mut retained = VecDeque::with_capacity(accepted.len());
        let mut after_restart = false;
        let mut adopting_suffix = false;
        let mut old_effect_prefix = 0;
        let mut old_artifact_prefix = 0;
        while let Some(key) = accepted.pop_front() {
            let evidence = self
                .slot_by_index(key.slot)
                .cell
                .as_ref()
                .ok_or(RetainedEngineAccessError::StaleCheckpoint)?
                .evidence;
            let identity = (evidence.position, evidence.boundary, evidence.ordinal);
            if identity == convergence {
                adopting_suffix = true;
                old_effect_prefix = evidence.effect_prefix;
                old_artifact_prefix = evidence.artifact_prefix;
            }
            if after_restart && !adopting_suffix {
                let cell = self.release_slot(key)?;
                self.live_roots = self
                    .live_roots
                    .saturating_sub(usize::from(cell.checkpoint.is_some()));
                continue;
            }
            {
                let row = self.slot_by_index_mut(key.slot);
                if row.generation != key.generation {
                    return Err(RetainedEngineAccessError::StaleCheckpoint);
                }
                let cell = row
                    .cell
                    .as_mut()
                    .ok_or(RetainedEngineAccessError::StaleCheckpoint)?;
                cell.evidence.revision = revision;
                if adopting_suffix {
                    cell.evidence.position =
                        map_revision_offset(cell.evidence.position, old_start, old_end, new_end)
                            .ok_or(RetainedEngineAccessError::StaleCheckpoint)?;
                    cell.evidence.effect_prefix = new_effect_prefix.saturating_add(
                        cell.evidence
                            .effect_prefix
                            .saturating_sub(old_effect_prefix),
                    );
                    cell.evidence.artifact_prefix = new_artifact_prefix.saturating_add(
                        cell.evidence
                            .artifact_prefix
                            .saturating_sub(old_artifact_prefix),
                    );
                    if let Some(checkpoint) = cell.checkpoint.as_mut() {
                        checkpoint.rehome_output_coordinates(
                            cell.evidence.position,
                            cell.evidence.effect_prefix,
                            cell.evidence.artifact_prefix,
                        );
                    }
                }
            }
            retained.push_back(key);
            after_restart |= identity == restart;
        }
        self.ownership = BoundaryOwnership::Accepted(retained);
        self.live_roots = self.visible_live_roots();
        Ok(())
    }

    fn validate_all(&self) -> Result<(), RetainedEngineAccessError> {
        self.visit_visible(|_, _| {})?;
        Ok(())
    }

    fn begin(&mut self, key: &RetainedCheckpointKey) {
        let raw = self.raw_key(key).expect("prevalidated boundary key owner");
        let BoundaryOwnership::Accepted(mut accepted) = std::mem::replace(
            &mut self.ownership,
            BoundaryOwnership::Accepted(VecDeque::new()),
        ) else {
            panic!("boundary lane already owns a candidate fork")
        };
        let selected = accepted
            .iter()
            .position(|candidate| *candidate == raw)
            .expect("prevalidated boundary key remains in accepted order");
        let detached_prior = accepted.split_off(selected.saturating_add(1));
        self.ownership = BoundaryOwnership::Forked {
            prefix: accepted,
            detached_prior,
            current: VecDeque::new(),
        };
        self.live_roots = self.visible_live_roots();
    }

    fn reject(&mut self) {
        let BoundaryOwnership::Forked {
            mut prefix,
            mut detached_prior,
            current,
        } = std::mem::replace(
            &mut self.ownership,
            BoundaryOwnership::Accepted(VecDeque::new()),
        )
        else {
            panic!("boundary rejection requires a candidate fork")
        };
        self.release_slots(current);
        prefix.append(&mut detached_prior);
        self.ownership = BoundaryOwnership::Accepted(prefix);
        self.live_roots = self.visible_live_roots();
    }

    fn accept(&mut self) {
        let BoundaryOwnership::Forked {
            mut prefix,
            detached_prior,
            mut current,
        } = std::mem::replace(
            &mut self.ownership,
            BoundaryOwnership::Accepted(VecDeque::new()),
        )
        else {
            panic!("boundary acceptance requires a candidate fork")
        };
        self.release_slots(detached_prior);
        prefix.append(&mut current);
        self.ownership = BoundaryOwnership::Accepted(prefix);
        self.live_roots = self.visible_live_roots();
    }

    fn pdf_history_low_water(&self) -> Option<(u64, u64)> {
        let mut low_water: Option<(u64, u64)> = None;
        self.visit_visible_cells(|cell| {
            let Some(checkpoint) = cell.checkpoint.as_ref() else {
                return;
            };
            let position = checkpoint.pdf_history_position();
            low_water = Some(low_water.map_or(position, |left| {
                (left.0.min(position.0), left.1.min(position.1))
            }));
        })
        .ok()?;
        low_water
    }

    fn prune(
        &mut self,
        retained: &[RetainedCheckpointKey],
    ) -> Result<CheckpointPruningReceipt, RetainedEngineAccessError> {
        for key in retained {
            self.get(key)?;
        }
        Ok(CheckpointPruningReceipt {
            released: 0,
            retained: self.live_roots,
            slots: self.visible_len(),
        })
    }

    fn public_key(&self, raw: BoundarySlotKey) -> RetainedCheckpointKey {
        RetainedCheckpointKey {
            owner: self.owner,
            slot: raw.slot,
            generation: raw.generation,
            record: raw.record,
        }
    }

    fn visible_contains(&self, key: BoundarySlotKey) -> bool {
        match &self.ownership {
            BoundaryOwnership::Accepted(accepted) => accepted.contains(&key),
            BoundaryOwnership::Forked {
                prefix, current, ..
            } => prefix.contains(&key) || current.contains(&key),
        }
    }

    fn remove_visible(&mut self, key: BoundarySlotKey) -> bool {
        let remove = |lane: &mut VecDeque<BoundarySlotKey>| {
            lane.iter()
                .position(|candidate| *candidate == key)
                .and_then(|position| lane.remove(position))
                .is_some()
        };
        match &mut self.ownership {
            BoundaryOwnership::Accepted(accepted) => remove(accepted),
            BoundaryOwnership::Forked {
                prefix, current, ..
            } => remove(prefix) || remove(current),
        }
    }

    fn release_slot(
        &mut self,
        key: BoundarySlotKey,
    ) -> Result<BoundaryCell<G>, RetainedEngineAccessError> {
        let free_head = self.free_head;
        let row = self.slot_by_index_mut(key.slot);
        if row.generation != key.generation {
            return Err(RetainedEngineAccessError::StaleCheckpoint);
        }
        let cell = row
            .cell
            .take()
            .ok_or(RetainedEngineAccessError::StaleCheckpoint)?;
        row.generation = row.generation.wrapping_add(1).max(1);
        row.free_next = free_head;
        self.free_head = Some(key.slot);
        self.rows_released = self.rows_released.saturating_add(1);
        Ok(cell)
    }

    fn release_slots(&mut self, mut keys: VecDeque<BoundarySlotKey>) {
        while let Some(key) = keys.pop_front() {
            drop(
                self.release_slot(key)
                    .expect("owned boundary suffix contains live rows"),
            );
        }
    }

    fn visit_visible(
        &self,
        mut visit: impl FnMut(BoundarySlotKey, &BoundaryCell<G>),
    ) -> Result<(), RetainedEngineAccessError> {
        let mut visit_lane = |lane: &VecDeque<BoundarySlotKey>| {
            for key in lane {
                let row = self.slot_by_index(key.slot);
                let cell = row
                    .cell
                    .as_ref()
                    .filter(|_| row.generation == key.generation)
                    .ok_or(RetainedEngineAccessError::StaleCheckpoint)?;
                visit(*key, cell);
            }
            Ok(())
        };
        match &self.ownership {
            BoundaryOwnership::Accepted(accepted) => visit_lane(accepted),
            BoundaryOwnership::Forked {
                prefix, current, ..
            } => {
                visit_lane(prefix)?;
                visit_lane(current)
            }
        }
    }

    fn visit_visible_cells(
        &self,
        mut visit: impl FnMut(&BoundaryCell<G>),
    ) -> Result<(), RetainedEngineAccessError> {
        self.visit_visible(|_, cell| visit(cell))
    }

    fn visible_len(&self) -> usize {
        match &self.ownership {
            BoundaryOwnership::Accepted(accepted) => accepted.len(),
            BoundaryOwnership::Forked {
                prefix, current, ..
            } => prefix.len().saturating_add(current.len()),
        }
    }

    fn visible_live_roots(&self) -> usize {
        let mut roots = 0_usize;
        self.visit_visible_cells(|cell| {
            roots = roots.saturating_add(usize::from(cell.checkpoint.is_some()));
        })
        .expect("boundary ownership contains only live rows");
        roots
    }
}

struct EngineGenerationSidecars<G> {
    generation: u64,
    attachment: Option<Box<dyn Any>>,
    boundaries: Option<BoundaryLane<G>>,
    command: Option<tex_command::CommandState<G>>,
    control: Option<crate::MainControl<G>>,
    ledger: Option<crate::OutputLedger>,
}

struct InitializeSidecars {
    generation: u64,
}

impl RetainedStateOperation for InitializeSidecars {
    type Output = RetainedAttachmentKey;

    fn run<G: 'static>(self, mut admitted: RetainedStateAdmission<'_, G>) -> Self::Output {
        admitted.attach(EngineGenerationSidecars::<G> {
            generation: self.generation,
            attachment: None,
            boundaries: Some(BoundaryLane::default()),
            command: None,
            control: None,
            ledger: Some(crate::OutputLedger::new()),
        })
    }
}

struct EngineOperationAdapter<'a, O> {
    generation: u64,
    sidecars: &'a RetainedAttachmentKey,
    operation: O,
}

impl<O: RetainedEngineOperation> RetainedStateOperation for EngineOperationAdapter<'_, O> {
    type Output = Result<O::Output, RetainedEngineAccessError>;

    fn run<G: 'static>(self, mut admitted: RetainedStateAdmission<'_, G>) -> Self::Output {
        let (universe, sidecars) =
            admitted.universe_and_attachment_mut::<EngineGenerationSidecars<G>>(self.sidecars)?;
        if sidecars.generation != self.generation {
            return Err(RetainedEngineAccessError::ForeignGeneration);
        }
        Ok(self.operation.run(AdmittedEngineGeneration {
            generation: self.generation,
            universe,
            sidecars,
        }))
    }
}

fn validate_attachment_key(
    generation: u64,
    key: &RetainedEngineAttachmentKey,
) -> Result<(), RetainedEngineAccessError> {
    if key.generation != generation {
        return Err(RetainedEngineAccessError::ForeignGeneration);
    }
    Ok(())
}

fn next_generation() -> u64 {
    NEXT_ENGINE_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .expect("retained engine generation identity space exhausted")
}

fn map_revision_offset(
    old: usize,
    old_start: usize,
    old_end: usize,
    new_end: usize,
) -> Option<usize> {
    if old <= old_start {
        Some(old)
    } else if old >= old_end {
        new_end.checked_add(old.checked_sub(old_end)?)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_command::{CommandProfile, CommandState, RegisteredSourceKind, SourceRegistration};
    use tex_state::env::AssignmentScope;
    use tex_state::interner::InternerBudget;

    fn store() -> ReachabilityStore {
        ReachabilityStore::new(
            InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024).expect("test budget"),
        )
    }

    struct Capture;

    impl RetainedEngineOperation for Capture {
        type Output = RetainedCheckpointKey;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let parked = admitted.sidecars.command.take();
            let (checkpoint, control) = {
                let universe = admitted.universe();
                let mut control = parked.map_or_else(
                    || crate::MainControl::tex82_initex(universe),
                    |command| {
                        crate::MainControl::from_checkpoint_fork(command, crate::ModeNest::new())
                    },
                );
                let checkpoint = control
                    .capture_checkpoint(
                        crate::EngineBoundary::JobStart,
                        universe,
                        crate::ExecutionBudgetCounters::default(),
                    )
                    .expect("quiescent checkpoint");
                (checkpoint, control)
            };
            admitted
                .prepare_checkpoint_control(control)
                .expect("cold command owner parks")
                .accept();
            admitted.retain_checkpoint(checkpoint)
        }
    }

    struct Read<'a>(&'a RetainedCheckpointKey);

    impl RetainedEngineOperation for Read<'_> {
        type Output = Result<crate::EngineBoundary, RetainedEngineAccessError>;

        fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            admitted
                .checkpoint(self.0)
                .map(crate::EngineCheckpoint::boundary)
        }
    }

    struct CaptureCount(i32);

    impl RetainedEngineOperation for CaptureCount {
        type Output = RetainedCheckpointKey;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            admitted
                .universe()
                .assign_count(0, self.0, AssignmentScope::Global)
                .expect("baseline count");
            let (checkpoint, control) = {
                let universe = admitted.universe();
                let mut control = crate::MainControl::tex82_initex(universe);
                let checkpoint = control
                    .capture_checkpoint(
                        crate::EngineBoundary::JobStart,
                        universe,
                        crate::ExecutionBudgetCounters {
                            committed_steps: 7,
                            cumulative_fuel: 11,
                        },
                    )
                    .expect("quiescent checkpoint");
                (checkpoint, control)
            };
            admitted
                .prepare_checkpoint_control(control)
                .expect("cold command owner parks")
                .accept();
            admitted.retain_checkpoint(checkpoint)
        }
    }

    struct ConsumeFork {
        runtime: RetainedEngineAttachmentKey,
        replacement: i32,
        accept_modes: bool,
    }

    impl RetainedEngineOperation for ConsumeFork {
        type Output = (i32, RetainedCheckpointKey);

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let _runtime = admitted
                .take_attachment::<RestoredCheckpointRuntime>(self.runtime)
                .expect("fork owns restored main control");
            let mut control = admitted
                .take_checkpoint_control()
                .expect("fork owns typed main control");
            let before = admitted
                .universe()
                .command_context()
                .expect("command context")
                .count(0)
                .expect("restored count");
            admitted
                .universe()
                .assign_count(0, self.replacement, AssignmentScope::Global)
                .expect("candidate mutation");
            let checkpoint = control
                .capture_checkpoint(
                    crate::EngineBoundary::JobStart,
                    admitted.universe(),
                    crate::ExecutionBudgetCounters::default(),
                )
                .expect("candidate checkpoint");
            let prepared = admitted
                .prepare_checkpoint_control(control)
                .expect("candidate command owner parks");
            if self.accept_modes {
                prepared.accept();
            } else {
                prepared.reject();
            }
            (before, admitted.retain_checkpoint(checkpoint))
        }
    }

    struct CaptureLoadedFormat;

    impl RetainedEngineOperation for CaptureLoadedFormat {
        type Output = RetainedCheckpointKey;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let mut control = crate::MainControl::with_profile(CommandProfile::TEX82);
            let checkpoint = control
                .capture_checkpoint(
                    crate::EngineBoundary::JobStart,
                    admitted.universe(),
                    crate::ExecutionBudgetCounters::default(),
                )
                .expect("loaded format checkpoint");
            admitted
                .prepare_checkpoint_control(control)
                .expect("loaded command owner parks")
                .accept();
            admitted.retain_checkpoint(checkpoint)
        }
    }

    struct CaptureResourceInput;

    impl RetainedEngineOperation for CaptureResourceInput {
        type Output = RetainedCheckpointKey;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let mut control = crate::MainControl::tex82_initex(admitted.universe());
            control
                .register_root_source(SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    std::sync::Arc::<[u8]>::from(&b"\\input child\\end"[..]),
                ))
                .expect("root source");
            let checkpoint = control
                .capture_checkpoint(
                    crate::EngineBoundary::JobStart,
                    admitted.universe(),
                    crate::ExecutionBudgetCounters::default(),
                )
                .expect("resource checkpoint");
            admitted
                .prepare_checkpoint_control(control)
                .expect("resource command owner parks")
                .accept();
            admitted.retain_checkpoint(checkpoint)
        }
    }

    struct SuspendFork(RetainedEngineAttachmentKey);

    impl RetainedEngineOperation for SuspendFork {
        type Output = RetainedEngineAttachmentKey;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let runtime = admitted
                .take_attachment::<RestoredCheckpointRuntime>(self.0)
                .expect("fork runtime");
            let mut control = admitted
                .take_checkpoint_control()
                .expect("fork owns typed main control");
            let step = control
                .advance_episode(admitted.universe())
                .expect("resource suspension");
            assert!(matches!(
                step,
                crate::StepResult::Suspended(crate::ResourceNeed::Input { ref name, .. })
                    if name == "child.tex"
            ));
            admitted
                .attach_with_checkpoint_control(runtime, control)
                .expect("suspended control reattaches")
        }
    }

    struct ResumeFork(RetainedEngineAttachmentKey);

    impl RetainedEngineOperation for ResumeFork {
        type Output = crate::StepResult;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let _runtime = admitted
                .take_attachment::<RestoredCheckpointRuntime>(self.0)
                .expect("suspended runtime");
            let mut control = admitted
                .take_checkpoint_control()
                .expect("suspension owns typed main control");
            control.capabilities_mut().register_input(
                "child.tex",
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    std::sync::Arc::<[u8]>::from(&b""[..]),
                ),
            );
            for _ in 0..8 {
                let step = control
                    .advance_episode(admitted.universe())
                    .expect("resource retry");
                if step == crate::StepResult::Progress(crate::MainControlStep::End) {
                    admitted
                        .prepare_checkpoint_control(control)
                        .expect("terminal control parks")
                        .accept();
                    return step;
                }
            }
            panic!("fulfilled resource input did not reach the terminal step")
        }
    }

    struct ReadCount;

    impl RetainedEngineOperation for ReadCount {
        type Output = i32;

        fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            admitted
                .universe
                .command_context()
                .expect("command context")
                .count(0)
                .expect("count")
        }
    }

    struct ReleaseCheckpoint(RetainedCheckpointKey);

    impl RetainedEngineOperation for ReleaseCheckpoint {
        type Output = Result<(), RetainedEngineAccessError>;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let (_universe, _ledger, mut checkpoints) = admitted.parts();
            checkpoints.release(self.0).map(drop)
        }
    }

    struct RestartFixture<G> {
        command: CommandState<G>,
        modes: crate::ModeNest,
    }

    struct CaptureRestorablePair;

    impl RetainedEngineOperation for CaptureRestorablePair {
        type Output = (
            RetainedCheckpointKey,
            RetainedCheckpointKey,
            RetainedEngineAttachmentKey,
        );

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let mut command = CommandState::default();
            let mut modes = crate::ModeNest::new();
            let first = {
                let universe = admitted.universe();
                universe
                    .assign_count(0, 10, AssignmentScope::Global)
                    .expect("baseline count");
                crate::EngineCheckpoint::capture_checkpoint(
                    crate::checkpoint::CheckpointEligibility::job_start(),
                    &mut command,
                    &mut modes,
                    universe,
                    crate::ExecutionBudgetCounters::default(),
                )
                .expect("first checkpoint")
            };
            let first = admitted.retain_checkpoint(first);
            let second = {
                let universe = admitted.universe();
                universe
                    .assign_count(0, 20, AssignmentScope::Global)
                    .expect("later count");
                crate::EngineCheckpoint::capture_checkpoint(
                    crate::checkpoint::CheckpointEligibility::named(
                        crate::EngineBoundary::OuterParagraphEnd,
                    ),
                    &mut command,
                    &mut modes,
                    universe,
                    crate::ExecutionBudgetCounters::default(),
                )
                .expect("second checkpoint")
            };
            let second = admitted.retain_checkpoint(second);
            let fixture = admitted.attach(RestartFixture { command, modes });
            (first, second, fixture)
        }
    }

    struct RestoreCount<'a> {
        checkpoint: &'a RetainedCheckpointKey,
        fixture: &'a RetainedEngineAttachmentKey,
    }

    impl RetainedEngineOperation for RestoreCount<'_> {
        type Output = i32;

        fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            validate_attachment_key(admitted.generation, self.fixture)
                .expect("fixture belongs to generation");
            let checkpoint = admitted
                .sidecars
                .boundaries
                .as_ref()
                .expect("generation owns its boundary lane")
                .get(self.checkpoint)
                .expect("surviving checkpoint");
            let fixture = admitted
                .sidecars
                .attachment
                .as_deref_mut()
                .expect("restart fixture")
                .downcast_mut::<RestartFixture<G>>()
                .expect("restart fixture type");
            checkpoint
                .restore_state(&mut fixture.command, &mut fixture.modes, admitted.universe)
                .expect("surviving checkpoint restores");
            admitted
                .universe
                .command_context()
                .expect("command context")
                .count(0)
                .expect("restored count")
        }
    }

    #[test]
    fn checkpoint_keys_are_owner_relative_across_live_generations() {
        let store = store();
        let mut first = RetainedEngineGeneration::new(&store, World::default()).expect("first");
        let key = first.with_admitted(Capture).expect("capture");
        assert_eq!(
            first.with_admitted(Read(&key)),
            Ok(Ok(crate::EngineBoundary::JobStart))
        );

        let mut second = RetainedEngineGeneration::new(&store, World::default()).expect("second");
        assert!(first.same_store(&second));
        assert_eq!(
            second.with_admitted(Read(&key)),
            Ok(Err(RetainedEngineAccessError::ForeignGeneration))
        );
    }

    #[test]
    fn retained_checkpoint_fork_rejects_and_reuses_the_current_slot() {
        let store = store();
        let mut accepted =
            RetainedEngineGeneration::new(&store, World::default()).expect("accepted");
        let checkpoint = accepted
            .with_admitted(CaptureCount(41))
            .expect("checkpoint");
        let accepted_witness = accepted.witness();

        let occupied =
            RetainedEngineGeneration::new(&store, World::default()).expect("occupied current slot");
        assert!(matches!(
            accepted.fork_checkpoint(&checkpoint),
            Err(RetainedEngineForkError::SlotsExhausted)
        ));
        assert_eq!(accepted.with_admitted(ReadCount), Ok(41));
        drop(occupied);

        let (mut rejected, runtime, counters) = accepted
            .fork_checkpoint(&checkpoint)
            .expect("first candidate fork");
        assert_eq!(
            accepted.with_admitted(ReadCount),
            Err(RetainedEngineAccessError::CandidateTransactionActive)
        );
        assert_eq!(counters.committed_steps, 7);
        assert_eq!(counters.cumulative_fuel, 11);
        let (before, _rejected_checkpoint) = rejected
            .with_admitted(ConsumeFork {
                runtime,
                replacement: 99,
                accept_modes: false,
            })
            .expect("candidate admission");
        assert_eq!(before, 41);
        drop(rejected);
        assert_eq!(
            accepted
                .with_admitted(ReadCount)
                .expect("accepted admission"),
            41,
            "candidate rejection leaves the accepted state unchanged"
        );

        let (mut accepted_candidate, runtime, _) = accepted
            .fork_checkpoint(&checkpoint)
            .expect("reused current slot");
        let (before, replacement_checkpoint) = accepted_candidate
            .with_admitted(ConsumeFork {
                runtime,
                replacement: 52,
                accept_modes: true,
            })
            .expect("candidate admission");
        assert_eq!(before, 41);
        accepted.prepare_candidate_accept(&mut accepted_candidate);
        accepted.finish_candidate_accept(&mut accepted_candidate);
        accepted.retire().expect("old prior retires");
        assert!(!accepted_witness.is_live());
        assert_eq!(
            accepted_candidate
                .with_admitted(ReadCount)
                .expect("replacement admission"),
            52,
            "the accepted replacement survives old-prior retirement"
        );

        let (mut prefix_restart, runtime, _) = accepted_candidate
            .fork_checkpoint(&checkpoint)
            .expect("unchanged prefix checkpoint survives acceptance");
        let (before, _checkpoint) = prefix_restart
            .with_admitted(ConsumeFork {
                runtime,
                replacement: 61,
                accept_modes: false,
            })
            .expect("prefix restart admission");
        assert_eq!(before, 41);
        drop(prefix_restart);
        assert_eq!(accepted_candidate.with_admitted(ReadCount), Ok(52));

        let (mut restarted, runtime, _) = accepted_candidate
            .fork_checkpoint(&replacement_checkpoint)
            .expect("later accepted restart");
        let (before, _checkpoint) = restarted
            .with_admitted(ConsumeFork {
                runtime,
                replacement: 73,
                accept_modes: false,
            })
            .expect("restart admission");
        assert_eq!(before, 52);
        drop(restarted);
        assert_eq!(accepted_candidate.with_admitted(ReadCount), Ok(52));
    }

    #[test]
    fn loaded_format_checkpoint_forks_the_first_document_without_retaining_the_image() {
        let image = crate::test_harness::with_tex82_universe(|universe| {
            universe
                .assign_count(0, 314, AssignmentScope::Global)
                .expect("format count");
            universe.capture_format_image().expect("format image")
        });
        let store = store();
        let mut accepted = RetainedEngineGeneration::from_format(&store, World::memory(), image)
            .expect("loaded format generation");
        let checkpoint = accepted
            .with_admitted(CaptureLoadedFormat)
            .expect("format checkpoint admission");
        let (mut current, runtime, _) = accepted
            .fork_checkpoint(&checkpoint)
            .expect("first document fork");
        let (before, _checkpoint) = current
            .with_admitted(ConsumeFork {
                runtime,
                replacement: 271,
                accept_modes: false,
            })
            .expect("first document admission");
        assert_eq!(before, 314);
        drop(current);
        assert_eq!(accepted.with_admitted(ReadCount), Ok(314));
    }

    #[test]
    fn forked_candidate_can_suspend_for_a_resource_and_resume_after_acceptance() {
        let store = store();
        let mut accepted =
            RetainedEngineGeneration::new(&store, World::memory()).expect("accepted generation");
        let checkpoint = accepted
            .with_admitted(CaptureResourceInput)
            .expect("resource checkpoint admission");
        let (mut current, runtime, _) = accepted
            .fork_checkpoint(&checkpoint)
            .expect("resource candidate fork");
        let suspension = current
            .with_admitted(SuspendFork(runtime))
            .expect("suspension admission");

        accepted.prepare_candidate_accept(&mut current);
        accepted.finish_candidate_accept(&mut current);
        accepted.retire().expect("accept current generation");
        let resumed = current
            .with_admitted(ResumeFork(suspension))
            .expect("resume admission");
        assert_eq!(
            resumed,
            crate::StepResult::Progress(crate::MainControlStep::End)
        );
    }

    #[test]
    fn dropping_a_resource_suspension_returns_the_attempt_before_reforking() {
        let store = store();
        let mut accepted =
            RetainedEngineGeneration::new(&store, World::memory()).expect("accepted generation");
        let checkpoint = accepted
            .with_admitted(CaptureResourceInput)
            .expect("resource checkpoint admission");
        let (mut current, runtime, _) = accepted
            .fork_checkpoint(&checkpoint)
            .expect("first resource candidate fork");
        let _suspension = current
            .with_admitted(SuspendFork(runtime))
            .expect("first suspension admission");

        drop(current);

        let (mut retry, runtime, _) = accepted
            .fork_checkpoint(&checkpoint)
            .expect("rejection returned the command and state owners");
        let _suspension = retry
            .with_admitted(SuspendFork(runtime))
            .expect("retry suspension does not nest the discarded attempt");
        drop(retry);
    }

    #[test]
    fn releasing_a_restart_root_preserves_earlier_boundary_evidence() {
        let store = store();
        let mut generation =
            RetainedEngineGeneration::new(&store, World::default()).expect("generation");
        let survivor = generation.with_admitted(Capture).expect("survivor");
        let stale = generation
            .with_admitted(Capture)
            .expect("discarded checkpoint");

        generation
            .with_admitted(ReleaseCheckpoint(stale))
            .expect("generation admission")
            .expect("release later restart root");
        assert_eq!(
            generation.with_admitted(Read(&survivor)),
            Ok(Ok(crate::EngineBoundary::JobStart))
        );
    }

    #[test]
    fn surviving_checkpoint_restarts_identically_after_newer_root_pruning() {
        let store = store();
        let mut generation =
            RetainedEngineGeneration::new(&store, World::default()).expect("generation");
        let (survivor, discarded, fixture) = generation
            .with_admitted(CaptureRestorablePair)
            .expect("checkpoint pair");

        generation
            .with_admitted(ReleaseCheckpoint(discarded))
            .expect("generation admission")
            .expect("release newer restart root");
        assert_eq!(
            generation.with_admitted(RestoreCount {
                checkpoint: &survivor,
                fixture: &fixture,
            }),
            Ok(10)
        );
    }

    #[test]
    fn boundary_lane_reuses_rows_and_indexes_evidence_only_fallbacks() {
        crate::test_harness::with_nonstop_universe(|universe| {
            let mut command = CommandState::default();
            let mut modes = crate::ModeNest::new();
            let mut lane = BoundaryLane::default();
            let job = crate::EngineCheckpoint::capture_checkpoint(
                crate::checkpoint::CheckpointEligibility::job_start(),
                &mut command,
                &mut modes,
                universe,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("job-start checkpoint");
            let job = lane.append(
                job,
                RetainedBoundaryEvidence::new(1, 0, crate::EngineBoundary::JobStart, 0, 0, 0, None),
            );
            let job_slot = (job.owner, job.slot, job.generation);

            for ordinal in 1..=128 {
                lane.append_evidence(RetainedBoundaryEvidence::new(
                    1,
                    1,
                    crate::EngineBoundary::ShipoutComplete,
                    ordinal,
                    ordinal as usize,
                    ordinal as usize,
                    None,
                ));
            }
            assert_eq!(lane.visible_len(), 129);
            let (fallback, evidence) = lane
                .select(Some((1, crate::EngineBoundary::ShipoutComplete, 128)))
                .expect("completion evidence falls back to its restart");
            assert_eq!(fallback, job);
            assert_eq!(evidence.boundary(), crate::EngineBoundary::JobStart);

            lane.begin(&job);
            let replacement = crate::EngineCheckpoint::capture_checkpoint(
                crate::checkpoint::CheckpointEligibility::named(
                    crate::EngineBoundary::OuterParagraphEnd,
                ),
                &mut command,
                &mut modes,
                universe,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("replacement checkpoint");
            let replacement = lane.append(
                replacement,
                RetainedBoundaryEvidence::new(
                    2,
                    2,
                    crate::EngineBoundary::OuterParagraphEnd,
                    0,
                    0,
                    0,
                    None,
                ),
            );
            lane.accept();
            assert_eq!((job.owner, job.slot, job.generation), job_slot);
            assert!(lane.get(&job).is_ok());
            assert!(lane.get(&replacement).is_ok());
            assert_eq!(lane.visible_len(), 2);

            lane.begin(&job);
            lane.append_evidence(RetainedBoundaryEvidence::new(
                3,
                3,
                crate::EngineBoundary::ShipoutComplete,
                0,
                0,
                0,
                None,
            ));
            lane.reject();
            assert!(lane.get(&job).is_ok());
            assert!(lane.get(&replacement).is_ok());
            assert_eq!(lane.visible_len(), 2);
            assert_eq!(lane.rows_released, 129);
            assert!(lane.pages.len() <= 9, "fixed row pages plateau and recycle");
        });
    }

    #[test]
    fn boundary_release_plateaus_over_thousands_of_named_boundaries() {
        crate::test_harness::with_nonstop_universe(|universe| {
            let mut control = crate::MainControl::tex82_initex(universe);
            let mut lane = BoundaryLane::default();
            let job = control
                .capture_job_start_checkpoint(universe, crate::ExecutionBudgetCounters::default())
                .expect("job-start checkpoint");
            let _job = lane.append(
                job,
                RetainedBoundaryEvidence::new(1, 0, crate::EngineBoundary::JobStart, 0, 0, 0, None),
            );
            let mut previous = None;
            let mut stale = None;
            for ordinal in 0..4_096_u32 {
                let checkpoint = control
                    .capture_checkpoint(
                        crate::EngineBoundary::OuterParagraphEnd,
                        universe,
                        crate::ExecutionBudgetCounters::default(),
                    )
                    .expect("quiescent paragraph checkpoint");
                let current = lane.append(
                    checkpoint,
                    RetainedBoundaryEvidence::new(
                        1,
                        ordinal as usize + 1,
                        crate::EngineBoundary::OuterParagraphEnd,
                        ordinal,
                        0,
                        0,
                        None,
                    ),
                );
                if let Some(previous) = previous.replace(current) {
                    stale = Some(RetainedCheckpointKey {
                        owner: previous.owner,
                        slot: previous.slot,
                        generation: previous.generation,
                        record: previous.record,
                    });
                    lane.release_key(previous)
                        .expect("oldest ordinary boundary releases")
                        .apply(&mut control, universe);
                }
            }

            let storage = lane.storage();
            assert_eq!(storage.live_rows, 2);
            assert_eq!(storage.live_restart_roots, 2);
            assert_eq!(storage.row_capacity, BOUNDARY_ROWS_PER_PAGE);
            assert!(storage.rows_released >= 4_095);
            assert!(storage.rows_reused >= 4_094);
            assert!(matches!(
                lane.get(stale.as_ref().expect("at least one released key")),
                Err(RetainedEngineAccessError::StaleCheckpoint)
            ));
        });
    }
}
