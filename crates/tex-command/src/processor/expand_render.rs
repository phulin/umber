//! TeX command and conversion rendering.

use std::fmt::Write as _;

use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::interner::ControlSequenceKind;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning};
use tex_state::page::PageMark;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

use crate::CurrentCommand;

pub(super) fn format_scaled(value: Scaled) -> String {
    let mut output = String::new();
    append_format_scaled(value, &mut output);
    output
}

fn append_format_scaled(value: Scaled, output: &mut String) {
    let mut raw = i64::from(value.raw());
    if raw < 0 {
        output.push('-');
        raw = -raw;
    }
    let unity = i64::from(Scaled::UNITY);
    write!(output, "{}", raw / unity).expect("writing to String cannot fail");
    output.push('.');
    let mut scaled = 10 * (raw % unity) + 5;
    let mut delta = 10;
    loop {
        if delta > unity {
            scaled += 0o100000 - 50_000;
        }
        output.push(char::from(
            b'0' + u8::try_from(scaled / unity).expect("scaled digit fits u8"),
        ));
        scaled = 10 * (scaled % unity);
        delta *= 10;
        if scaled <= delta {
            break;
        }
    }
    output.push_str("pt");
}

fn format_glue(value: GlueSpec, unit: &str) -> String {
    let mut output = String::new();
    append_format_glue(value, unit, &mut output);
    output
}

fn append_format_glue(value: GlueSpec, unit: &str, output: &mut String) {
    append_scaled_with_unit(value.width, unit, output);
    for (label, component, order) in [
        (" plus ", value.stretch, value.stretch_order),
        (" minus ", value.shrink, value.shrink_order),
    ] {
        if component.raw() == 0 {
            continue;
        }
        output.push_str(label);
        append_scaled_without_unit(component, output);
        output.push_str(match order {
            Order::Normal => unit,
            Order::Fil => "fil",
            Order::Fill => "fill",
            Order::Filll => "filll",
        });
    }
}

pub(super) fn append_scaled_without_unit(value: Scaled, output: &mut String) {
    let start = output.len();
    append_format_scaled(value, output);
    output.truncate(output.len() - "pt".len());
    debug_assert!(output.len() >= start);
}

fn append_scaled_with_unit(value: Scaled, unit: &str, output: &mut String) {
    append_scaled_without_unit(value, output);
    output.push_str(unit);
}

pub(crate) fn render_the_value(value: &crate::InternalValue) -> Option<String> {
    match value {
        crate::InternalValue::Integer(value) => Some(value.to_string()),
        crate::InternalValue::Dimension(value) => Some(format_scaled(*value)),
        crate::InternalValue::Glue(value) => Some(format_glue(*value, "pt")),
        crate::InternalValue::MuGlue(value) => Some(format_glue(*value, "mu")),
        crate::InternalValue::Font(_) => None,
        crate::InternalValue::Tokens { .. } => None,
    }
}

pub(super) fn page_mark(primitive: ExpandablePrimitive) -> PageMark {
    match primitive {
        ExpandablePrimitive::TopMark | ExpandablePrimitive::TopMarks => PageMark::Top,
        ExpandablePrimitive::FirstMark | ExpandablePrimitive::FirstMarks => PageMark::First,
        ExpandablePrimitive::BotMark | ExpandablePrimitive::BotMarks => PageMark::Bot,
        ExpandablePrimitive::SplitFirstMark | ExpandablePrimitive::SplitFirstMarks => {
            PageMark::SplitFirst
        }
        ExpandablePrimitive::SplitBotMark | ExpandablePrimitive::SplitBotMarks => {
            PageMark::SplitBot
        }
        _ => unreachable!("only mark primitives reach page_mark"),
    }
}

pub(crate) fn string_text<G>(state: &tex_state::CommandContext<'_, G>, token: Token) -> String {
    let mut text = String::new();
    append_string_text(state, token, &mut text);
    text
}

