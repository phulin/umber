use tex_expand::{ExpansionHooks, ReadRecorder};
use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::Universe;
use tex_state::ids::TokenListId;
use tex_state::token::Token;

use crate::{ExecError, ExecutionStats, ModeNest};

pub(super) fn replay_template<S, R, H>(
    template: TokenListId,
    sentinel: Option<Token>,
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
        input.push_token_list(template, TokenListReplayKind::Inserted);
        let mut stats = ExecutionStats::default();
        loop {
            if template_finished(input, stores, template, sentinel) {
                return Ok(());
            }
            super::execution::run_one_main_control_token(
                nest, input, stores, recorder, hooks, &mut stats,
            )?;
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
    replay_template(template, None, nest, input, stores, recorder, hooks)
}

fn template_finished<S>(
    input: &mut InputStack<S>,
    stores: &Universe,
    template: TokenListId,
    sentinel: Option<Token>,
) -> bool {
    let Some((frame, replay_kind, index)) = input.current_token_list_frame() else {
        return true;
    };
    if frame != template || replay_kind != TokenListReplayKind::Inserted {
        return false;
    }
    let tokens = stores.tokens(template);
    if sentinel.is_some_and(|token| tokens.get(index).is_some_and(|&next| next == token)) {
        return input.pop_current_token_list_frame(template, TokenListReplayKind::Inserted);
    }
    if index >= tokens.len() {
        return input.pop_current_token_list_frame(template, TokenListReplayKind::Inserted);
    }
    false
}
