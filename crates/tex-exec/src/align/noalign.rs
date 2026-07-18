use tex_expand::get_x_token_with_context;
use tex_lex::InputStack;
use tex_state::{ExpansionContext, PrintSink, Universe};

use crate::assignments::{flush_pending_hchars, next_non_space_x};
use crate::executor::sync_engine_state;
use crate::{ExecError, ExecutionStats, ModeNest, leave_group, push_tokens};

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
            report_missing_left_brace_inserted(stores);
            push_tokens(input, stores, [opener]);
        }
        stores.enter_group_with_kind(tex_state::GroupKind::NoAlign);
        // TeX scans \noalign in the alignment's own outer list. In
        // particular, \nointerlineskip must update the prev_depth that the
        // next row's append_to_vlist observes.
        crate::assignments::normal_paragraph(nest, stores);
        scan_noalign_group(nest, input, stores, execution)?;
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
                flush_pending_hchars(nest, stores)?;
                return Ok(());
            }
        }
        super::execution::dispatch_and_drain(nest, token, input, stores, execution, &mut stats)?;
    }
}

fn report_missing_left_brace_inserted(stores: &mut Universe) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "\n! Missing { inserted.\n");
}
