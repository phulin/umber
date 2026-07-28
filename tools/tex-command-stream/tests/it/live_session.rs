use std::collections::BTreeMap;
use std::sync::Arc;

use tex_command::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
    EffectRecord, ObservedToken,
};
use tex_command_stream::{LiveSessionOutcome, LiveSessionTranslator, LiveSource};
use tex_oracle::{
    CanonicalValue, EffectEvent, EffectKind, Event, InputEvent, InputReason, InputTransition,
    Normalizer, ObservationHeader, ObservationStream, SchemaVersion, SourceLocation,
};
use tex_state::SourceId;
use tex_state::token::OriginId;

fn header() -> ObservationHeader {
    ObservationHeader {
        schema: SchemaVersion::V1.number(),
        manifest: "a".repeat(64),
    }
}

fn translator() -> LiveSessionTranslator {
    LiveSessionTranslator::for_root(
        SchemaVersion::V1,
        "terminal",
        LiveSource {
            name: "trip.tex".into(),
            source: SourceId::new(1),
            bytes: Arc::from(&b"X\n"[..]),
        },
        BTreeMap::new(),
    )
}

fn canonical(events: impl IntoIterator<Item = Event>) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&header()).expect("header");
    bytes.push(b'\n');
    let mut normalizer = Normalizer::new();
    for event in events {
        bytes.extend_from_slice(&serde_json::to_vec(&normalizer.normalize(event)).expect("event"));
        bytes.push(b'\n');
    }
    bytes
}

#[test]
fn failure_before_any_stable_event_yields_valid_terminated_diagnostic_stream() {
    let streams = translator()
        .finish(
            header(),
            LiveSessionOutcome::Failed {
                diagnostic: "canonical_session_error".into(),
                detail: "undispatched parameter".into(),
            },
        )
        .expect("failed session translates");
    let diagnostic =
        ObservationStream::from_canonical_json_lines(&streams.diagnostic).expect("valid stream");
    assert!(matches!(
        diagnostic.events.first().map(|event| &event.semantic),
        Some(Event::Diagnostic(_))
    ));
    assert!(matches!(
        diagnostic.events.last().map(|event| &event.semantic),
        Some(Event::Effect(effect)) if effect.kind == EffectKind::Terminate
    ));
    let stable = ObservationStream::from_canonical_json_lines(&streams.stable).expect("stable");
    assert_eq!(stable.events.len(), 2);
}

#[test]
fn normal_stable_projection_is_byte_identical() {
    let observations = vec![
        CommandObservation::Effect(EffectRecord {
            kind: "shipout",
            detail: "dvi\0".to_owned() + "1",
            tokens: None,
        }),
        CommandObservation::Input(tex_command::InputRecord {
            transition: tex_command::InputTransition::Retire,
            reason: tex_command::InputReason::Source,
            source_name: None,
            level: 1,
            position: 2,
        }),
        CommandObservation::Input(tex_command::InputRecord {
            transition: tex_command::InputTransition::Stop,
            reason: tex_command::InputReason::Source,
            source_name: None,
            level: 0,
            position: 0,
        }),
        CommandObservation::Effect(EffectRecord {
            kind: "terminate",
            detail: "engine\0".into(),
            tokens: None,
        }),
    ];
    let mut translator = translator();
    translator.translate_captured(observations);
    let streams = translator
        .finish(header(), LiveSessionOutcome::Completed)
        .expect("completed session translates");
    assert_eq!(
        streams.stable,
        canonical([
            Event::Effect(EffectEvent {
                kind: EffectKind::Shipout,
                channel: "dvi".into(),
                value: CanonicalValue::Integer(1),
            }),
            Event::Input(InputEvent {
                transition: InputTransition::Stop,
                reason: InputReason::Source,
                name: "terminal".into(),
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Terminate,
                channel: "engine".into(),
                value: CanonicalValue::None,
            }),
        ])
    );
}

#[test]
fn captured_observations_are_not_replayed_or_duplicated() {
    let mut translator = translator();
    translator.translate_captured([CommandObservation::Effect(EffectRecord {
        kind: "message",
        detail: "once".into(),
        tokens: None,
    })]);
    let streams = translator
        .finish(
            header(),
            LiveSessionOutcome::Failed {
                diagnostic: "failure".into(),
                detail: "after rollback".into(),
            },
        )
        .expect("stream");
    let stream = ObservationStream::from_canonical_json_lines(&streams.diagnostic).expect("valid");
    assert_eq!(
        stream
            .events
            .iter()
            .filter(|event| matches!(
                &event.semantic,
                Event::Effect(effect)
                    if effect.kind == EffectKind::Message
                        && effect.value == CanonicalValue::Bytes(b"once".to_vec())
            ))
            .count(),
        1
    );
}

#[test]
fn command_source_location_and_provenance_are_retained() {
    let mut translator = translator();
    translator.translate_captured([CommandObservation::Command(CommandDeliveryRecord {
        boundary: CommandDeliveryBoundary::Raw,
        spelling: ObservedToken::Character {
            character: 'X',
            catcode: tex_state::token::Catcode::Letter,
        },
        command: "letter".into(),
        command_operand: None,
        provenance: CommandProvenance {
            input_level: 1,
            position: 0,
            delivery_sequence: 7,
            has_origin: true,
            origin: OriginId::UNKNOWN,
            source_range: None,
            source_location: Some(tex_command::SourceLocation::new(SourceId::new(1), 0)),
        },
    })]);
    let streams = translator
        .finish(
            header(),
            LiveSessionOutcome::Failed {
                diagnostic: "failure".into(),
                detail: "after command".into(),
            },
        )
        .expect("stream");
    let stream = ObservationStream::from_canonical_json_lines(&streams.diagnostic).expect("valid");
    let Event::Command(command) = &stream.events[0].semantic else {
        panic!("first event is command");
    };
    assert_eq!(
        command.command.location,
        Some(SourceLocation {
            source: "trip.tex".into(),
            line: 1,
            byte: 0,
        })
    );
}

#[test]
fn live_macro_command_retains_reference_operand() {
    let mut translator = translator();
    translator.translate_captured([CommandObservation::Command(CommandDeliveryRecord {
        boundary: CommandDeliveryBoundary::Raw,
        spelling: ObservedToken::ControlSequence("par".into()),
        command: "outer_call".into(),
        command_operand: Some(249_982),
        provenance: CommandProvenance {
            input_level: 1,
            position: 0,
            delivery_sequence: 0,
            has_origin: true,
            origin: OriginId::UNKNOWN,
            source_range: None,
            source_location: None,
        },
    })]);
    let streams = translator
        .finish(
            header(),
            LiveSessionOutcome::Failed {
                diagnostic: "failure".into(),
                detail: "after macro".into(),
            },
        )
        .expect("stream");
    let stream = ObservationStream::from_canonical_json_lines(&streams.diagnostic).expect("valid");
    let Event::Command(command) = &stream.events[0].semantic else {
        panic!("first event is command");
    };
    assert_eq!(command.command.operand, CanonicalValue::Integer(249_982));
}
