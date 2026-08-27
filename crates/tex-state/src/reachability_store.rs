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
    pdf_transaction: Option<PdfTransactionSlots>,
}

#[derive(Clone, Copy)]
struct PdfTransactionSlots {
    source: ReachabilityGenerationKey,
    candidate: ReachabilityGenerationKey,
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
                pdf_transaction: None,
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

    pub(crate) fn generation_has_pdf_candidate(&self, key: ReachabilityGenerationKey) -> bool {
        self.storage
            .borrow()
            .pdf_transaction
            .is_some_and(|transaction| transaction.source == key)
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
        assert!(storage.pdf_transaction.is_none());
        storage.pdf_transaction = Some(PdfTransactionSlots { source, candidate });
        Ok(candidate)
    }

    pub(crate) fn with_generation_mut<R>(
        &self,
        key: ReachabilityGenerationKey,
        operation: impl FnOnce(&mut PhysicalStateGeneration) -> R,
    ) -> Result<R, ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        if storage
            .pdf_transaction
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

    pub(crate) fn take_generation(
        &self,
        key: ReachabilityGenerationKey,
    ) -> Result<PhysicalStateGeneration, ReachabilityStoreError> {
        let mut storage = self.storage.borrow_mut();
        if let Some(transaction) = storage.pdf_transaction {
            if transaction.candidate == key {
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
                    .return_rejected_pdf_from(&mut candidate.universe);
                storage.pdf_transaction = None;
            } else if transaction.source == key {
                let candidate = storage
                    .slots
                    .get_mut(transaction.candidate.slot)
                    .and_then(|slot| slot.generation.as_mut())
                    .ok_or(ReachabilityStoreError::StaleGeneration)?;
                candidate.universe.commit_pdf_candidate();
                storage.pdf_transaction = None;
            }
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
