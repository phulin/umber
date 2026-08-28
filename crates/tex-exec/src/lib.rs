//! TeX stomach and main-control execution.
//!
//! This crate consumes completed commands from `tex-command`; it does not own
//! raw token delivery or expansion.

#![forbid(unsafe_code)]

mod align;
mod assignments;
mod box_runtime;
mod canonical_step;
mod checkpoint;
mod diagnostics;
mod dispatch;
mod effective_tail;
mod engine_completion;
mod episode;
mod error;
mod error_report;
mod execution_receipt;
mod font_support;
mod geometry;
mod host_api;
mod interpreter;
mod job;
mod job_output;
mod main_control;
mod math;
mod mode;
mod node_dump;
mod output_provenance;
mod pack_report;
mod packing_params;
mod page_builder;
mod page_output;
mod paragraph_end;
mod retained_generation;
mod retained_resource;
mod session_api;
mod shipout;
mod splitting;
mod timing;
pub mod transaction_protocol;
pub(crate) mod typeset_context;
mod vertical;

#[cfg(test)]
mod test_harness;

#[cfg(feature = "profiling")]
pub use align::{AlignmentTemplateMeasurement, alignment_template_measurement};
pub use assignments::{
    install_etex_unexpandable_primitives, install_unexpandable_primitives,
    register_etex_unexpandable_primitives, register_unexpandable_primitives,
};
pub use canonical_step::{
    CanonicalStepFailure, CanonicalStepResult, CanonicalStepRunner, OutputLedger,
    TerminalRevisionReceipt,
};
pub use checkpoint::{
    CheckpointOwnerCharge, CheckpointOwnerFamily, CheckpointOwnerKey, CheckpointRestoreError,
    CheckpointRetention, CheckpointSink, ENGINE_CHECKPOINT_SCHEMA_VERSION, EngineBoundary,
    EngineCheckpoint, EngineCheckpointRelease, REACHABLE_STATE_IDENTITY_SCHEMA_VERSION,
    ReachableStateIdentity,
};
pub use dispatch::{
    ArtifactLedger, DispatchAction, ExecutionStats, PreparedDviPage, RevisionOutputPatch,
    RevisionOutputPatchError,
};
pub use engine_completion::{
    CommittedEnginePublication, CompletionEffectOrdinal, CompletionPublication,
    CompletionPublicationFailure, DetachedEngineCompletion, DetachedPreparedPage,
    EngineCompletionDemand, EngineCompletionError, EnginePublicationError,
    PreparedEnginePublication, PublishedEnginePage,
};
pub use episode::{EpisodeCommit, EpisodeCommitBoundary, EpisodeTelemetry, SemanticEpisodeBarrier};
pub use error::{
    ExecError, FrozenDiagnosticContext, FrozenDiagnosticEvidence, FrozenDiagnosticGroup,
    FrozenDiagnosticOrigin,
};
pub use host_api::{
    FontResolver, FontSource, PdfImagePageBox, PdfImagePageSelection, PdfImageRequest,
    PdfImageResolver, ResolverResourceNeed, ResourceLookup, ResourceResult,
};
pub use job::{
    BANNER, DetachedFormatDump, DviJobOutput, ETEX26_BANNER, EngineBinaryIdentity, FormatDumpError,
    FormatDumpReceipt, PdfJobFinalizationReport, PreloadedFormat, TEX82_BANNER,
    confirm_format_dump_publication,
};
pub use main_control::{
    AdvanceOutcome, AdvanceReadiness, AdvanceTelemetry, DiagnosticStep, DiagnosticStepResult,
    MainControl, MainControlStep, PreparedCheckpointControl, ResourceNeed, RootCompletionPolicy,
    StepResult, TrackedStepResult,
};
pub use mode::{
    AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec, Mode, ModeLevelSummary, ModeList,
    ModeNest, ModeNestSummary,
};
pub use paragraph_end::cached_pretolerance_plan;
pub use retained_generation::{
    AdmittedEngineGeneration, BoundaryLaneStorage, CheckpointPruningReceipt,
    RestoredCheckpointRuntime, RetainedBoundaryEvidence, RetainedCheckpointKey,
    RetainedCheckpointStore, RetainedEngineAccessError, RetainedEngineAttachmentKey,
    RetainedEngineForkError, RetainedEngineGeneration, RetainedEngineOperation,
    RetainedEngineRetirement,
};
pub use retained_resource::{
    ResourceFulfillment, ResourceHost, ResourceOutcome, ResourceWorld, canonical_font_resource_path,
};
pub use session_api::{
    Cancellation, ExecutionBudgetCounters, ExecutionBudgets, ExecutionTelemetry, PendingInterrupt,
};
pub use shipout::retry_unavailable_stream_open;
