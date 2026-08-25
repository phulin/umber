//! Profiling-only, process-local measurements for the final runtime.
//!
//! Normal builds do not compile this module. These counters never participate
//! in engine state, snapshots, rollback, formats, or semantic identity.

mod hot_core;

pub use hot_core::{
    HOT_CORE_EXPANDABLE_OPCODE_COUNT, HOT_CORE_UNEXPANDABLE_OPCODE_COUNT,
    HotCoreAllocationMeasurement, HotCoreAllocationOwner, HotCoreAllocationTrace, HotCoreAllocator,
    HotCoreCensus, HotCoreCommandFamily, HotCoreMaterialization, HotCorePhase, HotCoreStopReason,
    RetainedGenerationCensus, RetainedGenerationLifetime, SaveJournalCensus,
    hot_core_allocation_scope, hot_core_allocation_trace_cursor, hot_core_allocation_trace_entry,
    hot_core_census, hot_core_thread_allocation_measurement, record_hot_core_command_family,
    record_hot_core_episode, record_hot_core_expandable_opcode,
    record_hot_core_interpreter_construction, record_hot_core_interpreter_operation_entry,
    record_hot_core_macro_expansion, record_hot_core_materialization, record_hot_core_phase,
    record_hot_core_unexpandable_opcode, record_retained_generation_retirement,
    retained_generation_census, save_journal_census,
};

pub(crate) use hot_core::record_save_journal_census;
