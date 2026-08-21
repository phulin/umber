//! e-TeX 2.6's `\tracingassigns` rendered assignment trace.
//!
//! `etex.ch` [17.687-750] intercepts TeX82's four generic eqtb-write
//! primitives -- `eq_define`/`eq_word_define` (local) and
//! `geq_define`/`geq_word_define` (global) -- and routes each write through
//! `assign_trace`, which is `restore_trace` (the same renderer
//! `\tracingrestores` uses at `unsave`) under a different label. Because
//! those four WEB routines are the *only* place a scoped eqtb cell is
//! written, hooking them covers every register and parameter family at once.
//!
//! Umber has no single low-level writer with that reach: the legacy assignment
//! front
//! centralizes the scalar register/parameter families in
//! `execute_assignment_to_target`, and code-table writes centralize
//! similarly in `execute_code_table_assignment`, so this module is called
//! from both. Box-register `\setbox` writes join that path at `box_end`;
//! font-selection (`\font`, `\textfont`, ...) assignments are also
//! eqtb-resident in real TeX and are not yet traced here; see
//! `docs/etex_primitives.md`.
//!
//! `\tracingrestores`'s own `unsave`-time "restoring"/"retaining" lines are a
//! distinct TeX82 (not e-TeX) primitive. They are produced at Universe's
//! group-exit boundary, where ordered old-value records remain available;
//! this module renders only the four `\tracingassigns` labels.

use tex_state::env::banks::IntParam;
use tex_state::glue::GlueSpec;
use tex_state::interner::{ControlSequenceKind, Symbol};
use tex_state::meaning::{Meaning, MeaningFlags, ResolvedMeaning};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::token_show::TokenDisplayState;
use tex_state::{
    CommandContext, GroupRestorationCell, GroupRestorationOutcome, GroupRestorationReceipt,
    GroupRestorationValue, PenaltyArrayKind, TokenListId,
};

use super::primitives::{dimen_param_name, glue_param_name, int_param_name, tok_param_name};
use crate::node_dump::{format_glue_with_unit, format_scaled_for_diagnostics};

/// Renders one primitive/register name through the live `\escapechar`,
/// matching e-TeX 2.6's own escape-aware `print_esc`-based rendering.
fn escaped<G>(stores: &mut CommandContext<'_, G>, name: &str) -> String {
    tex_command::print_esc_text(stores, name)
}

/// Prints `{label name=value}` unconditionally: the caller has already
/// decided `\tracingassigns` was positive at the moment that mattered.
fn print_trace<G>(stores: &mut CommandContext<'_, G>, label: &str, name: &str, value: &str) {
    let mut diagnostic = stores.begin_diagnostic();
    diagnostic
        .print_char('{')
        .print(label)
        .print_char(' ')
        .print(name)
        .print_char('=')
        .print(value)
        .print_char('}');
    diagnostic.end(false);
}

struct RestorationTokenDisplay<'a, 'state, G> {
    stores: &'a CommandContext<'state, G>,
    escape_char: i32,
}

impl<G> TokenDisplayState for RestorationTokenDisplay<'_, '_, G> {
    fn display_resolve(&self, symbol: Symbol) -> Option<&str> {
        Some(self.stores.resolve(symbol))
    }

    fn display_control_sequence_kind(&self, symbol: Symbol) -> Option<ControlSequenceKind> {
        Some(self.stores.control_sequence_kind(symbol))
    }

    fn display_catcode(&self, ch: char) -> Catcode {
        self.stores.catcode(ch)
    }

    fn display_frozen_primitive_name(&self, token: Token) -> Option<&str> {
        self.stores.frozen_primitive_name(token)
    }

    fn display_escape_char(&self) -> i32 {
        self.escape_char
    }
}

fn escaped_at(escape_char: i32, name: &str) -> String {
    let mut text = String::with_capacity(name.len() + 1);
    if let Ok(escape) = u8::try_from(escape_char) {
        text.push(char::from(escape));
    }
    text.push_str(name);
    text
}

fn restoration_token_text<G>(
    stores: &CommandContext<'_, G>,
    escape_char: i32,
    token: Token,
) -> String {
    let display = RestorationTokenDisplay {
        stores,
        escape_char,
    };
    let mut text = String::new();
    tex_state::token_show::append_token_show_text(&display, token, &mut text);
    text
}

