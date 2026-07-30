//! tex.web §§49, §262--§294's printable token spellings.
//!
//! `show_token_list` and `print_cs` need nothing beyond the interner, the
//! catcode table, and `\escapechar`, all of which this crate owns. They live
//! here rather than in the gullet so that every layer that must render a
//! token -- the value scanners, `\meaning`, and §310's `show_context` -- reads
//! one implementation.

use crate::env::banks::IntParam;
use crate::interner::ControlSequenceKind;
use crate::token::{Catcode, Token};
use crate::universe::ExpansionState;

/// Appends the form TeX82's `show_token_list` prints for one token.
///
/// In `tex.web` section 262, `print_cs` always terminates hash-table control
/// sequence names with a space. Direct-address single-character names only
/// receive that space when the character's current catcode is `letter`, and
/// active characters receive neither an escape nor a trailing space.
pub fn append_token_show_text(stores: &impl ExpansionState, token: Token, text: &mut String) {
    if let Token::Char { ch, cat } = token {
        append_tex_print_char(ch, text);
        if cat == Catcode::Parameter {
            append_tex_print_char(ch, text);
        }
    } else {
        text.push_str(&token_text(stores, token));
    }
    let Token::Cs(symbol) = token else {
        return;
    };
    if stores.control_sequence_kind(symbol) == ControlSequenceKind::ActiveCharacter {
        return;
    }

    let name = stores.resolve(symbol);
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if stores.catcode(ch) != Catcode::Letter => {}
        _ => text.push(' '),
    }
}

/// Appends the token text TeX builds with `selector = new_string`.
///
/// Unlike ordinary diagnostic display, character tokens remain raw; control
/// sequence spelling and its separator still follow `show_token_list`.
pub fn append_token_string_text(stores: &impl ExpansionState, token: Token, text: &mut String) {
    if let Token::Char { ch, cat } = token {
        text.push(ch);
        if cat == Catcode::Parameter {
            text.push(ch);
        }
    } else {
        append_token_show_text(stores, token, text);
    }
}

/// Appends one token as TeX82's `show_token_list` prints it through an active
/// output selector.
///
/// Section 262 sends every character through `print`. Section 59's `print`
/// recognizes the live new-line character before expanding any other
/// non-printable byte to its canonical `^^` spelling.
pub fn append_token_selector_text(
    stores: &impl ExpansionState,
    token: Token,
    newlinechar: Option<char>,
    text: &mut String,
) {
    let mut raw = String::new();
    append_token_string_text(stores, token, &mut raw);
    for ch in raw.chars() {
        if Some(ch) == newlinechar {
            text.push('\n');
        } else {
            append_tex_print_char(ch, text);
        }
    }
}

/// Appends TeX82's printable string for a character code.
///
/// `show_token_list` calls `print(c)`, not `print_char(c)`. The first 256
/// TeX strings therefore render non-printable bytes as `^^A`, `^^?`, or
/// lowercase hexadecimal `^^80` forms (tex.web sections 49 and 262).
pub fn append_tex_print_char(ch: char, text: &mut String) {
    let code = ch as u32;
    match code {
        0..=31 => {
            text.push_str("^^");
            text.push(char::from_u32(code + 64).expect("ASCII control marker"));
        }
        32..=126 => text.push(ch),
        127 => text.push_str("^^?"),
        128..=255 => {
            use std::fmt::Write as _;
            let _ = write!(text, "^^{code:02x}");
        }
        _ => text.push(ch),
    }
}

/// The characters TeX82 §69's `\string` produces for one token.
#[must_use]
pub fn token_text(stores: &impl ExpansionState, token: Token) -> String {
    string_tokens(stores, token)
        .into_iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
        })
        .collect()
}

/// TeX82 §69's `\string` expansion of one token, as `other`/`space` tokens.
#[must_use]
pub fn string_tokens(stores: &impl ExpansionState, token: Token) -> Vec<Token> {
    match token {
        Token::Char { ch, .. } => vec![rendered_char(ch)],
        Token::Cs(symbol) => {
            let name = stores.resolve(symbol);
            let escape = escapechar(stores);
            let kind = stores.control_sequence_kind(symbol);
            let capacity = match kind {
                ControlSequenceKind::ActiveCharacter => name.chars().count(),
                ControlSequenceKind::Named if name.is_empty() => {
                    "csname".len() + "endcsname".len() + 2 * usize::from(escape.is_some())
                }
                ControlSequenceKind::Named => name.chars().count() + usize::from(escape.is_some()),
            };
            let mut out = Vec::with_capacity(capacity);
            match kind {
                ControlSequenceKind::ActiveCharacter => {
                    out.extend(name.chars().map(rendered_char));
                }
                ControlSequenceKind::Named if name.is_empty() => {
                    append_escaped_text(escape, "csname", &mut out);
                    append_escaped_text(escape, "endcsname", &mut out);
                }
                ControlSequenceKind::Named => {
                    append_escaped_text(escape, name, &mut out);
                }
            }
            out
        }
        Token::Param(slot) => text_tokens(&format!("#{slot}")),
        Token::Frozen(_) => text_tokens("\\endtemplate"),
    }
}

/// Renders a string as the `other`/`space` token list TeX82 §69 produces.
#[must_use]
pub fn text_tokens(text: &str) -> Vec<Token> {
    text.chars().map(rendered_char).collect()
}

/// TeX82 §69's category assignment for one rendered character.
#[must_use]
pub const fn rendered_char(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch == ' ' {
            Catcode::Space
        } else {
            Catcode::Other
        },
    }
}

fn append_escaped_text(escape: Option<char>, value: &str, out: &mut Vec<Token>) {
    if let Some(escape) = escape {
        out.push(rendered_char(escape));
    }
    out.extend(value.chars().map(rendered_char));
}

fn escapechar(stores: &impl ExpansionState) -> Option<char> {
    u32::try_from(stores.int_param(IntParam::ESCAPE_CHAR))
        .ok()
        .filter(|&value| value < 256)
        .and_then(char::from_u32)
}
