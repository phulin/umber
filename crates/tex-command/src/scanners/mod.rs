//! Private typed scanner family.

mod font;
mod scalar;
mod structured;
mod token_list;

pub use scalar::{InternalValue, ScalarProvenance, ScalarRecovery, ScannedScalar};
pub use structured::{
    FileNameTermination, RegisteredInput, ScannedBalancedText, ScannedFileName,
    ScannedLetAssignment, ScannedMacroDefinition, StructuredProvenance,
};
pub use token_list::ScannedTokenRegisterAssignment;
