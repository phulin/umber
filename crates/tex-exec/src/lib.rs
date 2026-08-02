//! TeX execution engine scaffold.
//!
//! This crate owns the stomach's mode nest and main-control dispatch. It pulls
//! only fully expanded tokens from `tex_expand::get_x_token*`; raw token reads
//! stay in the lexer/gullet pipeline.

#![forbid(unsafe_code)]

mod align;
mod assignments;
mod canonical_main_control;
mod checkpoint;
mod diagnostics;
mod dispatch;
mod effective_tail;
mod error;
mod error_report;
mod executor;
mod job;
mod math;
mod mode;
mod node_dump;
mod output;
mod pack_report;
mod packing_params;
mod page_builder;
mod paragraph_memo;
mod splitting;
mod timing;
mod transaction;
mod vertical;

#[cfg(feature = "profiling")]
pub use align::{AlignmentTemplateMeasurement, alignment_template_measurement};

pub use assignments::{
    cached_pretolerance_plan, install_etex_unexpandable_primitives,
    install_unexpandable_primitives, register_etex_unexpandable_primitives,
    register_unexpandable_primitives, retry_unavailable_stream_open, try_execute_assignment,
};
pub use canonical_main_control::{
    CanonicalDiagnosticStep, CanonicalDiagnosticStepResult, CanonicalMainControl,
    CanonicalResourceNeed, CanonicalStepResult, MainControlStep,
};
pub use checkpoint::{
    CanonicalCheckpointRestoreError, CheckpointSink, ENGINE_CHECKPOINT_SCHEMA_VERSION,
    EditorRestoreError, EngineBoundary, EngineCheckpoint, EngineRestoreError, RootRehomeContext,
};
pub use dispatch::{DispatchAction, ExecutionStats, PreparedDviPage, dispatch_delivered_token};
pub use error::ExecError;
pub use executor::{
    Cancellation, ExecutionBudgetCounters, ExecutionBudgets, ExecutionContext, ExecutionLifecycle,
    ExecutionProgress, ExecutionRun, ExecutionServices, ExecutionState, ExecutionStep,
    ExecutionStepResult, ExecutionTelemetry, Executor, FontResolver, FontSource, PdfImagePageBox,
    PdfImagePageSelection, PdfImageRequest, PdfImageResolver, PendingInterrupt, ResourceSite,
    ResourceSuspension,
};
pub use job::{
    BANNER, DviJobOutput, ETEX26_BANNER, EngineBinaryIdentity, FormatDumpReceipt,
    PdfJobFinalizationReport, PreloadedFormat, TEX82_BANNER, confirm_format_dump_publication,
};
pub use mode::{
    AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec, Mode, ModeLevelSummary, ModeList,
    ModeNest, ModeNestSummary,
};
pub use tex_expand::{ResourceLookup, ResourceNeed, ResourceResult};

pub(crate) use dispatch::{
    insert_traced_tokens, leave_group, leave_group_with_origin, push_tokens, push_traced_tokens,
};

#[cfg(test)]
mod test_harness;
#[cfg(test)]
mod tests;
