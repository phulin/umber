//! tex.web §245's diagnostic print channel.
//!
//! Every `\tracing*` parameter writes its text through the same pair of
//! routines: `begin_diagnostic` redirects TeX's global print `selector` so the
//! trace reaches the transcript alone unless `\tracingonline` is positive, and
//! `end_diagnostic` closes the partial line and restores the selector. The
//! trace text itself is then produced by ordinary `print`/`print_nl` calls, so
//! no `\tracing*` implementation formats or routes its own output.
//!
//! Umber has no global selector: every write names its [`PrintSink`], so the
//! open scope carries the routed sink instead of mutating a global. The
//! observable behavior is the one tex.web specifies.
//!
//! Not modeled here: tex.web §245 also raises `history` to `warning_issued`
//! when it redirects to the transcript, which feeds TeX's process exit status.
//! Umber does not track `history` at all, so that part of §245 is absent
//! rather than approximated.

use crate::env::banks::IntParam;
use crate::scaled::Scaled;
use crate::universe::Universe;
use crate::world::PrintSink;

/// An open diagnostic print scope: tex.web §245's `begin_diagnostic`.
///
/// Close it with [`Diagnostic::end`], whose `blank_line` argument is
/// `end_diagnostic`'s.
#[must_use = "an open diagnostic must be closed with `Diagnostic::end`"]
pub struct Diagnostic<'a> {
    universe: &'a mut Universe,
    sink: PrintSink,
}

impl<'a> Diagnostic<'a> {
    fn begin(universe: &'a mut Universe) -> Self {
        // tex.web §245: `if (tracing_online<=0)and(selector=term_and_log) then
        // decr(selector)`, moving `term_and_log` to `log_only`.
        let sink = if universe.int_param(IntParam::TRACING_ONLINE) <= 0 {
            PrintSink::Log
        } else {
            PrintSink::TerminalAndLog
        };
        Self { universe, sink }
    }

    /// The state being traced. Diagnostics read live state while printing.
    #[must_use]
    pub fn state(&self) -> &Universe {
        self.universe
    }

    /// tex.web §59's `print`.
    pub fn print(&mut self, text: &str) -> &mut Self {
        self.universe.world_mut().write_text(self.sink, text);
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

    /// tex.web §245's `end_diagnostic`.
    pub fn end(mut self, blank_line: bool) {
        self.print_nl("");
        if blank_line {
            self.print_ln();
        }
    }

    /// tex.web §62's guard, `(term_offset>0)and(odd(selector))` or
    /// `(file_offset>0)and(selector>=log_only)`, over the sinks this scope
    /// routes to. `log_only` is even and `term_and_log` is odd, so the
    /// transcript offset always counts and the terminal offset counts only
    /// when the terminal is also being written.
    fn line_is_open(&self) -> bool {
        let streams = self.universe.world().stream_bufs();
        match self.sink {
            PrintSink::Log => !streams.log_partial_line().is_empty(),
            PrintSink::TerminalAndLog => {
                !streams.terminal_partial_line().is_empty()
                    || !streams.log_partial_line().is_empty()
            }
            PrintSink::Terminal => !streams.terminal_partial_line().is_empty(),
            PrintSink::Stream(slot) => !streams.partial_line(slot).is_empty(),
        }
    }
}

impl Universe {
    /// tex.web §245's `begin_diagnostic`: opens the shared `\tracing*` print
    /// channel.
    pub fn begin_diagnostic(&mut self) -> Diagnostic<'_> {
        Diagnostic::begin(self)
    }
}

#[cfg(test)]
mod tests;
