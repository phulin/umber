//! e-TeX 2.6's `\tracingnesting` cross-file group/conditional balance
//! warning: `etex.ch` [23.328]'s `file_warning`.
//!
//! The sibling `group_warning`/`if_warning` path reports groups and
//! conditionals closed in a different file at the closer itself.

use tex_state::env::banks::IntParam;

use crate::conditionals::{ConditionFrame, IfLimit};
use crate::input::SourceOpenDepths;
use crate::processor::CommandProcessor;

impl CommandProcessor<'_> {
    /// Completes e-TeX 2.6 [23.328]'s `group_warning`/`if_warning` tail.
    /// Both procedures terminate the warning line and render the live
    /// `show_context` only at `\tracingnesting>1`.
    fn finish_cross_file_nesting_warning(&mut self, tracing_nesting: i32) {
        if tracing_nesting > 1 {
            let context = self.error_context();
            self.state.printer().print_rendered(&context);
        }
        self.state.record_warning_history();
    }

    /// e-TeX 2.6 [23.328]'s `group_warning`, emitted immediately before an
    /// `unsave` closes a group that was already open when this file began.
    pub fn warn_cross_file_group_close(
        &mut self,
        group_depth: usize,
        kind: &str,
        entered_line: u32,
    ) {
        let tracing_nesting = self.state.int_param(IntParam::TRACING_NESTING);
        if tracing_nesting <= 0 {
            return;
        }
        let Some(open_depths) = self.command.current_source_open_depths() else {
            return;
        };
        if group_depth > open_depths.group_depth as usize {
            return;
        }
        {
            let mut printer = self.state.printer();
            printer.print_nl("Warning: end of ");
            printer.print(kind);
            printer.print(" (level ");
            printer.print_int(i32::try_from(group_depth).unwrap_or(i32::MAX));
            printer.print_char(')');
            if entered_line != 0 {
                printer.print(" entered at line ");
                printer.print_int(i32::try_from(entered_line).unwrap_or(i32::MAX));
            }
            printer.print(" of a different file");
        }
        self.finish_cross_file_nesting_warning(tracing_nesting);
    }

    /// e-TeX 2.6 [23.328]'s `if_warning`, emitted when a delimiter closes a
    /// conditional that was already open when the current file began.
    pub(crate) fn warn_cross_file_conditional_close(&mut self, frame: &ConditionFrame) {
        let tracing_nesting = self.state.int_param(IntParam::TRACING_NESTING);
        if tracing_nesting <= 0 {
            return;
        }
        let Some(open_depths) = self.command.current_source_open_depths() else {
            return;
        };
        if self.command.conditions.frames.len() > open_depths.conditional_depth as usize {
            return;
        }
        let name = self.conditional_kind_text(frame);
        {
            let mut printer = self.state.printer();
            printer.print_nl("Warning: end of ");
            printer.print(&name);
            if frame.source_line != 0 {
                printer.print(" entered on line ");
                printer.print_int(i32::try_from(frame.source_line).unwrap_or(i32::MAX));
            }
            printer.print(" of a different file");
        }
        self.finish_cross_file_nesting_warning(tracing_nesting);
    }

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
        let group_start = if current_group_depth > group_depth {
            group_depth
        } else if current_group_depth == group_depth
            && self.state.current_group_lineage() != open_depths.group_lineage
        {
            group_depth.saturating_sub(1)
        } else {
            current_group_depth
        };
        let condition_start = if current_conditional_depth > conditional_depth {
            conditional_depth
        } else if current_conditional_depth == conditional_depth
            && self
                .command
                .conditions
                .current()
                .map(|frame| frame.identity.0)
                != open_depths.conditional_identity
        {
            conditional_depth.saturating_sub(1)
        } else {
            current_conditional_depth
        };

        // Pre-render every line's text before opening any print scope: the
        // group text needs no borrow, but the conditional text borrows
        // `self` read-only through `conditional_kind_text`, which cannot
        // overlap the printer's mutable borrow below.
        let group_lines: Vec<(usize, &'static str, u32)> = self
            .state
            .group_frames_from(group_start)
            .into_iter()
            .rev()
            .collect();
        let condition_frames =
            self.command.conditions.frames[condition_start..current_conditional_depth].to_vec();
        let condition_lines: Vec<(String, u32)> = condition_frames
            .iter()
            .rev()
            .map(|frame| {
                let mut text = self.conditional_kind_text(frame);
                // e-TeX 2.6 [23.328] renders the saved branch as `\else`
                // precisely when `if_limit=fi_code`; the other limits add no
                // delimiter to `print_cmd_chr(if_test,cur_if)`.
                if frame.limit == IfLimit::Fi {
                    text.push_str(&crate::processor::expand::print_esc_text(
                        &self.state,
                        "else",
                    ));
                }
                (text, frame.source_line)
            })
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
        for (name, entered_line) in &condition_lines {
            let mut printer = self.state.printer();
            printer.print_nl("Warning: end of file when ");
            printer.print(name);
            if *entered_line != 0 {
                printer.print(" entered on line ");
                printer.print_int(i32::try_from(*entered_line).unwrap_or(i32::MAX));
            }
            printer.print(" is incomplete");
        }
        if !group_lines.is_empty() || !condition_lines.is_empty() {
            self.state.printer().print_ln();
            self.state.record_warning_history();
        }
    }
}
