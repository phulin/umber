//! Canonical TeX command-processing boundary.
//!
//! Semantic state machines are crate-private. The public facade grows only as
//! executor integration requires stable, end-state operations.

/// Publishes one semantic observation, evaluating its payload only when the
/// episode has an external observer or active paragraph input recording.
///
/// The payload is a textual argument rather than a closure, so it may borrow
/// the processor immutably before `observe` takes it mutably, and it costs
/// nothing beyond the runtime predicate in an unobserved episode. Every
/// observation site in this crate goes through here, which is what lets the
/// observation vocabulary compile unconditionally without building records
/// when neither consumer is active. Every constructed record is first offered
/// to the paragraph transaction, then optionally delivered to the external
/// observer.
///
/// This replaced `#[cfg(any(test, feature = "observe"))]` on roughly 250
/// sites. Those attributes compiled the engine three different ways -- the
/// `test` arm, the feature arm, and neither -- so the traced engine the oracle
/// compares was never literally the engine that ships. They also forced every
/// entry point in `processor/observe.rs` to be written twice, once real and
/// once empty, because a cfg'd-out call site leaves its inputs unused
/// (`umber2-johp.200`). See `docs/cargo_feature_axes.md`.
macro_rules! observe {
    ($processor:expr, $observation:expr $(,)?) => {
        if $processor.is_observed() {
            let observation = $observation;
            $processor.observe(observation);
        }
    };
}

mod command;
mod conditionals;
mod continuation;
pub use conditionals::{ActiveCondition, IncompleteCondition};
mod error;
mod fatal;
mod fuel;
mod host;
mod input;
mod macro_call;
mod observation;
mod paragraph;
mod primitives;
pub use primitives::{
    exceeds_max_non_prefixed_command, install_etex_expandable_primitives,
    install_latex_expandable_primitives, install_pdftex_expandable_primitives,
    install_tex82_expandable_primitives, register_etex_expandable_primitives,
    register_latex_expandable_primitives, register_pdftex_expandable_primitives,
    register_tex82_expandable_primitives,
};
mod processor;
mod profile;
mod provenance;
mod scan_toks;
mod scanners;
mod snapshot;
mod state;
mod tracing_nesting;

#[cfg(test)]
mod fixture_replay;
#[cfg(test)]
mod test_harness;

