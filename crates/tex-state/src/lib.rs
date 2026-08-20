//! Core TeX state layer. See `docs/core_state.md` for the design.

/// Version of the schedule-relative checkpoint hash framing.
///
/// Version 3 introduced canonical frozen node-list identities and shallow
/// node-root composition. Version 4 encodes RNG state as canonical numeric
/// words. Version 5 orders changed cells by a cached canonical key fingerprint
/// with full-key collision fallback. Version 6 frames the six code tables as
/// independent canonical projections so unchanged persistent roots can be
/// reused. Version 7 frames mutable page-tail nodes independently so
/// checkpoint-local caches can reuse an unchanged prefix. Version 8 replaces
/// the per-field full avalanche with a faster ordered streaming recurrence and
/// retains the same strong final avalanche. Version 9 makes absolute editor-root
/// source coordinates revision-mapping metadata while retaining normalized-line
/// cursor state in the semantic projection. Version 10 widens font-parameter
/// counts and fontdimen slots to the 17-bit domain used by LaTeX's font-backed
/// integer arrays. Version 11 adds by-value transient input replay frames and
/// packed macro arguments, hashing their semantic token sequences without
/// diagnostic origins.
/// Version 12 adds the pdfTeX document ledger and committed page/object
/// identities to checkpoint hashing. Version 13 adds the output controls
/// frozen by the first shipped page. Version 14 adds pdfTeX's mutable
/// per-font character-code and ligature-suppression state. Version 15 adds
/// checkpointed PDF font resource and indirect-object identities. Version 16
/// adds typed external-image metadata used by pdfTeX page-box enquiries.
/// Version 17 adds reserved and initialized raw PDF object records.
/// Version 18 adds document dictionary/trailer fragments and their final
/// object identities. Version 19 adds typed catalog actions and forward page
/// reservations.
/// Version 20 aligns canonical PDF allocation order with pdfTeX, including
/// user objects, pages, and final document dictionaries.
/// Version 21 adds checkpointed pdfTeX color-stack allocation and traversal
/// state. Version 22 adds saved-position enquiries and snapping reference
/// coordinates. Version 23 adds pdfTeX's session-global return value. Version
/// 24 adds total-page and color-depth facts to external-image metadata.
/// Version 25 adopts node semantic-identity v3, whose canonical node-list
/// stream groups maximal same-font character runs.
/// Version 26 includes token-parameter presence, distinguishing an absent
/// parameter cell from a present cell whose value is the empty token list.
/// Version 27 adopts node semantic-identity v4, which includes merged e-TeX
/// WEB §53a's three-state `box_lr` identity.
/// Version 28 includes the left- and right-boundary-hit vocabulary carried by
/// ligature nodes.
/// Hashes are
/// comparable only when both this version and the named-boundary schedule
/// match.
pub const CHECKPOINT_STATE_HASH_SCHEMA_VERSION: u32 = 28;

pub mod cell;
pub mod code_tables;
mod command_context;
pub(crate) mod definition_arena;
pub mod dependency;
pub mod diagnostic;
pub(crate) mod durable_arena;
mod effect_journal;
mod engine_state;
pub mod env;
pub mod epoch;
pub mod etex_tracing;
mod expansion_diagnostic;
mod expansion_recovery;
pub mod file_framing;
pub mod font;
mod format_container;
mod frozen_lookup;
pub(crate) mod generation;
pub mod glue;
#[allow(dead_code)] // Storage substrate is consumed as HotCore families migrate.
pub(crate) mod hot_core;
pub mod hyphenation;
pub(crate) mod identity;
pub mod ids;
pub mod input;
pub mod interner;
pub(crate) mod journal;
pub mod macro_definition;
pub mod math;
pub mod meaning;
pub mod memo;
pub mod node;
pub mod node_arena;
pub mod node_sequence;
pub mod page;
mod pdf;
pub mod print;
pub mod provenance;
mod provenance_resolver;
mod pure_memo;
mod read_observation;
mod resource;
pub mod scaled;
pub mod source_fragments;
pub mod source_map;
pub(crate) mod state_hash;
mod stores;
pub mod token;
pub mod token_show;
mod universe;

pub use pdf::{
    PdfActionDestination, PdfActionIdentifier, PdfActionRecord, PdfActionSpec, PdfActionTarget,
    PdfActionWindow, PdfAnnotationData, PdfAnnotationDimensions, PdfAnnotationInitializeError,
    PdfAnnotationRecord, PdfColorStackAction, PdfColorStackApplyError, PdfColorStackCapacityError,
    PdfColorStackEmission, PdfColorStackMode, PdfColorStackTarget, PdfDestinationDefinition,
    PdfDestinationIdentity, PdfDestinationRecord, PdfDocumentFragmentKind, PdfDocumentObjectIds,
    PdfExternalImageDimensions, PdfExternalImageId, PdfExternalImageIdError,
    PdfExternalImageMetadata, PdfExternalImageRecord, PdfExternalImageRegistrationError,
    PdfExternalImageSource, PdfFontConfiguration, PdfFontMapOperation, PdfFontResourceRecord,
    PdfFormArtifact, PdfFormColorRollback, PdfFormRecord, PdfGlyphToUnicode, PdfLinkRecord,
    PdfObjectCapacityError, PdfOpenLink, PdfOutlineRecord, PdfOutputParameters, PdfPageBox,
    PdfPageGroupInclusion, PdfPageGroupSelector, PdfPageGroupWarning, PdfPageRecord,
    PdfPageRotation, PdfRasterColorSpace, PdfRasterFormat, PdfRasterImageMetadata,
    PdfRawObjectData, PdfRawObjectId, PdfRawObjectInitializeError, PdfRawObjectRecord,
    PdfThreadBeadRecord, PdfThreadRecord,
};
pub mod world;

