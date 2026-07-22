use tex_lex::{InputStack, TokenListReplayKind};
use tex_state::ids::TokenListId;
use tex_state::{ExpansionState, Universe};

use crate::{ExecError, ExecutionStats, ModeNest};

pub(super) fn replay_template(
    template: TokenListId,
    cell_v_template: TokenListId,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<tex_state::token::TracedTokenWord>, ExecError> {
    {
        #[cfg(feature = "profiling-stats")]
        super::record_template_invocation();
        // TeX82's end_token_list callback ends a u_template even when its
        // final token expands and pops the template below a macro frame. A
        // live-frame marker gives this synchronous replay the same boundary;
        // token-list identity alone is ambiguous for hash-consed templates.
        let replay_marker =
            input.push_token_list(template, TokenListReplayKind::AlignmentUTemplate);
        input.begin_alignment_cell(
            Some(replay_marker),
            cell_v_template,
            stores.execution_group_depth(),
        );
        let mut stats = ExecutionStats::default();
        loop {
            if template_finished(input, stores, replay_marker) {
                return Ok(None);
            }
            match super::execution::run_one_main_control_token(
                nest, input, stores, execution, &mut stats,
            )? {
                super::execution::TemplateStep::Continue => {}
                super::execution::TemplateStep::EndV(command) => return Ok(Some(command)),
            }
        }
    }
}

pub(super) fn expand_spanned_column_template_at_span_time(
    template: TokenListId,
    cell_v_template: TokenListId,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<tex_state::token::TracedTokenWord>, ExecError> {
    // Architecture §7 makes alignment the only impure kernel: span-time
    // template expansion is the single explicit gullet interleave while the
    // mutable alignment state on the mode nest is live.
    replay_template(template, cell_v_template, nest, input, stores, execution)
}

fn template_finished(
    input: &mut InputStack,
    stores: &Universe,
    replay_marker: tex_lex::TokenListReplayMarker,
) -> bool {
    if input.finish_exhausted_token_list_replay(replay_marker, stores) {
        return true;
    }
    if !input.contains_token_list_replay_marker(replay_marker) {
        return true;
    }
    false
}
