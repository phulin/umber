//! Profiling-only allocation and structural census for the canonical engine.

use std::sync::atomic::{AtomicU64, Ordering};

pub use umber_hot_core_allocator::HotCoreAllocator;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HotCoreAllocationOwner {
    DeliveryAndScan,
    SemanticApply,
    EvidencePublication,
    InterpreterConstruction,
    InterpreterBorrow,
    ColdMaterialization,
    AttemptScratch,
    GenerationBoundary,
    ArenaGrowth,
}

impl HotCoreAllocationOwner {
    pub const COUNT: usize = 9;

    const fn index(self) -> usize {
        self as usize
    }

    pub const NAMES: [&'static str; Self::COUNT] = [
        "delivery_and_scan",
        "semantic_apply",
        "evidence_publication",
        "interpreter_construction",
        "interpreter_borrow",
        "cold_materialization",
        "attempt_scratch",
        "generation_boundary",
        "arena_growth",
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HotCorePhase {
    DeliveryAndScan,
    SemanticApply,
    EvidencePublication,
    BarrierDecision,
}

impl HotCorePhase {
    pub const COUNT: usize = 4;

    const fn index(self) -> usize {
        self as usize
    }

    pub const NAMES: [&'static str; Self::COUNT] = [
        "delivery_and_scan",
        "semantic_apply",
        "evidence_publication",
        "barrier_decision",
    ];
}

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
    RollbackResource,
    RollbackDiagnostic,
    RollbackFuel,
}

impl HotCoreStopReason {
    pub const COUNT: usize = 16;

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
        "rollback_resource",
        "rollback_diagnostic",
        "rollback_fuel",
    ];
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HotCoreMaterialization {
    ExpansionCommand,
    ScannedStep,
    PreparedOperation,
}

impl HotCoreMaterialization {
    pub const COUNT: usize = 3;

    const fn index(self) -> usize {
        self as usize
    }

