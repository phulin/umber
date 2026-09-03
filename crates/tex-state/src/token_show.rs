//! tex.web §§49, §262--§294's printable token spellings.
//!
//! `show_token_list` and `print_cs` need nothing beyond the interner, the
//! catcode table, and `\escapechar`, all of which this crate owns. They live
//! here rather than in the gullet so that every layer that must render a
//! token -- the value scanners, `\meaning`, and §310's `show_context` -- reads
//! one implementation.

use crate::definition_arena::DefinitionRef;
use crate::env::banks::IntParam;
use crate::interner::ControlSequenceKind;
use crate::meaning::{Meaning, MeaningFlags, ResolvedMeaning, UnexpandablePrimitive};
use crate::token::{Catcode, Token, TokenWord};
use crate::{CommandContext, Universe};

/// Borrow-only state needed by TeX's token-spelling routines.
pub trait TokenDisplayState {
    fn display_resolve(&self, symbol: crate::interner::Symbol) -> Option<&str>;
    fn display_control_sequence_kind(
        &self,
        symbol: crate::interner::Symbol,
    ) -> Option<ControlSequenceKind>;
    fn display_catcode(&self, ch: char) -> Catcode;
    fn display_frozen_primitive_name(&self, token: Token) -> Option<&str>;
    fn display_escape_char(&self) -> i32;
}

impl<G> TokenDisplayState for Universe<G> {
    fn display_resolve(&self, symbol: crate::interner::Symbol) -> Option<&str> {
        self.resolve(symbol)
    }

    fn display_control_sequence_kind(
        &self,
        symbol: crate::interner::Symbol,
    ) -> Option<ControlSequenceKind> {
        self.control_sequence_kind(symbol)
    }

    fn display_catcode(&self, ch: char) -> Catcode {
        self.catcode(ch)
    }

    fn display_frozen_primitive_name(&self, token: Token) -> Option<&str> {
        self.frozen_primitive_name(token)
    }

    fn display_escape_char(&self) -> i32 {
        self.int_param(IntParam::ESCAPE_CHAR)
    }
}

pub(crate) trait MeaningDisplayState<G>: TokenDisplayState {
    fn display_active_character_symbol(&self, ch: char) -> Option<crate::interner::Symbol>;
    fn display_meaning(&self, symbol: crate::interner::Symbol) -> Option<ResolvedMeaning<G>>;
    fn display_macro_words(&self, definition: DefinitionRef<G>)
    -> (Vec<TokenWord>, Vec<TokenWord>);
    fn display_font_name(&self, font: crate::ids::FontId) -> String;
    fn display_primitive_name(&self, meaning: Meaning) -> Option<&str>;
    fn display_symbol_name(&self, symbol: crate::interner::Symbol) -> Option<&str>;
}

impl<G> MeaningDisplayState<G> for Universe<G> {
    fn display_active_character_symbol(&self, ch: char) -> Option<crate::interner::Symbol> {
        self.active_character_symbol(ch)
            .map(|symbol| symbol.symbol())
    }

    fn display_meaning(&self, symbol: crate::interner::Symbol) -> Option<ResolvedMeaning<G>> {
        self.meaning(symbol).ok()
    }

    fn display_macro_words(
        &self,
        definition: DefinitionRef<G>,
    ) -> (Vec<TokenWord>, Vec<TokenWord>) {
        let admitted = self.admitted().expect("live universe");
        let definition = admitted.definition(definition);
        (
            definition.parameter_text().to_vec(),
            definition.replacement_text().to_vec(),
        )
    }

    fn display_font_name(&self, font: crate::ids::FontId) -> String {
        self.font_name(font).to_owned()
    }

    fn display_primitive_name(&self, meaning: Meaning) -> Option<&str> {
        self.primitive_name(meaning)
    }

    fn display_symbol_name(&self, symbol: crate::interner::Symbol) -> Option<&str> {
        self.resolve(symbol)
    }
}