/// Renders e-TeX [19.282--283]'s `restore_trace` values after state mutation
/// and before §282 replays any `\aftergroup` token.
pub(crate) fn trace_group_restorations<G>(
    stores: &mut CommandContext<'_, G>,
    receipt: &GroupRestorationReceipt<G>,
) {
    for entry in receipt.entries() {
        let trace = entry.trace_state();
        if trace.tracing_restores() <= 0 {
            continue;
        }
        let Some((name, value, box_value)) = restoration_text(
            stores,
            entry.cell(),
            entry.live_value(),
            trace.escape_char(),
        ) else {
            continue;
        };
        let label = match entry.outcome() {
            GroupRestorationOutcome::Restored => "restoring",
            GroupRestorationOutcome::Retained => "retaining",
        };
        let mut diagnostic = stores.begin_group_restoration_diagnostic(trace);
        diagnostic
            .print_char('{')
            .print(label)
            .print_char(' ')
            .print(&name)
            .print_char('=');
        if box_value && value != "void" {
            diagnostic.print_ln().print_rendered(&value);
        } else {
            diagnostic.print_rendered(&value);
        }
        diagnostic.print_char('}');
        diagnostic.end(false);
    }
}

fn restoration_text<G>(
    stores: &mut CommandContext<'_, G>,
    cell: GroupRestorationCell,
    value: GroupRestorationValue<G>,
    escape_char: i32,
) -> Option<(String, String, bool)> {
    let escaped = |name: &str| escaped_at(escape_char, name);
    let (name, value, box_value) = match (cell, value) {
        (GroupRestorationCell::Meaning(symbol), GroupRestorationValue::Meaning(value)) => (
            restoration_token_text(stores, escape_char, Token::Cs(symbol)),
            meaning_value_text_at(stores, value, escape_char),
            false,
        ),
        (GroupRestorationCell::Count(index), GroupRestorationValue::Integer(value)) => {
            (escaped(&format!("count{index}")), value.to_string(), false)
        }
        (GroupRestorationCell::Dimension(index), GroupRestorationValue::Dimension(value)) => {
            (escaped(&format!("dimen{index}")), dimen_text(value), false)
        }
        (GroupRestorationCell::TokenRegister(index), GroupRestorationValue::TokenList(value)) => (
            escaped(&format!("toks{index}")),
            tokens_text_at(stores, value, escape_char),
            false,
        ),
        (GroupRestorationCell::GlueRegister(index), GroupRestorationValue::Glue(value)) => (
            escaped(&format!("skip{index}")),
            format_glue_with_unit(value.map_or(GlueSpec::ZERO, |id| stores.glue(id)), "pt"),
            false,
        ),
        (GroupRestorationCell::MuGlueRegister(index), GroupRestorationValue::Glue(value)) => (
            escaped(&format!("muskip{index}")),
            format_glue_with_unit(value.map_or(GlueSpec::ZERO, |id| stores.glue(id)), "mu"),
            false,
        ),
        (GroupRestorationCell::BoxRegister(index), GroupRestorationValue::NodeList(value)) => {
            debug_assert_eq!(stores.box_register(index).ok().flatten(), value);
            let page = stores.copy_box_to_page(index);
            (
                escaped(&format!("box{index}")),
                stores.box_assignment_trace_text(page),
                true,
            )
        }
        (GroupRestorationCell::IntegerParameter(index), GroupRestorationValue::Integer(value)) => {
            (escaped(&int_param_name(index)), value.to_string(), false)
        }
        (
            GroupRestorationCell::DimensionParameter(index),
            GroupRestorationValue::Dimension(value),
        ) => (escaped(&dimen_param_name(index)), dimen_text(value), false),
        (GroupRestorationCell::TokenParameter(index), GroupRestorationValue::TokenList(value)) => (
            escaped(&tok_param_name(index)),
            tokens_text_at(stores, value, escape_char),
            false,
        ),
        (GroupRestorationCell::GlueParameter(index), GroupRestorationValue::Glue(value)) => {
            let (raw_name, unit) = glue_param_name(index);
            (
                escaped(&raw_name),
                format_glue_with_unit(value.map_or(GlueSpec::ZERO, |id| stores.glue(id)), unit),
                false,
            )
        }
        (GroupRestorationCell::CurrentFont, GroupRestorationValue::Font(font)) => (
            "current font".to_owned(),
            font_identifier_text(stores, font, escape_char),
            false,
        ),
        (GroupRestorationCell::MathFamilyFont(index), GroupRestorationValue::Font(font)) => {
            let (prefix, family) = match index {
                0..=15 => ("textfont", index),
                16..=31 => ("scriptfont", index - 16),
                _ => ("scriptscriptfont", index - 32),
            };
            (
                escaped(&format!("{prefix}{family}")),
                font_identifier_text(stores, font, escape_char),
                false,
            )
        }
        (GroupRestorationCell::Code(kind, index), GroupRestorationValue::Code(value)) => {
            let primitive = match kind {
                tex_state::CodeTableKind::Catcode => "catcode",
                tex_state::CodeTableKind::Lccode => "lccode",
                tex_state::CodeTableKind::Uccode => "uccode",
                tex_state::CodeTableKind::Sfcode => "sfcode",
                tex_state::CodeTableKind::Mathcode => "mathcode",
                tex_state::CodeTableKind::Delcode => "delcode",
            };
            (
                escaped(&format!("{primitive}{index}")),
                value.to_string(),
                false,
            )
        }
        // Per-font runtime cells are not eqtb locations and therefore are not
        // operands of e-TeX's `restore_trace(p, ...)` hook.
        (GroupRestorationCell::FontRuntime(_), _) => return None,
        _ => unreachable!("state restoration receipt preserves cell/value kinds"),
    };
    Some((name, value, box_value))
}

