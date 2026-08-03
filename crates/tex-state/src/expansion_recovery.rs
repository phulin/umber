use crate::token::{Token, TracedTokenWord};

/// Recoverable expansion failures consumed by legacy main-control loops.
///
/// This detached vocabulary prevents execution from depending on the retired
/// expansion engine's recursive error representation.
#[derive(Debug)]
pub enum ExpansionRecovery {
    UndefinedControlSequence,
    ExtraConditionalControl {
        name: &'static str,
    },
    InvalidCharacter,
    MacroDoesNotMatch {
        macro_name: String,
    },
    ParagraphEndedBeforeComplete {
        macro_name: String,
        context: TracedTokenWord,
        partial: Vec<Token>,
    },
    ForbiddenOuterToken {
        macro_name: String,
        context: TracedTokenWord,
        partial: Vec<Token>,
    },
}
