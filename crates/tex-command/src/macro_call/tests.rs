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
    let arguments = processor
        .input
        .levels
        .iter()
        .rev()
        .find_map(|level| level.macro_body().and_then(|body| body.arguments))
        .expect("macro argument set");
    let range = processor
        .scratch
        .argument_range(arguments, 1)
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

        let delivered = processor
            .get_x_token()
            .expect("expanded delivery")
            .expect("terminal command");
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
        let mut call = processor
            .get_next()
            .expect("macro delivery")
            .expect("macro command");

        assert_eq!(processor.macro_call(&mut call), Ok(true));
        let after = super::macro_activation_counters();
        assert_eq!(after.exceptional - before.exceptional, 1);
        assert_eq!(after.simple - before.simple, 0);
        assert_eq!(after.empty_rows_elided - before.empty_rows_elided, 0);
        assert!(
            processor.command.input.levels.last().is_some_and(|level| {
                level.macro_body().is_some_and(|body| body.body.is_empty())
            })
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
        let mut call = processor
            .get_next()
            .expect("macro delivery")
            .expect("macro command");

        assert_eq!(processor.macro_call(&mut call), Ok(true));
        assert_eq!(
            processor.command.scratch.parked_scanner_storage_counts(),
            (0, 0, 0),
            "ordinary macro matching never enters a continuation lane"
        );
        let arguments = processor
            .command
            .input
            .levels
            .iter()
            .rev()
            .find_map(|level| level.macro_body().and_then(|body| body.arguments))
            .expect("macro argument set");
        let first = processor
            .command
            .scratch
            .argument_range(arguments, 1)
            .expect("live frame")
            .expect("first argument");
        let second = processor
            .command
            .scratch
            .argument_range(arguments, 2)
            .expect("live frame")
            .expect("second argument");
        assert_eq!(processor.command.scratch.argument_len(first), Ok(0));
        assert_eq!(processor.command.scratch.argument_len(second), Ok(1));
        assert_eq!(
            processor.command.scratch.argument_word(second, 0),
            Ok(tex_state::token::TracedTokenWord::pack(
                letter('x'),
                tex_state::token::OriginId::UNKNOWN,
            ))
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
        let mut call = processor
            .get_next()
            .expect("macro delivery")
            .expect("macro command");

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
            processor
                .get_x_token()
                .expect("argument replay delivery")
                .expect("argument replay command")
                .spelling()
                .semantic_token(),
            Token::Cs(opening.symbol())
        );
        assert_eq!(
            processor
                .get_next()
                .expect("following source command delivery")
                .expect("following source command")
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
            let body = processor
                .command
                .input
                .levels
                .last()
                .and_then(crate::input::InputLevel::macro_body)
                .expect("live macro body");
            assert_eq!(body.arguments.is_some(), has_arguments);
            assert_eq!(
                processor.command.input.levels.active_macro_parameters(),
                usize::from(has_arguments)
            );
            assert_eq!(
                processor.command.scratch.frame_len(),
                usize::from(has_arguments)
            );
            assert_eq!(
                body.body.definition_ref().semantic_owner_count(),
                owners_before
            );
            assert_eq!(
                processor
                    .state
                    .definition(body.body.definition_ref())
                    .replacement_text()
                    .len(),
                expected_replacement_len
            );
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
            let mut call = processor
                .get_next()
                .expect("macro delivery")
                .expect("macro command");

            let activates = expected;
            assert_eq!(processor.command.scratch.retained_slot_len(), 0);
            let writer_before = processor.command.scratch.match_writer_operations();
            assert_eq!(processor.macro_call(&mut call), Ok(expected));
            assert_eq!(
                processor.command.scratch.match_writer_operations(),
                writer_before
            );
            assert_eq!(processor.command.scratch.retained_slot_len(), 0);
            assert_eq!(processor.command.scratch.argument_word_len(), 0);
            assert_eq!(processor.command.scratch.frame_len(), 0);
            assert_eq!(processor.command.input.levels.active_macro_parameters(), 0);
            if activates {
                let body = processor
                    .command
                    .input
                    .levels
                    .last()
                    .and_then(crate::input::InputLevel::macro_body)
                    .expect("literal-prefix macro body");
                assert!(body.arguments.is_none());
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
        let mut call = processor
            .get_next()
            .expect("macro delivery")
            .expect("macro command");
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
        assert_eq!(processor.command.scratch.frame_len(), 0);
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
        assert_eq!(processor.command.scratch.frame_len(), 0);
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
        assert_eq!(processor.command.scratch.frame_len(), 0);
        assert_eq!(processor.command.scratch.retained_slot_len(), 0);
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
        command.profile_reset_macro_kernel_counters();
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
            let command = processor
                .get_x_token()
                .expect("nested expansion")
                .expect("argument token");
            assert_eq!(command.spelling().semantic_token(), letter(expected));
        }
        let (body_words, body_advances, _, body_writes, ..) =
            processor.command.profile_macro_kernel_counters();
        assert!(body_words >= 6);
        assert!(
            body_advances < body_words,
            "the ordinary argument interior advances as one admitted span"
        );
        assert!(
            body_writes < body_words,
            "span-consumed words never materialize CurrentCommand values"
        );
    });
}