fn font_identifier_text<G>(
    stores: &CommandContext<'_, G>,
    font: tex_state::ids::FontId,
    escape_char: i32,
) -> String {
    stores.font_identifier_symbol(font).map_or_else(
        || escaped_at(escape_char, "nullfont"),
        |symbol| restoration_token_text(stores, escape_char, Token::Cs(symbol)),
    )
}

/// e-TeX 2.6 [19.277]'s `assign_trace(p, label) == if tracing_assigns>0 then
/// restore_trace(p, label)`, gated against the *current* (live) state. Used
/// for the "into"/"reassigning" half of a write, which etex.ch checks after
/// the mutation has already happened.
fn emit<G>(stores: &mut CommandContext<'_, G>, label: &str, name: &str, value: &str) {
    if stores.int_param(IntParam::TRACING_ASSIGNS) > 0 {
        print_trace(stores, label, name, value);
    }
}

/// e-TeX 2.6 [19.277-19.279]'s shared assign-trace decision table, common to
/// `eq_word_define`/`eq_define` (local) and `geq_word_define`/`geq_define`
/// (global): "reassigning" for a same-value local write (no save needed, so
/// no "changing" half), "changing"+"into" for a different-value local write,
/// and "globally changing"+"into" unconditionally for a global write --
/// `geq_word_define`/`geq_define` never special-case an unchanged value.
///
/// Called after the write has already happened; `old`/`new` are the pre- and
/// post-image value text. `tracing_before` is `\tracingassigns>0` *before*
/// the write: etex.ch's `assign_trace(p,"changing")`/`"globally changing"`
/// call sits before the mutation in `eq_word_define`/`geq_word_define`, so a
/// write that turns tracing on must not show its own "changing" line, and one
/// that turns it off must still show it. The "into"/"reassigning" half is
/// checked live (post-write) through [`emit`], for the same reason in
/// reverse.
fn trace_scalar<G>(
    stores: &mut CommandContext<'_, G>,
    tracing_before: bool,
    global: bool,
    changed: bool,
    name: &str,
    old: &str,
    new: &str,
) {
    if global {
        if tracing_before {
            print_trace(stores, "globally changing", name, old);
        }
        emit(stores, "into", name, new);
    } else if changed {
        if tracing_before {
            print_trace(stores, "changing", name, old);
        }
        emit(stores, "into", name, new);
    } else {
        emit(stores, "reassigning", name, new);
    }
}

