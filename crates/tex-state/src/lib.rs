//! Core TeX state layer. See `docs/core_state.md` for the design.

pub mod cell;
pub mod code_tables;
pub mod env;
pub mod epoch;
pub mod glue;
pub mod ids;
pub mod interner;
pub(crate) mod journal;
pub mod macro_store;
pub mod meaning;
pub mod node;
pub mod node_arena;
pub mod scaled;
mod stores;
pub mod survivor;
pub mod token;
pub mod token_store;
mod universe;
pub mod world;

pub use stores::{GroupKind, GroupMismatch, PrepareMagDiagnostic};
pub use universe::{InputSummary, InteractionMode, Snapshot, Universe};
pub use world::{
    ContentHash, EffectPos, EffectRecord, FileContent, InputRecord, JobClock, PrintSink, RngState,
    ShellEscapePolicy, ShellEscapeRecord, StreamBufState, StreamSlot, World, WorldError,
    WorldSnapshot,
};

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(!env!("CARGO_PKG_NAME").is_empty());
    }
}
