//! Canonical TeX command-processing boundary.
//!
//! Semantic state machines are crate-private. The public facade grows only as
//! executor integration requires stable, end-state operations.

mod command;
mod conditionals;
mod error;
mod fatal;
mod host;
mod input;
mod macro_call;
mod observation;
mod primitives;
pub use primitives::exceeds_max_non_prefixed_command;
mod processor;
mod profile;
mod provenance;
mod scan_toks;
mod scanners;
mod snapshot;
mod state;

#[cfg(test)]
mod fixture_replay;

pub use command::{CurrentCommand, DeliveryStamp};
pub use error::CommandError;
pub use fatal::{FATAL_SEVERITY, FatalError};
pub use host::{
    CommandHostCapabilities, CommandHostContext, ConditionalMode, ConditionalState, FontResource,
    LastNodeItem, PdfImageResource,
};
pub use input::{
    CatcodeQueries, InvalidSourceCharacter, LexerState, LineTerminator, MalformedUnicodeRange,
    PhysicalLine, RegisteredSourceKind, SourceCharacter, SourceControlSequenceKind, SourceLocation,
    SourceNameClass, SourceProvenance, SourceRange, SourceRegistration, SourceRegistrationError,
    SourceScalarRange, SourceStepQueries, SourceToken, SourceTokenizationStep,
};
/// The single canonical naming vocabulary shared by every observation
/// producer and transport (`docs/tex_command_core.md` §33.3).
#[cfg(any(test, feature = "instrumentation"))]
pub use observation::canonical_names;
#[cfg(any(test, feature = "instrumentation"))]
pub use observation::{
    AlignmentRecord, CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation,
    CommandObserver, CommandProvenance, ConditionRecord, DiagnosticArgument, DiagnosticRecord,
    EffectRecord, GeometryRecord, InputReason, InputRecord, InputTransition, MacroRecord,
    MutationRecord, ObservedToken, ParameterClass, RecoveryKind, RecoveryRecord, ScannerRecord,
    ScannerStatusRecord, TokenListRecord, parameter_mutation_key,
};
pub use processor::{
    AlignmentCellDelimiter, AlignmentCellTemplates, AlignmentDelivery, AlignmentDeliveryEvent,
    AlignmentIdentity, AlignmentLifecycleError, AlignmentPreamble, AlignmentRequest,
    AlignmentRequestResult, CommandProcessor, FinishedAlignmentCell,
};
pub use profile::{
    CharacterCode, CharacterCodeError, CharacterMode, CommandCapabilities, CommandDialect,
    CommandProfile, CommandProfileBoundary, CommandProfileEncodingError, CommandProfileFingerprint,
    CommandProfileMismatch,
};
pub use scanners::{
    AlignmentCellOpening, CanonicalMathRequest, EquationNumberSide, FileNameTermination,
    FontLoadRequest, FontSizeRecovery, HyphenationDataKind, ImmediateExtension, InputStreamRequest,
    InternalValue, MathDelimiterBoundary, MathDelimiterBoundaryKind, MathFamilySize, MathFieldBody,
    MathFieldEpisode, MathFractionKind, MathLimitKind, MathScriptAttachment, MathScriptKind,
    MathStyleKind, MathTextFieldKind, PdfAnnotationRequest, PdfColorStackActionRequest,
    PdfDestinationRequest, PdfDocumentFragmentRequest, PdfFormRequest, PdfGraphicsRequest,
    PdfImagePageBox, PdfImageRequest, PdfNavigationRequest, PdfObjectRequest,
    PdfReferenceObjectRequest, PdfStartLinkRequest, PdfThreadRequest, RegisteredInput,
    RestrictedInteger, RestrictedIntegerClass, ScalarProvenance, ScalarRecovery, ScannedAccent,
    ScannedAccentBase, ScannedBalancedText, ScannedBoxConstruction, ScannedBoxKind,
    ScannedBoxRegister, ScannedBoxShift, ScannedBoxShiftPayload, ScannedCharacterDefinition,
    ScannedDiscretionary, ScannedDisplayDiagnostic, ScannedEquationNumber, ScannedFileName,
    ScannedGlueParameterAssignment, ScannedHyphenationData, ScannedInsertConstruction,
    ScannedLeaderPayload, ScannedLetAssignment, ScannedMacroDefinition, ScannedMathCharacter,
    ScannedMathDelimiter, ScannedMathFamily, ScannedMathFraction, ScannedMathMuMaterial,
    ScannedMathScript, ScannedPackingSpec, ScannedRegisterDefinition, ScannedRuleSpec,
    ScannedScalar, ScannedSetBoxAssignment, ScannedTokenRegisterAssignment, ScannedVSplit,
    StructuredProvenance,
};
pub use snapshot::{CommandStateSnapshot, CommandSummary, CommandSummaryError};
pub use state::{
    CommandReplayDelivery, CommandReplayEpisode, CommandRuntime, CommandState,
    UnknownRegisteredSource,
};
pub use tex_state::SourceId;
pub use tex_state::TracedTokenList;
pub use tex_state::token::Catcode;