pub(crate) fn append_string_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    token: Token,
    text: &mut String,
) {
    match token {
        Token::Cs(symbol) => {
            let escape = state.untracked_int_param(IntParam::ESCAPE_CHAR);
            if let Some(ch) = char::from_u32(u32::try_from(escape).unwrap_or(u32::MAX)) {
                text.push(ch);
            }
            text.push_str(state.resolve(symbol));
        }
        Token::Char { ch, .. } => text.push(ch),
        Token::Param(slot) => write!(text, "#{slot}").expect("writing to String cannot fail"),
        Token::Frozen(_) => text.push_str("\\relax"),
    }
}

/// TeX82 §262's `print_cs`, including its delimiter after a control word.
///
/// This is distinct from §263's `sprint_cs` spelling used by `\show` before
/// `=` and from §213's `\string`: named control words and `null_cs` append a
/// space, while active characters and single nonletter control symbols do not.
pub(crate) fn print_cs_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    symbol: tex_state::interner::Symbol,
) -> String {
    let mut text = String::new();
    append_print_cs_text(state, symbol, &mut text);
    text
}

pub(crate) fn append_print_cs_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    symbol: tex_state::interner::Symbol,
    text: &mut String,
) {
    let name = state.resolve(symbol);
    match state.control_sequence_kind(symbol) {
        ControlSequenceKind::ActiveCharacter => {
            text.push_str(name);
            return;
        }
        ControlSequenceKind::Null => {
            append_print_esc_text(state, "csname", text);
            append_print_esc_text(state, "endcsname", text);
            text.push(' ');
            return;
        }
        ControlSequenceKind::SingleCharacter
        | ControlSequenceKind::Named
        | ControlSequenceKind::Internal => {}
    }

    append_string_text(state, Token::Cs(symbol), text);
    let mut characters = name.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) if state.catcode(character) != Catcode::Letter => {}
        _ => text.push(' '),
    }
}

pub(crate) fn meaning_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    command: &CurrentCommand<G>,
) -> String {
    let mut text = String::new();
    append_meaning_text_with_token_selector(state, command, false, &mut text);
    text
}

/// TeX82 §§59, 262, and 296's `print_meaning` through an active selector.
///
/// `\meaning` builds a string, but `\show` prints a macro or mark token list
/// directly. Character tokens in the latter path therefore observe the live
/// `\newlinechar` instead of always using their context-free `^^` spelling.
pub(crate) fn selector_meaning_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    command: &CurrentCommand<G>,
) -> String {
    let mut text = String::new();
    append_meaning_text_with_token_selector(state, command, true, &mut text);
    text
}