pub(crate) fn trace_int_param<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    tracing_before: bool,
    global: bool,
    old: i32,
    new: i32,
) {
    // `\tracingassigns` itself can change the live gate.  Preserve both
    // etex.ch checks, but do not render names and values when neither the
    // pre-image nor post-image requests a trace.
    if !tracing_before && stores.int_param(IntParam::TRACING_ASSIGNS) <= 0 {
        return;
    }
    let name = escaped(stores, &int_param_name(index));
    trace_scalar(
        stores,
        tracing_before,
        global,
        old != new,
        &name,
        &old.to_string(),
        &new.to_string(),
    );
}

pub(crate) fn trace_int_register<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    old: i32,
    new: i32,
) {
    // Count registers cannot alias `\tracingassigns` itself, so the live gate
    // at call time already equals its pre-write value.
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let name = escaped(stores, &format!("count{index}"));
    trace_scalar(
        stores,
        tracing_before,
        global,
        old != new,
        &name,
        &old.to_string(),
        &new.to_string(),
    );
}

pub(crate) fn trace_dimen_param<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    old: Scaled,
    new: Scaled,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let name = escaped(stores, &dimen_param_name(index));
    trace_scalar(
        stores,
        tracing_before,
        global,
        old != new,
        &name,
        &dimen_text(old),
        &dimen_text(new),
    );
}

pub(crate) fn trace_dimen_register<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    old: Scaled,
    new: Scaled,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let name = escaped(stores, &format!("dimen{index}"));
    trace_scalar(
        stores,
        tracing_before,
        global,
        old != new,
        &name,
        &dimen_text(old),
        &dimen_text(new),
    );
}

fn dimen_text(value: Scaled) -> String {
    format!("{}pt", format_scaled_for_diagnostics(value))
}

/// Traces an `eq_define` glue write using the caller's TeX pointer decision.
///
/// `changed` cannot be reconstructed from [`GlueSpec`]: Umber hash-conses
/// equal immutable specs, while TeX allocates a fresh node for every nonzero
/// scanned specification. The assignment owner therefore supplies whether
/// e-TeX [19.277] took the same-pointer `reassigning` return.
pub(crate) fn trace_glue_param<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    old: GlueSpec,
    new: GlueSpec,
    changed: bool,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let (raw_name, unit) = glue_param_name(index);
    let name = escaped(stores, &raw_name);
    let old_text = format_glue_with_unit(old, unit);
    let new_text = format_glue_with_unit(new, unit);
    trace_scalar(
        stores,
        tracing_before,
        global,
        changed,
        &name,
        &old_text,
        &new_text,
    );
}

/// Register counterpart of [`trace_glue_param`].
pub(crate) fn trace_glue_register<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    old: GlueSpec,
    new: GlueSpec,
    changed: bool,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let name = escaped(stores, &format!("skip{index}"));
    let old_text = format_glue_with_unit(old, "pt");
    let new_text = format_glue_with_unit(new, "pt");
    trace_scalar(
        stores,
        tracing_before,
        global,
        changed,
        &name,
        &old_text,
        &new_text,
    );
}

/// Mu-glue register counterpart of [`trace_glue_param`].
pub(crate) fn trace_muglue_register<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    old: GlueSpec,
    new: GlueSpec,
    changed: bool,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let name = escaped(stores, &format!("muskip{index}"));
    let old_text = format_glue_with_unit(old, "mu");
    let new_text = format_glue_with_unit(new, "mu");
    trace_scalar(
        stores,
        tracing_before,
        global,
        changed,
        &name,
        &old_text,
        &new_text,
    );
}

pub(crate) fn trace_tok_param<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    old: Option<TokenListId<G>>,
    new: Option<TokenListId<G>>,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let name = escaped(stores, &tok_param_name(index));
    let old_text = tokens_text(stores, old);
    let new_text = tokens_text(stores, new);
    trace_scalar(
        stores,
        tracing_before,
        global,
        old != new,
        &name,
        &old_text,
        &new_text,
    );
}

pub(crate) fn trace_toks_register<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    old: Option<TokenListId<G>>,
    new: Option<TokenListId<G>>,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let name = escaped(stores, &format!("toks{index}"));
    let old_text = tokens_text(stores, old);
    let new_text = tokens_text(stores, new);
    trace_scalar(
        stores,
        tracing_before,
        global,
        old != new,
        &name,
        &old_text,
        &new_text,
    );
}

