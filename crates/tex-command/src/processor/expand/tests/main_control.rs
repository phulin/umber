//! Main-control command-slot reuse and bounded expansion work.

use super::*;

#[test]
#[cfg(feature = "profiling")]
fn one_and_4096_main_control_expansions_reuse_one_slot_with_exact_linear_work() {
    let one = empty_macro_delivery_evidence(1);
    let many = empty_macro_delivery_evidence(4_096);

    for (expansions, evidence) in [(1, one), (4_096, many)] {
        // The first macro and every later activation consume only resolved
        // invocation facts; only the terminal character becomes a command.
        assert_eq!(evidence.resolved_writes, 1);
        assert_eq!(evidence.expanded_classifications, 1);
        assert_eq!(evidence.token_frame_steps, expansions + 1);
        assert_eq!(evidence.meaning_lookups, expansions);
        assert_eq!(evidence.expanded_deliveries, 1);
        #[cfg(feature = "profiling")]
        {
            assert_eq!(evidence.allocations, 0);
            assert_eq!(evidence.allocated_bytes, 0);
        }
    }
}

#[test]
fn expandable_main_control_delivery_uses_one_caller_owned_command_slot() {
    crate::test_harness::with_universe(|universe| {
        let replacement = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        };
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("definition");
        let definition_publication = definition;
        let definition = definition_publication.reference();
        let symbol = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [Token::Cs(symbol.symbol())]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(2).expect("expanded delivery fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        let ownership_before = crate::command::command_ownership_counters();
        assert_eq!(
            processor
                .main_control_expanded_into(&mut destination)
                .expect("main_control delivery"),
            crate::DeliveryStatus::Command
        );
        let settled = destination
            .as_ref()
            .expect("expanded delivery occupies the caller destination");
        assert_eq!(settled.spelling().semantic_token(), replacement);
        assert_eq!(
            settled.meaning(),
            Meaning::CharToken {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
        assert_eq!(processor.fuel.burned(), 2);
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after.resolved_writes - ownership_before.resolved_writes,
            1
        );
    });
}

#[test]
fn source_macro_main_control_materializes_only_when_observed() {
    crate::test_harness::with_universe(|universe| {
        let replacement = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        };
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("definition");
        let definition_publication = definition;
        let definition = definition_publication.reference();
        let symbol = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");

        for observed in [false, true] {
            let mut command = CommandState::default();
            let source = command
                .register_source(crate::SourceRegistration::new(
                    crate::RegisteredSourceKind::Generated,
                    &b"\\m"[..],
                ))
                .expect("source registration");
            command
                .open_registered_source(source)
                .expect("source opening");
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(2).expect("main_control fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut observer = RecordingObserver::default();
            let mut context = universe.command_context().expect("command context");
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
            let mut destination = None;
            assert_eq!(
                processor
                    .main_control_expanded_into(&mut destination)
                    .expect("source macro main_control"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(
                destination
                    .expect("expanded terminal")
                    .spelling()
                    .semantic_token(),
                replacement
            );
            assert_eq!(processor.fuel.burned(), 2);
            let after = crate::command::command_ownership_counters();
            assert_eq!(
                after.resolved_writes - before.resolved_writes,
                if observed { 2 } else { 1 }
            );
            drop(processor);
            let boundaries = observer
                .0
                .iter()
                .filter_map(|observation| match observation {
                    CommandObservation::Command(record) => Some(record.boundary),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if observed {
                assert_eq!(
                    boundaries,
                    [
                        CommandDeliveryBoundary::Raw,
                        CommandDeliveryBoundary::Raw,
                        CommandDeliveryBoundary::Expanded,
                    ]
                );
            } else {
                assert!(boundaries.is_empty());
            }
        }
    });
}

// TeX's direct main-control fetch inserts/rereads frozen endv. Once a
// macro has expanded, or in a scanner, get_x_token converts it inline.
#[test]
fn end_template_preserves_raw_and_expanded_reader_boundaries_at_fuel_cuts() {
    for (main_control, native) in [(false, false), (true, false), (true, true)] {
        for expanded in [false, true] {
            for observed in [false, true] {
                for fuel_cut in [false, true] {
                    crate::test_harness::with_universe(|universe| {
                        let template = install_static(
                            universe,
                            "template",
                            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate),
                        );
                        let publication = universe
                            .allocate_definition(&[], &[TokenWord::pack(template)])
                            .expect("template macro");
                        let symbol = universe.intern("template_macro").expect("name");
                        universe
                            .assign_meaning(
                                symbol,
                                MeaningWord::macro_definition(
                                    MeaningFlags::EMPTY,
                                    publication.reference(),
                                ),
                                AssignmentScope::Global,
                            )
                            .expect("macro meaning");
                        let insertion = main_control && !expanded;
                        let charges = if expanded || insertion { 2 } else { 1 };
                        let mut command = CommandState::default();
                        crate::test_harness::push(
                            &mut command,
                            [
                                if expanded {
                                    Token::Cs(symbol.symbol())
                                } else {
                                    template
                                },
                                letter('Z'),
                            ],
                        );
                        let mut capabilities = CommandHostCapabilities::default();
                        let mut fuel = crate::CommandFuelLedger::new(if fuel_cut {
                            (charges - 1).max(1)
                        } else {
                            charges + 2
                        })
                        .expect("template fuel");
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
                        if fuel_cut && charges == 1 {
                            processor
                                .charge_command_action()
                                .expect("consume available fuel");
                        }
                        let mut destination = None;
                        let mut native_endv = false;
                        let result = if native {
                            processor.consume_main_control(&mut |_, spelling, meaning, _, _| {
                                assert!(spelling.semantic_token().is_frozen_endv());
                                assert_eq!(
                                    meaning.resolve(),
                                    tex_state::meaning::ResolvedMeaning::Static(Meaning::EndV)
                                );
                                native_endv = true;
                                Ok(std::ops::ControlFlow::Break(()))
                            })
                        } else if main_control {
                            processor.main_control_expanded_into(&mut destination)
                        } else {
                            processor.get_x_token_into(&mut destination)
                        };
                        if fuel_cut {
                            assert!(matches!(result, Err(crate::CommandError::FuelExhausted(_))));
                            assert!(destination.is_none());
                            assert!(!native_endv);
                        } else {
                            assert_eq!(
                                result.expect("template delivery"),
                                crate::DeliveryStatus::Command
                            );
                            if native {
                                assert!(native_endv);
                                assert!(destination.is_none());
                            } else {
                                let delivered = destination.take().expect("endv");
                                assert!(delivered.spelling().semantic_token().is_frozen_endv());
                                assert_eq!(delivered.meaning(), Meaning::EndV);
                            }
                            assert_eq!(processor.fuel.burned(), charges);
                            assert_eq!(
                                processor
                                    .get_x_token_into(&mut destination)
                                    .expect("parent resumes"),
                                crate::DeliveryStatus::Command
                            );
                            assert_eq!(
                                destination
                                    .take()
                                    .expect("parent token")
                                    .spelling()
                                    .semantic_token(),
                                letter('Z')
                            );
                        }
                        assert_eq!(processor.command.transient.active_expansion_depth, 0);
                        assert_eq!(processor.expansion_depth, 0);
                        drop(processor);
                        if observed && !fuel_cut {
                            let backups = observer.0.iter().filter(|record| matches!(record,
                                CommandObservation::Input(input) if input.transition == InputTransition::Backup
                            )).count();
                            assert_eq!(backups, usize::from(insertion));
                        } else if !observed {
                            assert!(observer.0.is_empty());
                        }
                    });
                }
            }
        }
    }
}

#[test]
fn empty_macro_main_control_reaches_end_without_an_expanded_command() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe.allocate_definition(&[], &[]).expect("definition");
        let definition_publication = definition;
        let definition = definition_publication.reference();
        let symbol = universe.intern("empty").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [Token::Cs(symbol.symbol())]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(2).expect("main_control fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let mut destination = None;
        assert_eq!(
            processor
                .main_control_expanded_into(&mut destination)
                .expect("empty macro main_control"),
            crate::DeliveryStatus::End
        );
        assert!(destination.is_none());
        assert_eq!(processor.fuel.burned(), 2);
    });
}

#[test]
fn unexpandable_main_control_classifies_once_and_reuses_one_slot() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(1).expect("raw delivery fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        let classifications_before = super::super::expanded_classifications();
        assert_eq!(
            processor
                .main_control_expanded_into(&mut destination)
                .expect("main_control delivery"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .as_ref()
                .expect("raw delivery occupies the caller destination")
                .spelling()
                .semantic_token(),
            token
        );
        assert_eq!(processor.fuel.burned(), 1);
        drop(processor);
        assert_eq!(
            super::super::expanded_classifications() - classifications_before,
            1
        );
    });
}

#[test]
#[cfg(feature = "profiling")]
fn raw_main_loop_exit_preserves_the_existing_expanded_work_boundary() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let work_before = fuel.work();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;

        assert_eq!(
            processor
                .main_loop_lookahead_into(&mut destination)
                .expect("raw main-loop exit"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .as_ref()
                .expect("main-loop exit occupies its caller destination")
                .spelling()
                .semantic_token(),
            token
        );
        drop(processor);
        assert_eq!(
            fuel.work().expanded_deliveries - work_before.expanded_deliveries,
            0
        );
    });
}
