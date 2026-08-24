//! Session-epoch owner for retained revision generations.

use std::sync::{Arc, Mutex};

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
}

/// Coarse owner of the one reachability domain shared by a session's prior
/// and current retained generations.
///
/// The single `Arc` allocation is made at session construction so a suspended
/// candidate can remain practical across host turns without a self-reference.
/// Generation creation reuses one of two inline slots and performs no
/// reachability-control allocation. Runtime values never clone this owner.
#[derive(Clone)]
pub struct ReachabilityStore {
    epoch: SessionInternerEpoch,
    storage: Arc<Mutex<ReachabilityStorage>>,
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
    /// fixed two-slot array is part of this one coarse allocation; callers
    /// cannot combine the store with a foreign or separately retained epoch.
    #[must_use]
    pub fn new(interner_budget: InternerBudget) -> Self {
        Self {
            epoch: SessionInternerEpoch::new(interner_budget),
            storage: Arc::new(Mutex::new(ReachabilityStorage {
                next_serial: 1,
                slots: std::array::from_fn(|_| ReachabilitySlot::default()),
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
        Arc::ptr_eq(&self.storage, &other.storage)
    }

    /// Number of occupied prior/current slots. This is a cold lifecycle
    /// projection and is never consulted by value access.
    #[must_use]
    pub fn live_generation_count(&self) -> usize {
        self.storage
            .lock()
            .expect("reachability store lock poisoned")
            .slots
            .iter()
            .filter(|slot| slot.generation.is_some())
            .count()
    }

    pub(crate) fn insert_generation(
        &self,
        generation: PhysicalStateGeneration,
    ) -> Result<ReachabilityGenerationKey, ReachabilityStoreError> {
        let mut storage = self
            .storage
            .lock()
            .expect("reachability store lock poisoned");
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

    pub(crate) fn with_generation_mut<R>(
        &self,
        key: ReachabilityGenerationKey,
        operation: impl FnOnce(&mut PhysicalStateGeneration) -> R,
    ) -> Result<R, ReachabilityStoreError> {
        let mut storage = self
            .storage
            .lock()
            .expect("reachability store lock poisoned");
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
        let mut storage = self
            .storage
            .lock()
            .expect("reachability store lock poisoned");
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

#[derive(Clone, Copy)]
pub(crate) struct ReachabilityGenerationKey {
    slot: usize,
    serial: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReachabilityStoreError {
    GenerationSlotsExhausted,
    GenerationIdentityExhausted,
    StaleGeneration,
}