/// Traces TeX82 §1077's box-register `eq_define` through e-TeX's generic
/// assignment hook. The non-void value begins on the newline introduced by
/// `show_node_list`, exactly as §252's `show_eqtb` does.
pub(crate) fn trace_box_write<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    global: bool,
    new: Option<&tex_state::node_arena::PageListId>,
    write: impl FnOnce(&mut CommandContext<'_, G>),
) {
    fn print_box_trace<G>(
        stores: &mut CommandContext<'_, G>,
        label: &str,
        name: &str,
        value: &str,
    ) {
        let mut diagnostic = stores.begin_diagnostic();
        diagnostic
            .print_char('{')
            .print(label)
            .print_char(' ')
            .print(name)
            .print_char('=');
        if value == "void" {
            diagnostic.print(value);
        } else {
            diagnostic.print_ln().print(value);
        }
        diagnostic.print_char('}');
        diagnostic.end(false);
    }

    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        write(stores);
        return;
    }
    let old = stores.copy_box_to_page(index);
    let name = escaped(stores, &format!("box{index}"));
    let old_text = stores.box_assignment_trace_text(old.clone());
    write(stores);
    let new_text = stores.box_assignment_trace_text(new.cloned());
    let changed = old.as_ref() != new;
    if global {
        if tracing_before {
            print_box_trace(stores, "globally changing", &name, &old_text);
        }
        if stores.int_param(IntParam::TRACING_ASSIGNS) > 0 {
            print_box_trace(stores, "into", &name, &new_text);
        }
    } else if changed {
        if tracing_before {
            print_box_trace(stores, "changing", &name, &old_text);
        }
        if stores.int_param(IntParam::TRACING_ASSIGNS) > 0 {
            print_box_trace(stores, "into", &name, &new_text);
        }
    } else if stores.int_param(IntParam::TRACING_ASSIGNS) > 0 {
        print_box_trace(stores, "reassigning", &name, &new_text);
    }
}

/// Renders one of e-TeX's four penalty arrays as merged `etex.web` §17
/// `show_eqtb` does: the empty shape is `0`, a singleton prints its count and
/// value, and a longer shape abbreviates everything after its first value as
/// `\ETC.`.
fn penalty_array_text<G>(stores: &mut CommandContext<'_, G>, values: &[i32]) -> String {
    let mut text = values.len().to_string();
    if let Some(first) = values.first() {
        text.push(' ');
        text.push_str(&first.to_string());
        if values.len() > 1 {
            text.push_str(&escaped(stores, "ETC."));
        }
    }
    text
}

/// Traces e-TeX's shape-backed penalty-array assignment through the same
/// merged `etex.web` §17 `assign_trace`/`show_eqtb` contract as ordinary
/// eqtb writes. Every populated assignment allocates a fresh shape node, so
/// even equal values are a change; only two null (empty) shapes are the same
/// local equivalent and therefore produce `reassigning`.
pub(crate) fn trace_penalty_array<G>(
    stores: &mut CommandContext<'_, G>,
    kind: PenaltyArrayKind,
    global: bool,
    old: &[i32],
    new: &[i32],
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let raw_name = match kind {
        PenaltyArrayKind::InterLine => "interlinepenalties",
        PenaltyArrayKind::Club => "clubpenalties",
        PenaltyArrayKind::Widow => "widowpenalties",
        PenaltyArrayKind::DisplayWidow => "displaywidowpenalties",
    };
    let name = escaped(stores, raw_name);
    let old_text = penalty_array_text(stores, old);
    let new_text = penalty_array_text(stores, new);
    trace_scalar(
        stores,
        tracing_before,
        global,
        !old.is_empty() || !new.is_empty(),
        &name,
        &old_text,
        &new_text,
    );
}

fn tokens_text<G>(stores: &mut CommandContext<'_, G>, tokens: Option<TokenListId<G>>) -> String {
    tokens_text_at(
        stores,
        tokens,
        stores.untracked_int_param(IntParam::ESCAPE_CHAR),
    )
}

fn tokens_text_at<G>(
    stores: &CommandContext<'_, G>,
    tokens: Option<TokenListId<G>>,
    escape_char: i32,
) -> String {
    let display = RestorationTokenDisplay {
        stores,
        escape_char,
    };
    let mut text = String::new();
    let words = tokens.map_or_else(Vec::new, |id| stores.token_list(id).to_vec());
    for token in words {
        tex_state::token_show::append_token_show_text(&display, token.semantic_token(), &mut text);
    }
    text
}

