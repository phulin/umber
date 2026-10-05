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
                .consume_expanded_with_mode::<false, false>(
                    &mut destination,
                    None,
                    super::super::ExpandedReaderRole::MainControl,
                    &mut super::super::DeliveredCommand
                )
                .expect("main_control delivery"),
            crate::DeliveryStatus::PendingExpanded
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
                    .consume_expanded_with_mode::<false, false>(
                        &mut destination,
                        None,
                        super::super::ExpandedReaderRole::MainControl,
                        &mut super::super::DeliveredCommand
                    )
                    .expect("source macro main_control"),
                crate::DeliveryStatus::PendingExpanded
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

// TeX82 §380 changes cur_cs/cur_cmd to frozen endv in place. Only actual
// input words consume fuel; there is no backed-up replacement token.
#[test]
fn end_template_preserves_raw_and_expanded_reader_boundaries_at_fuel_cuts() {
    for (main_control, native, lookahead) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, true, true),
    ] {
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
                        let charges = (if expanded { 2 } else { 1 }) + u64::from(lookahead);
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
                            let mut consume =
                                |_: &mut crate::CommandProcessor<'_, '_, _, _>,
                                 spelling: tex_state::token::TracedTokenWord,
                                 meaning: &MeaningWord<_>,
                                 _: u64,
                                 _: Option<tex_state::interner::Symbol>,
                                 _: Option<&crate::CurrentCommand<_>>| {
                                    assert!(spelling.semantic_token().is_frozen_endv());
                                    assert_eq!(
                                        meaning.resolve(),
                                        tex_state::meaning::ResolvedMeaning::Static(Meaning::EndV)
                                    );
                                    native_endv = true;
                                    Ok(std::ops::ControlFlow::Break(()))
                                };
                            if lookahead {
                                processor.consume_main_loop_lookahead(&mut consume)
                            } else {
                                processor.consume_main_control(&mut consume)
                            }
                        } else if main_control {
                            processor.consume_expanded_with_mode::<false, false>(
                                &mut destination,
                                None,
                                super::super::ExpandedReaderRole::MainControl,
                                &mut super::super::DeliveredCommand,
                            )
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
                                if main_control && !native && expanded {
                                    crate::DeliveryStatus::PendingExpanded
                                } else {
                                    crate::DeliveryStatus::Command
                                }
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
                            assert_eq!(backups, usize::from(lookahead));
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
                .consume_expanded_with_mode::<false, false>(
                    &mut destination,
                    None,
                    super::super::ExpandedReaderRole::MainControl,
                    &mut super::super::DeliveredCommand
                )
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
                .consume_expanded_with_mode::<false, false>(
                    &mut destination,
                    None,
                    super::super::ExpandedReaderRole::MainControl,
                    &mut super::super::DeliveredCommand
                )
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
fn main_loop_lookahead_counts_only_x_token_deliveries() {
    crate::test_harness::with_universe(|universe| {
        // TeX82 section 1038 returns letters directly after get_next. A
        // non-character such as a space reaches x_token before reswitch.
        for (token, expected_expanded_deliveries) in [
            (
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
                0,
            ),
            (
                Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                },
                1,
            ),
        ] {
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
                    .consume_expanded_with_mode::<false, false>(
                        &mut destination,
                        None,
                        super::super::ExpandedReaderRole::MainLoopLookahead,
                        &mut super::super::DeliveredCommand
                    )
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
                expected_expanded_deliveries
            );
        }
    });
}

#[test]
fn native_main_control_continues_across_macro_bodies_without_command_records() {
    crate::test_harness::with_universe(|universe| {
        let relax = universe.intern("relax").expect("relax symbol");
        universe
            .assign_meaning(
                relax,
                MeaningWord::scalar(Meaning::RELAX_WORD),
                AssignmentScope::Global,
            )
            .expect("relax meaning");
        let body = [TokenWord::pack(Token::Cs(relax.symbol())); 2];
        let definition = universe
            .allocate_definition(&[], &body)
            .expect("definition");
        let name = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                name,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition.reference()),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                Token::Cs(name.symbol()),
                Token::Cs(name.symbol()),
                Token::Cs(name.symbol()),
                Token::Cs(name.symbol()),
                Token::Char {
                    ch: 'Z',
                    cat: Catcode::Letter,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(13).expect("exact fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let before = crate::command::command_ownership_counters();
        let mut terminal = None;
        let mut consumed = 0;
        assert_eq!(
            processor
                .consume_main_control(&mut crate::terminal_consumer::<
                    crate::CommandProcessor<'_, '_, _>,
                    _,
                    _,
                >(
                    |_, spelling, meaning, _code, _, exceptional| {
                        assert!(exceptional.is_none());
                        if meaning.scalar_word() == Some(Meaning::RELAX_WORD) {
                            consumed += 1;
                            Ok(std::ops::ControlFlow::Continue(()))
                        } else {
                            terminal = Some(spelling.semantic_token());
                            Ok(std::ops::ControlFlow::Break(()))
                        }
                    }
                ))
                .expect("native continuation"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(consumed, 8);
        assert_eq!(processor.fuel.burned(), 13);
        assert_eq!(
            terminal.expect("actual consumed terminal"),
            Token::Char {
                ch: 'Z',
                cat: Catcode::Letter
            }
        );
        let after = crate::command::command_ownership_counters();
        assert_eq!(
            after.resolved_writes - before.resolved_writes,
            0,
            "native terminals need no command records"
        );
    });
}
