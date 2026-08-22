//! Opaque physical ownership for one retained revision generation.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::generation::Generation;
use crate::session_epoch::{SessionEpochError, SessionInternerEpoch};
use crate::stores::StateCore;
use crate::{DetachedFormatImage, FormatError, Universe, UniverseError, World};

static NEXT_RETAINED_GENERATION: AtomicU64 = AtomicU64::new(1);

enum PhysicalGenerationCoordinate {}

/// An operation admitted against an opaque retained generation.
///
/// Implementations must work for every coordinate and return a
/// coordinate-independent result. Runtime ids and borrows therefore cannot
/// escape [`RetainedStateGeneration::with_admitted`].
pub trait RetainedStateOperation {
    type Output;

    fn run<G: 'static>(self, admitted: RetainedStateAdmission<'_, G>) -> Self::Output;
}

/// Branded mutable admission of one physical generation and its sidecars.
pub struct RetainedStateAdmission<'a, G> {
    incarnation: u64,
    universe: &'a mut Universe<G>,
    attachments: &'a mut Vec<Option<Box<dyn Any + Send>>>,
}

impl<G: 'static> RetainedStateAdmission<'_, G> {
    pub fn universe(&mut self) -> &mut Universe<G> {
        self.universe
    }

    /// Stores one generation-typed engine sidecar under the same owner.
    pub fn attach<T: Send + 'static>(&mut self, attachment: T) -> RetainedAttachmentKey {
        let slot = self.attachments.len();
        self.attachments.push(Some(Box::new(attachment)));
        RetainedAttachmentKey {
            incarnation: self.incarnation,
            slot,
        }
    }

    pub fn attachment_mut<T: Send + 'static>(
        &mut self,
        key: &RetainedAttachmentKey,
    ) -> Result<&mut T, RetainedStateAccessError> {
        if key.incarnation != self.incarnation {
            return Err(RetainedStateAccessError::ForeignGeneration);
        }
        self.attachments
            .get_mut(key.slot)
            .and_then(Option::as_deref_mut)
            .ok_or(RetainedStateAccessError::StaleAttachment)?
            .downcast_mut::<T>()
            .ok_or(RetainedStateAccessError::AttachmentTypeMismatch)
    }

    pub fn take_attachment<T: Send + 'static>(
        &mut self,
        key: RetainedAttachmentKey,
    ) -> Result<T, RetainedStateAccessError> {
        if key.incarnation != self.incarnation {
            return Err(RetainedStateAccessError::ForeignGeneration);
        }
        let attachment = self
            .attachments
            .get_mut(key.slot)
            .and_then(Option::take)
            .ok_or(RetainedStateAccessError::StaleAttachment)?;
        attachment
            .downcast::<T>()
            .map(|attachment| *attachment)
            .map_err(|_| RetainedStateAccessError::AttachmentTypeMismatch)
    }

    /// Splits the aggregate runtime and one validated sidecar for an engine
    /// episode. Validation completes before either mutable borrow is exposed.
    pub fn universe_and_attachment_mut<T: Send + 'static>(
        &mut self,
        key: &RetainedAttachmentKey,
    ) -> Result<(&mut Universe<G>, &mut T), RetainedStateAccessError> {
        if key.incarnation != self.incarnation {
            return Err(RetainedStateAccessError::ForeignGeneration);
        }
        let attachment = self
            .attachments
            .get_mut(key.slot)
            .and_then(Option::as_deref_mut)
            .ok_or(RetainedStateAccessError::StaleAttachment)?
            .downcast_mut::<T>()
            .ok_or(RetainedStateAccessError::AttachmentTypeMismatch)?;
        Ok((self.universe, attachment))
    }
}

/// Owner-relative, non-serializable key for one retained engine sidecar.
///
/// It is intentionally neither `Copy` nor `Clone`.
#[derive(Debug, Eq, PartialEq)]
pub struct RetainedAttachmentKey {
    incarnation: u64,
    slot: usize,
}

/// Mutation-free retained-generation admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedStateAccessError {
    ForeignGeneration,
    StaleAttachment,
    AttachmentTypeMismatch,
}

/// Once-only scalar evidence from retiring one complete physical generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedStateRetirement {
    pub definitions: usize,
    pub token_lists: usize,
    pub glue_values: usize,
    pub provenance_records: usize,
    pub durable_node_lists: usize,
    pub journal_entries: usize,
    pub allocated_overflow_pages: usize,
}

/// Non-generic physical owner of one complete revision generation.
///
/// The concrete coordinate is private. Engine crates retain their generic
/// control/checkpoint sidecars in the attachment vector and recover them only
/// through a universally generic operation.
pub struct RetainedStateGeneration {
    incarnation: u64,
    epoch: SessionInternerEpoch,
    universe: Universe<PhysicalGenerationCoordinate>,
    attachments: Vec<Option<Box<dyn Any + Send>>>,
}

impl core::fmt::Debug for RetainedStateGeneration {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RetainedStateGeneration")
            .field("attachments", &self.attachments.len())
            .finish_non_exhaustive()
    }
}