/// e-TeX 2.6 [17.687-750]'s code-table assign-trace: `\catcode`, `\lccode`,
/// `\uccode`, `\sfcode`, `\mathcode`, and `\delcode` are eqtb-resident 256/257
/// element tables written through the same `eq_word_define`/`geq_word_define`
/// as the integer parameter family, so their trace is a plain integer value
/// keyed by the character code rather than a register index.
pub(crate) fn trace_code<G>(
    stores: &mut CommandContext<'_, G>,
    primitive_name: &str,
    ch: char,
    global: bool,
    old: i32,
    new: i32,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let name = escaped(stores, &format!("{primitive_name}{}", ch as u32));
    trace_scalar(
        stores,
        tracing_before,
        global,
        old != new,
        &name,
        &old.to_string(),
        &new.to_string(),
    );
}

/// e-TeX 2.6 [17.687-750]'s meaning assign-trace for `\def`/`\edef`/`\gdef`/
/// `\xdef`/`\let`/`\futurelet` and the register/character `\...def` family.
///
/// Unlike the scalar families above, meaning text (`\show`'s rendering) can
/// only be read live from the token's current installed meaning, so this
/// wraps the actual write instead of taking pre-rendered old/new text: it
/// reads the pre-image, performs `write`, and reads the post-image, exactly
/// as `eq_define`'s `show_eqtb(p)` calls do on either side of the mutation.
///
/// `changed` is the caller's `eq_type(p)=t and equiv(p)=e` test (etex.ch
/// [17.696/17.715]): real TeX82 compares the *old and new equivalents by
/// pointer*, not by rendered content, so two separately-allocated `\def`s
/// with byte-identical bodies are never "reassigning" -- only a `\let`/
/// `\futurelet` that installs an already-installed meaning is. Callers pass
/// that decision explicitly rather than this module re-deriving it from a
/// `Meaning` equality check, which would accept the `\def` case too.
pub(crate) fn trace_meaning_write<G>(
    stores: &mut CommandContext<'_, G>,
    token: Token,
    changed: bool,
    global: bool,
    write: impl FnOnce(&mut CommandContext<'_, G>),
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        write(stores);
        return;
    }
    let mut name = String::new();
    stores.append_token_show_text(token, &mut name);
    if global {
        let old_text = stores.bounded_meaning_text(token, 32);
        write(stores);
        let new_text = stores.bounded_meaning_text(token, 32);
        print_trace(stores, "globally changing", &name, &old_text);
        emit(stores, "into", &name, &new_text);
    } else if changed {
        let old_text = stores.bounded_meaning_text(token, 32);
        write(stores);
        let new_text = stores.bounded_meaning_text(token, 32);
        print_trace(stores, "changing", &name, &old_text);
        emit(stores, "into", &name, &new_text);
    } else {
        write(stores);
        let text = stores.bounded_meaning_text(token, 32);
        emit(stores, "reassigning", &name, &text);
    }
}

/// Emits the assign trace for a meaning write already committed by the
/// command-owned scanner.
///
/// TeX82 §1224 installs a provisional `\relax` before scanning a shorthand
/// definition's numeric operand. That mutation must remain scanner-owned so a
/// self-referential target terminates the scan, but e-TeX [17.687--750]
/// observes it through the same `eq_define` hook as the later committed
/// meaning. Carrying the copyable pre-image across the scan/apply seam lets
/// this renderer preserve that ownership without replaying the mutation.
pub(crate) fn trace_completed_provisional_meaning_write<G>(
    stores: &mut CommandContext<'_, G>,
    token: Token,
    old: ResolvedMeaning<G>,
    new: Meaning,
    global: bool,
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        return;
    }
    let mut name = String::new();
    stores.append_token_show_text(token, &mut name);
    let escape_char = stores.untracked_int_param(IntParam::ESCAPE_CHAR);
    let old_text = meaning_value_text_at(stores, old, escape_char);
    let new_text = meaning_value_text_at(stores, ResolvedMeaning::Static(new), escape_char);
    trace_scalar(
        stores,
        tracing_before,
        global,
        old != new,
        &name,
        &old_text,
        &new_text,
    );
}

