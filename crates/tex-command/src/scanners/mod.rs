//! Private typed scanner family.

mod font;
mod scalar;
mod structured;
mod token_list;

pub use scalar::{InternalValue, ScalarProvenance, ScalarRecovery, ScannedScalar};
pub use structured::{
    AlignmentCellOpening, CanonicalMathRequest, EquationNumberSide, FileNameTermination,
    FontLoadRequest, ImmediateExtension, InputStreamRequest, MathFamilySize, MathFractionKind,
    MathLimitKind, MathScriptKind, MathStyleKind, MathTextFieldKind, RegisteredInput,
    ScannedAccent, ScannedAccentBase, ScannedBalancedText, ScannedBoxConstruction, ScannedBoxKind,
    ScannedBoxRegister, ScannedDiscretionary, ScannedDisplayDiagnostic, ScannedEquationNumber,
    ScannedFileName, ScannedGlueParameterAssignment, ScannedLeaderPayload, ScannedLetAssignment,
    ScannedMacroDefinition, ScannedMathCharacter, ScannedMathDelimiter, ScannedMathFamily,
    ScannedMathFraction, ScannedMathMuMaterial, ScannedMathScript, ScannedPackingSpec,
    ScannedRuleSpec, ScannedSetBoxAssignment, ScannedVSplit, StructuredProvenance,
};
pub use token_list::ScannedTokenRegisterAssignment;
