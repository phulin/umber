//! Opaque retained executor generations and owner-relative checkpoint keys.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use tex_state::{
    DetachedFormatImage, FormatError, ReachabilityStore, RetainedAttachmentKey,
    RetainedStateAccessError, RetainedStateAdmission, RetainedStateGeneration,
    RetainedStateOperation, RetainedStateRetirement, SessionEpochError, Universe, UniverseError,
    World,
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
    pub fn parts(&mut self) -> (&mut Universe<G>, RetainedCheckpointStore<'_, G>) {
        (
            self.universe,
            RetainedCheckpointStore {
                generation: self.generation,
                checkpoints: &mut self.sidecars.checkpoints,
            },
        )
    }

    pub fn retain_checkpoint(&mut self, checkpoint: EngineCheckpoint<G>) -> RetainedCheckpointKey {
        self.sidecars
            .checkpoints
            .retain(self.generation, checkpoint)
    }

    pub fn checkpoint(
        &self,
        key: &RetainedCheckpointKey,
    ) -> Result<&EngineCheckpoint<G>, RetainedEngineAccessError> {
        self.sidecars.checkpoints.get(self.generation, key)
    }

    pub fn attach<T: Send + 'static>(&mut self, attachment: T) -> RetainedEngineAttachmentKey {
        assert!(
            self.sidecars.attachment.is_none(),
            "one retained engine generation accepts one suspended runtime"
        );
        self.sidecars.attachment = Some(Box::new(attachment));
        RetainedEngineAttachmentKey {
            generation: self.generation,
        }
    }

    pub fn attachment_mut<T: Send + 'static>(
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

    pub fn take_attachment<T: Send + 'static>(
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
    generation: u64,
    checkpoints: &'a mut RetainedCheckpointSlots<G>,
}

impl<G> RetainedCheckpointStore<'_, G> {
    pub fn retain(&mut self, checkpoint: EngineCheckpoint<G>) -> RetainedCheckpointKey {
        self.checkpoints.retain(self.generation, checkpoint)
    }
}

/// Private owner-relative identity of one named retained checkpoint.
///
/// The key is non-`Copy`, non-`Clone`, and non-serializable. Detached boundary
/// identity remains in tex-incr's `BoundaryRecord` instead.
#[derive(Debug, Eq, PartialEq)]
pub struct RetainedCheckpointKey {
    generation: u64,
    slot: usize,
    serial: u64,
}

/// Owner-relative key for one unpublished executor episode sidecar.
#[derive(Debug, Eq, PartialEq)]
pub struct RetainedEngineAttachmentKey {
    generation: u64,
}

/// Mutation-free retained executor admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedEngineAccessError {
    ForeignGeneration,
    StaleCheckpoint,
    StaleAttachment,
    AttachmentTypeMismatch,
    LiveAttachment,
    State(RetainedStateAccessError),
}

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
pub struct RetainedEngineGeneration {
    generation: u64,
    state: RetainedStateGeneration,
    sidecars: RetainedAttachmentKey,
    liveness: Arc<()>,
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

impl core::fmt::Debug for RetainedEngineGeneration {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RetainedEngineGeneration")
            .field("checkpoints", &self.state.attachment_count())
            .finish_non_exhaustive()
    }
}

impl RetainedEngineGeneration {
    pub fn new(store: &ReachabilityStore, world: World) -> Result<Self, SessionEpochError> {
        let generation = next_generation();
        let mut state = RetainedStateGeneration::new(store, world)?;
        let sidecars = state.with_admitted(InitializeSidecars { generation });
        Ok(Self {
            generation,
            state,
            sidecars,
            liveness: Arc::new(()),
        })
    }

    pub fn from_format(
        store: &ReachabilityStore,
        world: World,
        image: &DetachedFormatImage,
    ) -> Result<Self, FormatError> {
        let generation = next_generation();
        let mut state = RetainedStateGeneration::from_format(store, world, image)?;
        let sidecars = state.with_admitted(InitializeSidecars { generation });
        Ok(Self {
            generation,
            state,
            sidecars,
            liveness: Arc::new(()),
        })
    }

