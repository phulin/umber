//! Reusable weak-slot storage for reachability-owned immutable values.
//!
//! Strong references live in typed semantic roots. This module owns only
//! timeline-local coordinates and a bounded, non-authoritative lookup index.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::sync::{Arc, Weak};

use crate::identity::{HandleIdentity, ReusableIdentityAllocator};

const DEFAULT_INDEX_KEY_BUDGET: usize = 1_024;
const INDEX_BUCKET_ENTRY_BUDGET: usize = 64;
const RECLAIM_WORK_PER_OPERATION: usize = 8;

/// Deterministic primitive work performed by one weak-pool lookup.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LookupWork {
    pub(crate) fixed_root_probes: usize,
    pub(crate) generation_checks: usize,
    pub(crate) slot_probes: usize,
    pub(crate) weak_upgrades: usize,
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
            + self.weak_upgrades
            + self.candidate_entries
            + self.exact_comparisons
            + self.patch_lease_probes
    }
}

/// One owning reference to an immutable exact-content value.
pub(crate) struct ReachableValueRef<T> {
    object: Arc<ReachableValueObject<T>>,
}

struct ReachableValueObject<T> {
    identity: HandleIdentity,
    value: Arc<T>,
}

impl<T> Clone for ReachableValueRef<T> {
    fn clone(&self) -> Self {
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_arc_retain();
        Self {
            object: Arc::clone(&self.object),
        }
    }
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
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_arc_retain();
        Arc::clone(&self.object.value)
    }

    #[cfg(test)]
    pub(crate) fn strong_count(&self) -> usize {
        Arc::strong_count(&self.object)
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
        }),
    }
}

#[derive(Debug)]
struct WeakSlot<T> {
    identity: HandleIdentity,
    value: Weak<ReachableValueObject<T>>,
}

/// Weak reusable slots plus a bounded candidate index.
///
/// `K` is only a candidate key. `intern` always invokes exact equality before
/// reusing a live object, so key collisions cannot alias content.
#[derive(Debug)]
pub(crate) struct ReachableValuePool<K, T> {
    identities: ReusableIdentityAllocator,
    slots: Vec<Option<WeakSlot<T>>>,
    sweep_cursor: usize,
    index: HashMap<K, Vec<u32>>,
    index_key_budget: usize,
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
                    slot.as_ref().map(|slot| WeakSlot {
                        identity: slot.identity,
                        value: {
                            #[cfg(feature = "profiling")]
                            crate::measurement::record_hot_core_weak_retain();
                            slot.value.clone()
                        },
                    })
                })
                .collect(),
            sweep_cursor: self.sweep_cursor,
            index: self.index.clone(),
            index_key_budget: self.index_key_budget,
        }
    }
}

