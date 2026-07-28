//! tex.web §54's print `selector`, §§57-65's print primitives, §73's
//! `print_err`, and §82's `error`.
//!
//! Every message TeX shows a user -- ordinary `\message` text, `\show`
//! output, and every recoverable-error report -- is produced by the same
//! print primitives routed through one global `selector`. This module owns
//! that channel so no caller formats or routes its own output.
//!
//! [`crate::diagnostic`] is the `\tracing*` half of the same machinery:
//! tex.web §245's `begin_diagnostic` is defined entirely as a temporary
//! `decr(selector)`, so it is built on [`Printer`] here rather than
//! duplicating the primitives.
//!
//! # What Umber models differently, and what it does not model
//!
//! - **`log_opened` is constantly true.** tex.web §75 lists four possible
//!   `selector` values at error time, split by whether the transcript is
//!   open. Umber's [`crate::world::World`] always accepts
//!   [`PrintSink::Log`] writes -- there is no `open_log_file` moment -- so
//!   §75's `no_print`/`term_only` half of the table is unreachable and
//!   [`Selector::for_interaction`] yields only `log_only` and
//!   `term_and_log`.
//! - **`show_context` is not called.** tex.web §82 shows the input-stack
//!   context after every error. That needs the live input stack, which is
//!   not reachable from this layer; the context line is absent rather than
//!   approximated.
//! - **`history` is not tracked** (see [`crate::diagnostic`]), so §82's
//!   `error_message_issued` transition and §1335's "see the transcript file"
//!   note are absent.
//! - **`jump_out` is not representable.** §82's 100-error abort and §84's
//!   `X` option therefore return from `error` instead of terminating the
//!   job.
//! - **`deletions_allowed` is effectively false** and §87's `I` insertion is
//!   unavailable, because §84's digit and `I` options both drive the input
//!   stack. Both fall to §84's `othercases do_nothing`, which is exactly
//!   what tex.web does when `deletions_allowed` is false: §85's menu is
//!   printed and the prompt repeats.

use crate::env::banks::IntParam;
use crate::scaled::Scaled;
use crate::universe::{InteractionMode, Universe};
use crate::world::PrintSink;

/// tex.web §54's `max_print_line`.
pub const MAX_PRINT_LINE: usize = 79;

/// tex.web §54's `selector` values that ordinary printing can hold.
///
/// The discriminants are tex.web's own, so `decr`/`incr` on a selector and
/// the `odd(selector)` and `selector>=log_only` tests are the module's
/// arithmetic rather than a re-derivation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum Selector {
    /// §54's `no_print=16`.
    NoPrint = 16,
    /// §54's `term_only=17`.
    TermOnly = 17,
    /// §54's `log_only=18`.
    LogOnly = 18,
    /// §54's `term_and_log=19`.
    TermAndLog = 19,
}

impl Selector {
    /// tex.web §75's `<Initialize the print selector based on interaction>`
    /// followed by §1265's `if log_opened then selector:=selector+2`.
    /// Umber's transcript is always open, so the `+2` always applies.
    #[must_use]
    pub const fn for_interaction(interaction: InteractionMode) -> Self {
        match interaction {
            InteractionMode::Batch => Self::LogOnly,
            _ => Self::TermAndLog,
        }
    }

    /// The routed sink, or `None` for §54's `no_print`.
    #[must_use]
    pub const fn sink(self) -> Option<PrintSink> {
        match self {
            Self::NoPrint => None,
            Self::TermOnly => Some(PrintSink::Terminal),
            Self::LogOnly => Some(PrintSink::Log),
            Self::TermAndLog => Some(PrintSink::TerminalAndLog),
        }
    }

    /// tex.web's `decr(selector)`, saturating at §54's `no_print`.
    #[must_use]
    pub const fn decr(self) -> Self {
        match self {
            Self::NoPrint | Self::TermOnly => Self::NoPrint,
            Self::LogOnly => Self::TermOnly,
            Self::TermAndLog => Self::LogOnly,
        }
    }

    /// tex.web's `incr(selector)`, saturating at §54's `term_and_log`.
    #[must_use]
    pub const fn incr(self) -> Self {
        match self {
            Self::NoPrint => Self::TermOnly,
            Self::TermOnly => Self::LogOnly,
            Self::LogOnly | Self::TermAndLog => Self::TermAndLog,
        }
    }

    /// tex.web §62's `odd(selector)`.
    #[must_use]
    pub const fn writes_terminal(self) -> bool {
        matches!(self, Self::TermOnly | Self::TermAndLog)
    }

