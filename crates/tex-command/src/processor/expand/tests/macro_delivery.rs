//! Macro and active-character delivery through the hot owner.

use super::*;

#[test]
fn parameterless_macro_expands_from_a_generation_typed_definition() {
    crate::test_harness::with_universe(|universe| {
        let replacement = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        };
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("definition");
        let symbol = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [Token::Cs(symbol.symbol())]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let expanded = crate::test_harness::expect_expanded_command(&mut processor);
        assert_eq!(expanded.spelling().semantic_token(), replacement);
        assert_eq!(
            expanded.meaning(),
            Meaning::CharToken {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        {
            let mut destination = None;
            assert_eq!(
                processor.get_x_token_into(&mut destination).expect("end"),
                crate::DeliveryStatus::End
            );
            assert!(destination.is_none());
        };
    });
}

#[test]
fn active_character_unexpandable_result_preserves_origin_and_backs_up_once() {
    crate::test_harness::with_universe(|universe| {
        let active = universe
            .intern_active_character('~')
            .expect("active character");
        universe
            .assign_meaning(
                active,
                MeaningWord::from_static(Meaning::CharGiven('A')),
                AssignmentScope::Global,
            )
            .expect("active character meaning");

        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"x"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        )
        .with_observer(&mut observer);

        let source_command = crate::test_harness::expect_raw_command(&mut processor);
        let origin = source_command.origin();
        assert_ne!(origin, OriginId::UNKNOWN);
        let fuel_before_treatment = processor.fuel.burned();
        processor
            .treat_as_active_character('~', origin)
            .expect("active-character treatment");
        assert_eq!(processor.fuel.burned(), fuel_before_treatment);

        let backed_up = crate::test_harness::expect_raw_command(&mut processor);
        assert_eq!(
            backed_up.spelling().semantic_token(),
            Token::Char {
                ch: '~',
                cat: Catcode::Active,
            }
        );
        assert_eq!(backed_up.origin(), origin);
        drop(processor);

        let expanded = observer
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Command(record)
                    if record.boundary == CommandDeliveryBoundary::Expanded =>
                {
                    Some(record)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].provenance.origin, origin);
        assert!(expanded[0].provenance.has_origin);
        assert_eq!(
            observer
                .0
                .iter()
                .filter(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Recovery(record) if record.kind == RecoveryKind::Backup
                    )
                })
                .count(),
            1,
        );
        assert!(observer.0.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Backup
            )
        }));
    });
}

#[test]
fn active_character_empty_macro_retires_replay_before_settling_next_command() {
    crate::test_harness::with_universe(|universe| {
        let active = universe
            .intern_active_character('~')
            .expect("active character");
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty active macro definition");
        universe
            .assign_meaning(
                active,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("active macro meaning");

        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [Token::Char {
                ch: 'B',
                cat: Catcode::Letter,
            }],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        processor
            .treat_as_active_character('~', OriginId::UNKNOWN)
            .expect("empty active macro treatment");
        let backed_up = crate::test_harness::expect_raw_command(&mut processor);
        assert_eq!(
            backed_up.spelling().semantic_token(),
            Token::Char {
                ch: 'B',
                cat: Catcode::Letter,
            }
        );
        {
            let mut destination = None;
            assert_eq!(
                processor
                    .get_next_into(&mut destination)
                    .expect("end of input"),
                crate::DeliveryStatus::End
            );
            assert!(destination.is_none());
        };
    });
}

#[test]
fn one_hundred_macros_materialize_only_the_final_command() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty definition");
        let symbol = universe.intern("hotchain").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let terminal = Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        };
        let mut input = vec![Token::Cs(symbol.symbol()); 100];
        input.push(terminal);

        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, input);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let before = crate::command::command_ownership_counters();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let delivered = crate::test_harness::expect_expanded_command(&mut processor);
        let after = crate::command::command_ownership_counters();
        assert_eq!(delivered.spelling().semantic_token(), terminal);
        assert_eq!(
            after.rich_materializations - before.rich_materializations,
            1
        );
        assert_eq!(after.slot_initializations - before.slot_initializations, 0);
        assert_eq!(
            after.resolved_writes - before.resolved_writes,
            1,
            "ordinary macros never construct even the hot command record"
        );
        assert_eq!(
            after.delivery_stamp_writes - before.delivery_stamp_writes,
            1
        );
    });
}

#[test]
fn synchronous_primitive_chain_stays_in_the_occupied_hot_owner() {
    crate::test_harness::with_universe(|universe| {
        let top_mark = install_static(
            universe,
            "topmark",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::TopMark),
        );
        let first_mark = install_static(
            universe,
            "firstmark",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FirstMark),
        );
        let bot_mark = install_static(
            universe,
            "botmark",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::BotMark),
        );
        let terminal = Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [top_mark, first_mark, bot_mark, terminal]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let ownership_before = crate::command::command_ownership_counters();
        #[cfg(feature = "profiling")]
        let allocation_before = tex_state::measurement::hot_core_thread_allocation_measurement(
            tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
        );
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
        );
        let mut destination = None;
        assert_eq!(
            processor
                .expanded_next_hot(&mut destination, None)
                .expect("hot primitive chain"),
            crate::DeliveryStatus::PendingExpanded
        );
        let delivered = destination.expect("hot terminal command");
        assert_eq!(delivered.spelling().semantic_token(), terminal);
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after.rich_materializations - ownership_before.rich_materializations,
            0
        );
        assert_eq!(
            ownership_after.hot_reconstructions - ownership_before.hot_reconstructions,
            0
        );
        #[cfg(feature = "profiling")]
        {
            let allocation_after = tex_state::measurement::hot_core_thread_allocation_measurement(
                tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
            );
            assert_eq!(allocation_after.calls - allocation_before.calls, 0);
            assert_eq!(
                allocation_after.requested_bytes - allocation_before.requested_bytes,
                0
            );
        }
    });
}

#[test]
fn primitive_scanner_start_uses_compact_opener_state() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                number,
                Token::Char {
                    ch: '4',
                    cat: Catcode::Other,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let ownership_before = crate::command::command_ownership_counters();
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
                .expanded_next_hot(&mut destination, None)
                .expect("compact number scanner"),
            crate::DeliveryStatus::PendingExpanded
        );
        assert_eq!(
            destination
                .expect("number result")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: '4',
                cat: Catcode::Other,
            }
        );
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after.rich_materializations - ownership_before.rich_materializations,
            0
        );
        assert_eq!(
            ownership_after.hot_reconstructions - ownership_before.hot_reconstructions,
            0
        );
    });
}

#[test]
fn expandafter_replays_first_token_when_undefined_second_is_compact() {
    crate::test_harness::with_universe(|universe| {
        let expandafter = install_static(
            universe,
            "expandafter",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let missing = Token::Cs(universe.intern("missing").expect("undefined name").symbol());
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                expandafter,
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                missing,
                Token::Char {
                    ch: 'Z',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "AZ");
    });
}