impl<G> MeaningDisplayState<G> for CommandContext<'_, G> {
    fn display_active_character_symbol(&self, ch: char) -> Option<crate::interner::Symbol> {
        self.active_character_symbol(ch)
    }

    fn display_meaning(&self, symbol: crate::interner::Symbol) -> Option<ResolvedMeaning<G>> {
        Some(self.meaning(symbol))
    }

    fn display_macro_words(
        &self,
        definition: DefinitionRef<G>,
    ) -> (Vec<TokenWord>, Vec<TokenWord>) {
        let definition = self.definition(definition);
        (
            definition.parameter_text().to_vec(),
            definition.replacement_text().to_vec(),
        )
    }

    fn display_font_name(&self, font: crate::ids::FontId) -> String {
        self.font_name(font)
    }

    fn display_primitive_name(&self, meaning: Meaning) -> Option<&str> {
        self.primitive_name(meaning)
    }

    fn display_symbol_name(&self, symbol: crate::interner::Symbol) -> Option<&str> {
        Some(self.resolve(symbol))
    }
}

/// Renders the meaning TeX82's `\meaning` and `\show` expose for one token.
#[must_use]
pub fn meaning_text<G>(stores: &Universe<G>, token: Token) -> String {
    meaning_text_with(stores, token)
}

pub(crate) fn meaning_text_with<G, S: MeaningDisplayState<G> + ?Sized>(
    stores: &S,
    token: Token,
) -> String {
    match token {
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores.display_active_character_symbol(ch).map_or_else(
            || "undefined".to_owned(),
            |symbol| meaning_text_with(stores, Token::Cs(symbol)),
        ),
        Token::Char {
            ch,
            cat: Catcode::Letter,
        } => format!("the letter {ch}"),
        Token::Char { ch, .. } => format!("the character {ch}"),
        Token::Param(slot) => format!("macro parameter character #{slot}"),
        Token::Frozen(_) => "end of alignment template".to_owned(),
        Token::Cs(symbol) => match stores.display_meaning(symbol) {
            Some(ResolvedMeaning::Static(Meaning::Undefined)) | None => "undefined".to_owned(),
            Some(ResolvedMeaning::Static(Meaning::Relax)) => "\\relax".to_owned(),
            Some(ResolvedMeaning::Static(Meaning::EndV)) => "\\endtemplate".to_owned(),
            Some(ResolvedMeaning::Static(Meaning::CharGiven(ch))) => format!("the character {ch}"),
            Some(ResolvedMeaning::Static(Meaning::CharToken {
                ch,
                cat: Catcode::Letter,
            })) => format!("the letter {ch}"),
            Some(ResolvedMeaning::Static(Meaning::CharToken { ch, .. })) => {
                format!("the character {ch}")
            }
            Some(ResolvedMeaning::Static(Meaning::MathCharGiven(value))) => {
                format!("\\mathchar\"{value:X}")
            }
            Some(ResolvedMeaning::Static(Meaning::CountRegister(index))) => {
                format!("\\count{index}")
            }
            Some(ResolvedMeaning::Static(Meaning::DimenRegister(index))) => {
                format!("\\dimen{index}")
            }
            Some(ResolvedMeaning::Static(Meaning::SkipRegister(index))) => format!("\\skip{index}"),
            Some(ResolvedMeaning::Static(Meaning::MuskipRegister(index))) => {
                format!("\\muskip{index}")
            }
            Some(ResolvedMeaning::Static(Meaning::ToksRegister(index))) => format!("\\toks{index}"),
            Some(ResolvedMeaning::Static(
                Meaning::IntParam(_)
                | Meaning::InternalInteger(_)
                | Meaning::DimenParam(_)
                | Meaning::GlueParam(_)
                | Meaning::MuGlueParam(_)
                | Meaning::TokParam(_)
                | Meaning::PageDimension(_)
                | Meaning::PageInteger(_),
            )) => format!("\\{}", stores.display_symbol_name(symbol).unwrap_or("")),
            Some(ResolvedMeaning::Static(Meaning::Font(font))) => {
                format!("select font {}", stores.display_font_name(font))
            }
            Some(ResolvedMeaning::Static(meaning @ Meaning::ExpandablePrimitive(_))) => format!(
                "\\{}",
                stores
                    .display_primitive_name(meaning)
                    .or_else(|| stores.display_symbol_name(symbol))
                    .unwrap_or("")
            ),
            Some(ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Radical,
            ))) => "\\radical".to_owned(),
            Some(ResolvedMeaning::Static(meaning @ Meaning::UnexpandablePrimitive(_))) => format!(
                "\\{}",
                stores
                    .display_primitive_name(meaning)
                    .or_else(|| stores.display_symbol_name(symbol))
                    .unwrap_or("")
            ),
            Some(ResolvedMeaning::Macro { flags, definition }) => {
                let (parameter, replacement) = stores.display_macro_words(definition);
                let mut text = macro_prefix(flags);
                text.push_str("macro:");
                append_token_list(stores, &parameter, &mut text);
                text.push_str("->");
                append_token_list(stores, &replacement, &mut text);
                text
            }
            Some(ResolvedMeaning::Static(Meaning::Unknown(_))) => "unknown".to_owned(),
        },
    }
}

