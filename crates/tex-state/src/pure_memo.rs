//! Optional bounded storage for detached pure-query results.
//!
//! The runtime is operational session metadata: it is excluded from snapshots,
//! formats, and semantic hashes. Disabled execution is one `Option` branch and
//! uses no locks or atomics.

use crate::{ContentHash, DetachedMemoValue};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PureMemoConfig {
    pub max_entries: usize,
    pub max_retained_bytes: usize,
}

impl Default for PureMemoConfig {
    fn default() -> Self {
        Self {
            max_entries: 1_024,
            max_retained_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PureMemoStats {
    pub lookups: u64,
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
    pub malformed: u64,
    pub retained_entries: usize,
    pub retained_bytes: usize,
}

/// Strong key used to verify a compact candidate bucket.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PureMemoKey {
    domain: u32,
    candidate: u64,
    integrity: ContentHash,
}

impl PureMemoKey {
    #[must_use]
    pub const fn new(domain: u32, candidate: u64, integrity: ContentHash) -> Self {
        Self {
            domain,
            candidate,
            integrity,
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    value: DetachedMemoValue,
    charge: usize,
}

#[derive(Clone, Debug)]
struct PureMemoCache {
    config: PureMemoConfig,
    entries: HashMap<PureMemoKey, Entry>,
    insertion_order: VecDeque<PureMemoKey>,
    stats: PureMemoStats,
}

/// Opaque operational cache owned by a long-lived execution session.
///
/// Moving this runtime between a session and a scratch [`crate::Universe`]
/// keeps memo contents out of semantic state while preserving them across
/// accepted editor revisions.
#[derive(Clone, Debug, Default)]
pub struct PureMemoRuntime {
    cache: Option<PureMemoCache>,
}

impl PureMemoRuntime {
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.cache.is_some()
    }

    pub(crate) fn enable(&mut self, config: PureMemoConfig) {
        self.cache = Some(PureMemoCache {
            config,
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            stats: PureMemoStats::default(),
        });
    }

    pub(crate) fn disable(&mut self) {
        self.cache = None;
    }

    pub(crate) fn lookup(&mut self, key: PureMemoKey) -> Option<DetachedMemoValue> {
        let cache = self.cache.as_mut()?;
        cache.stats.lookups = cache.stats.lookups.saturating_add(1);
        let hit = cache.entries.get(&key).map(|entry| entry.value.clone());
        if hit.is_some() {
            cache.stats.hits = cache.stats.hits.saturating_add(1);
        } else {
            cache.stats.misses = cache.stats.misses.saturating_add(1);
        }
        hit
    }

    pub(crate) fn insert(&mut self, key: PureMemoKey, value: DetachedMemoValue) {
        let Some(cache) = self.cache.as_mut() else {
            return;
        };
        let payload_bytes = value
            .retained_bytes()
            .saturating_sub(std::mem::size_of::<DetachedMemoValue>());
        // Charge the map key and FIFO key as well as the entry and owned payload.
        let charge = std::mem::size_of::<Entry>()
            .saturating_add(std::mem::size_of::<PureMemoKey>().saturating_mul(2))
            .saturating_add(payload_bytes);
        if cache.config.max_entries == 0 || charge > cache.config.max_retained_bytes {
            return;
        }
        if let Some(entry) = cache.entries.get_mut(&key) {
            cache.stats.retained_bytes = cache
                .stats
                .retained_bytes
                .saturating_sub(entry.charge)
                .saturating_add(charge);
            entry.value = value;
            entry.charge = charge;
        } else {
            cache.entries.insert(key, Entry { value, charge });
            cache.insertion_order.push_back(key);
            cache.stats.inserts = cache.stats.inserts.saturating_add(1);
            cache.stats.retained_entries = cache.stats.retained_entries.saturating_add(1);
            cache.stats.retained_bytes = cache.stats.retained_bytes.saturating_add(charge);
        }
        cache.evict_to_budget();
    }

    pub(crate) fn reject(&mut self, key: PureMemoKey) {
        let Some(cache) = self.cache.as_mut() else {
            return;
        };
        cache.stats.malformed = cache.stats.malformed.saturating_add(1);
        cache.remove(key, false);
    }

    #[must_use]
    pub fn stats(&self) -> PureMemoStats {
        self.cache
            .as_ref()
            .map_or_else(PureMemoStats::default, |cache| cache.stats)
    }
}

impl PureMemoCache {
    fn evict_to_budget(&mut self) {
        while self.stats.retained_entries > self.config.max_entries
            || self.stats.retained_bytes > self.config.max_retained_bytes
        {
            let Some(key) = self.insertion_order.pop_front() else {
                break;
            };
            self.remove(key, true);
        }
    }

    fn remove(&mut self, key: PureMemoKey, eviction: bool) {
        let Some(entry) = self.entries.remove(&key) else {
            return;
        };
        self.stats.retained_entries = self.stats.retained_entries.saturating_sub(1);
        self.stats.retained_bytes = self.stats.retained_bytes.saturating_sub(entry.charge);
        if eviction {
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests;
