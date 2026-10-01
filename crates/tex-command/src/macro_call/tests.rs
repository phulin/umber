use tex_state::env::AssignmentScope;
use tex_state::env::banks::IntParam;
use tex_state::meaning::{Meaning, MeaningFlags, MeaningWord, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token, TokenWord};

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
    let (arguments, _) = processor
        .input
        .levels
        .innermost_macro_activation()
        .expect("macro activation");
    arguments[0]
        .iter()
        .map(|word| word.semantic_token())
        .collect()
}

#[test]
fn unobserved_parameterless_macro_activates_directly_and_elides_empty_body_row() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_replacement_macro(universe, "directempty", &[]);
        let terminal = letter('z');
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [macro_token, terminal]);
        let before = super::macro_activation_counters();
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

        let delivered = crate::test_harness::expect_expanded_command(&mut processor);
        let after = super::macro_activation_counters();
        assert_eq!(delivered.spelling().semantic_token(), terminal);
        assert_eq!(after.simple - before.simple, 1);
        assert_eq!(after.empty_rows_elided - before.empty_rows_elided, 1);
        assert_eq!(after.matching - before.matching, 0);
        assert_eq!(after.exceptional - before.exceptional, 0);
        assert_eq!(processor.command.stack_usage().input_stack, 1);
        assert!(
            !processor
                .command
                .input
                .levels
                .iter()
                .any(|level| level.macro_body().is_some()),
            "an empty simple replacement has no resident input row"
        );
    });
}

#[test]
fn outer_macro_matching_returns_to_ordinary_delivery() {
    crate::test_harness::with_universe(|universe| {
        let outer = install_macro_with_flags(
            universe,
            "outer_sequence",
            &[Token::Param(1)],
            MeaningFlags::OUTER,
        );
        let begin = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let end = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let argument = letter('x');
        let following = letter('z');
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [outer, begin, argument, end, following]);
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

        assert_eq!(
            crate::test_harness::expect_expanded_command(&mut processor)
                .spelling()
                .semantic_token(),
            argument
        );
        assert_eq!(
            crate::test_harness::expect_expanded_command(&mut processor)
                .spelling()
                .semantic_token(),
            following
        );
    });
}

#[test]
fn observed_parameterless_macro_keeps_exceptional_activation_semantics() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_replacement_macro(universe, "observedempty", &[]);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [macro_token]);
        let before = super::macro_activation_counters();
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
        let mut call = crate::test_harness::expect_raw_command(&mut processor);

        assert_eq!(processor.macro_call(&mut call), Ok(true));
        let after = super::macro_activation_counters();
        assert_eq!(after.exceptional - before.exceptional, 1);
        assert_eq!(after.simple - before.simple, 0);
        assert_eq!(after.empty_rows_elided - before.empty_rows_elided, 0);
        assert!(
            processor
                .command
                .input
                .levels
                .innermost_macro_activation()
                .is_some_and(|(_, replacement)| replacement.is_empty())
        );
    });
}

#[test]
fn empty_final_macro_keeps_replay_completion_on_an_exceptional_descendant() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_replacement_macro(universe, "completionempty", &[]);
        let replay = universe
            .allocate_token_list(&[TokenWord::pack(macro_token)])
            .expect("stored replay");
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [letter('z')]);
        let episode = {
            let context = universe.command_context().expect("command context");
            command.push_discretionary_episode(&context, replay)
        };
        let before = super::macro_activation_counters();
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
                .get_x_token_with_replay_completion_into(&mut destination)
                .expect("replay completion"),
            DeliveryStatus::ReplayCompleted(episode)
        );
        assert!(destination.is_none());
        let after = super::macro_activation_counters();
        assert_eq!(after.exceptional - before.exceptional, 1);
        assert_eq!(after.empty_rows_elided - before.empty_rows_elided, 0);
        assert_eq!(
            processor
                .get_x_token_with_replay_completion_into(&mut destination)
                .expect("enclosing command"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("enclosing command value")
                .spelling()
                .semantic_token(),
            letter('z')
        );
    });
}

