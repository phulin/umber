use super::*;
use tex_oracle::GeometryLocation;

#[test]
fn geometry_translation_captures_active_source_and_observation_line() {
    let observed = translate_observation(
        "chapters/math.tex",
        None,
        None,
        None,
        CommandObservation::Geometry(GeometryRecord::Hpack {
            width_sp: 10,
            height_sp: 20,
            depth_sp: 3,
            line: 47,
            source: None,
        }),
        &mut AlignmentNesting::default(),
        false,
    );
    assert_eq!(
        observed.event,
        Event::Geometry(GeometryEvent::Hpack {
            width_sp: 10,
            height_sp: 20,
            depth_sp: 3,
            location: Some(GeometryLocation {
                source: "chapters/math.tex".into(),
                line: 47,
            }),
        })
    );
}

use crate::translation::*;
use tex_command::ScannerRecord;
use tex_state::token::Catcode;

#[test]
fn source_push_uses_its_inherited_name_before_the_child_becomes_active() {
    let event = translate_input(
        InputRecord {
            transition: InputTransition::Push,
            reason: CommandInputReason::Source,
            source_name: Some(tex_command::SourceNameClass::Terminal),
            level: 7,
            position: 0,
        },
        "parent.tex",
    );
    assert_eq!(
        event,
        Event::Input(InputEvent {
            transition: tex_oracle::InputTransition::Push,
            reason: tex_oracle::InputReason::Source,
            name: "terminal".into(),
        })
    );
}

#[test]
fn pseudo_source_retirement_keeps_the_surrounding_file_active() {
    let root = LiveSource {
        name: "root.tex".into(),
        source: SourceId::new(7),
        bytes: Arc::from(&b"x"[..]),
    };
    let mut translator = LiveSessionTranslator::for_root(SchemaVersion::V1, "terminal", root);
    translator.translate_captured([
        CommandObservation::Input(InputRecord {
            transition: InputTransition::Push,
            reason: CommandInputReason::Source,
            source_name: Some(tex_command::SourceNameClass::ReadStream(1)),
            level: 9,
            position: 0,
        }),
        CommandObservation::Input(InputRecord {
            transition: InputTransition::Retire,
            reason: CommandInputReason::Source,
            source_name: Some(tex_command::SourceNameClass::ReadStream(1)),
            level: 9,
            position: 0,
        }),
        CommandObservation::Scanner(ScannerRecord {
            kind: "integer",
            value: "1".into(),
            tokens: None,
        }),
    ]);

    assert_eq!(
        translator.events[1].event,
        Event::Input(InputEvent {
            transition: tex_oracle::InputTransition::Retire,
            reason: InputReason::Source,
            name: "read_stream".into(),
        })
    );
    assert!(
        translator.events[2].context.starts_with("source=root.tex"),
        "retiring the nested readline level must not pop its outer file"
    );
    assert_eq!(translator.sources.len(), 2);

    let terminal = translate_input(
        InputRecord {
            transition: InputTransition::Retire,
            reason: CommandInputReason::Source,
            source_name: Some(tex_command::SourceNameClass::Terminal),
            level: 10,
            position: 0,
        },
        "root.tex",
    );
    assert_eq!(
        terminal,
        Event::Input(InputEvent {
            transition: tex_oracle::InputTransition::Retire,
            reason: InputReason::Source,
            name: "terminal".into(),
        })
    );
}

#[test]
fn source_line_index_preserves_line_and_column_boundaries() {
    let starts = source_line_starts(b"first\n\nlast");
    assert_eq!(&*starts, &[0, 6, 7]);
    assert_eq!(starts.partition_point(|start| *start < 1), 1);
    assert_eq!(starts.partition_point(|start| *start <= 5), 1);
    assert_eq!(starts.partition_point(|start| *start <= 6), 2);
    assert_eq!(starts.partition_point(|start| *start <= 7), 3);
    assert_eq!(starts.partition_point(|start| *start <= 10), 3);
}

#[test]
fn token_transport_carries_only_canonical_names() {
    assert_eq!(
        oracle_token(ObservedToken::Character {
            character: '_',
            catcode: Catcode::Subscript,
        }),
        OracleToken {
            character: u32::from('_'),
            catcode: "sub_mark".into(),
            control_sequence: None,
            location: None,
        }
    );
    assert_eq!(
        oracle_token(ObservedToken::FrozenEndTemplate),
        OracleToken {
            character: 0,
            catcode: "escape".into(),
            control_sequence: Some("endtemplate".into()),
            location: None,
        }
    );
    assert_eq!(
        command_token(&ObservedToken::FrozenEndV),
        (CanonicalValue::None, Some("endtemplate".into()))
    );
}