impl RetainedStateGeneration {
    /// Allocates a fresh physical revision generation under one session epoch.
    pub fn new(epoch: &SessionInternerEpoch, world: World) -> Result<Self, SessionEpochError> {
        let interner = epoch.lease()?;
        let generation = Generation::<PhysicalGenerationCoordinate>::new();
        let core = StateCore::new(generation).map_err(|_| SessionEpochError::Retired)?;
        let mut universe = Universe::new(interner, core);
        *universe.world_mut() = world;
        drop(universe.release_session_epoch());
        Ok(Self {
            incarnation: next_incarnation(),
            epoch: epoch.clone(),
            universe,
            attachments: Vec::new(),
        })
    }

    /// Materializes one validated format directly into a retained physical
    /// generation under the caller's existing session epoch.
    pub fn from_format(
        epoch: &SessionInternerEpoch,
        world: World,
        image: &DetachedFormatImage,
    ) -> Result<Self, FormatError> {
        let interner = epoch.lease().map_err(|error| {
            FormatError::InvalidState(format!("session epoch is not available: {error:?}"))
        })?;
        let generation = Generation::<PhysicalGenerationCoordinate>::new();
        let mut universe =
            crate::format::materialize_retained_format(interner, generation, world, image)?;
        drop(universe.release_session_epoch());
        Ok(Self {
            incarnation: next_incarnation(),
            epoch: epoch.clone(),
            universe,
            attachments: Vec::new(),
        })
    }

    /// Admits the bundle under an operation whose output cannot name the
    /// private physical coordinate.
    pub fn with_admitted<O: RetainedStateOperation>(&mut self, operation: O) -> O::Output {
        let interner = self
            .epoch
            .lease()
            .expect("one retained generation is admitted at a time");
        self.universe.admit_session_epoch(interner);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            operation.run::<PhysicalGenerationCoordinate>(RetainedStateAdmission {
                incarnation: self.incarnation,
                universe: &mut self.universe,
                attachments: &mut self.attachments,
            })
        }));
        drop(self.universe.release_session_epoch());
        match result {
            Ok(output) => output,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[must_use]
    pub fn attachment_count(&self) -> usize {
        self.attachments
            .iter()
            .filter(|attachment| attachment.is_some())
            .count()
    }

    /// Releases every engine sidecar before retiring the complete immutable,
    /// node, dense-state, and journal generation exactly once.
    pub fn retire(mut self) -> Result<RetainedStateRetirement, UniverseError> {
        self.attachments.clear();
        let interner = self
            .epoch
            .lease()
            .expect("retirement exclusively admits its session epoch");
        self.universe.admit_session_epoch(interner);
        let retired = self.universe.retire_generation()?;
        drop(self.universe.release_session_epoch());
        Ok(RetainedStateRetirement {
            definitions: retired.generation.definitions,
            token_lists: retired.generation.token_lists,
            glue_values: retired.generation.glue_values,
            provenance_records: retired.generation.provenance_records,
            durable_node_lists: retired.durable_node_lists,
            journal_entries: retired.journal_entries,
            allocated_overflow_pages: retired.allocated_overflow_pages,
        })
    }
}

fn next_incarnation() -> u64 {
    NEXT_RETAINED_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .expect("retained generation incarnation space exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interner::InternerBudget;

    fn epoch() -> SessionInternerEpoch {
        SessionInternerEpoch::new(
            InternerBudget::new(64, 128, 4096).expect("retained-generation budget"),
        )
    }

    struct Attach(&'static str);

    impl RetainedStateOperation for Attach {
        type Output = RetainedAttachmentKey;

        fn run<G: 'static>(self, mut admitted: RetainedStateAdmission<'_, G>) -> Self::Output {
            let tokens = admitted
                .universe()
                .allocate_token_list(&[])
                .expect("allocate retained root");
            admitted.attach::<(&'static str, crate::TokenListId<G>)>((self.0, tokens))
        }
    }

    struct Read<'a>(&'a RetainedAttachmentKey);

    impl RetainedStateOperation for Read<'_> {
        type Output = Result<&'static str, RetainedStateAccessError>;

        fn run<G: 'static>(self, mut admitted: RetainedStateAdmission<'_, G>) -> Self::Output {
            admitted
                .attachment_mut::<(&'static str, crate::TokenListId<G>)>(self.0)
                .map(|attachment| attachment.0)
        }
    }

    #[test]
    fn attachment_key_is_generation_relative_and_foreign_rejection_is_mutation_free() {
        let epoch = epoch();
        let mut first = RetainedStateGeneration::new(&epoch, World::default()).expect("first");
        let key = first.with_admitted(Attach("first"));
        assert_eq!(first.with_admitted(Read(&key)), Ok("first"));
        assert_eq!(first.attachment_count(), 1);

        let mut second = RetainedStateGeneration::new(&epoch, World::default()).expect("second");
        assert_eq!(
            second.with_admitted(Read(&key)),
            Err(RetainedStateAccessError::ForeignGeneration)
        );
        assert_eq!(second.attachment_count(), 0);
    }
}