    /// tex.web §62's `selector>=log_only`.
    #[must_use]
    pub const fn writes_log(self) -> bool {
        matches!(self, Self::LogOnly | Self::TermAndLog)
    }
}

/// tex.web §§57-65's print primitives over one [`Selector`].
pub struct Printer<'a> {
    universe: &'a mut Universe,
    selector: Selector,
}

impl<'a> Printer<'a> {
    /// Opens a print scope routed by `selector`.
    pub fn new(universe: &'a mut Universe, selector: Selector) -> Self {
        Self { universe, selector }
    }

    /// The state being printed from. Printing reads live state.
    #[must_use]
    pub fn state(&self) -> &Universe {
        self.universe
    }

    /// Mutable state, for callers that must render from and then update it.
    pub fn state_mut(&mut self) -> &mut Universe {
        self.universe
    }

    #[must_use]
    pub const fn selector(&self) -> Selector {
        self.selector
    }

    /// tex.web's `selector:=#`, for the scoped redirections in §71, §86,
    /// §90, §245, and §1298.
    pub const fn set_selector(&mut self, selector: Selector) {
        self.selector = selector;
    }

    /// tex.web §59's `print`. §60's `slow_print` differs only in how the
    /// string pool is consulted, which Umber has no analogue for.
    pub fn print(&mut self, text: &str) -> &mut Self {
        if let Some(sink) = self.selector.sink() {
            self.universe.world_mut().write_text(sink, text);
        }
        self
    }

    /// tex.web §58's `print_char`.
    pub fn print_char(&mut self, character: char) -> &mut Self {
        let mut buffer = [0u8; 4];
        self.print(character.encode_utf8(&mut buffer))
    }

    /// tex.web §57's `print_ln`.
    pub fn print_ln(&mut self) -> &mut Self {
        self.print("\n")
    }

    /// tex.web §62's `print_nl`: prints `text` at the start of a line.
    pub fn print_nl(&mut self, text: &str) -> &mut Self {
        if self.line_is_open() {
            self.print_ln();
        }
        self.print(text)
    }

    /// tex.web §65's `print_int`.
    pub fn print_int(&mut self, value: i32) -> &mut Self {
        self.print(&value.to_string())
    }

    /// tex.web §103's `print_scaled`. The unit, if any, is the caller's.
    pub fn print_scaled(&mut self, value: Scaled) -> &mut Self {
        self.print(&crate::scaled::print_scaled(value))
    }

    /// tex.web §63's `print_esc`: `\escapechar` followed by the name, with
    /// the escape omitted when `\escapechar` is outside `0..255`.
    pub fn print_esc(&mut self, name: &str) -> &mut Self {
        let escape = self.universe.int_param(IntParam::ESCAPE_CHAR);
        if (0..256).contains(&escape)
            && let Some(character) = u32::try_from(escape).ok().and_then(char::from_u32)
        {
            self.print_char(character);
        }
        self.print(name)
    }

    /// tex.web §54's `term_offset`.
    #[must_use]
    pub fn terminal_offset(&self) -> usize {
        self.universe
            .world()
            .stream_bufs()
            .terminal_partial_line()
            .chars()
            .count()
    }

    /// tex.web §54's `file_offset`.
    #[must_use]
    pub fn log_offset(&self) -> usize {
        self.universe
            .world()
            .stream_bufs()
            .log_partial_line()
            .chars()
            .count()
    }

    /// tex.web §62's guard, `(term_offset>0)and(odd(selector))` or
    /// `(file_offset>0)and(selector>=log_only)`.
    fn line_is_open(&self) -> bool {
        (self.selector.writes_terminal() && self.terminal_offset() > 0)
            || (self.selector.writes_log() && self.log_offset() > 0)
    }
}

/// A recoverable error being reported: tex.web §73's `print_err` through
/// §82's `error`.
///
/// The message text is printed as the report is built; §79's help lines and
/// §1283's `use_err_help` are accumulated and shown by [`Self::error`].
#[must_use = "an opened error report must be completed with `error` or `int_error`"]
pub struct ErrorReport<'a> {
    printer: Printer<'a>,
    help: Vec<String>,
    err_help: Option<String>,
}

impl<'a> ErrorReport<'a> {
    fn begin(universe: &'a mut Universe, text: &str) -> Self {
        // tex.web §73: `print_nl("! "); print(#)`.
        let selector = Selector::for_interaction(universe.interaction_mode());
        let mut printer = Printer::new(universe, selector);
        printer.print_nl("! ").print(text);
        Self {
            printer,
            help: Vec::new(),
            err_help: None,
        }
    }

