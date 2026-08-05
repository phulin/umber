//! Detached translation from command-core observations to portable oracle evidence.

use std::sync::Arc;

use tex_command::canonical_names;
use tex_command::{
    AlignmentRecord, CommandDeliveryBoundary, CommandObservation, CommandObserver, ConditionRecord,
    EffectRecord, GeometryRecord, InputReason as CommandInputReason, InputRecord, InputTransition,
    MacroRecord, MutationRecord, ObservedToken, RecoveryKind as CommandRecoveryKind,
    RecoveryRecord, ScannerStatusRecord, TokenListRecord,
};
use tex_oracle::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, ConditionEvent, ConditionTransition, DiagnosticEvent, DiagnosticSeverity,
    EffectEvent, EffectKind, Event, GeometryEvent, InputEvent, InputReason, MacroEvent,
    MutationEvent, Normalizer, ObservationHeader, ObservationStream, OracleToken,
    RecoveryEvent, RecoveryKind, ScannerEvent, ScannerStatus, ScannerStatusEvent, SchemaVersion,
    SourceLocation, StateTarget, Tex82ObserverProfile, TokenListEvent, TokenListTransition,
};
use tex_oracle::OracleBundle;
use tex_state::SourceId;

mod translation;

use translation::{AlignmentNesting, source_line_starts, translate_observation};

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

/// Host-side translation of captured normal-session observations.
///
/// This owns only detached oracle transport state. It neither drives nor
/// mutates a `EngineSession`; callers may hand it observations after
/// the engine has returned, including after an early failure.
pub struct LiveSessionTranslator {
    sources: Vec<ActiveSource>,
    known_source_names: Vec<(SourceId, String)>,
    alignment_nesting: AlignmentNesting,
    events: Vec<ObservedEvent>,
    geometry: bool,
    preserve_macro_reference_operands: bool,
}

type Recorder = LiveSessionTranslator;

struct ActiveSource {
    name: String,
    source: Option<SourceId>,
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
            sources: vec![ActiveSource {
                name: source.into(),
                source: None,
                bytes: Arc::from(&b""[..]),
                line_starts: Arc::from([0]),
            }],
            known_source_names: Vec::new(),
            alignment_nesting: AlignmentNesting::default(),
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
        let diagnostic =
            encode_observed_stream(&header, self.events.iter().map(|event| &event.event))?;
        let stable_events = self.events.iter().filter_map(|event| match &event.event {
            Event::Effect(effect)
                if matches!(effect.kind, EffectKind::Shipout | EffectKind::Terminate) =>
            {
                Some(&event.event)
            }
            Event::Input(input)
                if input.transition == tex_oracle::InputTransition::Stop
                    && input.reason == InputReason::Source
                    && input.name == "terminal" =>
            {
                Some(&event.event)
            }
            Event::Diagnostic(diagnostic)
                if diagnostic.severity == DiagnosticSeverity::Fatal
                    && !event.context.contains("terminal_outcome=failure") =>
            {
                Some(&event.event)
            }
            _ => None,
        });
        let stable = encode_observed_stream(&header, stable_events)?;
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
        if !self.known_source_names.iter().any(|(id, _)| *id == source) {
            self.known_source_names.push((source, name.clone()));
        }
        self.sources.push(ActiveSource {
            name,
            source: Some(source),
            bytes,
            line_starts,
        });
    }

    fn activate_registered_input(&mut self, name: &str, source: SourceId, bytes: Arc<[u8]>) {
        self.record_source_open(CANONICAL_ROOT_PUSH_NAME, name, source);
        self.activate_source(name.to_owned(), source, bytes);
    }

    fn current_source(&self) -> &ActiveSource {
        self.sources
            .last()
            .expect("terminal source is always active during replay")
    }

    fn retire_current_source(&mut self) {
        if self.sources.len() > 1 {
            self.sources.pop();
        }
    }
}

fn encode_observed_stream<'a>(
    header: &ObservationHeader,
    events: impl IntoIterator<Item = &'a Event>,
) -> Result<Vec<u8>, String> {
    let mut normalizer = Normalizer::new();
    let events = events
        .into_iter()
        .map(|event| normalizer.normalize(event.clone()))
        .collect::<Vec<_>>();
    let mut oracle = serde_json::to_vec(header).map_err(|error| error.to_string())?;
    oracle.push(b'\n');
    tex_oracle::canonical_bundle_json_lines(&events, &oracle)
}

fn geometry_source(observation: &CommandObservation) -> Option<SourceId> {
    match observation {
        CommandObservation::Geometry(
            GeometryRecord::Hpack { source, .. }
            | GeometryRecord::Vpack { source, .. }
            | GeometryRecord::Shipout { source, .. },
        ) => *source,
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
        if matches!(
            observation,
            CommandObservation::Effect(EffectRecord {
                kind: "showgroups" | "showifs" | "showtokens",
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
            kind: "input",
            detail,
            source: Some(source),
            ..
        }) = &observation
        {
            // The effect carries the command-core capability hand-off, while
            // the portable trace observes only the resulting source push.
            self.activate_registered_input(detail, source.id, Arc::clone(&source.bytes));
            return;
        }
        let (source_name, source_id, source_bytes, source_line_starts) = {
            let source = self.current_source();
            (
                source.name.clone(),
                source.source,
                Arc::clone(&source.bytes),
                Arc::clone(&source.line_starts),
            )
        };
        let source_name = geometry_source(&observation)
            .and_then(|source| {
                self.known_source_names
                    .iter()
                    .find_map(|(id, name)| (*id == source).then_some(name.clone()))
            })
            .unwrap_or(source_name);
        self.events.push(translate_observation(
            &source_name,
            source_id,
            Some(&source_bytes),
            Some(&source_line_starts),
            observation.clone(),
            &mut self.alignment_nesting,
            self.preserve_macro_reference_operands,
        ));
        if let CommandObservation::Input(InputRecord {
            transition: InputTransition::Retire,
            reason: CommandInputReason::Source,
            source_name,
            ..
        }) = observation
            && matches!(
                source_name,
                None | Some(
                    tex_command::SourceNameClass::Scantokens(_)
                        | tex_command::SourceNameClass::File
                )
            )
        {
            self.retire_current_source();
        }
    }
}

impl LiveSessionTranslator {
    /// Returns translated events, retaining host-only context for diagnostics.
    #[must_use]
    pub fn into_events(self) -> Vec<ObservedEvent> {
        self.events
    }

    /// Finalizes portable normalized evidence without engine or source identities.
    #[must_use]
    pub fn finalize_detached_evidence(self) -> OracleBundle {
        let mut semantic_normalizer = Normalizer::new();
        let mut geometry_normalizer = Normalizer::new();
        let mut evidence = OracleBundle::default();
        for observed in self.events {
            match observed.event {
                Event::Geometry(event) => evidence
                    .geometry
                    .push(geometry_normalizer.normalize(Event::Geometry(event))),
                event => evidence.semantic.push(semantic_normalizer.normalize(event)),
            }
        }
        evidence
    }
}

#[cfg(test)]
mod tests;