    pub fn with_admitted<O: RetainedEngineOperation>(
        &mut self,
        operation: O,
    ) -> Result<O::Output, RetainedEngineAccessError> {
        self.state.with_admitted(EngineOperationAdapter {
            generation: self.generation,
            sidecars: &self.sidecars,
            operation,
        })
    }

    #[must_use]
    pub fn witness(&self) -> RetainedEngineGenerationWitness {
        RetainedEngineGenerationWitness(Arc::downgrade(&self.liveness))
    }

    /// Whether two generations reside in the same external session store.
    #[must_use]
    pub fn same_store(&self, other: &Self) -> bool {
        self.state.same_store(&other.state)
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
    /// retained key set. Immutable generation rows remain append-only.
    pub fn prune_checkpoints(
        &mut self,
        retained: &[RetainedCheckpointKey],
    ) -> Result<CheckpointPruningReceipt, RetainedEngineAccessError> {
        self.with_admitted(PruneCheckpoints { retained })?
    }

    pub fn retire(self) -> Result<RetainedEngineRetirement, UniverseError> {
        Ok(RetainedEngineRetirement {
            state: self.state.retire()?,
        })
    }
}

struct PruneCheckpoints<'a> {
    retained: &'a [RetainedCheckpointKey],
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
                .checkpoints
                .get(admitted.generation, key)?;
        }
        Ok(())
    }
}

impl RetainedEngineOperation for PruneCheckpoints<'_> {
    type Output = Result<CheckpointPruningReceipt, RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        admitted
            .sidecars
            .checkpoints
            .prune(admitted.generation, self.retained)
    }
}

struct RetainedCheckpointSlots<G> {
    slots: Vec<RetainedCheckpointSlot<G>>,
    live: Vec<usize>,
    free: Vec<usize>,
    next_serial: u64,
}

impl<G> Default for RetainedCheckpointSlots<G> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            live: Vec::new(),
            free: Vec::new(),
            next_serial: 0,
        }
    }
}

struct RetainedCheckpointSlot<G> {
    checkpoint: Option<EngineCheckpoint<G>>,
    serial: u64,
    live_index: Option<usize>,
    keep: bool,
}

impl<G> RetainedCheckpointSlots<G> {
    fn retain(
        &mut self,
        generation: u64,
        checkpoint: EngineCheckpoint<G>,
    ) -> RetainedCheckpointKey {
        self.next_serial = self
            .next_serial
            .checked_add(1)
            .expect("retained checkpoint serial space is exhausted");
        let serial = self.next_serial;
        let slot = if let Some(slot) = self.free.pop() {
            let row = &mut self.slots[slot];
            debug_assert!(row.checkpoint.is_none());
            debug_assert!(row.live_index.is_none());
            row.checkpoint = Some(checkpoint);
            row.serial = serial;
            row.keep = false;
            slot
        } else {
            let slot = self.slots.len();
            self.slots.push(RetainedCheckpointSlot {
                checkpoint: Some(checkpoint),
                serial,
                live_index: None,
                keep: false,
            });
            slot
        };
        let live_index = self.live.len();
        self.live.push(slot);
        self.slots[slot].live_index = Some(live_index);
        RetainedCheckpointKey {
            generation,
            slot,
            serial,
        }
    }

    fn get(
        &self,
        generation: u64,
        key: &RetainedCheckpointKey,
    ) -> Result<&EngineCheckpoint<G>, RetainedEngineAccessError> {
        validate_checkpoint_key(generation, key)?;
        let row = self
            .slots
            .get(key.slot)
            .filter(|row| row.serial == key.serial)
            .ok_or(RetainedEngineAccessError::StaleCheckpoint)?;
        row.checkpoint
            .as_ref()
            .ok_or(RetainedEngineAccessError::StaleCheckpoint)
    }

