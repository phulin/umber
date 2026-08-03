//! Source-free paragraph completion transaction for canonical command control.

use tex_state::Universe;
use tex_typeset::linebreak::WidowPenaltySelector;

use super::paragraph::{ParagraphBreakResult, break_current_paragraph, normal_paragraph};
use super::{commit_current_list, flush_pending_hchars_with_fuel};
use crate::vertical::build_page_if_outer_vertical;
use crate::{ExecError, Mode, ModeNest};

/// Typed continuation selected after paragraph lines have been materialized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParagraphEndContinuation {
    End,
    DisplayInterruption,
}

/// Command-owned paragraph completion inputs.
pub(crate) struct CanonicalParagraphEnd {
    continuation: ParagraphEndContinuation,
    error_context: Option<String>,
}

impl CanonicalParagraphEnd {
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
        if is_display {
            stores.begin_paragraph_break_dependency_region();
        }
        if nest.current_list().is_empty() {
            let _ = commit_current_list(nest, stores, fuel)?;
            if !is_display {
                normal_paragraph(nest, stores);
                build_page_if_outer_vertical(nest, stores)?;
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
pub(crate) fn end_canonical_paragraph_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    command: &tex_command::CommandState,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let context = command.output_open_context(&stores.command_context());
    CanonicalParagraphEnd::end(Some(context))
        .finish(nest, stores, fuel)
        .map(|_| ())
}

pub(crate) fn end_canonical_paragraph_without_source(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    CanonicalParagraphEnd::end(None)
        .finish(nest, stores, fuel)
        .map(|_| ())
}

pub(crate) fn interrupt_canonical_paragraph_for_display(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<ParagraphBreakResult, ExecError> {
    CanonicalParagraphEnd::display_interruption().finish(nest, stores, fuel)
}
