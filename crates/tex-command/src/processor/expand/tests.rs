use tex_state::env::AssignmentScope;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::{
    CommandDeliveryBoundary, CommandHostCapabilities, CommandObservation, CommandObserver,
    CommandProfile, CommandState, InputTransition, RecoveryKind,
};

#[derive(Default)]
struct RecordingObserver(Vec<CommandObservation>);

impl CommandObserver for RecordingObserver {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn install_static<G>(universe: &mut tex_state::Universe<G>, name: &str, meaning: Meaning) -> Token {
    let symbol = universe.intern(name).expect("intern primitive");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::from_static(meaning),
            AssignmentScope::Global,
        )
        .expect("install primitive");
    Token::Cs(symbol.symbol())
}

fn collect_expanded_characters<G>(
    universe: &mut tex_state::Universe<G>,
    command: &mut CommandState<G>,
) -> String {
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::default();
    let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("command context");
    let mut processor = crate::test_harness::processor(
        command,
        &mut context,
        &mut capabilities,
        &mut fuel,
        &mut diagnostic_effects,
    );
    let mut output = String::new();
    while let Some(command) = processor.get_x_token().expect("expanded delivery") {
        match command.meaning() {
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken { ch, .. }) => {
                output.push(ch);
            }
            other => panic!("expected expanded character, found {other:?}"),
        }
    }
    output
}

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
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
    });
}

#[test]
fn deeply_nested_the_requests_use_the_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let toks = install_static(universe, "toks", Meaning::ToksRegister(0));
        let relax = install_static(universe, "relax", Meaning::Relax);
        for depth in [1_024, 10_240, 100_000] {
            let mut input = Vec::with_capacity(depth * 2 + 1);
            input.extend(std::iter::repeat_n(the, depth));
            input.extend(std::iter::repeat_n(toks, depth));
            input.push(relax);
            let mut command = CommandState::default();
            let _operation = command.begin_attempt_operation();
            crate::test_harness::push(&mut command, input);
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
            let settled = processor
                .get_x_token()
                .expect("nested the delivery")
                .expect("terminal command");
            assert_eq!(settled.meaning(), Meaning::Relax);
            drop(processor);
            assert_eq!(command.scratch.driver_continuation_depth(), 0);
            assert_eq!(
                command.scratch.recursive_delivery_entries_with_control(),
                0,
                "nested the must not re-enter expanded delivery while a control is live"
            );
        }
    });
}

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
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
        assert_eq!(
            command.scratch.recursive_delivery_entries_with_control(),
            0,
            "nested number must return through the compact control lane"
        );
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
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
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
        assert_eq!(collect_expanded_characters(universe, &mut command), "X");
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
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
        assert_eq!(collect_expanded_characters(universe, &mut command), "X");
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
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
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
    });
}

#[test]
fn nested_the_register_indices_do_not_reenter_the_delivery_stack() {
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
        for depth in [1_024, 10_240, 100_000] {
            let mut input = Vec::with_capacity(depth * 2 + 2);
            for _ in 0..depth {
                input.extend([the, count]);
            }
            input.extend([
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ]);
            let mut command = CommandState::default();
            let _operation = command.begin_attempt_operation();
            crate::test_harness::push(&mut command, input);
            let output = collect_expanded_characters(universe, &mut command);
            assert_eq!(output, "0X");
            assert_eq!(command.scratch.driver_continuation_depth(), 0);
            assert_eq!(command.scratch.recursive_delivery_entries_with_control(), 0);
        }
    });
}

#[test]
fn nested_the_integer_expressions_use_the_shared_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let numexpr = install_static(
            universe,
            "numexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::NumExpr),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        for depth in [1_024, 10_240, 100_000] {
            let mut input = Vec::with_capacity(depth * 3 + 2);
            for _ in 0..depth {
                input.extend([the, numexpr]);
            }
            input.push(Token::Char {
                ch: '0',
                cat: Catcode::Other,
            });
            input.extend(std::iter::repeat_n(relax, depth));
            input.push(Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            });
            let mut command = CommandState::default();
            let _operation = command.begin_attempt_operation();
            crate::test_harness::push(&mut command, input);
            let output = collect_expanded_characters(universe, &mut command);
            assert_eq!(output, "0X");
            assert_eq!(command.scratch.driver_continuation_depth(), 0);
            assert_eq!(command.scratch.recursive_delivery_entries_with_control(), 0);
        }
    });
}

