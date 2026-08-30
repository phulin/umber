use tex_state::env::AssignmentScope;
use tex_state::env::banks::IntParam;
use tex_state::meaning::{Meaning, MeaningFlags, MeaningWord, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token, TokenWord};

use super::{MacroCallOutcome, MacroParameterEscape};
use crate::{
    CommandHostCapabilities, CommandObservation, CommandObserver, CommandState, DeliveryStatus,
    ObservedToken,
};

#[derive(Default)]
struct RecordingObserver(Vec<CommandObservation>);

impl CommandObserver for RecordingObserver {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}
#[test]
fn parameter_escape_distinguishes_substitution_from_a_literal_hash() {
    assert_eq!(
        MacroParameterEscape::classify(Token::Param(4)),
        Some(MacroParameterEscape::OutParameter(4))
    );
    assert_eq!(
        MacroParameterEscape::classify(Token::Char {
            ch: '#',
            cat: Catcode::Parameter,
        }),
        Some(MacroParameterEscape::EscapedParameter)
    );
}

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn letter(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Letter,
    }
}

fn install_macro<G>(
    universe: &mut tex_state::Universe<G>,
    name: &str,
    parameters: &[Token],
) -> Token {
    install_macro_with_flags(universe, name, parameters, MeaningFlags::EMPTY)
}

fn install_macro_with_flags<G>(
    universe: &mut tex_state::Universe<G>,
    name: &str,
    parameters: &[Token],
    flags: MeaningFlags,
) -> Token {
    let definition = universe
        .allocate_definition(
            &parameters
                .iter()
                .copied()
                .map(TokenWord::pack)
                .collect::<Vec<_>>(),
            &[TokenWord::pack(Token::Param(1))],
        )
        .expect("macro definition");
    let symbol = universe.intern(name).expect("macro name");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::macro_definition(flags, definition),
            AssignmentScope::Global,
        )
        .expect("macro meaning");
    Token::Cs(symbol.symbol())
}

fn install_par<G>(universe: &mut tex_state::Universe<G>) -> Token {
    let symbol = universe.intern("par").expect("paragraph name");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::from_static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)),
            AssignmentScope::Global,
        )
        .expect("paragraph meaning");
    Token::Cs(symbol.symbol())
}

fn install_replacement_macro<G>(
    universe: &mut tex_state::Universe<G>,
    name: &str,
    replacement: &[Token],
) -> Token {
    let definition = universe
        .allocate_definition(
            &[],
            &replacement
                .iter()
                .copied()
                .map(TokenWord::pack)
                .collect::<Vec<_>>(),
        )
        .expect("macro definition");
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

fn active_argument_tokens<G>(processor: &CommandState<G>) -> Vec<Token> {
    let arguments = processor
        .parameters
        .activations
        .last()
        .expect("macro activation")
        .arguments;
    let range = processor
        .scratch
        .argument_range(arguments.frame(), 1)
        .expect("live frame")
        .expect("first argument");
    (0..processor.scratch.argument_len(range).expect("live range"))
        .map(|index| {
            processor
                .scratch
                .argument_word(range, index)
                .expect("argument word")
                .semantic_token()
        })
        .collect()
}

#[test]
fn nested_and_tail_macro_calls_keep_only_live_stable_slots() {
    crate::test_harness::with_universe(|universe| {
        let inner = install_replacement_macro(universe, "inner", &[letter('i')]);
        let nested = install_replacement_macro(universe, "nested", &[inner, letter('t')]);
        let tail = install_replacement_macro(universe, "tail", &[inner]);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [nested, letter('n'), tail, letter('z')]);
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
                .get_x_token_into(&mut destination)
                .expect("nested expansion"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("inner result")
                .spelling()
                .semantic_token(),
            letter('i')
        );
        assert_eq!(processor.command.scratch.frame_len(), 2);
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("outer tail"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("tail token")
                .spelling()
                .semantic_token(),
            letter('t')
        );
        assert_eq!(processor.command.scratch.frame_len(), 1);
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("source separator"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("separator token")
                .spelling()
                .semantic_token(),
            letter('n')
        );
        assert_eq!(processor.command.scratch.frame_len(), 0);

        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("tail expansion"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("tail result")
                .spelling()
                .semantic_token(),
            letter('i')
        );
        assert_eq!(processor.command.scratch.frame_len(), 1);
        assert_eq!(processor.command.scratch.retained_slot_len(), 2);
        assert_eq!(processor.command.scratch.physical_macro_word_copies(), 0);
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("post-tail source"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("source token")
                .spelling()
                .semantic_token(),
            letter('z')
        );
        assert!(processor.command.scratch.is_quiescent());
    });
}

