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
    let definition_publication = definition;
    let definition = definition_publication.reference();
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
fn nested_readers(
    observed: bool,
    main_control: bool,
    fused: bool,
    limit: u64,
) -> (String, u64, bool) {
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
        let exhausted = if fused {
            let mut consume = |_: &mut crate::CommandProcessor<'_, '_, _>,
                               spelling: tex_state::token::TracedTokenWord,
                               _: &tex_state::meaning::MeaningWord<_>,
                               _,
                               _: Option<&crate::CurrentCommand<_>>| {
                let Token::Char { ch, .. } = spelling.semantic_token() else {
                    panic!("expected expanded character");
                };
                output.push(ch);
                Ok(std::ops::ControlFlow::Continue(()))
            };
            let result = if main_control {
                processor.consume_main_control(&mut consume).map(|status| {
                    assert_eq!(status, crate::DeliveryStatus::End);
                })
            } else {
                processor.consume_expanded_words(&mut consume)
            };
            assert_eq!(processor.command.transient.active_expansion_depth, 0);
            assert_eq!(processor.expansion_depth, 0);
            match result {
                Ok(()) => false,
                Err(crate::CommandError::FuelExhausted(_)) => true,
                Err(error) => panic!("unexpected delivery error: {error:?}"),
            }
        } else {
            loop {
                let result = if main_control {
                    processor.main_control_expanded_into(&mut destination)
                } else {
                    processor.get_x_token_into(&mut destination)
                };
                match result {
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
                    Err(crate::CommandError::FuelExhausted(_)) => {
                        assert!(destination.is_none());
                        assert_eq!(processor.command.transient.active_expansion_depth, 0);
                        assert_eq!(processor.expansion_depth, 0);
                        break true;
                    }
                    Err(error) => panic!("unexpected delivery error: {error:?}"),
                }
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
    let ordinary = nested_readers(false, false, false, 100);
    assert_eq!(ordinary.0, "ALBCRD");
    assert!(!ordinary.2);
    for observed in [false, true] {
        for main_control in [false, true] {
            assert_eq!(nested_readers(observed, main_control, false, 100), ordinary);
        }
    }
}

#[test]
fn observation_specialization_preserves_every_nested_fuel_cut() {
    for limit in 1..=20 {
        let ordinary = nested_readers(false, false, false, limit);
        for observed in [false, true] {
            for main_control in [false, true] {
                assert_eq!(
                    nested_readers(observed, main_control, false, limit),
                    ordinary,
                    "fuel {limit}"
                );
            }
        }
    }
}

#[test]
fn synchronous_consumer_preserves_nested_readers_and_every_fuel_cut() {
    for limit in (1..=20).chain([100]) {
        let ordinary = nested_readers(false, false, false, limit);
        for observed in [false, true] {
            for main_control in [false, true] {
                assert_eq!(
                    nested_readers(observed, main_control, true, limit),
                    ordinary,
                    "fuel {limit}"
                );
            }
        }
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
            crate::test_harness::expect_expanded_command(&mut processor)
                .spelling()
                .semantic_token(),
            letter('A')
        );
        let mut processor = processor.with_observer(&mut observer);
        for expected in ['B', 'C'] {
            let current = crate::test_harness::expect_expanded_command(&mut processor);
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
        let definition_publication = definition;
        let definition = definition_publication.reference();
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
        let first = crate::test_harness::expect_expanded_command(&mut processor);
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
            let next = crate::test_harness::expect_expanded_command(&mut processor);
            assert_eq!(next.meaning(), Meaning::CharGiven('B'));
        }
        assert_eq!(
            crate::test_harness::expect_expanded_command(&mut processor)
                .spelling()
                .semantic_token(),
            letter('Z')
        );
    });
}

