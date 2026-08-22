//! Opaque retained executor generations and owner-relative checkpoint keys.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

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

    pub fn attach<T: 'static>(&mut self, attachment: T) -> RetainedEngineAttachmentKey {
        let slot = self.sidecars.attachments.len();
        self.sidecars.attachments.push(Some(Box::new(attachment)));
        RetainedEngineAttachmentKey {
            generation: self.generation,
            slot,
        }
    }

    pub fn attachment_mut<T: 'static>(
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

    pub fn take_attachment<T: 'static>(
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
    State(RetainedStateAccessError),
}

#[derive(Debug)]
pub enum RetainedEngineCompactionError {
    State(tex_state::UniverseCompactionError),
    Universe(UniverseError),
    Command(tex_command::CommandRestoreError),
    Nodes(tex_state::node_arena::NodeArenaError),
    Access(RetainedEngineAccessError),
    LiveEpisode,
    DuplicateCheckpoint,
    SemanticMismatch,
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

    /// Drops optional checkpoint roots only after validating the complete
    /// retained key set. Immutable generation rows remain append-only.
    pub fn prune_checkpoints(
        &mut self,
        retained: &[RetainedCheckpointKey],
    ) -> Result<CheckpointPruningReceipt, RetainedEngineAccessError> {
        self.with_admitted(PruneCheckpoints { retained })?
    }

    /// Cold-copies the complete physical generation and every selected named
    /// checkpoint, then atomically swaps owners and invalidates all old keys.
    pub fn compact(
        &mut self,
        retained: Vec<RetainedCheckpointKey>,
    ) -> Result<(Vec<RetainedCheckpointKey>, RetainedEngineRetirement), RetainedEngineCompactionError>
    {
        let destination_generation = next_generation();
        let (result, state) = self
            .state
            .compact(CompactEngineGeneration {
                source_generation: self.generation,
                destination_generation,
                source_sidecars: &self.sidecars,
                retained,
            })
            .map_err(|error| match error {
                tex_state::RetainedStateCompactionError::State(error) => {
                    RetainedEngineCompactionError::State(error)
                }
                tex_state::RetainedStateCompactionError::Operation(error) => error,
                tex_state::RetainedStateCompactionError::Retirement(error) => {
                    RetainedEngineCompactionError::Universe(error)
                }
            })?;
        self.generation = destination_generation;
        self.sidecars = result.sidecars;
        Ok((result.checkpoints, RetainedEngineRetirement { state }))
    }

    pub fn retire(self) -> Result<RetainedEngineRetirement, UniverseError> {
        Ok(RetainedEngineRetirement {
            state: self.state.retire()?,
        })
    }
}

struct CompactedEngineGeneration {
    sidecars: RetainedAttachmentKey,
    checkpoints: Vec<RetainedCheckpointKey>,
}

struct CompactEngineGeneration<'a> {
    source_generation: u64,
    destination_generation: u64,
    source_sidecars: &'a RetainedAttachmentKey,
    retained: Vec<RetainedCheckpointKey>,
}

