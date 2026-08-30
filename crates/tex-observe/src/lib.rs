//! Detached translation from command-core observations to portable oracle evidence.

use std::sync::Arc;

use tex_command::canonical_names;
use tex_command::{
    AlignmentRecord, CommandDeliveryBoundary, CommandObservation, CommandObserver, ConditionRecord,
    DiagnosticClass as CommandDiagnosticClass, DiagnosticHistory as CommandDiagnosticHistory,
    DiagnosticLifecycleRecord, DiagnosticOutcome as CommandDiagnosticOutcome, EffectRecord,
    GeometryRecord, InputReason as CommandInputReason, InputRecord, InputTransition, MacroRecord,
    MutationRecord, MutationTarget, ObservationEffectKind, ObservationValue, ObservedToken,
    RecoveryKind as CommandRecoveryKind, RecoveryRecord, ScannerStatusRecord, TokenListRecord,
};
use tex_oracle::OracleBundle;
use tex_oracle::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, ConditionEvent, ConditionTransition, DiagnosticClass, DiagnosticEvent,
    DiagnosticHistory, DiagnosticLifecycleEvent, DiagnosticOutcome, DiagnosticSeverity,
    EffectEvent, EffectKind, Event, EventView, GeometryEvent, InputEvent, InputReason, MacroEvent,
    MutationEvent, NormalizedEvent, Normalizer, ObservationHeader, ObservationStream, OracleToken,
    RecoveryEvent, RecoveryKind, ScannerEvent, ScannerStatus, ScannerStatusEvent, SchemaVersion,
    SourceLocation, StateTarget, Tex82ObserverProfile, TokenListEvent, TokenListTransition,
};
use tex_state::SourceId;

mod translation;

use translation::{source_line_starts, translate_observation};

/// Projects one committed command effect into the portable oracle schema.
///
/// This is the shared boundary for consumers that need a typed effect without
/// constructing a complete source-attributed observation stream. In
/// particular, command-semantic channel comparison uses it for the Umber side
/// and reads the same [`EffectEvent`] type from the reference-engine stream.
#[must_use]
pub fn portable_effect_observation(observation: &CommandObservation) -> Option<EffectEvent> {
    let CommandObservation::Effect(record) = observation else {
        return None;
    };
    let Event::Effect(effect) = translation::translate_effect(record.clone()) else {
        unreachable!("effect translation always produces an effect event")
    };
    Some(effect)
}

const CANONICAL_ROOT_PUSH_NAME: &str = "terminal";

/// One translated observer event plus source/provenance-only diagnostic context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedEvent {
    pub event: Event,
    pub context: String,
}

impl ObservedEvent {
    /// Creates an observed event with detached diagnostic context.
    #[must_use]
    pub fn new(event: Event, context: String) -> Self {
        Self { event, context }
    }
    /// Portable semantic value, without host-only diagnostic context.
    #[must_use]
    pub fn semantic(&self) -> &Event {
        &self.event
    }
    /// Source and delivery context retained for divergence reporting.
    #[must_use]
    pub fn context(&self) -> &str {
        &self.context
    }
}

/// Immutable source material needed to translate command provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSource {
    pub name: String,
    pub source: SourceId,
    pub bytes: Arc<[u8]>,
}

/// Terminal state supplied by the host after normal engine execution returns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveSessionOutcome {
    Completed,
    Failed { diagnostic: String, detail: String },
}

/// Canonical full diagnostic stream and its stable TRIP-profile projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSessionStreams {
    pub diagnostic: Vec<u8>,
    pub stable: Vec<u8>,
}

/// Non-fallible detached capture used when source context is available only
/// after the engine returns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapturedObservations {
    observations: Vec<CommandObservation>,
}

impl CapturedObservations {
    #[must_use]
    pub fn into_captured(self) -> Vec<CommandObservation> {
        self.observations
    }
}

impl CommandObserver for CapturedObservations {
    fn observes_geometry(&self) -> bool {
        true
    }

    fn committed(&mut self, observation: CommandObservation) {
        self.observations.push(observation);
    }
}

/// Closed semantic projections produced from one translated observation run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticEvidenceProfile {
    /// Every portable non-geometry observation, in committed order.
    Complete,
    /// TeX82's bounded full-TRIP stream: shipouts and terminal outcome.
    Tex82Trip,
}

/// Closed geometry projections produced beside semantic evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryEvidenceProfile {
    /// Retain source/line attribution (schema v3 and detached evidence).
    Located,
    /// Erase attribution while retaining geometry values (schema v2 TRIP).
    Positionless,
}

