//! tex.web §245's diagnostic print channel.
//!
//! Every `\tracing*` parameter writes its text through the same pair of
//! routines: `begin_diagnostic` redirects TeX's global print `selector` so the
//! trace reaches the transcript alone unless `\tracingonline` is positive, and
//! `end_diagnostic` closes the partial line and restores the selector. The
//! trace text itself is then produced by ordinary `print`/`print_nl` calls, so
//! no `\tracing*` implementation formats or routes its own output.
//!
//! The primitives themselves live in [`crate::print`], which owns tex.web
//! §54's `selector` and §§57-65's print routines; `begin_diagnostic` is
//! defined entirely as a temporary `decr(selector)` over that channel, so it
//! is a scope on [`crate::print::Printer`] rather than a second set of print
//! routines.
//!
//! §245 also raises `history` to `warning_issued` when it redirects to the
//! transcript. That is what makes §1335 print `(see the transcript file for
//! additional information)`: a run whose only diagnostics went to the log
//! has to tell the terminal they exist.

use crate::env::banks::IntParam;
use crate::print::{Printer, Selector};
use crate::scaled::Scaled;
use crate::universe::Universe;

/// An open diagnostic print scope: tex.web §245's `begin_diagnostic`.
///
/// Close it with [`Diagnostic::end`], whose `blank_line` argument is
/// `end_diagnostic`'s.
#[must_use = "an open diagnostic must be closed with `Diagnostic::end`"]
pub struct Diagnostic<'a, G> {
    printer: Printer<'a, G>,
}

impl<'a, G> Diagnostic<'a, G> {
    fn begin(universe: &'a mut Universe<G>) -> Self {
        let tracing_online = universe.int_param(IntParam::TRACING_ONLINE);
        Self::begin_with_tracing_online(universe, tracing_online)
    }

    pub(crate) fn begin_with_tracing_online(
        universe: &'a mut Universe<G>,
        tracing_online: i32,
    ) -> Self {
        // tex.web §245: `if (tracing_online<=0)and(selector=term_and_log) then
        // decr(selector)`, moving `term_and_log` to `log_only`.
        let mut selector = Selector::for_interaction(universe.interaction_mode());
        if tracing_online <= 0 && selector == Selector::TermAndLog {
            selector = selector.decr();
            // §245's other half: `if history=spotless then
            // history:=warning_issued`. The trace is about to become
            // transcript-only, and §1335's note is how the terminal learns
            // there is something there to read.
            universe
                .world_mut()
                .error_channel_mut()
                .record_warning_history();
        }
        Self {
            printer: Printer::new(universe, selector),
        }
    }

    /// The state being traced. Diagnostics read live state while printing.
    #[must_use]
    pub fn state(&self) -> &Universe<G> {
        self.printer.state()
    }

    /// tex.web §59's `print`.
    pub fn print(&mut self, text: &str) -> &mut Self {
        self.printer.print(text);
        self
    }

    /// Writes a display already rendered through TeX's print primitives.
    pub fn print_rendered(&mut self, text: &str) -> &mut Self {
        self.printer.print_rendered(text);
        self
    }

    /// Writes characters already encoded at the command-profile boundary.
    pub fn print_encoded_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.printer.print_encoded_bytes(bytes);
        self
    }

    /// tex.web §58's `print_char`.
    pub fn print_char(&mut self, character: char) -> &mut Self {
        self.printer.print_char(character);
        self
    }

    /// tex.web §68's `print_ASCII`, via the one-character string table.
    pub fn print_ascii(&mut self, character: char) -> &mut Self {
        self.printer.print_character_string(character);
        self
    }

    /// tex.web §57's `print_ln`.
    pub fn print_ln(&mut self) -> &mut Self {
        self.printer.print_ln();
        self
    }

    /// tex.web §62's `print_nl`: prints `text` at the start of a line.
    pub fn print_nl(&mut self, text: &str) -> &mut Self {
        self.printer.print_nl(text);
        self
    }

    /// tex.web §65's `print_int`.
    pub fn print_int(&mut self, value: i32) -> &mut Self {
        self.printer.print_int(value);
        self
    }

    /// tex.web §63's `print_esc`.
    pub fn print_esc(&mut self, name: &str) -> &mut Self {
        self.printer.print_esc(name);
        self
    }

    /// tex.web §103's `print_scaled`. The unit, if any, is the caller's.
    pub fn print_scaled(&mut self, value: Scaled) -> &mut Self {
        self.printer.print_scaled(value);
        self
    }

    /// tex.web §245's `end_diagnostic`.
    pub fn end(mut self, blank_line: bool) {
        self.print_nl("");
        if blank_line {
            self.print_ln();
        }
    }
}

impl<G> Universe<G> {
    /// tex.web §245's `begin_diagnostic`: opens the shared `\tracing*` print
    /// channel.
    pub fn begin_diagnostic(&mut self) -> Diagnostic<'_, G> {
        Diagnostic::begin(self)
    }

    /// Opens §245's diagnostic channel with terminal-and-log routing.
    ///
    /// e-TeX 2.6 change 17.516 temporarily sets the Pascal variable
    /// `tracing_online` while reporting level-two missing characters. This
    /// routing override deliberately does not perform an eqtb assignment or
    /// create a save-stack entry.
    pub fn begin_online_diagnostic(&mut self) -> Diagnostic<'_, G> {
        Diagnostic::begin_with_tracing_online(self, 1)
    }
}

#[cfg(test)]
mod tests;
