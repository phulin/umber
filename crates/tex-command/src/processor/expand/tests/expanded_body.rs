//! Balanced expanded-body collection, nesting, and recovery.

use super::*;

#[test]
fn the_scans_its_target_from_the_same_expanded_delivery_loop() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let count = install_static(
            universe,
            "count",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                the,
                count,
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        let output = collect_expanded_characters(universe, &mut command);
        assert_eq!(output, "0X");
    });
}

#[test]
fn expanded_collects_a_balanced_body_in_the_shared_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let expanded = install_static(
            universe,
            "expanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                expanded,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'B',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "ABX");
    });
}

#[test]
fn expanded_body_uses_the_shared_lane_for_nested_the() {
    crate::test_harness::with_universe(|universe| {
        let expanded = install_static(
            universe,
            "expanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
        );
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let count = install_static(
            universe,
            "count",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                expanded,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                the,
                count,
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "0X");
    });
}

#[test]
fn nested_expanded_bodies_return_through_the_same_driver() {
    crate::test_harness::with_universe(|universe| {
        let expanded = install_static(
            universe,
            "expanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
        );
        let definition = universe
            .allocate_definition(
                &[],
                &[TokenWord::pack(Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                })],
            )
            .expect("nested definition");
        let symbol = universe.intern("nested").expect("nested name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("nested meaning");
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                expanded,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                expanded,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(symbol.symbol()),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "AX");
    });
}

#[test]
fn expanded_body_splices_unexpanded_children_without_reentering_delivery() {
    crate::test_harness::with_universe(|universe| {
        let expanded = install_static(
            universe,
            "expanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
        );
        let unexpanded = install_static(
            universe,
            "unexpanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded),
        );
        let definition = universe
            .allocate_definition(
                &[],
                &[TokenWord::pack(Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                })],
            )
            .expect("nested definition");
        let symbol = universe.intern("nested").expect("nested name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("nested meaning");
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                expanded,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                unexpanded,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(symbol.symbol()),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "AX");
    });
}

#[test]
fn expanded_body_detokenizes_children_into_the_parent_buffer() {
    crate::test_harness::with_universe(|universe| {
        let expanded = install_static(
            universe,
            "expanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
        );
        let detokenize = install_static(
            universe,
            "detokenize",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                expanded,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                detokenize,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                },
                Token::Char {
                    ch: 'B',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "A BX");
    });
}

#[test]
fn expanded_missing_opening_brace_uses_balanced_recovery() {
    crate::test_harness::with_universe(|universe| {
        let expanded = install_static(
            universe,
            "expanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                expanded,
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "AX");
    });
}

#[test]
fn expanded_unterminated_body_recovers_at_end_and_exposes_collected_text() {
    crate::test_harness::with_universe(|universe| {
        let expanded = install_static(
            universe,
            "expanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
        );
        let mut command = CommandState::default();
        let operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                expanded,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
            ],
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
        let recovered = crate::test_harness::expect_expanded_command(&mut processor);
        assert_eq!(
            recovered.meaning(),
            Meaning::CharToken {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        {
            let mut destination = None;
            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("terminal delivery after recovered body"),
                crate::DeliveryStatus::End
            );
            assert!(destination.is_none());
        };
        drop(processor);
        let diagnostics = command.take_semantic_diagnostics();
        assert!(matches!(
            diagnostics.as_slice(),
            [crate::CommandSemanticDiagnostic::Recoverable {
                message,
                runaway: Some(crate::state::RunawayPrelude { heading, partial }),
                ..
            }] if message.starts_with("File ended while scanning text")
                && *heading == "Runaway text?"
                && partial == "A"
        ));
        command
            .rollback_attempt_operation(operation)
            .expect("rollback after unterminated expanded body");
        assert!(command.scratch.is_quiescent());
        assert!(command.attempt.is_empty());
    });
}
