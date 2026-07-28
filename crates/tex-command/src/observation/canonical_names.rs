//! The complete canonical observation vocabulary, in one compile-time
//! exhaustive place.
//!
//! Every string an observation payload can carry for a *concept* -- a category
//! code, a character command, a scanner status, a glue order, a token's
//! catcode -- is spelled here and nowhere else. Each function is total over
//! its Rust domain, so a new `Catcode`, `Order`, or `ScannerStatus` variant is
//! a build failure (`error[E0004]`) rather than a silently misspelled event.
//!
//! This is `docs/tex_command_core.md` §33.2's dispatch-completeness invariant
//! applied to naming, and it is deliberately a *single entry point per family*
//! (`umber2-johp.134`'s `parameter_mutation_key`, generalized by
//! `umber2-johp.141`): a call site that formats a raw engine value into an
//! observation is the defect this module exists to make impossible. In
//! particular, a Rust `Debug` rendering must never reach an observation
//! payload -- `Debug` spells Umber's *Rust* variant names, and the oracle
//! spells tex.web's.
//!
//! Authority is tex.web, never Umber's own enum spellings:
//!
//! - §207 fixes the category codes and the command codes that share their
//!   numeric values. The two tables are distinct where a catcode cannot
//!   emerge from the scanning routine: catcode 5 is `car_ret` but command 5
//!   is `out_param`, catcode 9 is `ignore` but command 9 is `endv`, catcode 13
//!   is `active_char` but command 13 is `par_end`/`match`, catcode 14 is
//!   `comment` but command 14 is `end_match`/`stop`, and catcode 15 is
//!   `invalid_char` but command 15 is `delim_num`.
//! - §135 fixes the glue infinity orders `normal`, `fil`, `fill`, `filll`.
//! - §305 fixes the six `scanner_status` values.

use tex_state::glue::Order;
use tex_state::token::Catcode;

use super::ObservedToken;

/// tex.web §207's category-code name for one stored catcode.
///
/// This is the `\catcode` table's own vocabulary: the name that appears in a
/// token payload and as a `\catcode` mutation's value.
#[must_use]
pub fn catcode_name(catcode: Catcode) -> &'static str {
    match catcode {
        Catcode::Escape => "escape",
        Catcode::BeginGroup => "left_brace",
        Catcode::EndGroup => "right_brace",
        Catcode::MathShift => "math_shift",
        Catcode::AlignmentTab => "tab_mark",
        Catcode::EndLine => "car_ret",
        Catcode::Parameter => "mac_param",
        Catcode::Superscript => "sup_mark",
        Catcode::Subscript => "sub_mark",
        Catcode::Ignored => "ignore",
        Catcode::Space => "spacer",
        Catcode::Letter => "letter",
        Catcode::Other => "other_char",
        Catcode::Active => "active_char",
        Catcode::Comment => "comment",
        Catcode::Invalid => "invalid_char",
    }
}

/// tex.web §207's category-code name for a scanned `\catcode` assignment.
///
/// `\catcode` accepts any integer, so an out-of-range value has no name; the
/// caller reports the raw assignment instead of inventing one.
#[must_use]
pub fn catcode_assignment_name(value: i64) -> Option<&'static str> {
    let code = u8::try_from(value).ok()?;
    Some(catcode_name(match code {
        0 => Catcode::Escape,
        1 => Catcode::BeginGroup,
        2 => Catcode::EndGroup,
        3 => Catcode::MathShift,
        4 => Catcode::AlignmentTab,
        5 => Catcode::EndLine,
        6 => Catcode::Parameter,
        7 => Catcode::Superscript,
        8 => Catcode::Subscript,
        9 => Catcode::Ignored,
        10 => Catcode::Space,
        11 => Catcode::Letter,
        12 => Catcode::Other,
        13 => Catcode::Active,
        14 => Catcode::Comment,
        15 => Catcode::Invalid,
        _ => return None,
    }))
}