#[test]
fn the_direct_internal_meanings_use_the_hot_value_projection() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let count_alias = install_static(universe, "countalias", Meaning::CountRegister(0));
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                the,
                count_alias,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "0X");
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
    });
}

#[test]
fn the_integer_expression_lane_preserves_operator_precedence() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let numexpr = install_static(
            universe,
            "numexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::NumExpr),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                the,
                numexpr,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '+',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '*',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '3',
                    cat: Catcode::Other,
                },
                relax,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "7X");
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
    });
}

#[test]
fn number_integer_expression_uses_the_shared_expression_lane() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let numexpr = install_static(
            universe,
            "numexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::NumExpr),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                number,
                numexpr,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '+',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '*',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '3',
                    cat: Catcode::Other,
                },
                relax,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "7X");
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
    });
}

#[test]
fn nested_fontname_operands_use_the_shared_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let fontname = install_static(
            universe,
            "fontname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName),
        );
        let nullfont = install_static(
            universe,
            "nullfont",
            Meaning::Font(tex_state::font::NULL_FONT),
        );
        // Only the innermost selector can be valid: each enclosing
        // `\fontname` sees the first rendered character of its child and
        // therefore exercises TeX's missing-identifier recovery. Keep the
        // chain below the engine's fatal-error threshold while still
        // traversing the compact control lane repeatedly.
        let depth = 16;
        let mut input = Vec::with_capacity(depth + 2);
        input.extend(std::iter::repeat_n(fontname, depth));
        input.extend([
            nullfont,
            Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            },
        ]);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, input);
        let output = collect_expanded_characters(universe, &mut command);
        assert!(output.starts_with("nullfont"));
        assert!(output.ends_with('X'));
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
        assert_eq!(
            command.scratch.recursive_delivery_entries_with_control(),
            0,
            "nested fontname operands must not re-enter delivery while a control is live"
        );
    });
}

#[test]
fn ifcsname_collects_in_the_shared_delivery_lane() {
    crate::test_harness::with_universe(|universe| {
        let ifcsname = install_static(
            universe,
            "ifcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCsName),
        );
        let endcsname = install_static(
            universe,
            "endcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
        );
        let _known = install_static(universe, "known", Meaning::Relax);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifcsname,
                Token::Char {
                    ch: 'k',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'n',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'o',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'w',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'n',
                    cat: Catcode::Letter,
                },
                endcsname,
                Token::Char {
                    ch: 'T',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'F',
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
        let result = processor
            .get_x_token()
            .expect("ifcsname delivery")
            .expect("selected branch");
        assert_eq!(
            result.spelling().semantic_token(),
            Token::Char {
                ch: 'T',
                cat: Catcode::Letter,
            }
        );
        drop(processor);
        assert_eq!(command.scratch.driver_continuation_depth(), 0);
    });
}

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

        let expanded = processor
            .get_x_token()
            .expect("macro expansion")
            .expect("replacement command");
        assert_eq!(expanded.spelling().semantic_token(), replacement);
        assert_eq!(
            expanded.meaning(),
            Meaning::CharToken {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        assert!(processor.get_x_token().expect("end").is_none());
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

        let source_command = processor
            .get_next()
            .expect("source delivery")
            .expect("source command");
        let origin = source_command.origin();
        assert_ne!(origin, OriginId::UNKNOWN);
        let fuel_before_treatment = processor.fuel.burned();
        processor
            .treat_as_active_character('~', origin)
            .expect("active-character treatment");
        assert_eq!(processor.fuel.burned(), fuel_before_treatment);

        let backed_up = processor
            .get_next()
            .expect("backed-up active character")
            .expect("active character command");
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
        let backed_up = processor
            .get_next()
            .expect("backed-up next command")
            .expect("settled command");
        assert_eq!(
            backed_up.spelling().semantic_token(),
            Token::Char {
                ch: 'B',
                cat: Catcode::Letter,
            }
        );
        assert!(processor.get_next().expect("end of input").is_none());
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

        let delivered = processor
            .get_x_token()
            .expect("expanded chain")
            .expect("terminal command");
        let after = crate::command::command_ownership_counters();
        assert_eq!(delivered.spelling().semantic_token(), terminal);
        assert_eq!(
            after.rich_materializations - before.rich_materializations,
            1
        );
        assert_eq!(after.slot_initializations - before.slot_initializations, 0);
    });
}

