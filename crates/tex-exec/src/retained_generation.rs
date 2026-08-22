//! Opaque retained executor generations and owner-relative checkpoint keys.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use tex_state::{
    DetachedFormatImage, FormatError, RetainedAttachmentKey, RetainedStateAccessError,
    RetainedStateAdmission, RetainedStateGeneration, RetainedStateOperation,
    RetainedStateRetirement, SessionEpochError, SessionInternerEpoch, Universe, UniverseError,
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
        let slot = self.sidecars.checkpoints.len();
        self.sidecars.checkpoints.push(Some(checkpoint));
        RetainedCheckpointKey {
            generation: self.generation,
            slot,
        }
    }

    pub fn checkpoint(
        &self,
        key: &RetainedCheckpointKey,
    ) -> Result<&EngineCheckpoint<G>, RetainedEngineAccessError> {
        validate_checkpoint_key(self.generation, key)?;
        self.sidecars
            .checkpoints
            .get(key.slot)
            .and_then(Option::as_ref)
            .ok_or(RetainedEngineAccessError::StaleCheckpoint)
    }

    pub fn attach<T: Send + 'static>(&mut self, attachment: T) -> RetainedEngineAttachmentKey {
        let slot = self.sidecars.attachments.len();
        self.sidecars.attachments.push(Some(Box::new(attachment)));
        RetainedEngineAttachmentKey {
            generation: self.generation,
            slot,
        }
    }

    pub fn attachment_mut<T: Send + 'static>(
        &mut self,
        key: &RetainedEngineAttachmentKey,
    ) -> Result<&mut T, RetainedEngineAccessError> {
        validate_attachment_key(self.generation, key)?;
        self.sidecars
            .attachments
            .get_mut(key.slot)
            .and_then(Option::as_deref_mut)
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
            .attachments
            .get_mut(key.slot)
            .and_then(Option::take)
            .ok_or(RetainedEngineAccessError::StaleAttachment)?
            .downcast::<T>()
            .map(|attachment| *attachment)
            .map_err(|_| RetainedEngineAccessError::AttachmentTypeMismatch)
    }
}

/// Restricted checkpoint-store borrow used by a synchronous sink.
pub struct RetainedCheckpointStore<'a, G> {
    generation: u64,
    checkpoints: &'a mut Vec<Option<EngineCheckpoint<G>>>,
}

impl<G> RetainedCheckpointStore<'_, G> {
    pub fn retain(&mut self, checkpoint: EngineCheckpoint<G>) -> RetainedCheckpointKey {
        let slot = self.checkpoints.len();
        self.checkpoints.push(Some(checkpoint));
        RetainedCheckpointKey {
            generation: self.generation,
            slot,
        }
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
}

/// Owner-relative key for one unpublished executor episode sidecar.
#[derive(Debug, Eq, PartialEq)]
pub struct RetainedEngineAttachmentKey {
    generation: u64,
    slot: usize,
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
}

impl CheckpointPruningReceipt {
    #[must_use]
    pub const fn released(self) -> usize {
        self.released
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
/// The state layer owns physical storage. Main-control and checkpoint roots
/// remain generation-typed sidecars under that exact physical owner.
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
    pub fn new(epoch: &SessionInternerEpoch, world: World) -> Result<Self, SessionEpochError> {
        let generation = next_generation();
        let mut state = RetainedStateGeneration::new(epoch, world)?;
        let sidecars = state.with_admitted(InitializeSidecars { generation });
        Ok(Self {
            generation,
            state,
            sidecars,
            liveness: Arc::new(()),
        })
    }

    pub fn from_format(
        epoch: &SessionInternerEpoch,
        world: World,
        image: &DetachedFormatImage,
    ) -> Result<Self, FormatError> {
        let generation = next_generation();
        let mut state = RetainedStateGeneration::from_format(epoch, world, image)?;
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
        if admitted.sidecars.attachments.iter().any(Option::is_some) {
            return Err(RetainedEngineAccessError::LiveAttachment);
        }
        for key in self.retained {
            validate_checkpoint_key(admitted.generation, key)?;
            if admitted
                .sidecars
                .checkpoints
                .get(key.slot)
                .and_then(Option::as_ref)
                .is_none()
            {
                return Err(RetainedEngineAccessError::StaleCheckpoint);
            }
        }
        Ok(())
    }
}

impl RetainedEngineOperation for PruneCheckpoints<'_> {
    type Output = Result<CheckpointPruningReceipt, RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        let mut keep = vec![false; admitted.sidecars.checkpoints.len()];
        for key in self.retained {
            validate_checkpoint_key(admitted.generation, key)?;
            if admitted
                .sidecars
                .checkpoints
                .get(key.slot)
                .and_then(Option::as_ref)
                .is_none()
            {
                return Err(RetainedEngineAccessError::StaleCheckpoint);
            }
            keep[key.slot] = true;
        }
        let before = admitted
            .sidecars
            .checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.is_some())
            .count();
        for (slot, checkpoint) in admitted.sidecars.checkpoints.iter_mut().enumerate() {
            if !keep[slot] {
                *checkpoint = None;
            }
        }
        Ok(CheckpointPruningReceipt {
            released: before.saturating_sub(self.retained.len()),
        })
    }
}

struct EngineGenerationSidecars<G> {
    generation: u64,
    checkpoints: Vec<Option<EngineCheckpoint<G>>>,
    attachments: Vec<Option<Box<dyn Any + Send>>>,
}

struct InitializeSidecars {
    generation: u64,
}

impl RetainedStateOperation for InitializeSidecars {
    type Output = RetainedAttachmentKey;

    fn run<G: 'static>(self, mut admitted: RetainedStateAdmission<'_, G>) -> Self::Output {
        admitted.attach(EngineGenerationSidecars::<G> {
            generation: self.generation,
            checkpoints: Vec::new(),
            attachments: Vec::new(),
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
    use tex_state::interner::InternerBudget;

    fn epoch() -> SessionInternerEpoch {
        SessionInternerEpoch::new(
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

    #[test]
    fn checkpoint_keys_are_owner_relative_across_live_generations() {
        let epoch = epoch();
        let mut first = RetainedEngineGeneration::new(&epoch, World::default()).expect("first");
        let key = first.with_admitted(Capture).expect("capture");
        assert_eq!(
            first.with_admitted(Read(&key)),
            Ok(Ok(crate::EngineBoundary::JobStart))
        );

        let mut second = RetainedEngineGeneration::new(&epoch, World::default()).expect("second");
        assert_eq!(
            second.with_admitted(Read(&key)),
            Ok(Err(RetainedEngineAccessError::ForeignGeneration))
        );
    }
}