struct FinalizedEvidence {
    bundle: OracleBundle,
    trip: Vec<NormalizedEvent>,
}

/// Host-side translation of captured normal-session observations.
///
/// This owns only detached oracle transport state. It neither drives nor
/// mutates a `EngineSession`; callers may hand it observations after
/// the engine has returned, including after an early failure.
pub struct LiveSessionTranslator {
    schema: SchemaVersion,
    default_source: String,
    sources: Vec<ActiveSource>,
    current_source: Option<SourceId>,
    events: Vec<ObservedEvent>,
    geometry: bool,
    preserve_macro_reference_operands: bool,
}

type Recorder = LiveSessionTranslator;

struct ActiveSource {
    name: String,
    source: SourceId,
    bytes: Arc<[u8]>,
    /// Physical line starts, calculated once when the source becomes active.
    ///
    /// Command observation needs a line and byte-column for every direct
    /// source delivery. Recounting newlines in the source prefix for each of
    /// those deliveries made provenance translation scale quadratically with
    /// the document length.
    line_starts: Arc<[usize]>,
}

impl LiveSessionTranslator {
    pub fn new(source: impl Into<String>, schema: SchemaVersion) -> Self {
        Self {
            schema,
            default_source: source.into(),
            sources: Vec::new(),
            current_source: None,
            events: Vec::new(),
            geometry: schema >= SchemaVersion::V2,
            preserve_macro_reference_operands: false,
        }
    }

    /// Creates a translator for an already-open root source.
    #[must_use]
    pub fn for_root(
        schema: SchemaVersion,
        terminal_name: impl Into<String>,
        root: LiveSource,
    ) -> Self {
        let mut translator = Self::new(terminal_name, schema);
        translator.preserve_macro_reference_operands = true;
        translator.activate_source(root.name, root.source, root.bytes);
        translator
    }

    /// Translates a captured committed observation sequence exactly once.
    pub fn translate_captured(
        &mut self,
        observations: impl IntoIterator<Item = CommandObservation>,
    ) {
        for observation in observations {
            self.committed(observation);
        }
    }

    /// Finalizes both the full diagnostic stream and the byte-identical stable
    /// TRIP projection under the caller-supplied pinned stream header.
    pub fn finish(
        mut self,
        header: ObservationHeader,
        outcome: LiveSessionOutcome,
    ) -> Result<LiveSessionStreams, String> {
        let schema = SchemaVersion::try_from(header.schema)?;
        if schema != SchemaVersion::V1 {
            return Err("live diagnostic translation currently requires schema v1".into());
        }
        if let LiveSessionOutcome::Failed { diagnostic, detail } = outcome {
            self.events.push(ObservedEvent::new(
                Event::Diagnostic(DiagnosticEvent {
                    severity: DiagnosticSeverity::Fatal,
                    diagnostic,
                    arguments: vec![CanonicalValue::Name(detail)],
                }),
                "source=host; terminal_outcome=failure".into(),
            ));
            self.ensure_terminated();
        }
        let finalized = self.finalize_once(true, GeometryEvidenceProfile::Located);
        let diagnostic = encode_normalized_stream(&header, &finalized.bundle.semantic)?;
        let stable = encode_normalized_stream(&header, &finalized.trip)?;
        let decoded = ObservationStream::from_canonical_json_lines(&stable)
            .map_err(|error| error.to_string())?;
        Tex82ObserverProfile::Trip.validate(&decoded)?;
        Ok(LiveSessionStreams { diagnostic, stable })
    }

    fn ensure_terminated(&mut self) {
        let ends_in_stop = self.events.last().is_some_and(|event| {
            matches!(
                &event.event,
                Event::Input(input)
                    if input.transition == tex_oracle::InputTransition::Stop
                        && input.reason == InputReason::Source
                        && input.name == "terminal"
            )
        });
        let ends_in_termination = self.events.last().is_some_and(|event| {
            matches!(
                &event.event,
                Event::Effect(effect)
                    if effect.kind == EffectKind::Terminate && effect.channel == "engine"
            )
        });
        if !ends_in_stop && !ends_in_termination {
            self.events.push(ObservedEvent::new(
                Event::Input(InputEvent {
                    transition: tex_oracle::InputTransition::Stop,
                    reason: InputReason::Source,
                    name: "terminal".into(),
                }),
                "source=terminal; terminal_outcome=failure".into(),
            ));
        }
        if !ends_in_termination {
            self.events.push(ObservedEvent::new(
                Event::Effect(EffectEvent {
                    kind: EffectKind::Terminate,
                    channel: "engine".into(),
                    value: CanonicalValue::None,
                }),
                "source=terminal; terminal_outcome=failure".into(),
            ));
        }
    }

