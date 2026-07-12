use tex_expand::{ExpansionHooks, ReadRecorder};
use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::ids::TokenListId;
use tex_state::{ExpansionState, Universe};

use crate::{ExecError, ExecutionStats, ModeNest};

pub(super) fn replay_template<S, R, H>(
    template: TokenListId,
    cell_v_template: TokenListId,
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
    {
        // TeX82's end_token_list callback ends a u_template even when its
        // final token expands and pops the template below a macro frame. A
        // live-frame marker gives this synchronous replay the same boundary;
        // token-list identity alone is ambiguous for hash-consed templates.
        let replay_marker = input.push_token_list(template, TokenListReplayKind::Inserted);
        input.begin_alignment_cell(Some(replay_marker), cell_v_template);
        let mut stats = ExecutionStats::default();
        loop {
            if template_finished(input, stores, replay_marker) {
                return Ok(());
            }
            match super::execution::run_one_main_control_token(
                nest, input, stores, recorder, hooks, &mut stats,
            )? {
                super::execution::TemplateStep::Continue => {}
                super::execution::TemplateStep::DeferredOuterRecovery => return Ok(()),
                super::execution::TemplateStep::EndV => {
                    // Malformed preambles can cause the cell terminator to fire
                    // while a u-template replay is still retiring. Preserve the
                    // pending alignment-cell boundary by replaying a fresh frozen
                    // end marker for the cell-body driver instead of panicking.
                    let end = stores.intern_token_list(&[stores.frozen_end_template_token()]);
                    input.push_token_list(end, TokenListReplayKind::Inserted);
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        "\n! Missing alignment template material inserted.\n",
                    );
                    return Ok(());
                }
            }
        }
    }
}

pub(super) fn expand_spanned_column_template_at_span_time<S, R, H>(
    template: TokenListId,
    cell_v_template: TokenListId,
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
    // Architecture §7 makes alignment the only impure kernel: span-time
    // template expansion is the single explicit gullet interleave while the
    // mutable alignment state on the mode nest is live.
    replay_template(
        template,
        cell_v_template,
        nest,
        input,
        stores,
        recorder,
        hooks,
    )
}

fn template_finished<S>(
    input: &mut InputStack<S>,
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