#[test]
fn empty_delimited_argument_reuses_its_direct_destination_for_the_next_argument() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_macro(
            universe,
            "emptydelimited",
            &[Token::Param(1), other(','), Token::Param(2)],
        );
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
            [macro_token, other(','), begin, letter('x'), end],
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
        let mut call = crate::test_harness::expect_raw_command(&mut processor);

        assert_eq!(processor.macro_call(&mut call), Ok(true));
        let (arguments, _) = processor
            .command
            .input
            .levels
            .innermost_macro_activation()
            .expect("macro activation");
        assert_eq!(arguments.len(), 2);
        assert!(arguments[0].is_empty());
        assert_eq!(
            arguments[1],
            [tex_state::token::TracedTokenWord::pack(
                letter('x'),
                tex_state::token::OriginId::UNKNOWN,
            )]
        );
    });
}

/// TeX82 §394 classifies argument braces and leading spaces from `cur_tok`,
/// not from the resolved `cur_cmd`. A `\let`-style brace alias is therefore
/// one undelimited control-sequence argument; this is the form used by
/// LaTeX's `\@ifnextchar\bgroup\@iinput\@@input` call.
#[test]
fn undelimited_argument_keeps_brace_alias_as_one_control_sequence_token() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_macro(universe, "aliasedbraces", &[Token::Param(1)]);
        let opening = universe.intern("openingalias").expect("opening alias");
        universe
            .assign_meaning(
                opening,
                MeaningWord::from_static(Meaning::CharToken {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                }),
                AssignmentScope::Global,
            )
            .expect("brace alias meaning");
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [macro_token, Token::Cs(opening.symbol()), letter('x')],
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
        let mut call = crate::test_harness::expect_raw_command(&mut processor);

        assert_eq!(processor.macro_call(&mut call), Ok(true));
        assert_eq!(
            active_argument_tokens(processor.command),
            [Token::Cs(opening.symbol())]
        );
        assert_eq!(
            processor.command.profile_token_collector_path_counters().1,
            1,
            "each raw argument command is classified once"
        );
        assert_eq!(
            crate::test_harness::expect_expanded_command(&mut processor)
                .spelling()
                .semantic_token(),
            Token::Cs(opening.symbol())
        );
        assert_eq!(
            crate::test_harness::expect_raw_command(&mut processor)
                .spelling()
                .semantic_token(),
            letter('x'),
            "the alias must not absorb following source input as a brace group"
        );
    });
}

#[test]
fn successful_macro_calls_admit_one_nonowning_replacement_row() {
    crate::test_harness::with_universe(|universe| {
        let parameterless =
            install_replacement_macro(universe, "parameterlessowner", &[letter('p')]);
        let parameterized = install_macro(universe, "parameterizedowner", &[Token::Param(1)]);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [parameterless, parameterized, letter('a')]);
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

        for (expected_replacement_len, has_arguments) in [(1, false), (1, true)] {
            assert_eq!(
                processor
                    .get_next_into(&mut destination)
                    .expect("macro delivery"),
                DeliveryStatus::Command
            );
            let mut call = destination.take().expect("macro command");
            let owners_before = match call.meaning_ref() {
                tex_state::meaning::ResolvedMeaning::Macro { definition, .. } => {
                    definition.semantic_owner_count()
                }
                _ => panic!("macro meaning"),
            };
            let retains_before = tex_state::definition_retain_count();
            assert_eq!(processor.macro_call(&mut call), Ok(true));
            assert_eq!(
                tex_state::definition_retain_count(),
                retains_before,
                "successful matching and activation borrow then move the resident owner"
            );
            let (arguments, replacement) = processor
                .command
                .input
                .levels
                .innermost_macro_activation()
                .expect("live macro body");
            assert_eq!(!arguments.is_empty(), has_arguments);
            assert_eq!(replacement.len(), expected_replacement_len);
            assert_eq!(
                processor.command.input.levels.parameters.len(),
                arguments.len()
            );
            assert!(processor.command.scratch.is_quiescent());
            let tex_state::meaning::ResolvedMeaning::Macro { definition, .. } = call.meaning_ref()
            else {
                panic!("macro meaning");
            };
            assert_eq!(definition.semantic_owner_count(), owners_before);
            assert!(matches!(
                call.meaning_ref(),
                tex_state::meaning::ResolvedMeaning::Macro { .. }
            ));

            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("replacement delivery"),
                DeliveryStatus::Command
            );
            let _ = destination.take();
        }
    });
}