#[test]
fn repeated_out_parameter_replay_restarts_its_private_chunk_cursor() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(
                &[TokenWord::pack(Token::Param(1))],
                &[
                    TokenWord::pack(Token::Param(1)),
                    TokenWord::pack(Token::Param(1)),
                ],
            )
            .expect("repeated-parameter definition");
        let symbol = universe.intern("repeatarg").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let expected = (0..70)
            .map(|index| letter(char::from(b'a' + (index % 26) as u8)))
            .collect::<Vec<_>>();
        let mut input = Vec::with_capacity(expected.len() + 4);
        input.push(Token::Cs(symbol.symbol()));
        input.push(Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        });
        input.extend(expected.iter().copied());
        input.push(Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        });
        input.push(letter('z'));

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
        processor.command.profile_reset_raw_delivery_path_counters();
        let mut actual = Vec::new();
        let mut destination = None;
        for _ in 0..expected.len() * 2 {
            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("parameter replay"),
                DeliveryStatus::Command
            );
            actual.push(
                destination
                    .take()
                    .expect("argument token")
                    .spelling()
                    .semantic_token(),
            );
        }
        assert_eq!(actual[..expected.len()], expected);
        assert_eq!(actual[expected.len()..], expected);
        let (_, _, literal_arguments, out_parameters) =
            processor.command.profile_raw_delivery_path_counters();
        assert_eq!(literal_arguments, (expected.len() * 2) as u64);
        assert_eq!(out_parameters, 2);
        assert_eq!(
            processor
                .retire_exhausted_token_levels_for_named_boundary()
                .expect("named-boundary retirement"),
            2,
        );
        assert!(processor.command.scratch.is_quiescent());
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("following source"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("following token")
                .spelling()
                .semantic_token(),
            letter('z')
        );
    });
}

#[test]
fn outer_group_trim_bounds_macro_trace_and_observation_at_both_ends() {
    crate::test_harness::with_universe(|universe| {
        universe
            .assign_int_param(IntParam::TRACING_MACROS, 1, AssignmentScope::Global)
            .expect("enable macro tracing");
        let macro_token = install_macro(universe, "trimmed", &[Token::Param(1)]);
        let begin = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let end = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                macro_token,
                begin,
                begin,
                letter('x'),
                end,
                end,
                letter('z'),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            )
            .with_observer(&mut observer);
            let mut destination = None;
            assert_eq!(
                processor
                    .get_next_into(&mut destination)
                    .expect("macro delivery"),
                DeliveryStatus::Command
            );
            let call = destination.take().expect("macro command");
            assert_eq!(processor.macro_call(&call), Ok(MacroCallOutcome::Activated));
        }
        universe
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);

        let observed = observer
            .0
            .iter()
            .find_map(|observation| match observation {
                CommandObservation::Macro(crate::observation::MacroRecord::Argument {
                    tokens,
                    ..
                }) => Some(tokens.clone()),
                _ => None,
            })
            .expect("argument observation");
        assert_eq!(
            observed,
            vec![
                ObservedToken::Character {
                    character: '{',
                    catcode: Catcode::BeginGroup,
                },
                ObservedToken::Character {
                    character: 'x',
                    catcode: Catcode::Letter,
                },
                ObservedToken::Character {
                    character: '}',
                    catcode: Catcode::EndGroup,
                },
            ]
        );
        let trace: String = universe
            .world()
            .effect_records()
            .iter()
            .filter_map(|effect| match effect {
                tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(trace.contains("#1<-{x}"), "{trace}");
        assert!(!trace.contains("#1<-{x}}"), "{trace}");
    });
}

