//! e-TeX 2.6's `\tracingnesting` cross-file group/conditional balance
//! warning: `etex.ch` [23.328]'s `file_warning`.
//!
//! `group_warning`/`if_warning` (the sibling "a group/conditional closed in
//! a different file than it opened in" case, reported at the group/
//! conditional's own close rather than the file's) are not implemented yet;
//! see `docs/etex_primitives.md`.

use tex_state::env::banks::IntParam;

use crate::input::SourceOpenDepths;
use crate::processor::CommandProcessor;

impl CommandProcessor<'_> {
    /// `etex.ch` [23.328]'s `file_warning`, called once a source level has
    /// retired (its `end_file_reading` has run) with `open_depths` the
    /// group/conditional depth recorded when that level began.
    ///
    /// Prints "Warning: end of file when <group> is incomplete" for every
    /// group opened since, innermost first, then "Warning: end of file when
    /// <conditional> is incomplete" for every conditional opened since,
    /// innermost first, then one trailing newline -- exactly `file_warning`'s
    /// own two `while` loops and shared `print_ln`. Unlike
    /// `\tracingassigns`/`\tracinggroups`/`\tracingifs`, this prints through
    /// the ambient selector rather than `begin_diagnostic`'s `\tracingonline`
    /// redirect: `file_warning` is not `stat`-gated in `etex.ch` and reaches
    /// the terminal whenever the ambient selector already does.
    pub(crate) fn warn_file_boundary_incomplete(&mut self, open_depths: SourceOpenDepths) {
        if self.state.int_param(IntParam::TRACING_NESTING) <= 0 {
            return;
        }
        let group_depth = open_depths.group_depth as usize;
        let conditional_depth = open_depths.conditional_depth as usize;
        let current_group_depth = self.state.current_group_values().0.max(0) as usize;
        let current_conditional_depth = self.command.conditions.frames.len();
        if group_depth >= current_group_depth && conditional_depth >= current_conditional_depth {
            return;
        }

        // Pre-render every line's text before opening any print scope: the
        // group text needs no borrow, but the conditional text borrows
        // `self` read-only through `conditional_kind_text`, which cannot
        // overlap the printer's mutable borrow below.
        let group_lines: Vec<(usize, &'static str, u32)> = self
            .state
            .group_frames_from(group_depth)
            .into_iter()
            .rev()
            .collect();
        let condition_frames = self.command.conditions.frames
            [conditional_depth.min(current_conditional_depth)..]
            .to_vec();
        let condition_lines: Vec<String> = condition_frames
            .iter()
            .rev()
            .map(|frame| self.conditional_kind_text(frame))
            .collect();

        for (level, kind_text, entered_line) in &group_lines {
            let mut printer = self.state.printer();
            printer.print_nl("Warning: end of file when ");
            printer.print(kind_text);
            printer.print(" (level ");
            printer.print_int(i32::try_from(*level).unwrap_or(i32::MAX));
            printer.print_char(')');
            if *entered_line != 0 {
                printer.print(" entered at line ");
                printer.print_int(i32::try_from(*entered_line).unwrap_or(i32::MAX));
            }
            printer.print(" is incomplete");
        }
        for name in &condition_lines {
            let mut printer = self.state.printer();
            printer.print_nl("Warning: end of file when ");
            printer.print(name);
            printer.print(" is incomplete");
        }
        if !group_lines.is_empty() || !condition_lines.is_empty() {
            self.state.printer().print_ln();
            self.state.record_warning_history();
        }
    }
}