#[test]
fn literal_prefix_only_macros_use_no_argument_scratch() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(
                &[TokenWord::pack(other('!'))],
                &[TokenWord::pack(letter('p'))],
            )
            .expect("literal-prefix definition");
        let symbol = universe.intern("literalprefix").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let macro_token = Token::Cs(symbol.symbol());

        for (actual, expected) in [(other('!'), true), (other('?'), false)] {
            let mut command = CommandState::default();
            crate::test_harness::push(&mut command, [macro_token, actual]);
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
            let mut call = crate::test_harness::expect_raw_command(&mut processor);

            let activates = expected;
            assert_eq!(processor.macro_call(&mut call), Ok(expected));
            assert!(processor.command.scratch.is_quiescent());
            assert!(processor.command.input.levels.parameters.is_empty());
            if activates {
                let (arguments, _) = processor
                    .command
                    .input
                    .levels
                    .innermost_macro_activation()
                    .expect("literal-prefix macro body");
                assert!(arguments.is_empty());
            }
        }
    });
}

#[test]
fn failed_macro_call_keeps_the_resident_definition_owner() {
    crate::test_harness::with_universe(|universe| {
        let macro_token = install_macro(universe, "prefixowner", &[other('!'), Token::Param(1)]);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [macro_token, other('?')]);
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
        let mut call = crate::test_harness::expect_raw_command(&mut processor);
        let owners_before = match call.meaning_ref() {
            tex_state::meaning::ResolvedMeaning::Macro { definition, .. } => {
                definition.semantic_owner_count()
            }
            _ => panic!("macro meaning"),
        };

        assert_eq!(processor.macro_call(&mut call), Ok(false));
        match call.meaning_ref() {
            tex_state::meaning::ResolvedMeaning::Macro { definition, .. } => {
                assert_eq!(definition.semantic_owner_count(), owners_before);
            }
            _ => panic!("failed call retains macro meaning"),
        }
        assert!(
            !processor
                .command
                .input
                .levels
                .iter()
                .any(|level| level.macro_body().is_some())
        );
    });
}