pub use expansion_diagnostic::RecoverableExpansionDiagnostic;
pub use expansion_recovery::ExpansionRecovery;
pub use generation::GenerationOwner;
pub use read_observation::{ReadRecorder, ReadRecorderBatch, ReadSetRecorder};
pub use resource::{
    InputOpenContext, InputOpenState, InputReadState, InputResolver, ResourceLookup, ResourceNeed,
    ResourceResult,
};

pub use command_context::CommandContext;
pub use definition_arena::{DefinitionAllocationError, DefinitionId, DefinitionView};
pub use dependency::{
    ChangedAt, DependencyCodeTable, DependencyEngineField, DependencyFontField, DependencyKey,
    DependencyPageField, DependencyRegion, DependencyRegionError, DependencyRegionToken,
    DependencyRuntime, DependencyTracker, DependencyValidation, DependencyValue,
    DependencyWorldField, ObservedDependency, TrackedRegionBarrier,
};
pub use durable_arena::{DurableAllocationError, GlueId, ProvenanceId, TokenListId};
pub use effect_journal::EffectJournal;
pub use engine_state::{EngineMode, EngineStateSnapshot};
pub use env::group::{GroupFrame, GroupKind, GroupMismatch};
pub use env::{AssignmentScope, CodeTableKind, StateError};
pub use font::PdfFontCode;
pub use generation::GenerationBrand;
pub use input::{
    AlignmentScannerPhase, ConditionFrameSummary, ConditionFrameToken, ConditionKind,
    ConditionLimb, InputFrameSummary, InputSummary, LexerState, LiteralSpanPolicy,
    MACRO_ARGUMENT_SLOTS, MacroArgumentRange, MacroArguments, MacroReplaySite, SourceFrameSummary,
    SourceId, TokenListReplayKind, TokenListReplayMarker, TracedExpansionToken, TracedTokenList,
};
pub use journal::JournalCursor;
pub use meaning::{MeaningWord, ResolvedMeaning};
pub use memo::{
    DetachedArtifact, DetachedDiagnostic, DetachedInputTransition, DetachedMemoValue,
    DetachedPageTransition, DetachedPureKernelPlan, DetachedVirtualEffect,
    MEMO_VALUE_SCHEMA_VERSION, MemoValueError, MemoValueKind, MemoValueLimits,
};
pub use page::{
    AWFUL_BAD, DEPLORABLE, EJECT_PENALTY, INF_PENALTY, PageBreak, PageContents, PageDimension,
    PageFireUp, PageInteger,
};
pub use provenance::{ProvenanceBudgets, ProvenanceDemand};
pub use provenance_resolver::{
    DetachedGeneratedSourceSpan, ProvenanceResolver, ResolvedSourceLocation,
};
pub use pure_memo::{
    MemoLayerStats, MemoTimingPhase, OutputProvenanceRecipe, OutputProvenanceSpan,
    PureBreakDecision, PureBreakMemoryEvent, PureBreakMemoryOwner, PureBreakMemoryPlan,
    PureBreakPlan, PureMemoConfig, PureMemoKey, PureMemoLayer, PureMemoRecordingPolicy,
    PureMemoRuntime, PureMemoStats, PurePageEntry, PureShipoutEntry,
};
pub use source_fragments::{
    EditorLayout, EditorLayoutError, FragmentId, FragmentStore, LayoutGeneration,
    LayoutResolvedOrigin, Piece, PieceId, RootSpanId,
};
pub use universe::{
    DefinitionPromotion, InteractionMode, PromotionError, PromotionReceipt, TokenListPromotion,
    Universe, UniverseError, UniverseRetirement, with_universe,
};
#[cfg(feature = "profiling")]
pub use world::ProfilingTimer;
pub use world::{
    ArtifactOrigin, ArtifactPublicationId, ArtifactPublicationRecord,
    ArtifactPublicationReservation, CommittedArtifact, ContentDomain, ContentHash, ContentIdentity,
    EffectDomain, EffectOutputAttemptId, EffectPlacementIntraOrder, EffectPos,
    EffectPublicationCandidate, EffectPublicationDisposition, EffectPublicationId,
    EffectPublicationRecordOrdinal, EffectRecord, EffectRetrySafety, EffectRootIdentity,
    EffectSemanticRecordOrdinal, EffectSequence, ExecutionTraceEvent, FileContent,
    FileModificationDate, InputDependency, InputDependencyAccess, InputDependencyOutcome,
    InputOrigin, InputRecord, InputRecordId, JobClock, MAX_INPUT_DEPENDENCIES,
    MemoryMaterializationCheckpoint, MemoryOutput, PageOutputPublicationReceipt,
    PageOutputPublicationReceiptId, PrintSink, ReadTarget, RenderOriginIter, RenderOrigins,
    RenderProvenanceBuilder, RetainedOutputOpenOutcome, RngState, ShellEscapePolicy,
    ShellEscapeRecord, StreamBufState, StreamOpenFailure, StreamSlot, TerminalInputPosition,
    TerminalPublicationId, TerminalPublicationPhase, VerifiedArtifact, World, WorldCommitMode,
    WorldError, WorldSnapshot,
};

#[cfg(test)]
mod tests;