#[cfg(feature = "profiling")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OrdinaryDeliveryEvidence {
    slot_initializations: u64,
    rich_materializations: u64,
    resolved_writes: u64,
    expanded_classifications: u64,
    command_clones: u64,
    token_frame_steps: u64,
    meaning_lookups: u64,
    expanded_deliveries: u64,
    #[cfg(feature = "profiling")]
    allocations: u64,
    #[cfg(feature = "profiling")]
    allocated_bytes: u64,
}

#[cfg(feature = "profiling")]
fn empty_macro_delivery_evidence(expansions: usize) -> OrdinaryDeliveryEvidence {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty definition");
        let symbol = universe.intern("m").expect("macro name");
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
        let mut input = Vec::with_capacity((expansions + 1) * 2);
        for _ in 0..2 {
            input.resize(input.len() + expansions, Token::Cs(symbol.symbol()));
            input.push(terminal);
        }

        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, input);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut destination = None;

        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("warm ordinary expanded delivery"),
                crate::DeliveryStatus::Command
            );
        }
        assert_eq!(
            destination
                .take()
                .expect("warm terminal command")
                .spelling()
                .semantic_token(),
            terminal
        );
        let before_ownership = crate::command::command_ownership_counters();
        let classifications_before = super::expanded_classifications();
        let work_before = fuel.work();

        #[cfg(feature = "profiling")]
        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        #[cfg(feature = "profiling")]
        let before_allocations =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            #[cfg(feature = "profiling")]
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                processor
                    .preflight_command_into(&mut destination)
                    .expect("preflight expansion delivery"),
                crate::DeliveryStatus::Command
            );
        }
        #[cfg(feature = "profiling")]
        let after_allocations =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);

        let delivered = destination.expect("terminal command");
        assert_eq!(delivered.spelling().semantic_token(), terminal);
        assert_eq!(
            delivered.meaning(),
            Meaning::CharToken {
                ch: 'Z',
                cat: Catcode::Letter,
            }
        );
        let after_ownership = crate::command::command_ownership_counters();
        let work = fuel.work();
        OrdinaryDeliveryEvidence {
            slot_initializations: after_ownership.slot_initializations
                - before_ownership.slot_initializations,
            rich_materializations: after_ownership.rich_materializations
                - before_ownership.rich_materializations,
            resolved_writes: after_ownership.resolved_writes - before_ownership.resolved_writes,
            expanded_classifications: super::expanded_classifications() - classifications_before,
            command_clones: after_ownership.clones - before_ownership.clones,
            token_frame_steps: work.token_frame_steps - work_before.token_frame_steps,
            meaning_lookups: work.meaning_lookups - work_before.meaning_lookups,
            expanded_deliveries: work.expanded_deliveries - work_before.expanded_deliveries,
            #[cfg(feature = "profiling")]
            allocations: after_allocations.calls - before_allocations.calls,
            #[cfg(feature = "profiling")]
            allocated_bytes: after_allocations.requested_bytes - before_allocations.requested_bytes,
        }
    })
}

