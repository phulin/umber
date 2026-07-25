//! Canonical TeX command-processing boundary.
//!
//! Semantic state machines are crate-private. The public facade grows only as
//! executor integration requires stable, end-state operations.

mod command;
mod conditionals;
mod error;
mod host;
mod input;
mod macro_call;
mod observation;
mod primitives;
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
pub use host::{
    CommandHostCapabilities, CommandHostContext, ConditionalMode, ConditionalState, FontResource,
};
pub use input::{
    InvalidSourceCharacter, LexerState, LineTerminator, MalformedUnicodeRange, PhysicalLine,
    RegisteredSourceKind, SourceCharacter, SourceControlSequenceKind, SourceLocation,
    SourceProvenance, SourceRange, SourceRegistration, SourceRegistrationError, SourceScalarRange,
    SourceToken, SourceTokenizationStep,
};
#[cfg(any(test, feature = "instrumentation"))]
pub use observation::{
    AlignmentRecord, CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation,
    CommandObserver, CommandProvenance, ConditionRecord, DiagnosticArgument, EffectRecord,
    InputReason, InputRecord, InputTransition, MacroRecord, MutationRecord, ObservedToken,
    RecoveryKind, RecoveryRecord, ScannerRecord, ScannerStatusRecord, TokenListRecord,
};
#[cfg(any(test, feature = "instrumentation"))]
pub use processor::AlignmentCellFinishObservations;
pub use processor::{
    AlignmentCellCompletion, AlignmentCellDelimiter, AlignmentCellTemplates, AlignmentDelivery,
    AlignmentDeliveryEvent, AlignmentIdentity, AlignmentLifecycleError, AlignmentPreamble,
    AlignmentRequest, AlignmentRequestResult, CommandProcessor, FinishedAlignmentCell,
};
pub use profile::{
    CharacterCode, CharacterCodeError, CharacterMode, CommandCapabilities, CommandDialect,
    CommandProfile, CommandProfileBoundary, CommandProfileEncodingError, CommandProfileFingerprint,
    CommandProfileMismatch,
};
pub use scanners::{
    AlignmentCellOpening, FileNameTermination, FontLoadRequest, ImmediateExtension,
    InputStreamRequest, InternalValue, RegisteredInput, ScalarProvenance, ScalarRecovery,
    ScannedAccent, ScannedAccentBase, ScannedBalancedText, ScannedBoxConstruction, ScannedBoxKind,
    ScannedBoxRegister, ScannedDiscretionary, ScannedFileName, ScannedGlueParameterAssignment,
    ScannedLetAssignment, ScannedMacroDefinition, ScannedPackingSpec, ScannedRuleSpec,
    ScannedScalar, ScannedSetBoxAssignment, ScannedTokenRegisterAssignment, StructuredProvenance,
};
pub use snapshot::{CommandStateSnapshot, CommandSummary, CommandSummaryError};
pub use state::{CommandReplayEpisode, CommandRuntime, CommandState, UnknownRegisteredSource};
pub use tex_state::SourceId;
pub use tex_state::TracedTokenList;
pub use tex_state::token::Catcode;
