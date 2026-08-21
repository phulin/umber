use tex_state::env::AssignmentScope;
use tex_state::meaning::{Meaning, MeaningFlags, MeaningWord, ResolvedMeaning};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use super::{CurrentCommand, DeliveryStamp};

fn resolved<G>(universe: &mut tex_state::Universe<G>, token: Token) -> CurrentCommand<G> {
    CurrentCommand::resolve(
        TracedTokenWord::pack(token, OriginId::UNKNOWN),
        DeliveryStamp::new(17, 23, 29),
        None,
        false,
        &universe.command_context().expect("command context"),
    )
}

#[test]
fn delivered_command_keeps_the_resolved_meaning_and_exact_spelling() {
    crate::test_harness::with_universe(|universe| {
        let symbol = universe.intern("defined").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::CharGiven('A')),
                AssignmentScope::Global,
            )
            .expect("first meaning");
        let command = resolved(universe, Token::Cs(symbol.symbol()));
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::CharGiven('B')),
                AssignmentScope::Global,
            )
            .expect("replacement meaning");

        assert_eq!(
            command.spelling().semantic_token(),
            Token::Cs(symbol.symbol())
        );
        assert_eq!(command.meaning(), Meaning::CharGiven('A'));
        assert_eq!(command.delivery_stamp(), DeliveryStamp::new(17, 23, 29));
    });
}

#[test]
fn ordinary_character_is_resolved_without_a_state_handle() {
    crate::test_harness::with_universe(|universe| {
        let command = resolved(
            universe,
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        assert_eq!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                ch: 'x',
                cat: Catcode::Letter,
            })
        );
        assert_eq!(command.control_sequence(), None);
    });
}

#[test]
fn macro_delivery_carries_a_generation_typed_definition_coordinate() {
    crate::test_harness::with_universe(|universe| {
        let replacement = TokenWord::pack(Token::Char {
            ch: 'M',
            cat: Catcode::Letter,
        });
        let definition = universe
            .allocate_definition(&[], &[replacement])
            .expect("definition");
        let symbol = universe.intern("macro").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::LONG, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");

        let command = resolved(universe, Token::Cs(symbol.symbol()));
        let ResolvedMeaning::Macro {
            flags,
            definition: delivered,
        } = command.meaning()
        else {
            panic!("macro meaning")
        };
        assert_eq!(flags, MeaningFlags::LONG);
        assert_eq!(delivered, definition);
        let context = universe.command_context().expect("context");
        assert_eq!(
            context.definition(delivered).replacement_text(),
            &[replacement]
        );
    });
}
