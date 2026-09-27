//! Profiling-only volume of complete page output-carrier transfers.

use std::sync::atomic::{AtomicU64, Ordering};

static MOVES: AtomicU64 = AtomicU64::new(0);
static OUTPUT_SEMANTIC_NODES: AtomicU64 = AtomicU64::new(0);
static ALL_ROOTS_SEMANTIC_NODES: AtomicU64 = AtomicU64::new(0);
static MOVED_ENVELOPE_NODES: AtomicU64 = AtomicU64::new(0);
static MOVED_ENVELOPE_ANNEX_WORDS: AtomicU64 = AtomicU64::new(0);
static SURVIVOR_NODES_COPIED: AtomicU64 = AtomicU64::new(0);
static OBSERVATION_FAILURES: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutputCarrierTransferCensus {
    pub moves: u64,
    pub output_semantic_nodes: u64,
    pub all_roots_semantic_nodes: u64,
    pub moved_envelope_nodes: u64,
    pub moved_envelope_annex_words: u64,
    pub survivor_nodes_copied: u64,
    pub observation_failures: u64,
}

impl OutputCarrierTransferCensus {
    #[must_use]
    pub fn saturating_sub(self, before: Self) -> Self {
        Self {
            moves: self.moves.saturating_sub(before.moves),
            output_semantic_nodes: self
                .output_semantic_nodes
                .saturating_sub(before.output_semantic_nodes),
            all_roots_semantic_nodes: self
                .all_roots_semantic_nodes
                .saturating_sub(before.all_roots_semantic_nodes),
            moved_envelope_nodes: self
                .moved_envelope_nodes
                .saturating_sub(before.moved_envelope_nodes),
            moved_envelope_annex_words: self
                .moved_envelope_annex_words
                .saturating_sub(before.moved_envelope_annex_words),
            survivor_nodes_copied: self
                .survivor_nodes_copied
                .saturating_sub(before.survivor_nodes_copied),
            observation_failures: self
                .observation_failures
                .saturating_sub(before.observation_failures),
        }
    }
}

#[must_use]
pub fn output_carrier_transfer_census() -> OutputCarrierTransferCensus {
    OutputCarrierTransferCensus {
        moves: MOVES.load(Ordering::Relaxed),
        output_semantic_nodes: OUTPUT_SEMANTIC_NODES.load(Ordering::Relaxed),
        all_roots_semantic_nodes: ALL_ROOTS_SEMANTIC_NODES.load(Ordering::Relaxed),
        moved_envelope_nodes: MOVED_ENVELOPE_NODES.load(Ordering::Relaxed),
        moved_envelope_annex_words: MOVED_ENVELOPE_ANNEX_WORDS.load(Ordering::Relaxed),
        survivor_nodes_copied: SURVIVOR_NODES_COPIED.load(Ordering::Relaxed),
        observation_failures: OBSERVATION_FAILURES.load(Ordering::Relaxed),
    }
}

pub(crate) fn record_output_carrier_transfer(census: OutputCarrierTransferCensus) {
    MOVES.fetch_add(1, Ordering::Relaxed);
    OUTPUT_SEMANTIC_NODES.fetch_add(census.output_semantic_nodes, Ordering::Relaxed);
    ALL_ROOTS_SEMANTIC_NODES.fetch_add(census.all_roots_semantic_nodes, Ordering::Relaxed);
    MOVED_ENVELOPE_NODES.fetch_add(census.moved_envelope_nodes, Ordering::Relaxed);
    MOVED_ENVELOPE_ANNEX_WORDS.fetch_add(census.moved_envelope_annex_words, Ordering::Relaxed);
    SURVIVOR_NODES_COPIED.fetch_add(census.survivor_nodes_copied, Ordering::Relaxed);
    OBSERVATION_FAILURES.fetch_add(census.observation_failures, Ordering::Relaxed);
}
