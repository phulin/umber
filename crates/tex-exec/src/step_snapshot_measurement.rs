//! Profiling-only attribution for canonical step snapshots.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StepSnapshotMeasurement {
    pub calls: u64,
    pub nanos: u64,
    pub logical_bytes: u64,
}

static CALLS: AtomicU64 = AtomicU64::new(0);
static NANOS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn record_step_snapshot(elapsed: Duration, logical_bytes: usize) {
    CALLS.fetch_add(1, Ordering::Relaxed);
    NANOS.fetch_add(
        elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
    BYTES.fetch_add(logical_bytes as u64, Ordering::Relaxed);
}

#[must_use]
pub fn step_snapshot_measurement() -> StepSnapshotMeasurement {
    StepSnapshotMeasurement {
        calls: CALLS.load(Ordering::Relaxed),
        nanos: NANOS.load(Ordering::Relaxed),
        logical_bytes: BYTES.load(Ordering::Relaxed),
    }
}