    /// Records the harness's completed source-open operation. This is an
    /// actual typed startup transition, not an expected-event reconstruction.
    pub fn record_source_open(&mut self, trace_name: &str, root_name: &str, source: SourceId) {
        self.events.push(ObservedEvent::new(
            Event::Input(InputEvent {
                transition: tex_oracle::InputTransition::Push,
                reason: InputReason::Source,
                name: trace_name.into(),
            }),
            format!("source={root_name}; source_id={}", source.raw()),
        ));
    }

    pub fn activate_source(&mut self, name: impl Into<String>, source: SourceId, bytes: Arc<[u8]>) {
        let name = name.into();
        let line_starts = source_line_starts(&bytes);
        if let Some(known) = self.sources.iter_mut().find(|known| known.source == source) {
            known.name = name;
            known.bytes = bytes;
            known.line_starts = line_starts;
        } else {
            self.sources.push(ActiveSource {
                name,
                source,
                bytes,
                line_starts,
            });
        }
        self.current_source = Some(source);
    }

    fn activate_registered_input(&mut self, name: &str, source: SourceId, bytes: Arc<[u8]>) {
        self.record_source_open(CANONICAL_ROOT_PUSH_NAME, name, source);
        self.activate_source(name.to_owned(), source, bytes);
    }

    fn source(&self, source: SourceId) -> Option<&ActiveSource> {
        self.sources.iter().find(|known| known.source == source)
    }
}

fn encode_normalized_stream(
    header: &ObservationHeader,
    events: &[NormalizedEvent],
) -> Result<Vec<u8>, String> {
    let mut oracle = serde_json::to_vec(header).map_err(|error| error.to_string())?;
    oracle.push(b'\n');
    tex_oracle::canonical_bundle_json_lines(events, &oracle)
}

fn observation_source(observation: &CommandObservation) -> Option<SourceId> {
    match observation {
        CommandObservation::Command(record) => record
            .provenance
            .source_location
            .map(tex_command::SourceLocation::source)
            .or_else(|| {
                record
                    .provenance
                    .source_range
                    .map(tex_command::SourceRange::source)
            }),
        CommandObservation::Input(record) => record.source,
        CommandObservation::Geometry(
            GeometryRecord::Hpack { source, .. }
            | GeometryRecord::Vpack { source, .. }
            | GeometryRecord::Shipout { source, .. },
        ) => *source,
        CommandObservation::DiagnosticLifecycle(DiagnosticLifecycleRecord::Report {
            location,
            ..
        }) => Some(location.source()),
        _ => None,
    }
}

impl CommandObserver for Recorder {
    fn observes_geometry(&self) -> bool {
        self.geometry
    }

    fn committed(&mut self, observation: CommandObservation) {
        if matches!(observation, CommandObservation::Geometry(_)) && !self.geometry {
            return;
        }
        if matches!(observation, CommandObservation::DiagnosticLifecycle(_))
            && self.schema < SchemaVersion::V4
        {
            return;
        }
        if matches!(
            observation,
            CommandObservation::Effect(EffectRecord {
                kind: ObservationEffectKind::ShowGroups
                    | ObservationEffectKind::ShowIfs
                    | ObservationEffectKind::ShowTokens,
                ..
            })
        ) {
            return;
        }
        if let CommandObservation::GeneratedSource(record) = &observation {
            self.activate_source(
                record.name.clone(),
                record.source.id,
                Arc::clone(&record.source.bytes),
            );
            return;
        }
        if let CommandObservation::Effect(EffectRecord {
            kind: ObservationEffectKind::Input,
            channel,
            source: Some(source),
            ..
        }) = &observation
        {
            // The effect carries the command-core capability hand-off, while
            // the portable trace observes only the resulting source push.
            self.activate_registered_input(channel, source.id, Arc::clone(&source.bytes));
            return;
        }
        let source_id = observation_source(&observation).or(self.current_source);
        if let Some(source) = observation_source(&observation) {
            self.current_source = Some(source);
        }
        let source = source_id.and_then(|source| self.source(source));
        let source_name =
            source.map_or_else(|| self.default_source.clone(), |source| source.name.clone());
        let source_bytes = source.map(|source| Arc::clone(&source.bytes));
        let source_line_starts = source.map(|source| Arc::clone(&source.line_starts));
        self.events.push(translate_observation(
            &source_name,
            source_id,
            source_bytes.as_deref(),
            source_line_starts.as_deref(),
            observation,
            self.preserve_macro_reference_operands,
        ));
    }
}