/// TeX82 §252's `show_eqtb` meaning text, bounded like `show_token_list`.
#[must_use]
pub fn bounded_meaning_text<G>(stores: &Universe<G>, token: Token, breadth: usize) -> String {
    bounded_meaning_text_with(stores, token, breadth)
}

pub(crate) fn bounded_meaning_text_with<G, S: MeaningDisplayState<G> + ?Sized>(
    stores: &S,
    token: Token,
    breadth: usize,
) -> String {
    let Token::Cs(symbol) = token else {
        return meaning_text_with(stores, token);
    };
    let Some(ResolvedMeaning::Macro { flags, definition }) = stores.display_meaning(symbol) else {
        return meaning_text_with(stores, token);
    };
    let (parameter, replacement) = stores.display_macro_words(definition);
    let mut text = macro_prefix(flags);
    text.push_str("macro:");
    let mut shown = 0;
    let mut tally = 0;
    while shown < parameter.len() && tally < breadth {
        let before = text.chars().count();
        append_token_show_text(stores, parameter[shown].semantic_token(), &mut text);
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
                append_token_show_text(stores, replacement[shown].semantic_token(), &mut text);
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

fn append_token_list<S: TokenDisplayState + ?Sized>(
    stores: &S,
    tokens: &[TokenWord],
    text: &mut String,
) {
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].semantic_token();
        if let Token::Char {
            ch,
            cat: Catcode::Parameter,
        } = token
            && let Some(Token::Param(slot)) =
                tokens.get(index + 1).map(|word| word.semantic_token())
        {
            append_tex_print_char(ch, text);
            text.push(char::from(b'0' + slot));
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
pub fn append_token_show_text<S: TokenDisplayState + ?Sized>(
    stores: &S,
    token: Token,
    text: &mut String,
) {
    if let Token::Char { ch, cat } = token {
        append_tex_print_char(ch, text);
        if cat == Catcode::Parameter {
            append_tex_print_char(ch, text);
        }
    } else {
        append_non_character_token_text(stores, token, text);
    }
    let name = match token {
        Token::Cs(symbol) => {
            if stores.display_control_sequence_kind(symbol)
                == Some(ControlSequenceKind::ActiveCharacter)
            {
                return;
            }
            stores.display_resolve(symbol).unwrap_or("")
        }
        Token::Frozen(_) => stores
            .display_frozen_primitive_name(token)
            .unwrap_or("endtemplate"),
        Token::Char { .. } | Token::Param(_) => return,
    };
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if stores.display_catcode(ch) != Catcode::Letter => {}
        _ => text.push(' '),
    }
}

trait TokenTextSink {
    fn push(&mut self, ch: char);
    fn push_str(&mut self, text: &str);
}

impl TokenTextSink for String {
    fn push(&mut self, ch: char) {
        String::push(self, ch);
    }

    fn push_str(&mut self, text: &str) {
        String::push_str(self, text);
    }
}

struct CallbackTextSink<'a, F> {
    visit: &'a mut F,
}

impl<F: FnMut(char)> TokenTextSink for CallbackTextSink<'_, F> {
    fn push(&mut self, ch: char) {
        (self.visit)(ch);
    }

    fn push_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.push(ch);
        }
    }
}

