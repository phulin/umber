//! TeX execution engine scaffold.
//!
//! This crate owns the stomach's mode nest and main-control dispatch. It pulls
//! only fully expanded tokens from `tex_expand::get_x_token*`; raw token reads
//! stay in the lexer/gullet pipeline.

#![forbid(unsafe_code)]

mod align;
#[cfg(test)]
mod assignments;
mod canonical_assignments;
mod canonical_box_runtime;
mod canonical_diagnostics;
mod canonical_font_support;
mod canonical_main_control;
mod canonical_page_output;
mod canonical_paragraph_end;
mod canonical_paragraph_memo;
mod canonical_shipout;
mod checkpoint;
mod dispatch;
mod effective_tail;
mod error;
mod error_report;
#[cfg(test)]
mod executor;
mod host_api;
mod job;
mod job_output;
#[cfg(test)]
mod legacy_assignments;
#[cfg(test)]
mod legacy_diagnostics;
#[cfg(test)]
mod legacy_dispatch;
#[cfg(test)]
mod legacy_editor_restart;
#[cfg(test)]
mod legacy_output;
#[cfg(test)]
mod legacy_paragraph_memo;
mod math;
mod mode;
mod node_dump;
mod pack_report;
mod packing_params;
mod page_builder;
#[cfg(test)]
mod raw_delivery;
mod retained_resource;
mod session_api;
mod splitting;
mod timing;
mod transaction;
mod vertical;

use canonical_diagnostics as diagnostics;

#[cfg(feature = "profiling")]
pub use align::{AlignmentTemplateMeasurement, alignment_template_measurement};

pub use canonical_assignments::{
    install_etex_unexpandable_primitives, install_unexpandable_primitives,
    register_etex_unexpandable_primitives, register_unexpandable_primitives,
};
pub use canonical_main_control::{
    CanonicalAdvanceOutcome, CanonicalAdvanceReadiness, CanonicalAdvanceTelemetry,
    CanonicalDiagnosticStep, CanonicalDiagnosticStepResult, CanonicalMainControl,
    CanonicalParagraphRegion, CanonicalResourceNeed, CanonicalStepResult, MainControlStep,
};
pub use canonical_paragraph_end::cached_pretolerance_plan;
pub use canonical_shipout::retry_unavailable_stream_open;
pub use checkpoint::{
    CanonicalCheckpointRestoreError, CheckpointSink, ENGINE_CHECKPOINT_SCHEMA_VERSION,
    EditorRestoreError, EngineBoundary, EngineCheckpoint, RootRehomeContext,
};
pub use dispatch::{DispatchAction, ExecutionStats, PreparedDviPage};
pub use error::{ExecError, FrozenDiagnosticOrigin};
#[cfg(test)]
pub use executor::{
    ExecutionContext, ExecutionLifecycle, ExecutionProgress, ExecutionRun, ExecutionServices,
    ExecutionState, ExecutionStep, ExecutionStepResult, Executor, ResourceSite, ResourceSuspension,
};
pub use host_api::{
    FontResolver, FontSource, PdfImagePageBox, PdfImagePageSelection, PdfImageRequest,
    PdfImageResolver, ResourceLookup, ResourceNeed, ResourceResult,
};
pub use job::{
    BANNER, DviJobOutput, ETEX26_BANNER, EngineBinaryIdentity, FormatDumpReceipt,
    PdfJobFinalizationReport, PreloadedFormat, TEX82_BANNER, confirm_format_dump_publication,
};
#[cfg(test)]
pub use legacy_assignments::try_execute_assignment;
#[cfg(test)]
pub use legacy_dispatch::dispatch_delivered_token;
#[cfg(test)]
pub(crate) use legacy_dispatch::{
    insert_traced_tokens, leave_group, leave_group_with_origin, push_tokens, push_traced_tokens,
};
pub use mode::{
    AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec, Mode, ModeLevelSummary, ModeList,
    ModeNest, ModeNestSummary,
};
pub use retained_resource::{
    CanonicalResourceFulfillment, CanonicalResourceHost, CanonicalResourceOutcome,
    CanonicalResourceWorld, canonical_font_resource_path,
};
pub use session_api::{
    Cancellation, ExecutionBudgetCounters, ExecutionBudgets, ExecutionTelemetry, PendingInterrupt,
};

#[cfg(test)]
mod test_harness;
#[cfg(test)]
mod tests;
