//! The one path an ordinary recoverable error takes out of the stomach.
//!
//! tex.web reports every recoverable error as §73's `print_err`, then §79's
//! help lines, then §82's `error`. Writing the whole report as one literal
//! string to a print sink instead reproduces only the first of those three,
//! and silently drops the rest:
//!
//! - §82's `show_context` display. The location lines (`l.4␣\spacefactor`,
//!   `<to be read again>␣`, `<inserted text>␣`) are not part of the message;
//!   `error` prints them from the live input stack after the message's
//!   closing period. A literal cannot know them, so a report written as a
//!   literal either omits them or hard-codes a guess.
//! - §90's `<Put help message on the transcript file>`, which is defined as a
//!   temporary `decr(selector)`. Help is **log-only** in every non-batch
//!   interaction; a literal sends it to the terminal too.
//! - §76's `history` and §82's `error_count`. Without them `\end`'s §1335
//!   note never reports that errors occurred, and the 100-error limit never
//!   trips.
//!
//! Every site therefore calls one of the entry points here. Adding a new
//! error message means adding a `report_*` call, never a `write_text`.

use tex_expand::back_error_input;
use tex_lex::InputStack;
use tex_state::token::TracedTokenWord;
use tex_state::{ExpansionContext, Universe};

use crate::diagnostics::show_context;

/// tex.web §73's `print_err`, §79's help lines, and §82's `error`.
///
/// `context` is §82's already-rendered `show_context` display; pass
/// [`String::new`] only for a site with no input stack to display at all.
pub(crate) fn report_error(
    stores: &mut Universe,
    message: &str,
    help: &[&str],
    context: String,
) -> Result<(), crate::ExecError> {
    let mut report = stores.print_err(message);
    report.help(help);
    report.context(context);
    Ok(report.error().jump_out()?)
}

/// [`report_error`] with §82's context read from the live input stack.
pub(crate) fn report_input_error(
    input: &InputStack,
    stores: &mut Universe,
    message: &str,
    help: &[&str],
) -> Result<(), crate::ExecError> {
    let context = show_context(stores, &input.summary());
    report_error(stores, message, help, context)
}

/// tex.web §327's `back_error`: `back_input` and then `error`.
///
/// Backing the offending token up first is what makes §314 describe it as
/// `<to be read again>␣` on its own context line, so the report names the
/// token that caused it without the message text having to quote it.
pub(crate) fn back_error(
    input: &mut InputStack,
    stores: &mut Universe,
    token: TracedTokenWord,
    message: &str,
    help: &[&str],
) -> Result<(), crate::ExecError> {
    back_tokens(input, stores, std::iter::once(token));
    report_input_error(input, stores, message, help)
}

/// tex.web §325's `back_input` for a report that backs up more than one
/// token, or that must back up before printing anything.
pub(crate) fn back_tokens<I>(input: &mut InputStack, stores: &mut Universe, tokens: I)
where
    I: IntoIterator<Item = TracedTokenWord>,
{
    back_error_input(input, &mut ExpansionContext::new(stores), tokens);
}

/// tex.web §327's `ins_error`: `back_input` with the list retyped as
/// `inserted`, then `error`.
///
/// The tokens are what TeX is inserting on the user's behalf -- the `$` of
/// `Missing $ inserted`, the `{` of `Missing { inserted` -- so §314 shows
/// them under `<inserted text>␣` rather than as something the user wrote.
pub(crate) fn ins_error<I>(
    input: &mut InputStack,
    stores: &mut Universe,
    tokens: I,
    message: &str,
    help: &[&str],
) -> Result<(), crate::ExecError>
where
    I: IntoIterator<Item = TracedTokenWord>,
{
    insert_tokens(input, stores, tokens);
    report_input_error(input, stores, message, help)
}

/// The `inserted` half of [`ins_error`], for a site that must insert before
/// it can render the message text.
///
/// Deliberately not [`back_tokens`]'s path: these tokens are TeX's own
/// repair, delivered for the first time, so the alignment brace count must
/// see them exactly as it would see any other token. Backing them up instead
/// would cancel a delivery that never happened.
pub(crate) fn insert_tokens<I>(input: &mut InputStack, stores: &mut Universe, tokens: I)
where
    I: IntoIterator<Item = TracedTokenWord>,
{
    crate::insert_traced_tokens(input, stores, tokens);
}
