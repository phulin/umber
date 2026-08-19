//! Region-owned immutable values with generation-checked dense coordinates.
//!
//! The arena is the sole liveness authority. Typed roots share immutable
//! payloads, while rollback explicitly truncates the region and advances its
//! generation before a suffix coordinate can be reused.

use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::identity::{HandleIdentity, ReusableIdentityAllocator};

const RECLAIM_WORK_PER_OPERATION: usize = 8;

/// Deterministic primitive work performed by one arena lookup.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LookupWork {
    pub(crate) fixed_root_probes: usize,
    pub(crate) generation_checks: usize,
    pub(crate) slot_probes: usize,
    pub(crate) owner_clones: usize,
    pub(crate) candidate_entries: usize,
    pub(crate) exact_comparisons: usize,
    pub(crate) patch_lease_probes: usize,
}

#[cfg(any(test, feature = "testing"))]
impl LookupWork {
    pub(crate) const fn total(self) -> usize {
        self.fixed_root_probes
            + self.generation_checks
            + self.slot_probes
            + self.owner_clones
            + self.candidate_entries
            + self.exact_comparisons
            + self.patch_lease_probes
    }
}

/// One owning reference to an immutable exact-content value.
pub(crate) struct ReachableValueRef<T> {
    object: Arc<ReachableValueObject<T>>,
}

impl<T> Clone for ReachableValueRef<T> {
    fn clone(&self) -> Self {
        Self {
            object: Arc::clone(&self.object),
        }
    }
}

#[derive(Debug)]
struct ReachableValueObject<T> {
    identity: HandleIdentity,
    value: Arc<T>,
    #[cfg(test)]
    region_memberships: AtomicUsize,
}

impl<T: fmt::Debug> fmt::Debug for ReachableValueRef<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReachableValueRef")
            .field("identity", &self.object.identity)
            .field("value", &self.object.value)
            .finish()
    }
}

impl<T> ReachableValueRef<T> {
    pub(crate) fn identity(&self) -> HandleIdentity {
        self.object.identity
    }

    pub(crate) fn value(&self) -> &T {
        &self.object.value
    }

    pub(crate) fn shared(&self) -> Arc<T> {
        Arc::clone(&self.object.value)
    }

    #[cfg(test)]
    pub(crate) fn strong_count(&self) -> usize {
        Arc::strong_count(&self.object)
            .saturating_sub(self.object.region_memberships.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object)
    }
}

#[cfg(any(test, feature = "testing"))]
pub(crate) fn testing_value_ref<T>(identity: HandleIdentity, value: T) -> ReachableValueRef<T> {
    ReachableValueRef {
        object: Arc::new(ReachableValueObject {
            identity,
            value: Arc::new(value),
            #[cfg(test)]
            region_memberships: AtomicUsize::new(0),
        }),
    }
}

#[derive(Debug)]
struct ArenaSlot<K, T> {
    identity: HandleIdentity,
    key: Option<K>,
    value: Arc<ReachableValueObject<T>>,
}

/// Strong region slots plus a cold exact-comparison path.
///
/// `K` is only a candidate key. `intern` always invokes exact equality before
/// reusing a live object, so key collisions cannot alias content.
#[derive(Debug)]
pub(crate) struct ReachableValuePool<K, T> {
    identities: ReusableIdentityAllocator,
    slots: Vec<Option<ArenaSlot<K, T>>>,
    allocation_events: Vec<HandleIdentity>,
    sweep_cursor: usize,
    marker: PhantomData<K>,
}