#[test]
#[cfg(feature = "profiling")]
fn one_and_4096_preflight_expansions_reuse_one_slot_with_exact_linear_work() {
    let one = empty_macro_delivery_evidence(1);
    let many = empty_macro_delivery_evidence(4_096);

    for (expansions, evidence) in [(1, one), (4_096, many)] {
        assert_eq!(evidence.slot_initializations, 0);
        assert_eq!(evidence.rich_materializations, 1);
        assert_eq!(evidence.resolved_writes, expansions + 1);
        assert_eq!(evidence.expanded_classifications, expansions + 1);
        assert_eq!(evidence.command_clones, 0);
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
fn destination_owned_expansion_result_excludes_suspension_payload() {
    struct FormerSuspendedExpansion<G> {
        _resume: crate::state::PendingExpansionResume,
        _child: Option<
            crate::execution_scratch::ChildContinuation<
                G,
                crate::state::PendingExpansionChildDestination,
            >,
        >,
    }
    struct FormerExpansionFailure<G> {
        _error: crate::CommandError,
        _suspended: Option<FormerSuspendedExpansion<G>>,
    }

    let current = core::mem::size_of::<Result<(), crate::CommandError>>();
    let former = core::mem::size_of::<FormerExpansionFailure<()>>();
    assert!(current < former, "current={current}, former={former}");
}

#[test]
fn expandable_preflight_delivery_uses_one_caller_owned_command_slot() {
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
                .preflight_command_into(&mut destination)
                .expect("preflight delivery"),
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
        assert_eq!(ownership_after.clones - ownership_before.clones, 0);
    });
}

#[test]
fn unexpandable_preflight_classifies_once_and_reuses_one_slot() {
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
        let ownership_before = crate::command::command_ownership_counters();
        let classifications_before = super::expanded_classifications();
        assert_eq!(
            processor
                .preflight_command_into(&mut destination)
                .expect("preflight delivery"),
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
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            super::expanded_classifications() - classifications_before,
            1
        );
        assert_eq!(
            ownership_after.slot_initializations - ownership_before.slot_initializations,
            0
        );
        assert_eq!(ownership_after.clones - ownership_before.clones, 0);
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

#[test]
fn main_loop_character_run_resolves_only_its_non_character_tail() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(3).expect("character-run fuel");
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
        let mut characters = String::new();
        assert_eq!(
            processor
                .main_loop_character_run_into(&mut destination, &mut |_, _, _, ch, _| {
                    characters.push(ch);
                    true
                },)
                .expect("borrowed character run"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert_eq!(characters, "Ab");
        assert!(matches!(
            destination.as_ref().expect("tail command").meaning(),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::Space,
                ..
            })
        ));
        drop(processor);
        assert_eq!(
            fuel.burned(),
            3,
            "two run characters and the consumed tail each cost one charge"
        );
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after.resolved_writes - ownership_before.resolved_writes,
            1,
            "the borrowed characters never become CurrentCommand values"
        );
    });
}

#[test]
fn main_loop_character_run_lexes_a_resident_source_prefix_once() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"xab c"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(4).expect("source character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let first = processor
            .get_next()
            .expect("source line acquisition")
            .expect("first source token");
        assert_eq!(
            first.spelling().semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }
        );
        processor
            .command
            .profile_reset_input_cursor_mutation_counters();
        processor
            .command
            .profile_reset_input_source_context_counters();
        let ownership_before = crate::command::command_ownership_counters();
        let mut destination = None;
        let mut characters = String::new();
        let mut origins = Vec::new();
        assert_eq!(
            processor
                .main_loop_character_run_into(&mut destination, &mut |_, _, _, ch, origin| {
                    characters.push(ch);
                    origins.push(origin);
                    true
                },)
                .expect("source character run"),
            crate::DeliveryStatus::CharacterRun
        );
        assert_eq!(characters, "ab");
        assert_eq!(origins.len(), 2);
        assert_ne!(origins[0], origins[1]);
        assert!(destination.is_none());
        assert_eq!(
            processor.command.profile_resident_input_branch_counters(),
            (1, 1, 0, 0)
        );
        assert_eq!(
            processor.command.profile_input_source_context_counters(),
            (0, 0, 0, 1)
        );
        assert_eq!(
            processor.fuel.burned(),
            3,
            "the deferred space tail remains uncharged until its owner fetches it"
        );
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after.resolved_writes - ownership_before.resolved_writes,
            0
        );

        assert_eq!(
            processor
                .main_loop_character_run_into(&mut destination, &mut |_, _, _, _, _| true)
                .expect("scalar source boundary"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert!(matches!(
            destination.as_ref().expect("space boundary").meaning(),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::Space,
                ..
            })
        ));
        drop(processor);
        assert_eq!(
            fuel.burned(),
            4,
            "the initial source character plus the run and tail each cost one charge"
        );
    });
}

