//! Numeric operands, registers, and conditional nesting.

use super::*;

#[test]
fn nested_number_conversions_return_through_the_shared_delivery_loop() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        let mut input = vec![number, number, number];
        input.extend([
            Token::Char {
                ch: '4',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            },
        ]);
        crate::test_harness::push(&mut command, input);
        assert_eq!(collect_expanded_characters(universe, &mut command), "4X");
    });
}

#[test]
fn number_accepts_repeated_signs_spaces_and_macro_boundaries() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let minusone_definition = universe
            .allocate_definition(
                &[],
                &[TokenWord::pack(other('-')), TokenWord::pack(other('1'))],
            )
            .expect("minus-one definition");
        let minusone = universe.intern("minusone").expect("minus-one macro");
        universe
            .assign_meaning(
                minusone,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, minusone_definition),
                AssignmentScope::Global,
            )
            .expect("minus-one meaning");

        let cases = [
            (
                vec![number, other('-'), other('-'), other('1'), letter('X')],
                "1X",
            ),
            (
                vec![
                    number,
                    other('-'),
                    other('-'),
                    other('-'),
                    other('1'),
                    letter('X'),
                ],
                "-1X",
            ),
            (
                vec![number, Token::Cs(minusone.symbol()), letter('X')],
                "-1X",
            ),
            (
                vec![
                    number,
                    Token::Char {
                        ch: ' ',
                        cat: Catcode::Space,
                    },
                    other('+'),
                    Token::Char {
                        ch: ' ',
                        cat: Catcode::Space,
                    },
                    Token::Cs(minusone.symbol()),
                    letter('X'),
                ],
                "-1X",
            ),
        ];
        for (input, expected) in cases {
            let mut command = CommandState::default();
            let _operation = command.begin_attempt_operation();
            crate::test_harness::push(&mut command, input);
            assert_eq!(
                collect_expanded_characters(universe, &mut command),
                expected
            );
            assert!(command.take_semantic_diagnostics().is_empty());
        }
    });
}

#[test]
fn number_reports_a_genuinely_missing_operand_and_preserves_the_token() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [number, letter('X')]);
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
        let mut output = String::new();
        loop {
            let mut destination = None;
            match processor
                .get_x_token_into(&mut destination)
                .expect("missing-number recovery")
            {
                crate::DeliveryStatus::Command => {}
                crate::DeliveryStatus::End => {
                    assert!(destination.is_none());
                    break;
                }
                status => panic!("unexpected expanded delivery status: {status:?}"),
            }
            let command = destination.expect("expanded delivery filled caller slot");
            match command.meaning() {
                tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken { ch, .. }) => {
                    output.push(ch);
                }
                other => panic!("expected expanded character, found {other:?}"),
            }
        }
        assert_eq!(output, "0X");
        drop(processor);
        assert!(
            diagnostic_effects.has_first_recoverable(),
            "a genuinely missing operand must report through the cold diagnostic effect"
        );
    });
}

#[test]
fn number_admits_direct_internal_values_and_composes_polarity() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let time = install_static(universe, "time", Meaning::IntParam(IntParam::TIME.raw()));
        let count = install_static(universe, "countalias", Meaning::CountRegister(0));
        let dimension = install_static(universe, "dimenalias", Meaning::DimenRegister(0));
        universe
            .assign_int_param(IntParam::TIME, 23, AssignmentScope::Global)
            .expect("time parameter assignment");
        universe
            .assign_count(0, 17, AssignmentScope::Global)
            .expect("count register assignment");
        universe
            .assign_dimension(0, Scaled::from_raw(123), AssignmentScope::Global)
            .expect("dimension register assignment");

        let cases = [
            (vec![number, time, letter('X')], "23X"),
            (vec![number, count, letter('X')], "17X"),
            (
                vec![number, other('-'), other('-'), count, letter('X')],
                "17X",
            ),
            (vec![number, other('-'), dimension, letter('X')], "-123X"),
        ];
        for (input, expected) in cases {
            let mut command = CommandState::default();
            let _operation = command.begin_attempt_operation();
            crate::test_harness::push(&mut command, input);
            assert_eq!(
                collect_expanded_characters(universe, &mut command),
                expected
            );
            assert!(command.take_semantic_diagnostics().is_empty());
        }
    });
}

#[test]
fn number_register_operands_use_the_shared_index_lane() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
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
                number,
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
        assert_eq!(collect_expanded_characters(universe, &mut command), "0X");
    });
}

#[test]
fn ifnum_register_operands_use_the_shared_index_lane() {
    crate::test_harness::with_universe(|universe| {
        let ifnum = install_static(
            universe,
            "ifnum",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfNum),
        );
        let count = install_static(
            universe,
            "count",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
        );
        let fi = install_static(
            universe,
            "fi",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifnum,
                count,
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '=',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                fi,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        // TeX82 §§379, 443–444 leave the recovery relax in the true branch.
        assert_eq!(
            collect_expanded_meanings(universe, &mut command),
            [
                Meaning::Relax,
                Meaning::CharToken {
                    ch: 'X',
                    cat: Catcode::Letter
                }
            ]
        );
    });
}

