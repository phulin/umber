//! Session-epoch owner for retained revision generations.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interner::InternerBudget;
use crate::retained_generation::PhysicalStateGeneration;
use crate::session_epoch::SessionInternerEpoch;

const RETAINED_GENERATION_SLOTS: usize = 2;

#[derive(Default)]
struct ReachabilitySlot {
    serial: u64,
    generation: Option<PhysicalStateGeneration>,
}

struct ReachabilityStorage {
    next_serial: u64,
    slots: [ReachabilitySlot; RETAINED_GENERATION_SLOTS],
    candidate_transaction: Option<CandidateTransactionSlots>,
}

#[derive(Clone, Copy)]
struct CandidateTransactionSlots {
    source: ReachabilityGenerationKey,
    candidate: ReachabilityGenerationKey,
    origin: CandidateTransactionOrigin,
    phase: CandidateTransactionPhase,
}

/// Whether the current slot borrowed reversible owners from the accepted
/// source or was materialized independently from a frozen JobStart anchor.
/// The distinction belongs to the aggregate transaction, not to individual
/// state families: an independent current slot has nothing to return on
/// rejection and nothing to promote in-place on acceptance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateTransactionOrigin {
    Forked,
    Independent,
}

/// State of the one aggregate accepted/current transaction. Components do not
/// publish their own phases: a transition becomes observable only after every
/// owner has completed the corresponding ordered operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateTransactionPhase {
    AcceptedRewound,
    CandidateLive,
    DestinationAcceptCommitted,
    DestinationRejectReturned,
}

/// Coarse owner of the one reachability domain shared by a session's prior
/// and current retained generations.
///
/// The ordinary Rust API keeps a caller-owned handle above every retained
/// generation and editor session. One coarse allocation supports exported FFI
/// sessions without self-references. Generation creation reuses one of two
/// inline slots and performs no reachability-control allocation. Runtime
/// values never clone this owner.
pub struct ReachabilityStore {
    epoch: SessionInternerEpoch,
    storage: Rc<RefCell<ReachabilityStorage>>,
}

impl Clone for ReachabilityStore {
    fn clone(&self) -> Self {
        Self {
            epoch: self.epoch.clone(),
            storage: Rc::clone(&self.storage),
        }
    }
}

impl core::fmt::Debug for ReachabilityStore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ReachabilityStore")
            .field("live_generations", &self.live_generation_count())
            .finish_non_exhaustive()
    }
}

impl ReachabilityStore {
    /// Creates one inseparable session reachability and interning epoch. The
    /// fixed two-slot array is inline in one coarse store allocation; callers
    /// cannot combine the store with a foreign or separately retained epoch.
    #[must_use]
    pub fn new(interner_budget: InternerBudget) -> Self {
        Self {
            epoch: SessionInternerEpoch::new(interner_budget),
            storage: Rc::new(RefCell::new(ReachabilityStorage {
                next_serial: 1,
                slots: std::array::from_fn(|_| ReachabilitySlot::default()),
                candidate_transaction: None,
            })),
        }
    }

    /// Creates one reachability domain with interner storage reserved from the
    /// executable profile selected for the session.
    #[must_use]
    pub fn new_for_profile(profile: crate::EngineCapacityProfile) -> Self {
        Self {
            epoch: SessionInternerEpoch::new_for_profile(profile),
            storage: Rc::new(RefCell::new(ReachabilityStorage {
                next_serial: 1,
                slots: std::array::from_fn(|_| ReachabilitySlot::default()),
                candidate_transaction: None,
            })),
        }
    }

    #[must_use]
    pub(crate) const fn epoch(&self) -> &SessionInternerEpoch {
        &self.epoch
    }

