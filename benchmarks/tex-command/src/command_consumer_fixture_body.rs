use tex_state::interner::Symbol;
use tex_state::token::{Catcode, Token, TokenWord};

use super::BODY_CHARACTER;

pub(crate) fn expected_body_checksum(words: usize) -> u64 {
    (0..words).fold(0_u64, |checksum, index| {
        checksum.wrapping_add(body_token_value(index, words))
    })
}

pub(super) fn append_source_body(source: &mut String, words: usize) {
    for index in 0..words {
        match body_token_kind(index, words) {
            0 => source.push_str("\\consumerbodytoken"),
            1 => source.push('{'),
            2 | 4 | 7 => source.push(BODY_CHARACTER),
            3 => source.push('{'),
            5 | 6 => source.push('}'),
            _ => unreachable!(),
        }
    }
}

pub(super) fn append_stored_body(words: &mut Vec<TokenWord>, body_control: Symbol, count: usize) {
    for index in 0..count {
        words.push(TokenWord::pack(body_token(body_control, index, count)));
    }
}

pub(crate) fn body_tokens(body_control: Symbol, count: usize) -> Vec<Token> {
    (0..count)
        .map(|index| body_token(body_control, index, count))
        .collect()
}

pub(super) fn body_token(body_control: Symbol, index: usize, total: usize) -> Token {
    match body_token_kind(index, total) {
        0 => Token::Cs(body_control),
        1 | 3 => begin_group_token(),
        2 | 4 => Token::Char {
            ch: BODY_CHARACTER,
            cat: Catcode::Letter,
        },
        5 | 6 => end_group_token(),
        _ => Token::Char {
            ch: BODY_CHARACTER,
            cat: Catcode::Letter,
        },
    }
}

fn body_token_kind(index: usize, total: usize) -> usize {
    let complete_words = total / 7 * 7;
    if index >= complete_words {
        7
    } else {
        index % 7
    }
}

pub(super) fn body_token_value(index: usize, total: usize) -> u64 {
    match body_token_kind(index, total) {
        0 => 0xB0D1_C501,
        1 | 3 => 0xB0D1_C502,
        2 | 4 => BODY_CHARACTER as u64,
        5 | 6 => 0xB0D1_C503,
        7 => BODY_CHARACTER as u64,
        _ => unreachable!(),
    }
}

fn begin_group_token() -> Token {
    Token::Char {
        ch: '{',
        cat: Catcode::BeginGroup,
    }
}

fn end_group_token() -> Token {
    Token::Char {
        ch: '}',
        cat: Catcode::EndGroup,
    }
}