#[test]
fn nested_macro_argument_consumes_replayed_argument_without_a_command_handoff() {
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
        processor.command.profile_reset_macro_kernel_counters();

        for expected in ['a', 'b', 'c'] {
            let command = processor
                .get_x_token()
                .expect("nested expansion")
                .expect("argument token");
            assert_eq!(command.spelling().semantic_token(), letter(expected));
        }
        let (_, _, _, _, argument_words, argument_advances, argument_writes) =
            processor.command.profile_macro_kernel_counters();
        assert!(argument_words >= 3);
        assert!(
            argument_writes < argument_words,
            "words={argument_words} advances={argument_advances} writes={argument_writes}"
        );
    });
}

#[test]
#[cfg(feature = "profiling")]
fn warmed_one_and_nine_argument_calls_replay_through_the_singular_kernel() {
    #[derive(Debug, Eq, PartialEq)]
    struct Evidence {
        macro_kernel: (u64, u64, u64, u64, u64, u64, u64),
        argument_writer: (u64, u64, u64, u64, u64),
        allocations: u64,
        allocated_bytes: u64,
        command_clones: u64,
    }

    fn run(argument_count: u8) -> Evidence {
        crate::test_harness::with_universe(|universe| {
            let parameters = (1..=argument_count)
                .map(Token::Param)
                .map(TokenWord::pack)
                .collect::<Vec<_>>();
            let definition = universe
                .allocate_definition(&parameters, &parameters)
                .expect("parameterized definition");
            let symbol = universe.intern("kernelargs").expect("macro name");
            universe
                .assign_meaning(
                    symbol,
                    MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                    AssignmentScope::Global,
                )
                .expect("macro meaning");
            let macro_token = Token::Cs(symbol.symbol());
            let arguments = (0..argument_count)
                .map(|offset| letter(char::from(b'a' + offset)))
                .collect::<Vec<_>>();
            let marker = other('!');
            let input = [macro_token]
                .into_iter()
                .chain(arguments.iter().copied())
                .chain([marker, macro_token])
                .chain(arguments.iter().copied())
                .chain([marker]);

            let mut command = CommandState::default();
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
                for expected in arguments.iter().copied().chain([marker]) {
                    assert_eq!(
                        processor
                            .get_x_token_into(&mut destination)
                            .expect("warm macro delivery"),
                        DeliveryStatus::Command
                    );
                    assert_eq!(
                        destination
                            .take()
                            .expect("warm command")
                            .spelling()
                            .semantic_token(),
                        expected
                    );
                }
            }

            command.profile_reset_macro_kernel_counters();
            let writer_before = command.scratch.match_writer_operations();
            let ownership_before = crate::command::command_ownership_counters();
            let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let allocations_before =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                for expected in arguments.iter().copied().chain([marker]) {
                    assert_eq!(
                        processor
                            .get_x_token_into(&mut destination)
                            .expect("measured macro delivery"),
                        DeliveryStatus::Command
                    );
                    assert_eq!(
                        destination
                            .take()
                            .expect("measured command")
                            .spelling()
                            .semantic_token(),
                        expected
                    );
                }
            }
            let allocations_after =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let ownership_after = crate::command::command_ownership_counters();
            let writer_after = command.scratch.match_writer_operations();
            Evidence {
                macro_kernel: command.profile_macro_kernel_counters(),
                argument_writer: (
                    writer_after.0 - writer_before.0,
                    writer_after.1 - writer_before.1,
                    writer_after.2 - writer_before.2,
                    writer_after.3 - writer_before.3,
                    writer_after.4 - writer_before.4,
                ),
                allocations: allocations_after.calls - allocations_before.calls,
                allocated_bytes: allocations_after.requested_bytes
                    - allocations_before.requested_bytes,
                command_clones: ownership_after.clones - ownership_before.clones,
            }
        })
    }

    for arguments in [1_u64, 9] {
        assert_eq!(
            run(arguments as u8),
            Evidence {
                macro_kernel: (
                    arguments, arguments, arguments, 0, arguments, arguments, arguments
                ),
                argument_writer: (arguments, arguments, arguments, arguments, arguments * 2),
                allocations: 0,
                allocated_bytes: 0,
                command_clones: 0,
            }
        );
    }
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
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [letter('z')]);
        let episode = {
            let context = universe.command_context().expect("command context");
            command.push_discretionary_episode(&context, replay)
        };
        command.profile_reset_raw_delivery_path_counters();
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
        assert_eq!(
            processor.command.profile_replay_completion_counters(),
            (3, 3, 2)
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
                processor
                    .get_x_token()
                    .expect("replacement delivery")
                    .expect("replacement command")
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
        let arguments = processor
            .command
            .input
            .levels
            .iter()
            .find_map(|level| level.macro_body().and_then(|body| body.arguments))
            .expect("long macro argument set");
        let range = processor
            .command
            .scratch
            .argument_range(arguments, 1)
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
        let mut short_call = destination.take().expect("short macro command");
        assert_eq!(
            processor.macro_call(&mut short_call),
            Err(crate::CommandError::ParagraphInMacroArgument)
        );
        assert_eq!(processor.command.scratch.frame_len(), 0);
        assert_eq!(
            processor
                .get_next()
                .expect("backed-up paragraph delivery")
                .expect("backed-up paragraph command")
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
        let mut call = processor
            .get_next()
            .expect("macro delivery")
            .expect("macro command");

        assert_eq!(
            processor.macro_call(&mut call),
            Err(crate::CommandError::ParagraphInMacroArgument)
        );
        for expected in [paragraph, closer] {
            assert_eq!(
                processor
                    .get_next()
                    .expect("recovery delivery")
                    .expect("recovery command")
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
        let arguments = processor
            .command
            .input
            .levels
            .iter()
            .find_map(|level| level.macro_body().and_then(|body| body.arguments))
            .expect("delimited macro argument set");
        let range = processor
            .command
            .scratch
            .argument_range(arguments, 1)
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

#[cfg(feature = "profiling")]
#[test]
fn mixed_one_64_and_4096_token_arguments_use_one_fused_settlement_without_copies() {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Evidence {
        fact_classifications: u64,
        token_settlements: u64,
        fact_updates: u64,
        writer_admissions: u64,
        writer_finalizations: u64,
        slot_validations: u64,
        allocation_calls: u64,
        requested_bytes: u64,
        whole_token_copies: u64,
        whole_command_copies: u64,
        definition_retains: u64,
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
                let mut call = processor
                    .get_next()
                    .expect("warm macro delivery")
                    .expect("warm macro command");
                assert_eq!(processor.macro_call(&mut call), Ok(true));
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
            let mut call = processor
                .get_next()
                .expect("measured macro delivery")
                .expect("measured macro command");
            processor
                .command
                .profile_reset_token_collector_path_counters();
            let writer_before = processor.command.scratch.match_writer_operations();
            let token_copies = processor.command.scratch.physical_macro_word_copies();
            let aggregate_reads = processor.command.scratch.match_word_reads();
            let command_copies = crate::command::command_ownership_counters().clones;
            let definition_retains = tex_state::definition_retain_count();
            let timeline = processor.command.profile_timeline_counters();
            let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let allocations = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                assert_eq!(processor.macro_call(&mut call), Ok(true));
            }
            let after_allocations =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let after_commands = crate::command::command_ownership_counters();
            let after_timeline = processor.command.profile_timeline_counters();
            let collector_counters = processor.command.profile_token_collector_path_counters();
            let writer_after = processor.command.scratch.match_writer_operations();
            Evidence {
                fact_classifications: collector_counters.1,
                token_settlements: writer_after.1 - writer_before.1,
                fact_updates: writer_after.2 - writer_before.2,
                writer_admissions: writer_after.0 - writer_before.0,
                writer_finalizations: writer_after.3 - writer_before.3,
                slot_validations: writer_after.4 - writer_before.4,
                allocation_calls: after_allocations.calls - allocations.calls,
                requested_bytes: after_allocations.requested_bytes - allocations.requested_bytes,
                whole_token_copies: processor.command.scratch.physical_macro_word_copies()
                    - token_copies,
                whole_command_copies: after_commands.clones - command_copies,
                definition_retains: tex_state::definition_retain_count() - definition_retains,
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
            fact_updates: 1,
            writer_admissions: 1,
            writer_finalizations: 1,
            slot_validations: 2,
            allocation_calls: 0,
            requested_bytes: 0,
            whole_token_copies: 0,
            whole_command_copies: 0,
            definition_retains: 0,
            whole_input_frame_copies: 0,
            aggregate_word_reads: 0,
        }
    );
    for token_count in [64, 4_096] {
        let measured = run(token_count);
        assert_eq!(measured.fact_classifications, token_count as u64);
        assert_eq!(measured.token_settlements, token_count as u64);
        assert_eq!(measured.fact_updates, token_count as u64);
        assert_eq!(measured.writer_admissions, 1);
        assert_eq!(measured.writer_finalizations, 1);
        assert_eq!(measured.slot_validations, 2);
        assert_eq!(measured.allocation_calls, 0);
        assert_eq!(measured.requested_bytes, 0);
        assert_eq!(measured.whole_token_copies, 0);
        assert_eq!(measured.whole_command_copies, 0);
        assert_eq!(measured.definition_retains, 0);
        assert_eq!(measured.whole_input_frame_copies, 0);
        assert_eq!(measured.aggregate_word_reads, 0);
    }
}
