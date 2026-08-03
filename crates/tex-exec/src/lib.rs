//! Canonical TeX stomach and main-control execution.
//!
//! This crate consumes completed commands from `tex-command`; it does not own
//! raw token delivery or expansion.

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    reason = "canonical kernels are exercised through higher-level command fixtures; retired unit-only callers were removed with the compatibility graph"
)]

mod align;
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
mod host_api;
mod job;
mod job_output;
mod math;
mod mode;
mod node_dump;
mod pack_report;
mod packing_params;
mod page_builder;
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
    CanonicalCheckpointRestoreError, CanonicalEditorFork, CheckpointSink,
    ENGINE_CHECKPOINT_SCHEMA_VERSION, EditorRestoreError, EngineBoundary, EngineCheckpoint,
    RootRehomeContext,
};
pub use dispatch::{DispatchAction, ExecutionStats, PreparedDviPage};
pub use error::{ExecError, FrozenDiagnosticOrigin};
pub use host_api::{
    FontResolver, FontSource, PdfImagePageBox, PdfImagePageSelection, PdfImageRequest,
    PdfImageResolver, ResourceLookup, ResourceNeed, ResourceResult,
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
pub use session_api::{
    Cancellation, ExecutionBudgetCounters, ExecutionBudgets, ExecutionTelemetry, PendingInterrupt,
};

#[cfg(any())]
mod tests;
