//! Source-free paragraph completion transaction for canonical command control.

use tex_state::CommandContext;
use tex_typeset::linebreak::WidowPenaltySelector;

mod hyphenation;
mod runtime;

pub(crate) use hyphenation::{apply_scanned_hyphenation_exceptions, apply_scanned_patterns};

use crate::box_runtime::{commit_current_list, flush_pending_hchars_with_fuel};
use crate::{ExecError, Mode, ModeNest};
pub use runtime::cached_pretolerance_plan;
pub(crate) use runtime::{
    ParagraphBreakResult, break_current_paragraph, display_line_dimensions, normal_paragraph,
    start_paragraph,
};

/// Typed continuation selected after paragraph lines have been materialized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParagraphEndContinuation {
    End,
    DisplayInterruption,
}

/// Command-owned paragraph completion inputs.
pub(crate) struct ParagraphEnd {
    continuation: ParagraphEndContinuation,
    error_context: String,
}

impl ParagraphEnd {
    pub(crate) fn end(error_context: String) -> Self {
        Self {
            continuation: ParagraphEndContinuation::End,
            error_context,
        }
    }

    pub(crate) fn display_interruption(error_context: String) -> Self {
        Self {
            continuation: ParagraphEndContinuation::DisplayInterruption,
            error_context,
        }
    }

    pub(crate) fn finish<G>(
        self,
        nest: &mut ModeNest,
        stores: &mut CommandContext<'_, G>,
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
                crate::vertical::build_page_if_outer_vertical_with_error_context(
                    nest,
                    stores,
                    &self.error_context,
                )?;
            }
            return Ok(ParagraphBreakResult::empty());
        }
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
pub(crate) fn end_paragraph_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    command: &tex_command::CommandState<G>,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let context = command.output_open_context(stores);
    ParagraphEnd::end(context).finish(nest, stores, fuel)?;
    Ok(())
}

pub(crate) fn end_paragraph_with_context<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    fuel: &mut tex_command::CommandFuel,
    error_context: String,
) -> Result<(), ExecError> {
    ParagraphEnd::end(error_context).finish(nest, stores, fuel)?;
    Ok(())
}

pub(crate) fn interrupt_paragraph_for_display<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    fuel: &mut tex_command::CommandFuel,
    error_context: String,
) -> Result<ParagraphBreakResult, ExecError> {
    let result = ParagraphEnd::display_interruption(error_context).finish(nest, stores, fuel)?;
    Ok(result)
}