#[test]
fn main_loop_character_run_charges_resident_macro_body_once_per_character() {
    crate::test_harness::with_universe(|universe| {
        let body_tokens = [
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'c',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
        ];
        let words: Vec<_> = body_tokens.iter().copied().map(TokenWord::pack).collect();
        let definition = universe
            .allocate_definition(&[], &words)
            .expect("macro body definition");
        let macro_name = universe.intern("runbody").expect("macro name").symbol();
        let body = universe
            .command_context()
            .expect("macro body context")
            .admit_macro_body(definition)
            .expect("resident macro body")
            .2;
        let mut command = CommandState::default();
        command.push_macro_activation(macro_name, body, None, OriginId::UNKNOWN);

        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(4).expect("macro body character-run fuel");
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
        let mut characters = String::new();
        assert_eq!(
            processor
                .main_loop_character_run_into(&mut destination, &mut |_, _, _, ch, _| {
                    characters.push(ch);
                    true
                })
                .expect("macro body character run"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert_eq!(characters, "Abc");
        assert!(matches!(
            destination.as_ref().expect("macro body tail").meaning(),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::Space,
                ..
            })
        ));
        drop(processor);
        assert_eq!(fuel.burned(), 4);
    });
}

#[test]
fn main_loop_character_run_charges_macro_argument_chars_once() {
    crate::test_harness::with_universe(|universe| {
        let argument_tokens = [
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'c',
                cat: Catcode::Letter,
            },
        ];
        let traced = argument_tokens.map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN));
        let mut command = CommandState::default();
        let matching = command.scratch.begin_macro_match().expect("macro match");
        let mut writer = command
            .scratch
            .begin_argument_writer(&matching)
            .expect("macro argument writer");
        for word in traced {
            command
                .scratch
                .append_argument_token(
                    &mut writer,
                    crate::token_collector::ClassifiedToken::from_word(word, None),
                    true,
                )
                .expect("macro argument word");
        }
        command
            .scratch
            .publish_argument(writer)
            .expect("macro argument range");
        let argument_set = command
            .scratch
            .commit_macro_match(matching)
            .expect("macro argument set");
        let macro_name = universe.intern("runargument").expect("macro name").symbol();
        let body_definition = universe
            .allocate_definition(
                &[TokenWord::pack(Token::Param(1))],
                &[TokenWord::pack(Token::Param(1))],
            )
            .expect("macro body definition");
        let body = universe
            .command_context()
            .expect("macro body context")
            .admit_macro_body(body_definition)
            .expect("resident macro body")
            .2;
        command.push_macro_activation(macro_name, body, Some(argument_set), OriginId::UNKNOWN);

        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(3).expect("macro argument fuel");
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
        let mut characters = String::new();
        assert_eq!(
            processor
                .main_loop_character_run_into(&mut destination, &mut |_, _, _, ch, _| {
                    characters.push(ch);
                    true
                })
                .expect("macro argument character run"),
            crate::DeliveryStatus::CharacterRun
        );
        assert_eq!(characters, "Abc");
        assert!(destination.is_none());
        drop(processor);
        assert_eq!(fuel.burned(), 3);
    });
}