#[test]
fn ifdim_register_operands_use_the_shared_index_lane() {
    crate::test_harness::with_universe(|universe| {
        let ifdim = install_static(
            universe,
            "ifdim",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfDim),
        );
        let dimen = install_static(
            universe,
            "dimen",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Dimen),
        );
        let fi = install_static(
            universe,
            "fi",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifdim,
                dimen,
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '=',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'p',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 't',
                    cat: Catcode::Other,
                },
                fi,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        // TeX82 §§379, 443–444 leave the recovery relax in the true branch.
        assert_eq!(
            collect_expanded_meanings(universe, &mut command),
            [
                Meaning::Relax,
                Meaning::CharToken {
                    ch: 'X',
                    cat: Catcode::Letter
                }
            ]
        );
    });
}

#[test]
fn ifodd_register_operands_use_the_shared_index_lane() {
    crate::test_harness::with_universe(|universe| {
        let ifodd = install_static(
            universe,
            "ifodd",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfOdd),
        );
        let count = install_static(
            universe,
            "count",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
        );
        let fi = install_static(
            universe,
            "fi",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifodd,
                count,
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                fi,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "X");
    });
}

#[test]
fn ifodd_resumes_its_exact_parent_after_nested_expandafter() {
    crate::test_harness::with_universe(|universe| {
        let ifodd = install_static(
            universe,
            "ifodd",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfOdd),
        );
        let expandafter = install_static(
            universe,
            "expandafter",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let fi = install_static(
            universe,
            "fi",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifodd,
                expandafter,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                fi,
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
fn ifodd_exact_parent_matrix_covers_nested_scalar_children() {
    crate::test_harness::with_universe(|universe| {
        let ifodd = install_static(
            universe,
            "ifodd-matrix",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfOdd),
        );
        let the = install_static(
            universe,
            "the-matrix",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let count = install_static(
            universe,
            "count-matrix",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
        );
        let number = install_static(
            universe,
            "number-matrix",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let fi = install_static(
            universe,
            "fi-matrix",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        let x = Meaning::CharToken {
            ch: 'X',
            cat: Catcode::Letter,
        };
        // TeX82 §§379 and 444 preserve the inserted relax in the true
        // number-conversion case. False branches skip it, including §446's
        // outer missing-number recovery in the nested conditional case.
        let cases = [
            (
                vec![ifodd, the, count, other('0'), relax, fi, letter('X')],
                vec![x],
            ),
            (
                vec![ifodd, number, other('1'), fi, letter('X')],
                vec![Meaning::Relax, x],
            ),
            (
                vec![ifodd, ifodd, other('1'), fi, other('1'), fi, letter('X')],
                vec![x],
            ),
        ];
        for (input, expected) in cases {
            let mut command = CommandState::default();
            let _operation = command.begin_attempt_operation();
            crate::test_harness::push(&mut command, input);
            assert_eq!(collect_expanded_meanings(universe, &mut command), expected);
            assert!(command.scratch.is_quiescent());
        }
    });
}

#[test]
fn low_fuel_nested_if_controls_balance_their_inline_frames() {
    crate::test_harness::with_universe(|universe| {
        let ifodd = install_static(
            universe,
            "ifodd-low-fuel",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfOdd),
        );
        let ifnum = install_static(
            universe,
            "ifnum-low-fuel",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfNum),
        );
        let expandafter = install_static(
            universe,
            "expandafter-low-fuel",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let fi = install_static(
            universe,
            "fi-low-fuel",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifodd,
                expandafter,
                other('1'),
                letter('A'),
                fi,
                ifnum,
                other('1'),
                other('='),
                other('1'),
                fi,
                letter('X'),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(128).expect("small fuel ledger");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut output = Vec::new();
        loop {
            let mut destination = None;
            match processor
                .get_x_token_into(&mut destination)
                .expect("low-fuel delivery")
            {
                crate::DeliveryStatus::Command => {}
                crate::DeliveryStatus::End => {
                    assert!(destination.is_none());
                    break;
                }
                status => panic!("unexpected expanded delivery status: {status:?}"),
            }
            let command = destination.expect("expanded delivery filled caller slot");
            match command.meaning() {
                tex_state::meaning::ResolvedMeaning::Static(meaning) => {
                    output.push(meaning);
                }
                other => panic!("expected expanded character, found {other:?}"),
            }
        }
        assert_eq!(
            output,
            [
                Meaning::CharToken {
                    ch: 'A',
                    cat: Catcode::Letter
                },
                Meaning::Relax,
                Meaning::CharToken {
                    ch: 'X',
                    cat: Catcode::Letter
                },
            ]
        );
        assert!(processor.fuel.burned() <= 128);
        assert!(processor.command.scratch.is_quiescent());
    });
}