#[test]
fn delimited_argument_stops_at_its_literal_delimiter() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_macro(universe, "m", &[Token::Param(1), other(',')]);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [macro_token, letter('x'), other(','), letter('z')],
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
        let mut destination = None;
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("macro delivery"),
            DeliveryStatus::Command
        );
        let call = destination.take().expect("macro command");

        assert_eq!(
            processor.macro_call(&call).expect("macro call"),
            MacroCallOutcome::Activated
        );
        assert_eq!(active_argument_tokens(processor.command), [letter('x')]);
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("replacement"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("argument replay")
                .spelling()
                .semantic_token(),
            letter('x')
        );
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("following source"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("following token")
                .spelling()
                .semantic_token(),
            letter('z')
        );
    });
}

#[test]
fn delimited_argument_preserves_a_failed_overlapping_prefix() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_macro(
            universe,
            "overlap",
            &[Token::Param(1), other('a'), other('b'), other('a')],
        );
        let expected = [letter('x'), other('a'), other('b'), letter('x')];
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                macro_token,
                expected[0],
                expected[1],
                expected[2],
                expected[3],
                other('a'),
                other('b'),
                other('a'),
                letter('z'),
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
        let mut destination = None;
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("macro delivery"),
            DeliveryStatus::Command
        );
        let call = destination.take().expect("macro command");

        assert_eq!(
            processor.macro_call(&call).expect("macro call"),
            MacroCallOutcome::Activated
        );
        assert_eq!(active_argument_tokens(processor.command), expected);
        for expected_token in expected {
            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("argument replay"),
                DeliveryStatus::Command
            );
            assert_eq!(
                destination
                    .take()
                    .expect("argument token")
                    .spelling()
                    .semantic_token(),
                expected_token
            );
        }
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("following source"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("following token")
                .spelling()
                .semantic_token(),
            letter('z')
        );
    });
}

#[test]
fn delimited_argument_ignores_delimiters_inside_literal_braces() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_macro(universe, "m", &[Token::Param(1), other(',')]);
        let begin = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let end = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                macro_token,
                begin,
                letter('a'),
                other(','),
                letter('b'),
                end,
                letter('c'),
                other(','),
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
        let mut destination = None;
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("macro delivery"),
            DeliveryStatus::Command
        );
        let call = destination.take().expect("macro command");

        assert_eq!(
            processor.macro_call(&call).expect("macro call"),
            MacroCallOutcome::Activated
        );
        assert_eq!(
            active_argument_tokens(processor.command),
            [
                begin,
                letter('a'),
                other(','),
                letter('b'),
                end,
                letter('c')
            ]
        );
    });
}

#[test]
fn paragraph_fact_preserves_long_and_non_long_token_semantics() {
    crate::test_harness::with_universe(|universe| {
        let paragraph = install_par(universe);
        let begin = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let end = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let long = install_macro_with_flags(
            universe,
            "longmacro",
            &[Token::Param(1)],
            MeaningFlags::LONG,
        );
        let short = install_macro(universe, "shortmacro", &[Token::Param(1)]);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [long, begin, paragraph, end, short, begin, paragraph, end],
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

        let mut destination = None;
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("long macro delivery"),
            DeliveryStatus::Command
        );
        let long_call = destination.take().expect("long macro command");
        assert_eq!(
            processor.macro_call(&long_call),
            Ok(MacroCallOutcome::Activated)
        );
        let arguments = processor.command.parameters.activations[0].arguments;
        let range = processor
            .command
            .scratch
            .argument_range(arguments.frame(), 1)
            .expect("live long-macro frame")
            .expect("long macro first argument");
        let facts = processor
            .command
            .scratch
            .argument_facts(range)
            .expect("sealed long-macro facts");
        assert!(facts.rejects_non_long_paragraph());
        assert!(facts.removable_outer_group());
        assert_eq!(processor.command.scratch.match_word_reads(), 0);

        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("long argument replay"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("paragraph argument token")
                .spelling()
                .semantic_token(),
            paragraph
        );
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("short macro delivery"),
            DeliveryStatus::Command
        );
        let short_call = destination.take().expect("short macro command");
        assert_eq!(
            processor.macro_call(&short_call),
            Err(crate::CommandError::ParagraphInMacroArgument)
        );
        assert_eq!(processor.command.scratch.frame_len(), 0);
    });
}