fn append_meaning_text_with_token_selector<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    command: &CurrentCommand<G>,
    active_selector: bool,
    text: &mut String,
) {
    if let ResolvedMeaning::Macro { flags, definition } = command.meaning() {
        let macro_meaning = state.definition(definition);
        if flags.contains(MeaningFlags::PROTECTED) {
            append_print_esc_text(state, "protected", text);
        }
        if flags.contains(MeaningFlags::LONG) {
            append_print_esc_text(state, "long", text);
        }
        if flags.contains(MeaningFlags::OUTER) {
            append_print_esc_text(state, "outer", text);
        }
        if flags.bits()
            & (MeaningFlags::PROTECTED | MeaningFlags::LONG | MeaningFlags::OUTER).bits()
            != 0
        {
            text.push(' ');
        }
        text.push_str("macro:");
        append_meaning_token_words(
            state,
            macro_meaning.parameter_text().iter(),
            active_selector,
            text,
        );
        text.push_str("->");
        append_meaning_token_words(
            state,
            macro_meaning.replacement_text().iter(),
            active_selector,
            text,
        );
        return;
    }
    let ResolvedMeaning::Static(meaning) = command.meaning() else {
        unreachable!("macro meanings returned above")
    };
    match meaning {
        Meaning::Undefined => text.push_str("undefined"),
        Meaning::Relax => append_print_esc_text(state, "relax", text),
        Meaning::CharToken { ch, cat } => append_character_command_text(ch, cat, text),
        Meaning::CharGiven(ch) => {
            append_print_esc_text(state, "char", text);
            write!(text, "\"{:X}", u32::from(ch)).expect("writing to String cannot fail");
        }
        Meaning::MathCharGiven(value) => {
            write!(text, "\\mathchar\"{value:X}").expect("writing to String cannot fail");
        }
        Meaning::CountRegister(index) => {
            write!(text, "\\count{index}").expect("writing to String cannot fail");
        }
        Meaning::DimenRegister(index) => {
            write!(text, "\\dimen{index}").expect("writing to String cannot fail");
        }
        Meaning::SkipRegister(index) => {
            write!(text, "\\skip{index}").expect("writing to String cannot fail");
        }
        Meaning::MuskipRegister(index) => {
            write!(text, "\\muskip{index}").expect("writing to String cannot fail");
        }
        Meaning::ToksRegister(index) => {
            write!(text, "\\toks{index}").expect("writing to String cannot fail");
        }
        meaning @ (Meaning::IntParam(_)
        | Meaning::InternalInteger(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)) => {
            append_meaning_control_sequence_text(state, command, meaning, text);
        }
        Meaning::Font(font) => {
            text.push_str("select font ");
            text.push_str(state.font_external_name(font));
            let size = state.font_size(font);
            if size != state.font_design_size(font) {
                text.push_str(" at ");
                append_scaled_without_unit(size, text);
            }
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate) => {
            append_print_esc_text(state, "outer", text);
            text.push_str(" endtemplate:");
        }
        Meaning::ExpandablePrimitive(
            primitive @ (ExpandablePrimitive::TopMark
            | ExpandablePrimitive::FirstMark
            | ExpandablePrimitive::BotMark
            | ExpandablePrimitive::SplitFirstMark
            | ExpandablePrimitive::SplitBotMark),
        ) => {
            append_meaning_control_sequence_text(
                state,
                command,
                Meaning::ExpandablePrimitive(primitive),
                text,
            );
            text.push(':');
            let tokens = state.page_mark(page_mark(primitive));
            append_meaning_token_words(
                state,
                state
                    .node_token_words(tokens)
                    .expect("page mark token key belongs to the admitted generation")
                    .iter()
                    .copied(),
                active_selector,
                text,
            );
        }
        meaning @ (Meaning::ExpandablePrimitive(_) | Meaning::UnexpandablePrimitive(_)) => {
            append_meaning_control_sequence_text(state, command, meaning, text);
        }
        Meaning::EndV => text.push_str("end of alignment template"),
        Meaning::Unknown(_) => text.push_str("unknown"),
    }
}

pub(crate) fn append_meaning_token_words<G>(
    state: &tex_state::CommandContext<'_, G>,
    tokens: impl IntoIterator<Item = tex_state::token::TokenWord>,
    active_selector: bool,
    text: &mut String,
) {
    let mut tokens = tokens.into_iter().peekable();
    while let Some(word) = tokens.next() {
        let token = word.token().expect("durable token word is valid");
        if let Token::Char {
            ch,
            cat: Catcode::Parameter,
        } = token
            && let Some(Token::Param(slot)) = tokens.peek().and_then(|word| word.token())
        {
            let raw = [ch, char::from(b'0' + slot)]
                .into_iter()
                .collect::<String>();
            if active_selector {
                state.append_selector_string_text(&raw, text);
            } else {
                text.push_str(&raw);
            }
            let _ = tokens.next();
            continue;
        }
        if active_selector {
            state.append_token_selector_text(token, text);
        } else {
            // §296 runs `print_meaning` with `selector=new_string` before
            // turning that string back into other-character tokens. Section
            // 59 therefore keeps every byte of a macro replacement raw here;
            // only the direct diagnostic (`active_selector`) path applies
            // printable `^^xx` spelling.
            state.append_token_string_text(token, text);
        }
    }
}

/// The copyable portion of a delivered command needed by TeX82 §298.
///
/// This is captured from `CurrentCommand<G>`, not reconstructed from `Meaning`,
/// so the delivered control-sequence identity remains available across the
/// executor's transactional scan/apply seam.
#[derive(Debug, Eq, PartialEq)]
pub struct PrintCommand<G> {
    meaning: ResolvedMeaning<G>,
    control_sequence: Option<tex_state::interner::Symbol>,
}