    fn prune(
        &mut self,
        generation: u64,
        retained: &[RetainedCheckpointKey],
    ) -> Result<CheckpointPruningReceipt, RetainedEngineAccessError> {
        for key in retained {
            self.get(generation, key)?;
        }
        for key in retained {
            self.slots[key.slot].keep = true;
        }
        let before = self.live.len();
        let mut live_index = 0;
        while live_index < self.live.len() {
            let slot = self.live[live_index];
            if self.slots[slot].keep {
                self.slots[slot].keep = false;
                live_index += 1;
            } else {
                self.release(slot);
            }
        }
        Ok(CheckpointPruningReceipt {
            released: before - self.live.len(),
            retained: self.live.len(),
            slots: self.slots.len(),
        })
    }

    fn release(&mut self, slot: usize) {
        let live_index = self.slots[slot]
            .live_index
            .take()
            .expect("only a live checkpoint slot can be released");
        let removed = self.live.swap_remove(live_index);
        debug_assert_eq!(removed, slot);
        if let Some(&moved) = self.live.get(live_index) {
            self.slots[moved].live_index = Some(live_index);
        }
        self.slots[slot].keep = false;
        let checkpoint = self.slots[slot]
            .checkpoint
            .take()
            .expect("a live checkpoint slot owns a checkpoint");
        self.free.push(slot);
        drop(checkpoint);
    }
}

struct EngineGenerationSidecars<G> {
    generation: u64,
    checkpoints: RetainedCheckpointSlots<G>,
    attachment: Option<Box<dyn Any + Send>>,
}

struct InitializeSidecars {
    generation: u64,
}

impl RetainedStateOperation for InitializeSidecars {
    type Output = RetainedAttachmentKey;

