//! Optional bounded storage for detached pure-query results.
//!
//! The runtime is operational session metadata: it is excluded from snapshots,
//! formats, and semantic hashes. Disabled execution is one `Option` branch and
//! uses no locks or atomics.

use crate::dependency::DependencyKey;
use crate::env::banks::IntParam;
use crate::glue::GlueSpec;
use crate::survivor::RetainedNodeList;
use crate::{ContentHash, DetachedMemoValue, InputSummary, ObservedDependency, RootSpanId};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PureMemoRecordingPolicy {
    pub pretolerance: bool,
    pub paragraphs: bool,
    pub pages: bool,
    pub shipouts: bool,
}

impl PureMemoRecordingPolicy {
    #[must_use]
    pub const fn all() -> Self {
        Self {
            pretolerance: true,
            paragraphs: true,
            pages: true,
            shipouts: true,
        }
    }
}

impl Default for PureMemoRecordingPolicy {
    fn default() -> Self {
        Self {
            pretolerance: false,
            paragraphs: true,
            pages: false,
            shipouts: false,
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
    Paragraph,
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

/// Profiling-only attribution for paragraph recording work that sits outside
/// the cache layer's generic lookup/record/validation/import buckets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ParagraphRecordingPhase {
    FrontEndDependencies,
    InputTransition,
    RegionPublication,
    BreakDependencies,
    BreakKeyDiscovery,
    BreakStampRegistration,
    BreakValueProjection,
    LineProvenance,
    LineRetention,
}

/// Work and retained storage attributed to accepted paragraph history.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParagraphOpportunityMetric {
    pub regions: u64,
    pub bytes: u64,
    pub nanos: u64,
}

impl ParagraphOpportunityMetric {
    #[must_use]
    pub fn saturating_since(self, earlier: Self) -> Self {
        Self {
            regions: self.regions.saturating_sub(earlier.regions),
            bytes: self.bytes.saturating_sub(earlier.bytes),
            nanos: self.nanos.saturating_sub(earlier.nanos),
        }
    }
}

/// Accepted paragraph history publication and carry-forward telemetry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParagraphOpportunityStats {
    pub carried_forward: ParagraphOpportunityMetric,
    pub published: ParagraphOpportunityMetric,
}

