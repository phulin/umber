use super::{Catcode, OriginId, Token, TracedTokenWord};
use crate::interner::Symbol;

#[test]
fn token_is_one_word() {
    assert_eq!(core::mem::size_of::<Token>(), 8);
    assert_eq!(core::mem::size_of::<OriginId>(), 4);
    assert_eq!(core::mem::size_of::<TracedTokenWord>(), 8);
}

#[test]
fn token_variants_are_copy_and_comparable() {
    let char_token = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    let cs_token = Token::Cs(Symbol::new(7));
    let param_token = Token::param(3);

    assert_eq!(char_token, char_token);
    assert_eq!(cs_token, Token::Cs(Symbol::new(7)));
    assert_eq!(param_token, Token::Param(3));
}

#[test]
fn origin_zero_is_unknown() {
    assert_eq!(OriginId::UNKNOWN.raw(), 0);
    assert_eq!(OriginId::default(), OriginId::UNKNOWN);
}

#[test]
fn char_token_round_trips_with_origin() {
    let origin = OriginId::from_raw(42);
    let token = Token::Char {
        ch: '🙂',
        cat: Catcode::Active,
    };

    let packed = TracedTokenWord::pack(token, origin);

    assert_eq!(packed.unpack(), Some((token, origin)));
}

#[test]
fn control_sequence_token_round_trips_with_origin() {
    let origin = OriginId::from_raw(u32::MAX);
    let token = Token::Cs(Symbol::new((1 << 30) - 1));

    let packed = TracedTokenWord::pack(token, origin);

    assert_eq!(packed.unpack(), Some((token, origin)));
}

#[test]
fn parameter_token_round_trips_with_origin() {
    let origin = OriginId::from_raw(7);
    let token = Token::param(9);

    let packed = TracedTokenWord::pack(token, origin);

    assert_eq!(packed.unpack(), Some((token, origin)));
}
