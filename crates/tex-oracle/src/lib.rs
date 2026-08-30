//! Canonical semantic-event contract for instrumented TeX-family engines.
//!
//! This crate intentionally contains no command-engine types. Reference
//! engines and Umber must copy committed semantic values across this boundary;
//! observation may not inspect either engine's storage representation.

mod bootstrap;
mod bundle;
mod encoding;
mod event;
mod fixture;
mod fixture_audit;
mod minifixture_budget;
mod normalize;
mod profile;
mod schema;
mod suite;
mod transport;

pub use bootstrap::bootstrap_tex82_fixture;
pub use bundle::{
    MAX_BUNDLE_BYTES, MAX_BUNDLE_EVENT_BYTES, MAX_BUNDLE_EVENTS_PER_STREAM,
    MAX_BUNDLE_NESTING_DEPTH, MAX_BUNDLE_STRING_BYTES, ORACLE_BUNDLE_MAGIC, ORACLE_BUNDLE_SCHEMA,
    OracleBundle, canonical_bundle_json_lines, decode_oracle_bundle, encode_oracle_bundle,
};
pub use encoding::{EncodingError, ManifestIdentity, StreamIdentity};
pub use event::{
    ConciseEvent, EventAlignmentKey, EventAnchorKey, EventClass, EventLocation, EventLocationMut,
    EventView, EventViewMut,
};
pub use fixture::{
    CanonicalCitation, CommittedFixture, FIXTURE_CONTRACT_VERSION, FIXTURE_MANIFEST_NAME,
    FixtureArtifact, FixtureError, FixtureManifest, FixtureProfile, ToolIdentity,
};
pub use minifixture_budget::{
    MINIFIXTURE_MAX_EVENTS, MINIFIXTURE_MAX_SOURCE_BYTES, MINIFIXTURE_MAX_SOURCES,
    validate_minifixture_budget,
};
pub use normalize::{NormalizedEvent, Normalizer};
pub use profile::Tex82ObserverProfile;
pub use schema::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, ConditionEvent, ConditionTransition, DiagnosticClass, DiagnosticEvent,
    DiagnosticHistory, DiagnosticLifecycleEvent, DiagnosticOutcome, DiagnosticSeverity,
    EffectEvent, EffectKind, EngineDialect, EngineIdentity, Event, GeometryEvent, GeometryLocation,
    InputEvent, InputReason, InputTransition, LATEST_SCHEMA_VERSION, MacroEvent, Manifest,
    ManifestInput, MutationEvent, OracleToken, RecoveryEvent, RecoveryKind, SCHEMA_VERSION,
    ScannerEvent, ScannerStatus, ScannerStatusEvent, SchemaVersion, SourceLocation, StateTarget,
    TokenListEvent, TokenListTransition,
};
pub use suite::{
    Tex82CommandTraceSuite, Tex82GeometryTraceFixture, Tex82TraceFixture,
    validate_tex82_command_trace_suite, validate_tex82_geometry_trace_fixture,
};
pub use transport::{
    DisabledObserver, EventObserver, JsonLinesObserver, ObservationError, ObservationHeader,
    ObservationStream,
};

#[cfg(test)]
mod tests;