#[test]
fn recovery_transport_preserves_the_command_owned_kind() {
    let token = ObservedToken::ControlSequence("par".into());
    for (kind, expected) in [
        (CommandRecoveryKind::Backup, RecoveryKind::Backup),
        (
            CommandRecoveryKind::InsertedToken,
            RecoveryKind::InsertedToken,
        ),
        (
            CommandRecoveryKind::InsertedControlSequence,
            RecoveryKind::InsertedControlSequence,
        ),
    ] {
        assert!(matches!(
            translate_recovery(RecoveryRecord {
                kind,
                tokens: vec![token.clone()],
            }),
            Event::Recovery(RecoveryEvent { kind: actual, .. }) if actual == expected
        ));
    }
}

#[test]
fn message_effects_use_terminal_bytes() {
    assert_eq!(
        translate_effect(EffectRecord {
            kind: "message",
            detail: "READY".into(),
            source: None,
            tokens: None,
        }),
        Event::Effect(EffectEvent {
            kind: EffectKind::Message,
            channel: "terminal".into(),
            value: CanonicalValue::Bytes(b"READY".to_vec()),
        })
    );
}

#[test]
fn live_transport_suppresses_only_uninstrumented_print_effects() {
    // e-TeX [49.1292] routes these commands through TeX82 §1293's
    // diagnostic ending. The canonical reference observer has no events
    // at those print-only seams, so the host translation must not invent
    // any that displace the following command transitions.
    let mut translator = LiveSessionTranslator::new("terminal", SchemaVersion::V1);
    for record in [
        EffectRecord {
            kind: "showgroups",
            detail: "\n\n### bottom level".into(),
            source: None,
            tokens: None,
        },
        EffectRecord {
            kind: "showifs",
            detail: "\n### no active conditionals".into(),
            source: None,
            tokens: None,
        },
        EffectRecord {
            kind: "showtokens",
            detail: "A".into(),
            source: None,
            tokens: Some(vec![ObservedToken::Character {
                character: 'A',
                catcode: Catcode::Letter,
            }]),
        },
    ] {
        translator.committed(CommandObservation::Effect(record));
    }
    assert!(translator.events.is_empty());

    for record in [
        EffectRecord {
            kind: "message",
            detail: "READY".into(),
            source: None,
            tokens: None,
        },
        EffectRecord {
            kind: "open",
            detail: "stream:1\0result.log".into(),
            source: None,
            tokens: None,
        },
        EffectRecord {
            kind: "write",
            detail: "stream:1\0".into(),
            source: None,
            tokens: Some(vec![ObservedToken::Character {
                character: 'W',
                catcode: Catcode::Letter,
            }]),
        },
        EffectRecord {
            kind: "shipout",
            detail: "dvi\0".to_owned() + "1",
            source: None,
            tokens: None,
        },
        EffectRecord {
            kind: "terminate",
            detail: "engine\0".into(),
            source: None,
            tokens: None,
        },
    ] {
        translator.committed(CommandObservation::Effect(record));
    }
    assert_eq!(
        translator
            .events
            .iter()
            .map(|observed| observed.event.clone())
            .collect::<Vec<_>>(),
        vec![
            Event::Effect(EffectEvent {
                kind: EffectKind::Message,
                channel: "terminal".into(),
                value: CanonicalValue::Bytes(b"READY".to_vec()),
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Open,
                channel: "stream:1".into(),
                value: CanonicalValue::Name("result.log".into()),
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Write,
                channel: "stream:1".into(),
                value: CanonicalValue::Tokens(vec![OracleToken {
                    character: u32::from('W'),
                    catcode: "letter".into(),
                    control_sequence: None,
                    location: None,
                }]),
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Shipout,
                channel: "dvi".into(),
                value: CanonicalValue::Integer(1),
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Terminate,
                channel: "engine".into(),
                value: CanonicalValue::None,
            }),
        ]
    );
}

#[test]
fn shipout_effects_use_dvi_page_numbers() {
    assert_eq!(
        translate_effect(EffectRecord {
            kind: "shipout",
            detail: "dvi\x001".into(),
            source: None,
            tokens: None,
        }),
        Event::Effect(EffectEvent {
            kind: EffectKind::Shipout,
            channel: "dvi".into(),
            value: CanonicalValue::Integer(1),
        })
    );
}

