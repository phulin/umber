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
pub mod meaning;
pub mod node;
pub mod node_arena;
pub mod scaled;
pub(crate) mod state_hash;
mod stores;
pub mod survivor;
pub mod token;
pub mod token_store;
mod universe;
pub mod world;

pub use input::{
    ConditionFrameSummary, ConditionKind, ConditionLimb, InputFrameSummary, InputSummary,
    LexerState, MACRO_ARGUMENT_SLOTS, MacroArguments, SourceFrameSummary, SourceId,
    TokenListReplayKind,
};
pub use stores::{FontParameterError, GroupKind, GroupMismatch, PrepareMagDiagnostic};
pub use universe::{
    BoxDimension, ExpansionContext, ExpansionState, InputOpenContext, InputOpenState,
    InputReadState, InteractionMode, Snapshot, Universe,
};
pub use world::{
    ContentHash, EffectPos, EffectRecord, FileContent, InputRecord, JobClock, PrintSink,
    ReadTarget, RngState, ShellEscapePolicy, ShellEscapeRecord, StreamBufState, StreamSlot, World,
    WorldError, WorldSnapshot,
};

#[cfg(test)]
mod tests {
    use crate::Universe;
    use crate::hyphenation::{ExceptionSpec, PatternSpec};

    #[test]
    fn smoke() {
        assert!(!env!("CARGO_PKG_NAME").is_empty());
    }

    #[test]
    fn hyphenation_state_rolls_back_with_snapshots() {
        let mut universe = Universe::new();
        universe.add_hyphenation_exception(ExceptionSpec {
            word: "before".to_owned(),
            positions: vec![2],
        });
        let snapshot = universe.snapshot();
        universe.add_hyphenation_exception(ExceptionSpec {
            word: "after".to_owned(),
            positions: vec![3],
        });
        universe.add_hyphenation_pattern(PatternSpec {
            letters: "after".chars().collect(),
            values: vec![0, 0, 1, 0, 0, 0],
        });

        assert_eq!(universe.hyphen_positions("after", 1, 1), vec![3]);
        universe.rollback(&snapshot);
        assert_eq!(universe.hyphen_positions("before", 1, 1), vec![2]);
        assert!(universe.hyphen_positions("after", 1, 1).is_empty());
    }
}