impl LiveSessionTranslator {
    /// Returns translated events, retaining host-only context for diagnostics.
    #[must_use]
    pub fn into_events(self) -> Vec<ObservedEvent> {
        self.events
    }

    /// Drains translated events while retaining source/provenance state.
    ///
    /// Long-running diagnostic consumers can compare and release an exact
    /// prefix without keeping a second full-document stream resident.
    #[must_use]
    pub fn take_events(&mut self) -> Vec<ObservedEvent> {
        std::mem::take(&mut self.events)
    }

    /// Finalizes portable normalized evidence without engine or source identities.
    #[must_use]
    pub fn finalize_detached_evidence(self) -> OracleBundle {
        self.finalize_once(false, GeometryEvidenceProfile::Located)
            .bundle
    }

    /// Finalizes a typed semantic profile and geometry projection in one pass.
    #[must_use]
    pub fn finalize_profile(
        self,
        semantic: SemanticEvidenceProfile,
        geometry: GeometryEvidenceProfile,
    ) -> OracleBundle {
        let finalized =
            self.finalize_once(semantic == SemanticEvidenceProfile::Tex82Trip, geometry);
        if semantic == SemanticEvidenceProfile::Tex82Trip {
            OracleBundle {
                semantic: finalized.trip,
                geometry: finalized.bundle.geometry,
            }
        } else {
            finalized.bundle
        }
    }

    fn finalize_once(
        self,
        include_trip: bool,
        geometry_profile: GeometryEvidenceProfile,
    ) -> FinalizedEvidence {
        let mut semantic_normalizer = Normalizer::new();
        let mut geometry_normalizer = Normalizer::new();
        let mut trip_normalizer = Normalizer::new();
        let mut bundle = OracleBundle::default();
        let mut trip = Vec::new();
        for observed in self.events {
            match observed.event.view() {
                EventView::Geometry(_) => {
                    let event = match geometry_profile {
                        GeometryEvidenceProfile::Located => observed.event,
                        GeometryEvidenceProfile::Positionless => observed.event.without_locations(),
                    };
                    bundle.geometry.push(geometry_normalizer.normalize(event));
                }
                _ => {
                    if include_trip && let Some(event) = trip_event(&observed) {
                        trip.push(trip_normalizer.normalize(event));
                    }
                    bundle
                        .semantic
                        .push(semantic_normalizer.normalize(observed.event));
                }
            }
        }
        FinalizedEvidence { bundle, trip }
    }
}

fn trip_event(observed: &ObservedEvent) -> Option<Event> {
    match observed.event.view() {
        EventView::Effect(effect)
            if matches!(effect.kind, EffectKind::Shipout | EffectKind::Terminate) =>
        {
            Some(observed.event.clone())
        }
        EventView::Input(input)
            if input.reason == InputReason::Source
                && ((input.transition == tex_oracle::InputTransition::Stop
                    && input.name == "terminal")
                    || (input.transition == tex_oracle::InputTransition::Retire
                        && matches!(input.name.as_str(), "terminal" | "read_stream"))) =>
        {
            Some(Event::Input(InputEvent {
                transition: tex_oracle::InputTransition::Stop,
                reason: InputReason::Source,
                name: "terminal".into(),
            }))
        }
        EventView::Diagnostic(diagnostic)
            if diagnostic.severity == DiagnosticSeverity::Fatal
                && !observed.context.contains("terminal_outcome=failure") =>
        {
            Some(observed.event.clone())
        }
        EventView::Command(_)
        | EventView::Input(_)
        | EventView::Recovery(_)
        | EventView::ScannerStatus(_)
        | EventView::Macro(_)
        | EventView::Condition(_)
        | EventView::Scanner(_)
        | EventView::TokenList(_)
        | EventView::Alignment(_)
        | EventView::Mutation(_)
        | EventView::Diagnostic(_)
        | EventView::DiagnosticLifecycle(_)
        | EventView::Effect(_)
        | EventView::Geometry(_) => None,
    }
}

#[cfg(test)]
mod tests;
