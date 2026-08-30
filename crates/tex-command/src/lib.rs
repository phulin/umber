//! Canonical TeX command-processing boundary.
//!
//! Semantic state machines are crate-private. The public facade grows only as
//! executor integration requires stable, end-state operations.

#[cfg(all(test, feature = "profiling"))]
#[global_allocator]
static TEST_HOT_CORE_ALLOCATOR: tex_state::measurement::HotCoreAllocator =
    tex_state::measurement::HotCoreAllocator;

/// Publishes one semantic observation, evaluating its payload only when the
/// episode has an external observer.
///
/// The payload is a textual argument rather than a closure, so it may borrow
/// the processor immutably before `observe` takes it mutably, and it costs
/// nothing beyond the runtime predicate in an unobserved episode. Every
/// observation site in this crate goes through here, which is what lets the
/// observation vocabulary compile unconditionally without building records
/// when no observer is active.
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

mod attempt;
pub(crate) use attempt::CommandAttemptMark;
pub use attempt::{
    AttemptDefinitionId, AttemptError, AttemptGlueId, AttemptNameId, AttemptPromotionReceipt,
    AttemptPromotionRoots, AttemptProvenanceId, AttemptResumePoint, AttemptScope,
    AttemptSuspendError, AttemptSuspendFailure, AttemptTokenListId, AttemptTokenListIter,
    AttemptTokenListView, CommandAttempt, CommandAttemptChildScope, CommandAttemptOperation,
    PendingCommandAttempt, ScopedAttemptTokenListId,
};
mod command;
mod conditionals;
mod continuation;
pub use conditionals::{ActiveCondition, IncompleteCondition};
mod error;
mod execution_scratch;
pub use execution_scratch::ScannerFrameKey;
mod expansion_work;
pub use expansion_work::ExpansionWorkKey;
mod fatal;
mod fuel;
mod host;
mod input;
mod macro_call;
mod observation;
mod primitives;
pub use primitives::{
    CatalogueValidationError, DocumentationFamily, EnumPrimitiveView, ExpansionClass,
    GlueParameterDefault, InstallationPolicy, JobClockField, ParameterBankClass, ParameterCell,
    ParameterDefault, PrefixAdmissibility, PrimitiveCatalogue, PrimitiveDescriptor,
    PrimitiveDocumentationRow, PrimitiveOperand, PrimitiveOperandDomain, PrimitiveParameterView,
    PrimitiveProfile, PrimitiveProfiles, PrimitiveRegistration, PrimitiveSpelling,
    SpecialPrimitiveView, SpellingKind, WebIdentity, enum_primitive_views,
    exceeds_max_non_prefixed_command, install_etex_expandable_primitives,
    install_etex_unexpandable_primitives, install_latex_expandable_primitives,
    install_pdftex_expandable_primitives, install_pdftex_unexpandable_primitives,
    install_tex82_expandable_primitives, install_tex82_unexpandable_primitives,
    meaning_for_operand, primitive_documentation_rows, primitive_names,
    primitive_observation_identity, primitive_parameter_views, primitive_registrations,
    register_etex_expandable_primitives, register_etex_unexpandable_primitives,
    register_latex_expandable_primitives, register_pdftex_expandable_primitives,
    register_pdftex_unexpandable_primitives, register_tex82_expandable_primitives,
    register_tex82_unexpandable_primitives, render_primitive_documentation_table,
    special_primitive_views,
};
mod processor;
mod profile;
mod scalar_journal;
mod scan_toks;
mod scanners;
mod snapshot;
mod state;
mod timeline;
mod tracing_nesting;

#[cfg(test)]
mod test_harness;

