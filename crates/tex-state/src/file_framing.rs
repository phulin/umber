//! tex.web §54's `open_parens` and the three prints that maintain it.
//!
//! A job brackets every file it reads: §537's `start_input` prints `(` and the
//! file's name, §362 prints the matching `)` when the file's last line is
//! consumed, and §1335's `final_cleanup` closes whatever is still open with
//! `␣)` apiece.
//!
//! The counter is print-adjacent state on [`World`](crate::world::World),
//! beside §76's `history`, rather than driver state in the engine crate,
//! because §362 does not print its `)` at a step boundary:
//!
//! ```text
//! if force_eof then
//!   begin print_char(")"); decr(open_parens);
//!   ...
//!   end_file_reading; {resume previous level}
//!   check_outer_validity; goto restart;
//!   end;
//! ```
//!
//! `check_outer_validity` is what reports `Incomplete \iffalse` and the
//! runaway family, so the `)` precedes a diagnostic printed from deep inside
//! `get_next`. Only the command core stands at that point. A counter owned by
//! the engine driver forces the paren to be queued and rendered once the step
//! is over, which puts every such diagnostic *inside* the file bracket that
//! tex.web had already closed.
//!
//! Living on `World` also makes the counter roll back exactly as the prints
//! do: a step that opens a paren and is then abandoned restores both together
//! from the same `Universe` snapshot.

#[cfg(test)]
mod tests;

use crate::print::{Printer, Selector};
use crate::universe::Universe;

/// tex.web §54's `open_parens`: how many `(` have been printed with no
/// matching `)` yet.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileFraming {
    open_parens: u32,
}

impl FileFraming {
    /// The live count, which §1335 unwinds.
    #[must_use]
    pub const fn open_parens(&self) -> u32 {
        self.open_parens
    }

    pub(crate) const fn open(&mut self) {
        self.open_parens = self.open_parens.saturating_add(1);
    }

    pub(crate) const fn close(&mut self) {
        self.open_parens = self.open_parens.saturating_sub(1);
    }
}

/// tex.web §537's `print_char("("); incr(open_parens); slow_print(name)`,
/// with §537's own line-break decision in front of it.
///
/// The break tests `term_offset` alone, exactly as tex.web does -- not
/// `file_offset` -- even though the resulting `print_ln` or space is written
/// through the ambient selector to every channel it routes to. That asymmetry
/// is tex.web's, not an approximation of it.
pub fn print_file_open<G>(universe: &mut Universe<G>, name: &str) {
    let mut printer = universe.printer();
    let term_offset = printer.terminal_offset();
    if term_offset + name.chars().count() > printer.max_print_line() - 2 {
        printer.print_ln();
    } else if term_offset > 0 || printer.log_offset() > 0 {
        printer.print_char(' ');
    }
    printer.print_char('(');
    printer.print(name);
    universe.world_mut().file_framing_mut().open();
}

/// tex.web §362's bare `)`.
pub fn print_file_close<G>(universe: &mut Universe<G>) {
    universe.printer().print_char(')');
    universe.world_mut().file_framing_mut().close();
}

/// tex.web §1335's `while open_parens>0 do begin print("␣)"); decr(open_parens); end`.
pub fn print_remaining_file_closes<G>(universe: &mut Universe<G>) {
    while universe.world().file_framing().open_parens() > 0 {
        universe.printer().print(" )");
        universe.world_mut().file_framing_mut().close();
    }
}

/// §537's opening for a root file the driver selected before canonical
/// execution began.
///
/// A retained session opens its root outside the command core, so the opening
/// cannot arrive through the input stack. It is still §537's
/// `print_char("("); incr(open_parens)`, and §1335 must therefore see it when
/// `\end` or `\dump` abandons the still-open root -- but it is terminal-only,
/// because the log is not open yet when a root is selected.
pub fn print_startup_file_open<G>(universe: &mut Universe<G>, name: &str) {
    Printer::new(universe, Selector::TermOnly)
        .print_char('(')
        .print(name);
    universe.world_mut().file_framing_mut().open();
}

/// §537's startup opening after §536 has opened the transcript.
pub fn print_startup_file_open_after_log<G>(universe: &mut Universe<G>, name: &str) {
    let selector = Selector::for_interaction(universe.interaction_mode());
    Printer::new(universe, selector).print_char('(').print(name);
    universe.world_mut().file_framing_mut().open();
}
