//! Optional bounded storage for detached pure-query results.
//!
//! The runtime is operational session metadata: it is excluded from snapshots,
//! formats, and semantic hashes. Disabled execution is one `Option` branch and
//! uses no locks or atomics.

use crate::glue::GlueSpec;
use crate::{ContentHash, DetachedMemoValue, RootSpanId};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

struct TelemetryTimer {
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
}

impl TelemetryTimer {
    #[allow(clippy::disallowed_methods)] // Operational telemetry; semantic state never observes it.
    fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            started: Instant::now(),
        }
    }

    fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Duration::ZERO
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PureMemoRecordingPolicy {
    pub pretolerance: bool,
    pub pages: bool,
    pub shipouts: bool,
}

impl PureMemoRecordingPolicy {
    #[must_use]
    pub const fn all() -> Self {
        Self {
            pretolerance: true,
            pages: true,
            shipouts: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PureMemoConfig {
    pub max_entries: usize,
    pub max_retained_bytes: usize,
    pub recording: PureMemoRecordingPolicy,
}

impl Default for PureMemoConfig {
    fn default() -> Self {
        Self {
            max_entries: 1_024,
            max_retained_bytes: 64 * 1024 * 1024,
            recording: PureMemoRecordingPolicy::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoLayerStats {
    pub lookups: u64,
    pub hits: u64,
    pub not_attempted: u64,
    pub ineligible_barriers: u64,
    pub key_misses: u64,
    pub validation_failures: u64,
    pub evicted_before_reuse: u64,
    pub import_failures: u64,
    pub inserts: u64,
    pub evictions: u64,
    pub retained_bytes: usize,
    pub record_nanos: u64,
    pub lookup_nanos: u64,
    pub validation_nanos: u64,
    pub import_nanos: u64,
}

impl MemoLayerStats {
    #[must_use]
    pub fn saturating_since(self, earlier: Self) -> Self {
        Self {
            lookups: self.lookups.saturating_sub(earlier.lookups),
            hits: self.hits.saturating_sub(earlier.hits),
            not_attempted: self.not_attempted.saturating_sub(earlier.not_attempted),
            ineligible_barriers: self
                .ineligible_barriers
                .saturating_sub(earlier.ineligible_barriers),
            key_misses: self.key_misses.saturating_sub(earlier.key_misses),
            validation_failures: self
                .validation_failures
                .saturating_sub(earlier.validation_failures),
            evicted_before_reuse: self
                .evicted_before_reuse
                .saturating_sub(earlier.evicted_before_reuse),
            import_failures: self.import_failures.saturating_sub(earlier.import_failures),
            inserts: self.inserts.saturating_sub(earlier.inserts),
            evictions: self.evictions.saturating_sub(earlier.evictions),
            retained_bytes: self.retained_bytes,
            record_nanos: self.record_nanos.saturating_sub(earlier.record_nanos),
            lookup_nanos: self.lookup_nanos.saturating_sub(earlier.lookup_nanos),
            validation_nanos: self
                .validation_nanos
                .saturating_sub(earlier.validation_nanos),
            import_nanos: self.import_nanos.saturating_sub(earlier.import_nanos),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PureMemoLayer {
    Pretolerance,
    Page,
    Shipout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoTimingPhase {
    Record,
    Lookup,
    Validation,
    Import,
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
    pub pretolerance_retained_bytes: usize,
    pub page_retained_bytes: usize,
    pub shipout_retained_bytes: usize,
    pub pretolerance_evictions: u64,
    pub page_evictions: u64,
    pub shipout_evictions: u64,
    pub page_import_failures: u64,
    pub page_lookups: u64,
    pub page_hits: u64,
    pub page_inserts: u64,
    pub page_contributions_skipped: u64,
    pub page_imported_bytes: u64,
    pub shipout_lookups: u64,
    pub shipout_hits: u64,
    pub shipout_inserts: u64,
    pub shipout_barriers: u64,
    pub shipout_imported_bytes: u64,
    pub output_routine_executions: u64,
    pub pretolerance: MemoLayerStats,
    pub page: MemoLayerStats,
    pub shipout: MemoLayerStats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PureBreakDecision {
    pub position: usize,
    pub penalty: i32,
    pub hyphenated: bool,
}

/// A line-break scratch owner in TeX's variable-size `mem` arena.
///
/// These identities are local to one [`PureBreakPlan`] and deliberately do
/// not participate in semantic identity or format serialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PureBreakMemoryOwner {
    Active(u32),
    Passive(u32),
}

/// One ordered §§126--127 allocation event from the pure line breaker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PureBreakMemoryEvent {
    Allocate {
        owner: PureBreakMemoryOwner,
        words: u8,
    },
    Free(PureBreakMemoryOwner),
}

/// Allocator-only evidence split around §880's `post_line_break` call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PureBreakMemoryPlan {
    pub search: Vec<PureBreakMemoryEvent>,
    pub cleanup: Vec<PureBreakMemoryEvent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PureBreakPlan {
    pub breaks: Vec<PureBreakDecision>,
    pub demerits: i32,
    pub last_line_fill: Option<GlueSpec>,
    pub memory: PureBreakMemoryPlan,
}

/// Stable current-revision recipe for artifact provenance slots.
///
/// `piece_anchors` stores one full stable identity per referenced editor piece;
/// `root_spans` stores compact offsets into those pieces. `origin_slots`
/// indexes `root_spans`; `u32::MAX` denotes provenance which cannot be
/// represented by a stable root.
#[derive(Clone, Debug, Default)]
pub struct OutputProvenanceRecipe {
    pub piece_anchors: Arc<[RootSpanId]>,
    pub root_spans: Arc<[OutputProvenanceSpan]>,
    pub origin_slots: Arc<[u32]>,
}

impl OutputProvenanceRecipe {
    /// Resolves one compact diagnostic slot to stable editor backing without
    /// allocating a live provenance record.
    #[must_use]
    pub fn stable_span(&self, slot: usize) -> Option<RootSpanId> {
        let ordinal = usize::try_from(*self.origin_slots.get(slot)?).ok()?;
        let span = self.root_spans.get(ordinal)?;
        let piece = self.piece_anchors.get(usize::try_from(span.piece).ok()?)?;
        Some(piece.with_offsets(span.start, span.end))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OutputProvenanceSpan {
    pub piece: u32,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug)]
pub struct PurePageEntry {
    pub transition: DetachedMemoValue,
    pub contributions: usize,
    pub origin_ordinals: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct PureShipoutEntry {
    pub artifact: DetachedMemoValue,
    pub render_origin_ends: Vec<u32>,
    pub render_provenance: OutputProvenanceRecipe,
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
    referenced: bool,
    protected_until_reuse: bool,
}

#[derive(Clone, Debug)]
enum PureMemoValue {
    Pretolerance(Option<PureBreakPlan>),
    Page(PurePageEntry),
    Shipout(PureShipoutEntry),
}

#[derive(Clone, Debug)]
struct PureMemoCache {
    config: PureMemoConfig,
    entries: HashMap<PureMemoKey, Entry>,
    clock: VecDeque<PureMemoKey>,
    stats: PureMemoStats,
    /// Bounded telemetry only. Membership never affects a cache hit or result.
    eviction_history: VecDeque<PureMemoKey>,
}

/// Opaque operational cache owned by a long-lived execution session.
///
/// Moving this runtime between a session and a scratch [`crate::Universe`]
/// keeps memo contents out of semantic state while preserving them across
/// accepted editor revisions.
#[derive(Clone, Debug, Default)]
pub struct PureMemoRuntime {
    cache: Option<PureMemoCache>,
    pretolerance: bool,
    page_episodes: bool,
    shipout_episodes: bool,
}

#[allow(clippy::disallowed_methods)] // Operational profiling timers never become TeX facts.
impl PureMemoRuntime {
    #[must_use]
    pub fn new(config: PureMemoConfig) -> Self {
        let mut runtime = Self::default();
        runtime.enable(config);
        runtime
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.cache.is_some()
    }

    #[must_use]
    pub const fn pretolerance_enabled(&self) -> bool {
        self.cache.is_some() && self.pretolerance
    }

    #[must_use]
    pub const fn page_episodes_enabled(&self) -> bool {
        self.cache.is_some() && self.page_episodes
    }

    #[must_use]
    pub const fn shipout_episodes_enabled(&self) -> bool {
        self.cache.is_some() && self.shipout_episodes
    }

    pub fn enable_page_episodes(&mut self) {
        self.page_episodes = self.cache.is_some();
    }

    pub fn enable_shipout_episodes(&mut self) {
        self.shipout_episodes = self.cache.is_some();
    }

    pub(crate) fn enable(&mut self, config: PureMemoConfig) {
        self.pretolerance = config.recording.pretolerance;
        self.page_episodes = config.recording.pages;
        self.shipout_episodes = config.recording.shipouts;
        self.cache = Some(PureMemoCache {
            config,
            entries: HashMap::new(),
            clock: VecDeque::new(),
            stats: PureMemoStats::default(),
            eviction_history: VecDeque::new(),
        });
    }

    pub fn lookup_pretolerance(&mut self, key: PureMemoKey) -> Option<Option<PureBreakPlan>> {
        if !self.pretolerance {
            self.record_not_attempted(PureMemoLayer::Pretolerance);
            return None;
        }
        let started = TelemetryTimer::start();
        let cache = self.cache.as_mut()?;
        cache.stats.lookups = cache.stats.lookups.saturating_add(1);
        let hit = cache
            .entries
            .get_mut(&key)
            .and_then(|entry| match &entry.value {
                PureMemoValue::Pretolerance(plan) => {
                    entry.referenced = true;
                    entry.protected_until_reuse = false;
                    Some(plan.clone())
                }
                PureMemoValue::Page(_) | PureMemoValue::Shipout(_) => None,
            });
        if hit.is_some() {
            cache.stats.hits = cache.stats.hits.saturating_add(1);
        } else {
            cache.stats.misses = cache.stats.misses.saturating_add(1);
            if cache.take_evicted(key) {
                cache.stats.pretolerance.evicted_before_reuse = cache
                    .stats
                    .pretolerance
                    .evicted_before_reuse
                    .saturating_add(1);
            } else {
                cache.stats.pretolerance.key_misses =
                    cache.stats.pretolerance.key_misses.saturating_add(1);
            }
        }
        cache.stats.pretolerance.lookups = cache.stats.pretolerance.lookups.saturating_add(1);
        cache.stats.pretolerance.hits = cache
            .stats
            .pretolerance
            .hits
            .saturating_add(u64::from(hit.is_some()));
        cache.stats.pretolerance.lookup_nanos = cache
            .stats
            .pretolerance
            .lookup_nanos
            .saturating_add(elapsed_nanos(started.elapsed()));
        hit
    }

    pub fn lookup_page(&mut self, key: PureMemoKey) -> Option<PurePageEntry> {
        if !self.page_episodes {
            self.record_not_attempted(PureMemoLayer::Page);
            return None;
        }
        let started = TelemetryTimer::start();
        let cache = self.cache.as_mut()?;
        cache.stats.lookups = cache.stats.lookups.saturating_add(1);
        cache.stats.page_lookups = cache.stats.page_lookups.saturating_add(1);
        let hit = cache
            .entries
            .get_mut(&key)
            .and_then(|entry| match &entry.value {
                PureMemoValue::Page(value) => {
                    entry.referenced = true;
                    entry.protected_until_reuse = false;
                    Some(value.clone())
                }
                _ => None,
            });
        if hit.is_some() {
            cache.stats.hits = cache.stats.hits.saturating_add(1);
            cache.stats.page_hits = cache.stats.page_hits.saturating_add(1);
        } else {
            cache.stats.misses = cache.stats.misses.saturating_add(1);
            if cache.take_evicted(key) {
                cache.stats.page.evicted_before_reuse =
                    cache.stats.page.evicted_before_reuse.saturating_add(1);
            } else {
                cache.stats.page.key_misses = cache.stats.page.key_misses.saturating_add(1);
            }
        }
        cache.stats.page.lookups = cache.stats.page.lookups.saturating_add(1);
        cache.stats.page.hits = cache
            .stats
            .page
            .hits
            .saturating_add(u64::from(hit.is_some()));
        cache.stats.page.lookup_nanos = cache
            .stats
            .page
            .lookup_nanos
            .saturating_add(elapsed_nanos(started.elapsed()));
        hit
    }

    pub fn insert_page(&mut self, key: PureMemoKey, value: PurePageEntry) {
        if !self.page_episodes {
            return;
        }
        let owned_bytes = value
            .transition
            .retained_bytes()
            .saturating_sub(std::mem::size_of::<DetachedMemoValue>())
            .saturating_add(value.origin_ordinals.capacity().saturating_mul(4));
        let started = TelemetryTimer::start();
        let before = self.cache.as_ref().map_or(0, |cache| cache.stats.inserts);
        self.insert_value(key, PureMemoValue::Page(value), owned_bytes);
        if let Some(cache) = &mut self.cache
            && cache.stats.inserts != before
        {
            cache.stats.page_inserts = cache.stats.page_inserts.saturating_add(1);
        }
        self.record_timing(
            PureMemoLayer::Page,
            MemoTimingPhase::Record,
            started.elapsed(),
        );
    }

    pub fn record_page_hit(&mut self, contributions: usize, imported_bytes: usize) {
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

    pub fn record_page_import_failure(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.page_import_failures = cache.stats.page_import_failures.saturating_add(1);
            cache.stats.page.import_failures = cache.stats.page.import_failures.saturating_add(1);
        }
    }

    pub fn lookup_shipout(&mut self, key: PureMemoKey) -> Option<PureShipoutEntry> {
        if !self.shipout_episodes {
            self.record_not_attempted(PureMemoLayer::Shipout);
            return None;
        }
        let started = TelemetryTimer::start();
        let cache = self.cache.as_mut()?;
        cache.stats.lookups = cache.stats.lookups.saturating_add(1);
        cache.stats.shipout_lookups = cache.stats.shipout_lookups.saturating_add(1);
        let hit = cache
            .entries
            .get_mut(&key)
            .and_then(|entry| match &entry.value {
                PureMemoValue::Shipout(value) => {
                    entry.referenced = true;
                    entry.protected_until_reuse = false;
                    Some(value.clone())
                }
                _ => None,
            });
        if hit.is_some() {
            cache.stats.hits = cache.stats.hits.saturating_add(1);
            cache.stats.shipout_hits = cache.stats.shipout_hits.saturating_add(1);
        } else {
            cache.stats.misses = cache.stats.misses.saturating_add(1);
            if cache.take_evicted(key) {
                cache.stats.shipout.evicted_before_reuse =
                    cache.stats.shipout.evicted_before_reuse.saturating_add(1);
            } else {
                cache.stats.shipout.key_misses = cache.stats.shipout.key_misses.saturating_add(1);
            }
        }
        cache.stats.shipout.lookups = cache.stats.shipout.lookups.saturating_add(1);
        cache.stats.shipout.hits = cache
            .stats
            .shipout
            .hits
            .saturating_add(u64::from(hit.is_some()));
        cache.stats.shipout.lookup_nanos = cache
            .stats
            .shipout
            .lookup_nanos
            .saturating_add(elapsed_nanos(started.elapsed()));
        hit
    }

    pub fn insert_shipout(&mut self, key: PureMemoKey, value: PureShipoutEntry) {
        if !self.shipout_episodes {
            return;
        }
        let owned_bytes = value
            .artifact
            .retained_bytes()
            .saturating_sub(std::mem::size_of::<DetachedMemoValue>())
            .saturating_add(value.render_origin_ends.capacity().saturating_mul(4))
            .saturating_add(output_provenance_retained_bytes(&value.render_provenance));
        let started = TelemetryTimer::start();
        let before = self.cache.as_ref().map_or(0, |cache| cache.stats.inserts);
        self.insert_value(key, PureMemoValue::Shipout(value), owned_bytes);
        if let Some(cache) = &mut self.cache
            && cache.stats.inserts != before
        {
            cache.stats.shipout_inserts = cache.stats.shipout_inserts.saturating_add(1);
        }
        self.record_timing(
            PureMemoLayer::Shipout,
            MemoTimingPhase::Record,
            started.elapsed(),
        );
    }

    pub fn record_shipout_hit(&mut self, imported_bytes: usize) {
        if let Some(cache) = &mut self.cache {
            cache.stats.shipout_imported_bytes = cache
                .stats
                .shipout_imported_bytes
                .saturating_add(imported_bytes as u64);
        }
    }

    pub fn record_shipout_barrier(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.shipout_barriers = cache.stats.shipout_barriers.saturating_add(1);
            cache.stats.shipout.ineligible_barriers =
                cache.stats.shipout.ineligible_barriers.saturating_add(1);
        }
    }

    pub fn record_output_routine_execution(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.output_routine_executions =
                cache.stats.output_routine_executions.saturating_add(1);
        }
    }

    pub fn insert_pretolerance(&mut self, key: PureMemoKey, plan: Option<PureBreakPlan>) {
        if !self.pretolerance {
            self.record_not_attempted(PureMemoLayer::Pretolerance);
            return;
        }
        let started = TelemetryTimer::start();
        let owned_bytes = plan.as_ref().map_or(0, |plan| {
            plan.breaks
                .capacity()
                .saturating_mul(std::mem::size_of::<PureBreakDecision>())
                .saturating_add(
                    plan.memory
                        .search
                        .capacity()
                        .saturating_mul(std::mem::size_of::<PureBreakMemoryEvent>()),
                )
                .saturating_add(
                    plan.memory
                        .cleanup
                        .capacity()
                        .saturating_mul(std::mem::size_of::<PureBreakMemoryEvent>()),
                )
        });
        self.insert_value(key, PureMemoValue::Pretolerance(plan), owned_bytes);
        self.record_timing(
            PureMemoLayer::Pretolerance,
            MemoTimingPhase::Record,
            started.elapsed(),
        );
    }

    fn insert_value(&mut self, key: PureMemoKey, value: PureMemoValue, owned_bytes: usize) {
        let Some(cache) = self.cache.as_mut() else {
            return;
        };
        // Charge the map key and CLOCK key as well as the entry and owned payload.
        let charge = std::mem::size_of::<Entry>()
            .saturating_add(std::mem::size_of::<PureMemoKey>().saturating_mul(2))
            .saturating_add(owned_bytes);
        if cache.config.max_entries == 0 || charge > cache.config.max_retained_bytes {
            return;
        }
        if !cache.entries.contains_key(&key) && !cache.prepare_admission(charge) {
            let layer = value.kind().layer();
            let stats = cache.stats.layer_mut(layer);
            stats.not_attempted = stats.not_attempted.saturating_add(1);
            return;
        }
        if let Some(entry) = cache.entries.get_mut(&key) {
            let old_kind = entry.value.kind();
            cache
                .stats
                .remove_kind_charge(old_kind, entry.charge, false);
            cache.stats.retained_bytes = cache
                .stats
                .retained_bytes
                .saturating_sub(entry.charge)
                .saturating_add(charge);
            entry.value = value;
            entry.charge = charge;
            entry.referenced = true;
            entry.protected_until_reuse = true;
            cache.stats.add_kind_charge(entry.value.kind(), charge);
        } else {
            let kind = value.kind();
            cache.entries.insert(
                key,
                Entry {
                    value,
                    charge,
                    referenced: false,
                    protected_until_reuse: true,
                },
            );
            cache.clock.push_back(key);
            cache.stats.inserts = cache.stats.inserts.saturating_add(1);
            cache.stats.retained_entries = cache.stats.retained_entries.saturating_add(1);
            cache.stats.retained_bytes = cache.stats.retained_bytes.saturating_add(charge);
            cache.stats.add_kind_charge(kind, charge);
            cache.stats.layer_mut(kind.layer()).inserts = cache
                .stats
                .layer_mut(kind.layer())
                .inserts
                .saturating_add(1);
        }
    }

    pub fn reject(&mut self, key: PureMemoKey) {
        let Some(cache) = self.cache.as_mut() else {
            return;
        };
        cache.stats.malformed = cache.stats.malformed.saturating_add(1);
        cache.remove(key, false);
    }

    /// Drops every rebuildable memo result without changing engine state.
    ///
    /// Configuration and cumulative operational counters remain installed so
    /// later queries may repopulate the cache under the same limits.
    pub fn evict_all(&mut self) {
        let Some(cache) = self.cache.as_mut() else {
            return;
        };
        let entries = std::mem::take(&mut cache.entries);
        cache.clock.clear();
        cache.eviction_history.clear();
        for entry in entries.into_values() {
            cache.stats.retained_entries = cache.stats.retained_entries.saturating_sub(1);
            cache.stats.retained_bytes = cache.stats.retained_bytes.saturating_sub(entry.charge);
            cache
                .stats
                .remove_kind_charge(entry.value.kind(), entry.charge, true);
            cache.stats.evictions = cache.stats.evictions.saturating_add(1);
        }
    }

    #[must_use]
    pub fn stats(&self) -> PureMemoStats {
        let mut stats = self
            .cache
            .as_ref()
            .map_or_else(PureMemoStats::default, |cache| cache.stats);
        stats.pretolerance.retained_bytes = stats.pretolerance_retained_bytes;
        stats.page.retained_bytes = stats.page_retained_bytes;
        stats.shipout.retained_bytes = stats.shipout_retained_bytes;
        stats
    }

    pub fn record_not_attempted(&mut self, layer: PureMemoLayer) {
        if let Some(cache) = &mut self.cache {
            let stats = cache.stats.layer_mut(layer);
            stats.not_attempted = stats.not_attempted.saturating_add(1);
        }
    }

    pub fn record_timing(
        &mut self,
        layer: PureMemoLayer,
        phase: MemoTimingPhase,
        elapsed: Duration,
    ) {
        let Some(cache) = &mut self.cache else {
            return;
        };
        let elapsed = elapsed_nanos(elapsed);
        let stats = cache.stats.layer_mut(layer);
        let target = match phase {
            MemoTimingPhase::Record => &mut stats.record_nanos,
            MemoTimingPhase::Lookup => &mut stats.lookup_nanos,
            MemoTimingPhase::Validation => &mut stats.validation_nanos,
            MemoTimingPhase::Import => &mut stats.import_nanos,
        };
        *target = target.saturating_add(elapsed);
    }
}

impl PureMemoCache {
    fn take_evicted(&mut self, key: PureMemoKey) -> bool {
        let Some(index) = self
            .eviction_history
            .iter()
            .position(|candidate| *candidate == key)
        else {
            return false;
        };
        self.eviction_history.remove(index);
        true
    }

    fn remember_eviction(&mut self, key: PureMemoKey) {
        let key_bytes = std::mem::size_of::<PureMemoKey>();
        let byte_limit = self.config.max_retained_bytes / key_bytes.max(1);
        let limit = self.config.max_entries.min(byte_limit);
        if limit == 0 {
            return;
        }
        if self.eviction_history.len() == limit {
            self.eviction_history.pop_front();
        }
        self.eviction_history.push_back(key);
    }

    fn prepare_admission(&mut self, charge: usize) -> bool {
        while self.stats.retained_entries.saturating_add(1) > self.config.max_entries
            || self.stats.retained_bytes.saturating_add(charge) > self.config.max_retained_bytes
        {
            let Some(key) = self.clock.pop_front() else {
                return false;
            };
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if entry.protected_until_reuse || entry.referenced {
                entry.referenced = false;
                self.clock.push_back(key);
                if self
                    .clock
                    .iter()
                    .all(|candidate| self.entries[candidate].protected_until_reuse)
                {
                    return false;
                }
                continue;
            }
            self.remove(key, true);
        }
        true
    }

    fn remove(&mut self, key: PureMemoKey, eviction: bool) {
        let Some(entry) = self.entries.remove(&key) else {
            return;
        };
        self.stats.retained_entries = self.stats.retained_entries.saturating_sub(1);
        self.stats.retained_bytes = self.stats.retained_bytes.saturating_sub(entry.charge);
        self.stats
            .remove_kind_charge(entry.value.kind(), entry.charge, eviction);
        if eviction {
            self.stats.evictions = self.stats.evictions.saturating_add(1);
            self.remember_eviction(key);
        } else {
            self.clock.retain(|candidate| *candidate != key);
        }
    }
}

#[derive(Clone, Copy)]
enum PureMemoKind {
    Pretolerance,
    Page,
    Shipout,
}

impl PureMemoValue {
    fn kind(&self) -> PureMemoKind {
        match self {
            Self::Pretolerance(_) => PureMemoKind::Pretolerance,
            Self::Page(_) => PureMemoKind::Page,
            Self::Shipout(_) => PureMemoKind::Shipout,
        }
    }
}

impl PureMemoKind {
    const fn layer(self) -> PureMemoLayer {
        match self {
            Self::Pretolerance => PureMemoLayer::Pretolerance,
            Self::Page => PureMemoLayer::Page,
            Self::Shipout => PureMemoLayer::Shipout,
        }
    }
}

impl PureMemoStats {
    fn add_kind_charge(&mut self, kind: PureMemoKind, charge: usize) {
        let retained = match kind {
            PureMemoKind::Pretolerance => &mut self.pretolerance_retained_bytes,
            PureMemoKind::Page => &mut self.page_retained_bytes,
            PureMemoKind::Shipout => &mut self.shipout_retained_bytes,
        };
        *retained = retained.saturating_add(charge);
    }

    fn remove_kind_charge(&mut self, kind: PureMemoKind, charge: usize, eviction: bool) {
        let (retained, evictions) = match kind {
            PureMemoKind::Pretolerance => (
                &mut self.pretolerance_retained_bytes,
                &mut self.pretolerance_evictions,
            ),
            PureMemoKind::Page => (&mut self.page_retained_bytes, &mut self.page_evictions),
            PureMemoKind::Shipout => (
                &mut self.shipout_retained_bytes,
                &mut self.shipout_evictions,
            ),
        };
        *retained = retained.saturating_sub(charge);
        if eviction {
            *evictions = evictions.saturating_add(1);
            self.layer_mut(kind.layer()).evictions =
                self.layer_mut(kind.layer()).evictions.saturating_add(1);
        }
    }

    #[must_use]
    pub const fn layer(&self, layer: PureMemoLayer) -> MemoLayerStats {
        match layer {
            PureMemoLayer::Pretolerance => self.pretolerance,
            PureMemoLayer::Page => self.page,
            PureMemoLayer::Shipout => self.shipout,
        }
    }

    fn layer_mut(&mut self, layer: PureMemoLayer) -> &mut MemoLayerStats {
        match layer {
            PureMemoLayer::Pretolerance => &mut self.pretolerance,
            PureMemoLayer::Page => &mut self.page,
            PureMemoLayer::Shipout => &mut self.shipout,
        }
    }
}

fn elapsed_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn output_provenance_retained_bytes(recipe: &OutputProvenanceRecipe) -> usize {
    recipe
        .piece_anchors
        .len()
        .saturating_mul(std::mem::size_of::<RootSpanId>())
        .saturating_add(
            recipe
                .root_spans
                .len()
                .saturating_mul(std::mem::size_of::<OutputProvenanceSpan>()),
        )
        .saturating_add(
            recipe
                .origin_slots
                .len()
                .saturating_mul(std::mem::size_of::<u32>()),
        )
}

#[cfg(test)]
mod tests;
