//! Opaque retained executor generations and owner-relative checkpoint keys.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use tex_state::fork_arena::{ArenaListId, CheckpointMark, ChunkPool, ForkArena};
use tex_state::{
    DetachedFormatImage, FormatError, ReachabilityStore, RetainedAttachmentKey,
    RetainedStateAccessError, RetainedStateAdmission, RetainedStateCandidateOperation,
    RetainedStateForkBuild, RetainedStateForkError, RetainedStateForkOperation,
    RetainedStateGeneration, RetainedStateOperation, RetainedStateRetirement, SessionEpochError,
    Universe, UniverseError, World,
};

use crate::EngineCheckpoint;

static NEXT_ENGINE_GENERATION: AtomicU64 = AtomicU64::new(1);

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

impl<G> AdmittedEngineGeneration<'_, G> {
    pub fn universe(&mut self) -> &mut Universe<G> {
        self.universe
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

    /// Releases one restart root during publication-time budget enforcement.
    /// Detached boundary evidence is owned by the incremental layer and is
    /// intentionally unaffected.
    pub fn release(&mut self, key: RetainedCheckpointKey) -> Result<(), RetainedEngineAccessError> {
        self.boundaries.release_key(key)
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

pub enum BoundaryLaneKind {}

/// Private owner-relative identity of one named retained checkpoint.
///
/// The key is non-`Copy`, non-`Clone`, and non-serializable. Detached boundary
/// identity remains in tex-incr's `BoundaryRecord` instead.
#[derive(Debug)]
pub struct RetainedCheckpointKey {
    list: ArenaListId<BoundaryLaneKind>,
    mark: CheckpointMark<BoundaryLaneKind>,
    record: usize,
}

impl PartialEq for RetainedCheckpointKey {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list
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

/// Direct owners restored together from one aggregate checkpoint.
#[doc(hidden)]
pub struct RestoredCheckpointRuntime<G> {
    pub control: crate::MainControl<G>,
}

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
        let generation = next_generation();
        let mut state = RetainedStateGeneration::from_format_owned(store, world, image)?;
        let sidecars = state.with_admitted(InitializeSidecars { generation });
        Ok(Self {
            generation,
            state: Some(state),
            sidecars,
            liveness: Arc::new(()),
        })
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
    ) -> Result<
        (
            Self,
            RetainedEngineAttachmentKey,
            crate::ExecutionBudgetCounters,
            RetainedBoundaryEvidence,
        ),
        RetainedEngineForkError,
    > {
        let selected = self
            .with_admitted(SelectBoundary { selected })
            .map_err(RetainedEngineForkError::Access)?;
        let (key, evidence) = selected.map_err(RetainedEngineForkError::Access)?;
        let (generation, runtime, counters) = self.fork_checkpoint(&key)?;
        Ok((generation, runtime, counters, evidence))
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
        candidate
            .state
            .as_mut()
            .expect("a live candidate owns state")
            .with_candidate_source(SettleOutputLedger::Accept)
            .expect("the accepted/current pair settles one output owner")
            .expect("the accepted/current sidecars own the output ledger");
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
            state
                .with_candidate_source(SettleOutputLedger::Reject)
                .expect("the rejected current pair returns one output owner")
                .expect("the accepted/current sidecars own the output ledger");
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
        let source = source.sole_attachment_mut::<EngineGenerationSidecars<G>>()?;
        let candidate = candidate.sole_attachment_mut::<EngineGenerationSidecars<G>>()?;
        let ledger = candidate
            .ledger
            .as_mut()
            .ok_or(RetainedStateAccessError::StaleAttachment)?;
        match self {
            Self::Accept => {
                candidate
                    .boundaries
                    .as_mut()
                    .ok_or(RetainedStateAccessError::StaleAttachment)?
                    .accept();
                ledger.accept_checkpoint_candidate();
            }
            Self::Reject => {
                candidate
                    .boundaries
                    .as_mut()
                    .ok_or(RetainedStateAccessError::StaleAttachment)?
                    .reject();
                ledger.reject_checkpoint_candidate();
                source.boundaries = candidate.boundaries.take();
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
        let (universe, control) = match checkpoint.fork_state(universe, &mut ledger) {
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
                attachment: Some(Box::new(RestoredCheckpointRuntime { control })),
                boundaries: Some(boundaries),
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
    type Output =
        Result<(RetainedCheckpointKey, RetainedBoundaryEvidence), RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        admitted
            .sidecars
            .boundaries
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .select(self.selected)
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
    previous_restart: Option<usize>,
}

struct BoundaryLane<G> {
    pool: ChunkPool<BoundaryCell<G>>,
    arena: ForkArena<BoundaryCell<G>, BoundaryLaneKind>,
    head: Option<CheckpointMark<BoundaryLaneKind>>,
    prior_head: Option<CheckpointMark<BoundaryLaneKind>>,
    prior_records: Option<usize>,
    prior_live_roots: Option<usize>,
    last_restart: Option<usize>,
    prior_last_restart: Option<usize>,
    records: usize,
    live_roots: usize,
}

impl<G> Default for BoundaryLane<G> {
    fn default() -> Self {
        Self {
            pool: ChunkPool::with_chunk_bytes(
                core::mem::size_of::<Option<BoundaryCell<G>>>().max(1),
            ),
            arena: ForkArena::new(),
            head: None,
            prior_head: None,
            prior_records: None,
            prior_live_roots: None,
            last_restart: None,
            prior_last_restart: None,
            records: 0,
            live_roots: 0,
        }
    }
}

impl<G> BoundaryLane<G> {
    fn append(
        &mut self,
        checkpoint: EngineCheckpoint<G>,
        evidence: RetainedBoundaryEvidence,
    ) -> RetainedCheckpointKey {
        self.append_cell(Some(checkpoint), evidence)
    }

    #[cfg(test)]
    fn append_evidence(&mut self, evidence: RetainedBoundaryEvidence) {
        let _ = self.append_cell(None, evidence);
    }

    fn append_cell(
        &mut self,
        checkpoint: Option<EngineCheckpoint<G>>,
        evidence: RetainedBoundaryEvidence,
    ) -> RetainedCheckpointKey {
        let has_restart = checkpoint.is_some();
        let record = self.records;
        let mut builder = self
            .arena
            .begin_builder(&mut self.pool)
            .expect("boundary lane owns the sole cell builder");
        builder
            .push(BoundaryCell {
                evidence,
                checkpoint,
                previous_restart: self.last_restart,
            })
            .expect("one boundary cell fits its logical chunk");
        let list = builder.seal().expect("boundary cell seals canonically");
        let boundary = self
            .arena
            .seal_boundary(&mut self.pool)
            .expect("boundary cell retires its builder");
        let mark = self
            .arena
            .checkpoint_mark(boundary)
            .expect("boundary cell names its sealed whole-chunk mark");
        self.head = Some(mark);
        self.records = self.records.saturating_add(1);
        if has_restart {
            self.live_roots = self.live_roots.saturating_add(1);
            self.last_restart = Some(record);
        }
        RetainedCheckpointKey { list, mark, record }
    }

    fn cell(
        &self,
        key: &RetainedCheckpointKey,
    ) -> Result<&BoundaryCell<G>, RetainedEngineAccessError> {
        let view = self
            .arena
            .list(&self.pool, key.list)
            .map_err(|error| match error {
                tex_state::fork_arena::ForkArenaError::ForeignArena => {
                    RetainedEngineAccessError::ForeignGeneration
                }
                _ => RetainedEngineAccessError::StaleCheckpoint,
            })?;
        view.get(0)
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

    fn release_key(&mut self, key: RetainedCheckpointKey) -> Result<(), RetainedEngineAccessError> {
        self.get(&key)?;
        let previous_restart = self.cell(&key)?.previous_restart;
        let removed = self
            .arena
            .with_single_value_mut(&mut self.pool, key.list, |cell| cell.checkpoint.take())
            .map_err(|_| RetainedEngineAccessError::StaleCheckpoint)?;
        drop(removed.expect("validated boundary cell owns its restart root"));
        self.live_roots = self.live_roots.saturating_sub(1);
        if self.last_restart == Some(key.record) {
            self.last_restart = previous_restart;
        }
        Ok(())
    }

    fn can_begin(&self, key: &RetainedCheckpointKey) -> bool {
        self.get(key).is_ok() && self.arena.can_begin_checkpoint_candidate(key.mark)
    }

    fn select(
        &self,
        selected: Option<(usize, crate::EngineBoundary, u32)>,
    ) -> Result<(RetainedCheckpointKey, RetainedBoundaryEvidence), RetainedEngineAccessError> {
        let len = self
            .arena
            .sealed_single_len()
            .map_err(|_| RetainedEngineAccessError::StaleCheckpoint)?;
        let index = if let Some((position, boundary, ordinal)) = selected {
            let mut low = 0;
            let mut high = len;
            while low < high {
                let middle = low + (high - low) / 2;
                let (_, _, cell) = self
                    .arena
                    .sealed_single_at(&self.pool, middle)
                    .map_err(|_| RetainedEngineAccessError::StaleCheckpoint)?;
                if cell.evidence.position <= position {
                    low = middle.saturating_add(1);
                } else {
                    high = middle;
                }
            }
            let mut exact = None;
            while low != 0 {
                low -= 1;
                let (_, _, cell) = self
                    .arena
                    .sealed_single_at(&self.pool, low)
                    .map_err(|_| RetainedEngineAccessError::StaleCheckpoint)?;
                if cell.evidence.position != position {
                    break;
                }
                if cell.evidence.boundary == boundary && cell.evidence.ordinal == ordinal {
                    exact = Some(low);
                    break;
                }
            }
            exact.ok_or(RetainedEngineAccessError::StaleCheckpoint)?
        } else {
            0
        };
        let index = self.restart_at_or_before(index)?;
        let (list, mark, cell) = self
            .arena
            .sealed_single_at(&self.pool, index)
            .map_err(|_| RetainedEngineAccessError::StaleCheckpoint)?;
        Ok((
            RetainedCheckpointKey {
                list,
                mark,
                record: index,
            },
            cell.evidence,
        ))
    }

    fn restart_at_or_before(&self, mut index: usize) -> Result<usize, RetainedEngineAccessError> {
        loop {
            let (_, _, cell) = self
                .arena
                .sealed_single_at(&self.pool, index)
                .map_err(|_| RetainedEngineAccessError::StaleCheckpoint)?;
            if cell.checkpoint.is_some() {
                return Ok(index);
            }
            index = cell
                .previous_restart
                .ok_or(RetainedEngineAccessError::StaleCheckpoint)?;
        }
    }

    fn validate_all(&self) -> Result<(), RetainedEngineAccessError> {
        let Some(head) = self.head else {
            return Ok(());
        };
        (self.arena.validates_checkpoint(head)
            && self
                .arena
                .sealed_single_len()
                .is_ok_and(|records| records == self.records))
        .then_some(())
        .ok_or(RetainedEngineAccessError::StaleCheckpoint)
    }

    fn begin(&mut self, key: &RetainedCheckpointKey) {
        self.prior_head = self.head;
        self.prior_records = Some(self.records);
        self.prior_live_roots = Some(self.live_roots);
        self.prior_last_restart = self.last_restart;
        let mut detached_records = 0_usize;
        let mut detached_live_roots = 0_usize;
        self.arena
            .visit_accepted_checkpoint_suffix(&self.pool, key.mark, |cell| {
                detached_records = detached_records.saturating_add(1);
                detached_live_roots =
                    detached_live_roots.saturating_add(usize::from(cell.checkpoint.is_some()));
            })
            .expect("prevalidated boundary mark visits only its detached suffix");
        self.arena
            .begin_checkpoint_candidate(key.mark)
            .expect("prevalidated boundary mark begins the sole metadata fork");
        self.head = Some(key.mark);
        self.records = self.records.saturating_sub(detached_records);
        self.live_roots = self.live_roots.saturating_sub(detached_live_roots);
        self.last_restart = Some(
            self.restart_at_or_before(key.record)
                .expect("selected boundary retains one restart root"),
        );
    }

    fn reject(&mut self) {
        let prior_head = self
            .prior_head
            .take()
            .expect("a rejected boundary lane retains its accepted head");
        let boundary = self
            .arena
            .seal_boundary(&mut self.pool)
            .expect("rejected boundary lane has no live builder");
        self.arena
            .reject_checkpoint_candidate(&mut self.pool, boundary)
            .expect("rejected boundary lane reattaches its prior suffix");
        self.head = Some(prior_head);
        self.records = self
            .prior_records
            .take()
            .expect("a rejected boundary lane retains its accepted record count");
        self.live_roots = self
            .prior_live_roots
            .take()
            .expect("a rejected boundary lane retains its accepted root count");
        self.last_restart = self.prior_last_restart.take();
    }

    fn accept(&mut self) {
        let boundary = self
            .arena
            .seal_boundary(&mut self.pool)
            .expect("accepted boundary lane has no live builder");
        self.arena
            .accept_checkpoint_candidate(&mut self.pool, boundary)
            .expect("accepted boundary lane drops its detached prior suffix");
        self.prior_head = None;
        self.prior_records = None;
        self.prior_live_roots = None;
        self.prior_last_restart = None;
    }

    fn pdf_history_low_water(&self) -> Option<(u64, u64)> {
        let mut low_water: Option<(u64, u64)> = None;
        let head = self.head?;
        self.arena
            .visit_checkpoint_values(&self.pool, head, |cell| {
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
            slots: self.records,
        })
    }
}

struct EngineGenerationSidecars<G> {
    generation: u64,
    attachment: Option<Box<dyn Any>>,
    boundaries: Option<BoundaryLane<G>>,
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
            let checkpoint = {
                let universe = admitted.universe();
                let mut control = crate::MainControl::tex82_initex(universe);
                control
                    .capture_checkpoint(
                        crate::EngineBoundary::JobStart,
                        universe,
                        crate::ExecutionBudgetCounters::default(),
                    )
                    .expect("quiescent checkpoint")
            };
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
            let checkpoint = {
                let universe = admitted.universe();
                let mut control = crate::MainControl::tex82_initex(universe);
                control
                    .capture_checkpoint(
                        crate::EngineBoundary::JobStart,
                        universe,
                        crate::ExecutionBudgetCounters {
                            committed_steps: 7,
                            cumulative_fuel: 11,
                        },
                    )
                    .expect("quiescent checkpoint")
            };
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
            let RestoredCheckpointRuntime { mut control } = admitted
                .take_attachment::<RestoredCheckpointRuntime<G>>(self.runtime)
                .expect("fork owns restored main control");
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
            if self.accept_modes {
                control.accept_checkpoint_candidate();
            } else {
                control.reject_checkpoint_candidate();
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
            admitted.retain_checkpoint(checkpoint)
        }
    }

    struct SuspendFork(RetainedEngineAttachmentKey);

    impl RetainedEngineOperation for SuspendFork {
        type Output = RetainedEngineAttachmentKey;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let RestoredCheckpointRuntime { mut control } = admitted
                .take_attachment::<RestoredCheckpointRuntime<G>>(self.0)
                .expect("fork runtime");
            let step = control
                .advance_episode(admitted.universe())
                .expect("resource suspension");
            assert!(matches!(
                step,
                crate::StepResult::Suspended(crate::ResourceNeed::Input { ref name, .. })
                    if name == "child.tex"
            ));
            admitted.attach(RestoredCheckpointRuntime { control })
        }
    }

    struct ResumeFork(RetainedEngineAttachmentKey);

    impl RetainedEngineOperation for ResumeFork {
        type Output = crate::StepResult;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let RestoredCheckpointRuntime { mut control } = admitted
                .take_attachment::<RestoredCheckpointRuntime<G>>(self.0)
                .expect("suspended runtime");
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
            checkpoints.release(self.0)
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
                    crate::checkpoint::CheckpointEligibility::outer_paragraph_end(),
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
    fn boundary_lane_keeps_prefix_marks_and_indexes_evidence_only_fallbacks() {
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
            let job_list = job.list;

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
            let (_, _, newest_evidence) = lane
                .arena
                .sealed_single_at(&lane.pool, 128)
                .expect("newest evidence cell");
            assert_eq!(
                newest_evidence.previous_restart,
                Some(0),
                "each evidence-only record stores the nearest restart coordinate at append"
            );
            let (fallback, evidence) = lane
                .select(Some((1, crate::EngineBoundary::ShipoutComplete, 128)))
                .expect("completion evidence falls back to its restart");
            assert_eq!(fallback, job);
            assert_eq!(evidence.boundary(), crate::EngineBoundary::JobStart);

            lane.begin(&job);
            let replacement = crate::EngineCheckpoint::capture_checkpoint(
                crate::checkpoint::CheckpointEligibility::outer_paragraph_end(),
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
            assert_eq!(job.list, job_list);
            assert!(lane.arena.validates_checkpoint(job.mark));
            assert!(lane.get(&job).is_ok());
            assert!(lane.get(&replacement).is_ok());

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
            assert_eq!(lane.arena.counters().source_nodes_copied, 0);
        });
    }
}
