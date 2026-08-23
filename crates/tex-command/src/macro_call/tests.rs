use tex_state::env::AssignmentScope;
use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use super::{
    MacroArgumentBuildError, MacroArgumentBuilder, MacroCallOutcome, MacroParameterEscape,
    ParameterState,
};
use crate::attempt::AttemptArena;
use crate::{CommandHostCapabilities, CommandState};

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

#[test]
fn arguments_are_completed_in_tex_parameter_order_and_live_in_the_attempt() {
    crate::test_harness::with_universe(|_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let scope = attempt.begin_owned_scope().expect("argument scope");
        let first = attempt.allocate_token_list([word('a')]).expect("first");
        let second = attempt.allocate_token_list([]).expect("second");
        let mut builder = MacroArgumentBuilder::default();
        builder.complete(1, first).expect("slot one");
        assert_eq!(
            builder.complete(3, second),
            Err(MacroArgumentBuildError::OutOfOrderSlot {
                expected: 2,
                actual: 3,
            })
        );
        builder.complete(2, second).expect("slot two");
        let arguments = builder
            .finish(&mut attempt, scope)
            .expect("argument record");
        assert_eq!(
            attempt.arguments(arguments.record().expect("nonempty record")),
            Ok(&[first, second][..])
        );
    });
}

#[test]
fn activation_retirement_drops_only_the_current_macro_frame() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(Token::Param(1))])
            .expect("definition");
        let name = universe.intern("m").expect("name").symbol();
        let mut attempt = AttemptArena::<()>::default();
        let first_scope = attempt.begin_owned_scope().expect("first scope");
        let first_arguments = MacroArgumentBuilder::default()
            .finish(&mut attempt, first_scope)
            .expect("first arguments");
        let second_scope = attempt.begin_owned_scope().expect("second scope");
        let second_arguments = MacroArgumentBuilder::default()
            .finish(&mut attempt, second_scope)
            .expect("second arguments");
        let mut parameters = ParameterState::default();
        let first =
            parameters.push_activation(name, definition, first_arguments, OriginId::UNKNOWN);
        let second =
            parameters.push_activation(name, definition, second_arguments, OriginId::UNKNOWN);
        assert_ne!(first, second);
        assert_eq!(parameters.activations.len(), 2);
        parameters.retire_last_activation();
        assert_eq!(parameters.activations.len(), 1);
        assert_eq!(parameters.activations[0].identity, first);
    });
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

fn active_argument_tokens<G>(processor: &CommandState<G>) -> Vec<Token> {
    let arguments = processor
        .parameters
        .activations
        .last()
        .expect("macro activation")
        .arguments;
    let record = arguments.record().expect("argument record");
    processor
        .attempt
        .arena()
        .arguments(record)
        .expect("argument lists")
        .iter()
        .flat_map(|argument| {
            processor
                .attempt
                .arena()
                .token_words(*argument)
                .expect("argument words")
                .iter()
                .map(|word| word.semantic_token())
        })
        .collect()
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