/// tex.web §207's *command* code name for a character command whose category
/// code is `catcode`.
///
/// A character command is `(cur_cmd, cur_chr)` after §341's `get_next`
/// classification, so only the catcodes that survive scanning appear. Escape,
/// ignore, comment, and invalid characters are consumed by §341 itself and an
/// active character is replaced by its meaning before any command is
/// delivered, so those five have no character command and say so rather than
/// borrowing a name from the catcode table -- command 0 is `relax`, not
/// `escape`, and command 13 is `par_end`, not `active_char`.
#[must_use]
pub fn character_command_name(catcode: Catcode) -> Option<&'static str> {
    Some(match catcode {
        Catcode::BeginGroup => "left_brace",
        Catcode::EndGroup => "right_brace",
        Catcode::MathShift => "math_shift",
        Catcode::AlignmentTab => "tab_mark",
        Catcode::EndLine => "car_ret",
        Catcode::Parameter => "mac_param",
        Catcode::Superscript => "sup_mark",
        Catcode::Subscript => "sub_mark",
        Catcode::Space => "spacer",
        Catcode::Letter => "letter",
        Catcode::Other => "other_char",
        Catcode::Escape
        | Catcode::Ignored
        | Catcode::Active
        | Catcode::Comment
        | Catcode::Invalid => return None,
    })
}

/// tex.web §135's infinity-order name for a glue component.
#[must_use]
pub fn glue_order_name(order: Order) -> &'static str {
    match order {
        Order::Normal => "normal",
        Order::Fil => "fil",
        Order::Fill => "fill",
        Order::Filll => "filll",
    }
}

/// The catcode name one observed token carries in an event payload.
///
/// §289's token representation is not the `\catcode` table: a macro's
/// parameter text stores `match` and `end_match` tokens and its replacement
/// text stores `out_param` tokens, none of which any character can have as a
/// category code. A control sequence is stored above `cs_token_flag` and is
/// reported under `escape` with its spelling attached.
#[must_use]
pub fn observed_token_catcode(token: &ObservedToken) -> &'static str {
    match token {
        ObservedToken::Character { catcode, .. } => catcode_name(*catcode),
        ObservedToken::ControlSequence(_)
        | ObservedToken::FrozenEndTemplate
        | ObservedToken::FrozenEndV
        | ObservedToken::FrozenPrimitive(_)
        | ObservedToken::FrozenOther => "escape",
        ObservedToken::MacroMatch => "match",
        ObservedToken::MacroEndMatch => "end_match",
        // The oracle spells §289's `out_param` in full rather than by its
        // abbreviated tex.web macro name.
        ObservedToken::Parameter(_) => "out_parameter",
    }
}

/// The control-sequence spelling one observed token carries, when it has one.
///
/// Umber's frozen sentinels are TeX's frozen control sequences, whose `text`
/// tex.web assigns explicitly: §258 gives `frozen_dont_expand` the spelling
/// `notexpanded:`, §780 gives `frozen_end_template` and `frozen_endv` the
/// single shared spelling `endtemplate`, and §1216's `\outer` protection uses
/// `inaccessible`. Only Umber's own expanded-text boundary has no TeX
/// counterpart, and it deliberately gets a name no engine installs so that a
/// spelling reaching a trace under it looks like the internal-sentinel leak it
/// is, rather than being folded into a real control sequence.
#[must_use]
pub fn observed_token_control_sequence(token: &ObservedToken) -> Option<&str> {
    match token {
        ObservedToken::ControlSequence(name) => Some(name),
        ObservedToken::FrozenEndTemplate | ObservedToken::FrozenEndV => Some("endtemplate"),
        ObservedToken::FrozenPrimitive(name) => Some(name),
        ObservedToken::FrozenOther => Some("umber_internal_sentinel"),
        ObservedToken::Character { .. }
        | ObservedToken::MacroMatch
        | ObservedToken::MacroEndMatch
        | ObservedToken::Parameter(_) => None,
    }
}

