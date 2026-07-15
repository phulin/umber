//! Optional bounded storage for detached pure-query results.
//!
//! The runtime is operational session metadata: it is excluded from snapshots,
//! formats, and semantic hashes. Disabled execution is one `Option` branch and
//! uses no locks or atomics.

use crate::{ContentHash, DetachedMemoValue};
use std::collections::{BTreeMap, VecDeque};

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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    key: PureMemoKey,
    value: DetachedMemoValue,
    charge: usize,
}

#[derive(Clone, Debug)]
struct PureMemoCache {
    config: PureMemoConfig,
    buckets: BTreeMap<u64, Vec<Entry>>,
    insertion_order: VecDeque<PureMemoKey>,
    stats: PureMemoStats,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PureMemoRuntime {
    cache: Option<PureMemoCache>,
}

impl PureMemoRuntime {
    pub(crate) const fn is_enabled(&self) -> bool {
        self.cache.is_some()
    }

    pub(crate) fn enable(&mut self, config: PureMemoConfig) {
        self.cache = Some(PureMemoCache {
            config,
            buckets: BTreeMap::new(),
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
        let hit = cache
            .buckets
            .get(&key.candidate)
            .and_then(|bucket| bucket.iter().find(|entry| entry.key == key))
            .map(|entry| entry.value.clone());
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
        let charge = std::mem::size_of::<Entry>().saturating_add(value.retained_bytes());
        if cache.config.max_entries == 0 || charge > cache.config.max_retained_bytes {
            return;
        }
        let bucket = cache.buckets.entry(key.candidate).or_default();
        if let Some(entry) = bucket.iter_mut().find(|entry| entry.key == key) {
            cache.stats.retained_bytes = cache
                .stats
                .retained_bytes
                .saturating_sub(entry.charge)
                .saturating_add(charge);
            entry.value = value;
            entry.charge = charge;
        } else {
            bucket.push(Entry { key, value, charge });
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

    pub(crate) fn stats(&self) -> PureMemoStats {
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
        let Some(bucket) = self.buckets.get_mut(&key.candidate) else {
            return;
        };
        let Some(index) = bucket.iter().position(|entry| entry.key == key) else {
            return;
        };
        let entry = bucket.swap_remove(index);
        self.stats.retained_entries = self.stats.retained_entries.saturating_sub(1);
        self.stats.retained_bytes = self.stats.retained_bytes.saturating_sub(entry.charge);
        if eviction {
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
        if bucket.is_empty() {
            self.buckets.remove(&key.candidate);
        }
    }
}

#[cfg(test)]
mod tests;