impl<K, T> Clone for ReachableValuePool<K, T>
where
    K: Clone,
{
    fn clone(&self) -> Self {
        Self {
            identities: self.identities.fork(),
            slots: self
                .slots
                .iter()
                .map(|slot| {
                    slot.as_ref().map(|slot| {
                        #[cfg(test)]
                        {
                            slot.value
                                .region_memberships
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        ArenaSlot {
                            identity: slot.identity,
                            key: slot.key.clone(),
                            value: Arc::clone(&slot.value),
                        }
                    })
                })
                .collect(),
            allocation_events: self.allocation_events.clone(),
            sweep_cursor: self.sweep_cursor,
            marker: PhantomData,
        }
    }
}

#[cfg(test)]
impl<K, T> Drop for ReachableValuePool<K, T> {
    fn drop(&mut self) {
        for slot in self.slots.iter().flatten() {
            let previous = slot
                .value
                .region_memberships
                .fetch_sub(1, Ordering::Relaxed);
            assert_ne!(previous, 0, "arena region membership underflow");
        }
    }
}

impl<K, T> ReachableValuePool<K, T>
where
    K: Clone + Eq + Hash,
{
    pub(crate) fn new() -> Self {
        Self::with_index_key_budget(0)
    }

    /// Installs a validated immutable prefix and returns its explicit owners.
    ///
    /// Fixed values participate in coordinate resolution but not in the
    /// dynamic exact key. A family-specific frozen lookup selects them.
    pub(crate) fn from_fixed_values(
        values: Vec<T>,
        builtin_slots: u32,
    ) -> (Self, Vec<ReachableValueRef<T>>) {
        let fixed_slots = u32::try_from(values.len()).expect("fixed value count exceeds u32");
        let identities = ReusableIdentityAllocator::from_fixed_len(builtin_slots, fixed_slots);
        let mut slots = Vec::with_capacity(values.len());
        let mut roots = Vec::with_capacity(values.len());
        for (raw, value) in values.into_iter().enumerate() {
            let identity = identities
                .identity_at(raw as u32)
                .expect("fixed value identity is live");
            let value = Arc::new(ReachableValueObject {
                identity,
                value: Arc::new(value),
                #[cfg(test)]
                region_memberships: AtomicUsize::new(1),
            });
            slots.push(Some(ArenaSlot {
                identity,
                key: None,
                value: Arc::clone(&value),
            }));
            roots.push(ReachableValueRef { object: value });
        }
        (
            Self {
                identities,
                slots,
                allocation_events: Vec::new(),
                sweep_cursor: 0,
                marker: PhantomData,
            },
            roots,
        )
    }

    fn with_index_key_budget(_index_key_budget: usize) -> Self {
        Self {
            identities: ReusableIdentityAllocator::new(0),
            slots: Vec::new(),
            allocation_events: Vec::new(),
            sweep_cursor: 0,
            marker: PhantomData,
        }
    }

    /// Reuses an exact live value or installs one new arena slot.
    pub(crate) fn intern(
        &mut self,
        key: K,
        value: T,
        exact_eq: impl Fn(&T, &T) -> bool,
    ) -> ReachableValueRef<T> {
        self.intern_with_status(key, value, exact_eq).0
    }

    /// Reuses one exact live object or returns a newly installed object plus
    /// the fact that the caller must attach its typed ownership metadata.
    pub(crate) fn intern_with_status(
        &mut self,
        key: K,
        value: T,
        exact_eq: impl Fn(&T, &T) -> bool,
    ) -> (ReachableValueRef<T>, bool) {
        if let Some(value) = self.find_exact(&key, |candidate| exact_eq(candidate, &value)) {
            return (value, false);
        }
        (self.insert_new(key, value), true)
    }

    /// Finds an exactly matching live candidate after reclaiming dead slots.
    pub(crate) fn find_exact(
        &mut self,
        key: &K,
        exact_eq: impl Fn(&T) -> bool,
    ) -> Option<ReachableValueRef<T>> {
        self.reclaim_some_dead_slots(RECLAIM_WORK_PER_OPERATION);
        self.slots.iter().flatten().find_map(|slot| {
            (slot.key.as_ref() == Some(key)
                && exact_eq(&slot.value.value))
            .then(|| ReachableValueRef {
                object: Arc::clone(&slot.value),
            })
        })
    }

    /// Installs a value after the caller has performed exact candidate lookup.
    pub(crate) fn insert_new(&mut self, key: K, value: T) -> ReachableValueRef<T> {
        self.insert(value, Some(key))
    }

    /// Installs a fresh runtime value without publishing it in the cold exact
    /// candidate index.
    ///
    /// Ordinary TeX execution gives each occurrence its own physical
    /// coordinate. Canonical identity and exact reuse are publication-barrier
    /// concerns, so indexing a value that no hot caller will query only grows
    /// metadata and turns allocation into hash-table work.
    pub(crate) fn insert_unindexed(&mut self, value: T) -> ReachableValueRef<T> {
        self.insert(value, None)
    }

    /// Reserves one generation-bearing coordinate whose value is owned by a
    /// family-specific immutable arena rather than by this pool's payload row.
    ///
    /// The empty slot is deliberate: identity validation remains shared with
    /// exact values, while hot arena values do not allocate a second owner
    /// merely to keep a coordinate live.
    pub(crate) fn reserve_external(&mut self) -> HandleIdentity {
        let identity = self
            .identities
            .allocate()
            .expect("reachable-value identity capacity exhausted");
        let raw = identity.slot() as usize;
        if raw == self.slots.len() {
            self.slots.push(None);
        }
        assert!(raw < self.slots.len(), "external value slot exceeds table");
        assert!(self.slots[raw].is_none(), "external value slot is occupied");
        self.allocation_events.push(identity);
        identity
    }

    /// Retires one coordinate whose payload was owned by a family-specific
    /// arena, advancing its generation before the slot can be reused.
    pub(crate) fn release_external(&mut self, identity: HandleIdentity) {
        let raw = identity.slot() as usize;
        assert!(
            self.slots.get(raw).is_some_and(Option::is_none),
            "external value slot is not reserved"
        );
        self.identities
            .release(identity)
            .expect("external arena coordinate is stale or foreign");
    }

    fn insert(&mut self, value: T, key: Option<K>) -> ReachableValueRef<T> {
        self.reclaim_some_dead_slots(RECLAIM_WORK_PER_OPERATION);
        let identity = self
            .identities
            .allocate()
            .expect("reachable-value identity capacity exhausted");
        let shared = Arc::new(ReachableValueObject {
            identity,
            value: Arc::new(value),
            #[cfg(test)]
            region_memberships: AtomicUsize::new(1),
        });
        let raw = identity.slot() as usize;
        if raw == self.slots.len() {
            self.slots.push(None);
        }
        assert!(raw < self.slots.len(), "reusable value slot exceeds table");
        assert!(self.slots[raw].is_none(), "reusable value slot is occupied");
        self.slots[raw] = Some(ArenaSlot {
            identity,
            key,
            value: Arc::clone(&shared),
        });
        self.allocation_events.push(identity);
        ReachableValueRef { object: shared }
    }

    /// Resolves one exact live arena coordinate.
    pub(crate) fn resolve(&self, identity: HandleIdentity) -> Option<ReachableValueRef<T>> {
        if !self.identities.contains(identity) {
            return None;
        }
        let slot = self.slots.get(identity.slot() as usize)?.as_ref()?;
        if slot.identity != identity {
            return None;
        }
        Some(ReachableValueRef {
            object: Arc::clone(&slot.value),
        })
    }

    /// Executes the production identity-resolution branches while reporting
    /// deterministic primitive work for focused regression gates.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_resolve(
        &self,
        identity: HandleIdentity,
    ) -> (Option<ReachableValueRef<T>>, LookupWork) {
        let mut work = LookupWork {
            generation_checks: 1,
            ..LookupWork::default()
        };
        if !self.identities.contains(identity) {
            return (None, work);
        }
        work.slot_probes += 1;
        let Some(slot) = self
            .slots
            .get(identity.slot() as usize)
            .and_then(Option::as_ref)
        else {
            return (None, work);
        };
        if slot.identity != identity {
            return (None, work);
        }
        (
            Some(ReachableValueRef {
                object: Arc::clone(&slot.value),
            }),
            work,
        )
    }

    /// Resolves the currently live value in one physical slot.
    ///
    /// This is a compact-coordinate projection, not an ownership query: the
    /// the returned owner shares the region-owned immutable payload.
    pub(crate) fn resolve_slot(&self, raw: u32) -> Option<ReachableValueRef<T>> {
        let slot = self.slots.get(raw as usize)?.as_ref()?;
        Some(ReachableValueRef {
            object: Arc::clone(&slot.value),
        })
    }

    /// Executes the production stored-slot branches with deterministic work
    /// accounting for a benchmark or regression gate.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_resolve_slot(
        &self,
        raw: u32,
    ) -> (Option<ReachableValueRef<T>>, LookupWork) {
        let work = LookupWork {
            slot_probes: 1,
            ..LookupWork::default()
        };
        let Some(slot) = self.slots.get(raw as usize).and_then(Option::as_ref) else {
            return (None, work);
        };
        (
            Some(ReachableValueRef {
                object: Arc::clone(&slot.value),
            }),
            work,
        )
    }

    /// Probes one collision bucket with the same exact-content policy as
    /// `find_exact`, without advancing reclamation for benchmark repeatability.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_find_exact(
        &self,
        key: &K,
        exact_eq: impl Fn(&T) -> bool,
    ) -> (Option<ReachableValueRef<T>>, LookupWork) {
        let mut work = LookupWork::default();
        let mut exact = None;
        for slot in self.slots.iter().flatten() {
            if slot.key.as_ref() == Some(key) {
                work.candidate_entries += 1;
                work.slot_probes += 1;
                work.exact_comparisons += 1;
                if exact.is_none() && exact_eq(&slot.value.value) {
                    exact = Some(ReachableValueRef {
                        object: Arc::clone(&slot.value),
                    });
                }
            }
        }
        (exact, work)
    }

    /// Returns the physical slot-table extent used by compact projections.
    pub(crate) fn slot_len(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn allocation_mark(&self) -> usize {
        self.allocation_events.len()
    }

    /// Retires all allocations after a mark, including coordinates that
    /// reused holes below the mark's physical slot-table extent.
    pub(crate) fn rollback_to_allocation_mark(&mut self, mark: usize) {
        assert!(
            mark <= self.allocation_events.len(),
            "arena allocation mark is ahead of state"
        );
        while self.allocation_events.len() > mark {
            let identity = self
                .allocation_events
                .pop()
                .expect("allocation event journal is nonempty");
            if !self.identities.contains(identity) {
                continue;
            }
            let raw = identity.slot() as usize;
            if self.slots[raw]
                .as_ref()
                .is_some_and(|slot| slot.identity != identity)
            {
                continue;
            }
            self.clear_slot(raw);
            self.identities
                .release(identity)
                .expect("arena allocation event and identity table diverged");
        }
        self.sweep_cursor = self.sweep_cursor.min(self.slots.len());
    }

    /// Validates a typed coordinate without cloning its immutable payload.
    pub(crate) fn contains_identity(&self, identity: HandleIdentity) -> bool {
        self.identities.contains(identity)
    }

    /// Returns whether a live coordinate is owned by a family-specific arena
    /// rather than by an exact-value payload row in this pool.
    pub(crate) fn contains_external_identity(&self, identity: HandleIdentity) -> bool {
        self.identities.contains(identity)
            && self
                .slots
                .get(identity.slot() as usize)
                .is_some_and(Option::is_none)
    }

    fn reclaim_some_dead_slots(&mut self, work: usize) -> usize {
        let mut visited = 0;
        while visited < work && !self.slots.is_empty() {
            if self.sweep_cursor >= self.slots.len() {
                self.sweep_cursor = 0;
            }
            let index = self.sweep_cursor;
            self.sweep_cursor += 1;
            visited += 1;
            let Some(slot) = &self.slots[index] else {
                continue;
            };
            if Arc::strong_count(&slot.value) != 1 {
                continue;
            }
            let identity = slot.identity;
            self.clear_slot(index);
            self.identities
                .release(identity)
                .expect("arena slot and identity table diverged");
        }
        visited
    }

    /// Reclaims an unowned rollback suffix immediately and biases later
    /// bounded reclamation toward any externally retained values in it.
    pub(crate) fn prioritize_reclamation_from(&mut self, slot: usize) {
        assert!(slot <= self.slots.len(), "arena rollback mark is ahead of state");
        for index in slot..self.slots.len() {
            let Some(entry) = &self.slots[index] else {
                continue;
            };
            if Arc::strong_count(&entry.value) != 1 {
                continue;
            }
            let identity = entry.identity;
            self.clear_slot(index);
            self.identities
                .release(identity)
                .expect("arena rollback slot and identity table diverged");
        }
        self.sweep_cursor = slot.min(self.slots.len());
    }

    fn clear_slot(&mut self, index: usize) {
        let _slot = self.slots[index].take();
        #[cfg(test)]
        if let Some(slot) = &_slot {
            let previous = slot
                .value
                .region_memberships
                .fetch_sub(1, Ordering::Relaxed);
            assert_ne!(previous, 0, "arena region membership underflow");
        }
    }

    #[cfg(test)]
    fn clear_index(&mut self) {}

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_shape(&self) -> (usize, usize, usize, usize, usize, usize) {
        let (identity_slots, identity_capacity, free) = self.identities.testing_shape();
        debug_assert_eq!(identity_slots, self.slots.len());
        (
            identity_slots,
            identity_capacity,
            0,
            0,
            0,
            free,
        )
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_live_totals(
        &self,
        logical_bytes: impl Fn(&T) -> usize,
    ) -> (usize, usize) {
        self.slots
            .iter()
            .filter_map(|slot| {
                slot.as_ref().map(|slot| &slot.value)
            })
            .fold((0, 0), |(objects, bytes), value| {
                (
                    objects + 1,
                    bytes.saturating_add(logical_bytes(&value.value)),
                )
            })
    }
}

#[cfg(test)]
mod tests;
