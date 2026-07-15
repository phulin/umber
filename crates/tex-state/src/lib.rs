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
/// coordinates. Version 23 adds pdfTeX's session-global return value.
/// Hashes are
/// comparable only when both this version and the named-boundary schedule
/// match.
pub const CHECKPOINT_STATE_HASH_SCHEMA_VERSION: u32 = 23;

pub mod cell;
pub mod code_tables;
pub mod env;
pub mod epoch;
pub mod font;
pub mod glue;
pub mod hyphenation;
pub(crate) mod identity;
pub mod ids;
pub mod input;
pub mod interner;
pub(crate) mod journal;
pub mod macro_store;
pub mod math;
pub mod meaning;
#[cfg(feature = "profiling-stats")]
pub mod measurement;
pub mod node;
pub mod node_arena;
pub mod page;
mod pdf;
pub mod provenance;
mod provenance_resolver;
pub mod scaled;
pub mod source_fragments;
pub mod source_map;
pub(crate) mod state_hash;
mod stores;
pub mod survivor;
pub mod token;
pub mod token_store;
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
    PdfObjectCapacityError, PdfOpenLink, PdfOutputParameters, PdfPageBox, PdfPageGroupInclusion,
    PdfPageGroupSelector, PdfPageGroupWarning, PdfPageRecord, PdfRasterColorSpace, PdfRasterFormat,
    PdfRasterImageMetadata, PdfRawObjectData, PdfRawObjectId, PdfRawObjectInitializeError,
    PdfRawObjectRecord,
};
pub mod world;

pub use font::PdfFontCode;
pub use input::{
    ConditionFrameSummary, ConditionFrameToken, ConditionKind, ConditionLimb, InputFrameSummary,
    InputSummary, LexerState, MACRO_ARGUMENT_SLOTS, MacroArgumentRange, MacroArguments,
    SourceFrameSummary, SourceId, TokenListReplayKind, TracedTokenList,
};
pub use page::{
    AWFUL_BAD, DEPLORABLE, EJECT_PENALTY, INF_PENALTY, PageBreak, PageContents, PageDimension,
    PageFireUp, PageInteger,
};
pub use provenance_resolver::{ProvenanceResolver, ResolvedSourceLocation};
pub use source_fragments::{
    EditorLayout, EditorLayoutError, FragmentId, FragmentStore, LayoutGeneration,
    LayoutResolvedOrigin, Piece,
};
pub use stores::{FontParameterError, GroupKind, GroupMismatch, PrepareMagDiagnostic};
pub use universe::{
    BoxBuildTransaction, BoxDimension, EngineBoundaryHasher, ExpansionContext, ExpansionState,
    FormatError, GenerationForkError, GenerationSubstrate, InputOpenContext, InputOpenState,
    InputReadState, InteractionMode, MeaningCacheGuard, ParagraphShapeLine, PenaltyArrayKind,
    ShipoutTransaction, Snapshot, TakeUnboxResult, UnboxKind, Universe,
};
pub use world::{
    CommittedArtifact, ContentDomain, ContentHash, ContentIdentity, EffectPos, EffectRecord,
    EffectRetrySafety, ExecutionTraceEvent, FileContent, FileModificationDate, InputRecord,
    InputRecordId, JobClock, MemoryOutput, PrintSink, ReadTarget, RngState, ShellEscapePolicy,
    ShellEscapeRecord, StreamBufState, StreamSlot, VerifiedArtifact, World, WorldCommitMode,
    WorldError, WorldSnapshot,
};

#[cfg(test)]
mod tests;
