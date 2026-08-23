//! The one path an ordinary recoverable error takes out of the stomach.
//!
//! tex.web reports every recoverable error as §73's `print_err`, then §79's
//! help lines, then §82's `error`. Writing the whole report as one literal
//! string to a print sink instead reproduces only the first of those three,
//! and silently drops the rest:
//!
//! - §82's `show_context` display. The location lines (`l.4␣\spacefactor`,
//!   `<to be read again>␣`, `<inserted text>␣`) are not part of the message;
//!   `error` prints them from the live command input after the message's
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

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;

/// tex.web §73's `print_err`, §79's help lines, and §82's `error`.
///
/// `context` is §82's already-rendered `show_context` display; pass
/// [`String::new`] only for a site with no input stack to display at all. The
/// renderer consumes an already-admitted command context and never resolves
/// input or source ownership itself.
pub(crate) fn report_error<G>(
    stores: &mut CommandContext<'_, G>,
    message: &str,
    help: &[&str],
    context: String,
) -> Result<(), crate::ExecError> {
    let mut report = stores.print_err(message);
    report.help(help);
    report.context(context);
    Ok(report.error().jump_out()?)
}

/// [`report_error`] after publishing the operation-local diagnostics that
/// precede this synchronous World-facing dialogue.
pub(crate) fn report_ordered_error<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    message: &str,
    help: &[&str],
    context: String,
) -> Result<(), crate::ExecError> {
    stores.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
    report_error(stores, message, help, context)
}
