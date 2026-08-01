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
//! Umber has no single low-level writer with that reach: [`super::variables`]
//! centralizes the scalar register/parameter families in
//! `execute_assignment_to_target`, and code-table writes centralize
//! similarly in `execute_code_table_assignment`, so this module is called
//! from both. Box-register (`\setbox`) and font-selection (`\font`,
//! `\textfont`, ...) assignments are also eqtb-resident in real TeX and are
//! not yet traced here; see `docs/etex_primitives.md`.
//!
//! `\tracingrestores`'s own `unsave`-time "restoring"/"retaining" lines are a
//! distinct TeX82 (not e-TeX) primitive. They are produced at Universe's
//! group-exit boundary, where ordered old-value records remain available;
//! this module renders only the four `\tracingassigns` labels.

use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::ids::{GlueId, TokenListId};
use tex_state::scaled::Scaled;
use tex_state::token::Token;

use super::primitives::{dimen_param_name, glue_param_name, int_param_name, tok_param_name};
use crate::node_dump::{format_glue_with_unit, format_scaled_for_diagnostics};

/// Renders one primitive/register name through the live `\escapechar`,
/// matching e-TeX 2.6's own escape-aware `print_esc`-based rendering.
fn escaped(stores: &mut Universe, name: &str) -> String {
    tex_command::print_esc_text(&stores.command_context(), name)
}

/// Prints `{label name=value}` unconditionally: the caller has already
/// decided `\tracingassigns` was positive at the moment that mattered.
fn print_trace(stores: &mut Universe, label: &str, name: &str, value: &str) {
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

/// e-TeX 2.6 [19.277]'s `assign_trace(p, label) == if tracing_assigns>0 then
/// restore_trace(p, label)`, gated against the *current* (live) state. Used
/// for the "into"/"reassigning" half of a write, which etex.ch checks after
/// the mutation has already happened.
fn emit(stores: &mut Universe, label: &str, name: &str, value: &str) {
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
fn trace_scalar(
    stores: &mut Universe,
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

pub(crate) fn trace_int_param(
    stores: &mut Universe,
    index: u16,
    tracing_before: bool,
    global: bool,
    old: i32,
    new: i32,
) {
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

pub(crate) fn trace_int_register(
    stores: &mut Universe,
    index: u16,
    global: bool,
    old: i32,
    new: i32,
) {
    let name = escaped(stores, &format!("count{index}"));
    // Count registers cannot alias `\tracingassigns` itself, so the live gate
    // at call time already equals its pre-write value.
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
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

pub(crate) fn trace_dimen_param(
    stores: &mut Universe,
    index: u16,
    global: bool,
    old: Scaled,
    new: Scaled,
) {
    let name = escaped(stores, &dimen_param_name(index));
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
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

pub(crate) fn trace_dimen_register(
    stores: &mut Universe,
    index: u16,
    global: bool,
    old: Scaled,
    new: Scaled,
) {
    let name = escaped(stores, &format!("dimen{index}"));
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
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

/// Note: this compares interned glue-spec identity, so two separately
/// scanned but value-equal *nonzero* glue literals are treated as
/// "changing" here even though Umber's hash-consing gives them the same
/// [`GlueId`]. TeX82 §1237's `trap_zero_glue` (the special case
/// `etex_redundant_local_zero_glue_assignment` guards at the call site) is
/// the only real "reassigning"-equivalent identity for glue parameters, and
/// zero glue always interns to the same [`GlueId`] either way, so this
/// approximation only over-reports "changing" for the narrower nonzero case.
pub(crate) fn trace_glue_param(
    stores: &mut Universe,
    index: u16,
    global: bool,
    old: GlueId,
    new: GlueId,
) {
    let (raw_name, unit) = glue_param_name(index);
    let name = escaped(stores, &raw_name);
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    let old_text = format_glue_with_unit(stores.glue(old), unit);
    let new_text = format_glue_with_unit(stores.glue(new), unit);
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

pub(crate) fn trace_glue_register(
    stores: &mut Universe,
    index: u16,
    global: bool,
    old: GlueId,
    new: GlueId,
) {
    let name = escaped(stores, &format!("skip{index}"));
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    let old_text = format_glue_with_unit(stores.glue(old), "pt");
    let new_text = format_glue_with_unit(stores.glue(new), "pt");
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

pub(crate) fn trace_muglue_register(
    stores: &mut Universe,
    index: u16,
    global: bool,
    old: GlueId,
    new: GlueId,
) {
    let name = escaped(stores, &format!("muskip{index}"));
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    let old_text = format_glue_with_unit(stores.glue(old), "mu");
    let new_text = format_glue_with_unit(stores.glue(new), "mu");
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

pub(crate) fn trace_tok_param(
    stores: &mut Universe,
    index: u16,
    global: bool,
    old: TokenListId,
    new: TokenListId,
) {
    let name = escaped(stores, &tok_param_name(index));
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
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

pub(crate) fn trace_toks_register(
    stores: &mut Universe,
    index: u16,
    global: bool,
    old: TokenListId,
    new: TokenListId,
) {
    let name = escaped(stores, &format!("toks{index}"));
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
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

fn tokens_text(stores: &Universe, id: TokenListId) -> String {
    let mut text = String::new();
    for &token in stores.tokens(id) {
        crate::diagnostics::append_token_show_text(stores, token, &mut text);
    }
    text
}

/// e-TeX 2.6 [17.687-750]'s code-table assign-trace: `\catcode`, `\lccode`,
/// `\uccode`, `\sfcode`, `\mathcode`, and `\delcode` are eqtb-resident 256/257
/// element tables written through the same `eq_word_define`/`geq_word_define`
/// as the integer parameter family, so their trace is a plain integer value
/// keyed by the character code rather than a register index.
pub(crate) fn trace_code(
    stores: &mut Universe,
    primitive_name: &str,
    ch: char,
    global: bool,
    old: i32,
    new: i32,
) {
    let name = escaped(stores, &format!("{primitive_name}{}", ch as u32));
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
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
pub(crate) fn trace_meaning_write(
    stores: &mut Universe,
    token: Token,
    changed: bool,
    global: bool,
    write: impl FnOnce(&mut Universe),
) {
    let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
    if !tracing_before {
        write(stores);
        return;
    }
    let mut name = String::new();
    crate::diagnostics::append_token_show_text(stores, token, &mut name);
    if global {
        let old_text = tex_expand::meaning_text(stores, token);
        write(stores);
        let new_text = tex_expand::meaning_text(stores, token);
        print_trace(stores, "globally changing", &name, &old_text);
        emit(stores, "into", &name, &new_text);
    } else if changed {
        let old_text = tex_expand::meaning_text(stores, token);
        write(stores);
        let new_text = tex_expand::meaning_text(stores, token);
        print_trace(stores, "changing", &name, &old_text);
        emit(stores, "into", &name, &new_text);
    } else {
        write(stores);
        let text = tex_expand::meaning_text(stores, token);
        emit(stores, "reassigning", &name, &text);
    }
}
