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

    let pattern = MacroParameterPattern::from_words(&words).expect("valid parameter program");

    assert_eq!(pattern.parameter_count(), 2);
    assert_eq!(pattern.leading_end(words.len()), 1);
    assert_eq!(pattern.marker_index(0), Some(1));
    assert_eq!(pattern.delimiter_bounds(0, words.len()), (3, 4));
    assert_eq!(pattern.marker_index(1), None);
    assert_eq!(pattern.delimiter_bounds(1, words.len()), (5, 6));
}

#[test]
fn sealed_failure_links_retain_the_longest_literal_suffix() {
    // TeX82 §394 commits only words outside the longest reusable prefix.
    // Exhaust all short binary delimiters and mismatch boundaries, comparing
    // with the literal suffix rule rather than a second failure-table builder.
    let alphabet = [
        TokenWord::pack(Token::frozen_relax()),
        TokenWord::pack(Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        }),
    ];
    for len in 1..=7 {
        for bits in 0..(1 << len) {
            let words: Vec<_> = std::iter::once(TokenWord::pack(Token::param(1)))
                .chain((0..len).map(|index| alphabet[(bits >> index) & 1]))
                .collect();
            let pattern = MacroParameterPattern::from_words(&words).expect("parameter program");
            let delimiter = pattern.delimiter(0, &words);
            for prefix in 0..len {
                for current in alphabet {
                    if current == delimiter.words()[prefix] {
                        continue;
                    }
                    let held: Vec<_> = delimiter.words()[..prefix]
                        .iter()
                        .copied()
                        .chain(std::iter::once(current))
                        .collect();
                    let expected = (0..len)
                        .rev()
                        .find(|&retained| {
                            retained <= held.len()
                                && held[held.len() - retained..] == delimiter.words()[..retained]
                        })
                        .expect("empty suffix always matches");
                    assert_eq!(delimiter.overlap(prefix, current), expected);
                }
            }
        }
    }
}

#[test]
fn sealed_delimiters_keep_catcodes_and_parameter_boundaries() {
    let letter = TokenWord::pack(Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    });
    let other = TokenWord::pack(Token::Char {
        ch: 'a',
        cat: Catcode::Other,
    });
    let words = [
        letter,
        TokenWord::pack(Token::param(1)),
        letter,
        other,
        letter,
        TokenWord::pack(Token::param(2)),
        other,
        other,
        letter,
    ];
    let pattern = MacroParameterPattern::from_words(&words).expect("parameter program");
    let first = pattern.delimiter(0, &words);
    let second = pattern.delimiter(1, &words);
    assert_eq!(first.words(), &[letter, other, letter]);
    assert_eq!(first.overlap(2, other), 0);
    assert_eq!(second.words(), &[other, other, letter]);
    assert_eq!(second.overlap(2, other), 2);
}
