use tex_expand::get_x_token_with_context;
#[cfg(test)]
mod tests;

use tex_lex::InputStack;
use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionContext, Universe};

use crate::assignments::{flush_pending_hchars, next_non_space_x};
use crate::executor::sync_engine_state;
use crate::{ExecError, ExecutionStats, ModeNest, leave_group};

pub(super) fn execute_noalign(
    _align_level: usize,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    {
        let opener =
            next_non_space_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
                context: "\\noalign group",
            })?;
        if !super::support::is_begin_group(stores, opener) {
            report_missing_left_brace(opener, input, stores);
        }
        stores.enter_group_with_kind(tex_state::GroupKind::NoAlign);
        // TeX scans \noalign in the alignment's own outer list. In
        // particular, a `\prevdepth` assignment must update the prev_depth
        // that the next row's append_to_vlist observes.
        crate::assignments::normal_paragraph(nest, stores);
        scan_noalign_group(nest, input, stores, execution)?;
        // TeX82 §1133's `no_align_group` case of `handle_right_brace` is
        // `end_graf; unsave; align_peek`: a paragraph the body started is
        // line-broken onto the alignment's own vertical list before the
        // group closes (`umber2-usol`).
        crate::assignments::end_paragraph_with_fuel(nest, stores, execution.command_fuel())?;
        leave_group(input, stores, tex_state::GroupKind::NoAlign)?;
        execution.paragraph_group_exited(stores);
        Ok(())
    }
}

fn scan_noalign_group(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let mut stats = ExecutionStats::default();
    let mut brace_depth = 1usize;
    loop {
        sync_engine_state(execution, nest, stores);
        let token = {
            let mut expansion = ExpansionContext::new(stores);
            get_x_token_with_context(input, &mut expansion, execution)?
        }
        .ok_or(ExecError::MissingToken {
            context: "\\noalign closing brace",
        })?;
        let semantic = tex_expand::semantic_token(token);
        if super::support::is_begin_group(stores, semantic) {
            brace_depth += 1;
        }
        if super::support::is_end_group(stores, semantic) {
            brace_depth -= 1;
            if brace_depth == 0 {
                flush_pending_hchars(nest, stores, execution.command_fuel())?;
                return Ok(());
            }
        }
        super::execution::dispatch_and_drain(nest, token, input, stores, execution, &mut stats)?;
    }
}

/// TeX82 §403's `scan_left_brace` for the mandatory brace after `\noalign`.
///
/// §403 reports with `back_error` and then merely *pretends* the offending
/// token was a `{`; it never inserts one. Backing the token up is therefore
/// the whole recovery, and §314 shows it as its own `<to be read again>` line.
fn report_missing_left_brace(opener: Token, input: &mut InputStack, stores: &mut Universe) {
    let opener = TracedTokenWord::pack(opener, OriginId::UNKNOWN);
    crate::error_report::back_error(
        input,
        stores,
        opener,
        "Missing { inserted",
        &[
            "A left brace was mandatory here, so I've put one in.",
            "You might want to delete and/or insert some corrections",
            "so that I will find a matching right brace soon.",
            "(If you're confused by all this, try typing `I}' now.)",
        ],
    );
}