/// The character code one observed token carries in an event payload.
#[must_use]
pub fn observed_token_character(token: &ObservedToken) -> u32 {
    match token {
        ObservedToken::Character { character, .. } => u32::from(*character),
        // §294 stores a match token as `match*256 + the delimiter character`,
        // which `scan_toks` always sets to the macro parameter character.
        ObservedToken::MacroMatch => u32::from('#'),
        ObservedToken::Parameter(slot) => u32::from(*slot),
        ObservedToken::ControlSequence(_)
        | ObservedToken::MacroEndMatch
        | ObservedToken::FrozenEndTemplate
        | ObservedToken::FrozenEndV
        | ObservedToken::FrozenPrimitive(_)
        | ObservedToken::FrozenOther => 0,
    }
}

/// tex.web §207/§208's command name for one stored meaning.
///
/// This is the same total classification raw command delivery uses, exposed
/// for the observers that must name a meaning they did not deliver -- §1221's
/// `\let`, whose committed value is the copied meaning's `eq_type`, not the
/// spelling of the control sequence it was copied from.
#[must_use]
pub fn meaning_command_name(meaning: tex_state::meaning::Meaning) -> String {
    super::canonical_command_identity(meaning).0
}

/// tex.web §307's `token_type` name for one input level.
///
/// §307 gives sixteen `token_type` codes, and the reference instrumentation's
/// `umber_trace_input_name` spells exactly those sixteen. A source level has
/// no `token_type` -- TeX names it by the file it is reading -- so it reports
/// `None` and the transport supplies the active source's identity.
///
/// A level Umber owns that tex.web reads live has no code to report, so it is
/// named under an `umber` marker rather than borrowing a neighbouring one, on
/// the same reasoning that makes `parameter_mutation_key` write `umber<slot>`
/// instead of a bare number.
#[must_use]
pub fn input_level_name(reason: super::InputReason) -> Option<&'static str> {
    use super::{InputReason, UmberReplayKind};
    Some(match reason {
        InputReason::Source => return None,
        InputReason::Parameter => "parameter",
        InputReason::AlignmentUTemplate => "u_template",
        InputReason::AlignmentVTemplate => "v_template",
        InputReason::Backup => "backup",
        InputReason::Recovery => "recovery",
        InputReason::Macro => "macro",
        InputReason::OutputRoutine => "output",
        InputReason::EveryPar => "every_par",
        InputReason::EveryMath => "every_math",
        InputReason::EveryDisplay => "every_display",
        InputReason::EveryHBox => "every_hbox",
        InputReason::EveryVBox => "every_vbox",
        InputReason::EveryJob => "every_job",
        InputReason::EveryCr => "every_cr",
        InputReason::Mark => "mark",
        InputReason::Write => "write",
        InputReason::UmberReplay(UmberReplayKind::Discretionary) => "umber:discretionary",
    })
}

/// tex.web §303's `name` classification for one source input level.
///
/// [`input_level_name`] returns `None` for a source level because §307's
/// `token_type` does not classify one. This is the classification that does:
/// §303 splits `name` into the terminal (`0`), input stream `name-1`
/// (`1..=17`), and a text file (`>17`), and §329's `end_file_reading` closes
/// a handle for the last of those alone.
///
/// The stream number stays on [`SourceNameClass::ReadStream`] rather than in
/// the name, so that every source level in a trace is named by its channel
/// and never by a value a fixture would have to spell one variant at a time.
#[must_use]
pub fn source_name_class_name(class: crate::SourceNameClass) -> &'static str {
    use crate::SourceNameClass;
    match class {
        SourceNameClass::Terminal => "terminal",
        SourceNameClass::ReadStream(_) => "read_stream",
        SourceNameClass::File => "file",
    }
}

/// tex.web §305's `scanner_status` name.
///
/// The canonical vocabulary is the six documented values, not Umber's Rust
/// variant spellings, and never a `Debug` rendering of the context each
/// variant carries.
#[must_use]
pub(crate) fn scanner_status_name(status: &crate::processor::ScannerStatus) -> &'static str {
    use crate::processor::ScannerStatus;
    match status {
        ScannerStatus::Normal => "normal",
        ScannerStatus::Skipping(_) => "skipping",
        ScannerStatus::Defining(_) => "defining",
        ScannerStatus::Matching(_) => "matching",
        ScannerStatus::Aligning(_) => "aligning",
        ScannerStatus::Absorbing(_) => "absorbing",
    }
}

#[cfg(test)]
mod tests;
