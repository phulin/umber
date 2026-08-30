//! e-TeX 2.6's `\tracingnesting` cross-file group/conditional balance
//! warning: `etex.ch` [23.328]'s `file_warning`.
//!
//! The sibling `group_warning`/`if_warning` path reports groups and
//! conditionals closed in a different file at the closer itself.

use tex_state::env::banks::IntParam;

use crate::conditionals::{ConditionFrame, IfLimit};
use crate::input::SourceOpenDepths;
use crate::input::{FileWarningBoundary, InputLevelId};
use crate::processor::CommandProcessor;

impl<G> CommandProcessor<'_, '_, G> {
    /// Captures e-TeX 2.6 [23.328]'s source-opening ancestry before the new
    /// input frame becomes visible.
    pub(crate) fn capture_source_open_depths(&self) -> SourceOpenDepths {
        SourceOpenDepths {
            group_lineages: self.state.group_lineages().into_boxed_slice(),
            conditional_identities: self
                .command
                .conditions
                .frames
                .iter()
                .map(|frame| frame.identity.0)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    /// Borrows the still-live source ancestry and returns the only facts
    /// `file_warning` needs after retirement. No ancestry owner crosses the
    /// pop or enters a checkpoint retirement receipt.
    pub(crate) fn prepare_file_warning_boundary(
        &self,
        identity: InputLevelId,
    ) -> Option<FileWarningBoundary> {
        let open_depths = self.command.top_source_open_depths(identity)?;
        let group_start = open_depths
            .group_lineages
            .iter()
            .zip(self.state.group_frames())
            .take_while(|(saved, current)| **saved == current.lineage())
            .count();
        let condition_start = open_depths
            .conditional_identities
            .iter()
            .zip(&self.command.conditions.frames)
            .take_while(|(saved, current)| **saved == current.identity.0)
            .count();
        Some(FileWarningBoundary {
            group_start: u32::try_from(group_start).unwrap_or(u32::MAX),
            condition_start: u32::try_from(condition_start).unwrap_or(u32::MAX),
        })
    }

    /// Completes e-TeX 2.6 [23.328]'s nesting-warning context tail.
    fn finish_nesting_warning(&mut self, tracing_nesting: i32) {
        if tracing_nesting > 1 {
            let starts_with_print_ln = self.command.open_context_starts_with_print_ln(self.state);
            let context = self.error_context();
            let mut printer = self.state.printer();
            if starts_with_print_ln {
                // `group_warning`/`if_warning` have already finished their
                // warning line. §314 nevertheless performs an unconditional
                // `print_ln` before an ordinary token-list level.
                printer.print_ln();
            }
            printer.print_rendered(&context);
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
        if group_depth > open_depths.group_lineages.len() {
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
        self.finish_nesting_warning(tracing_nesting);
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
        if self.command.conditions.frames.len() > open_depths.conditional_identities.len() {
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
        self.finish_nesting_warning(tracing_nesting);
    }

    /// `etex.ch` [23.328]'s `file_warning`, called once a source level has
    /// retired (its `end_file_reading` has run) with `open_depths` the
    /// group/conditional boundary ancestry recorded when that level began.
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
    pub(crate) fn warn_file_boundary_incomplete(
        &mut self,
        boundary: FileWarningBoundary,
        saved_context: Option<String>,
    ) {
        let tracing_nesting = self.state.int_param(IntParam::TRACING_NESTING);
        if tracing_nesting <= 0 {
            return;
        }
        let group_start = boundary.group_start as usize;
        let current_conditional_depth = self.command.conditions.frames.len();
        let condition_start = (boundary.condition_start as usize).min(current_conditional_depth);

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
                    crate::processor::expand_render::append_print_esc_text(
                        self.state, "else", &mut text,
                    );
                }
                (text, frame.source_line)
            })
            .collect();

        for (level, kind_text, entered_line) in &group_lines {
            self.observe_diagnostic_lifecycle(
                crate::DiagnosticClass::Warning,
                "warning",
                "incomplete-source-nesting",
                vec![crate::DiagnosticArgument::Name("group".into())],
            );
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
            self.observe_diagnostic_lifecycle(
                crate::DiagnosticClass::Warning,
                "warning",
                "incomplete-source-nesting",
                vec![crate::DiagnosticArgument::Name("conditional".into())],
            );
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
            if tracing_nesting > 1 {
                let starts_with_print_ln = saved_context.is_none()
                    && self.command.open_context_starts_with_print_ln(self.state);
                let context = saved_context.unwrap_or_else(|| self.error_context());
                let mut printer = self.state.printer();
                if starts_with_print_ln {
                    printer.print_ln();
                }
                printer.print_rendered(&context);
                self.state.record_warning_history();
            } else {
                self.state.printer().print_ln();
                self.state.record_warning_history();
            }
        }
    }
}
