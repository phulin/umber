use tex_state::env::AssignmentScope;
use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

use super::{MacroCallOutcome, MacroParameterEscape};
use crate::{CommandHostCapabilities, CommandState};
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
            MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
            AssignmentScope::Global,
        )
        .expect("macro meaning");
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor
                .get_x_token()
                .expect("nested expansion")
                .expect("inner result")
                .spelling()
                .semantic_token(),
            letter('i')
        );
        assert_eq!(processor.command.scratch.frame_len(), 2);
        assert_eq!(
            processor
                .get_x_token()
                .expect("outer tail")
                .expect("tail token")
                .spelling()
                .semantic_token(),
            letter('t')
        );
        assert_eq!(processor.command.scratch.frame_len(), 1);
        assert_eq!(
            processor
                .get_x_token()
                .expect("source separator")
                .expect("separator token")
                .spelling()
                .semantic_token(),
            letter('n')
        );
        assert_eq!(processor.command.scratch.frame_len(), 0);

        assert_eq!(
            processor
                .get_x_token()
                .expect("tail expansion")
                .expect("tail result")
                .spelling()
                .semantic_token(),
            letter('i')
        );
        assert_eq!(processor.command.scratch.frame_len(), 1);
        assert_eq!(processor.command.scratch.retained_slot_len(), 2);
        assert_eq!(processor.command.scratch.copied_macro_words(), 0);
        assert_eq!(
            processor
                .get_x_token()
                .expect("post-tail source")
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let mut actual = Vec::new();
        for _ in 0..expected.len() * 2 {
            actual.push(
                processor
                    .get_x_token()
                    .expect("parameter replay")
                    .expect("argument token")
                    .spelling()
                    .semantic_token(),
            );
        }
        assert_eq!(actual[..expected.len()], expected);
        assert_eq!(actual[expected.len()..], expected);
        assert_eq!(
            processor
                .get_x_token()
                .expect("following source")
                .expect("following token")
                .spelling()
                .semantic_token(),
            letter('z')
        );
        assert!(processor.command.scratch.is_quiescent());
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let call = processor
            .get_next()
            .expect("macro delivery")
            .expect("macro command");

        assert_eq!(
            processor.macro_call(&call).expect("macro call"),
            MacroCallOutcome::Activated
        );
        assert_eq!(active_argument_tokens(processor.command), [letter('x')]);
        assert_eq!(
            processor
                .get_x_token()
                .expect("replacement")
                .expect("argument replay")
                .spelling()
                .semantic_token(),
            letter('x')
        );
        assert_eq!(
            processor
                .get_x_token()
                .expect("following source")
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let call = processor
            .get_next()
            .expect("macro delivery")
            .expect("macro command");

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
