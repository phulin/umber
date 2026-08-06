//! tex.web §§49, §262--§294's printable token spellings.
//!
//! `show_token_list` and `print_cs` need nothing beyond the interner, the
//! catcode table, and `\escapechar`, all of which this crate owns. They live
//! here rather than in the gullet so that every layer that must render a
//! token -- the value scanners, `\meaning`, and §310's `show_context` -- reads
//! one implementation.

use crate::Universe;
use crate::env::banks::IntParam;
use crate::interner::ControlSequenceKind;
use crate::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use crate::token::{Catcode, Token};

/// Renders the meaning TeX82's `\meaning` and `\show` expose for one token.
#[must_use]
pub fn meaning_text(stores: &Universe, token: Token) -> String {
    match token {
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores.active_character_symbol(ch).map_or_else(
            || "undefined".to_owned(),
            |symbol| meaning_text(stores, Token::Cs(symbol.symbol())),
        ),
        Token::Char {
            ch,
            cat: Catcode::Letter,
        } => format!("the letter {ch}"),
        Token::Char { ch, .. } => format!("the character {ch}"),
        Token::Param(slot) => format!("macro parameter character #{slot}"),
        Token::Frozen(_) => "end of alignment template".to_owned(),
        Token::Cs(symbol) => match stores.meaning(symbol) {
            Meaning::Undefined => "undefined".to_owned(),
            Meaning::Relax => "\\relax".to_owned(),
            Meaning::EndV => "\\endtemplate".to_owned(),
            Meaning::CharGiven(ch) => format!("the character {ch}"),
            Meaning::CharToken {
                ch,
                cat: Catcode::Letter,
            } => format!("the letter {ch}"),
            Meaning::CharToken { ch, .. } => format!("the character {ch}"),
            Meaning::MathCharGiven(value) => format!("\\mathchar\"{value:X}"),
            Meaning::CountRegister(index) => format!("\\count{index}"),
            Meaning::DimenRegister(index) => format!("\\dimen{index}"),
            Meaning::SkipRegister(index) => format!("\\skip{index}"),
            Meaning::MuskipRegister(index) => format!("\\muskip{index}"),
            Meaning::ToksRegister(index) => format!("\\toks{index}"),
            Meaning::IntParam(_)
            | Meaning::InternalInteger(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_) => format!("\\{}", stores.resolve(symbol)),
            Meaning::Font(font) => format!("select font {}", stores.font_name(font)),
            meaning @ Meaning::ExpandablePrimitive(_) => format!(
                "\\{}",
                stores
                    .primitive_name(meaning)
                    .unwrap_or_else(|| stores.resolve(symbol))
            ),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Radical) => {
                "\\radical".to_owned()
            }
            meaning @ Meaning::UnexpandablePrimitive(_) => format!(
                "\\{}",
                stores
                    .primitive_name(meaning)
                    .unwrap_or_else(|| stores.resolve(symbol))
            ),
            Meaning::Macro { flags, definition } => {
                let macro_meaning = stores.macro_definition(definition);
                let mut text = macro_prefix(flags);
                text.push_str("macro:");
                append_token_list(stores, macro_meaning.parameter_text(), &mut text);
                text.push_str("->");
                append_token_list(stores, macro_meaning.replacement_text(), &mut text);
                text
            }
            Meaning::Unknown(_) => "unknown".to_owned(),
        },
    }
}

/// TeX82 §252's `show_eqtb` meaning text, bounded like `show_token_list`.
#[must_use]
pub fn bounded_meaning_text(stores: &Universe, token: Token, breadth: usize) -> String {
    let Token::Cs(symbol) = token else {
        return meaning_text(stores, token);
    };
    let Meaning::Macro { flags, definition } = stores.meaning(symbol) else {
        return meaning_text(stores, token);
    };
    let macro_meaning = stores.macro_definition(definition);
    let mut text = macro_prefix(flags);
    text.push_str("macro:");
    let parameter = stores.tokens(macro_meaning.parameter_text());
    let replacement = stores.tokens(macro_meaning.replacement_text());
    let mut shown = 0;
    let mut tally = 0;
    while shown < parameter.len() && tally < breadth {
        let before = text.chars().count();
        append_token_show_text(stores, parameter[shown], &mut text);
        tally += text.chars().count() - before;
        shown += 1;
    }
    let mut remaining = shown < parameter.len();
    if !remaining {
        if tally < breadth {
            text.push_str("->");
            tally += 2;
            shown = 0;
            while shown < replacement.len() && tally < breadth {
                let before = text.chars().count();
                append_token_show_text(stores, replacement[shown], &mut text);
                tally += text.chars().count() - before;
                shown += 1;
            }
            remaining = shown < replacement.len();
        } else {
            remaining = true;
        }
    }
    if remaining {
        text.push_str("\\ETC.");
    }
    text
}

