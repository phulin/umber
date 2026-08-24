//! Opaque physical ownership for one retained revision generation.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::generation::Generation;
use crate::reachability_store::{
    ReachabilityGenerationKey, ReachabilityStore, ReachabilityStoreError,
};
use crate::session_epoch::SessionEpochError;
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
pub(crate) struct PhysicalStateGeneration {
    incarnation: u64,
    universe: Universe<PhysicalGenerationCoordinate>,
    attachments: Vec<Option<Box<dyn Any + Send>>>,
    #[cfg(feature = "profiling")]
    _profiling_lifetime: crate::measurement::RetainedGenerationLifetime,
}

/// Move-only handle to one physical generation stored in its session's
/// external reachability domain.
pub struct RetainedStateGeneration {
    store: ReachabilityStore,
    key: Option<ReachabilityGenerationKey>,
}

impl core::fmt::Debug for RetainedStateGeneration {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RetainedStateGeneration")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl RetainedStateGeneration {
    /// Allocates a fresh physical revision generation under one session epoch.
    pub fn new(store: &ReachabilityStore, world: World) -> Result<Self, SessionEpochError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = crate::measurement::hot_core_allocation_scope(
            crate::measurement::HotCoreAllocationOwner::GenerationBoundary,
        );
        let interner = store.epoch().lease()?;
        let generation = Generation::<PhysicalGenerationCoordinate>::new();
        let core = StateCore::new(generation).map_err(|_| SessionEpochError::Retired)?;
        let mut universe = Universe::new(interner, core);
        *universe.world_mut() = world;
        drop(universe.release_session_epoch());
        let physical = PhysicalStateGeneration {
            incarnation: next_incarnation(),
            universe,
            attachments: Vec::new(),
            #[cfg(feature = "profiling")]
            _profiling_lifetime: crate::measurement::RetainedGenerationLifetime::begin(),
        };
        let key = store
            .insert_generation(physical)
            .map_err(map_store_construction_error)?;
        Ok(Self {
            store: store.clone(),
            key: Some(key),
        })
    }

    /// Materializes one validated format directly into a retained physical
    /// generation under the caller's existing session epoch.
    pub fn from_format(
        store: &ReachabilityStore,
        world: World,
        image: &DetachedFormatImage,
    ) -> Result<Self, FormatError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = crate::measurement::hot_core_allocation_scope(
            crate::measurement::HotCoreAllocationOwner::ColdMaterialization,
        );
        let interner = store.epoch().lease().map_err(|error| {
            FormatError::InvalidState(format!("session epoch is not available: {error:?}"))
        })?;
        let generation = Generation::<PhysicalGenerationCoordinate>::new();
        let mut universe =
            crate::format::materialize_retained_format(interner, generation, world, image)?;
        drop(universe.release_session_epoch());
        let physical = PhysicalStateGeneration {
            incarnation: next_incarnation(),
            universe,
            attachments: Vec::new(),
            #[cfg(feature = "profiling")]
            _profiling_lifetime: crate::measurement::RetainedGenerationLifetime::begin(),
        };
        let key = store.insert_generation(physical).map_err(|error| {
            FormatError::InvalidState(format!("reachability store rejected generation: {error:?}"))
        })?;
        Ok(Self {
            store: store.clone(),
            key: Some(key),
        })
    }

    /// Admits the bundle under an operation whose output cannot name the
    /// private physical coordinate.
    pub fn with_admitted<O: RetainedStateOperation>(&mut self, operation: O) -> O::Output {
        let interner = self
            .store
            .epoch()
            .lease()
            .expect("one retained generation is admitted at a time");
        let key = self
            .key
            .expect("a live retained generation has a store slot");
        let result = self
            .store
            .with_generation_mut(key, |physical| {
                physical.universe.admit_session_epoch(interner);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    operation.run::<PhysicalGenerationCoordinate>(RetainedStateAdmission {
                        incarnation: physical.incarnation,
                        universe: &mut physical.universe,
                        attachments: &mut physical.attachments,
                    })
                }));
                drop(physical.universe.release_session_epoch());
                result
            })
            .expect("a live retained generation keeps its store slot");
        match result {
            Ok(output) => output,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[must_use]
    pub fn attachment_count(&self) -> usize {
        let key = self
            .key
            .expect("a live retained generation has a store slot");
        self.store
            .with_generation_mut(key, |physical| {
                physical
                    .attachments
                    .iter()
                    .filter(|attachment| attachment.is_some())
                    .count()
            })
            .expect("a live retained generation keeps its store slot")
    }

    /// Whether two generations reside in the same external session store.
    #[must_use]
    pub fn same_store(&self, other: &Self) -> bool {
        self.store.same_store(&other.store)
    }

    /// Releases every engine sidecar before retiring the complete immutable,
    /// node, dense-state, and journal generation exactly once.
    pub fn retire(mut self) -> Result<RetainedStateRetirement, UniverseError> {
        let key = self.key.take().expect("a live generation retires once");
        let mut physical = self
            .store
            .take_generation(key)
            .expect("a live retained generation keeps its store slot");
        physical.attachments.clear();
        let interner = self
            .store
            .epoch()
            .lease()
            .expect("retirement exclusively admits its session epoch");
        physical.universe.admit_session_epoch(interner);
        let retired = physical.universe.retire_generation()?;
        drop(physical.universe.release_session_epoch());
        #[cfg(feature = "profiling")]
        crate::measurement::record_retained_generation_retirement();
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

impl Drop for RetainedStateGeneration {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let _ = self.store.take_generation(key);
    }
}

fn map_store_construction_error(error: ReachabilityStoreError) -> SessionEpochError {
    match error {
        ReachabilityStoreError::GenerationSlotsExhausted => {
            SessionEpochError::GenerationSlotsExhausted
        }
        ReachabilityStoreError::GenerationIdentityExhausted
        | ReachabilityStoreError::StaleGeneration => SessionEpochError::Retired,
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

    fn store() -> ReachabilityStore {
        ReachabilityStore::new(
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

    struct InternKnown;

    impl RetainedStateOperation for InternKnown {
        type Output = u32;

        fn run<G: 'static>(self, mut admitted: RetainedStateAdmission<'_, G>) -> Self::Output {
            admitted
                .universe()
                .intern("session-known")
                .expect("intern")
                .symbol()
                .raw()
        }
    }

    struct AdmitKnown;

    impl RetainedStateOperation for AdmitKnown {
        type Output = (u32, crate::ResolvedMeaning<()>);

        fn run<G: 'static>(self, mut admitted: RetainedStateAdmission<'_, G>) -> Self::Output {
            let mut context = admitted.universe().command_context().expect("context");
            let symbol = context
                .known_control_sequence("session-known")
                .expect("same epoch symbol");
            let meaning = context.meaning(symbol);
            let detached = match meaning {
                crate::ResolvedMeaning::Static(meaning) => crate::ResolvedMeaning::Static(meaning),
                crate::ResolvedMeaning::Macro { .. } => panic!("fresh current root is undefined"),
            };
            (symbol.raw(), detached)
        }
    }

    #[test]
    fn attachment_key_is_generation_relative_and_foreign_rejection_is_mutation_free() {
        let store = store();
        let mut first = RetainedStateGeneration::new(&store, World::default()).expect("first");
        let key = first.with_admitted(Attach("first"));
        assert_eq!(first.with_admitted(Read(&key)), Ok("first"));
        assert_eq!(first.attachment_count(), 1);

        let mut second = RetainedStateGeneration::new(&store, World::default()).expect("second");
        assert!(first.same_store(&second));
        assert_eq!(
            second.with_admitted(Read(&key)),
            Err(RetainedStateAccessError::ForeignGeneration)
        );
        assert_eq!(second.attachment_count(), 0);
    }

    #[test]
    fn session_epoch_symbol_is_admitted_into_each_fresh_generation_bank() {
        let store = store();
        let mut prior =
            RetainedStateGeneration::new(&store, World::default()).expect("prior generation");
        let prior_symbol = prior.with_admitted(InternKnown);
        let mut current =
            RetainedStateGeneration::new(&store, World::default()).expect("current generation");
        let (current_symbol, meaning) = current.with_admitted(AdmitKnown);
        assert_eq!(current_symbol, prior_symbol);
        assert_eq!(
            meaning,
            crate::ResolvedMeaning::Static(crate::meaning::Meaning::Undefined)
        );
    }

    #[test]
    fn external_store_has_exactly_two_reusable_generation_slots() {
        let store = store();
        let prior = RetainedStateGeneration::new(&store, World::default()).expect("prior");
        let current = RetainedStateGeneration::new(&store, World::default()).expect("current");
        assert!(prior.same_store(&current));
        assert_eq!(store.live_generation_count(), 2);
        assert_eq!(
            RetainedStateGeneration::new(&store, World::default()).unwrap_err(),
            SessionEpochError::GenerationSlotsExhausted
        );

        drop(current);
        assert_eq!(store.live_generation_count(), 1);
        let replacement =
            RetainedStateGeneration::new(&store, World::default()).expect("replacement current");
        assert!(prior.same_store(&replacement));
        assert_eq!(store.live_generation_count(), 2);
    }
}
