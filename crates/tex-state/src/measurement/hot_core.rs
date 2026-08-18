//! Profiling-only census for the current canonical main-control hot core.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub use umber_hot_core_allocator::HotCoreAllocator;

/// Heap-allocation owner selected around one structurally distinct hot-core phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HotCoreAllocationOwner {
    CommandStateClone,
    StepSnapshotClone,
    DeliveryAndScan,
    SemanticApply,
    WeakValueStore,
    ProvenanceMaterialization,
    EvidencePublication,
}

impl HotCoreAllocationOwner {
    pub const COUNT: usize = 7;

    const fn index(self) -> usize {
        self as usize
    }

    pub const NAMES: [&'static str; Self::COUNT] = [
        "command_state_clone",
        "step_snapshot_clone",
        "delivery_and_scan",
        "semantic_apply",
        "weak_value_store",
        "provenance_materialization",
        "evidence_publication",
    ];
}

/// A stable boundary crossed by one canonical main-control operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HotCorePhase {
    StepSnapshot,
    DeliveryAndScan,
    SemanticApply,
    EvidencePublication,
    BarrierDecision,
}

impl HotCorePhase {
    pub const COUNT: usize = 5;

    const fn index(self) -> usize {
        self as usize
    }

    pub const NAMES: [&'static str; Self::COUNT] = [
        "step_snapshot",
        "delivery_and_scan",
        "semantic_apply",
        "evidence_publication",
        "barrier_decision",
    ];
}

/// Exhaustive terminal classification for an attempted bounded episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HotCoreStopReason {
    SliceLimit,
    SemanticResource,
    SemanticEffect,
    SemanticObserver,
    SemanticDiagnostic,
    SemanticCheckpoint,
    SemanticFormat,
    SemanticOutput,
    SemanticCancellation,
    SemanticFuel,
    SemanticStateIdentity,
    NamedCheckpoint,
    Terminal,
    InternalGroupLineage,
    InternalRollbackLineage,
    RollbackResource,
    RollbackDiagnostic,
    RollbackFuel,
}

impl HotCoreStopReason {
    pub const COUNT: usize = 18;

    const fn index(self) -> usize {
        self as usize
    }

    pub const NAMES: [&'static str; Self::COUNT] = [
        "slice_limit",
        "semantic_resource",
        "semantic_effect",
        "semantic_observer",
        "semantic_diagnostic",
        "semantic_checkpoint",
        "semantic_format",
        "semantic_output",
        "semantic_cancellation",
        "semantic_fuel",
        "semantic_state_identity",
        "named_checkpoint",
        "terminal",
        "internal_group_lineage",
        "internal_rollback_lineage",
        "rollback_resource",
        "rollback_diagnostic",
        "rollback_fuel",
    ];
}

/// Stable top-level meaning family delivered to main-control dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HotCoreCommandFamily {
    Character,
    Relax,
    Undefined,
    Macro,
    ExpandablePrimitive,
    UnexpandablePrimitive,
    RegisterOrParameter,
    Font,
    InternalQuantity,
    EndTemplate,
    Unknown,
}

impl HotCoreCommandFamily {
    pub const COUNT: usize = 11;

    const fn index(self) -> usize {
        self as usize
    }

