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
/// hashes their remaining semantic token sequence without diagnostic origins.
/// Hashes are
/// comparable only when both this version and the named-boundary schedule
/// match.
pub const CHECKPOINT_STATE_HASH_SCHEMA_VERSION: u32 = 11;

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
pub mod world;

pub use input::{
    ConditionFrameSummary, ConditionFrameToken, ConditionKind, ConditionLimb, InputFrameSummary,
    InputSummary, LexerState, MACRO_ARGUMENT_SLOTS, MacroArguments, SourceFrameSummary, SourceId,
    TokenListReplayKind, TracedTokenList,
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
    EffectRetrySafety, ExecutionTraceEvent, FileContent, InputRecord, InputRecordId, JobClock,
    MemoryOutput, PrintSink, ReadTarget, RngState, ShellEscapePolicy, ShellEscapeRecord,
    StreamBufState, StreamSlot, VerifiedArtifact, World, WorldCommitMode, WorldError,
    WorldSnapshot,
};

#[cfg(test)]
mod tests;
