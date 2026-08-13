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

/// One owning reference to an immutable exact-content value.
pub(crate) struct ReachableValueRef<T> {
    identity: HandleIdentity,
    value: Arc<T>,
}

impl<T> Clone for ReachableValueRef<T> {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity,
            value: Arc::clone(&self.value),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for ReachableValueRef<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReachableValueRef")
            .field("identity", &self.identity)
            .field("value", &self.value)
            .finish()
    }
}

impl<T> ReachableValueRef<T> {
    pub(crate) const fn identity(&self) -> HandleIdentity {
        self.identity
    }

    pub(crate) fn value(&self) -> &T {
        &self.value
    }

    pub(crate) fn shared(&self) -> Arc<T> {
        Arc::clone(&self.value)
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.value, &other.value)
    }
}

#[derive(Debug)]
struct WeakSlot<T> {
    identity: HandleIdentity,
    value: Weak<T>,
}

/// Weak reusable slots plus a bounded candidate index.
///
/// `K` is only a candidate key. `intern` always invokes exact equality before
/// reusing a live object, so key collisions cannot alias content.
#[derive(Debug)]
pub(crate) struct ReachableValuePool<K, T> {
    identities: ReusableIdentityAllocator,
    slots: Vec<Option<WeakSlot<T>>>,
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
                        value: slot.value.clone(),
                    })
                })
                .collect(),
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
            let value = Arc::new(value);
            slots.push(Some(WeakSlot {
                identity,
                value: Arc::downgrade(&value),
            }));
            roots.push(ReachableValueRef { identity, value });
        }
        (
            Self {
                identities,
                slots,
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
        self.reclaim_dead_slots();
        if let Some(candidates) = self.index.get(key) {
            for &raw in candidates {
                let Some(slot) = self.slots.get(raw as usize).and_then(Option::as_ref) else {
                    continue;
                };
                let Some(candidate) = slot.value.upgrade() else {
                    continue;
                };
                if exact_eq(&candidate) {
                    return Some(ReachableValueRef {
                        identity: slot.identity,
                        value: candidate,
                    });
                }
            }
        }
        None
    }

    /// Installs a value after the caller has performed exact candidate lookup.
    pub(crate) fn insert_new(&mut self, key: K, value: T) -> ReachableValueRef<T> {
        let identity = self
            .identities
            .allocate()
            .expect("reachable-value identity capacity exhausted");
        let shared = Arc::new(value);
        let raw = identity.slot() as usize;
        if raw == self.slots.len() {
            self.slots.push(None);
        }
        assert!(raw < self.slots.len(), "reusable value slot exceeds table");
        assert!(self.slots[raw].is_none(), "reusable value slot is occupied");
        self.slots[raw] = Some(WeakSlot {
            identity,
            value: Arc::downgrade(&shared),
        });
        if self.index_key_budget != 0 {
            if self.index.len() >= self.index_key_budget && !self.index.contains_key(&key) {
                self.index.clear();
            }
            let bucket = self.index.entry(key).or_default();
            if bucket.len() >= INDEX_BUCKET_ENTRY_BUDGET {
                bucket.clear();
            }
            bucket.push(identity.slot());
        }
        ReachableValueRef {
            identity,
            value: shared,
        }
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
        Some(ReachableValueRef {
            identity,
            value: slot.value.upgrade()?,
        })
    }

    fn reclaim_dead_slots(&mut self) {
        // Release high slots first so the allocator's stack returns the
        // lowest reusable coordinate. This is nonsemantic, but it keeps the
        // temporary dense compatibility prefix gap-free during migration.
        for slot in self.slots.iter_mut().rev() {
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
    }

    #[cfg(test)]
    fn clear_index(&mut self) {
        self.index.clear();
    }

    #[cfg(test)]
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
}

#[cfg(test)]
mod tests;