    pub const NAMES: [&'static str; Self::COUNT] = [
        "character",
        "relax",
        "undefined",
        "macro",
        "expandable_primitive",
        "unexpandable_primitive",
        "register_or_parameter",
        "font",
        "internal_quantity",
        "end_template",
        "unknown",
    ];
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotCoreAllocationMeasurement {
    pub calls: u64,
    pub requested_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotCoreCloneMeasurement {
    pub calls: u64,
    pub nanos: u64,
    pub logical_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotCoreWeakGraphMeasurement {
    pub arc_retains: u64,
    pub weak_retains: u64,
    pub weak_upgrade_calls: u64,
    pub weak_upgrade_hits: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotCoreWeakIndexMeasurement {
    pub calls: u64,
    pub candidate_entries: u64,
    pub exact_comparisons: u64,
    pub content_hash_calls: u64,
}

/// One process-local monotonic reading. Subtract a run-entry reading to form a receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotCoreCensus {
    pub allocations: [HotCoreAllocationMeasurement; HotCoreAllocationOwner::COUNT],
    pub episode_lengths: [u64; 257],
    pub stop_reasons: [u64; HotCoreStopReason::COUNT],
    pub command_state_clones: HotCoreCloneMeasurement,
    pub step_snapshot_clones: HotCoreCloneMeasurement,
    pub weak_graph: HotCoreWeakGraphMeasurement,
    pub weak_index: HotCoreWeakIndexMeasurement,
    pub provenance_materialization_calls: u64,
    pub provenance_materialization_hits: u64,
    pub command_families: [u64; HotCoreCommandFamily::COUNT],
    pub phase_boundaries: [u64; HotCorePhase::COUNT],
}

impl Default for HotCoreCensus {
    fn default() -> Self {
        Self {
            allocations: [HotCoreAllocationMeasurement::default(); HotCoreAllocationOwner::COUNT],
            episode_lengths: [0; 257],
            stop_reasons: [0; HotCoreStopReason::COUNT],
            command_state_clones: HotCoreCloneMeasurement::default(),
            step_snapshot_clones: HotCoreCloneMeasurement::default(),
            weak_graph: HotCoreWeakGraphMeasurement::default(),
            weak_index: HotCoreWeakIndexMeasurement::default(),
            provenance_materialization_calls: 0,
            provenance_materialization_hits: 0,
            command_families: [0; HotCoreCommandFamily::COUNT],
            phase_boundaries: [0; HotCorePhase::COUNT],
        }
    }
}

impl HotCoreCensus {
    #[must_use]
    pub fn saturating_sub(self, baseline: Self) -> Self {
        Self {
            allocations: core::array::from_fn(|index| HotCoreAllocationMeasurement {
                calls: self.allocations[index]
                    .calls
                    .saturating_sub(baseline.allocations[index].calls),
                requested_bytes: self.allocations[index]
                    .requested_bytes
                    .saturating_sub(baseline.allocations[index].requested_bytes),
            }),
            episode_lengths: core::array::from_fn(|index| {
                self.episode_lengths[index].saturating_sub(baseline.episode_lengths[index])
            }),
            stop_reasons: core::array::from_fn(|index| {
                self.stop_reasons[index].saturating_sub(baseline.stop_reasons[index])
            }),
            command_state_clones: subtract_clone(
                self.command_state_clones,
                baseline.command_state_clones,
            ),
            step_snapshot_clones: subtract_clone(
                self.step_snapshot_clones,
                baseline.step_snapshot_clones,
            ),
            weak_graph: HotCoreWeakGraphMeasurement {
                arc_retains: self
                    .weak_graph
                    .arc_retains
                    .saturating_sub(baseline.weak_graph.arc_retains),
                weak_retains: self
                    .weak_graph
                    .weak_retains
                    .saturating_sub(baseline.weak_graph.weak_retains),
                weak_upgrade_calls: self
                    .weak_graph
                    .weak_upgrade_calls
                    .saturating_sub(baseline.weak_graph.weak_upgrade_calls),
                weak_upgrade_hits: self
                    .weak_graph
                    .weak_upgrade_hits
                    .saturating_sub(baseline.weak_graph.weak_upgrade_hits),
            },
            weak_index: HotCoreWeakIndexMeasurement {
                calls: self
                    .weak_index
                    .calls
                    .saturating_sub(baseline.weak_index.calls),
                candidate_entries: self
                    .weak_index
                    .candidate_entries
                    .saturating_sub(baseline.weak_index.candidate_entries),
                exact_comparisons: self
                    .weak_index
                    .exact_comparisons
                    .saturating_sub(baseline.weak_index.exact_comparisons),
                content_hash_calls: self
                    .weak_index
                    .content_hash_calls
                    .saturating_sub(baseline.weak_index.content_hash_calls),
            },
            provenance_materialization_calls: self
                .provenance_materialization_calls
                .saturating_sub(baseline.provenance_materialization_calls),
            provenance_materialization_hits: self
                .provenance_materialization_hits
                .saturating_sub(baseline.provenance_materialization_hits),
            command_families: core::array::from_fn(|index| {
                self.command_families[index].saturating_sub(baseline.command_families[index])
            }),
            phase_boundaries: core::array::from_fn(|index| {
                self.phase_boundaries[index].saturating_sub(baseline.phase_boundaries[index])
            }),
        }
    }
}

fn subtract_clone(
    measurement: HotCoreCloneMeasurement,
    baseline: HotCoreCloneMeasurement,
) -> HotCoreCloneMeasurement {
    HotCoreCloneMeasurement {
        calls: measurement.calls.saturating_sub(baseline.calls),
        nanos: measurement.nanos.saturating_sub(baseline.nanos),
        logical_bytes: measurement
            .logical_bytes
            .saturating_sub(baseline.logical_bytes),
    }
}

static EPISODE_LENGTHS: [AtomicU64; 257] = [const { AtomicU64::new(0) }; 257];
static STOP_REASONS: [AtomicU64; HotCoreStopReason::COUNT] =
    [const { AtomicU64::new(0) }; HotCoreStopReason::COUNT];
static COMMAND_STATE_CLONE: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static STEP_SNAPSHOT_CLONE: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static ARC_RETAINS: AtomicU64 = AtomicU64::new(0);
static WEAK_RETAINS: AtomicU64 = AtomicU64::new(0);
static WEAK_UPGRADE_CALLS: AtomicU64 = AtomicU64::new(0);
static WEAK_UPGRADE_HITS: AtomicU64 = AtomicU64::new(0);
static WEAK_INDEX_CALLS: AtomicU64 = AtomicU64::new(0);
static WEAK_INDEX_CANDIDATES: AtomicU64 = AtomicU64::new(0);
static WEAK_INDEX_COMPARISONS: AtomicU64 = AtomicU64::new(0);
static CONTENT_HASH_CALLS: AtomicU64 = AtomicU64::new(0);
static PROVENANCE_MATERIALIZATION_CALLS: AtomicU64 = AtomicU64::new(0);
static PROVENANCE_MATERIALIZATION_HITS: AtomicU64 = AtomicU64::new(0);
static COMMAND_FAMILIES: [AtomicU64; HotCoreCommandFamily::COUNT] =
    [const { AtomicU64::new(0) }; HotCoreCommandFamily::COUNT];
static PHASE_BOUNDARIES: [AtomicU64; HotCorePhase::COUNT] =
    [const { AtomicU64::new(0) }; HotCorePhase::COUNT];

#[must_use]
pub fn hot_core_allocation_scope(
    owner: HotCoreAllocationOwner,
) -> umber_hot_core_allocator::AllocationScope {
    umber_hot_core_allocator::scope(owner.index())
}

/// Called by a profiling binary's allocator wrapper. Allocations outside a named scope are ignored.
pub fn record_hot_core_allocation(requested_bytes: usize) {
    umber_hot_core_allocator::record(requested_bytes);
}

pub fn record_hot_core_phase(phase: HotCorePhase) {
    PHASE_BOUNDARIES[phase.index()].fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_episode(operations: usize, reason: HotCoreStopReason) {
    let bounded = operations.min(256);
    EPISODE_LENGTHS[bounded].fetch_add(1, Ordering::Relaxed);
    STOP_REASONS[reason.index()].fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_snapshot_clone(
    command_state: bool,
    elapsed: Duration,
    logical_bytes: usize,
) {
    let counters = if command_state {
        &COMMAND_STATE_CLONE
    } else {
        &STEP_SNAPSHOT_CLONE
    };
    counters[0].fetch_add(1, Ordering::Relaxed);
    counters[1].fetch_add(
        elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
    counters[2].fetch_add(logical_bytes as u64, Ordering::Relaxed);
}

pub fn record_hot_core_arc_retain() {
    ARC_RETAINS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_weak_retain() {
    WEAK_RETAINS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_weak_upgrade(hit: bool) {
    WEAK_UPGRADE_CALLS.fetch_add(1, Ordering::Relaxed);
    if hit {
        WEAK_UPGRADE_HITS.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn record_hot_core_weak_index(candidate_entries: usize, exact_comparisons: usize) {
    WEAK_INDEX_CALLS.fetch_add(1, Ordering::Relaxed);
    WEAK_INDEX_CANDIDATES.fetch_add(candidate_entries as u64, Ordering::Relaxed);
    WEAK_INDEX_COMPARISONS.fetch_add(exact_comparisons as u64, Ordering::Relaxed);
}

pub fn record_hot_core_content_hash() {
    CONTENT_HASH_CALLS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_provenance_materialization(hit: bool) {
    PROVENANCE_MATERIALIZATION_CALLS.fetch_add(1, Ordering::Relaxed);
    if hit {
        PROVENANCE_MATERIALIZATION_HITS.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn record_hot_core_command_family(family: HotCoreCommandFamily) {
    COMMAND_FAMILIES[family.index()].fetch_add(1, Ordering::Relaxed);
}

#[must_use]
pub fn hot_core_census() -> HotCoreCensus {
    let clone_measurement = |counters: &[AtomicU64; 3]| HotCoreCloneMeasurement {
        calls: counters[0].load(Ordering::Relaxed),
        nanos: counters[1].load(Ordering::Relaxed),
        logical_bytes: counters[2].load(Ordering::Relaxed),
    };
    HotCoreCensus {
        allocations: core::array::from_fn(|index| {
            let measurement = umber_hot_core_allocator::measurement(index);
            HotCoreAllocationMeasurement {
                calls: measurement.calls,
                requested_bytes: measurement.requested_bytes,
            }
        }),
        episode_lengths: core::array::from_fn(|index| {
            EPISODE_LENGTHS[index].load(Ordering::Relaxed)
        }),
        stop_reasons: core::array::from_fn(|index| STOP_REASONS[index].load(Ordering::Relaxed)),
        command_state_clones: clone_measurement(&COMMAND_STATE_CLONE),
        step_snapshot_clones: clone_measurement(&STEP_SNAPSHOT_CLONE),
        weak_graph: HotCoreWeakGraphMeasurement {
            arc_retains: ARC_RETAINS.load(Ordering::Relaxed),
            weak_retains: WEAK_RETAINS.load(Ordering::Relaxed),
            weak_upgrade_calls: WEAK_UPGRADE_CALLS.load(Ordering::Relaxed),
            weak_upgrade_hits: WEAK_UPGRADE_HITS.load(Ordering::Relaxed),
        },
        weak_index: HotCoreWeakIndexMeasurement {
            calls: WEAK_INDEX_CALLS.load(Ordering::Relaxed),
            candidate_entries: WEAK_INDEX_CANDIDATES.load(Ordering::Relaxed),
            exact_comparisons: WEAK_INDEX_COMPARISONS.load(Ordering::Relaxed),
            content_hash_calls: CONTENT_HASH_CALLS.load(Ordering::Relaxed),
        },
        provenance_materialization_calls: PROVENANCE_MATERIALIZATION_CALLS.load(Ordering::Relaxed),
        provenance_materialization_hits: PROVENANCE_MATERIALIZATION_HITS.load(Ordering::Relaxed),
        command_families: core::array::from_fn(|index| {
            COMMAND_FAMILIES[index].load(Ordering::Relaxed)
        }),
        phase_boundaries: core::array::from_fn(|index| {
            PHASE_BOUNDARIES[index].load(Ordering::Relaxed)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_scopes_are_nested_and_ignore_unowned_work() {
        let before = hot_core_census();
        record_hot_core_allocation(11);
        {
            let _outer = hot_core_allocation_scope(HotCoreAllocationOwner::DeliveryAndScan);
            record_hot_core_allocation(13);
            {
                let _inner = hot_core_allocation_scope(HotCoreAllocationOwner::WeakValueStore);
                record_hot_core_allocation(17);
            }
            record_hot_core_allocation(19);
        }
        record_hot_core_allocation(23);
        let delta = hot_core_census().saturating_sub(before);
        assert_eq!(
            delta.allocations[HotCoreAllocationOwner::DeliveryAndScan.index()],
            HotCoreAllocationMeasurement {
                calls: 2,
                requested_bytes: 32,
            }
        );
        assert_eq!(
            delta.allocations[HotCoreAllocationOwner::WeakValueStore.index()],
            HotCoreAllocationMeasurement {
                calls: 1,
                requested_bytes: 17,
            }
        );
        assert_eq!(
            delta
                .allocations
                .iter()
                .map(|value| value.calls)
                .sum::<u64>(),
            3
        );
    }

    #[test]
    fn structural_counters_have_positive_and_zero_controls() {
        let before = hot_core_census();
        record_hot_core_episode(7, HotCoreStopReason::SemanticEffect);
        record_hot_core_snapshot_clone(true, Duration::from_nanos(5), 29);
        record_hot_core_weak_upgrade(true);
        record_hot_core_weak_index(3, 2);
        record_hot_core_provenance_materialization(false);
        record_hot_core_command_family(HotCoreCommandFamily::Character);
        record_hot_core_phase(HotCorePhase::DeliveryAndScan);
        let delta = hot_core_census().saturating_sub(before);

        assert_eq!(delta.episode_lengths[7], 1);
        assert_eq!(
            delta.stop_reasons[HotCoreStopReason::SemanticEffect.index()],
            1
        );
        assert_eq!(delta.command_state_clones.calls, 1);
        assert_eq!(delta.command_state_clones.logical_bytes, 29);
        assert_eq!(delta.step_snapshot_clones.calls, 0);
        assert_eq!(delta.weak_graph.weak_upgrade_calls, 1);
        assert_eq!(delta.weak_graph.weak_upgrade_hits, 1);
        assert_eq!(delta.weak_index.candidate_entries, 3);
        assert_eq!(delta.weak_index.exact_comparisons, 2);
        assert_eq!(delta.provenance_materialization_calls, 1);
        assert_eq!(delta.provenance_materialization_hits, 0);
        assert_eq!(
            delta.command_families[HotCoreCommandFamily::Character.index()],
            1
        );
        assert_eq!(
            delta.phase_boundaries[HotCorePhase::DeliveryAndScan.index()],
            1
        );
        assert_eq!(
            delta.phase_boundaries[HotCorePhase::BarrierDecision.index()],
            0
        );
    }
}
