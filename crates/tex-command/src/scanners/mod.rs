//! Private typed scanner family.

mod font;
mod hyphenation;
mod restricted;
mod scalar;
mod structured;
mod token_list;

pub use hyphenation::{HyphenationDataKind, ScannedHyphenationData};
pub use restricted::{RestrictedInteger, RestrictedIntegerClass, RestrictedIntegerRecovery};
pub use scalar::{InternalValue, ScalarProvenance, ScalarRecovery, ScannedScalar};
pub use structured::{
    AlignmentCellOpening, CanonicalMathRequest, EquationNumberSide, FileNameTermination,
    FontLoadRequest, FontSizeRecovery, ImmediateExtension, InputStreamRequest,
    MathDelimiterBoundary, MathDelimiterBoundaryKind, MathFamilySize, MathFieldBody,
    MathFieldEpisode, MathFractionKind, MathLimitKind, MathScriptAttachment, MathScriptKind,
    MathStyleKind, MathTextFieldKind, PdfAnnotationRequest, PdfColorStackActionRequest,
    PdfDestinationRequest, PdfDocumentFragmentRequest, PdfFormRequest, PdfGraphicsRequest,
    PdfImagePageBox, PdfImageRequest, PdfNavigationRequest, PdfObjectRequest,
    PdfReferenceObjectRequest, PdfStartLinkRequest, PdfThreadRequest, RegisteredInput,
    ScannedAccent, ScannedAccentBase, ScannedBalancedText, ScannedBoxConstruction, ScannedBoxKind,
    ScannedBoxRegister, ScannedBoxShift, ScannedBoxShiftPayload, ScannedCharacterDefinition,
    ScannedDiscretionary, ScannedDisplayDiagnostic, ScannedEquationNumber, ScannedFileName,
    ScannedGlueParameterAssignment, ScannedInsertConstruction, ScannedLeaderPayload,
    ScannedLetAssignment, ScannedMacroDefinition, ScannedMathCharacter, ScannedMathDelimiter,
    ScannedMathFamily, ScannedMathFraction, ScannedMathMuMaterial, ScannedMathScript,
    ScannedPackingSpec, ScannedRegisterDefinition, ScannedRuleSpec, ScannedSetBoxAssignment,
    ScannedVSplit, StructuredProvenance,
};
pub use token_list::ScannedTokenRegisterAssignment;