    /// tex.web §59's `print`, continuing the message text.
    pub fn print(&mut self, text: &str) -> &mut Self {
        self.printer.print(text);
        self
    }

    /// tex.web §58's `print_char`.
    pub fn print_char(&mut self, character: char) -> &mut Self {
        self.printer.print_char(character);
        self
    }

    /// tex.web §65's `print_int`.
    pub fn print_int(&mut self, value: i32) -> &mut Self {
        self.printer.print_int(value);
        self
    }

    /// tex.web §103's `print_scaled`.
    pub fn print_scaled(&mut self, value: Scaled) -> &mut Self {
        self.printer.print_scaled(value);
        self
    }

    /// tex.web §63's `print_esc`.
    pub fn print_esc(&mut self, name: &str) -> &mut Self {
        self.printer.print_esc(name);
        self
    }

    /// The selector this report prints through, for the scoped redirections
    /// tex.web performs mid-message (§1298's is the only one).
    #[must_use]
    pub const fn selector(&self) -> Selector {
        self.printer.selector()
    }

    /// tex.web's `selector:=#` inside an open report.
    pub const fn set_selector(&mut self, selector: Selector) {
        self.printer.set_selector(selector);
    }

    /// tex.web §79's `help1`..`help6`, in the order the lines are printed.
    pub fn help(&mut self, lines: &[&str]) -> &mut Self {
        self.help = lines.iter().map(|line| (*line).to_owned()).collect();
        self
    }

    /// tex.web §1283's `use_err_help:=true`, carrying the rendered
    /// `\errhelp` list §1284's `give_err_help` shows. Token rendering is the
    /// caller's, so this layer never inspects a token list.
    pub fn use_err_help(&mut self, rendered: String) -> &mut Self {
        self.err_help = Some(rendered);
        self
    }

    /// tex.web §91's `int_error`.
    pub fn int_error(mut self, value: i32) {
        self.print(" (").print_int(value).print_char(')');
        self.error();
    }

    /// tex.web §82's `error`.
    pub fn error(mut self) {
        self.printer.print_char('.');
        // §82's `show_context` is not modeled; see the module documentation.
        if self.printer.universe.interaction_mode() == InteractionMode::ErrorStop {
            self.users_advice();
            return;
        }
        self.printer
            .universe
            .world_mut()
            .error_channel_mut()
            .record_scrolled_error();
        self.help_on_transcript();
    }

    /// tex.web §90's `<Put help message on the transcript file>`.
    fn help_on_transcript(&mut self) {
        let restore = self.printer.selector();
        let redirect = self.printer.universe.interaction_mode() != InteractionMode::Batch;
        if redirect {
            self.printer.set_selector(restore.decr());
        }
        if let Some(rendered) = self.err_help.clone() {
            self.printer.print_ln().print(&rendered);
        } else {
            let lines = std::mem::take(&mut self.help);
            for line in &lines {
                self.printer.print_nl(line);
            }
        }
        self.printer.print_ln();
        if redirect {
            self.printer.set_selector(restore);
        }
        self.printer.print_ln();
    }

    /// tex.web §83's `<Get user's advice and return>` over the subset of
    /// §84's options that need no input stack.
    fn users_advice(&mut self) {
        loop {
            if self.printer.universe.interaction_mode() != InteractionMode::ErrorStop {
                return;
            }
            // §330's `clear_for_error_prompt` flushes pending terminal input,
            // which Umber's line-oriented terminal source has none of.
            self.printer.print("? ");
            let Some(line) = self.terminal_input() else {
                return;
            };
            let Some(code) = line.bytes().next().map(|byte| byte.to_ascii_uppercase()) else {
                return;
            };
            match code {
                // §89's `<Print the help information and goto continue>`.
                b'H' => self.show_help(),
                // §86's `<Change the interaction level and return>`.
                b'Q' | b'R' | b'S' => {
                    self.change_interaction(code);
                    return;
                }
                // §84's `X` ends the job through `jump_out`, which this layer
                // cannot do; the interaction change it makes first is real.
                b'X' => {
                    self.printer
                        .universe
                        .set_interaction_mode(InteractionMode::Scroll);
                    return;
                }
                // §84's `othercases do_nothing`, then §85's menu.
                _ => self.show_menu(),
            }
        }
    }