struct SelectorTextSink<'a> {
    text: &'a mut String,
    newlinechar: Option<char>,
}

impl TokenTextSink for SelectorTextSink<'_> {
    fn push(&mut self, ch: char) {
        if Some(ch) == self.newlinechar {
            self.text.push('\n');
        } else {
            append_tex_print_char(ch, self.text);
        }
    }

    fn push_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.push(ch);
        }
    }
}

fn append_non_character_token_text<S: TokenDisplayState + ?Sized, T: TokenTextSink>(
    stores: &S,
    token: Token,
    text: &mut T,
) {
    match token {
        Token::Cs(symbol) => {
            let name = stores.display_resolve(symbol).unwrap_or("");
            let escape = escapechar(stores);
            match stores.display_control_sequence_kind(symbol) {
                Some(ControlSequenceKind::ActiveCharacter) => text.push_str(name),
                Some(ControlSequenceKind::Null) => {
                    if let Some(escape) = escape {
                        text.push(escape);
                    }
                    text.push_str("csname");
                    if let Some(escape) = escape {
                        text.push(escape);
                    }
                    text.push_str("endcsname");
                }
                Some(
                    ControlSequenceKind::SingleCharacter
                    | ControlSequenceKind::Named
                    | ControlSequenceKind::Internal,
                ) => {
                    if let Some(escape) = escape {
                        text.push(escape);
                    }
                    text.push_str(name);
                }
                None => {}
            }
        }
        Token::Param(slot) => {
            text.push('#');
            text.push(char::from(b'0' + slot));
        }
        Token::Frozen(_) => {
            if let Some(escape) = escapechar(stores) {
                text.push(escape);
            }
            text.push_str(
                stores
                    .display_frozen_primitive_name(token)
                    .unwrap_or("endtemplate"),
            );
        }
        Token::Char { .. } => unreachable!("character tokens are handled by the caller"),
    }
}

/// Appends the token text TeX builds with `selector = new_string`.
///
/// Unlike ordinary diagnostic display, character tokens remain raw; control
/// sequence spelling and its separator still follow `show_token_list`.
pub fn append_token_string_text<S: TokenDisplayState + ?Sized>(
    stores: &S,
    token: Token,
    text: &mut String,
) {
    if let Token::Char { ch, cat } = token {
        text.push(ch);
        if cat == Catcode::Parameter {
            text.push(ch);
        }
    } else {
        // §262's `token_show` reaches `print_cs` while §293's caller has
        // selected `new_string`. Section 59 makes every character of an
        // internal string take the raw `print_char` path, including the
        // one-byte name of an active character. Applying the external
        // selector spelling here would prematurely turn such a byte into
        // `^^xx`; §1370 must instead print the completed temporary string
        // through the write-file selector.
        append_non_character_token_text(stores, token, text);
        let name = match token {
            Token::Cs(symbol) => {
                if stores.display_control_sequence_kind(symbol)
                    == Some(ControlSequenceKind::ActiveCharacter)
                {
                    return;
                }
                stores.display_resolve(symbol).unwrap_or("")
            }
            Token::Frozen(_) => stores
                .display_frozen_primitive_name(token)
                .unwrap_or("endtemplate"),
            Token::Char { .. } | Token::Param(_) => return,
        };
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) if stores.display_catcode(ch) != Catcode::Letter => {}
            _ => text.push(' '),
        }
    }
}

/// Visits the characters TeX82's `token_show` emits for one token selected by
/// `new_string`, without allocating an intermediate `String`. Conversion
/// collectors use this form when their destination is a fixed-chunk token
/// sink and can therefore keep the per-token hot path allocation-free.
pub fn for_each_token_string_char<S: TokenDisplayState + ?Sized, F: FnMut(char)>(
    stores: &S,
    token: Token,
    mut visit: F,
) {
    let mut sink = CallbackTextSink { visit: &mut visit };
    if let Token::Char { ch, cat } = token {
        sink.push(ch);
        if cat == Catcode::Parameter {
            sink.push(ch);
        }
    } else {
        append_non_character_token_text(stores, token, &mut sink);
        let name = match token {
            Token::Cs(symbol) => {
                if stores.display_control_sequence_kind(symbol)
                    == Some(ControlSequenceKind::ActiveCharacter)
                {
                    return;
                }
                stores.display_resolve(symbol).unwrap_or("")
            }
            Token::Frozen(_) => stores
                .display_frozen_primitive_name(token)
                .unwrap_or("endtemplate"),
            Token::Char { .. } | Token::Param(_) => return,
        };
        let mut chars = name.chars();
        if !matches!(
            (chars.next(), chars.next()),
            (Some(ch), None) if stores.display_catcode(ch) != Catcode::Letter
        ) {
            sink.push(' ');
        }
    }
}

