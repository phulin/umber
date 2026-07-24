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

pub use command::{CurrentCommand, DeliveryStamp};
pub use error::CommandError;
pub use host::{CommandHostCapabilities, CommandHostContext, ConditionalMode, ConditionalState};
pub use input::{
    InvalidSourceCharacter, LexerState, LineTerminator, MalformedUnicodeRange, PhysicalLine,
    RegisteredSourceKind, SourceCharacter, SourceControlSequenceKind, SourceRange,
    SourceRegistration, SourceRegistrationError, SourceScalarRange, SourceToken,
    SourceTokenizationStep,
};
#[cfg(any(test, feature = "instrumentation"))]
pub use observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandObserver,
    CommandProvenance, InputRecord, InputTransition, ObservedToken, RecoveryRecord,
};
pub use processor::{
    AlignmentCellTemplates, AlignmentDelivery, AlignmentDeliveryEvent, AlignmentIdentity,
    AlignmentLifecycleError, AlignmentRequest, AlignmentRequestResult, CommandProcessor,
};
pub use profile::{
    CharacterCode, CharacterCodeError, CharacterMode, CommandCapabilities, CommandDialect,
    CommandProfile, CommandProfileBoundary, CommandProfileEncodingError, CommandProfileFingerprint,
    CommandProfileMismatch,
};
pub use snapshot::{CommandStateSnapshot, CommandSummary, CommandSummaryError};
pub use state::{CommandRuntime, CommandState, UnknownRegisteredSource};
pub use tex_state::SourceId;
pub use tex_state::token::Catcode;