/// TeX82 §252's bounded `show_eqtb` value for a detached meaning pre-image.
fn meaning_value_text_at<G>(
    stores: &CommandContext<'_, G>,
    meaning: ResolvedMeaning<G>,
    escape_char: i32,
) -> String {
    let escaped = |name: &str| escaped_at(escape_char, name);
    let display = RestorationTokenDisplay {
        stores,
        escape_char,
    };
    match meaning {
        ResolvedMeaning::Static(Meaning::Undefined) => "undefined".to_owned(),
        ResolvedMeaning::Static(Meaning::Relax) => escaped("relax"),
        ResolvedMeaning::Static(Meaning::EndV) => escaped("endtemplate"),
        ResolvedMeaning::Static(Meaning::CharGiven(ch)) => format!("the character {ch}"),
        ResolvedMeaning::Static(Meaning::CharToken {
            ch,
            cat: tex_state::token::Catcode::Letter,
        }) => format!("the letter {ch}"),
        ResolvedMeaning::Static(Meaning::CharToken { ch, .. }) => {
            format!("the character {ch}")
        }
        ResolvedMeaning::Static(Meaning::MathCharGiven(value)) => {
            escaped(&format!("mathchar\"{value:X}"))
        }
        ResolvedMeaning::Static(Meaning::CountRegister(index)) => escaped(&format!("count{index}")),
        ResolvedMeaning::Static(Meaning::DimenRegister(index)) => escaped(&format!("dimen{index}")),
        ResolvedMeaning::Static(Meaning::SkipRegister(index)) => escaped(&format!("skip{index}")),
        ResolvedMeaning::Static(Meaning::MuskipRegister(index)) => {
            escaped(&format!("muskip{index}"))
        }
        ResolvedMeaning::Static(Meaning::ToksRegister(index)) => escaped(&format!("toks{index}")),
        ResolvedMeaning::Macro { flags, definition } => {
            let definition = stores.definition(definition);
            let parameter_text = definition.parameter_text().to_vec();
            let replacement_text = definition.replacement_text().to_vec();
            let mut text = String::new();
            for (flag, name) in [
                (MeaningFlags::PROTECTED, "protected"),
                (MeaningFlags::LONG, "long"),
                (MeaningFlags::OUTER, "outer"),
            ] {
                if flags.contains(flag) {
                    text.push_str(&escaped(name));
                }
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str("macro:");
            let mut shown = 0;
            let mut remaining = false;
            for token in parameter_text {
                if shown >= 32 {
                    remaining = true;
                    break;
                }
                let before = text.chars().count();
                tex_state::token_show::append_token_show_text(
                    &display,
                    token.semantic_token(),
                    &mut text,
                );
                shown += text.chars().count() - before;
            }
            if !remaining && shown < 32 {
                text.push_str("->");
                shown += 2;
                for token in replacement_text {
                    if shown >= 32 {
                        remaining = true;
                        break;
                    }
                    let before = text.chars().count();
                    tex_state::token_show::append_token_show_text(
                        &display,
                        token.semantic_token(),
                        &mut text,
                    );
                    shown += text.chars().count() - before;
                }
            } else {
                remaining = true;
            }
            if remaining {
                text.push_str(&escaped("ETC."));
            }
            text
        }
        ResolvedMeaning::Static(Meaning::Font(font)) => {
            let name = stores.font_name(font);
            format!("select font {name}")
        }
        ResolvedMeaning::Static(
            meaning @ (Meaning::ExpandablePrimitive(_) | Meaning::UnexpandablePrimitive(_)),
        ) => {
            let name = stores.primitive_name(meaning).map(str::to_owned);
            name.map_or_else(|| "unknown".to_owned(), |name| escaped(&name))
        }
        ResolvedMeaning::Static(
            meaning @ (Meaning::IntParam(_)
            | Meaning::InternalInteger(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)),
        ) => {
            let name = stores.primitive_name(meaning).map(str::to_owned);
            name.map_or_else(|| "unknown".to_owned(), |name| escaped(&name))
        }
        ResolvedMeaning::Static(Meaning::Unknown(_)) => "unknown".to_owned(),
    }
}