impl<G> PrintCommand<G> {
    #[must_use]
    pub fn from_current(command: &CurrentCommand<G>) -> Self {
        Self {
            meaning: command.meaning(),
            control_sequence: command.control_sequence(),
        }
    }

    #[must_use]
    pub(crate) fn meaning(&self) -> ResolvedMeaning<G> {
        self.meaning
    }
}

impl<G> Clone for PrintCommand<G> {
    fn clone(&self) -> Self {
        Self {
            meaning: self.meaning,
            control_sequence: self.control_sequence,
        }
    }
}

/// TeX82 §298's `print_cmd_chr` representation of one delivered command.
///
/// The input is the full ephemeral equivalent of `cur_cmd`, `cur_chr`, and
/// `cur_cs`, rather than a decoded `Meaning`. This keeps command-class
/// vocabulary independent of the token spelling: a control-sequence alias of
/// a primitive prints the primitive, while aliases of character commands keep
/// their character command class.
#[must_use]
pub fn print_cmd_chr_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    command: PrintCommand<G>,
) -> String {
    let mut text = String::new();
    append_print_cmd_chr_text(state, command, &mut text);
    text
}

/// Appends TeX82 §298's `print_cmd_chr` representation to caller-owned text.
pub fn append_print_cmd_chr_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    command: PrintCommand<G>,
    text: &mut String,
) {
    if let ResolvedMeaning::Macro { flags, .. } = command.meaning {
        if flags.contains(MeaningFlags::PROTECTED) {
            append_print_esc_text(state, "protected", text);
        }
        if flags.contains(MeaningFlags::LONG) {
            append_print_esc_text(state, "long", text);
        }
        if flags.contains(MeaningFlags::OUTER) {
            append_print_esc_text(state, "outer", text);
        }
        if flags.bits()
            & (MeaningFlags::PROTECTED | MeaningFlags::LONG | MeaningFlags::OUTER).bits()
            != 0
        {
            text.push(' ');
        }
        text.push_str("macro");
        return;
    }
    let ResolvedMeaning::Static(meaning) = command.meaning else {
        unreachable!("macro meanings returned above")
    };
    match meaning {
        Meaning::Undefined => text.push_str("undefined"),
        Meaning::Relax => append_print_esc_text(state, "relax", text),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate) => {
            append_print_esc_text(state, "outer", text);
            text.push_str(" endtemplate");
        }
        Meaning::CharToken { ch, cat } => append_character_command_text(ch, cat, text),
        Meaning::CharGiven(ch) => {
            append_print_esc_text(state, "char", text);
            write!(text, "\"{:X}", ch as u32).expect("writing to String cannot fail");
        }
        Meaning::MathCharGiven(value) => {
            append_print_esc_text(state, "mathchar", text);
            write!(text, "\"{value:X}").expect("writing to String cannot fail");
        }
        Meaning::CountRegister(index) => append_escaped_index(state, "count", index, text),
        Meaning::DimenRegister(index) => append_escaped_index(state, "dimen", index, text),
        Meaning::SkipRegister(index) => append_escaped_index(state, "skip", index, text),
        Meaning::MuskipRegister(index) => append_escaped_index(state, "muskip", index, text),
        Meaning::ToksRegister(index) => append_escaped_index(state, "toks", index, text),
        meaning @ (Meaning::IntParam(_)
        | Meaning::InternalInteger(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)
        | Meaning::ExpandablePrimitive(_)
        | Meaning::UnexpandablePrimitive(_)) => {
            append_print_command_control_sequence_text(state, command, meaning, text);
        }
        Meaning::Font(font) => {
            text.push_str("select font ");
            text.push_str(state.font_external_name(font));
            let size = state.font_size(font);
            if size != state.font_design_size(font) {
                text.push_str(" at ");
                append_scaled_without_unit(size, text);
                text.push_str("pt");
            }
        }
        Meaning::EndV => text.push_str("end of alignment template"),
        Meaning::Unknown(_) => text.push_str("[unknown command code!]"),
    }
}

fn append_escaped_index<G>(
    state: &tex_state::CommandContext<'_, G>,
    name: &str,
    index: u16,
    text: &mut String,
) {
    append_print_esc_text(state, name, text);
    write!(text, "{index}").expect("writing to String cannot fail");
}

