//! Feature-gated process-local performance-owner measurements.
//!
//! These counters describe allocation owners without intercepting the global
//! allocator. They are absent from normal builds and never participate in
//! snapshots, rollback, replay, or semantic hashing.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::state_hash::StateHashComponent;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NodeAppendMeasurement {
    pub calls: u64,
    pub words: u64,
    pub sidecar_rows: [u64; 13],
    pub capacity_growth_events: u64,
    pub retained_payload_bytes_grown: u64,
}

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

static NODE_APPEND_CALLS: AtomicU64 = AtomicU64::new(0);
static NODE_APPEND_WORDS: AtomicU64 = AtomicU64::new(0);
static NODE_APPEND_SIDECARS: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];
static NODE_APPEND_GROWTH_EVENTS: AtomicU64 = AtomicU64::new(0);
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

pub(crate) fn record_node_append(
    words: usize,
    sidecars: [u32; 13],
    capacity_growth_events: usize,
    retained_payload_bytes_grown: usize,
) {
    NODE_APPEND_CALLS.fetch_add(1, Ordering::Relaxed);
    NODE_APPEND_WORDS.fetch_add(words as u64, Ordering::Relaxed);
    for (counter, value) in NODE_APPEND_SIDECARS.iter().zip(sidecars) {
        counter.fetch_add(u64::from(value), Ordering::Relaxed);
    }
    NODE_APPEND_GROWTH_EVENTS.fetch_add(capacity_growth_events as u64, Ordering::Relaxed);
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

#[must_use]
pub fn node_append_measurement() -> NodeAppendMeasurement {
    NodeAppendMeasurement {
        calls: NODE_APPEND_CALLS.load(Ordering::Relaxed),
        words: NODE_APPEND_WORDS.load(Ordering::Relaxed),
        sidecar_rows: core::array::from_fn(|index| {
            NODE_APPEND_SIDECARS[index].load(Ordering::Relaxed)
        }),
        capacity_growth_events: NODE_APPEND_GROWTH_EVENTS.load(Ordering::Relaxed),
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

impl StateHashMeasurement {
    #[must_use]
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