pub use command::{CurrentCommand, DeliveryStamp};
pub use continuation::{CommandContinuationError, OwnedCommandContinuation};
pub use error::{CommandError, DimensionDiagnostic, InsertedUnit};
pub use fatal::{FATAL_SEVERITY, FatalError};
pub use fuel::{
    CommandFuel, CommandFuelLedger, CommandFuelLimitError, CommandWorkCounters,
    DEFAULT_COMMAND_FUEL_LIMIT, MAX_COMMAND_FUEL_LIMIT,
};
pub use host::{
    CommandHostCapabilities, CommandHostContext, ConditionalMode, ConditionalState,
    FileEnquiryIntent, FileEnquiryRequest, FileEnquiryResource, FontResource, LastNodeItem,
    PdfImageResource,
};
pub use input::{
    CONTROL_SEQUENCE_NAME_INLINE_CAPACITY, CatcodeQueries, ControlSequenceName,
    InvalidSourceCharacter, LexerState, LineTerminator, MalformedUnicodeRange, PhysicalLine,
    RegisteredSourceKind, SourceCharacter, SourceControlSequenceKind, SourceFramingPolicy,
    SourceLocation, SourceNameClass, SourceProvenance, SourceRange, SourceRegistration,
    SourceRegistrationError, SourceScalarRange, SourceStepQueries, SourceToken,
    SourceTokenizationStep,
};
#[cfg(feature = "profiling")]
pub use input::{
    LongMacroArgumentCursorBenchmark, LongMacroArgumentCursorReceipt, MixedPackedCursorBenchmark,
    MixedPackedCursorReceipt,
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
pub use processor::{
    AlignmentCellDelimiter, AlignmentCellTemplates, AlignmentDelivery, AlignmentDeliveryEvent,
    AlignmentIdentity, AlignmentLifecycleError, AlignmentLookahead, AlignmentPreamble,
    AlignmentRequest, AlignmentRequestResult, CommandDeliveryCursor, CommandProcessor,
    DeliveryStatus, FinishedAlignmentCell, PreparedAlignmentCellTemplates, PrintCommand,
    append_character_command_text, append_command_token_text, append_print_cmd_chr_text,
    append_print_esc_text, character_command_text, command_token_text, print_cmd_chr_text,
    print_esc_text,
};
pub use profile::{
    CharacterCode, CharacterCodeError, CharacterMode, CommandCapabilities, CommandDialect,
    CommandEngineSemantics, CommandProfile, CommandProfileBoundary, CommandProfileEncodingError,
    CommandProfileFingerprint, CommandProfileMismatch,
};
pub use scanners::{
    AlignmentCellOpening, EquationNumberSide, ExpandedWriteText, FileNameComponents,
    FontLoadRequest, FontSizeRecovery, GeneratedFontKind, HyphenationDataKind, ImmediateExtension,
    InputStreamRequest, InternalValue, MathDelimiterBoundary, MathDelimiterBoundaryKind,
    MathFamilySize, MathFieldBody, MathFieldEpisode, MathFractionKind, MathLimitKind, MathRequest,
    MathScriptKind, MathStyleKind, MathTextFieldKind, PdfActionDestination, PdfActionIdentifier,
    PdfActionSpec, PdfActionTarget, PdfAnnotationRequest, PdfColorStackActionRequest,
    PdfDestinationRequest, PdfDocumentFragmentRequest, PdfFormRequest, PdfGraphicsRequest,
    PdfImagePageBox, PdfImagePageSelection, PdfImageRequest, PdfNavigationRequest,
    PdfObjectRequest, PdfOutlineRequest, PdfReferenceObjectRequest, PdfStartLinkRequest,
    PdfThreadRequest, RegisteredInput, RestrictedInteger, RestrictedIntegerClass,
    RetainedScalarScan, ScalarProvenance, ScalarRecovery, ScalarScanFrame, ScalarScanStatus,
    ScannedAccent, ScannedAccentBase, ScannedBalancedText, ScannedBoxConstruction, ScannedBoxKind,
    ScannedBoxRegister, ScannedBoxShift, ScannedBoxShiftPayload, ScannedCharacterDefinition,
    ScannedDiscretionaryOpening, ScannedDisplayDiagnostic, ScannedEquationNumber, ScannedFileName,
    ScannedGeneratedFontDefinition, ScannedGlueParameterAssignment, ScannedHyphenationData,
    ScannedInsertConstruction, ScannedLeaderPayload, ScannedLetAssignment, ScannedMacroDefinition,
    ScannedMathCharacter, ScannedMathDelimiter, ScannedMathFamily, ScannedMathFraction,
    ScannedMathMuMaterial, ScannedMathScript, ScannedPackingSpec, ScannedRegisterDefinition,
    ScannedRuleSpec, ScannedScalar, ScannedSetBoxAssignment, ScannedSetBoxPath,
    ScannedTokenParameterAssignment, ScannedTokenRegisterAssignment, ScannedVSplit,
    StructuredProvenance, WriteStreamSelector,
};
pub use snapshot::{
    CommandCheckpointReleaseReceipt, CommandGenerationOwner, CommandRestoreError,
    CommandStateSnapshot, CommandSummary, CommandSummaryError, CommandTimelineCounters,
    PreparedCommandRestore, TransientCommandSnapshot,
};
pub use state::{
    CommandGroupError, CommandGroupExit, CommandReplayDelivery, CommandReplayEpisode,
    CommandSemanticDiagnostic, CommandStackUsage, CommandState, DiagnosticContextCoordinate,
    RunawayPrelude, StaleDiagnosticContext, UnknownRegisteredSource,
};
pub use tex_state::SourceId;
pub use tex_state::token::Catcode;
