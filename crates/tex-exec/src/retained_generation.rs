//! Opaque retained executor generations and owner-relative checkpoint keys.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use tex_state::{
    DetachedFormatImage, FormatError, ReachabilityStore, RetainedAttachmentKey,
    RetainedStateAccessError, RetainedStateAdmission, RetainedStateForkBuild,
    RetainedStateForkError, RetainedStateForkOperation, RetainedStateGeneration,
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
    state: RetainedStateGeneration<'store>,
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

impl core::fmt::Debug for RetainedEngineGeneration<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RetainedEngineGeneration")
            .field("checkpoints", &self.state.attachment_count())
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
            state,
            sidecars,
            liveness: Arc::new(()),
        })
    }

    pub fn from_format(
        store: &'store ReachabilityStore,
        world: World,
        image: &DetachedFormatImage,
    ) -> Result<Self, FormatError> {
        Self::from_format_owned(store.clone(), world, image)
    }

    #[doc(hidden)]
    pub fn from_format_owned(
        store: ReachabilityStore,
        world: World,
        image: &DetachedFormatImage,
    ) -> Result<Self, FormatError> {
        let generation = next_generation();
        let mut state = RetainedStateGeneration::from_format_owned(store, world, image)?;
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
        let generation = next_generation();
        let result = self.state.try_fork_owned(ForkCheckpoint {
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
                state,
                sidecars,
                liveness: Arc::new(()),
            },
            RetainedEngineAttachmentKey { generation },
            budget_counters,
        ))
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
    /// retained key set. Their immutable semantic owners retire with them.
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
        let checkpoint = sidecars
            .checkpoints
            .get(self.source_generation, self.checkpoint)
            .map_err(RetainedEngineForkError::Access)?;
        let budget_counters = checkpoint.budget_counters();
        let (universe, control) = checkpoint
            .fork_state(universe)
            .map_err(RetainedEngineForkError::Restore)?;
        Ok(RetainedStateForkBuild::new(
            universe,
            Box::new(EngineGenerationSidecars::<G> {
                generation: self.generation,
                checkpoints: RetainedCheckpointSlots::default(),
                attachment: Some(Box::new(control)),
            }),
            budget_counters,
        ))
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
    attachment: Option<Box<dyn Any>>,
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
    }

    impl RetainedEngineOperation for ConsumeFork {
        type Output = (i32, RetainedCheckpointKey);

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let mut control = admitted
                .take_attachment::<crate::MainControl<G>>(self.runtime)
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
            let mut control = admitted
                .take_attachment::<crate::MainControl<G>>(self.0)
                .expect("fork runtime");
            let step = control
                .advance_episode(admitted.universe())
                .expect("resource suspension");
            assert!(matches!(
                step,
                crate::StepResult::Suspended(crate::ResourceNeed::Input { ref name, .. })
                    if name == "child.tex"
            ));
            admitted.attach(control)
        }
    }

    struct ResumeFork(RetainedEngineAttachmentKey);

    impl RetainedEngineOperation for ResumeFork {
        type Output = crate::StepResult;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            let mut control = admitted
                .take_attachment::<crate::MainControl<G>>(self.0)
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
        assert_eq!(counters.committed_steps, 7);
        assert_eq!(counters.cumulative_fuel, 11);
        let (before, _rejected_checkpoint) = rejected
            .with_admitted(ConsumeFork {
                runtime,
                replacement: 99,
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
            })
            .expect("candidate admission");
        assert_eq!(before, 41);
        accepted.retire().expect("old prior retires");
        assert!(!accepted_witness.is_live());
        assert_eq!(
            accepted_candidate
                .with_admitted(ReadCount)
                .expect("replacement admission"),
            52,
            "the accepted replacement survives old-prior retirement"
        );

        let (mut restarted, runtime, _) = accepted_candidate
            .fork_checkpoint(&replacement_checkpoint)
            .expect("later accepted restart");
        let (before, _checkpoint) = restarted
            .with_admitted(ConsumeFork {
                runtime,
                replacement: 73,
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
        let mut accepted = RetainedEngineGeneration::from_format(&store, World::memory(), &image)
            .expect("loaded format generation");
        let checkpoint = accepted
            .with_admitted(CaptureLoadedFormat)
            .expect("format checkpoint admission");
        drop(image);

        let (mut current, runtime, _) = accepted
            .fork_checkpoint(&checkpoint)
            .expect("first document fork");
        let (before, _checkpoint) = current
            .with_admitted(ConsumeFork {
                runtime,
                replacement: 271,
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