fn macro_prefix(flags: MeaningFlags) -> String {
    let mut text = String::new();
    for (flag, label) in [
        (MeaningFlags::PROTECTED, "\\protected"),
        (MeaningFlags::LONG, "\\long"),
        (MeaningFlags::OUTER, "\\outer"),
    ] {
        if flags.contains(flag) {
            text.push_str(label);
        }
    }
    if flags.bits() & (MeaningFlags::PROTECTED | MeaningFlags::LONG | MeaningFlags::OUTER).bits()
        != 0
    {
        text.push(' ');
    }
    text
}

fn append_token_list(stores: &Universe, list: crate::ids::TokenListId, text: &mut String) {
    let tokens = stores.tokens(list);
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        if let Token::Char {
            ch,
            cat: Catcode::Parameter,
        } = token
            && let Some(Token::Param(slot)) = tokens.get(index + 1)
        {
            append_tex_print_char(ch, text);
            text.push(char::from(b'0' + *slot));
            index += 2;
            continue;
        }
        append_token_show_text(stores, token, text);
        index += 1;
    }
}

/// Appends the form TeX82's `show_token_list` prints for one token.
///
/// In `tex.web` section 262, `print_cs` always terminates hash-table control
/// sequence names with a space. Direct-address single-character names only
/// receive that space when the character's current catcode is `letter`, and
/// active characters receive neither an escape nor a trailing space.
pub fn append_token_show_text(stores: &Universe, token: Token, text: &mut String) {
    if let Token::Char { ch, cat } = token {
        append_tex_print_char(ch, text);
        if cat == Catcode::Parameter {
            append_tex_print_char(ch, text);
        }
    } else {
        append_non_character_token_text(stores, token, text);
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

fn append_non_character_token_text(stores: &Universe, token: Token, text: &mut String) {
    match token {
        Token::Cs(symbol) => {
            let name = stores.resolve(symbol);
            let escape = escapechar(stores);
            match stores.control_sequence_kind(symbol) {
                ControlSequenceKind::ActiveCharacter => text.push_str(name),
                ControlSequenceKind::Null => {
                    if let Some(escape) = escape {
                        text.push(escape);
                    }
                    text.push_str("csname");
                    if let Some(escape) = escape {
                        text.push(escape);
                    }
                    text.push_str("endcsname");
                }
                ControlSequenceKind::SingleCharacter
                | ControlSequenceKind::Named
                | ControlSequenceKind::Internal => {
                    if let Some(escape) = escape {
                        text.push(escape);
                    }
                    text.push_str(name);
                }
            }
        }
        Token::Param(slot) => {
            text.push('#');
            text.push(char::from(b'0' + slot));
        }
        Token::Frozen(_) => text.push_str("\\endtemplate"),
        Token::Char { .. } => unreachable!("character tokens are handled by the caller"),
    }
}

/// Appends the token text TeX builds with `selector = new_string`.
///
/// Unlike ordinary diagnostic display, character tokens remain raw; control
/// sequence spelling and its separator still follow `show_token_list`.
pub fn append_token_string_text(stores: &Universe, token: Token, text: &mut String) {
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
    stores: &Universe,
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
pub fn token_text(stores: &Universe, token: Token) -> String {
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
pub fn string_tokens(stores: &Universe, token: Token) -> Vec<Token> {
    match token {
        Token::Char { ch, .. } => vec![rendered_char(ch)],
        Token::Cs(symbol) => {
            let name = stores.resolve(symbol);
            let escape = escapechar(stores);
            let kind = stores.control_sequence_kind(symbol);
            let capacity = match kind {
                ControlSequenceKind::ActiveCharacter => name.chars().count(),
                ControlSequenceKind::Null => {
                    "csname".len() + "endcsname".len() + 2 * usize::from(escape.is_some())
                }
                ControlSequenceKind::SingleCharacter
                | ControlSequenceKind::Named
                | ControlSequenceKind::Internal => {
                    name.chars().count() + usize::from(escape.is_some())
                }
            };
            let mut out = Vec::with_capacity(capacity);
            match kind {
                ControlSequenceKind::ActiveCharacter => {
                    out.extend(name.chars().map(rendered_char));
                }
                ControlSequenceKind::Null => {
                    append_escaped_text(escape, "csname", &mut out);
                    append_escaped_text(escape, "endcsname", &mut out);
                }
                ControlSequenceKind::SingleCharacter
                | ControlSequenceKind::Named
                | ControlSequenceKind::Internal => {
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

fn escapechar(stores: &Universe) -> Option<char> {
    u32::try_from(stores.int_param(IntParam::ESCAPE_CHAR))
        .ok()
        .filter(|&value| value < 256)
        .and_then(char::from_u32)
}
