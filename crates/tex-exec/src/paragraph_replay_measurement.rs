//! Process-local attribution for the paragraph-replay deletion baseline.
//!
//! These counters exist only in profiling builds. They never participate in
//! engine state, snapshots, replay, hashes, or output.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParagraphReplayMeasurement {
    pub step_snapshot_calls: u64,
    pub step_snapshot_nanos: u64,
    pub step_snapshot_logical_bytes: u64,
    pub active_paragraph_step_snapshot_calls: u64,
    pub active_paragraph_recorder_logical_bytes: u64,
    pub continuation_detach_calls: u64,
    pub continuation_detach_nanos: u64,
    pub continuation_detach_paragraphs: u64,
    pub continuation_materialize_calls: u64,
    pub continuation_materialize_nanos: u64,
    pub continuation_materialize_paragraphs: u64,
}

static STEP_CALLS: AtomicU64 = AtomicU64::new(0);
static STEP_NANOS: AtomicU64 = AtomicU64::new(0);
static STEP_BYTES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_STEP_CALLS: AtomicU64 = AtomicU64::new(0);
static ACTIVE_RECORDER_BYTES: AtomicU64 = AtomicU64::new(0);
static DETACH_CALLS: AtomicU64 = AtomicU64::new(0);
static DETACH_NANOS: AtomicU64 = AtomicU64::new(0);
static DETACH_PARAGRAPHS: AtomicU64 = AtomicU64::new(0);
static MATERIALIZE_CALLS: AtomicU64 = AtomicU64::new(0);
static MATERIALIZE_NANOS: AtomicU64 = AtomicU64::new(0);
static MATERIALIZE_PARAGRAPHS: AtomicU64 = AtomicU64::new(0);

fn nanos(elapsed: Duration) -> u64 {
    elapsed.as_nanos().min(u128::from(u64::MAX)) as u64
}

pub(crate) fn record_step_snapshot(
    elapsed: Duration,
    logical_bytes: usize,
    active_paragraph_recorder_logical_bytes: Option<usize>,
) {
    STEP_CALLS.fetch_add(1, Ordering::Relaxed);
    STEP_NANOS.fetch_add(nanos(elapsed), Ordering::Relaxed);
    STEP_BYTES.fetch_add(logical_bytes as u64, Ordering::Relaxed);
    if let Some(bytes) = active_paragraph_recorder_logical_bytes {
        ACTIVE_STEP_CALLS.fetch_add(1, Ordering::Relaxed);
        ACTIVE_RECORDER_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

pub(crate) fn record_continuation_detach(elapsed: Duration, paragraphs: usize) {
    DETACH_CALLS.fetch_add(1, Ordering::Relaxed);
    DETACH_NANOS.fetch_add(nanos(elapsed), Ordering::Relaxed);
    DETACH_PARAGRAPHS.fetch_add(paragraphs as u64, Ordering::Relaxed);
}

pub(crate) fn record_continuation_materialize(elapsed: Duration, paragraphs: usize) {
    MATERIALIZE_CALLS.fetch_add(1, Ordering::Relaxed);
    MATERIALIZE_NANOS.fetch_add(nanos(elapsed), Ordering::Relaxed);
    MATERIALIZE_PARAGRAPHS.fetch_add(paragraphs as u64, Ordering::Relaxed);
}

#[must_use]
pub fn paragraph_replay_measurement() -> ParagraphReplayMeasurement {
    ParagraphReplayMeasurement {
        step_snapshot_calls: STEP_CALLS.load(Ordering::Relaxed),
        step_snapshot_nanos: STEP_NANOS.load(Ordering::Relaxed),
        step_snapshot_logical_bytes: STEP_BYTES.load(Ordering::Relaxed),
        active_paragraph_step_snapshot_calls: ACTIVE_STEP_CALLS.load(Ordering::Relaxed),
        active_paragraph_recorder_logical_bytes: ACTIVE_RECORDER_BYTES.load(Ordering::Relaxed),
        continuation_detach_calls: DETACH_CALLS.load(Ordering::Relaxed),
        continuation_detach_nanos: DETACH_NANOS.load(Ordering::Relaxed),
        continuation_detach_paragraphs: DETACH_PARAGRAPHS.load(Ordering::Relaxed),
        continuation_materialize_calls: MATERIALIZE_CALLS.load(Ordering::Relaxed),
        continuation_materialize_nanos: MATERIALIZE_NANOS.load(Ordering::Relaxed),
        continuation_materialize_paragraphs: MATERIALIZE_PARAGRAPHS.load(Ordering::Relaxed),
    }
}
