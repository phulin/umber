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
mod job_output;
mod math;
mod mode;
mod node_dump;
mod output;
mod pack_report;
mod packing_params;
mod page_builder;
mod paragraph_memo;
mod retained_resource;
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
    CanonicalAdvanceOutcome, CanonicalAdvanceReadiness, CanonicalAdvanceTelemetry,
    CanonicalDiagnosticStep, CanonicalDiagnosticStepResult, CanonicalMainControl,
    CanonicalParagraphRegion, CanonicalResourceNeed, CanonicalStepResult, MainControlStep,
};
pub use checkpoint::{
    CanonicalCheckpointRestoreError, CheckpointSink, ENGINE_CHECKPOINT_SCHEMA_VERSION,
    EditorRestoreError, EngineBoundary, EngineCheckpoint, RootRehomeContext,
};
pub use dispatch::{DispatchAction, ExecutionStats, PreparedDviPage, dispatch_delivered_token};
pub(crate) use dispatch::{
    insert_traced_tokens, leave_group, leave_group_with_origin, push_tokens, push_traced_tokens,
};
pub use error::{ExecError, FrozenDiagnosticOrigin};
pub use executor::{
    Cancellation, ExecutionBudgetCounters, ExecutionBudgets, ExecutionContext, ExecutionLifecycle,
    ExecutionProgress, ExecutionRun, ExecutionServices, ExecutionState, ExecutionStep,
    ExecutionStepResult, ExecutionTelemetry, Executor, FontResolver, FontSource, PdfImagePageBox,
    PdfImagePageSelection, PdfImageRequest, PdfImageResolver, PendingInterrupt, ResourceLookup,
    ResourceNeed, ResourceResult, ResourceSite, ResourceSuspension,
};
pub use job::{
    BANNER, DviJobOutput, ETEX26_BANNER, EngineBinaryIdentity, FormatDumpReceipt,
    PdfJobFinalizationReport, PreloadedFormat, TEX82_BANNER, confirm_format_dump_publication,
};
pub use mode::{
    AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec, Mode, ModeLevelSummary, ModeList,
    ModeNest, ModeNestSummary,
};
pub use retained_resource::{
    CanonicalResourceFulfillment, CanonicalResourceHost, CanonicalResourceOutcome,
    CanonicalResourceWorld, canonical_font_resource_path,
};

#[cfg(test)]
mod test_harness;
#[cfg(test)]
mod tests;