    pub const NAMES: [&'static str; Self::COUNT] =
        ["expansion_command", "scanned_step", "prepared_operation"];
}

pub const HOT_CORE_EXPANDABLE_OPCODE_COUNT: usize = 86;
pub const HOT_CORE_UNEXPANDABLE_OPCODE_COUNT: usize = 266;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotCoreAllocationMeasurement {
    pub calls: u64,
    pub requested_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotCoreAllocationTrace {
    pub owner: HotCoreAllocationOwner,
    pub requested_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotCoreCensus {
    pub allocations: [HotCoreAllocationMeasurement; HotCoreAllocationOwner::COUNT],
    pub episode_lengths: [u64; 257],
    pub stop_reasons: [u64; HotCoreStopReason::COUNT],
    pub command_families: [u64; HotCoreCommandFamily::COUNT],
    pub expandable_opcodes: [u64; HOT_CORE_EXPANDABLE_OPCODE_COUNT],
    pub macro_expansions: u64,
    pub unexpandable_opcodes: [u64; HOT_CORE_UNEXPANDABLE_OPCODE_COUNT],
    pub materializations: [u64; HotCoreMaterialization::COUNT],
    pub interpreter_constructions: u64,
    pub interpreter_operation_entries: u64,
    pub phase_boundaries: [u64; HotCorePhase::COUNT],
}

/// Coarse physical-generation ownership observed by profiling builds.
///
/// This deliberately counts owners, not rows or runtime ids. The lifetime
/// guard exists only behind `tex-state/profiling`, so normal builds contain no
/// counter field, branch, or atomic operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedGenerationCensus {
    pub created: u64,
    pub dropped: u64,
    pub live: u64,
    pub peak_live: u64,
    pub retired_explicitly: u64,
}

impl RetainedGenerationCensus {
    #[must_use]
    pub fn saturating_sub(self, baseline: Self) -> Self {
        Self {
            created: self.created.saturating_sub(baseline.created),
            dropped: self.dropped.saturating_sub(baseline.dropped),
            // Live and peak are gauges, not event counters.
            live: self.live,
            peak_live: self.peak_live,
            retired_explicitly: self
                .retired_explicitly
                .saturating_sub(baseline.retired_explicitly),
        }
    }
}

/// Profiling-only lifetime guard for one coarse retained generation.
#[derive(Debug)]
pub struct RetainedGenerationLifetime;

impl RetainedGenerationLifetime {
    #[must_use]
    pub fn begin() -> Self {
        RETAINED_GENERATIONS_CREATED.fetch_add(1, Ordering::Relaxed);
        let live = RETAINED_GENERATIONS_LIVE.fetch_add(1, Ordering::Relaxed) + 1;
        RETAINED_GENERATIONS_PEAK.fetch_max(live, Ordering::Relaxed);
        Self
    }
}

impl Drop for RetainedGenerationLifetime {
    fn drop(&mut self) {
        RETAINED_GENERATIONS_DROPPED.fetch_add(1, Ordering::Relaxed);
        let previous = RETAINED_GENERATIONS_LIVE.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous != 0, "retained generation count underflow");
    }
}

impl Default for HotCoreCensus {
    fn default() -> Self {
        Self {
            allocations: [HotCoreAllocationMeasurement::default(); HotCoreAllocationOwner::COUNT],
            episode_lengths: [0; 257],
            stop_reasons: [0; HotCoreStopReason::COUNT],
            command_families: [0; HotCoreCommandFamily::COUNT],
            expandable_opcodes: [0; HOT_CORE_EXPANDABLE_OPCODE_COUNT],
            macro_expansions: 0,
            unexpandable_opcodes: [0; HOT_CORE_UNEXPANDABLE_OPCODE_COUNT],
            materializations: [0; HotCoreMaterialization::COUNT],
            interpreter_constructions: 0,
            interpreter_operation_entries: 0,
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
            command_families: core::array::from_fn(|index| {
                self.command_families[index].saturating_sub(baseline.command_families[index])
            }),
            expandable_opcodes: core::array::from_fn(|index| {
                self.expandable_opcodes[index].saturating_sub(baseline.expandable_opcodes[index])
            }),
            macro_expansions: self
                .macro_expansions
                .saturating_sub(baseline.macro_expansions),
            unexpandable_opcodes: core::array::from_fn(|index| {
                self.unexpandable_opcodes[index]
                    .saturating_sub(baseline.unexpandable_opcodes[index])
            }),
            materializations: core::array::from_fn(|index| {
                self.materializations[index].saturating_sub(baseline.materializations[index])
            }),
            interpreter_constructions: self
                .interpreter_constructions
                .saturating_sub(baseline.interpreter_constructions),
            interpreter_operation_entries: self
                .interpreter_operation_entries
                .saturating_sub(baseline.interpreter_operation_entries),
            phase_boundaries: core::array::from_fn(|index| {
                self.phase_boundaries[index].saturating_sub(baseline.phase_boundaries[index])
            }),
        }
    }
}

static EPISODE_LENGTHS: [AtomicU64; 257] = [const { AtomicU64::new(0) }; 257];
static STOP_REASONS: [AtomicU64; HotCoreStopReason::COUNT] =
    [const { AtomicU64::new(0) }; HotCoreStopReason::COUNT];
static COMMAND_FAMILIES: [AtomicU64; HotCoreCommandFamily::COUNT] =
    [const { AtomicU64::new(0) }; HotCoreCommandFamily::COUNT];
static EXPANDABLE_OPCODES: [AtomicU64; HOT_CORE_EXPANDABLE_OPCODE_COUNT] =
    [const { AtomicU64::new(0) }; HOT_CORE_EXPANDABLE_OPCODE_COUNT];
static MACRO_EXPANSIONS: AtomicU64 = AtomicU64::new(0);
static UNEXPANDABLE_OPCODES: [AtomicU64; HOT_CORE_UNEXPANDABLE_OPCODE_COUNT] =
    [const { AtomicU64::new(0) }; HOT_CORE_UNEXPANDABLE_OPCODE_COUNT];
static MATERIALIZATIONS: [AtomicU64; HotCoreMaterialization::COUNT] =
    [const { AtomicU64::new(0) }; HotCoreMaterialization::COUNT];
static INTERPRETER_CONSTRUCTIONS: AtomicU64 = AtomicU64::new(0);
static INTERPRETER_OPERATION_ENTRIES: AtomicU64 = AtomicU64::new(0);
static PHASE_BOUNDARIES: [AtomicU64; HotCorePhase::COUNT] =
    [const { AtomicU64::new(0) }; HotCorePhase::COUNT];
static RETAINED_GENERATIONS_CREATED: AtomicU64 = AtomicU64::new(0);
static RETAINED_GENERATIONS_DROPPED: AtomicU64 = AtomicU64::new(0);
static RETAINED_GENERATIONS_LIVE: AtomicU64 = AtomicU64::new(0);
static RETAINED_GENERATIONS_PEAK: AtomicU64 = AtomicU64::new(0);
static RETAINED_GENERATIONS_RETIRED: AtomicU64 = AtomicU64::new(0);

#[must_use]
pub fn hot_core_allocation_scope(
    owner: HotCoreAllocationOwner,
) -> umber_hot_core_allocator::AllocationScope {
    umber_hot_core_allocator::scope(owner.index())
}

#[must_use]
pub fn hot_core_thread_allocation_measurement(
    owner: HotCoreAllocationOwner,
) -> HotCoreAllocationMeasurement {
    let measurement = umber_hot_core_allocator::thread_measurement(owner.index());
    HotCoreAllocationMeasurement {
        calls: measurement.calls,
        requested_bytes: measurement.requested_bytes,
    }
}

#[must_use]
pub fn hot_core_allocation_trace_cursor() -> u64 {
    umber_hot_core_allocator::trace_cursor()
}

#[must_use]
pub fn hot_core_allocation_trace_entry(cursor: u64) -> Option<HotCoreAllocationTrace> {
    let entry = umber_hot_core_allocator::trace_entry(cursor)?;
    let owner = match entry.owner {
        0 => HotCoreAllocationOwner::DeliveryAndScan,
        1 => HotCoreAllocationOwner::SemanticApply,
        2 => HotCoreAllocationOwner::EvidencePublication,
        3 => HotCoreAllocationOwner::InterpreterConstruction,
        4 => HotCoreAllocationOwner::InterpreterBorrow,
        5 => HotCoreAllocationOwner::ColdMaterialization,
        6 => HotCoreAllocationOwner::AttemptScratch,
        7 => HotCoreAllocationOwner::GenerationBoundary,
        8 => HotCoreAllocationOwner::ArenaGrowth,
        _ => return None,
    };
    Some(HotCoreAllocationTrace {
        owner,
        requested_bytes: entry.requested_bytes,
    })
}

pub fn record_hot_core_phase(phase: HotCorePhase) {
    PHASE_BOUNDARIES[phase.index()].fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_episode(operations: usize, reason: HotCoreStopReason) {
    EPISODE_LENGTHS[operations.min(256)].fetch_add(1, Ordering::Relaxed);
    STOP_REASONS[reason.index()].fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_command_family(family: HotCoreCommandFamily) {
    COMMAND_FAMILIES[family.index()].fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_expandable_opcode(operand: usize) {
    assert!(operand < HOT_CORE_EXPANDABLE_OPCODE_COUNT);
    EXPANDABLE_OPCODES[operand].fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_macro_expansion() {
    MACRO_EXPANSIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_unexpandable_opcode(operand: usize) {
    assert!(operand < HOT_CORE_UNEXPANDABLE_OPCODE_COUNT);
    UNEXPANDABLE_OPCODES[operand].fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_materialization(materialization: HotCoreMaterialization) {
    MATERIALIZATIONS[materialization.index()].fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_interpreter_construction() {
    INTERPRETER_CONSTRUCTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_hot_core_interpreter_operation_entry() {
    INTERPRETER_OPERATION_ENTRIES.fetch_add(1, Ordering::Relaxed);
}

pub fn record_retained_generation_retirement() {
    RETAINED_GENERATIONS_RETIRED.fetch_add(1, Ordering::Relaxed);
}

#[must_use]
pub fn retained_generation_census() -> RetainedGenerationCensus {
    RetainedGenerationCensus {
        created: RETAINED_GENERATIONS_CREATED.load(Ordering::Relaxed),
        dropped: RETAINED_GENERATIONS_DROPPED.load(Ordering::Relaxed),
        live: RETAINED_GENERATIONS_LIVE.load(Ordering::Relaxed),
        peak_live: RETAINED_GENERATIONS_PEAK.load(Ordering::Relaxed),
        retired_explicitly: RETAINED_GENERATIONS_RETIRED.load(Ordering::Relaxed),
    }
}

#[must_use]
pub fn hot_core_census() -> HotCoreCensus {
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
        command_families: core::array::from_fn(|index| {
            COMMAND_FAMILIES[index].load(Ordering::Relaxed)
        }),
        expandable_opcodes: core::array::from_fn(|index| {
            EXPANDABLE_OPCODES[index].load(Ordering::Relaxed)
        }),
        macro_expansions: MACRO_EXPANSIONS.load(Ordering::Relaxed),
        unexpandable_opcodes: core::array::from_fn(|index| {
            UNEXPANDABLE_OPCODES[index].load(Ordering::Relaxed)
        }),
        materializations: core::array::from_fn(|index| {
            MATERIALIZATIONS[index].load(Ordering::Relaxed)
        }),
        interpreter_constructions: INTERPRETER_CONSTRUCTIONS.load(Ordering::Relaxed),
        interpreter_operation_entries: INTERPRETER_OPERATION_ENTRIES.load(Ordering::Relaxed),
        phase_boundaries: core::array::from_fn(|index| {
            PHASE_BOUNDARIES[index].load(Ordering::Relaxed)
        }),
    }
}
