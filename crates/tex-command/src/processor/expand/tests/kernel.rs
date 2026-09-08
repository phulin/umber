//! Behavioral contracts at reader and observation admission boundaries.

use super::*;

fn install_identity<G>(universe: &mut tex_state::Universe<G>, name: &str, body: &[Token]) -> Token {
    let definition = universe
        .allocate_definition(
            &[TokenWord::pack(Token::Param(1))],
            &body
                .iter()
                .copied()
                .map(TokenWord::pack)
                .collect::<Vec<_>>(),
        )
        .expect("definition");
    let symbol = universe.intern(name).expect("macro name");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
            AssignmentScope::Global,
        )
        .expect("macro meaning");
    Token::Cs(symbol.symbol())
}

// A source parent, replacement parent and argument parent all resume after
// nested input. Fuel cuts exercise every intermediate return/unwind position.
fn nested_readers(observed: bool, limit: u64) -> (String, u64, bool) {
    crate::test_harness::with_universe(|universe| {
        for (ch, cat) in [('{', Catcode::BeginGroup), ('}', Catcode::EndGroup)] {
            universe
                .assign_code(
                    tex_state::env::CodeTableKind::Catcode,
                    ch,
                    i64::from(cat as u8),
                    AssignmentScope::Global,
                )
                .expect("brace catcode");
        }
        let inner = install_identity(universe, "inner", &[Token::Param(1)]);
        install_identity(
            universe,
            "outer",
            &[letter('L'), inner, Token::Param(1), letter('R')],
        );
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"A\\outer{BC}D%"[..],
            ))
            .expect("fragment registration");
        command.open_registered_source(source).expect("source");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(limit).expect("fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("context");
        let processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let mut processor = if observed {
            processor.with_observer(&mut observer)
        } else {
            processor
        };
        let mut output = String::new();
        let mut destination = None;
        let exhausted = loop {
            match processor.get_x_token_into(&mut destination) {
                Ok(crate::DeliveryStatus::End) => {
                    assert!(destination.is_none());
                    break false;
                }
                Ok(_) => {
                    let current = destination.take().expect("delivered command");
                    let Token::Char { ch, .. } = current.spelling().semantic_token() else {
                        panic!("expected expanded character");
                    };
                    output.push(ch);
                }
                Err(crate::CommandError::FuelExhausted { .. }) => {
                    assert!(destination.is_none());
                    assert_eq!(processor.command.transient.active_expansion_depth, 0);
                    assert_eq!(processor.expansion_depth, 0);
                    break true;
                }
                Err(error) => panic!("unexpected delivery error: {error:?}"),
            }
        };
        let burned = processor.fuel.burned();
        drop(processor);
        if observed {
            assert!(!observer.0.is_empty());
        } else {
            assert!(observer.0.is_empty());
        }
        (output, burned, exhausted)
    })
}

#[test]
fn input_frame_readers_resume_identically_with_and_without_observation() {
    let ordinary = nested_readers(false, 100);
    assert_eq!(ordinary.0, "ALBCRD");
    assert!(!ordinary.2);
    assert_eq!(nested_readers(true, 100), ordinary);
}

#[test]
fn observation_specialization_preserves_every_nested_fuel_cut() {
    for limit in 1..=20 {
        assert_eq!(
            nested_readers(true, limit),
            nested_readers(false, limit),
            "fuel {limit}"
        );
    }
}

#[test]
fn attaching_observer_between_deliveries_selects_the_new_episode() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [letter('A'), letter('B'), letter('C')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(3).expect("fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        assert_eq!(
            processor
                .get_x_token()
                .expect("A")
                .expect("command")
                .spelling()
                .semantic_token(),
            letter('A')
        );
        let mut processor = processor.with_observer(&mut observer);
        for expected in ['B', 'C'] {
            let current = processor
                .get_x_token()
                .expect("observed token")
                .expect("command");
            assert_eq!(current.spelling().semantic_token(), letter(expected));
        }
        assert_eq!(processor.fuel.burned(), 3);
        drop(processor);
        let deliveries = observer
            .0
            .iter()
            .filter_map(|record| {
                let CommandObservation::Command(record) = record else {
                    return None;
                };
                Some((record.boundary, record.provenance.delivery_sequence))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            deliveries,
            [
                (CommandDeliveryBoundary::Raw, 0),
                (CommandDeliveryBoundary::Expanded, 0),
                (CommandDeliveryBoundary::Raw, 1),
                (CommandDeliveryBoundary::Expanded, 1),
            ]
        );
    });
}

#[test]
fn resumed_replacement_reads_the_live_meaning_after_backup() {
    crate::test_harness::with_universe(|universe| {
        let variable = install_static(universe, "variable", Meaning::CharGiven('A'));
        let Token::Cs(symbol) = variable else {
            unreachable!()
        };
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(variable), TokenWord::pack(variable)])
            .expect("replacement");
        let name = universe.intern("twice").expect("macro name");
        universe
            .assign_meaning(
                name,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro");
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [Token::Cs(name.symbol()), letter('Z')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let first = processor.get_x_token().expect("first").expect("command");
        assert_eq!(first.meaning(), Meaning::CharGiven('A'));
        processor.back_input(first).expect("backup");
        processor
            .state
            .assign_resolved_meaning(
                symbol,
                tex_state::meaning::ResolvedMeaning::Static(Meaning::CharGiven('B')),
                AssignmentScope::Global,
            )
            .expect("new live meaning");
        for _ in 0..2 {
            let next = processor.get_x_token().expect("resumed").expect("command");
            assert_eq!(next.meaning(), Meaning::CharGiven('B'));
        }
        assert_eq!(
            processor
                .get_x_token()
                .expect("parent")
                .expect("command")
                .spelling()
                .semantic_token(),
            letter('Z')
        );
    });
}