#[test]
fn nested_and_tail_macro_calls_keep_only_live_stable_slots() {
    crate::test_harness::with_universe(|universe| {
        let inner = install_replacement_macro(universe, "inner", &[letter('i')]);
        let nested = install_replacement_macro(universe, "nested", &[inner, letter('t')]);
        let tail = install_replacement_macro(universe, "tail", &[inner]);
        let mut command = CommandState::default();
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
        assert_eq!(processor.command.scratch.argument_word_len(), 0);
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
        assert_eq!(processor.command.scratch.argument_word_len(), 0);
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
        assert_eq!(processor.command.scratch.argument_word_len(), 0);

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
        assert_eq!(processor.command.scratch.argument_word_len(), 0);
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
fn nested_macro_argument_consumes_plain_body_words_as_one_span() {
    crate::test_harness::with_universe(|universe| {
        let inner = install_macro(universe, "inner_span", &[Token::Param(1)]);
        let begin = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let end = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let outer = install_replacement_macro(
            universe,
            "outer_span",
            &[inner, begin, letter('a'), letter('b'), letter('c'), end],
        );
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [outer]);
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

        for expected in ['a', 'b', 'c'] {
            let command = crate::test_harness::expect_expanded_command(&mut processor);
            assert_eq!(command.spelling().semantic_token(), letter(expected));
        }
    });
}

#[test]
fn nested_macro_argument_reader_counts_loaded_words_not_exhaustion() {
    crate::test_harness::with_universe(|universe| {
        let inner = install_macro(universe, "inner_argument_span", &[Token::Param(1)]);
        let definition = universe
            .allocate_definition(
                &[TokenWord::pack(Token::Param(1))],
                &[TokenWord::pack(inner), TokenWord::pack(Token::Param(1))],
            )
            .expect("outer definition");
        let symbol = universe.intern("outer_argument_span").expect("outer name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("outer meaning");
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
                Token::Cs(symbol.symbol()),
                begin,
                letter('a'),
                letter('b'),
                letter('c'),
                end,
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

        for expected in ['a', 'b', 'c'] {
            let command = crate::test_harness::expect_expanded_command(&mut processor);
            assert_eq!(command.spelling().semantic_token(), letter(expected));
        }
    });
}

#[test]
fn replay_completion_follows_nested_final_token_macro_descendants_once() {
    crate::test_harness::with_universe(|universe| {
        let inner = install_replacement_macro(universe, "replayinner", &[letter('i')]);
        let outer = install_replacement_macro(universe, "replayouter", &[inner]);
        let replay = universe
            .allocate_token_list(&[TokenWord::pack(outer)])
            .expect("stored replay");
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [letter('z')]);
        let episode = {
            let context = universe.command_context().expect("command context");
            command.push_discretionary_episode(&context, replay)
        };
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
                .get_x_token_with_replay_completion_into(&mut destination)
                .expect("nested replay result"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("nested expansion command")
                .spelling()
                .semantic_token(),
            letter('i')
        );
        assert_eq!(
            processor
                .get_x_token_with_replay_completion_into(&mut destination)
                .expect("nested replay completion"),
            DeliveryStatus::ReplayCompleted(episode)
        );
        assert!(destination.is_none());
        assert_eq!(
            processor
                .get_x_token_with_replay_completion_into(&mut destination)
                .expect("enclosing command"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("enclosing command value")
                .spelling()
                .semantic_token(),
            letter('z')
        );
        assert!(processor.command.replay_completions.is_empty());
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
            let mut call = destination.take().expect("macro command");
            assert_eq!(processor.macro_call(&mut call), Ok(true));
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
        let mut call = destination.take().expect("macro command");

        assert!(processor.macro_call(&mut call).expect("macro call"));
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
fn parameter_escape_distinguishes_substitution_from_a_literal_hash() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(
                &[TokenWord::pack(Token::Param(1))],
                &[
                    TokenWord::pack(Token::Param(1)),
                    TokenWord::pack(Token::Char {
                        ch: '#',
                        cat: Catcode::Parameter,
                    }),
                ],
            )
            .expect("parameter escape definition");
        let symbol = universe.intern("parameterescape").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let argument = letter('x');
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [Token::Cs(symbol.symbol()), argument]);
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

        for expected in [
            argument,
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
        ] {
            assert_eq!(
                crate::test_harness::expect_expanded_command(&mut processor)
                    .spelling()
                    .semantic_token(),
                expected
            );
        }
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
        let mut call = destination.take().expect("macro command");

        assert!(processor.macro_call(&mut call).expect("macro call"));
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
        let mut call = destination.take().expect("macro command");

        assert!(processor.macro_call(&mut call).expect("macro call"));
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
        let mut long_call = destination.take().expect("long macro command");
        assert_eq!(processor.macro_call(&mut long_call), Ok(true));
        assert_eq!(active_argument_tokens(processor.command), [paragraph]);

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
        let mut short_call = destination.take().expect("short macro command");
        assert_eq!(
            processor.macro_call(&mut short_call),
            Err(crate::CommandError::ParagraphInMacroArgument)
        );
        assert_eq!(processor.command.scratch.argument_word_len(), 0);
        assert_eq!(
            crate::test_harness::expect_raw_command(&mut processor)
                .spelling()
                .semantic_token(),
            paragraph
        );
    });
}

#[test]
fn extra_right_brace_recovery_keeps_inserted_paragraph_ahead_of_backed_closer() {
    crate::test_harness::with_universe(|universe| {
        let paragraph = install_par(universe);
        let macro_token = install_macro(universe, "rightbracerecovery", &[Token::Param(1)]);
        let closer = Token::Char {
            ch: ']',
            cat: Catcode::EndGroup,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [macro_token, closer]);
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
        let mut call = crate::test_harness::expect_raw_command(&mut processor);

        assert_eq!(
            processor.macro_call(&mut call),
            Err(crate::CommandError::ParagraphInMacroArgument)
        );
        for expected in [paragraph, closer] {
            assert_eq!(
                crate::test_harness::expect_raw_command(&mut processor)
                    .spelling()
                    .semantic_token(),
                expected
            );
        }
        assert!(processor.command.scratch.is_quiescent());
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
        let mut call = destination.take().expect("delimited macro command");

        assert_eq!(processor.macro_call(&mut call), Ok(true));
        assert_eq!(
            active_argument_tokens(processor.command),
            [paragraph, letter('x')]
        );
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
        let mut alias_call = destination.take().expect("alias argument macro command");
        assert_eq!(processor.macro_call(&mut alias_call), Ok(true));
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
        let mut paragraph_call = destination
            .take()
            .expect("paragraph argument macro command");
        assert_eq!(
            processor.macro_call(&mut paragraph_call),
            Err(crate::CommandError::ParagraphInMacroArgument)
        );
    });
}

#[test]
fn direct_macro_prefix_diagnostics_match_delivered_command_diagnostics() {
    let run = |observed, source| {
        crate::test_harness::with_universe(|universe| {
            let call = install_macro(universe, "prefixowner", &[other('!'), Token::Param(1)]);
            let mut command = CommandState::default();
            if source {
                let source = command
                    .register_source(crate::SourceRegistration::new(
                        crate::RegisteredSourceKind::Generated,
                        &b"\\prefixowner?z%"[..],
                    ))
                    .expect("source");
                command
                    .open_registered_source(source)
                    .expect("source opening");
            } else {
                crate::test_harness::push(&mut command, [call, other('?'), letter('z')]);
            }
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
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
            let before = crate::command::command_ownership_counters();
            let delivered = crate::test_harness::expect_expanded_command(&mut processor);
            assert_eq!(delivered.spelling().semantic_token(), letter('z'));
            let writes = crate::command::command_ownership_counters().resolved_writes
                - before.resolved_writes;
            if !observed {
                assert_eq!(
                    writes, 2,
                    "only the mismatching argument and terminal need commands"
                );
            }
            assert_eq!(processor.fuel.burned(), 3);
            assert_eq!(processor.command.semantic_diagnostics.len(), 1);
            processor.take_semantic_diagnostics()
        })
    };
    for source in [false, true] {
        assert_eq!(run(false, source), run(true, source));
    }
}

#[test]
fn overlapping_delimiter_preserves_source_origins_through_forwarded_arguments() {
    // TeX82 §394's failed prefix becomes literal argument material. Both its
    // source locations and the ordinary words must survive nested forwarding.
    crate::test_harness::with_universe(|universe| {
        let inner = install_macro(
            universe,
            "inner",
            &[
                Token::param(1),
                letter('a'),
                letter('b'),
                letter('a'),
                letter('b'),
            ],
        );
        let outer_definition = universe
            .allocate_definition(
                &[
                    TokenWord::pack(Token::param(1)),
                    TokenWord::pack(other('!')),
                ],
                &[TokenWord::pack(inner), TokenWord::pack(Token::param(1))],
            )
            .expect("forwarding definition");
        let outer = universe.intern("outer").expect("macro name");
        universe
            .assign_meaning(
                outer,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, outer_definition),
                AssignmentScope::Global,
            )
            .expect("forwarding meaning");
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"\\outer xabxabab!z"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let mut call = crate::test_harness::expect_raw_command(&mut processor);
        assert!(processor.macro_call(&mut call).expect("outer match"));
        let origins: Vec<_> = processor
            .command
            .input
            .levels
            .innermost_macro_activation()
            .expect("outer activation")
            .0[0]
            .iter()
            .map(|word| word.origin())
            .collect();
        let mut forwarded = Vec::new();
        for (index, expected) in "xabx".chars().enumerate() {
            let result = crate::test_harness::expect_expanded_command(&mut processor);
            assert_eq!(result.spelling().semantic_token(), letter(expected));
            assert_eq!(result.origin(), origins[index]);
            let provenance = processor
                .source_provenance(&result)
                .expect("forwarded source location");
            assert_eq!(provenance.range().start(), 7 + index as u64);
            forwarded.push(result);
        }
        let after = crate::test_harness::expect_expanded_command(&mut processor);
        assert_eq!(after.spelling().semantic_token(), letter('z'));
        // Origin resolution still works after both nested macro rows retire.
        assert!(
            processor
                .command
                .input
                .levels
                .innermost_macro_activation()
                .is_none()
        );
        for (index, result) in forwarded.iter().enumerate() {
            let provenance = processor
                .source_provenance(result)
                .expect("retired macro source location");
            assert_eq!(provenance.range().start(), 7 + index as u64);
        }
    });
}
