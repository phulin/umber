//! Private typed scanner family.

mod font;
mod scalar;
mod structured;
mod token_list;

pub use scalar::{InternalValue, ScalarProvenance, ScalarRecovery, ScannedScalar};
pub use structured::{
    AlignmentCellOpening, FileNameTermination, FontLoadRequest, ImmediateExtension,
    InputStreamRequest, RegisteredInput, ScannedAccent, ScannedAccentBase, ScannedBalancedText,
    ScannedBoxConstruction, ScannedBoxKind, ScannedBoxRegister, ScannedDiscretionary,
    ScannedFileName, ScannedGlueParameterAssignment, ScannedLetAssignment, ScannedMacroDefinition,
    ScannedPackingSpec, ScannedRuleSpec, ScannedSetBoxAssignment, StructuredProvenance,
};
pub use token_list::ScannedTokenRegisterAssignment;
