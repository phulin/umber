//! Source-free paragraph completion transaction for canonical command control.

use tex_state::Universe;
use tex_typeset::linebreak::WidowPenaltySelector;

mod hyphenation;
mod runtime;

#[cfg(any())]
pub(crate) use hyphenation::{
    apply_hyphenation_exceptions, apply_patterns, hyphenated_hlist as test_hyphenated_hlist_owned,
    parse_pattern_word, pattern_capacity_error, report_apply_diagnostics,
    test_automatic_discretionary, test_hyphenated_word,
    test_hyphenated_word as test_hyphenated_hlist, test_hyphenated_word_text,
    test_language_context, test_physical_post_break_span, test_physical_pre_break_projection,
};
pub(crate) use hyphenation::{apply_scanned_hyphenation_exceptions, apply_scanned_patterns};

pub use runtime::cached_pretolerance_plan;
pub(crate) use runtime::{
    ParagraphBreakResult, break_current_paragraph, display_line_dimensions, normal_paragraph,
    start_paragraph,
};
#[cfg(any())]
pub(crate) use runtime::{
    apply_line_expansion, break_hlist, test_apply_pdf_line_dimensions,
    test_discretionary_diagnostics_differ, test_materialize_pdf_line, test_pretolerance_memo_key,
};

use crate::box_runtime::{commit_current_list, flush_pending_hchars_with_fuel};
use crate::vertical::build_page_if_outer_vertical;
use crate::{ExecError, Mode, ModeNest};

/// Typed continuation selected after paragraph lines have been materialized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParagraphEndContinuation {
    End,
    DisplayInterruption,
}

/// Command-owned paragraph completion inputs.
pub(crate) struct ParagraphEnd {
    continuation: ParagraphEndContinuation,
    error_context: Option<String>,
}

impl ParagraphEnd {
    pub(crate) fn end(error_context: Option<String>) -> Self {
        Self {
            continuation: ParagraphEndContinuation::End,
            error_context,
        }
    }

    pub(crate) fn display_interruption() -> Self {
        Self {
            continuation: ParagraphEndContinuation::DisplayInterruption,
            error_context: None,
        }
    }

    pub(crate) fn finish(
        self,
        nest: &mut ModeNest,
        stores: &mut Universe,
        fuel: &mut tex_command::CommandFuel,
    ) -> Result<ParagraphBreakResult, ExecError> {
        let is_display = self.continuation == ParagraphEndContinuation::DisplayInterruption;
        if !is_display && nest.current_mode() != Mode::Horizontal {
            return Ok(ParagraphBreakResult::empty());
        }
        flush_pending_hchars_with_fuel(nest, stores, fuel)?;
        if nest.current_list().is_empty() {
            let _ = commit_current_list(nest, stores, fuel)?;
            if !is_display {
                normal_paragraph(nest, stores);
                build_page_if_outer_vertical(nest, stores)?;
            }
            return Ok(ParagraphBreakResult::empty());
        }
        stores.begin_paragraph_break_dependency_region();
        break_current_paragraph(
            nest,
            stores,
            if is_display {
                WidowPenaltySelector::DisplayInterrupted
            } else {
                WidowPenaltySelector::Ordinary
            },
            !is_display,
            self.error_context,
            fuel,
        )
    }
}

/// TeX82 §1096 `end_graf` with command-owned diagnostic context.
pub(crate) fn end_paragraph_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    command: &tex_command::CommandState,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let context = command.output_open_context(&stores.command_context());
    let result = ParagraphEnd::end(Some(context)).finish(nest, stores, fuel)?;
    if !result.finished_nodes.is_empty() {
        nest.publish_completed_paragraph_nodes(result.finished_nodes);
    }
    Ok(())
}

pub(crate) fn end_paragraph_without_source(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let result = ParagraphEnd::end(None).finish(nest, stores, fuel)?;
    if !result.finished_nodes.is_empty() {
        nest.publish_completed_paragraph_nodes(result.finished_nodes);
    }
    Ok(())
}

pub(crate) fn interrupt_paragraph_for_display(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<ParagraphBreakResult, ExecError> {
    let result = ParagraphEnd::display_interruption().finish(nest, stores, fuel)?;
    if !result.finished_nodes.is_empty() {
        nest.publish_completed_paragraph_nodes(result.finished_nodes.clone());
    }
    Ok(result)
}