fn append_print_command_control_sequence_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    command: PrintCommand<G>,
    meaning: Meaning,
    text: &mut String,
) {
    let name = state
        .primitive_name(meaning)
        .or_else(|| command.control_sequence.map(|symbol| state.resolve(symbol)));
    if let Some(name) = name {
        append_print_esc_text(state, name, text);
    } else {
        text.push_str("undefined");
    }
}

fn append_meaning_control_sequence_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    command: &CurrentCommand<G>,
    meaning: Meaning,
    text: &mut String,
) {
    let name = state.primitive_name(meaning).or_else(|| {
        command
            .control_sequence()
            .map(|symbol| state.resolve(symbol))
    });
    if let Some(name) = name {
        text.push('\\');
        text.push_str(name);
    } else {
        text.push_str("undefined");
    }
}

/// TeX82 §298's character-command cases used by `print_meaning`.
pub fn character_command_text(ch: char, cat: Catcode) -> String {
    let mut text = String::new();
    append_character_command_text(ch, cat, &mut text);
    text
}

/// Appends TeX82 §298's character-command representation.
pub fn append_character_command_text(ch: char, cat: Catcode, text: &mut String) {
    match cat {
        Catcode::BeginGroup => text.push_str("begin-group character "),
        Catcode::EndGroup => text.push_str("end-group character "),
        Catcode::MathShift => text.push_str("math shift character "),
        Catcode::AlignmentTab => text.push_str("alignment tab character "),
        Catcode::Parameter => text.push_str("macro parameter character "),
        Catcode::Superscript => text.push_str("superscript character "),
        Catcode::Subscript => text.push_str("subscript character "),
        Catcode::Space => {
            text.push_str("blank space  ");
            return;
        }
        Catcode::Letter => text.push_str("the letter "),
        Catcode::Other => text.push_str("the character "),
        // `get_next` maps a category-5 character to `car_ret` with its
        // character code as operand. It is therefore §298's non-`cr_code`
        // branch, whose vocabulary is `\crcr`.
        Catcode::EndLine => {
            text.push_str("\\crcr");
            return;
        }
        Catcode::Escape
        | Catcode::Ignored
        | Catcode::Active
        | Catcode::Comment
        | Catcode::Invalid => {
            text.push_str("[uncommandable character ");
            append_printable_character_text(ch, text);
            text.push(']');
            return;
        }
    }
    append_printable_character_text(ch, text);
}

/// TeX82 §§49/59's one-character string spelling used by §298.
///
/// Rendering happens before the completed diagnostic reaches its live output
/// selector, so generated caret notation must not be reinterpreted through
/// `\newlinechar` character by character.
fn append_printable_character_text(ch: char, text: &mut String) {
    tex_state::token_show::append_tex_print_char(ch, text);
}

/// TeX82 §63's `print_esc`: the current `\escapechar`, when it names a
/// character, followed by `name`.
///
/// §63 prints no escape at all when `\escapechar` is outside a character's
/// range, which is why the prefix is conditional rather than a hard-coded
/// backslash.
#[must_use]
pub fn print_esc_text<G>(state: &tex_state::CommandContext<'_, G>, name: &str) -> String {
    let mut text = String::with_capacity(name.len() + 1);
    append_print_esc_text(state, name, &mut text);
    text
}

/// Appends TeX82 §63's `print_esc` representation.
pub fn append_print_esc_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    name: &str,
    text: &mut String,
) {
    if let Ok(escape) = u8::try_from(state.untracked_int_param(IntParam::ESCAPE_CHAR)) {
        text.push(char::from(escape));
    }
    text.push_str(name);
}

/// TeX82 §298's `print_cmd_chr` representation for a delivered token.
///
/// Diagnostics use this same renderer as `\meaning`; consequently Rust enum
/// spellings cannot leak into ordinary terminal or transcript output.
#[must_use]
pub fn command_token_text<G>(state: &mut tex_state::CommandContext<'_, G>, token: Token) -> String {
    let mut text = String::new();
    append_command_token_text(state, token, &mut text);
    text
}

