use super::MacroParameterPattern;
use crate::token::{Catcode, Token, TokenWord};

#[test]
fn packed_words_preserve_parameter_program_boundaries() {
    let tokens = [
        Token::Char {
            ch: b'a'.into(),
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: '#',
            cat: Catcode::Parameter,
        },
        Token::param(1),
        Token::Char {
            ch: b','.into(),
            cat: Catcode::Other,
        },
        Token::param(2),
        Token::Char {
            ch: b'z'.into(),
            cat: Catcode::Letter,
        },
    ];
    let words = tokens.map(TokenWord::pack);

    let pattern = MacroParameterPattern::from_words(&words);

    assert_eq!(pattern.parameter_count(), 2);
    assert_eq!(pattern.leading_end(words.len()), 1);
    assert_eq!(pattern.marker_index(0), Some(1));
    assert_eq!(pattern.delimiter_bounds(0, words.len()), (3, 4));
    assert_eq!(pattern.marker_index(1), None);
    assert_eq!(pattern.delimiter_bounds(1, words.len()), (5, 6));
}