#[test]
fn main_loop_character_run_rollback_keeps_fuel_monotonic() {
    crate::test_harness::with_universe(|universe| {
        let tokens = [
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            },
        ];
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, tokens);
        let snapshot = command.snapshot(universe).expect("character-run snapshot");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(4).expect("rollback character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

        for expected_burned in [2, 4] {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let mut destination = None;
            let mut characters = String::new();
            assert_eq!(
                processor
                    .main_loop_character_run_into(&mut destination, &mut |_, _, _, ch, _| {
                        characters.push(ch);
                        true
                    })
                    .expect("character run retry"),
                crate::DeliveryStatus::CharacterRun
            );
            assert_eq!(characters, "Ab");
            drop(processor);
            assert_eq!(fuel.burned(), expected_burned);

            if expected_burned == 2 {
                drop(context);
                command
                    .rollback(&snapshot, universe)
                    .expect("rollback restores the character row");
            }
        }
    });
}

#[test]
fn noexpand_suppresses_exactly_one_expandable_delivery() {
    crate::test_harness::with_universe(|universe| {
        let noexpand = install_static(
            universe,
            "noexpand",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
        );
        let replacement = Token::Char {
            ch: 'B',
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
        let macro_token = Token::Cs(symbol.symbol());
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [noexpand, macro_token, macro_token]);
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

        let suppressed = processor
            .get_x_token()
            .expect("suppressed delivery")
            .expect("suppressed command");
        assert_eq!(suppressed.spelling().semantic_token(), macro_token);
        assert_eq!(suppressed.meaning(), Meaning::Relax);
        assert_eq!(
            processor
                .get_x_token()
                .expect("second delivery")
                .expect("replacement")
                .spelling()
                .semantic_token(),
            replacement
        );
    });
}