/// Appends TeX82 §298's representation for a delivered token.
pub fn append_command_token_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    token: Token,
    text: &mut String,
) {
    match token {
        Token::Char { ch, cat } => append_character_command_text(ch, cat, text),
        Token::Param(slot) => {
            write!(text, "macro parameter character #{slot}")
                .expect("writing to String cannot fail");
        }
        Token::Frozen(_) => text.push_str("end of alignment template"),
        Token::Cs(symbol) => {
            let meaning = state.meaning(symbol);
            let name = match meaning {
                ResolvedMeaning::Static(meaning) => state.primitive_name(meaning),
                ResolvedMeaning::Macro { .. } => None,
            }
            .unwrap_or_else(|| state.resolve(symbol));
            append_print_esc_text(state, name, text);
        }
    }
}

/// The string pdfTeX builds by selecting `new_string` around `show_token_list`.
///
/// Character tokens remain raw (with parameter characters doubled), while
/// control-sequence spelling and its separator observe the live escape
/// character and catcode table. The returned value owns no token-list handle,
/// so it remains stable when a typed resource continuation resumes the
/// enclosing command.
pub(crate) fn token_slice_string_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    tokens: &[Token],
) -> String {
    let mut text = String::new();
    let _ = state.int_param(IntParam::ESCAPE_CHAR);
    for &token in tokens {
        state.append_token_string_text(token, &mut text);
    }
    text
}

/// TeX82's `show_token_list` representation used by `\\meaning` distinguishes
/// a printed control word from following letter tokens with one space.  That
/// delimiter belongs to the rendered definition, not to source input.
pub(crate) fn token_list_token_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    token: Token,
) -> String {
    let mut text = String::new();
    append_token_list_token_text(state, token, &mut text);
    text
}

pub(crate) fn append_token_list_token_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    token: Token,
    text: &mut String,
) {
    let name = match token {
        Token::Cs(_) | Token::Char { .. } | Token::Param(_) => {
            state.append_token_show_text(token, text);
            return;
        }
        // tex.web gives every frozen equivalent a real eqtb `text()`, so §294
        // displays one exactly as it displays the ordinary control sequence of
        // the same name: `frozen_par` is `\par`, not its `\relax`-like
        // meaning.
        Token::Frozen(_) => match state.frozen_primitive_name(token) {
            Some(name) => name,
            None => {
                append_string_text(state, token, text);
                return;
            }
        },
    };
    // TeX82 §§63/294: `show_token_list` renders control sequences through
    // `print_cs`, and every escape prefix that `print_cs` emits comes from
    // the live `\escapechar`. This matters for backed-up recovery tokens:
    // §1064 inserts a closer ahead of the offending command, then §314
    // pseudoprints that command while the current integer parameters remain
    // in force.
    if name.is_empty() {
        append_print_esc_text(state, "csname", text);
        append_print_esc_text(state, "endcsname", text);
    } else {
        append_print_esc_text(state, name, text);
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) if state.untracked_catcode(character) != Catcode::Letter => {}
        _ => text.push(' '),
    }
}

pub(super) fn roman_numeral(value: i32) -> String {
    let mut output = String::new();
    append_roman_numeral(value, &mut output);
    output
}

fn append_roman_numeral(value: i32, output: &mut String) {
    if value <= 0 {
        return;
    }
    let mut remaining = value;
    for (amount, glyph) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while remaining >= amount {
            output.push_str(glyph);
            remaining -= amount;
        }
    }
}

pub(super) fn format_pdf_date(clock: tex_state::JobClock, utc_offset_minutes: i16) -> String {
    use std::fmt::Write as _;
    let mut date = format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}",
        clock.year,
        clock.month,
        clock.day,
        clock.time.div_euclid(60),
        clock.time.rem_euclid(60),
        clock.second,
    );
    if utc_offset_minutes == 0 {
        date.push('Z');
    } else {
        let sign = if utc_offset_minutes < 0 { '-' } else { '+' };
        let absolute = i32::from(utc_offset_minutes).abs();
        write!(date, "{sign}{:02}'{:02}'", absolute / 60, absolute % 60)
            .expect("writing to a String cannot fail");
    }
    date
}
