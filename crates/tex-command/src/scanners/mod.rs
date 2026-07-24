//! Private typed scanner family.

mod font;
mod scalar;
mod structured;
mod token_list;

pub use scalar::{InternalValue, ScalarProvenance, ScalarRecovery, ScannedScalar};
pub use structured::{
    AlignmentCellOpening, FileNameTermination, RegisteredInput, ScannedBalancedText,
    ScannedFileName, ScannedGlueParameterAssignment, ScannedLetAssignment, ScannedMacroDefinition,
    ScannedRuleSpec, ScannedSetBoxAssignment, StructuredProvenance,
};
pub use token_list::ScannedTokenRegisterAssignment;
