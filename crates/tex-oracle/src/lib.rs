//! Canonical semantic-event contract for instrumented TeX-family engines.
//!
//! This crate intentionally contains no command-engine types. Reference
//! engines and Umber must copy committed semantic values across this boundary;
//! observation may not inspect either engine's storage representation.

mod encoding;
mod normalize;
mod schema;
mod transport;

pub use encoding::{EncodingError, ManifestIdentity, StreamIdentity};
pub use normalize::{NormalizedEvent, Normalizer};
pub use schema::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, ConditionEvent, ConditionTransition, DiagnosticEvent, DiagnosticSeverity,
    EffectEvent, EffectKind, EngineDialect, EngineIdentity, Event, InputEvent, InputReason,
    InputTransition, MacroEvent, Manifest, ManifestInput, MutationEvent, OracleToken,
    RecoveryEvent, RecoveryKind, SCHEMA_VERSION, ScannerEvent, ScannerStatus, ScannerStatusEvent,
    SourceLocation, StateTarget, TokenListEvent, TokenListTransition,
};
pub use transport::{
    DisabledObserver, EventObserver, JsonLinesObserver, ObservationError, ObservationHeader,
};

#[cfg(test)]
mod tests;
