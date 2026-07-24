use tex_state::Universe;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{CurrentCommand, DeliveryStamp};

fn resolve(universe: &mut Universe, token: Token, origin: OriginId) -> CurrentCommand {
    let mut state = universe.command_context();
    CurrentCommand::resolve(
        TracedTokenWord::pack(token, origin),
        DeliveryStamp::new(17, 23, 29),
        None,
        &mut state,
    )
}

#[test]
fn control_sequence_spelling_survives_a_later_meaning_change() {
    let mut universe = Universe::new();
    let symbol = universe.intern("defined").symbol();
    universe.set_meaning(symbol, Meaning::CharGiven('A'));

    let command = resolve(&mut universe, Token::Cs(symbol), OriginId::UNKNOWN);
    universe.set_meaning(symbol, Meaning::CharGiven('B'));

    assert_eq!(command.spelling().semantic_token(), Token::Cs(symbol));
    assert_eq!(command.control_sequence(), Some(symbol));
    assert_eq!(command.meaning(), Meaning::CharGiven('A'));
    assert_eq!(command.origin(), OriginId::UNKNOWN);
    assert_eq!(command.delivery_stamp().input_level(), 17);
    assert_eq!(command.delivery_stamp().position(), 23);
    assert_eq!(command.delivery_stamp().sequence(), 29);
}

#[test]
fn active_character_uses_its_distinct_control_sequence_meaning() {
    let mut universe = Universe::new();
    let named = universe.intern("~").symbol();
    let active = universe.intern_active_character('~').symbol();
    universe.set_meaning(named, Meaning::CharGiven('N'));
    universe.set_meaning(active, Meaning::CharGiven('A'));

    let command = resolve(
        &mut universe,
        Token::Char {
            ch: '~',
            cat: Catcode::Active,
        },
        OriginId::UNKNOWN,
    );

    assert_eq!(command.control_sequence(), Some(active));
    assert_eq!(command.meaning(), Meaning::CharGiven('A'));
    assert_ne!(command.control_sequence(), Some(named));
}

#[test]
fn ordinary_character_has_its_literal_token_meaning() {
    let mut universe = Universe::new();

    let command = resolve(
        &mut universe,
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        OriginId::UNKNOWN,
    );

    assert_eq!(command.control_sequence(), None);
    assert_eq!(
        command.meaning(),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter,
        }
    );
}
