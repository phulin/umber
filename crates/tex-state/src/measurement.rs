//! Feature-gated process-local performance-owner measurements.
//!
//! These counters are absent from normal builds and never participate in
//! snapshots, rollback, replay, or semantic hashing. Most describe explicit
//! structural owners; `hot_core` additionally consumes the profiling
//! executable's isolated allocation wrapper inside named current-core scopes.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::state_hash::StateHashComponent;

mod hot_core;

pub use hot_core::{
    HOT_CORE_EXPANDABLE_OPCODE_COUNT, HOT_CORE_UNEXPANDABLE_OPCODE_COUNT,
    HotCoreAllocationMeasurement, HotCoreAllocationOwner, HotCoreAllocator, HotCoreCensus,
    HotCoreCloneMeasurement, HotCoreCommandFamily, HotCoreMaterialization, HotCorePhase,
    HotCoreStopReason, HotCoreWeakGraphMeasurement, HotCoreWeakIndexMeasurement,
    hot_core_allocation_scope, hot_core_census, record_hot_core_allocation,
    record_hot_core_arc_retain, record_hot_core_command_family, record_hot_core_content_hash,
    record_hot_core_episode, record_hot_core_expandable_opcode,
    record_hot_core_interpreter_construction, record_hot_core_interpreter_operation_entry,
    record_hot_core_macro_expansion, record_hot_core_materialization, record_hot_core_phase,
    record_hot_core_provenance_materialization, record_hot_core_snapshot_clone,
    record_hot_core_unexpandable_opcode, record_hot_core_weak_index, record_hot_core_weak_retain,
    record_hot_core_weak_upgrade,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeAppendMeasurement {
    pub calls: u64,
    pub words: u64,
    pub sidecar_rows: [u64; 14],
    pub capacity_growth_events: u64,
    pub capacity_growth_by_column: [u64; 33],
    pub compact_copy_calls: u64,
    pub compact_copy_words: u64,
    pub compact_copy_growth_by_column: [u64; 33],
    pub retained_payload_bytes_grown: u64,
}

impl Default for NodeAppendMeasurement {
    fn default() -> Self {
        Self {
            calls: 0,
            words: 0,
            sidecar_rows: [0; 14],
            capacity_growth_events: 0,
            capacity_growth_by_column: [0; 33],
            compact_copy_calls: 0,
            compact_copy_words: 0,
            compact_copy_growth_by_column: [0; 33],
            retained_payload_bytes_grown: 0,
        }
    }
}

pub const NODE_APPEND_CAPACITY_COLUMNS: [&str; 33] = [
    "words",
    "origins",
    "ligatures",
    "boxes",
    "unsets.kind",
    "unsets.width",
    "unsets.height",
    "unsets.depth",
    "unsets.span_count",
    "unsets.stretch",
    "unsets.stretch_order",
    "unsets.shrink",
    "unsets.shrink_order",
    "unsets.children",
    "rules",
    "leaders",
    "discs",
    "marks",
    "insertions.class",
    "insertions.size",
    "insertions.split_top_skip",
    "insertions.split_max_depth",
    "insertions.floating_penalty",
    "insertions.content",
    "whatsits",
    "noads.kind",
    "noads.nucleus",
    "noads.subscript",
    "noads.superscript",
    "fractions",
    "choices",
    "math_lists",
    "adjusts",
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StateHashMeasurement {
    pub calls: u64,
    pub journal_entries: u64,
    pub changed_cells: u64,
    pub node_frames: u64,
    pub owned_node_bytes: u64,
    pub owned_font_keys: u64,
    pub peak_changed_cell_scratch_bytes: u64,
    pub peak_node_scratch_bytes: u64,
    pub components: [StateHashComponentMeasurement; StateHashComponent::COUNT],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StateHashComponentMeasurement {
    pub calls: u64,
    pub visits: u64,
    pub nanos: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExactIdentityMeasurement {
    pub calls: u64,
    pub nanos: u64,
    pub projection_calls: u64,
    pub projection_visits: u64,
    pub projection_nanos: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TracedListMeasurement {
    pub finishes: u64,
    pub tokens: u64,
    pub token_builder_retained_bytes: u64,
    pub origin_builder_retained_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokenStoreMeasurement {
    pub intern_calls: u64,
    pub hits: u64,
    pub misses: u64,
    pub requested_tokens: u64,
    pub arena_capacity_bytes_grown: u64,
    pub semantic_identity_capacity_bytes_grown: u64,
}

/// Process-local census of loaded-format restoration work.
///
/// `allocations` counts explicit restoration-owned heap buffers, while
/// `copies` counts entries materialized solely for a later validation pass.
/// Allocator-level call and byte counts belong to the focused benchmark.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FormatRestoreMeasurement {
    pub calls: u64,
    pub bytes_decoded: u64,
    pub token_entries_restored: u64,
    pub macro_entries_restored: u64,
    pub glue_entries_restored: u64,
    pub node_entries_restored: u64,
    pub validation_passes: u64,
    pub copies: u64,
    pub allocations: u64,
}

impl FormatRestoreMeasurement {
    #[must_use]
    pub const fn saturating_sub(self, baseline: Self) -> Self {
        Self {
            calls: self.calls.saturating_sub(baseline.calls),
            bytes_decoded: self.bytes_decoded.saturating_sub(baseline.bytes_decoded),
            token_entries_restored: self
                .token_entries_restored
                .saturating_sub(baseline.token_entries_restored),
            macro_entries_restored: self
                .macro_entries_restored
                .saturating_sub(baseline.macro_entries_restored),
            glue_entries_restored: self
                .glue_entries_restored
                .saturating_sub(baseline.glue_entries_restored),
            node_entries_restored: self
                .node_entries_restored
                .saturating_sub(baseline.node_entries_restored),
            validation_passes: self
                .validation_passes
                .saturating_sub(baseline.validation_passes),
            copies: self.copies.saturating_sub(baseline.copies),
            allocations: self.allocations.saturating_sub(baseline.allocations),
        }
    }
}

/// Process-local census of structural provenance lifecycle work.
///
/// These fixed-width counters are compiled only into profiling builds. They
/// observe ownership operations without becoming part of engine state,
/// rollback, formats, or semantic identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProvenanceLifecycleMeasurement {
    pub atom_intern_calls: u64,
    pub atom_intern_hits: u64,
    pub atom_intern_misses: u64,
    pub atom_allocations: u64,
    pub frame_intern_calls: u64,
    pub frame_intern_hits: u64,
    pub frame_intern_misses: u64,
    pub frame_allocations: u64,
    pub list_intern_calls: u64,
    pub list_intern_hits: u64,
    pub list_intern_misses: u64,
    pub list_allocations: u64,
    pub atom_retains: u64,
    pub atom_releases: u64,
    pub frame_retains: u64,
    pub frame_releases: u64,
    pub origin_resolutions: u64,
    pub list_resolutions: u64,
    pub list_resolution_comparisons: u64,
}

impl ProvenanceLifecycleMeasurement {
    #[must_use]
    pub const fn saturating_sub(self, baseline: Self) -> Self {
        Self {
            atom_intern_calls: self
                .atom_intern_calls
                .saturating_sub(baseline.atom_intern_calls),
            atom_intern_hits: self
                .atom_intern_hits
                .saturating_sub(baseline.atom_intern_hits),
            atom_intern_misses: self
                .atom_intern_misses
                .saturating_sub(baseline.atom_intern_misses),
            atom_allocations: self
                .atom_allocations
                .saturating_sub(baseline.atom_allocations),
            frame_intern_calls: self
                .frame_intern_calls
                .saturating_sub(baseline.frame_intern_calls),
            frame_intern_hits: self
                .frame_intern_hits
                .saturating_sub(baseline.frame_intern_hits),
            frame_intern_misses: self
                .frame_intern_misses
                .saturating_sub(baseline.frame_intern_misses),
            frame_allocations: self
                .frame_allocations
                .saturating_sub(baseline.frame_allocations),
            list_intern_calls: self
                .list_intern_calls
                .saturating_sub(baseline.list_intern_calls),
            list_intern_hits: self
                .list_intern_hits
                .saturating_sub(baseline.list_intern_hits),
            list_intern_misses: self
                .list_intern_misses
                .saturating_sub(baseline.list_intern_misses),
            list_allocations: self
                .list_allocations
                .saturating_sub(baseline.list_allocations),
            atom_retains: self.atom_retains.saturating_sub(baseline.atom_retains),
            atom_releases: self.atom_releases.saturating_sub(baseline.atom_releases),
            frame_retains: self.frame_retains.saturating_sub(baseline.frame_retains),
            frame_releases: self.frame_releases.saturating_sub(baseline.frame_releases),
            origin_resolutions: self
                .origin_resolutions
                .saturating_sub(baseline.origin_resolutions),
            list_resolutions: self
                .list_resolutions
                .saturating_sub(baseline.list_resolutions),
            list_resolution_comparisons: self
                .list_resolution_comparisons
                .saturating_sub(baseline.list_resolution_comparisons),
        }
    }
}

/// Process-local census of TeX82 diagnostic main-memory projection reuse.
///
/// The counters describe derived accounting work only. They do not participate
/// in engine state, allocator high-water values, snapshots, or formats.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MainMemoryProjectionMeasurement {
    pub dynamic_observations: u64,
    pub base_requests: u64,
    pub base_reuses: u64,
    pub full_rebuilds: u64,
    pub operation_boundaries: u64,
    pub operation_boundaries_retained: u64,
    pub cell_root_updates: u64,
    pub cell_root_updates_retained: u64,
    pub box_root_updates: u64,
    pub box_root_updates_retained: u64,
    pub cache_losses: [u64; MainMemoryProjectionLossOwner::COUNT],
}

/// Exhaustive owner of a cached main-memory projection loss.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MainMemoryProjectionLossOwner {
    ProfileChange,
    CellRootUpdate,
    BoxRootUpdate,
}

impl MainMemoryProjectionLossOwner {
    pub(crate) const COUNT: usize = 3;

    const fn index(self) -> usize {
        self as usize
    }
}

impl MainMemoryProjectionMeasurement {
    /// Returns every possible cache-loss owner, including zero-count owners.
    pub fn named_cache_losses(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        const NAMES: [&str; MainMemoryProjectionLossOwner::COUNT] =
            ["profile_change", "cell_root_update", "box_root_update"];
        [("operation_boundary", 0), ("timeline_rollback", 0)]
            .into_iter()
            .chain(NAMES.into_iter().zip(self.cache_losses.iter().copied()))
    }
}

static NODE_APPEND_CALLS: AtomicU64 = AtomicU64::new(0);
static NODE_APPEND_WORDS: AtomicU64 = AtomicU64::new(0);
static NODE_APPEND_SIDECARS: [AtomicU64; 14] = [const { AtomicU64::new(0) }; 14];
static NODE_APPEND_GROWTH_EVENTS: AtomicU64 = AtomicU64::new(0);
static NODE_APPEND_GROWTH_BY_COLUMN: [AtomicU64; 33] = [const { AtomicU64::new(0) }; 33];
static NODE_COMPACT_COPY_CALLS: AtomicU64 = AtomicU64::new(0);
static NODE_COMPACT_COPY_WORDS: AtomicU64 = AtomicU64::new(0);
static NODE_COMPACT_COPY_GROWTH_BY_COLUMN: [AtomicU64; 33] = [const { AtomicU64::new(0) }; 33];
static NODE_APPEND_GROWN_BYTES: AtomicU64 = AtomicU64::new(0);

static HASH_CALLS: AtomicU64 = AtomicU64::new(0);
static HASH_JOURNAL_ENTRIES: AtomicU64 = AtomicU64::new(0);
static HASH_CHANGED_CELLS: AtomicU64 = AtomicU64::new(0);
static HASH_NODE_FRAMES: AtomicU64 = AtomicU64::new(0);
static HASH_OWNED_NODE_BYTES: AtomicU64 = AtomicU64::new(0);
static HASH_OWNED_FONT_KEYS: AtomicU64 = AtomicU64::new(0);
static HASH_PEAK_CHANGED_SCRATCH: AtomicU64 = AtomicU64::new(0);
static HASH_PEAK_NODE_SCRATCH: AtomicU64 = AtomicU64::new(0);
static HASH_COMPONENT_CALLS: [AtomicU64; StateHashComponent::COUNT] =
    [const { AtomicU64::new(0) }; StateHashComponent::COUNT];
static HASH_COMPONENT_VISITS: [AtomicU64; StateHashComponent::COUNT] =
    [const { AtomicU64::new(0) }; StateHashComponent::COUNT];
static HASH_COMPONENT_NANOS: [AtomicU64; StateHashComponent::COUNT] =
    [const { AtomicU64::new(0) }; StateHashComponent::COUNT];
static EXACT_IDENTITY_CALLS: AtomicU64 = AtomicU64::new(0);
static EXACT_IDENTITY_NANOS: AtomicU64 = AtomicU64::new(0);
static EXACT_IDENTITY_PROJECTION_CALLS: AtomicU64 = AtomicU64::new(0);
static EXACT_IDENTITY_PROJECTION_VISITS: AtomicU64 = AtomicU64::new(0);
static EXACT_IDENTITY_PROJECTION_NANOS: AtomicU64 = AtomicU64::new(0);

static TRACED_FINISHES: AtomicU64 = AtomicU64::new(0);
static TRACED_TOKENS: AtomicU64 = AtomicU64::new(0);
static TRACED_TOKEN_BUILDER_BYTES: AtomicU64 = AtomicU64::new(0);
static TRACED_ORIGIN_BUILDER_BYTES: AtomicU64 = AtomicU64::new(0);

static TOKEN_INTERN_CALLS: AtomicU64 = AtomicU64::new(0);
static TOKEN_HITS: AtomicU64 = AtomicU64::new(0);
static TOKEN_MISSES: AtomicU64 = AtomicU64::new(0);
static TOKEN_REQUESTED: AtomicU64 = AtomicU64::new(0);
static TOKEN_ARENA_GROWN_BYTES: AtomicU64 = AtomicU64::new(0);
static TOKEN_SEMANTIC_ID_GROWN_BYTES: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_CALLS: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_BYTES: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_TOKENS: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_MACROS: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_GLUE: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_NODES: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_VALIDATION_PASSES: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_COPIES: AtomicU64 = AtomicU64::new(0);
static FORMAT_RESTORE_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static PROVENANCE_COUNTERS: [AtomicU64; 19] = [const { AtomicU64::new(0) }; 19];
static MAIN_MEMORY_DYNAMIC_OBSERVATIONS: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_BASE_REQUESTS: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_BASE_REUSES: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_FULL_REBUILDS: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_OPERATION_BOUNDARIES: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_OPERATION_BOUNDARIES_RETAINED: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_CELL_ROOT_UPDATES: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_CELL_ROOT_UPDATES_RETAINED: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_BOX_ROOT_UPDATES: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_BOX_ROOT_UPDATES_RETAINED: AtomicU64 = AtomicU64::new(0);
static MAIN_MEMORY_CACHE_LOSSES: [AtomicU64; MainMemoryProjectionLossOwner::COUNT] =
    [const { AtomicU64::new(0) }; MainMemoryProjectionLossOwner::COUNT];

pub(crate) fn record_main_memory_dynamic_observation() {
    MAIN_MEMORY_DYNAMIC_OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_main_memory_base_request(reused: bool) {
    MAIN_MEMORY_BASE_REQUESTS.fetch_add(1, Ordering::Relaxed);
    if reused {
        MAIN_MEMORY_BASE_REUSES.fetch_add(1, Ordering::Relaxed);
    } else {
        MAIN_MEMORY_FULL_REBUILDS.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_main_memory_operation_boundary(retained: bool) {
    MAIN_MEMORY_OPERATION_BOUNDARIES.fetch_add(1, Ordering::Relaxed);
    if retained {
        MAIN_MEMORY_OPERATION_BOUNDARIES_RETAINED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_main_memory_cell_root_update(retained: bool) {
    MAIN_MEMORY_CELL_ROOT_UPDATES.fetch_add(1, Ordering::Relaxed);
    if retained {
        MAIN_MEMORY_CELL_ROOT_UPDATES_RETAINED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_main_memory_box_root_update(retained: bool) {
    MAIN_MEMORY_BOX_ROOT_UPDATES.fetch_add(1, Ordering::Relaxed);
    if retained {
        MAIN_MEMORY_BOX_ROOT_UPDATES_RETAINED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_main_memory_cache_loss(owner: MainMemoryProjectionLossOwner) {
    MAIN_MEMORY_CACHE_LOSSES[owner.index()].fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn record_node_append(
    words: usize,
    sidecars: [u32; 14],
    capacity_growth_by_column: [u8; 33],
    retained_payload_bytes_grown: usize,
    compact_copy: bool,
) {
    NODE_APPEND_CALLS.fetch_add(1, Ordering::Relaxed);
    NODE_APPEND_WORDS.fetch_add(words as u64, Ordering::Relaxed);
    for (counter, value) in NODE_APPEND_SIDECARS.iter().zip(sidecars) {
        counter.fetch_add(u64::from(value), Ordering::Relaxed);
    }
    let capacity_growth_events = capacity_growth_by_column
        .iter()
        .map(|&grew| u64::from(grew))
        .sum::<u64>();
    NODE_APPEND_GROWTH_EVENTS.fetch_add(capacity_growth_events, Ordering::Relaxed);
    for (counter, grew) in NODE_APPEND_GROWTH_BY_COLUMN
        .iter()
        .zip(capacity_growth_by_column)
    {
        counter.fetch_add(u64::from(grew), Ordering::Relaxed);
    }
    if compact_copy {
        NODE_COMPACT_COPY_CALLS.fetch_add(1, Ordering::Relaxed);
        NODE_COMPACT_COPY_WORDS.fetch_add(words as u64, Ordering::Relaxed);
        for (counter, grew) in NODE_COMPACT_COPY_GROWTH_BY_COLUMN
            .iter()
            .zip(capacity_growth_by_column)
        {
            counter.fetch_add(u64::from(grew), Ordering::Relaxed);
        }
    }
    NODE_APPEND_GROWN_BYTES.fetch_add(retained_payload_bytes_grown as u64, Ordering::Relaxed);
}

pub(crate) fn record_hash_call(journal_entries: usize) {
    HASH_CALLS.fetch_add(1, Ordering::Relaxed);
    HASH_JOURNAL_ENTRIES.fetch_add(journal_entries as u64, Ordering::Relaxed);
}

pub(crate) fn record_hash_changed_cells(changed: usize, scratch_bytes: usize) {
    HASH_CHANGED_CELLS.fetch_add(changed as u64, Ordering::Relaxed);
    HASH_PEAK_CHANGED_SCRATCH.fetch_max(scratch_bytes as u64, Ordering::Relaxed);
}

pub(crate) fn record_owned_font_key() {
    HASH_OWNED_FONT_KEYS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_state_hash_component(
    component: StateHashComponent,
    visits: usize,
    elapsed: std::time::Duration,
) {
    let index = component.index();
    HASH_COMPONENT_CALLS[index].fetch_add(1, Ordering::Relaxed);
    HASH_COMPONENT_VISITS[index].fetch_add(visits as u64, Ordering::Relaxed);
    HASH_COMPONENT_NANOS[index].fetch_add(
        elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
}

pub(crate) fn record_exact_identity(
    elapsed: std::time::Duration,
    projection_calls: u64,
    projection_visits: u64,
    projection_nanos: u64,
) {
    EXACT_IDENTITY_CALLS.fetch_add(1, Ordering::Relaxed);
    EXACT_IDENTITY_NANOS.fetch_add(
        elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
    EXACT_IDENTITY_PROJECTION_CALLS.fetch_add(projection_calls, Ordering::Relaxed);
    EXACT_IDENTITY_PROJECTION_VISITS.fetch_add(projection_visits, Ordering::Relaxed);
    EXACT_IDENTITY_PROJECTION_NANOS.fetch_add(projection_nanos, Ordering::Relaxed);
}

pub(crate) fn record_traced_list_finish(
    tokens: usize,
    token_builder_capacity: usize,
    origin_builder_capacity: usize,
) {
    TRACED_FINISHES.fetch_add(1, Ordering::Relaxed);
    TRACED_TOKENS.fetch_add(tokens as u64, Ordering::Relaxed);
    TRACED_TOKEN_BUILDER_BYTES.fetch_add(
        (token_builder_capacity * core::mem::size_of::<crate::token::Token>()) as u64,
        Ordering::Relaxed,
    );
    TRACED_ORIGIN_BUILDER_BYTES.fetch_add(
        (origin_builder_capacity * core::mem::size_of::<crate::token::OriginId>()) as u64,
        Ordering::Relaxed,
    );
}

pub(crate) fn record_token_intern(
    tokens: usize,
    hit: bool,
    arena_capacity_bytes_grown: usize,
    semantic_identity_capacity_bytes_grown: usize,
) {
    TOKEN_INTERN_CALLS.fetch_add(1, Ordering::Relaxed);
    TOKEN_REQUESTED.fetch_add(tokens as u64, Ordering::Relaxed);
    if hit {
        TOKEN_HITS.fetch_add(1, Ordering::Relaxed);
    } else {
        TOKEN_MISSES.fetch_add(1, Ordering::Relaxed);
    }
    TOKEN_ARENA_GROWN_BYTES.fetch_add(arena_capacity_bytes_grown as u64, Ordering::Relaxed);
    TOKEN_SEMANTIC_ID_GROWN_BYTES.fetch_add(
        semantic_identity_capacity_bytes_grown as u64,
        Ordering::Relaxed,
    );
}

pub(crate) fn record_format_restore_container(bytes: usize, allocations: usize) {
    FORMAT_RESTORE_CALLS.fetch_add(1, Ordering::Relaxed);
    FORMAT_RESTORE_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    FORMAT_RESTORE_ALLOCATIONS.fetch_add(allocations as u64, Ordering::Relaxed);
}

pub(crate) fn record_format_restore_entries(
    tokens: usize,
    macros: usize,
    glue: usize,
    nodes: usize,
) {
    FORMAT_RESTORE_TOKENS.fetch_add(tokens as u64, Ordering::Relaxed);
    FORMAT_RESTORE_MACROS.fetch_add(macros as u64, Ordering::Relaxed);
    FORMAT_RESTORE_GLUE.fetch_add(glue as u64, Ordering::Relaxed);
    FORMAT_RESTORE_NODES.fetch_add(nodes as u64, Ordering::Relaxed);
}

pub(crate) fn record_format_restore_work(
    validation_passes: usize,
    copies: usize,
    allocations: usize,
) {
    FORMAT_RESTORE_VALIDATION_PASSES.fetch_add(validation_passes as u64, Ordering::Relaxed);
    FORMAT_RESTORE_COPIES.fetch_add(copies as u64, Ordering::Relaxed);
    FORMAT_RESTORE_ALLOCATIONS.fetch_add(allocations as u64, Ordering::Relaxed);
}

const PROV_ATOM_INTERN_CALLS: usize = 0;
const PROV_ATOM_INTERN_HITS: usize = 1;
const PROV_ATOM_INTERN_MISSES: usize = 2;
const PROV_ATOM_ALLOCATIONS: usize = 3;
const PROV_FRAME_INTERN_CALLS: usize = 4;
const PROV_FRAME_INTERN_HITS: usize = 5;
const PROV_FRAME_INTERN_MISSES: usize = 6;
const PROV_FRAME_ALLOCATIONS: usize = 7;
const PROV_LIST_INTERN_CALLS: usize = 8;
const PROV_LIST_INTERN_HITS: usize = 9;
const PROV_LIST_INTERN_MISSES: usize = 10;
const PROV_LIST_ALLOCATIONS: usize = 11;
const PROV_ATOM_RETAINS: usize = 12;
const PROV_ATOM_RELEASES: usize = 13;
const PROV_FRAME_RETAINS: usize = 14;
const PROV_FRAME_RELEASES: usize = 15;
const PROV_ORIGIN_RESOLUTIONS: usize = 16;
const PROV_LIST_RESOLUTIONS: usize = 17;
const PROV_LIST_RESOLUTION_COMPARISONS: usize = 18;

pub(crate) fn record_provenance_list_intern(hit: bool, allocated: bool) {
    PROVENANCE_COUNTERS[PROV_LIST_INTERN_CALLS].fetch_add(1, Ordering::Relaxed);
    PROVENANCE_COUNTERS[if hit {
        PROV_LIST_INTERN_HITS
    } else {
        PROV_LIST_INTERN_MISSES
    }]
    .fetch_add(1, Ordering::Relaxed);
    if allocated {
        PROVENANCE_COUNTERS[PROV_LIST_ALLOCATIONS].fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_provenance_root_retain(frame: bool) {
    PROVENANCE_COUNTERS[if frame {
        PROV_FRAME_RETAINS
    } else {
        PROV_ATOM_RETAINS
    }]
    .fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_provenance_root_release(frame: bool) {
    PROVENANCE_COUNTERS[if frame {
        PROV_FRAME_RELEASES
    } else {
        PROV_ATOM_RELEASES
    }]
    .fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_provenance_origin_resolution() {
    PROVENANCE_COUNTERS[PROV_ORIGIN_RESOLUTIONS].fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_provenance_list_resolution(comparisons: usize) {
    PROVENANCE_COUNTERS[PROV_LIST_RESOLUTIONS].fetch_add(1, Ordering::Relaxed);
    PROVENANCE_COUNTERS[PROV_LIST_RESOLUTION_COMPARISONS]
        .fetch_add(comparisons as u64, Ordering::Relaxed);
}

#[must_use]
pub fn node_append_measurement() -> NodeAppendMeasurement {
    NodeAppendMeasurement {
        calls: NODE_APPEND_CALLS.load(Ordering::Relaxed),
        words: NODE_APPEND_WORDS.load(Ordering::Relaxed),
        sidecar_rows: core::array::from_fn(|index| {
            NODE_APPEND_SIDECARS[index].load(Ordering::Relaxed)
        }),
        capacity_growth_events: NODE_APPEND_GROWTH_EVENTS.load(Ordering::Relaxed),
        capacity_growth_by_column: core::array::from_fn(|index| {
            NODE_APPEND_GROWTH_BY_COLUMN[index].load(Ordering::Relaxed)
        }),
        compact_copy_calls: NODE_COMPACT_COPY_CALLS.load(Ordering::Relaxed),
        compact_copy_words: NODE_COMPACT_COPY_WORDS.load(Ordering::Relaxed),
        compact_copy_growth_by_column: core::array::from_fn(|index| {
            NODE_COMPACT_COPY_GROWTH_BY_COLUMN[index].load(Ordering::Relaxed)
        }),
        retained_payload_bytes_grown: NODE_APPEND_GROWN_BYTES.load(Ordering::Relaxed),
    }
}

#[must_use]
pub fn state_hash_measurement() -> StateHashMeasurement {
    StateHashMeasurement {
        calls: HASH_CALLS.load(Ordering::Relaxed),
        journal_entries: HASH_JOURNAL_ENTRIES.load(Ordering::Relaxed),
        changed_cells: HASH_CHANGED_CELLS.load(Ordering::Relaxed),
        node_frames: HASH_NODE_FRAMES.load(Ordering::Relaxed),
        owned_node_bytes: HASH_OWNED_NODE_BYTES.load(Ordering::Relaxed),
        owned_font_keys: HASH_OWNED_FONT_KEYS.load(Ordering::Relaxed),
        peak_changed_cell_scratch_bytes: HASH_PEAK_CHANGED_SCRATCH.load(Ordering::Relaxed),
        peak_node_scratch_bytes: HASH_PEAK_NODE_SCRATCH.load(Ordering::Relaxed),
        components: core::array::from_fn(|index| StateHashComponentMeasurement {
            calls: HASH_COMPONENT_CALLS[index].load(Ordering::Relaxed),
            visits: HASH_COMPONENT_VISITS[index].load(Ordering::Relaxed),
            nanos: HASH_COMPONENT_NANOS[index].load(Ordering::Relaxed),
        }),
    }
}

#[must_use]
pub fn exact_identity_measurement() -> ExactIdentityMeasurement {
    ExactIdentityMeasurement {
        calls: EXACT_IDENTITY_CALLS.load(Ordering::Relaxed),
        nanos: EXACT_IDENTITY_NANOS.load(Ordering::Relaxed),
        projection_calls: EXACT_IDENTITY_PROJECTION_CALLS.load(Ordering::Relaxed),
        projection_visits: EXACT_IDENTITY_PROJECTION_VISITS.load(Ordering::Relaxed),
        projection_nanos: EXACT_IDENTITY_PROJECTION_NANOS.load(Ordering::Relaxed),
    }
}

impl StateHashMeasurement {
    pub fn named_components(
        &self,
    ) -> impl Iterator<Item = (&'static str, StateHashComponentMeasurement)> + '_ {
        const NAMES: [&str; StateHashComponent::COUNT] = [
            "journal",
            "code_tables",
            "hyphenation",
            "prepared_mag",
            "font_selection",
            "world_effects",
            "world_shell_escapes",
            "world_streams",
            "world_scalars",
            "input_frames",
            "interaction",
            "page_scalars",
            "page_insertions",
            "page_marks",
            "page_contribution",
            "page_current",
            "page_discards",
            "mode",
        ];
        NAMES.into_iter().zip(self.components.iter().copied())
    }
}

#[must_use]
pub fn traced_list_measurement() -> TracedListMeasurement {
    TracedListMeasurement {
        finishes: TRACED_FINISHES.load(Ordering::Relaxed),
        tokens: TRACED_TOKENS.load(Ordering::Relaxed),
        token_builder_retained_bytes: TRACED_TOKEN_BUILDER_BYTES.load(Ordering::Relaxed),
        origin_builder_retained_bytes: TRACED_ORIGIN_BUILDER_BYTES.load(Ordering::Relaxed),
    }
}

#[must_use]
pub fn token_store_measurement() -> TokenStoreMeasurement {
    TokenStoreMeasurement {
        intern_calls: TOKEN_INTERN_CALLS.load(Ordering::Relaxed),
        hits: TOKEN_HITS.load(Ordering::Relaxed),
        misses: TOKEN_MISSES.load(Ordering::Relaxed),
        requested_tokens: TOKEN_REQUESTED.load(Ordering::Relaxed),
        arena_capacity_bytes_grown: TOKEN_ARENA_GROWN_BYTES.load(Ordering::Relaxed),
        semantic_identity_capacity_bytes_grown: TOKEN_SEMANTIC_ID_GROWN_BYTES
            .load(Ordering::Relaxed),
    }
}

#[must_use]
pub fn format_restore_measurement() -> FormatRestoreMeasurement {
    FormatRestoreMeasurement {
        calls: FORMAT_RESTORE_CALLS.load(Ordering::Relaxed),
        bytes_decoded: FORMAT_RESTORE_BYTES.load(Ordering::Relaxed),
        token_entries_restored: FORMAT_RESTORE_TOKENS.load(Ordering::Relaxed),
        macro_entries_restored: FORMAT_RESTORE_MACROS.load(Ordering::Relaxed),
        glue_entries_restored: FORMAT_RESTORE_GLUE.load(Ordering::Relaxed),
        node_entries_restored: FORMAT_RESTORE_NODES.load(Ordering::Relaxed),
        validation_passes: FORMAT_RESTORE_VALIDATION_PASSES.load(Ordering::Relaxed),
        copies: FORMAT_RESTORE_COPIES.load(Ordering::Relaxed),
        allocations: FORMAT_RESTORE_ALLOCATIONS.load(Ordering::Relaxed),
    }
}

#[must_use]
pub fn provenance_lifecycle_measurement() -> ProvenanceLifecycleMeasurement {
    let load = |index: usize| PROVENANCE_COUNTERS[index].load(Ordering::Relaxed);
    ProvenanceLifecycleMeasurement {
        atom_intern_calls: load(PROV_ATOM_INTERN_CALLS),
        atom_intern_hits: load(PROV_ATOM_INTERN_HITS),
        atom_intern_misses: load(PROV_ATOM_INTERN_MISSES),
        atom_allocations: load(PROV_ATOM_ALLOCATIONS),
        frame_intern_calls: load(PROV_FRAME_INTERN_CALLS),
        frame_intern_hits: load(PROV_FRAME_INTERN_HITS),
        frame_intern_misses: load(PROV_FRAME_INTERN_MISSES),
        frame_allocations: load(PROV_FRAME_ALLOCATIONS),
        list_intern_calls: load(PROV_LIST_INTERN_CALLS),
        list_intern_hits: load(PROV_LIST_INTERN_HITS),
        list_intern_misses: load(PROV_LIST_INTERN_MISSES),
        list_allocations: load(PROV_LIST_ALLOCATIONS),
        atom_retains: load(PROV_ATOM_RETAINS),
        atom_releases: load(PROV_ATOM_RELEASES),
        frame_retains: load(PROV_FRAME_RETAINS),
        frame_releases: load(PROV_FRAME_RELEASES),
        origin_resolutions: load(PROV_ORIGIN_RESOLUTIONS),
        list_resolutions: load(PROV_LIST_RESOLUTIONS),
        list_resolution_comparisons: load(PROV_LIST_RESOLUTION_COMPARISONS),
    }
}

#[must_use]
pub fn main_memory_projection_measurement() -> MainMemoryProjectionMeasurement {
    MainMemoryProjectionMeasurement {
        dynamic_observations: MAIN_MEMORY_DYNAMIC_OBSERVATIONS.load(Ordering::Relaxed),
        base_requests: MAIN_MEMORY_BASE_REQUESTS.load(Ordering::Relaxed),
        base_reuses: MAIN_MEMORY_BASE_REUSES.load(Ordering::Relaxed),
        full_rebuilds: MAIN_MEMORY_FULL_REBUILDS.load(Ordering::Relaxed),
        operation_boundaries: MAIN_MEMORY_OPERATION_BOUNDARIES.load(Ordering::Relaxed),
        operation_boundaries_retained: MAIN_MEMORY_OPERATION_BOUNDARIES_RETAINED
            .load(Ordering::Relaxed),
        cell_root_updates: MAIN_MEMORY_CELL_ROOT_UPDATES.load(Ordering::Relaxed),
        cell_root_updates_retained: MAIN_MEMORY_CELL_ROOT_UPDATES_RETAINED.load(Ordering::Relaxed),
        box_root_updates: MAIN_MEMORY_BOX_ROOT_UPDATES.load(Ordering::Relaxed),
        box_root_updates_retained: MAIN_MEMORY_BOX_ROOT_UPDATES_RETAINED.load(Ordering::Relaxed),
        cache_losses: core::array::from_fn(|index| {
            MAIN_MEMORY_CACHE_LOSSES[index].load(Ordering::Relaxed)
        }),
    }
}
