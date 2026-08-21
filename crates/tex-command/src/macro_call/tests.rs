use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use super::{MacroArgumentBuildError, MacroArgumentBuilder, MacroParameterEscape, ParameterState};
use crate::attempt::AttemptArena;

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
        let arguments = builder.finish(&mut attempt).expect("argument record");
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
        let mut parameters = ParameterState::default();
        let first =
            parameters.push_activation(name, definition, Default::default(), OriginId::UNKNOWN);
        let second =
            parameters.push_activation(name, definition, Default::default(), OriginId::UNKNOWN);
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