#[test]
fn condition_transitions_project_canonical_names_and_limits() {
    assert_eq!(
        translate_condition(ConditionRecord {
            transition: "push",
            identity: 17,
            condition: "iftrue".into(),
            limit: "evaluating",
            branch: None,
        }),
        Event::Condition(ConditionEvent {
            transition: ConditionTransition::Push,
            condition: "iftrue".into(),
            limit: "evaluating".into(),
            branch: None,
        })
    );
    assert_eq!(
        translate_condition(ConditionRecord {
            transition: "branch",
            identity: 17,
            condition: "iftrue".into(),
            limit: "evaluating",
            branch: Some("true".into()),
        }),
        Event::Condition(ConditionEvent {
            transition: ConditionTransition::Branch,
            condition: "iftrue".into(),
            limit: "evaluating".into(),
            branch: Some("true".into()),
        })
    );
}

#[test]
fn brace_delivery_transitions_preserve_command_owned_align_state_changes() {
    for (transition, previous_align_state, align_state) in [
        ("begin_group", -1_000_000, -999_999),
        ("end_group", -999_999, -1_000_000),
    ] {
        assert_eq!(
            translate_alignment(
                AlignmentRecord {
                    transition,
                    alignment: Some(1),
                    align_state,
                    delimiter: None,
                    previous_align_state: Some(previous_align_state),
                },
                Some(1),
            ),
            Event::Alignment(AlignmentEvent {
                transition: AlignmentTransition::StateChange,
                align_state: i64::from(align_state),
                template: None,
                nesting: Some(1),
                previous_align_state: Some(i64::from(previous_align_state)),
                delimiter: None,
                recovery: None,
            })
        );
    }
}

#[test]
fn missing_right_brace_alignment_correction_is_canonical_recovery() {
    assert_eq!(
        translate_alignment(
            AlignmentRecord {
                transition: "missing_right_brace",
                alignment: Some(1),
                align_state: 1,
                delimiter: None,
                previous_align_state: None,
            },
            Some(1),
        ),
        Event::Alignment(AlignmentEvent {
            transition: AlignmentTransition::Recovery,
            align_state: 1,
            template: None,
            nesting: Some(1),
            previous_align_state: None,
            delimiter: None,
            recovery: Some("missing_right_brace".into()),
        })
    );
}

#[test]
fn alignment_nesting_returns_to_one_after_nested_finish() {
    let mut nesting = AlignmentNesting::default();
    let record = |transition, alignment| AlignmentRecord {
        transition,
        alignment: Some(alignment),
        align_state: 0,
        delimiter: None,
        previous_align_state: None,
    };

    assert_eq!(nesting.observe(&record("begin", 1)), Some(1));
    assert_eq!(nesting.observe(&record("suspend", 1)), Some(1));
    assert_eq!(nesting.observe(&record("begin", 2)), Some(2));
    assert_eq!(nesting.observe(&record("finish", 2)), Some(2));
    assert_eq!(nesting.observe(&record("resume", 1)), Some(1));
    assert_eq!(nesting.observe(&record("finish", 1)), Some(1));

    // Identity allocation is monotonic, but TeX82 §37's `pop_alignment`
    // removes the completed structural level before this new alignment.
    assert_eq!(nesting.observe(&record("begin", 3)), Some(1));
}

#[test]
fn catcode_mutations_use_canonical_assignment_names_and_scope() {
    let event = translate_mutation(MutationRecord {
        target: "catcode",
        value: "123=1".into(),
        key: None,
        tokens: None,
        global: true,
    });
    assert_eq!(
        event,
        Event::Mutation(MutationEvent {
            target: StateTarget::Catcode,
            key: CanonicalValue::Character(123),
            value: CanonicalValue::Name("left_brace".into()),
            scope: "global".into(),
        })
    );
    assert_eq!(canonical_catcode_assignment("16"), None);
}

#[test]
fn token_register_mutations_keep_the_frozen_list() {
    let event = translate_mutation(MutationRecord {
        target: "register",
        value: "tokens".into(),
        key: Some("toks:0".into()),
        tokens: Some(vec![ObservedToken::Character {
            character: 'X',
            catcode: Catcode::Letter,
        }]),
        global: false,
    });
    assert_eq!(
        event,
        Event::Mutation(MutationEvent {
            target: StateTarget::Register,
            key: CanonicalValue::Name("toks:0".into()),
            value: CanonicalValue::Tokens(vec![OracleToken {
                character: u32::from('X'),
                catcode: "letter".into(),
                control_sequence: None,
                location: None,
            }]),
            scope: "local".into(),
        })
    );
}