#[test]
fn paragraph_delimiter_prefix_is_not_reclassified_after_commit() {
    crate::test_harness::with_universe(|universe| {
        let paragraph = install_par(universe);
        let macro_token = install_macro(
            universe,
            "paragraphdelimiter",
            &[Token::Param(1), paragraph, other(',')],
        );
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [macro_token, paragraph, letter('x'), paragraph, other(',')],
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
        let mut destination = None;
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("delimited macro delivery"),
            DeliveryStatus::Command
        );
        let call = destination.take().expect("delimited macro command");

        assert_eq!(processor.macro_call(&call), Ok(MacroCallOutcome::Activated));
        assert_eq!(
            active_argument_tokens(processor.command),
            [paragraph, letter('x')]
        );
        let arguments = processor.command.parameters.activations[0].arguments;
        let range = processor
            .command
            .scratch
            .argument_range(arguments.frame(), 1)
            .expect("live delimited-macro frame")
            .expect("delimited macro first argument");
        assert!(
            !processor
                .command
                .scratch
                .argument_facts(range)
                .expect("sealed delimited-macro facts")
                .rejects_non_long_paragraph()
        );
        assert_eq!(processor.command.scratch.match_word_reads(), 0);
    });
}

#[test]
fn paragraph_fact_uses_token_identity_not_current_meaning() {
    crate::test_harness::with_universe(|universe| {
        let paragraph = install_par(universe);
        let paragraph_id = universe.intern("par").expect("paragraph name");
        let alias = universe.intern("paragraphalias").expect("alias name");
        universe
            .assign_meaning(
                alias,
                MeaningWord::from_static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Par,
                )),
                AssignmentScope::Global,
            )
            .expect("paragraph alias meaning");
        universe
            .assign_meaning(
                paragraph_id,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("paragraph redefinition");
        let macro_token = install_macro(universe, "shortidentity", &[Token::Param(1)]);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                macro_token,
                Token::Cs(alias.symbol()),
                macro_token,
                paragraph,
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

        let mut destination = None;
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("alias argument macro delivery"),
            DeliveryStatus::Command
        );
        let alias_call = destination.take().expect("alias argument macro command");
        assert_eq!(
            processor.macro_call(&alias_call),
            Ok(MacroCallOutcome::Activated)
        );
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("alias argument replay"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("alias argument token")
                .spelling()
                .semantic_token(),
            Token::Cs(alias.symbol())
        );
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("paragraph argument macro delivery"),
            DeliveryStatus::Command
        );
        let paragraph_call = destination
            .take()
            .expect("paragraph argument macro command");
        assert_eq!(
            processor.macro_call(&paragraph_call),
            Err(crate::CommandError::ParagraphInMacroArgument)
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn mixed_one_and_4096_token_arguments_use_one_fused_settlement_without_copies() {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Evidence {
        fact_classifications: u64,
        token_settlements: u64,
        writer_admissions: u64,
        writer_finalizations: u64,
        allocation_calls: u64,
        requested_bytes: u64,
        whole_token_copies: u64,
        whole_command_copies: u64,
        whole_input_frame_copies: u64,
        aggregate_word_reads: u64,
    }

    fn run(token_count: usize) -> Evidence {
        crate::test_harness::with_universe(|universe| {
            let paragraph = install_par(universe);
            let definition = universe
                .allocate_definition(&[TokenWord::pack(Token::Param(1))], &[])
                .expect("empty replacement macro definition");
            let symbol = universe.intern("measured").expect("macro name");
            universe
                .assign_meaning(
                    symbol,
                    MeaningWord::macro_definition(MeaningFlags::LONG, definition),
                    AssignmentScope::Global,
                )
                .expect("macro meaning");
            let macro_token = Token::Cs(symbol.symbol());
            let argument = |count: usize| {
                if count == 1 {
                    return vec![letter('x')];
                }
                let begin = Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                };
                let end = Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                };
                let mut tokens = Vec::with_capacity(count);
                tokens.push(begin);
                for index in 1..count - 1 {
                    tokens.push(match index % 4 {
                        0 => paragraph,
                        1 => letter('a'),
                        2 => other('!'),
                        _ => Token::Char {
                            ch: ' ',
                            cat: Catcode::Space,
                        },
                    });
                }
                tokens.push(end);
                tokens
            };

            let mut command = CommandState::default();
            let _operation = command.begin_attempt_operation();
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

            // Warm every owner with the larger shape, then retire the empty
            // replacement so its macro frame and lane chunks become reusable.
            let mut warm = Vec::with_capacity(4_097);
            warm.push(macro_token);
            warm.extend(argument(4_096));
            crate::test_harness::push(&mut command, warm);
            {
                let mut context = universe.command_context().expect("warm command context");
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                let call = processor
                    .get_next()
                    .expect("warm macro delivery")
                    .expect("warm macro command");
                assert_eq!(processor.macro_call(&call), Ok(MacroCallOutcome::Activated));
                assert!(
                    processor
                        .get_x_token()
                        .expect("warm replacement retirement")
                        .is_none()
                );
            }
            assert_eq!(command.scratch.frame_len(), 0);

            let mut measured = Vec::with_capacity(token_count + 1);
            measured.push(macro_token);
            measured.extend(argument(token_count));
            crate::test_harness::push(&mut command, measured);
            let mut context = universe
                .command_context()
                .expect("measured command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let call = processor
                .get_next()
                .expect("measured macro delivery")
                .expect("measured macro command");
            processor.command.scratch.reset_match_settlement_counters();
            let admissions = processor.command.scratch.match_writer_admissions();
            let finalizations = processor.command.scratch.match_writer_finalizations();
            let token_copies = processor.command.scratch.physical_macro_word_copies();
            let aggregate_reads = processor.command.scratch.match_word_reads();
            let command_copies = crate::command::command_ownership_counters().clones;
            let timeline = processor.command.profile_timeline_counters();
            let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let allocations = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                assert_eq!(processor.macro_call(&call), Ok(MacroCallOutcome::Activated));
            }
            let after_allocations =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let after_commands = crate::command::command_ownership_counters();
            let after_timeline = processor.command.profile_timeline_counters();
            let (fact_classifications, token_settlements) =
                processor.command.scratch.match_settlement_counters();
            Evidence {
                fact_classifications,
                token_settlements,
                writer_admissions: processor.command.scratch.match_writer_admissions() - admissions,
                writer_finalizations: processor.command.scratch.match_writer_finalizations()
                    - finalizations,
                allocation_calls: after_allocations.calls - allocations.calls,
                requested_bytes: after_allocations.requested_bytes - allocations.requested_bytes,
                whole_token_copies: processor.command.scratch.physical_macro_word_copies()
                    - token_copies,
                whole_command_copies: after_commands.clones - command_copies,
                whole_input_frame_copies: after_timeline.full_frame_history_clones
                    - timeline.full_frame_history_clones,
                aggregate_word_reads: processor.command.scratch.match_word_reads()
                    - aggregate_reads,
            }
        })
    }

    let one = run(1);
    assert_eq!(
        one,
        Evidence {
            fact_classifications: 1,
            token_settlements: 1,
            writer_admissions: 1,
            writer_finalizations: 1,
            allocation_calls: 0,
            requested_bytes: 0,
            whole_token_copies: 0,
            whole_command_copies: 0,
            whole_input_frame_copies: 0,
            aggregate_word_reads: 0,
        }
    );
    let four_k = run(4_096);
    assert_eq!(four_k.fact_classifications, 4_096);
    assert_eq!(four_k.token_settlements, 4_096);
    assert_eq!(four_k.writer_admissions, 1);
    assert_eq!(four_k.writer_finalizations, 1);
    assert_eq!(four_k.allocation_calls, 0);
    assert_eq!(four_k.requested_bytes, 0);
    assert_eq!(four_k.whole_token_copies, 0);
    assert_eq!(four_k.whole_command_copies, 0);
    assert_eq!(four_k.whole_input_frame_copies, 0);
    assert_eq!(four_k.aggregate_word_reads, 0);
}
