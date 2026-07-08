use super::{Catcode, Token};
use crate::interner::Symbol;

#[test]
fn token_is_one_word() {
    assert_eq!(core::mem::size_of::<Token>(), 8);
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