impl tex_state::RetainedStateCompactionOperation for CompactEngineGeneration<'_> {
    type Output = CompactedEngineGeneration;
    type Error = RetainedEngineCompactionError;

    fn run<G: 'static>(
        self,
        mut context: tex_state::RetainedStateCompactionContext<'_, G>,
    ) -> Result<Self::Output, Self::Error> {
        let source = context
            .source_attachment::<EngineGenerationSidecars<G>>(self.source_sidecars)
            .map_err(|error| RetainedEngineCompactionError::Access(error.into()))?;
        if source.generation != self.source_generation {
            return Err(RetainedEngineCompactionError::Access(
                RetainedEngineAccessError::ForeignGeneration,
            ));
        }
        if source.attachments.iter().any(Option::is_some) {
            return Err(RetainedEngineCompactionError::LiveEpisode);
        }
        let mut seen = std::collections::BTreeSet::new();
        for key in &self.retained {
            validate_checkpoint_key(self.source_generation, key)
                .map_err(RetainedEngineCompactionError::Access)?;
            if !seen.insert(key.slot) {
                return Err(RetainedEngineCompactionError::DuplicateCheckpoint);
            }
            if source
                .checkpoints
                .get(key.slot)
                .and_then(Option::as_ref)
                .is_none()
            {
                return Err(RetainedEngineCompactionError::Access(
                    RetainedEngineAccessError::StaleCheckpoint,
                ));
            }
        }

        let mut checkpoints = Vec::new();
        checkpoints
            .try_reserve_exact(self.retained.len())
            .map_err(|_| {
                RetainedEngineCompactionError::State(
                    tex_state::UniverseCompactionError::AllocationFailed,
                )
            })?;
        for key in &self.retained {
            let source_checkpoint = source.checkpoints[key.slot]
                .as_ref()
                .expect("complete preflight retained every selected checkpoint");
            checkpoints.push(Some(source_checkpoint.dense_copy_for_compaction(&context)?));
        }
        let destination_keys = (0..checkpoints.len())
            .map(|slot| RetainedCheckpointKey {
                generation: self.destination_generation,
                slot,
            })
            .collect();
        let sidecars = context.attach(EngineGenerationSidecars::<G> {
            generation: self.destination_generation,
            checkpoints,
            attachments: Vec::new(),
        });
        Ok(CompactedEngineGeneration {
            sidecars,
            checkpoints: destination_keys,
        })
    }
}

struct PruneCheckpoints<'a> {
    retained: &'a [RetainedCheckpointKey],
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
    attachments: Vec<Option<Box<dyn Any>>>,
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

    struct AttachEpisode;

    impl RetainedEngineOperation for AttachEpisode {
        type Output = RetainedEngineAttachmentKey;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            admitted.attach(String::from("live episode"))
        }
    }

    struct TakeEpisode(RetainedEngineAttachmentKey);

    impl RetainedEngineOperation for TakeEpisode {
        type Output = Result<String, RetainedEngineAccessError>;

        fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
            admitted.take_attachment(self.0)
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

    #[test]
    fn compaction_atomically_replaces_the_owner_and_preserves_checkpoint_semantics() {
        let epoch = epoch();
        let mut generation =
            RetainedEngineGeneration::new(&epoch, World::default()).expect("generation");
        let key = generation.with_admitted(Capture).expect("capture");
        let old_key = RetainedCheckpointKey {
            generation: key.generation,
            slot: key.slot,
        };

        let (mut relocated, retirement) = generation.compact(vec![key]).expect("compact");
        let relocated = relocated.pop().expect("relocated checkpoint");
        assert_eq!(
            generation.with_admitted(Read(&relocated)),
            Ok(Ok(crate::EngineBoundary::JobStart))
        );
        assert_eq!(
            generation.with_admitted(Read(&old_key)),
            Ok(Err(RetainedEngineAccessError::ForeignGeneration))
        );
        assert_eq!(retirement.state().allocated_overflow_pages, 0);
    }

    #[test]
    fn malformed_compaction_request_is_mutation_free() {
        let epoch = epoch();
        let mut generation =
            RetainedEngineGeneration::new(&epoch, World::default()).expect("generation");
        let key = generation.with_admitted(Capture).expect("capture");
        let duplicate = vec![
            RetainedCheckpointKey {
                generation: key.generation,
                slot: key.slot,
            },
            RetainedCheckpointKey {
                generation: key.generation,
                slot: key.slot,
            },
        ];
        assert!(matches!(
            generation.compact(duplicate),
            Err(RetainedEngineCompactionError::DuplicateCheckpoint)
        ));
        assert_eq!(
            generation.with_admitted(Read(&key)),
            Ok(Ok(crate::EngineBoundary::JobStart))
        );
    }

    #[test]
    fn live_episode_rejects_compaction_without_consuming_the_episode() {
        let epoch = epoch();
        let mut generation =
            RetainedEngineGeneration::new(&epoch, World::default()).expect("generation");
        let episode = generation
            .with_admitted(AttachEpisode)
            .expect("attach episode");
        assert!(matches!(
            generation.compact(Vec::new()),
            Err(RetainedEngineCompactionError::LiveEpisode)
        ));
        assert_eq!(
            generation.with_admitted(TakeEpisode(episode)),
            Ok(Ok(String::from("live episode")))
        );
    }
}