#[test]
fn reused_raw_destination_is_cleared_at_eof_and_fuel_failure() {
    for limit in [2, 3] {
        crate::test_harness::with_universe(|universe| {
            let mut command = CommandState::default();
            crate::test_harness::push(&mut command, [letter('A'), letter('B')]);
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(limit).expect("fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let mut destination = None;
            for expected in ['A', 'B'] {
                assert_eq!(
                    processor
                        .raw_next_hot(&mut destination)
                        .expect("raw command"),
                    crate::DeliveryStatus::Command,
                );
                // Leave the previous command occupied for the next fetch.
                assert_eq!(
                    destination
                        .as_ref()
                        .expect("command")
                        .spelling_word()
                        .semantic_token(),
                    letter(expected),
                );
            }
            let result = processor.raw_next_hot(&mut destination);
            if limit == 2 {
                assert!(matches!(result, Err(crate::CommandError::FuelExhausted(_))));
            } else {
                assert_eq!(result.expect("EOF"), crate::DeliveryStatus::End);
            }
            assert!(destination.is_none());
        });
    }
}

#[test]
fn native_main_lookahead_preserves_mixed_expansion_and_observed_fuel_cuts() {
    fn run(
        native: bool,
        observed: bool,
        limit: u64,
    ) -> (Vec<Token>, u64, bool, Vec<(CommandDeliveryBoundary, u64)>) {
        crate::test_harness::with_universe(|universe| {
            let macro_token = install_identity(universe, "identity", &[Token::Param(1)]);
            let noexpand = install_static(
                universe,
                "noexpand",
                Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
            );
            let given = install_static(universe, "given", Meaning::CharGiven('G'));
            let relax = install_static(universe, "relax", Meaning::Relax);
            let mut command = CommandState::default();
            crate::test_harness::push(
                &mut command,
                [
                    letter('A'),
                    given,
                    macro_token,
                    letter('B'),
                    Token::Char {
                        ch: ' ',
                        cat: Catcode::Space,
                    },
                    noexpand,
                    macro_token,
                    relax,
                    letter('C'),
                ],
            );
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
            let mut output = Vec::new();
            let result = if native {
                processor.consume_main_loop_lookahead(&mut |_, spelling, _, _, _| {
                    output.push(spelling.semantic_token());
                    Ok(std::ops::ControlFlow::Continue(()))
                })
            } else {
                let mut destination = None;
                loop {
                    match processor.main_loop_lookahead_into(&mut destination) {
                        Ok(crate::DeliveryStatus::Command) => output.push(
                            destination
                                .take()
                                .expect("command")
                                .spelling()
                                .semantic_token(),
                        ),
                        result => break result,
                    }
                }
            };
            let exhausted = match result {
                Ok(crate::DeliveryStatus::End) => false,
                Err(crate::CommandError::FuelExhausted(_)) => true,
                result => panic!("unexpected lookahead boundary: {result:?}"),
            };
            assert_eq!(processor.command.transient.active_expansion_depth, 0);
            assert_eq!(processor.expansion_depth, 0);
            let burned = processor.fuel.burned();
            drop(processor);
            let deliveries = observer
                .0
                .into_iter()
                .filter_map(|record| {
                    let CommandObservation::Command(record) = record else {
                        return None;
                    };
                    Some((record.boundary, record.provenance.delivery_sequence))
                })
                .collect();
            (output, burned, exhausted, deliveries)
        })
    }
    for observed in [false, true] {
        for limit in (1..=20).chain([100]) {
            assert_eq!(
                run(true, observed, limit),
                run(false, observed, limit),
                "observed {observed}, fuel {limit}"
            );
        }
    }
}

#[test]
fn native_primitive_and_terminal_runs_construct_no_commands() {
    struct Observer;
    impl CommandObserver for Observer {
        fn committed(&mut self, _: CommandObservation) {}
    }
    for (observed, main_control, raw_spelling, lookahead) in [
        (false, false, false, false),
        (true, false, false, false),
        (false, true, false, false),
        (true, true, false, false),
        (false, false, true, false),
        (true, false, true, false),
        (false, false, false, true),
        (true, false, false, true),
    ] {
        for count in [1, 4096] {
            crate::test_harness::with_universe(|universe| {
                let iftrue = install_static(
                    universe,
                    "iftrue-native",
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue),
                );
                let fi = install_static(
                    universe,
                    "fi-native",
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
                );
                let letter = Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                };
                let mut command = CommandState::default();
                crate::test_harness::push(
                    &mut command,
                    if raw_spelling || lookahead {
                        vec![letter; count]
                    } else {
                        [iftrue, letter, fi]
                            .into_iter()
                            .cycle()
                            .take(count * 3)
                            .collect()
                    },
                );
                let mut capabilities = CommandHostCapabilities::default();
                let mut fuel = crate::CommandFuelLedger::default();
                let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                let mut observer = Observer;
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
                let before = crate::command::command_ownership_counters();
                let mut delivered = 0;
                let mut consume =
                    |_: &mut crate::CommandProcessor<'_, '_, _>,
                     spelling: tex_state::token::TracedTokenWord,
                     _: &tex_state::meaning::MeaningWord<_>,
                     _,
                     record: Option<&crate::CurrentCommand<_>>| {
                        assert_eq!(spelling.semantic_token(), letter);
                        assert_eq!(record.is_some(), observed);
                        delivered += 1;
                        Ok(std::ops::ControlFlow::Continue(()))
                    };
                if raw_spelling {
                    while let Some(word) =
                        processor.get_token_spelling().expect("native raw spelling")
                    {
                        assert_eq!(word.semantic_token(), letter);
                        delivered += 1;
                    }
                } else if lookahead {
                    assert_eq!(
                        processor
                            .consume_main_loop_lookahead(&mut consume)
                            .expect("native raw main lookahead"),
                        crate::DeliveryStatus::End,
                    );
                } else if main_control {
                    assert_eq!(
                        processor
                            .consume_main_control(&mut consume)
                            .expect("native main control"),
                        crate::DeliveryStatus::End,
                    );
                } else {
                    processor
                        .consume_expanded_words(&mut consume)
                        .expect("expanded native run");
                }
                assert_eq!(delivered, count);
                assert!(processor.command.conditions.current().is_none());
                let after = crate::command::command_ownership_counters();
                assert_eq!(
                    after.resolved_writes - before.resolved_writes,
                    count as u64
                        * if !observed {
                            0
                        } else if raw_spelling || lookahead {
                            1
                        } else {
                            3
                        },
                    "native expansion and actual terminal consumption need no command; observation does",
                );
                assert_eq!(
                    processor.fuel.burned(),
                    count as u64 * if raw_spelling || lookahead { 1 } else { 3 } + 1
                );
            });
        }
    }
}