#[test]
fn sparse_box_mutations_keep_the_named_state() {
    let event = translate_mutation(MutationRecord {
        target: "register",
        value: "name:occupied".into(),
        key: Some("box:300".into()),
        tokens: None,
        global: true,
    });
    assert_eq!(
        event,
        Event::Mutation(MutationEvent {
            target: StateTarget::Register,
            key: CanonicalValue::Name("box:300".into()),
            value: CanonicalValue::Name("occupied".into()),
            scope: "global".into(),
        })
    );
}

#[test]
fn meaning_mutations_keep_the_assigned_control_sequence() {
    let event = translate_mutation(MutationRecord {
        target: "meaning",
        value: "begin_group".into(),
        key: Some("alignmentbegingroup".into()),
        tokens: None,
        global: false,
    });
    assert_eq!(
        event,
        Event::Mutation(MutationEvent {
            target: StateTarget::Meaning,
            key: CanonicalValue::Name("alignmentbegingroup".into()),
            value: CanonicalValue::Name("begin_group".into()),
            scope: "local".into(),
        })
    );
}

#[test]
fn toksdef_meanings_project_as_assign_toks() {
    let event = translate_mutation(MutationRecord {
        target: "meaning",
        value: "assign_toks".into(),
        key: Some("tokens".into()),
        tokens: None,
        global: false,
    });
    assert_eq!(
        event,
        Event::Mutation(MutationEvent {
            target: StateTarget::Meaning,
            key: CanonicalValue::Name("tokens".into()),
            value: CanonicalValue::Name("assign_toks".into()),
            scope: "local".into(),
        })
    );
}

#[test]
fn glue_scanners_and_mutations_keep_structured_orders() {
    // The producer already spells tex.web §135's order names; the
    // transport carries them through verbatim rather than re-casing a
    // Rust `Debug` rendering (`umber2-johp.141`).
    let value = "width=131072;stretch=196608;stretch_order=fil;shrink=262144;shrink_order=normal";
    let expected = CanonicalValue::Glue {
        width: 131_072,
        stretch: 196_608,
        stretch_order: "fil".into(),
        shrink: 262_144,
        shrink_order: "normal".into(),
    };
    assert_eq!(parse_glue_scanner_value(value), Some(expected.clone()));
    assert_eq!(
        translate_mutation(MutationRecord {
            target: "register",
            value: format!("glue:{value}"),
            key: Some("skip:0".into()),
            tokens: None,
            global: false,
        }),
        Event::Mutation(MutationEvent {
            target: StateTarget::Register,
            key: CanonicalValue::Name("skip:0".into()),
            value: expected.clone(),
            scope: "local".into(),
        })
    );
    assert_eq!(
        translate_mutation(MutationRecord {
            target: "parameter",
            value: format!("glue:{value}"),
            key: Some("glue_parameter:11".into()),
            tokens: None,
            global: false,
        }),
        Event::Mutation(MutationEvent {
            target: StateTarget::Parameter,
            key: CanonicalValue::Name("glue_parameter:11".into()),
            value: expected,
            scope: "local".into(),
        })
    );
}

#[test]
fn glue_component_enquiry_results_keep_their_typed_values() {
    let root = LiveSource {
        name: "root.tex".into(),
        source: SourceId::new(7),
        bytes: Arc::from(&b""[..]),
    };
    let mut translator = LiveSessionTranslator::for_root(SchemaVersion::V1, "terminal", root);
    translator.translate_captured([
        CommandObservation::Scanner(ScannerRecord {
            kind: "glue_stretch_order",
            value: "2".into(),
            tokens: None,
        }),
        CommandObservation::Scanner(ScannerRecord {
            kind: "glue_shrink",
            value: "196608".into(),
            tokens: None,
        }),
    ]);
    assert_eq!(
        translator.events[0].event,
        Event::Scanner(ScannerEvent {
            scanner: "glue_stretch_order".into(),
            result: CanonicalValue::Integer(2),
        })
    );
    assert_eq!(
        translator.events[1].event,
        Event::Scanner(ScannerEvent {
            scanner: "glue_shrink".into(),
            result: CanonicalValue::Scaled(196_608),
        })
    );
}