impl<K, T> ReachableValuePool<K, T>
where
    K: Clone + Eq + Hash,
{
    pub(crate) fn new() -> Self {
        Self::with_index_key_budget(DEFAULT_INDEX_KEY_BUDGET)
    }

    /// Installs a validated immutable prefix and returns its explicit owners.
    ///
    /// Fixed values participate in coordinate resolution but not in the
    /// bounded dynamic candidate index. A family-specific frozen lookup can
    /// select them without making weak index membership authoritative.
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
            });
            slots.push(Some(WeakSlot {
                identity,
                value: {
                    #[cfg(feature = "profiling")]
                    crate::measurement::record_hot_core_weak_retain();
                    Arc::downgrade(&value)
                },
            }));
            roots.push(ReachableValueRef { object: value });
        }
        (
            Self {
                identities,
                slots,
                sweep_cursor: 0,
                index: HashMap::new(),
                index_key_budget: DEFAULT_INDEX_KEY_BUDGET,
            },
            roots,
        )
    }

    fn with_index_key_budget(index_key_budget: usize) -> Self {
        Self {
            identities: ReusableIdentityAllocator::new(0),
            slots: Vec::new(),
            sweep_cursor: 0,
            index: HashMap::new(),
            index_key_budget,
        }
    }

    /// Reuses an exact live value or installs one new weak slot.
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
        #[cfg(feature = "profiling")]
        let mut candidate_entries = 0;
        #[cfg(feature = "profiling")]
        let mut exact_comparisons = 0;
        self.reclaim_some_dead_slots(RECLAIM_WORK_PER_OPERATION);
        let mut exact = None;
        let mut remove_empty_candidates = false;
        if let Some(candidates) = self.index.get_mut(key) {
            candidates.retain(|&raw| {
                #[cfg(feature = "profiling")]
                {
                    candidate_entries += 1;
                }
                let Some(slot) = self.slots.get(raw as usize).and_then(Option::as_ref) else {
                    return false;
                };
                let upgraded = slot.value.upgrade();
                #[cfg(feature = "profiling")]
                crate::measurement::record_hot_core_weak_upgrade(upgraded.is_some());
                let Some(candidate) = upgraded else {
                    return false;
                };
                #[cfg(feature = "profiling")]
                {
                    exact_comparisons += 1;
                }
                if exact.is_none() && exact_eq(&candidate.value) {
                    exact = Some(ReachableValueRef { object: candidate });
                }
                true
            });
            remove_empty_candidates = candidates.is_empty();
        }
        if remove_empty_candidates {
            self.index.remove(key);
        }
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_weak_index(candidate_entries, exact_comparisons);
        exact
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

    fn insert(&mut self, value: T, key: Option<K>) -> ReachableValueRef<T> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = crate::measurement::hot_core_allocation_scope(
            crate::measurement::HotCoreAllocationOwner::WeakValueStore,
        );
        self.reclaim_some_dead_slots(RECLAIM_WORK_PER_OPERATION);
        let identity = self
            .identities
            .allocate()
            .expect("reachable-value identity capacity exhausted");
        let shared = Arc::new(ReachableValueObject {
            identity,
            value: Arc::new(value),
        });
        let raw = identity.slot() as usize;
        if raw == self.slots.len() {
            self.slots.push(None);
        }
        assert!(raw < self.slots.len(), "reusable value slot exceeds table");
        assert!(self.slots[raw].is_none(), "reusable value slot is occupied");
        self.slots[raw] = Some(WeakSlot {
            identity,
            value: {
                #[cfg(feature = "profiling")]
                crate::measurement::record_hot_core_weak_retain();
                Arc::downgrade(&shared)
            },
        });
        if let Some(key) = key.filter(|_| self.index_key_budget != 0) {
            if self.index.len() >= self.index_key_budget && !self.index.contains_key(&key) {
                self.index.clear();
            }
            let bucket = self.index.entry(key).or_default();
            if bucket.len() >= INDEX_BUCKET_ENTRY_BUDGET {
                bucket.clear();
            }
            bucket.push(identity.slot());
        }
        ReachableValueRef { object: shared }
    }

    /// Resolves one exact live coordinate without making the index authority.
    pub(crate) fn resolve(&self, identity: HandleIdentity) -> Option<ReachableValueRef<T>> {
        if !self.identities.contains(identity) {
            return None;
        }
        let slot = self.slots.get(identity.slot() as usize)?.as_ref()?;
        if slot.identity != identity {
            return None;
        }
        let upgraded = slot.value.upgrade();
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_weak_upgrade(upgraded.is_some());
        Some(ReachableValueRef { object: upgraded? })
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
        work.weak_upgrades += 1;
        (
            slot.value
                .upgrade()
                .map(|object| ReachableValueRef { object }),
            work,
        )
    }

    /// Resolves the currently live value in one physical slot.
    ///
    /// This is a compact-coordinate projection, not an ownership query: the
    /// weak slot upgrades only while a typed semantic owner already exists.
    pub(crate) fn resolve_slot(&self, raw: u32) -> Option<ReachableValueRef<T>> {
        let slot = self.slots.get(raw as usize)?.as_ref()?;
        let upgraded = slot.value.upgrade();
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_weak_upgrade(upgraded.is_some());
        Some(ReachableValueRef { object: upgraded? })
    }

    /// Executes the production stored-slot branches with deterministic work
    /// accounting for a benchmark or regression gate.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_resolve_slot(
        &self,
        raw: u32,
    ) -> (Option<ReachableValueRef<T>>, LookupWork) {
        let mut work = LookupWork {
            slot_probes: 1,
            ..LookupWork::default()
        };
        let Some(slot) = self.slots.get(raw as usize).and_then(Option::as_ref) else {
            return (None, work);
        };
        work.weak_upgrades += 1;
        (
            slot.value
                .upgrade()
                .map(|object| ReachableValueRef { object }),
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
        if let Some(candidates) = self.index.get(key) {
            for &raw in candidates {
                work.candidate_entries += 1;
                work.slot_probes += 1;
                let Some(slot) = self.slots.get(raw as usize).and_then(Option::as_ref) else {
                    continue;
                };
                work.weak_upgrades += 1;
                let Some(candidate) = slot.value.upgrade() else {
                    continue;
                };
                work.exact_comparisons += 1;
                if exact.is_none() && exact_eq(&candidate.value) {
                    exact = Some(ReachableValueRef { object: candidate });
                }
            }
        }
        (exact, work)
    }

    /// Returns the physical slot-table extent used by compact projections.
    pub(crate) fn slot_len(&self) -> usize {
        self.slots.len()
    }

    /// Validates a typed coordinate without upgrading the value's weak slot.
    pub(crate) fn contains_identity(&self, identity: HandleIdentity) -> bool {
        self.identities.contains(identity)
    }

    /// Advances a bounded weak-metadata sweep. The strong owner has already
    /// destroyed a dead value, so interning need not rescan every live slot
    /// before doing useful work. Reclaimed identities remain generation-safe
    /// through `ReusableIdentityAllocator`.
    fn reclaim_some_dead_slots(&mut self, work: usize) -> usize {
        let mut visited = 0;
        while visited < work && !self.slots.is_empty() {
            if self.sweep_cursor >= self.slots.len() {
                self.sweep_cursor = 0;
            }
            let index = self.sweep_cursor;
            self.sweep_cursor += 1;
            visited += 1;
            let slot = &mut self.slots[index];
            let Some(occupied) = slot else {
                continue;
            };
            if occupied.value.strong_count() != 0 {
                continue;
            }
            self.identities
                .release(occupied.identity)
                .expect("weak value slot and identity table diverged");
            *slot = None;
        }
        visited
    }

    /// Biases the next bounded sweep toward a rollback suffix.
    ///
    /// The caller supplies only the physical extent captured in its O(1)
    /// operation mark. No slot is released here: the ordinary generation-safe
    /// sweep still checks the weak owner after restoration has dropped the
    /// discarded roots.
    pub(crate) fn prioritize_reclamation_from(&mut self, slot: usize) {
        self.sweep_cursor = slot.min(self.slots.len());
    }

    #[cfg(test)]
    fn clear_index(&mut self) {
        self.index.clear();
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_shape(&self) -> (usize, usize, usize, usize, usize, usize) {
        let (identity_slots, identity_capacity, free) = self.identities.testing_shape();
        debug_assert_eq!(identity_slots, self.slots.len());
        (
            identity_slots,
            identity_capacity,
            self.index.len(),
            self.index.capacity(),
            self.index.values().map(Vec::capacity).max().unwrap_or(0),
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
            .filter_map(|slot| slot.as_ref()?.value.upgrade())
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