impl ParagraphOpportunityStats {
    #[must_use]
    pub fn saturating_since(self, earlier: Self) -> Self {
        Self {
            carried_forward: self
                .carried_forward
                .saturating_since(earlier.carried_forward),
            published: self.published.saturating_since(earlier.published),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParagraphRecordingStats {
    /// Number of `Instant::now`/`elapsed` pairs used by these named phases.
    pub timer_samples: u64,
    pub front_end_dependency_nanos: u64,
    pub input_transition_nanos: u64,
    pub region_publication_nanos: u64,
    pub break_dependency_nanos: u64,
    pub break_key_discovery_nanos: u64,
    pub break_stamp_registration_nanos: u64,
    pub break_value_projection_nanos: u64,
    pub line_provenance_nanos: u64,
    pub line_retention_nanos: u64,
}

impl ParagraphRecordingStats {
    #[must_use]
    pub fn saturating_since(self, earlier: Self) -> Self {
        Self {
            timer_samples: self.timer_samples.saturating_sub(earlier.timer_samples),
            front_end_dependency_nanos: self
                .front_end_dependency_nanos
                .saturating_sub(earlier.front_end_dependency_nanos),
            input_transition_nanos: self
                .input_transition_nanos
                .saturating_sub(earlier.input_transition_nanos),
            region_publication_nanos: self
                .region_publication_nanos
                .saturating_sub(earlier.region_publication_nanos),
            break_dependency_nanos: self
                .break_dependency_nanos
                .saturating_sub(earlier.break_dependency_nanos),
            break_key_discovery_nanos: self
                .break_key_discovery_nanos
                .saturating_sub(earlier.break_key_discovery_nanos),
            break_stamp_registration_nanos: self
                .break_stamp_registration_nanos
                .saturating_sub(earlier.break_stamp_registration_nanos),
            break_value_projection_nanos: self
                .break_value_projection_nanos
                .saturating_sub(earlier.break_value_projection_nanos),
            line_provenance_nanos: self
                .line_provenance_nanos
                .saturating_sub(earlier.line_provenance_nanos),
            line_retention_nanos: self
                .line_retention_nanos
                .saturating_sub(earlier.line_retention_nanos),
        }
    }

    fn add(&mut self, phase: ParagraphRecordingPhase, elapsed: Duration, samples: u64) {
        let target = match phase {
            ParagraphRecordingPhase::FrontEndDependencies => &mut self.front_end_dependency_nanos,
            ParagraphRecordingPhase::InputTransition => &mut self.input_transition_nanos,
            ParagraphRecordingPhase::RegionPublication => &mut self.region_publication_nanos,
            ParagraphRecordingPhase::BreakDependencies => &mut self.break_dependency_nanos,
            ParagraphRecordingPhase::BreakKeyDiscovery => &mut self.break_key_discovery_nanos,
            ParagraphRecordingPhase::BreakStampRegistration => {
                &mut self.break_stamp_registration_nanos
            }
            ParagraphRecordingPhase::BreakValueProjection => &mut self.break_value_projection_nanos,
            ParagraphRecordingPhase::LineProvenance => &mut self.line_provenance_nanos,
            ParagraphRecordingPhase::LineRetention => &mut self.line_retention_nanos,
        };
        *target = target.saturating_add(elapsed_nanos(elapsed));
        self.timer_samples = self.timer_samples.saturating_add(samples);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ParagraphValidationFailure {
    Meaning,
    Cell,
    Code,
    Font,
    Hyphenation,
    Input,
    Engine,
    Page,
    World,
    Query,
    Mutation,
    Effect,
    InputTransition,
    RetainedResult,
    BreakDependency,
}

impl ParagraphValidationFailure {
    #[must_use]
    pub const fn from_dependency(key: DependencyKey) -> Self {
        match key {
            DependencyKey::Meaning(_) => Self::Meaning,
            DependencyKey::Cell { .. } => Self::Cell,
            DependencyKey::Code { .. } | DependencyKey::CodeGeneration(_) => Self::Code,
            DependencyKey::Font { .. } => Self::Font,
            DependencyKey::HyphenationPatterns(_)
            | DependencyKey::HyphenationExceptions(_)
            | DependencyKey::HyphenationCodes(_) => Self::Hyphenation,
            DependencyKey::InputRecord(_)
            | DependencyKey::PhysicalLine { .. }
            | DependencyKey::InputLine
            | DependencyKey::InputStream(_)
            | DependencyKey::InputStack => Self::Input,
            DependencyKey::Engine(_) => Self::Engine,
            DependencyKey::PageDimension(_)
            | DependencyKey::PageInteger(_)
            | DependencyKey::PageMark(_)
            | DependencyKey::PageMarkClass { .. }
            | DependencyKey::Page(_) => Self::Page,
            DependencyKey::World { .. } => Self::World,
            DependencyKey::Query { .. } => Self::Query,
        }
    }
}

const PARAGRAPH_VALIDATION_FAILURES: usize = 15;

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
    pub paragraph_retained_bytes: usize,
    pub page_retained_bytes: usize,
    pub shipout_retained_bytes: usize,
    pub pretolerance_evictions: u64,
    pub paragraph_evictions: u64,
    pub page_evictions: u64,
    pub shipout_evictions: u64,
    pub paragraph_lookups: u64,
    pub paragraph_hits: u64,
    pub paragraph_inserts: u64,
    pub paragraph_commands_skipped: u64,
    pub paragraph_mutations_replayed: u64,
    /// Hits whose incoming count/int fingerprint differed, admitted because
    /// unchanged source and read dependencies establish the same write script.
    pub paragraph_line_hits: u64,
    pub paragraph_validation_misses: u64,
    pub paragraph_barriers: u64,
    pub paragraph_eligible_regions: u64,
    pub paragraph_display_math_barriers: u64,
    pub paragraph_scantokens_barriers: u64,
    pub paragraph_input_open_barriers: u64,
    pub paragraph_untracked_world_barriers: u64,
    pub paragraph_output_routine_barriers: u64,
    pub paragraph_endinput_barriers: u64,
    pub paragraph_unsupported_write_barriers: u64,
    pub paragraph_unsupported_input_transition_barriers: u64,
    pub paragraph_unsupported_group_transition_barriers: u64,
    pub page_lookups: u64,
    pub page_hits: u64,
    pub page_inserts: u64,
    pub page_contributions_skipped: u64,
    pub page_imported_bytes: u64,
    pub page_import_failures: u64,
    pub shipout_lookups: u64,
    pub shipout_hits: u64,
    pub shipout_inserts: u64,
    pub shipout_barriers: u64,
    pub shipout_imported_bytes: u64,
    pub output_routine_executions: u64,
    pub pretolerance: MemoLayerStats,
    pub paragraph: MemoLayerStats,
    pub page: MemoLayerStats,
    pub shipout: MemoLayerStats,
    pub paragraph_history_metadata_bytes: usize,
    pub paragraph_validation_failure_reasons: [u64; PARAGRAPH_VALIDATION_FAILURES],
    pub paragraph_recording: ParagraphRecordingStats,
    pub paragraph_opportunities: ParagraphOpportunityStats,
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

/// Why a cold paragraph trace cannot be replayed. These reasons are stable
/// telemetry rather than inferred failures at a later cache lookup.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ParagraphBarrierReason {
    DisplayMath,
    Scantokens,
    MidParagraphInputOpen,
    EndInput,
    UntrackedWorldAccess,
    NestedOutputRoutine,
    UnsupportedEscapingWrite,
    UnsupportedInputTransition,
    UnsupportedGroupTransition,
}

/// Stable current-revision rebinding recipe for the provenance slots reachable
/// from one retained paragraph graph.
///
/// `piece_anchors` stores one full stable identity per referenced editor piece;
/// `root_spans` stores compact offsets into those pieces. `origin_slots`
/// follows ordinary depth-first node traversal and indexes `root_spans`;
/// `u32::MAX` denotes provenance which cannot be represented by a stable root.
#[derive(Clone, Debug, Default)]
pub struct ParagraphProvenanceRecipe {
    pub piece_anchors: Arc<[RootSpanId]>,
    pub root_spans: Arc<[ParagraphProvenanceSpan]>,
    pub origin_slots: Arc<[u32]>,
}

#[derive(Clone, Copy, Debug)]
pub struct ParagraphProvenanceSpan {
    pub piece: u32,
    pub start: u32,
    pub end: u32,
}

/// Recorder output for one normally executed paragraph. Accepted history owns
/// shared survivor mounts with stable output-provenance recipes.
#[derive(Clone, Debug)]
pub struct RecordedParagraphRegion {
    /// Cheap candidate identity captured before the first raw delivery.
    pub starting_span: Option<RootSpanId>,
    /// Stable raw cursor reached after the paragraph terminator.
    pub ending_span: Option<RootSpanId>,
    pub consumed_spans: Arc<[RootSpanId]>,
    /// Delivered-token count retained only for avoided-work telemetry. No
    /// token values or origins are recorded.
    pub delivered_tokens: usize,
    pub dependencies: Arc<[ObservedDependency]>,
    /// Root and live-group mutation scripts use different compaction rules.
    pub mutation_entry_in_group: bool,
    pub mutations: Arc<[PureParagraphMutation]>,
    pub effects: Arc<[crate::DetachedVirtualEffect]>,
    pub ending_input: InputSummary,
    pub barriers: Arc<[ParagraphBarrierReason]>,
    /// Dependencies observed by horizontal-list construction, line breaking,
    /// materialization, and packing. A mismatch invalidates finished lines and
    /// sends the revision down the ordinary cold path.
    pub break_dependencies: Arc<[ObservedDependency]>,
    /// Enclosing vertical-list line offset consumed by `line_shape`, when a
    /// non-natural paragraph shape can observe it.
    pub break_prev_graf: Option<i32>,
    /// Finished line boxes interleaved with migrating material and penalties.
    pub lines: Option<RetainedNodeList>,
    pub line_count: i32,
    /// `\badness` left by packing the final materialized line.
    pub line_last_badness: i32,
    pub line_provenance: ParagraphProvenanceRecipe,
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
    pub render_origin_ordinals: Vec<Vec<u32>>,
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

/// Cold paragraph root-transition accounting observed directly by setters.
#[derive(Clone, Debug)]
pub struct PureParagraphMutationSummary {
    pub entry_in_group: bool,
    pub unsupported_group_ownership: bool,
    pub mutations: Vec<PureParagraphMutation>,
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
    Detached,
}

#[derive(Clone, Debug)]
struct PureMemoCache {
    config: PureMemoConfig,
    entries: HashMap<PureMemoKey, Entry>,
    clock: VecDeque<PureMemoKey>,
    stats: PureMemoStats,
    evicted_keys: HashSet<PureMemoKey>,
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
    pretolerance: bool,
    page_episodes: bool,
    shipout_episodes: bool,
    paragraph_recording: Option<crate::env::paragraph::ParagraphMutationCheckpoint>,
    prior_paragraphs: Vec<RecordedParagraphRegion>,
    /// Stable-source alignment index for the ordered accepted paragraph trace.
    prior_paragraph_starts: HashMap<RootSpanId, usize>,
    /// First accepted paragraph that may still align with the new execution.
    prior_paragraph_cursor: usize,
    recorded_paragraphs: Vec<RecordedParagraphRegion>,
    reuse_prior_paragraphs: bool,
    preserve_prior_paragraphs: bool,
    paragraph_barrier_reasons: BTreeMap<ParagraphBarrierReason, u64>,
}

#[allow(clippy::disallowed_methods)] // Operational profiling timers never become TeX facts.
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

    pub fn enable_paragraph_front_ends(&mut self) {
        self.paragraph_front_ends = self.cache.is_some();
    }

    pub fn enable_page_episodes(&mut self) {
        self.page_episodes = self.cache.is_some();
    }

    pub fn enable_shipout_episodes(&mut self) {
        self.shipout_episodes = self.cache.is_some();
    }

    pub(crate) fn enable(&mut self, config: PureMemoConfig) {
        self.pretolerance = config.recording.pretolerance;
        self.paragraph_front_ends = config.recording.paragraphs;
        self.page_episodes = config.recording.pages;
        self.shipout_episodes = config.recording.shipouts;
        self.cache = Some(PureMemoCache {
            config,
            entries: HashMap::new(),
            clock: VecDeque::new(),
            stats: PureMemoStats::default(),
            evicted_keys: HashSet::new(),
        });
    }

    pub(crate) fn disable(&mut self) {
        self.cache = None;
        self.pretolerance = false;
        self.paragraph_front_ends = false;
        self.page_episodes = false;
        self.shipout_episodes = false;
    }

    pub(crate) fn lookup_pretolerance(
        &mut self,
        key: PureMemoKey,
    ) -> Option<Option<PureBreakPlan>> {
        if !self.pretolerance {
            self.record_not_attempted(PureMemoLayer::Pretolerance);
            return None;
        }
        let started = std::time::Instant::now();
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
                PureMemoValue::Page(_) | PureMemoValue::Shipout(_) | PureMemoValue::Detached => {
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
            if cache.evicted_keys.remove(&key) {
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

    pub(crate) fn lookup_page(&mut self, key: PureMemoKey) -> Option<PurePageEntry> {
        if !self.page_episodes {
            self.record_not_attempted(PureMemoLayer::Page);
            return None;
        }
        let started = std::time::Instant::now();
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
            if cache.evicted_keys.remove(&key) {
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

    pub(crate) fn insert_page(&mut self, key: PureMemoKey, value: PurePageEntry) {
        if !self.page_episodes {
            return;
        }
        let owned_bytes = value
            .transition
            .retained_bytes()
            .saturating_sub(std::mem::size_of::<DetachedMemoValue>())
            .saturating_add(value.origin_ordinals.capacity().saturating_mul(4));
        let started = std::time::Instant::now();
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
            cache.stats.page.import_failures = cache.stats.page.import_failures.saturating_add(1);
        }
    }

    pub(crate) fn lookup_shipout(&mut self, key: PureMemoKey) -> Option<PureShipoutEntry> {
        if !self.shipout_episodes {
            self.record_not_attempted(PureMemoLayer::Shipout);
            return None;
        }
        let started = std::time::Instant::now();
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
            if cache.evicted_keys.remove(&key) {
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

    pub(crate) fn insert_shipout(&mut self, key: PureMemoKey, value: PureShipoutEntry) {
        if !self.shipout_episodes {
            return;
        }
        let owned_bytes = value
            .artifact
            .retained_bytes()
            .saturating_sub(std::mem::size_of::<DetachedMemoValue>())
            .saturating_add(
                value
                    .render_origin_ordinals
                    .iter()
                    .map(|origins| origins.capacity().saturating_mul(4))
                    .sum::<usize>(),
            );
        let started = std::time::Instant::now();
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

    pub(crate) fn record_shipout_hit(&mut self, imported_bytes: usize) {
        if let Some(cache) = &mut self.cache {
            cache.stats.shipout_imported_bytes = cache
                .stats
                .shipout_imported_bytes
                .saturating_add(imported_bytes as u64);
        }
    }

    pub(crate) fn record_shipout_barrier(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.shipout_barriers = cache.stats.shipout_barriers.saturating_add(1);
            cache.stats.shipout.ineligible_barriers =
                cache.stats.shipout.ineligible_barriers.saturating_add(1);
        }
    }

    pub(crate) fn record_output_routine_execution(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.output_routine_executions =
                cache.stats.output_routine_executions.saturating_add(1);
        }
    }

    pub(crate) fn begin_paragraph_recording(
        &mut self,
        checkpoint: crate::env::paragraph::ParagraphMutationCheckpoint,
    ) {
        if self.paragraph_front_ends_enabled() {
            self.paragraph_recording = Some(checkpoint);
        }
    }

    pub(crate) fn finish_paragraph_recording(
        &mut self,
    ) -> Option<crate::env::paragraph::ParagraphMutationCheckpoint> {
        self.paragraph_recording.take()
    }

    pub(crate) fn abandon_paragraph_recording(
        &mut self,
    ) -> Option<crate::env::paragraph::ParagraphMutationCheckpoint> {
        self.paragraph_recording.take()
    }

    pub(crate) fn record_paragraph_hit(&mut self, commands: usize, mutations: usize) {
        let Some(cache) = &mut self.cache else {
            return;
        };
        cache.stats.hits = cache.stats.hits.saturating_add(1);
        cache.stats.paragraph_hits = cache.stats.paragraph_hits.saturating_add(1);
        cache.stats.paragraph.hits = cache.stats.paragraph.hits.saturating_add(1);
        cache.stats.paragraph_commands_skipped = cache
            .stats
            .paragraph_commands_skipped
            .saturating_add(commands as u64);
        cache.stats.paragraph_mutations_replayed = cache
            .stats
            .paragraph_mutations_replayed
            .saturating_add(mutations as u64);
    }

    pub(crate) fn record_paragraph_line_hit(&mut self) {
        let Some(cache) = &mut self.cache else {
            return;
        };
        cache.stats.paragraph_line_hits = cache.stats.paragraph_line_hits.saturating_add(1);
    }

    pub(crate) fn finish_recorded_paragraph_lines(
        &mut self,
        dependencies: Vec<ObservedDependency>,
        prev_graf: Option<i32>,
        lines: RetainedNodeList,
        line_count: i32,
        last_badness: i32,
        provenance: ParagraphProvenanceRecipe,
    ) {
        let Some(region) = self.recorded_paragraphs.last_mut() else {
            return;
        };
        if region.barriers.is_empty() && region.lines.is_none() {
            region.break_dependencies = dependencies.into();
            region.break_prev_graf = prev_graf;
            region.lines = Some(lines);
            region.line_count = line_count;
            region.line_last_badness = last_badness;
            region.line_provenance = provenance;
        }
    }

    pub(crate) fn record_paragraph_validation_miss(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.stats.paragraph_validation_misses =
                cache.stats.paragraph_validation_misses.saturating_add(1);
            cache.stats.paragraph.validation_failures =
                cache.stats.paragraph.validation_failures.saturating_add(1);
        }
    }

    pub(crate) fn record_paragraph_barriers(&mut self, reasons: &[ParagraphBarrierReason]) {
        let Some(cache) = &mut self.cache else {
            return;
        };
        cache.stats.paragraph_barriers = cache.stats.paragraph_barriers.saturating_add(1);
        cache.stats.paragraph.ineligible_barriers = cache
            .stats
            .paragraph
            .ineligible_barriers
            .saturating_add(reasons.len() as u64);
        for &reason in reasons {
            let count = self.paragraph_barrier_reasons.entry(reason).or_default();
            *count = count.saturating_add(1);
            match reason {
                ParagraphBarrierReason::DisplayMath => {
                    cache.stats.paragraph_display_math_barriers = cache
                        .stats
                        .paragraph_display_math_barriers
                        .saturating_add(1);
                }
                ParagraphBarrierReason::Scantokens => {
                    cache.stats.paragraph_scantokens_barriers =
                        cache.stats.paragraph_scantokens_barriers.saturating_add(1);
                }
                ParagraphBarrierReason::MidParagraphInputOpen => {
                    cache.stats.paragraph_input_open_barriers =
                        cache.stats.paragraph_input_open_barriers.saturating_add(1);
                }
                ParagraphBarrierReason::UntrackedWorldAccess => {
                    cache.stats.paragraph_untracked_world_barriers = cache
                        .stats
                        .paragraph_untracked_world_barriers
                        .saturating_add(1);
                }
                ParagraphBarrierReason::NestedOutputRoutine => {
                    cache.stats.paragraph_output_routine_barriers = cache
                        .stats
                        .paragraph_output_routine_barriers
                        .saturating_add(1);
                }
                ParagraphBarrierReason::EndInput => {
                    cache.stats.paragraph_endinput_barriers =
                        cache.stats.paragraph_endinput_barriers.saturating_add(1);
                }
                ParagraphBarrierReason::UnsupportedEscapingWrite => {
                    cache.stats.paragraph_unsupported_write_barriers = cache
                        .stats
                        .paragraph_unsupported_write_barriers
                        .saturating_add(1);
                }
                ParagraphBarrierReason::UnsupportedInputTransition => {
                    cache.stats.paragraph_unsupported_input_transition_barriers = cache
                        .stats
                        .paragraph_unsupported_input_transition_barriers
                        .saturating_add(1);
                }
                ParagraphBarrierReason::UnsupportedGroupTransition => {
                    cache.stats.paragraph_unsupported_group_transition_barriers = cache
                        .stats
                        .paragraph_unsupported_group_transition_barriers
                        .saturating_add(1);
                }
            }
        }
    }

    pub(crate) fn record_paragraph_region(&mut self, region: RecordedParagraphRegion) {
        debug_assert!(region.barriers.is_empty());
        let started = std::time::Instant::now();
        let Some(cache) = &mut self.cache else {
            return;
        };
        cache.stats.paragraph_eligible_regions =
            cache.stats.paragraph_eligible_regions.saturating_add(1);
        cache.stats.paragraph_inserts = cache.stats.paragraph_inserts.saturating_add(1);
        cache.stats.paragraph.inserts = cache.stats.paragraph.inserts.saturating_add(1);
        let bytes = recorded_paragraph_retained_bytes(&region) as u64;
        let published = &mut cache.stats.paragraph_opportunities.published;
        published.regions = published.regions.saturating_add(1);
        published.bytes = published.bytes.saturating_add(bytes);
        self.recorded_paragraphs.push(region);
        let elapsed = started.elapsed();
        cache.stats.paragraph_opportunities.published.nanos = cache
            .stats
            .paragraph_opportunities
            .published
            .nanos
            .saturating_add(elapsed_nanos(elapsed));
        self.record_timing(PureMemoLayer::Paragraph, MemoTimingPhase::Record, elapsed);
    }

    pub(crate) fn record_carried_paragraph(&mut self, region: &RecordedParagraphRegion) {
        let started = std::time::Instant::now();
        if let Some(cache) = &mut self.cache {
            let metric = &mut cache.stats.paragraph_opportunities.carried_forward;
            metric.regions = metric.regions.saturating_add(1);
            metric.bytes = metric
                .bytes
                .saturating_add(recorded_paragraph_retained_bytes(region) as u64);
            metric.nanos = metric
                .nanos
                .saturating_add(elapsed_nanos(started.elapsed()));
        }
    }

    pub(crate) fn align_recorded_paragraph_start(
        &mut self,
        starting_span: RootSpanId,
    ) -> Option<RecordedParagraphRegion> {
        if !self.reuse_prior_paragraphs || !self.paragraph_front_ends {
            self.record_not_attempted(PureMemoLayer::Paragraph);
            return None;
        }
        let started = std::time::Instant::now();
        let cache = self.cache.as_mut()?;
        cache.stats.lookups = cache.stats.lookups.saturating_add(1);
        cache.stats.paragraph_lookups = cache.stats.paragraph_lookups.saturating_add(1);
        let aligned_index = self
            .prior_paragraph_starts
            .get(&starting_span)
            .copied()
            .filter(|&index| index >= self.prior_paragraph_cursor);
        let result = aligned_index.and_then(|index| self.prior_paragraphs.get(index).cloned());
        if let Some(index) = aligned_index {
            self.prior_paragraph_cursor = index.saturating_add(1);
        }
        if result.is_none() {
            cache.stats.misses = cache.stats.misses.saturating_add(1);
            cache.stats.paragraph.key_misses = cache.stats.paragraph.key_misses.saturating_add(1);
        }
        cache.stats.paragraph.lookups = cache.stats.paragraph.lookups.saturating_add(1);
        cache.stats.paragraph.lookup_nanos = cache
            .stats
            .paragraph
            .lookup_nanos
            .saturating_add(elapsed_nanos(started.elapsed()));
        result
    }

    /// Starts one speculative accepted-history suffix. Only a fork of the prior
    /// accepted substrate may resolve the retained node handles.
    pub fn begin_paragraph_history(&mut self, reuse_prior: bool) {
        self.recorded_paragraphs.clear();
        self.reuse_prior_paragraphs = reuse_prior;
        self.preserve_prior_paragraphs = false;
        self.prior_paragraph_cursor = 0;
    }

    /// Keeps the last accepted trace when a run-wide dependency mismatch makes
    /// every candidate unusable for the current revision. The trace may become
    /// valid again after a later inverse edit, and dropping its retained graphs
    /// here only adds deallocation and future priming work.
    pub(crate) fn preserve_prior_paragraph_history(&mut self) {
        self.recorded_paragraphs.clear();
        self.preserve_prior_paragraphs = true;
        self.reuse_prior_paragraphs = false;
        self.prior_paragraph_cursor = 0;
    }

    /// Publishes the speculative trace wholesale after its owning Universe is
    /// accepted as the new retained generation.
    pub fn accept_paragraph_history(&mut self) {
        if self.preserve_prior_paragraphs {
            self.recorded_paragraphs.clear();
            self.preserve_prior_paragraphs = false;
            self.prior_paragraph_cursor = 0;
            self.reuse_prior_paragraphs = false;
            return;
        }
        self.prior_paragraphs = std::mem::take(&mut self.recorded_paragraphs);
        self.prior_paragraph_starts.clear();
        for (index, region) in self.prior_paragraphs.iter().enumerate() {
            if let Some(start) = region.starting_span {
                self.prior_paragraph_starts.entry(start).or_insert(index);
            }
        }
        self.prior_paragraph_cursor = 0;
        self.reuse_prior_paragraphs = false;
    }

    /// Drops all trace metadata produced by an abandoned execution branch.
    pub fn discard_paragraph_history(&mut self) {
        self.recorded_paragraphs.clear();
        self.preserve_prior_paragraphs = false;
        self.reuse_prior_paragraphs = false;
        self.prior_paragraph_cursor = 0;
    }

    pub fn recorded_paragraphs(&self) -> &[RecordedParagraphRegion] {
        &self.recorded_paragraphs
    }

    /// Returns the currently accepted ordered paragraph history.
    #[doc(hidden)]
    pub fn accepted_paragraphs(&self) -> &[RecordedParagraphRegion] {
        &self.prior_paragraphs
    }

    pub(crate) fn insert_pretolerance(&mut self, key: PureMemoKey, plan: Option<PureBreakPlan>) {
        if !self.pretolerance {
            self.record_not_attempted(PureMemoLayer::Pretolerance);
            return;
        }
        let started = std::time::Instant::now();
        let owned_bytes = plan.as_ref().map_or(0, |plan| {
            plan.breaks
                .capacity()
                .saturating_mul(std::mem::size_of::<PureBreakDecision>())
        });
        self.insert_value(key, PureMemoValue::Pretolerance(plan), owned_bytes);
        self.record_timing(
            PureMemoLayer::Pretolerance,
            MemoTimingPhase::Record,
            started.elapsed(),
        );
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

    pub(crate) fn reject(&mut self, key: PureMemoKey) {
        let Some(cache) = self.cache.as_mut() else {
            return;
        };
        cache.stats.malformed = cache.stats.malformed.saturating_add(1);
        cache.remove(key, false);
    }

    #[must_use]
    pub fn stats(&self) -> PureMemoStats {
        let mut stats = self
            .cache
            .as_ref()
            .map_or_else(PureMemoStats::default, |cache| cache.stats);
        stats.paragraph_history_metadata_bytes = self.paragraph_metadata_bytes();
        stats.pretolerance.retained_bytes = stats.pretolerance_retained_bytes;
        stats.page.retained_bytes = stats.page_retained_bytes;
        stats.shipout.retained_bytes = stats.shipout_retained_bytes;
        stats.paragraph.retained_bytes = stats.paragraph_history_metadata_bytes;
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

    pub(crate) fn record_paragraph_phase(
        &mut self,
        phase: ParagraphRecordingPhase,
        elapsed: Duration,
    ) {
        if let Some(cache) = &mut self.cache {
            cache.stats.paragraph_recording.add(phase, elapsed, 1);
        }
    }

    pub(crate) fn record_paragraph_phase_samples(
        &mut self,
        phase: ParagraphRecordingPhase,
        elapsed: Duration,
        samples: u64,
    ) {
        if let Some(cache) = &mut self.cache {
            cache.stats.paragraph_recording.add(phase, elapsed, samples);
        }
    }

    pub fn record_paragraph_validation_failure(&mut self, reason: ParagraphValidationFailure) {
        self.record_paragraph_validation_miss();
        if let Some(cache) = &mut self.cache {
            let slot = &mut cache.stats.paragraph_validation_failure_reasons[reason as usize];
            *slot = slot.saturating_add(1);
        }
    }

    fn paragraph_metadata_bytes(&self) -> usize {
        self.prior_paragraphs
            .iter()
            .chain(&self.recorded_paragraphs)
            .map(recorded_paragraph_retained_bytes)
            .sum::<usize>()
            .saturating_add(
                self.prior_paragraph_starts
                    .capacity()
                    .saturating_mul(std::mem::size_of::<(RootSpanId, usize)>()),
            )
    }
}

impl PureMemoCache {
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
            self.evicted_keys.insert(key);
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
            Self::Pretolerance(_) | Self::Detached => PureMemoKind::Pretolerance,
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
            PureMemoLayer::Paragraph => self.paragraph,
            PureMemoLayer::Page => self.page,
            PureMemoLayer::Shipout => self.shipout,
        }
    }

    #[must_use]
    pub const fn paragraph_validation_failure_count(
        &self,
        reason: ParagraphValidationFailure,
    ) -> u64 {
        self.paragraph_validation_failure_reasons[reason as usize]
    }

    fn layer_mut(&mut self, layer: PureMemoLayer) -> &mut MemoLayerStats {
        match layer {
            PureMemoLayer::Pretolerance => &mut self.pretolerance,
            PureMemoLayer::Paragraph => &mut self.paragraph,
            PureMemoLayer::Page => &mut self.page,
            PureMemoLayer::Shipout => &mut self.shipout,
        }
    }
}

fn elapsed_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn recorded_paragraph_retained_bytes(region: &RecordedParagraphRegion) -> usize {
    std::mem::size_of::<RecordedParagraphRegion>()
        .saturating_add(
            region
                .lines
                .as_ref()
                .map_or(0, RetainedNodeList::resource_retained_bytes),
        )
        .saturating_add(
            region
                .consumed_spans
                .len()
                .saturating_mul(std::mem::size_of::<RootSpanId>()),
        )
        .saturating_add(
            region
                .dependencies
                .len()
                .saturating_mul(std::mem::size_of::<ObservedDependency>()),
        )
        .saturating_add(
            region
                .mutations
                .len()
                .saturating_mul(std::mem::size_of::<PureParagraphMutation>()),
        )
        .saturating_add(
            region
                .effects
                .iter()
                .map(|effect| {
                    effect
                        .operation
                        .capacity()
                        .saturating_add(effect.payload.capacity())
                })
                .sum::<usize>(),
        )
        .saturating_add(
            region
                .barriers
                .len()
                .saturating_mul(std::mem::size_of::<ParagraphBarrierReason>()),
        )
        .saturating_add(
            region
                .break_dependencies
                .len()
                .saturating_mul(std::mem::size_of::<ObservedDependency>()),
        )
        .saturating_add(paragraph_provenance_retained_bytes(&region.line_provenance))
}

fn paragraph_provenance_retained_bytes(recipe: &ParagraphProvenanceRecipe) -> usize {
    recipe
        .piece_anchors
        .len()
        .saturating_mul(std::mem::size_of::<RootSpanId>())
        .saturating_add(
            recipe
                .root_spans
                .len()
                .saturating_mul(std::mem::size_of::<ParagraphProvenanceSpan>()),
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
