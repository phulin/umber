use crate::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, ConditionEvent, ConditionTransition, DiagnosticEvent, DiagnosticSeverity,
    EffectEvent, EffectKind, Event, EventAnchorKey, EventClass, EventLocation, EventLocationMut,
    GeometryEvent, GeometryLocation, InputEvent, InputReason, InputTransition, MacroEvent,
    MutationEvent, OracleToken, RecoveryEvent, RecoveryKind, ScannerEvent, ScannerStatus,
    ScannerStatusEvent, SourceLocation, StateTarget, TokenListEvent, TokenListTransition,
};

fn location(line: u32) -> SourceLocation {
    SourceLocation {
        source: "nested\rsource.tex".into(),
        line,
        byte: line + 1,
    }
}

fn token(line: u32) -> OracleToken {
    OracleToken {
        character: 65,
        catcode: "letter\r".into(),
        control_sequence: None,
        location: Some(location(line)),
    }
}

fn every_event() -> Vec<Event> {
    vec![
        Event::Command(CommandEvent {
            delivery: CommandDelivery::Raw,
            command: CanonicalCommand {
                command: "letter\r".into(),
                operand: CanonicalValue::Token(token(2)),
                control_sequence: None,
                location: Some(location(1)),
            },
        }),
        Event::Input(InputEvent {
            transition: InputTransition::Push,
            reason: InputReason::Source,
            name: "main\r.tex".into(),
        }),
        Event::Recovery(RecoveryEvent {
            kind: RecoveryKind::Backup,
            tokens: vec![token(3)],
        }),
        Event::ScannerStatus(ScannerStatusEvent {
            from: ScannerStatus::Normal,
            to: ScannerStatus::Defining,
        }),
        Event::Macro(MacroEvent::Argument {
            parameter: 1,
            tokens: vec![token(4)],
        }),
        Event::Macro(MacroEvent::Activation {
            control_sequence: "macro".into(),
            argument_count: 1,
        }),
        Event::Condition(ConditionEvent {
            transition: ConditionTransition::Branch,
            condition: "if\r".into(),
            limit: "else\r".into(),
            branch: Some("true\r".into()),
        }),
        Event::Scanner(ScannerEvent {
            scanner: "integer\r".into(),
            result: CanonicalValue::Tokens(vec![token(5)]),
        }),
        Event::TokenList(TokenListEvent {
            transition: TokenListTransition::Complete,
            purpose: "write\r".into(),
            tokens: vec![token(6)],
        }),
        Event::Alignment(AlignmentEvent {
            transition: AlignmentTransition::Begin,
            align_state: 0,
            template: Some("u\r".into()),
            nesting: Some(1),
            previous_align_state: None,
            delimiter: None,
            recovery: None,
        }),
        Event::Mutation(MutationEvent {
            target: StateTarget::Register,
            key: CanonicalValue::Token(token(7)),
            value: CanonicalValue::Tokens(vec![token(8)]),
            scope: "global\r".into(),
        }),
        Event::Diagnostic(DiagnosticEvent {
            severity: DiagnosticSeverity::Error,
            diagnostic: "missing\r".into(),
            arguments: vec![CanonicalValue::Token(token(9))],
        }),
        Event::Effect(EffectEvent {
            kind: EffectKind::Write,
            channel: "log\r".into(),
            value: CanonicalValue::Token(token(10)),
        }),
        Event::Geometry(GeometryEvent::Hpack {
            width_sp: 1,
            height_sp: 2,
            depth_sp: 3,
            location: Some(GeometryLocation {
                source: "main\r.tex".into(),
                line: 11,
            }),
        }),
    ]
}

#[test]
fn views_classify_every_schema_carrier() {
    let classes = every_event()
        .iter()
        .map(|event| event.view().class())
        .collect::<Vec<_>>();
    assert_eq!(
        classes,
        [
            EventClass::Command,
            EventClass::Input,
            EventClass::Recovery,
            EventClass::ScannerStatus,
            EventClass::Macro,
            EventClass::Macro,
            EventClass::Condition,
            EventClass::Scanner,
            EventClass::TokenList,
            EventClass::Alignment,
            EventClass::Mutation,
            EventClass::Diagnostic,
            EventClass::Effect,
            EventClass::Geometry,
        ]
    );
}

#[test]
fn nested_location_walk_and_erasure_cover_all_carriers() {
    let mut events = every_event();
    let mut source_locations = 0;
    let mut geometry_locations = 0;
    for event in &mut events {
        event
            .view()
            .visit_locations(&mut |location| match location {
                EventLocation::Source(_) => source_locations += 1,
                EventLocation::Geometry(_) => geometry_locations += 1,
            });
        event
            .view_mut()
            .visit_locations(&mut |location| match location {
                EventLocationMut::Source(location) => location.line += 100,
                EventLocationMut::Geometry(location) => location.line += 100,
            });
        let erased = event.without_locations();
        erased
            .view()
            .visit_locations(&mut |_| panic!("erased event retained a location"));
    }
    assert_eq!(source_locations, 10);
    assert_eq!(geometry_locations, 1);
}

#[test]
fn mutable_view_normalizes_every_textual_carrier_without_erasing_payloads() {
    for mut event in every_event() {
        let before = event.without_locations();
        event.view_mut().normalize();
        let after = event.without_locations();
        assert!(!format!("{after:?}").contains('\r'));
        assert_eq!(before.view().class(), after.view().class());
    }
}

#[test]
fn schema_owned_keys_retain_historical_identity_rules() {
    let events = every_event();
    assert!(matches!(
        events[0].view().anchor_key(),
        Some(EventAnchorKey::Line {
            source: "nested\rsource.tex",
            line: 1
        })
    ));
    assert!(matches!(
        events[1].view().anchor_key(),
        Some(EventAnchorKey::Input { .. })
    ));
    assert!(events[2].view().anchor_key().is_none());
    assert_ne!(
        events[0].view().alignment_key(),
        events[1].view().alignment_key()
    );
}

#[test]
fn concise_rendering_is_unicode_safe_and_preserves_short_debug_bytes() {
    let event = Event::Input(InputEvent {
        transition: InputTransition::Push,
        reason: InputReason::Source,
        name: "ééé".into(),
    });
    assert_eq!(event.concise(usize::MAX).to_string(), format!("{event:?}"));
    assert_eq!(event.concise(4).to_string().chars().count(), 5);
    assert!(event.concise(4).to_string().ends_with('…'));
}
