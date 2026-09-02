use tex_command::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandObserver,
    CommandProvenance, DiagnosticRecord, EffectRecord, GeneratedSourceRecord, GeometryRecord,
    ObservationEffectKind, ObservationValue, ObservedToken, OpenedSourceSnapshot,
    SourceLocation as CommandSourceLocation,
};
use tex_observe::{
    GeometryEvidenceProfile, LiveSessionOutcome, LiveSessionTranslator, LiveSource,
    SemanticEvidenceProfile,
};
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
            bytes: (&b"X\n"[..]).into(),
        },
    )
}

#[test]
fn extraction_preserves_representative_detached_semantic_and_geometry_evidence() {
    let generated = SourceId::new(2);
    let mut translator = LiveSessionTranslator::for_root(
        SchemaVersion::V2,
        "terminal",
        LiveSource {
            name: "root.tex".into(),
            source: SourceId::new(1),
            bytes: (&b"R\n"[..]).into(),
        },
    );
    translator.translate_captured([
        CommandObservation::GeneratedSource(GeneratedSourceRecord {
            name: "generated".into(),
            source: OpenedSourceSnapshot {
                id: generated,
                bytes: (&b"G\n"[..]).into(),
            },
        }),
        CommandObservation::Command(CommandDeliveryRecord {
            boundary: CommandDeliveryBoundary::Raw,
            command: "letter".into(),
            command_operand: None,
            semantic_operand: None,
            spelling: ObservedToken::Character {
                character: 'G',
                catcode: tex_state::token::Catcode::Letter,
            },
            provenance: CommandProvenance {
                input_level: 2,
                position: 1,
                delivery_sequence: 7,
                has_origin: true,
                origin: OriginId::UNKNOWN,
                source_range: None,
                source_location: Some(CommandSourceLocation::new(generated, 0)),
            },
        }),
        CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "warning",
            diagnostic: "representative",
            arguments: Vec::new(),
        }),
        CommandObservation::Effect(EffectRecord {
            kind: ObservationEffectKind::Write,
            channel: "stream:1".into(),
            value: ObservationValue::Name("done".into()),
            source: None,
        }),
        CommandObservation::Geometry(GeometryRecord::Hpack {
            width_sp: 10,
            height_sp: 20,
            depth_sp: 3,
            line: 0,
            source: None,
        }),
    ]);

    let evidence = translator.finalize_detached_evidence();
    assert_eq!(evidence.semantic.len(), 3);
    assert_eq!(evidence.geometry.len(), 1);
    assert_eq!(
        evidence
            .semantic
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(evidence.geometry[0].sequence, 0);
    assert!(matches!(evidence.semantic[0].semantic, Event::Command(_)));
    assert!(matches!(
        evidence.semantic[1].semantic,
        Event::Diagnostic(_)
    ));
    assert!(matches!(evidence.semantic[2].semantic, Event::Effect(_)));
    assert!(matches!(evidence.geometry[0].semantic, Event::Geometry(_)));
}

#[test]
fn delayed_geometry_uses_its_captured_source_instead_of_the_live_frame() {
    let root = SourceId::new(1);
    let nested = SourceId::new(2);
    let mut translator = LiveSessionTranslator::for_root(
        SchemaVersion::V3,
        "terminal",
        LiveSource {
            name: "root.tex".into(),
            source: root,
            bytes: (&b"root\n"[..]).into(),
        },
    );
    translator.activate_source("nested.tex", nested, (&b"nested\n"[..]).into());
    translator.committed(CommandObservation::Geometry(GeometryRecord::Shipout {
        page_width_sp: 10,
        page_height_sp: 20,
        counts: [0; 10],
        line: 7,
        source: Some(root),
    }));

    let evidence = translator.finalize_detached_evidence();
    assert!(matches!(
        &evidence.geometry[0].semantic,
        Event::Geometry(tex_oracle::GeometryEvent::Shipout {
            location: Some(location),
            ..
        }) if location.source == "root.tex" && location.line == 7
    ));
}

