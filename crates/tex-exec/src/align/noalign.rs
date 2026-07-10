use tex_expand::{ExpansionHooks, ReadRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack};
use tex_state::{ExpansionContext, PrintSink, Universe};

use crate::assignments::{flush_pending_hchars, next_non_space_x};
use crate::executor::sync_engine_state;
use crate::{ExecError, ExecutionStats, ModeNest, leave_group, push_tokens};

pub(super) fn execute_noalign<S, R, H>(
    _align_level: usize,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    stores.with_hash_only_checkpoints(|stores| {
        let opener = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
            context: "\\noalign group",
        })?;
        if !super::support::is_begin_group(stores, opener) {
            report_missing_left_brace_inserted(stores);
            push_tokens(input, stores, [opener]);
        }
        stores.enter_group_with_kind(tex_state::GroupKind::Simple);
        // TeX scans \noalign in the alignment's own outer list. In
        // particular, \nointerlineskip must update the prev_depth that the
        // next row's append_to_vlist observes.
        crate::assignments::normal_paragraph(nest, stores);
        scan_noalign_group(nest, input, stores, recorder, hooks)?;
        leave_group(input, stores, tex_state::GroupKind::Simple)?;
        Ok(())
    })
}

fn scan_noalign_group<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut stats = ExecutionStats::default();
    let mut brace_depth = 1usize;
    loop {
        sync_engine_state::<S, _>(hooks, nest, stores);
        let token = {
            let mut expansion = ExpansionContext::new(stores);
            get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks)?
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
        super::execution::dispatch_and_drain(
            nest, token, input, stores, recorder, hooks, &mut stats,
        )?;
    }
}

fn report_missing_left_brace_inserted(stores: &mut Universe) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "\n! Missing { inserted.\n");
}