/// Appends one token as TeX82's `show_token_list` prints it through an active
/// output selector.
///
/// Section 262 sends every character through `print`. Section 59's `print`
/// recognizes the live new-line character before expanding any other
/// non-printable byte to its canonical `^^` spelling.
pub fn append_token_selector_text<S: TokenDisplayState + ?Sized>(
    stores: &S,
    token: Token,
    newlinechar: Option<char>,
    text: &mut String,
) {
    let mut sink = SelectorTextSink { text, newlinechar };
    if let Token::Char { ch, cat } = token {
        sink.push(ch);
        if cat == Catcode::Parameter {
            sink.push(ch);
        }
    } else {
        append_non_character_token_text(stores, token, &mut sink);
        let name = match token {
            Token::Cs(symbol) => {
                if stores.display_control_sequence_kind(symbol)
                    == Some(ControlSequenceKind::ActiveCharacter)
                {
                    return;
                }
                stores.display_resolve(symbol).unwrap_or("")
            }
            Token::Frozen(_) => stores
                .display_frozen_primitive_name(token)
                .unwrap_or("endtemplate"),
            Token::Char { .. } | Token::Param(_) => return,
        };
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) if stores.display_catcode(ch) != Catcode::Letter => {}
            _ => sink.push(' '),
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
pub fn token_text<S: TokenDisplayState + ?Sized>(stores: &S, token: Token) -> String {
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
pub fn string_tokens<S: TokenDisplayState + ?Sized>(stores: &S, token: Token) -> Vec<Token> {
    match token {
        Token::Char { ch, .. } => vec![rendered_char(ch)],
        Token::Cs(symbol) => {
            let name = stores.display_resolve(symbol).unwrap_or("");
            let escape = escapechar(stores);
            let kind = stores.display_control_sequence_kind(symbol);
            let capacity = match kind {
                Some(ControlSequenceKind::ActiveCharacter) => name.chars().count(),
                Some(ControlSequenceKind::Null) => {
                    "csname".len() + "endcsname".len() + 2 * usize::from(escape.is_some())
                }
                Some(
                    ControlSequenceKind::SingleCharacter
                    | ControlSequenceKind::Named
                    | ControlSequenceKind::Internal,
                ) => name.chars().count() + usize::from(escape.is_some()),
                None => 0,
            };
            let mut out = Vec::with_capacity(capacity);
            match kind {
                Some(ControlSequenceKind::ActiveCharacter) => {
                    out.extend(name.chars().map(rendered_char));
                }
                Some(ControlSequenceKind::Null) => {
                    append_escaped_text(escape, "csname", &mut out);
                    append_escaped_text(escape, "endcsname", &mut out);
                }
                Some(
                    ControlSequenceKind::SingleCharacter
                    | ControlSequenceKind::Named
                    | ControlSequenceKind::Internal,
                ) => {
                    append_escaped_text(escape, name, &mut out);
                }
                None => {}
            }
            out
        }
        Token::Param(slot) => text_tokens(&format!("#{slot}")),
        Token::Frozen(_) => {
            let name = stores
                .display_frozen_primitive_name(token)
                .unwrap_or("endtemplate");
            let mut out = Vec::with_capacity(name.len() + 1);
            append_escaped_text(escapechar(stores), name, &mut out);
            out
        }
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

fn escapechar<S: TokenDisplayState + ?Sized>(stores: &S) -> Option<char> {
    u32::try_from(stores.display_escape_char())
        .ok()
        .filter(|&value| value < 256)
        .and_then(char::from_u32)
}