pub use command::{CurrentCommand, DeliveryStamp};
pub use continuation::OwnedCommandContinuation;
pub use error::{CommandError, DimensionDiagnostic, InsertedUnit};
pub use fatal::{FATAL_SEVERITY, FatalError};
pub use fuel::{
    CommandFuel, CommandFuelLedger, CommandFuelLimitError, DEFAULT_COMMAND_FUEL_LIMIT,
    MAX_COMMAND_FUEL_LIMIT,
};
pub use host::{
    CommandHostCapabilities, CommandHostContext, ConditionalMode, ConditionalState,
    FileEnquiryIntent, FileEnquiryRequest, FileEnquiryResource, FontResource, LastNodeItem,
    PdfImageResource,
};
pub use input::{
    CatcodeQueries, FileFramingEvent, InvalidSourceCharacter, LexerState, LineTerminator,
    MalformedUnicodeRange, PhysicalLine, RegisteredSourceKind, SourceCharacter,
    SourceControlSequenceKind, SourceFramingPolicy, SourceLocation, SourceNameClass,
    SourceProvenance, SourceRange, SourceRegistration, SourceRegistrationError, SourceScalarRange,
    SourceStepQueries, SourceToken, SourceTokenizationStep,
};
/// The single canonical naming vocabulary shared by every observation
/// producer and transport (`docs/tex_command_core.md` §33.3).
pub use observation::canonical_names;
pub use observation::{
    AlignmentRecord, CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation,
    CommandObserver, CommandProvenance, ConditionRecord, DiagnosticArgument, DiagnosticRecord,
    EffectRecord, GeneratedSourceRecord, GeometryRecord, InputReason, InputRecord, InputTransition,
    MacroRecord, MutationRecord, MutationTarget, ObservationEffectKind, ObservationValue,
    ObservedToken, OpenedSourceSnapshot, ParameterClass, RecoveryKind, RecoveryRecord,
    ScannerRecord, ScannerStatusRecord, TokenListRecord, parameter_mutation_key,
    parameter_mutation_key_for_dialect,
};
pub use paragraph::{
    ParagraphInputCoverage, ParagraphInputReplayError, ParagraphInputTransaction,
    ParagraphMathShift,
};
pub use processor::{
    AlignmentCellDelimiter, AlignmentCellTemplates, AlignmentDelivery, AlignmentDeliveryEvent,
    AlignmentIdentity, AlignmentLifecycleError, AlignmentPreamble, AlignmentRequest,
    AlignmentRequestResult, CommandProcessor, FinishedAlignmentCell, PrintCommand,
    character_command_text, command_token_text, print_cmd_chr_text, print_esc_text,
};
pub use profile::{
    CharacterCode, CharacterCodeError, CharacterMode, CommandCapabilities, CommandDialect,
    CommandProfile, CommandProfileBoundary, CommandProfileEncodingError, CommandProfileFingerprint,
    CommandProfileMismatch,
};
pub use scanners::{
    AlignmentCellOpening, EquationNumberSide, ExpandedWriteText, FileNameComponents,
    FileNameTermination, FontLoadRequest, FontSizeRecovery, GeneratedFontKind, HyphenationDataKind,
    ImmediateExtension, InputStreamRequest, InternalValue, MathDelimiterBoundary,
    MathDelimiterBoundaryKind, MathFamilySize, MathFieldBody, MathFieldEpisode, MathFractionKind,
    MathLimitKind, MathRequest, MathScriptKind, MathStyleKind, MathTextFieldKind,
    PdfAnnotationRequest, PdfColorStackActionRequest, PdfDestinationRequest,
    PdfDocumentFragmentRequest, PdfFormRequest, PdfGraphicsRequest, PdfImagePageBox,
    PdfImagePageSelection, PdfImageRequest, PdfNavigationRequest, PdfObjectRequest,
    PdfOutlineRequest, PdfReferenceObjectRequest, PdfStartLinkRequest, PdfThreadRequest,
    RegisteredInput, RestrictedInteger, RestrictedIntegerClass, ScalarProvenance, ScalarRecovery,
    ScannedAccent, ScannedAccentBase, ScannedBalancedText, ScannedBoxConstruction, ScannedBoxKind,
    ScannedBoxRegister, ScannedBoxShift, ScannedBoxShiftPayload, ScannedCharacterDefinition,
    ScannedDiscretionaryOpening, ScannedDisplayDiagnostic, ScannedEquationNumber, ScannedFileName,
    ScannedGeneratedFontDefinition, ScannedGlueParameterAssignment, ScannedHyphenationData,
    ScannedInsertConstruction, ScannedLeaderPayload, ScannedLetAssignment, ScannedMacroDefinition,
    ScannedMathCharacter, ScannedMathDelimiter, ScannedMathFamily, ScannedMathFraction,
    ScannedMathMuMaterial, ScannedMathScript, ScannedPackingSpec, ScannedRegisterDefinition,
    ScannedRuleSpec, ScannedScalar, ScannedSetBoxAssignment, ScannedSetBoxPath,
    ScannedTokenRegisterAssignment, ScannedVSplit, StructuredProvenance, WriteStreamSelector,
};
pub use snapshot::{CommandStateSnapshot, CommandSummary, CommandSummaryError};
pub use state::{
    CommandReplayDelivery, CommandReplayEpisode, CommandRuntime, CommandSemanticDiagnostic,
    CommandState, RunawayPrelude, UnknownRegisteredSource,
};
pub use tex_state::SourceId;
pub use tex_state::TracedTokenList;
pub use tex_state::token::Catcode;