#[test]
fn typed_finalizer_projects_trip_and_positionless_geometry_once() {
    let mut translator = LiveSessionTranslator::for_root(
        SchemaVersion::V2,
        "terminal",
        LiveSource {
            name: "trip.tex".into(),
            source: SourceId::new(1),
            bytes: (&b"X\n"[..]).into(),
        },
    );
    translator.committed(CommandObservation::Effect(EffectRecord {
        kind: ObservationEffectKind::Message,
        channel: "terminal".into(),
        value: ObservationValue::Bytes(b"not stable".to_vec()),
        source: None,
    }));
    translator.committed(CommandObservation::Effect(EffectRecord {
        kind: ObservationEffectKind::Shipout,
        channel: "dvi".into(),
        value: ObservationValue::Integer(1),
        source: None,
    }));
    translator.committed(CommandObservation::Geometry(GeometryRecord::Hpack {
        width_sp: 10,
        height_sp: 20,
        depth_sp: 3,
        line: 47,
        source: None,
    }));
    translator.committed(CommandObservation::Effect(EffectRecord {
        kind: ObservationEffectKind::Terminate,
        channel: "engine".into(),
        value: ObservationValue::None,
        source: None,
    }));

    let evidence = translator.finalize_profile(
        SemanticEvidenceProfile::Tex82Trip,
        GeometryEvidenceProfile::Positionless,
    );
    assert_eq!(evidence.semantic.len(), 2);
    assert!(matches!(
        &evidence.geometry[0].semantic,
        Event::Geometry(tex_oracle::GeometryEvent::Hpack { location: None, .. })
    ));
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
                diagnostic: "engine_session_error".into(),
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
            kind: ObservationEffectKind::Shipout,
            channel: "dvi".into(),
            value: ObservationValue::Integer(1),
            source: None,
        }),
        CommandObservation::Input(tex_command::InputRecord {
            transition: tex_command::InputTransition::Retire,
            reason: tex_command::InputReason::Source,
            source_name: None,
            source: None,
            level: 1,
            position: 2,
        }),
        CommandObservation::Input(tex_command::InputRecord {
            transition: tex_command::InputTransition::Stop,
            reason: tex_command::InputReason::Source,
            source_name: None,
            source: None,
            level: 0,
            position: 0,
        }),
        CommandObservation::Effect(EffectRecord {
            kind: ObservationEffectKind::Terminate,
            channel: "engine".into(),
            value: ObservationValue::None,
            source: None,
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
fn trip_profile_projects_read_stream_retirement_as_terminal_stop() {
    let mut translator = translator();
    translator.translate_captured([
        CommandObservation::Input(tex_command::InputRecord {
            transition: tex_command::InputTransition::Retire,
            reason: tex_command::InputReason::Source,
            source_name: Some(tex_command::SourceNameClass::ReadStream(3)),
            source: None,
            level: 1,
            position: 0,
        }),
        CommandObservation::Effect(EffectRecord {
            kind: ObservationEffectKind::Terminate,
            channel: "engine".into(),
            value: ObservationValue::None,
            source: None,
        }),
    ]);
    let evidence = translator.finalize_profile(
        SemanticEvidenceProfile::Tex82Trip,
        GeometryEvidenceProfile::Located,
    );
    assert!(matches!(
        &evidence.semantic[0].semantic,
        Event::Input(InputEvent {
            transition: InputTransition::Stop,
            reason: InputReason::Source,
            name,
        }) if name == "terminal"
    ));
}

#[test]
fn captured_observations_are_not_replayed_or_duplicated() {
    let mut translator = translator();
    translator.translate_captured([CommandObservation::Effect(EffectRecord {
        kind: ObservationEffectKind::Message,
        channel: "terminal".into(),
        value: ObservationValue::Bytes(b"once".to_vec()),
        source: None,
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
        semantic_operand: None,
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
fn input_effect_source_identity_resolves_after_unobserved_source_allocations() {
    // TeX82 §537's selected text-file source can follow arbitrary generated
    // pseudo-files. The observer must carry its real identity instead of
    // guessing that source identities are consecutive input opens.
    let mut translator = LiveSessionTranslator::for_root(
        SchemaVersion::V1,
        "terminal",
        LiveSource {
            name: "etrip.tex".into(),
            source: SourceId::new(1),
            bytes: (&b""[..]).into(),
        },
    );
    translator.translate_captured([
        CommandObservation::Effect(EffectRecord {
            kind: ObservationEffectKind::Input,
            channel: "etrip.out".into(),
            value: ObservationValue::None,
            source: Some(OpenedSourceSnapshot {
                id: SourceId::new(41),
                bytes: (&b"\\endgroup\n"[..]).into(),
            }),
        }),
        CommandObservation::Command(CommandDeliveryRecord {
            boundary: CommandDeliveryBoundary::Raw,
            spelling: ObservedToken::ControlSequence("endgroup".into()),
            command: "end_group".into(),
            command_operand: Some(0),
            semantic_operand: None,
            provenance: CommandProvenance {
                input_level: 9,
                position: 0,
                delivery_sequence: 7,
                has_origin: true,
                origin: OriginId::UNKNOWN,
                source_range: None,
                source_location: Some(tex_command::SourceLocation::new(SourceId::new(41), 8)),
            },
        }),
    ]);

    let streams = translator
        .finish(
            header(),
            LiveSessionOutcome::Failed {
                diagnostic: "bounded".into(),
                detail: "after command".into(),
            },
        )
        .expect("stream");
    let stream = ObservationStream::from_canonical_json_lines(&streams.diagnostic).expect("valid");
    let Event::Command(command) = &stream.events[1].semantic else {
        panic!("source push is followed by the input command");
    };
    assert_eq!(
        command.command.location,
        Some(SourceLocation {
            source: "etrip.out".into(),
            line: 1,
            byte: 8,
        })
    );
}

#[test]
fn repeated_packed_name_uses_each_opened_source_snapshot_until_its_retirement() {
    fn command(source: SourceId, byte: u64, character: char) -> CommandObservation {
        CommandObservation::Command(CommandDeliveryRecord {
            boundary: CommandDeliveryBoundary::Raw,
            spelling: ObservedToken::Character {
                character,
                catcode: tex_state::token::Catcode::Letter,
            },
            command: "letter".into(),
            command_operand: Some(i64::from(u32::from(character))),
            semantic_operand: None,
            provenance: CommandProvenance {
                input_level: 1,
                position: byte,
                delivery_sequence: byte,
                has_origin: true,
                origin: OriginId::UNKNOWN,
                source_range: None,
                source_location: Some(tex_command::SourceLocation::new(source, byte)),
            },
        })
    }

    let opened = |id, bytes: &'static [u8]| {
        CommandObservation::Effect(EffectRecord {
            kind: ObservationEffectKind::Input,
            channel: "same.tex".into(),
            value: ObservationValue::None,
            source: Some(OpenedSourceSnapshot {
                id: SourceId::new(id),
                bytes: bytes.into(),
            }),
        })
    };
    let retired = || {
        CommandObservation::Input(tex_command::InputRecord {
            transition: tex_command::InputTransition::Retire,
            reason: tex_command::InputReason::Source,
            source_name: Some(tex_command::SourceNameClass::File),
            source: None,
            level: 1,
            position: 0,
        })
    };

    let mut translator = translator();
    translator.translate_captured([
        opened(17, b"A\n"),
        command(SourceId::new(17), 0, 'A'),
        opened(93, b"\nB\n"),
        command(SourceId::new(93), 1, 'B'),
        retired(),
        command(SourceId::new(17), 0, 'A'),
        retired(),
        opened(211, b"xxC\n"),
        command(SourceId::new(211), 2, 'C'),
        retired(),
    ]);
    let streams = translator
        .finish(
            header(),
            LiveSessionOutcome::Failed {
                diagnostic: "bounded".into(),
                detail: "after repeated opens".into(),
            },
        )
        .expect("stream");
    let stream = ObservationStream::from_canonical_json_lines(&streams.diagnostic).expect("valid");
    let locations = stream
        .events
        .iter()
        .filter_map(|observed| match &observed.semantic {
            Event::Command(command) => command.command.location.clone(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        locations,
        [
            SourceLocation {
                source: "same.tex".into(),
                line: 1,
                byte: 0,
            },
            SourceLocation {
                source: "same.tex".into(),
                line: 2,
                byte: 0,
            },
            SourceLocation {
                source: "same.tex".into(),
                line: 1,
                byte: 0,
            },
            SourceLocation {
                source: "same.tex".into(),
                line: 1,
                byte: 2,
            },
        ]
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
        semantic_operand: None,
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
