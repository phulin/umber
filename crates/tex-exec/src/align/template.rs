use tex_expand::{ExpansionHooks, ReadRecorder};
use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::Universe;
use tex_state::ids::TokenListId;

use crate::{ExecError, ExecutionStats, ModeNest};

pub(super) fn replay_template<S, R, H>(
    template: TokenListId,
    expects_endv: bool,
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
        if stores.tokens(template).is_empty() {
            return Ok(());
        }
        // TeX82's end_token_list callback ends a u_template even when its
        // final token expands and pops the template below a macro frame. A
        // live-frame marker gives this synchronous replay the same boundary;
        // token-list identity alone is ambiguous for hash-consed templates.
        let replay_marker = input.push_token_list(template, TokenListReplayKind::Inserted);
        let mut stats = ExecutionStats::default();
        loop {
            if !expects_endv && template_finished(input, stores, replay_marker) {
                return Ok(());
            }
            if super::execution::run_one_main_control_token(
                nest, input, stores, recorder, hooks, &mut stats,
            )? {
                assert!(expects_endv, "frozen endv escaped a v-template");
                let finished = input.finish_exhausted_token_list_replay(replay_marker, stores);
                assert!(finished, "frozen endv was not the final v-template token");
                return Ok(());
            }
        }
    })
}

pub(super) fn expand_spanned_column_template_at_span_time<S, R, H>(
    template: TokenListId,
    align_level: usize,
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
    let _ = align_level;
    replay_template(template, false, nest, input, stores, recorder, hooks)
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
