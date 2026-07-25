//! Canonical semantic-event contract for instrumented TeX-family engines.
//!
//! This crate intentionally contains no command-engine types. Reference
//! engines and Umber must copy committed semantic values across this boundary;
//! observation may not inspect either engine's storage representation.

mod bootstrap;
mod encoding;
mod fixture;
mod fixture_audit;
mod normalize;
mod profile;
mod schema;
mod suite;
mod transport;

pub use bootstrap::bootstrap_tex82_fixture;
pub use encoding::{EncodingError, ManifestIdentity, StreamIdentity};
pub use fixture::{
    CanonicalCitation, CommittedFixture, FIXTURE_CONTRACT_VERSION, FIXTURE_MANIFEST_NAME,
    FixtureArtifact, FixtureError, FixtureManifest, FixtureProfile, ToolIdentity,
};
pub use normalize::{NormalizedEvent, Normalizer};
pub use profile::Tex82ObserverProfile;
pub use schema::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, ConditionEvent, ConditionTransition, DiagnosticEvent, DiagnosticSeverity,
    EffectEvent, EffectKind, EngineDialect, EngineIdentity, Event, InputEvent, InputReason,
    InputTransition, MacroEvent, Manifest, ManifestInput, MutationEvent, OracleToken,
    RecoveryEvent, RecoveryKind, SCHEMA_VERSION, ScannerEvent, ScannerStatus, ScannerStatusEvent,
    SourceLocation, StateTarget, TokenListEvent, TokenListTransition,
};
pub use suite::{Tex82CommandTraceSuite, Tex82TraceFixture, validate_tex82_command_trace_suite};
pub use transport::{
    DisabledObserver, EventObserver, JsonLinesObserver, ObservationError, ObservationHeader,
    ObservationStream,
};

#[cfg(test)]
mod tests;