#[test]
fn input_suspension_retains_delivery_expansion_and_rollback_replays_the_same_prefix() {
    crate::test_harness::with_universe(|universe| {
        let ownership_before = crate::command::command_ownership_counters();
        let input = install_static(
            universe,
            "input",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
        );
        let filename = "child".chars().map(|ch| Token::Char {
            ch,
            cat: Catcode::Letter,
        });
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            std::iter::once(input)
                .chain(filename)
                .chain(std::iter::once(Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                })),
        );
        let snapshot = command.snapshot(universe).expect("input prefix snapshots");
        #[cfg(feature = "profiling")]
        {
            command.profile_reset_stored_token_advance_counters();
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

        let (resume, delivery_cursor) = {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let error = match processor.next_alignment_lookahead() {
                Err(error) => error,
                Ok(_) => panic!("unresolved input must suspend"),
            };
            assert!(matches!(
                error,
                crate::CommandError::MissingInput { ref name, .. } if name == "child.tex"
            ));
            let delivery_cursor = processor.delivery_cursor();
            let resume = processor
                .take_pending_expansion_work()
                .expect("typed parked expansion suspension");
            assert!(processor.scanner_resume.is_none());
            (resume, delivery_cursor)
        };
        assert_eq!(
            fuel.burned(),
            8,
            "the first suspended prefix charges each newly fetched token once"
        );
        let ownership_after_first = crate::command::command_ownership_counters();
        assert_eq!(ownership_after_first.clones - ownership_before.clones, 0);
        assert_eq!(
            ownership_after_first.expansion_moves_in - ownership_before.expansion_moves_in,
            1
        );
        assert_eq!(
            ownership_after_first.expansion_moves_out - ownership_before.expansion_moves_out,
            0
        );

        let resume = {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            processor.resume_delivery_cursor(delivery_cursor);
            processor.install_expansion_resume(resume);
            let error = match processor.next_alignment_lookahead() {
                Err(error) => error,
                Ok(_) => panic!("unfulfilled retry must resuspend"),
            };
            assert!(matches!(
                error,
                crate::CommandError::MissingInput { ref name, .. } if name == "child.tex"
            ));
            processor
                .take_pending_expansion_work()
                .expect("second suspension parks the same sole owner")
        };
        assert_eq!(
            fuel.burned(),
            8,
            "retrying the owned suspended prefix does not charge it again"
        );
        let ownership_after_second = crate::command::command_ownership_counters();
        assert_eq!(ownership_after_second.clones - ownership_before.clones, 0);
        assert_eq!(
            ownership_after_second.expansion_moves_in - ownership_before.expansion_moves_in,
            2
        );
        assert_eq!(
            ownership_after_second.expansion_moves_out - ownership_before.expansion_moves_out,
            1
        );
        capabilities.register_input(
            "child.tex",
            crate::SourceRegistration::new(crate::RegisteredSourceKind::Generated, &b"Q"[..])
                .with_name("child.tex"),
        );
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            processor.resume_delivery_cursor(delivery_cursor);
            processor.install_expansion_resume(resume);
            let lookahead = processor
                .next_alignment_lookahead()
                .expect("resource-backed resume")
                .expect("resumed source command");
            assert!(matches!(
                lookahead,
                crate::AlignmentLookahead::PendingExpanded(_)
            ));
            let delivered = processor.commit_alignment_lookahead_delivery(lookahead);
            assert_eq!(
                delivered.spelling().semantic_token(),
                Token::Char {
                    ch: 'Q',
                    cat: Catcode::Letter,
                }
            );
            assert!(processor.scanner_resume.is_none());
        }
        assert_eq!(
            fuel.burned(),
            9,
            "resource-backed delivery adds only the resumed semantic token"
        );
        let ownership_after_resume = crate::command::command_ownership_counters();
        assert_eq!(ownership_after_resume.clones - ownership_before.clones, 0);
        assert_eq!(
            ownership_after_resume.expansion_moves_in - ownership_before.expansion_moves_in,
            2
        );
        assert_eq!(
            ownership_after_resume.expansion_moves_out - ownership_before.expansion_moves_out,
            2
        );

        command
            .rollback(&snapshot, universe)
            .expect("resumed input prefix rolls back");
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert_eq!(
            processor
                .get_x_token()
                .expect("restored prefix expands")
                .expect("restored child source command")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'Q',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            processor.fuel.burned(),
            18,
            "rollback re-delivers the prefix and leaves prior fuel consumed"
        );
        #[cfg(feature = "profiling")]
        {
            let (
                _selections,
                loads,
                advances,
                writes,
                lookups,
                _parameters,
                relays,
                _segment_inspections,
                _run_transitions,
            ) = processor.command.profile_stored_token_advance_counters();
            assert!(loads > 0, "suspension fixture must traverse stored input");
            assert_eq!(advances, loads);
            assert_eq!(writes, loads);
            assert!(
                lookups > 0,
                "the restored input primitive must resolve once"
            );
            assert_eq!(relays, 0);
        }
    });
}

#[test]
fn nested_expandafter_suspension_parks_each_command_once() {
    crate::test_harness::with_universe(|universe| {
        let expandafter = install_static(
            universe,
            "expandafter",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let input = install_static(
            universe,
            "input",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
        );
        let first = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [expandafter, first, input]
                .into_iter()
                .chain("child".chars().map(|ch| Token::Char {
                    ch,
                    cat: Catcode::Letter,
                }))
                .chain([Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                }]),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let ownership_before = crate::command::command_ownership_counters();

        let (resume, delivery_cursor) = {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let mut destination = None;
            let error = processor
                .get_x_token_into(&mut destination)
                .expect_err("nested input expansion suspends");
            assert!(matches!(
                error,
                crate::CommandError::MissingInput { ref name, .. } if name == "child.tex"
            ));
            assert!(destination.is_none());
            (
                processor
                    .take_pending_expansion_work()
                    .expect("outer expansion owns the parked command chain"),
                processor.delivery_cursor(),
            )
        };
        let ownership_after_suspend = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after_suspend.clones - ownership_before.clones,
            0,
            "nested callers must retain only child edges, not cloned commands"
        );
        assert_eq!(
            ownership_after_suspend.expansion_moves_in - ownership_before.expansion_moves_in,
            2,
            "the input command and its expandafter parent each park once"
        );

        capabilities.register_input(
            "child.tex",
            crate::SourceRegistration::new(crate::RegisteredSourceKind::Generated, &b"Q"[..])
                .with_name("child.tex"),
        );
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            processor.resume_delivery_cursor(delivery_cursor);
            processor.install_expansion_resume(resume);
            let mut destination = None;
            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("nested expansion resumes"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(
                destination
                    .expect("expandafter replays its first token")
                    .spelling()
                    .semantic_token(),
                first
            );
        }
        let ownership_after_resume = crate::command::command_ownership_counters();
        assert_eq!(ownership_after_resume.clones - ownership_before.clones, 0);
        assert_eq!(
            ownership_after_resume.expansion_moves_out - ownership_before.expansion_moves_out,
            2
        );
    });
}