    /// Whether two coarse owners name the same physical reachability domain.
    #[must_use]
    pub fn same_store(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.storage, &other.storage)
    }

    /// Number of occupied prior/current slots. This is a cold lifecycle
    /// projection and is never consulted by value access.
    #[must_use]
    pub fn live_generation_count(&self) -> usize {
        self.storage
            .borrow()
            .slots
            .iter()
            .filter(|slot| slot.generation.is_some())
            .count()
    }

    pub(crate) fn generation_has_candidate_transaction(
        &self,
        key: ReachabilityGenerationKey,
    ) -> bool {
        self.storage
            .borrow()
            .candidate_transaction
            .is_some_and(|transaction| transaction.source == key)
    }

    pub(crate) fn generation_is_candidate(&self, key: ReachabilityGenerationKey) -> bool {
        self.storage
            .borrow()
            .candidate_transaction
            .is_some_and(|transaction| transaction.candidate == key)
    }

    pub(crate) fn generation_is_independent_candidate(
        &self,
        key: ReachabilityGenerationKey,
    ) -> bool {
        self.storage
            .borrow()
            .candidate_transaction
            .is_some_and(|transaction| {
                transaction.candidate == key
                    && transaction.origin == CandidateTransactionOrigin::Independent
            })
    }

    pub(crate) fn insert_generation(
        &self,
        generation: PhysicalStateGeneration,
    ) -> Result<ReachabilityGenerationKey, ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        let slot = storage
            .slots
            .iter()
            .position(|slot| slot.generation.is_none())
            .ok_or(ReachabilityStoreError::GenerationSlotsExhausted)?;
        let serial = storage.next_serial;
        storage.next_serial = serial
            .checked_add(1)
            .ok_or(ReachabilityStoreError::GenerationIdentityExhausted)?;
        storage.slots[slot] = ReachabilitySlot {
            serial,
            generation: Some(generation),
        };
        Ok(ReachabilityGenerationKey { slot, serial })
    }

    pub(crate) fn preflight_generation_insert(&self) -> Result<(), ReachabilityStoreError> {
        let storage = self.storage.borrow();
        if storage.slots.iter().all(|slot| slot.generation.is_some()) {
            return Err(ReachabilityStoreError::GenerationSlotsExhausted);
        }
        if storage.next_serial == u64::MAX {
            return Err(ReachabilityStoreError::GenerationIdentityExhausted);
        }
        Ok(())
    }

    pub(crate) fn insert_fork_generation(
        &self,
        source: ReachabilityGenerationKey,
        generation: PhysicalStateGeneration,
    ) -> Result<ReachabilityGenerationKey, ReachabilityStoreError> {
        let candidate = self.insert_generation(generation)?;
        let mut storage = self.storage.borrow_mut();
        assert!(storage.candidate_transaction.is_none());
        let mut transaction = CandidateTransactionSlots {
            source,
            candidate,
            origin: CandidateTransactionOrigin::Forked,
            phase: CandidateTransactionPhase::AcceptedRewound,
        };
        debug_assert_eq!(
            transaction.phase,
            CandidateTransactionPhase::AcceptedRewound
        );
        transaction.phase = CandidateTransactionPhase::CandidateLive;
        storage.candidate_transaction = Some(transaction);
        Ok(candidate)
    }

    /// Installs a freshly materialized current slot beside one accepted slot.
    /// Unlike a rooted fork, this does not rewind or lend any source owner.
    /// It still uses the sole aggregate candidate transaction so the fixed
    /// prior/current slot bound and atomic accept/reject barriers are shared.
    pub(crate) fn insert_independent_candidate_generation(
        &self,
        source: ReachabilityGenerationKey,
        generation: PhysicalStateGeneration,
    ) -> Result<ReachabilityGenerationKey, ReachabilityStoreError> {
        let candidate = self.insert_generation(generation)?;
        let mut storage = self.storage.borrow_mut();
        assert!(storage.candidate_transaction.is_none());
        storage.candidate_transaction = Some(CandidateTransactionSlots {
            source,
            candidate,
            origin: CandidateTransactionOrigin::Independent,
            phase: CandidateTransactionPhase::CandidateLive,
        });
        Ok(candidate)
    }

    pub(crate) fn with_generation_mut<R>(
        &self,
        key: ReachabilityGenerationKey,
        operation: impl FnOnce(&mut PhysicalStateGeneration) -> R,
    ) -> Result<R, ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        if storage
            .candidate_transaction
            .is_some_and(|transaction| transaction.source == key)
        {
            return Err(ReachabilityStoreError::CandidateTransactionActive);
        }
        let slot = storage
            .slots
            .get_mut(key.slot)
            .filter(|slot| slot.serial == key.serial)
            .ok_or(ReachabilityStoreError::StaleGeneration)?;
        let generation = slot
            .generation
            .as_mut()
            .ok_or(ReachabilityStoreError::StaleGeneration)?;
        Ok(operation(generation))
    }

    pub(crate) fn with_candidate_pair_mut<R>(
        &self,
        candidate_key: ReachabilityGenerationKey,
        operation: impl FnOnce(&mut PhysicalStateGeneration, &mut PhysicalStateGeneration) -> R,
    ) -> Result<R, ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        let transaction = storage
            .candidate_transaction
            .filter(|transaction| {
                transaction.candidate == candidate_key
                    && transaction.phase == CandidateTransactionPhase::CandidateLive
            })
            .ok_or(ReachabilityStoreError::CandidateTransactionMismatch)?;
        let [source, candidate] = two_slots_mut(
            &mut storage.slots,
            transaction.source.slot,
            transaction.candidate.slot,
        );
        let source = source
            .generation
            .as_mut()
            .ok_or(ReachabilityStoreError::StaleGeneration)?;
        let candidate = candidate
            .generation
            .as_mut()
            .ok_or(ReachabilityStoreError::StaleGeneration)?;
        Ok(operation(source, candidate))
    }

    pub(crate) fn take_generation(
        &self,
        key: ReachabilityGenerationKey,
    ) -> Result<PhysicalStateGeneration, ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        if storage
            .candidate_transaction
            .is_some_and(|transaction| transaction.source == key || transaction.candidate == key)
        {
            return Err(ReachabilityStoreError::CandidateTransactionActive);
        }
        let slot = storage
            .slots
            .get_mut(key.slot)
            .filter(|slot| slot.serial == key.serial)
            .ok_or(ReachabilityStoreError::StaleGeneration)?;
        slot.generation
            .take()
            .ok_or(ReachabilityStoreError::StaleGeneration)
    }

    /// Accepts the complete candidate transaction before either physical
    /// generation may retire. Every aggregate family will settle through this
    /// one ordered barrier; the current implementation delegates the families
    /// already owned by `Universe` and deliberately exposes no PDF-only seam.
    pub(crate) fn prepare_candidate_accept(
        &self,
        source_key: ReachabilityGenerationKey,
        candidate_key: ReachabilityGenerationKey,
    ) -> Result<(), ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        let mut transaction = storage
            .candidate_transaction
            .filter(|transaction| {
                transaction.source == source_key && transaction.candidate == candidate_key
            })
            .ok_or(ReachabilityStoreError::CandidateTransactionMismatch)?;
        if transaction.phase != CandidateTransactionPhase::CandidateLive {
            return Err(ReachabilityStoreError::CandidateTransactionMismatch);
        }
        if transaction.origin == CandidateTransactionOrigin::Forked {
            let candidate = storage.slots[transaction.candidate.slot]
                .generation
                .as_mut()
                .ok_or(ReachabilityStoreError::StaleGeneration)?;
            candidate.universe.accept_checkpoint_candidate();
        }
        transaction.phase = CandidateTransactionPhase::DestinationAcceptCommitted;
        storage.candidate_transaction = Some(transaction);
        Ok(())
    }

    /// Completes acceptance only after source-side command and mode move
    /// bookkeeping has released every superseded accepted range.
    pub(crate) fn finish_candidate_accept(
        &self,
        source_key: ReachabilityGenerationKey,
        candidate_key: ReachabilityGenerationKey,
    ) -> Result<(), ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        let transaction = storage
            .candidate_transaction
            .filter(|transaction| {
                transaction.source == source_key && transaction.candidate == candidate_key
            })
            .ok_or(ReachabilityStoreError::CandidateTransactionMismatch)?;
        if transaction.phase != CandidateTransactionPhase::DestinationAcceptCommitted {
            return Err(ReachabilityStoreError::CandidateTransactionMismatch);
        }
        storage.candidate_transaction = None;
        Ok(())
    }

    /// Rejects the complete candidate transaction in reverse owner order and
    /// restores the accepted source before either slot becomes accessible.
    pub(crate) fn prepare_candidate_reject(
        &self,
        candidate_key: ReachabilityGenerationKey,
    ) -> Result<(), ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        let mut transaction = storage
            .candidate_transaction
            .filter(|transaction| transaction.candidate == candidate_key)
            .ok_or(ReachabilityStoreError::CandidateTransactionMismatch)?;
        if transaction.phase != CandidateTransactionPhase::CandidateLive {
            return Err(ReachabilityStoreError::CandidateTransactionMismatch);
        }
        if transaction.origin == CandidateTransactionOrigin::Forked {
            let [source, candidate] = two_slots_mut(
                &mut storage.slots,
                transaction.source.slot,
                transaction.candidate.slot,
            );
            let source = source
                .generation
                .as_mut()
                .ok_or(ReachabilityStoreError::StaleGeneration)?;
            let candidate = candidate
                .generation
                .as_mut()
                .ok_or(ReachabilityStoreError::StaleGeneration)?;
            candidate.clear_attachment();
            source
                .universe
                .reject_checkpoint_candidate(&mut candidate.universe);
        } else {
            storage.slots[transaction.candidate.slot]
                .generation
                .as_mut()
                .ok_or(ReachabilityStoreError::StaleGeneration)?
                .clear_attachment();
        }
        transaction.phase = CandidateTransactionPhase::DestinationRejectReturned;
        storage.candidate_transaction = Some(transaction);
        Ok(())
    }

    /// Completes rejection only after source-side command and mode journals
    /// have consumed returned candidate carriers and replayed accepted moves.
    pub(crate) fn finish_candidate_reject(
        &self,
        candidate_key: ReachabilityGenerationKey,
    ) -> Result<(), ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        let transaction = storage
            .candidate_transaction
            .filter(|transaction| transaction.candidate == candidate_key)
            .ok_or(ReachabilityStoreError::CandidateTransactionMismatch)?;
        if transaction.phase != CandidateTransactionPhase::DestinationRejectReturned {
            return Err(ReachabilityStoreError::CandidateTransactionMismatch);
        }
        storage.candidate_transaction = None;
        Ok(())
    }

    /// Last-resort ownership cleanup. Normal acceptance and rejection call the
    /// explicit methods above; `Drop` uses this only to keep an unwinding host
    /// from stranding a loaned accepted owner.
    pub(crate) fn drop_generation(&self, key: ReachabilityGenerationKey) {
        let transaction = self.storage.borrow().candidate_transaction;
        if let Some(transaction) = transaction {
            if transaction.phase == CandidateTransactionPhase::CandidateLive {
                let _ = self.prepare_candidate_reject(transaction.candidate);
            }
            let _ = self.finish_candidate_reject(transaction.candidate);
        }
        let _ = self.take_generation(key);
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct ReachabilityGenerationKey {
    slot: usize,
    serial: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReachabilityStoreError {
    GenerationSlotsExhausted,
    GenerationIdentityExhausted,
    StaleGeneration,
    CandidateTransactionActive,
    CandidateTransactionMismatch,
}

fn two_slots_mut<T>(slots: &mut [T; 2], first: usize, second: usize) -> [&mut T; 2] {
    assert_ne!(first, second);
    if first < second {
        let (left, right) = slots.split_at_mut(second);
        [&mut left[first], &mut right[0]]
    } else {
        let (left, right) = slots.split_at_mut(first);
        [&mut right[0], &mut left[second]]
    }
}
