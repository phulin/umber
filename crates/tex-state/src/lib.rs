//! Core TeX state layer. See `docs/core_state.md` for the design.

pub mod cell;
pub mod code_tables;
pub mod env;
pub mod epoch;
pub mod font;
pub mod glue;
pub mod hyphenation;
pub mod ids;
pub mod input;
pub mod interner;
pub(crate) mod journal;
pub mod macro_store;
pub mod math;
pub mod meaning;
#[cfg(feature = "node-stats")]
pub mod measurement;
pub mod node;
pub mod node_arena;
pub mod page;
pub mod provenance;
mod provenance_resolver;
pub mod scaled;
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
pub use provenance_resolver::ProvenanceResolver;
pub use stores::{FontParameterError, GroupKind, GroupMismatch, PrepareMagDiagnostic};
pub use universe::{
    BoxDimension, CheckpointId, CheckpointMetadata, CheckpointResumeKind, ExpansionContext,
    ExpansionState, FormatError, InputOpenContext, InputOpenState, InputReadState, InteractionMode,
    ParagraphShapeLine, ResumeBoundary, ResumeFallback, Snapshot, Universe,
};
pub use world::{
    ContentHash, EffectPos, EffectRecord, FileContent, InputRecord, InputRecordId, JobClock,
    PrintSink, ReadTarget, RngState, ShellEscapePolicy, ShellEscapeRecord, StreamBufState,
    StreamSlot, World, WorldError, WorldSnapshot,
};

#[cfg(test)]
mod tests;