    /// tex.web §71's `term_input`, including its echo to the transcript.
    fn terminal_input(&mut self) -> Option<String> {
        let line = self
            .printer
            .universe
            .world_mut()
            .read_terminal_line()
            .ok()
            .flatten()?;
        let restore = self.printer.selector();
        self.printer.set_selector(restore.decr());
        self.printer.print(&line).print_ln();
        self.printer.set_selector(restore);
        Some(line)
    }

    /// tex.web §89's `<Print the help information and goto continue>`.
    fn show_help(&mut self) {
        if let Some(rendered) = self.err_help.take() {
            self.printer.print(&rendered).print_ln();
            return;
        }
        if self.help.is_empty() {
            self.help = vec![
                "Sorry, I don't know how to help in this situation.".into(),
                "Maybe you should try asking a human?".into(),
            ];
        }
        let lines = std::mem::take(&mut self.help);
        for line in &lines {
            self.printer.print(line).print_ln();
        }
        self.help = vec![
            "Sorry, I already gave what help I could...".into(),
            "Maybe you should try asking a human?".into(),
            "An error might have occurred before I noticed any problems.".into(),
            "``If all else fails, read the instructions.''".into(),
        ];
    }

    /// tex.web §86's `<Change the interaction level and return>`.
    fn change_interaction(&mut self, code: u8) {
        let (mode, name) = match code {
            b'Q' => (InteractionMode::Batch, "batchmode"),
            b'R' => (InteractionMode::Nonstop, "nonstopmode"),
            _ => (InteractionMode::Scroll, "scrollmode"),
        };
        self.printer
            .universe
            .world_mut()
            .error_channel_mut()
            .clear_error_count();
        self.printer.universe.set_interaction_mode(mode);
        self.printer.print("OK, entering ").print_esc(name);
        if mode == InteractionMode::Batch {
            let selector = self.printer.selector().decr();
            self.printer.set_selector(selector);
        }
        self.printer.print("...").print_ln();
    }

    /// tex.web §85's `<Print the menu of available options>`. Umber offers
    /// neither `E` nor token deletion, so those two lines never apply.
    fn show_menu(&mut self) {
        self.printer
            .print("Type <return> to proceed, S to scroll future error messages,")
            .print_nl("R to run without stopping, Q to run quietly,")
            .print_nl("I to insert something, ")
            .print_nl("H for help, X to quit.");
    }
}

/// Mutable state tex.web keeps for the error channel across errors.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ErrorChannel {
    error_count: i32,
    long_help_seen: bool,
}

impl ErrorChannel {
    /// tex.web §82's `incr(error_count)`. §82's 100-error `jump_out` is not
    /// representable here; see the module documentation.
    pub const fn record_scrolled_error(&mut self) {
        self.error_count = self.error_count.saturating_add(1);
    }

    /// tex.web §86's `error_count:=0`, also §1054's paragraph reset.
    pub const fn clear_error_count(&mut self) {
        self.error_count = 0;
    }

    /// tex.web §76's `error_count`.
    #[must_use]
    pub const fn error_count(&self) -> i32 {
        self.error_count
    }

    /// tex.web §1281's `long_help_seen`, returning whether the long
    /// `\errmessage` help has already been given and marking it seen.
    pub const fn take_long_help_seen(&mut self, mark: bool) -> bool {
        let seen = self.long_help_seen;
        if mark {
            self.long_help_seen = true;
        }
        seen
    }
}

impl Universe {
    /// tex.web §73's `print_err`, opening a recoverable-error report.
    pub fn print_err(&mut self, text: &str) -> ErrorReport<'_> {
        ErrorReport::begin(self, text)
    }

    /// tex.web §82's `error` for a message printed *without* §73's
    /// `print_err`.
    ///
    /// §1293's `\show` completion is the one such site: §1294 and §1297 print
    /// their `>␣` line through the ordinary print routines and then reach
    /// `common_ending`'s bare `error`.
    pub fn error_report(&mut self) -> ErrorReport<'_> {
        let selector = Selector::for_interaction(self.interaction_mode());
        ErrorReport {
            printer: Printer::new(self, selector),
            help: Vec::new(),
            err_help: None,
        }
    }

    /// An ordinary print scope at the §75 selector the current interaction
    /// mode implies.
    pub fn printer(&mut self) -> Printer<'_> {
        let selector = Selector::for_interaction(self.interaction_mode());
        Printer::new(self, selector)
    }
}

#[cfg(test)]
mod tests;