#[test]
fn protected_replay_delivery_writes_the_terminal_macro_into_its_caller_slot() {
    crate::test_harness::with_universe(|universe| {
        let replacement = Token::Char {
            ch: 'P',
            cat: Catcode::Letter,
        };
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("definition");
        let symbol = universe.intern("protected").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::PROTECTED, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let macro_token = Token::Cs(symbol.symbol());
        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(&mut command, [macro_token]);
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

        let mut destination = None;
        assert_eq!(
            processor
                .get_x_or_protected_with_replay_completion_into(&mut destination)
                .expect("protected delivery"),
            super::DeliveryStatus::Command
        );
        let delivered = destination.expect("caller destination");
        assert_eq!(delivered.spelling().semantic_token(), macro_token);
        assert!(matches!(
            delivered.meaning(),
            tex_state::meaning::ResolvedMeaning::Macro { flags, .. }
                if flags.contains(MeaningFlags::PROTECTED)
        ));
    });
}

#[test]
fn csname_relaxes_an_already_interned_undefined_name() {
    crate::test_harness::with_universe(|universe| {
        let csname = install_static(
            universe,
            "csname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName),
        );
        let endcsname = install_static(
            universe,
            "endcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
        );
        let latent = universe.intern("latent").expect("pre-intern name");
        let mut input = vec![csname];
        input.extend("latent".chars().map(|ch| Token::Char {
            ch,
            cat: Catcode::Letter,
        }));
        input.push(endcsname);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, input);
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

        let expanded = processor
            .get_x_token()
            .expect("csname expansion")
            .expect("named control sequence");
        assert_eq!(
            expanded.spelling().semantic_token(),
            Token::Cs(latent.symbol())
        );
        assert_eq!(expanded.meaning(), Meaning::Relax);
        assert!(processor.get_x_token().expect("end").is_none());
    });
}

#[test]
fn pdf_insert_height_queries_live_state_and_distinguishes_missing_from_zero() {
    crate::test_harness::with_universe(|universe| {
        let pdf_insert_height = install_static(
            universe,
            "pdfinsertht",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfInsertHeight),
        );
        let class = [Token::Char {
            ch: '7',
            cat: Catcode::Other,
        }];

        let mut missing = CommandState::new(CommandProfile::PDFTEX14029);
        crate::test_harness::push(
            &mut missing,
            std::iter::once(pdf_insert_height).chain(class),
        );
        assert_eq!(collect_expanded_characters(universe, &mut missing), "0pt");

        universe
            .command_context()
            .expect("command context")
            .upsert_page_insertion(tex_state::page::PageInsertion::new(
                7,
                tex_state::scaled::Scaled::from_raw(0),
            ));
        let mut present = CommandState::new(CommandProfile::PDFTEX14029);
        crate::test_harness::push(
            &mut present,
            std::iter::once(pdf_insert_height).chain(class),
        );
        assert_eq!(collect_expanded_characters(universe, &mut present), "0.0pt");
    });
}