    fn run<G: 'static>(self, mut admitted: RetainedStateAdmission<'_, G>) -> Self::Output {
        admitted.attach(EngineGenerationSidecars::<G> {
            generation: self.generation,
            checkpoints: RetainedCheckpointSlots::default(),
            attachment: None,
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

fn validate_checkpoint_key(
    generation: u64,
    key: &RetainedCheckpointKey,
) -> Result<(), RetainedEngineAccessError> {
    if key.generation != generation {
        return Err(RetainedEngineAccessError::ForeignGeneration);
    }
    Ok(())
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
    use tex_command::CommandState;
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

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct CheckpointStorageMetrics {
        slots_len: usize,
        slots_capacity: usize,
        live_len: usize,
        live_capacity: usize,
        free_len: usize,
        free_capacity: usize,
    }

    impl<G> RetainedCheckpointSlots<G> {
        fn metrics(&self) -> CheckpointStorageMetrics {
            CheckpointStorageMetrics {
                slots_len: self.slots.len(),
                slots_capacity: self.slots.capacity(),
                live_len: self.live.len(),
                live_capacity: self.live.capacity(),
                free_len: self.free.len(),
                free_capacity: self.free.capacity(),
            }
        }
    }

    struct RepeatedCheckpointPruning<'a>(&'a RetainedCheckpointKey);

    impl RetainedEngineOperation for RepeatedCheckpointPruning<'_> {
        type Output = Result<
            (
                CheckpointStorageMetrics,
                CheckpointStorageMetrics,
                CheckpointPruningReceipt,
            ),
            RetainedEngineAccessError,
        >;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            admitted
                .sidecars
                .checkpoints
                .get(admitted.generation, self.0)?;
            let generation = admitted.generation;
            let (universe, mut checkpoints) = admitted.parts();
            let mut control = crate::MainControl::tex82_initex(universe);
            let capture = |control: &mut crate::MainControl<G>, universe: &mut Universe<G>| {
                control
                    .capture_checkpoint(
                        crate::EngineBoundary::JobStart,
                        universe,
                        crate::ExecutionBudgetCounters::default(),
                    )
                    .expect("quiescent checkpoint")
            };

            let warm = checkpoints.retain(capture(&mut control, universe));
            let receipt = checkpoints
                .checkpoints
                .prune(generation, std::slice::from_ref(self.0))?;
            assert_eq!(receipt.released(), 1);
            assert!(matches!(
                checkpoints.checkpoints.get(generation, &warm),
                Err(RetainedEngineAccessError::StaleCheckpoint)
            ));
            let warmed = checkpoints.checkpoints.metrics();

            let mut last_receipt = receipt;
            for _ in 0..8_192 {
                let key = checkpoints.retain(capture(&mut control, universe));
                last_receipt = checkpoints
                    .checkpoints
                    .prune(generation, std::slice::from_ref(self.0))?;
                assert!(matches!(
                    checkpoints.checkpoints.get(generation, &key),
                    Err(RetainedEngineAccessError::StaleCheckpoint)
                ));
            }
            Ok((warmed, checkpoints.checkpoints.metrics(), last_receipt))
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
                    crate::EngineBoundary::JobStart,
                    &mut command,
                    &mut modes,
                    universe,
                    crate::ExecutionBudgetCounters::default(),
                    true,
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
                    crate::EngineBoundary::ShipoutComplete,
                    &mut command,
                    &mut modes,
                    universe,
                    crate::ExecutionBudgetCounters::default(),
                    true,
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
                .checkpoints
                .get(admitted.generation, self.checkpoint)
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
    fn pruning_releases_checkpoint_and_reuses_its_slot_without_aba() {
        let store = store();
        let mut generation =
            RetainedEngineGeneration::new(&store, World::default()).expect("generation");
        let survivor = generation.with_admitted(Capture).expect("survivor");
        let stale = generation
            .with_admitted(Capture)
            .expect("discarded checkpoint");

        let receipt = generation
            .prune_checkpoints(std::slice::from_ref(&survivor))
            .expect("checkpoint pruning");
        assert_eq!(receipt.released(), 1);
        assert_eq!(receipt.retained(), 1);
        assert_eq!(receipt.slots(), 2);
        assert_eq!(
            generation.with_admitted(Read(&survivor)),
            Ok(Ok(crate::EngineBoundary::JobStart))
        );
        assert_eq!(
            generation.with_admitted(Read(&stale)),
            Ok(Err(RetainedEngineAccessError::StaleCheckpoint))
        );

        let replacement = generation
            .with_admitted(Capture)
            .expect("replacement checkpoint");
        assert_eq!(replacement.slot, stale.slot);
        assert_ne!(replacement.serial, stale.serial);
        assert_eq!(
            generation.with_admitted(Read(&stale)),
            Ok(Err(RetainedEngineAccessError::StaleCheckpoint))
        );
        assert_eq!(
            generation.with_admitted(Read(&replacement)),
            Ok(Ok(crate::EngineBoundary::JobStart))
        );
    }

    #[test]
    fn repeated_8192_checkpoint_prunes_keep_storage_at_warmed_high_water() {
        let store = store();
        let mut generation =
            RetainedEngineGeneration::new(&store, World::default()).expect("generation");
        let survivor = generation.with_admitted(Capture).expect("survivor");

        let (warmed, after, receipt) = generation
            .with_admitted(RepeatedCheckpointPruning(&survivor))
            .expect("generation admission")
            .expect("checkpoint pruning");

        assert_eq!(after, warmed);
        assert_eq!(after.slots_len, 2);
        assert_eq!(after.live_len, 1);
        assert_eq!(after.free_len, 1);
        assert_eq!(receipt.released(), 1);
        assert_eq!(receipt.retained(), 1);
        assert_eq!(receipt.slots(), 2);
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

        let receipt = generation
            .prune_checkpoints(std::slice::from_ref(&survivor))
            .expect("checkpoint pruning");
        assert_eq!(receipt.released(), 1);
        assert_eq!(
            generation.with_admitted(Read(&discarded)),
            Ok(Err(RetainedEngineAccessError::StaleCheckpoint))
        );
        assert_eq!(
            generation.with_admitted(RestoreCount {
                checkpoint: &survivor,
                fixture: &fixture,
            }),
            Ok(10)
        );
    }
}
