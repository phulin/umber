//! Optional bounded storage for detached pure-query results.
//!
//! The runtime is operational session metadata: it is excluded from snapshots,
//! formats, and semantic hashes. Disabled execution is one `Option` branch and
//! uses no locks or atomics.

use crate::env::banks::IntParam;
use crate::glue::GlueSpec;
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
    pub paragraph_lookups: u64,
    pub paragraph_hits: u64,
    pub paragraph_inserts: u64,
    pub paragraph_commands_skipped: u64,
    pub paragraph_mutations_replayed: u64,
    pub paragraph_imported_bytes: u64,
    pub paragraph_validation_misses: u64,
    pub paragraph_import_failures: u64,
    pub paragraph_barriers: u64,
    pub page_lookups: u64,
    pub page_hits: u64,
    pub page_inserts: u64,
    pub page_contributions_skipped: u64,
    pub page_imported_bytes: u64,
    pub page_import_failures: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PureBreakDecision {
    pub position: usize,
    pub penalty: i32,
    pub hyphenated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PureBreakPlan {
    pub breaks: Vec<PureBreakDecision>,
    pub demerits: i32,
    pub last_line_fill: Option<GlueSpec>,
}

#[derive(Clone, Debug)]
pub struct PureParagraphEntry {
    pub hlist: DetachedMemoValue,
    pub mutations: Vec<PureParagraphMutation>,
    pub effects: Vec<crate::DetachedVirtualEffect>,
    pub origin_ordinals: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct PurePageEntry {
    pub transition: DetachedMemoValue,
    pub contributions: usize,
    pub origin_ordinals: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PureParagraphMutation {
    Count {
        index: u16,
        expected: i32,
        value: i32,
        global: bool,
    },
    IntParam {
        param: IntParam,
        expected: i32,
        value: i32,
        global: bool,
    },
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
    value: PureMemoValue,
    charge: usize,
}

#[derive(Clone, Debug)]
enum PureMemoValue {
    Pretolerance(Option<PureBreakPlan>),
    Paragraph(PureParagraphEntry),
    Page(PurePageEntry),
    Detached,
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
    paragraph_front_ends: bool,
    page_episodes: bool,
    paragraph_recording: Option<Vec<PureParagraphMutation>>,
}

impl PureMemoRuntime {
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.cache.is_some()
    }

    #[must_use]
    pub const fn paragraph_front_ends_enabled(&self) -> bool {
        self.cache.is_some() && self.paragraph_front_ends
    }

    #[must_use]
    pub const fn page_episodes_enabled(&self) -> bool {
        self.cache.is_some() && self.page_episodes
    }

    pub fn enable_paragraph_front_ends(&mut self) {
        self.paragraph_front_ends = self.cache.is_some();
    }

    pub fn enable_page_episodes(&mut self) {
        self.page_episodes = self.cache.is_some();
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
        self.paragraph_front_ends = false;
        self.page_episodes = false;
    }

    pub(crate) fn lookup_pretolerance(
        &mut self,
        key: PureMemoKey,
    ) -> Option<Option<PureBreakPlan>> {
        let cache = self.cache.as_mut()?;
        cache.stats.lookups = cache.stats.lookups.saturating_add(1);
        let hit = cache
            .entries
            .get(&key)
            .and_then(|entry| match &entry.value {
                PureMemoValue::Pretolerance(plan) => Some(plan.clone()),
                PureMemoValue::Paragraph(_) | PureMemoValue::Page(_) | PureMemoValue::Detached => {
                    None
                }
            });
        if hit.is_some() {
            cache.stats.hits = cache.stats.hits.saturating_add(1);
        } else if matches!(
            cache.entries.get(&key).map(|entry| &entry.value),
            Some(PureMemoValue::Detached)
        ) {
            cache.stats.malformed = cache.stats.malformed.saturating_add(1);
            cache.remove(key, false);
            cache.stats.misses = cache.stats.misses.saturating_add(1);
        } else {
            cache.stats.misses = cache.stats.misses.saturating_add(1);
        }
        hit
    }

    pub(crate) fn lookup_paragraph(&mut self, key: PureMemoKey) -> Option<PureParagraphEntry> {
        if !self.paragraph_front_ends {
            return None;
        }
        let cache = self.cache.as_mut()?;
        cache.stats.lookups = cache.stats.lookups.saturating_add(1);
        cache.stats.paragraph_lookups = cache.stats.paragraph_lookups.saturating_add(1);
        let hit = cache
            .entries
            .get(&key)
            .and_then(|entry| match &entry.value {
                PureMemoValue::Paragraph(value) => Some(value.clone()),
                PureMemoValue::Pretolerance(_)
                | PureMemoValue::Page(_)
                | PureMemoValue::Detached => None,
            });
        if hit.is_some() {
            cache.stats.hits = cache.stats.hits.saturating_add(1);
            cache.stats.paragraph_hits = cache.stats.paragraph_hits.saturating_add(1);
        } else {
            cache.stats.misses = cache.stats.misses.saturating_add(1);
        }
        hit
    }

    pub(crate) fn lookup_page(&mut self, key: PureMemoKey) -> Option<PurePageEntry> {
        if !self.page_episodes {
            return None;
        }
        let cache = self.cache.as_mut()?;
        cache.stats.lookups = cache.stats.lookups.saturating_add(1);
        cache.stats.page_lookups = cache.stats.page_lookups.saturating_add(1);
        let hit = cache
            .entries
            .get(&key)
            .and_then(|entry| match &entry.value {
                PureMemoValue::Page(value) => Some(value.clone()),
                _ => None,
            });
        if hit.is_some() {
            cache.stats.hits = cache.stats.hits.saturating_add(1);
            cache.stats.page_hits = cache.stats.page_hits.saturating_add(1);
        } else {
            cache.stats.misses = cache.stats.misses.saturating_add(1);
        }
        hit
    }

    pub(crate) fn insert_page(&mut self, key: PureMemoKey, value: PurePageEntry) {
        if !self.page_episodes {
            return;
        }
        let owned_bytes = value
            .transition
            .retained_bytes()
            .saturating_sub(std::mem::size_of::<DetachedMemoValue>())
            .saturating_add(value.origin_ordinals.capacity().saturating_mul(4));
        let before = self.cache.as_ref().map_or(0, |cache| cache.stats.inserts);
        self.insert_value(key, PureMemoValue::Page(value), owned_bytes);
        if let Some(cache) = &mut self.cache
            && cache.stats.inserts != before
        {
            cache.stats.page_inserts = cache.stats.page_inserts.saturating_add(1);
        }
    }

    pub(crate) fn record_page_hit(&mut self, contributions: usize, imported_bytes: usize) {
        if let Some(cache) = &mut self.cache {
            cache.stats.page_contributions_skipped = cache
                .stats
                .page_contributions_skipped
                .saturating_add(contributions as u64);
            cache.stats.page_imported_bytes = cache
                .stats
                .page_imported_bytes
                .saturating_add(imported_bytes as u64);
        }
    }

    pub(crate) fn record_page_import_failure(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.page_import_failures = cache.stats.page_import_failures.saturating_add(1);
        }
    }

    pub(crate) fn insert_paragraph(&mut self, key: PureMemoKey, value: PureParagraphEntry) {
        if !self.paragraph_front_ends {
            return;
        }
        let owned_bytes = value
            .hlist
            .retained_bytes()
            .saturating_sub(std::mem::size_of::<DetachedMemoValue>())
            .saturating_add(
                value
                    .mutations
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PureParagraphMutation>()),
            )
            .saturating_add(
                value
                    .effects
                    .iter()
                    .map(|effect| {
                        effect.operation.capacity()
                            + effect.payload.capacity()
                            + std::mem::size_of::<crate::DetachedVirtualEffect>()
                    })
                    .sum::<usize>(),
            )
            .saturating_add(value.origin_ordinals.capacity().saturating_mul(4));
        let before = self.cache.as_ref().map_or(0, |cache| cache.stats.inserts);
        self.insert_value(key, PureMemoValue::Paragraph(value), owned_bytes);
        if let Some(cache) = &mut self.cache
            && cache.stats.inserts != before
        {
            cache.stats.paragraph_inserts = cache.stats.paragraph_inserts.saturating_add(1);
        }
    }

    pub(crate) fn begin_paragraph_recording(&mut self) {
        if self.paragraph_front_ends_enabled() {
            self.paragraph_recording = Some(Vec::new());
        }
    }

    pub(crate) fn record_paragraph_mutation(&mut self, mutation: PureParagraphMutation) {
        if let Some(recording) = &mut self.paragraph_recording {
            recording.push(mutation);
        }
    }

    pub(crate) fn finish_paragraph_recording(&mut self) -> Option<Vec<PureParagraphMutation>> {
        self.paragraph_recording.take()
    }

    pub(crate) fn record_paragraph_hit(
        &mut self,
        commands: usize,
        mutations: usize,
        imported_bytes: usize,
    ) {
        let Some(cache) = &mut self.cache else {
            return;
        };
        cache.stats.paragraph_commands_skipped = cache
            .stats
            .paragraph_commands_skipped
            .saturating_add(commands as u64);
        cache.stats.paragraph_mutations_replayed = cache
            .stats
            .paragraph_mutations_replayed
            .saturating_add(mutations as u64);
        cache.stats.paragraph_imported_bytes = cache
            .stats
            .paragraph_imported_bytes
            .saturating_add(imported_bytes as u64);
    }

    pub(crate) fn record_paragraph_validation_miss(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.paragraph_validation_misses =
                cache.stats.paragraph_validation_misses.saturating_add(1);
        }
    }

    pub(crate) fn record_paragraph_import_failure(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.paragraph_import_failures =
                cache.stats.paragraph_import_failures.saturating_add(1);
        }
    }

    pub(crate) fn record_paragraph_barrier(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.paragraph_barriers = cache.stats.paragraph_barriers.saturating_add(1);
        }
    }

    pub(crate) fn insert_pretolerance(&mut self, key: PureMemoKey, plan: Option<PureBreakPlan>) {
        let owned_bytes = plan.as_ref().map_or(0, |plan| {
            plan.breaks
                .capacity()
                .saturating_mul(std::mem::size_of::<PureBreakDecision>())
        });
        self.insert_value(key, PureMemoValue::Pretolerance(plan), owned_bytes);
    }

    pub(crate) fn insert_detached(&mut self, key: PureMemoKey, value: DetachedMemoValue) {
        let owned_bytes = value
            .retained_bytes()
            .saturating_sub(std::mem::size_of::<DetachedMemoValue>());
        self.insert_value(key, PureMemoValue::Detached, owned_bytes);
    }

    fn insert_value(&mut self, key: PureMemoKey, value: PureMemoValue, owned_bytes: usize) {
        let Some(cache) = self.cache.as_mut() else {
            return;
        };
        // Charge the map key and FIFO key as well as the entry and owned payload.
        let charge = std::mem::size_of::<Entry>()
            .saturating_add(std::mem::size_of::<PureMemoKey>().saturating_mul(2))
            .saturating_add(owned_bytes);
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
