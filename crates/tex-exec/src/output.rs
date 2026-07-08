//! TeX.web output routine fire-up and end-of-job cleanup.

use tex_expand::{ExpansionHooks, ReadRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::env::banks::{DimenParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign};
use tex_state::page::{
    EJECT_PENALTY, INF_PENALTY, PageDimension, PageFireUp, PageInteger, PageMark,
};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::Token;
use tex_state::{ExpansionContext, GroupKind, Universe};
use tex_typeset::{INF_BAD, PackSpec, VpackParams, vpack};

use crate::assignments::{self, shipout_node};
use crate::dispatch::dispatch_delivered_token_with_recorder;
use crate::executor::sync_engine_state;
use crate::mode::IGNORE_DEPTH;
use crate::page_builder::build_page;
use crate::{DispatchAction, ExecError, ExecutionStats, Mode, ModeNest, leave_group};

pub(crate) fn drain_pending_output<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    while let Some(fire_up) = stores.page_fire_up() {
        fire_up_page(nest, input, stores, recorder, hooks, stats, fire_up)?;
    }
    Ok(())
}

pub(crate) fn finish_end<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    while !job_is_quiescent(stores) {
        append_end_cleanup_contributions(stores);
        build_page(stores)?;
        drain_pending_output(nest, input, stores, recorder, hooks, stats)?;
    }
    Ok(())
}

fn fire_up_page<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
    fire_up: PageFireUp,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    prepare_box255(stores, fire_up)?;
    let output = stores.tok_param(TokParam::OUTPUT);
    if stores.tokens(output).is_empty() {
        let node = take_box255_node(stores)?;
        let artifact = shipout_node(node, stores, recorder)?;
        stats.shipped_artifacts.push(artifact);
        build_page(stores)?;
        return Ok(());
    }

    let dead_cycles = stores.page_integer(PageInteger::DeadCycles);
    let max_dead_cycles = stores.int_param(IntParam::MAX_DEAD_CYCLES);
    if dead_cycles >= max_dead_cycles {
        return Err(ExecError::OutputLoop { dead_cycles });
    }
    stores.set_page_integer(PageInteger::DeadCycles, dead_cycles.saturating_add(1));
    run_output_routine(nest, input, stores, recorder, hooks, stats, output)?;
    build_page(stores)?;
    Ok(())
}

fn prepare_box255(stores: &mut Universe, fire_up: PageFireUp) -> Result<(), ExecError> {
    if stores.box_reg(255).is_some() {
        let _ = stores.take_box_reg_same_level(255);
        return Err(ExecError::Box255NotVoidBeforeOutput);
    }

    stores.set_page_integer(PageInteger::InsertPenalties, 0);
    let split_index = fire_up.best_break().index();
    let page_max_depth = stores.page_max_depth();
    let (page_nodes, mut after_break) = stores.take_current_page_prefix(split_index);
    let output_penalty = output_penalty_and_rewrite_break(stores, &mut after_break);
    stores.set_int_param_global(IntParam::OUTPUT_PENALTY, output_penalty);
    stores.prepend_page_contributions(after_break);
    update_page_marks_at_fire_up(stores, &page_nodes);

    let page_list = stores.freeze_node_list(&page_nodes);
    let packed = vpack(
        stores,
        page_list,
        PackSpec::Exactly(fire_up.best_size()),
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: Scaled::MAX_DIMEN,
            box_max_depth: page_max_depth,
        },
    );
    let box255 = stores.freeze_node_list(&[Node::VList(packed.node)]);
    stores.set_box_reg_global(255, box255);
    stores.start_new_page();
    Ok(())
}

fn update_page_marks_at_fire_up(stores: &mut Universe, page_nodes: &[Node]) {
    let top = stores.page_mark(PageMark::Bot);
    stores.set_page_mark(PageMark::Top, top);

    let mut first = None;
    let mut bot = None;
    for node in page_nodes {
        if let Node::Mark { class: 0, tokens } = node {
            if first.is_none() {
                first = Some(*tokens);
            }
            bot = Some(*tokens);
        }
    }

    match (first, bot) {
        (Some(first), Some(bot)) => {
            stores.set_page_mark(PageMark::First, first);
            stores.set_page_mark(PageMark::Bot, bot);
        }
        _ => {
            stores.set_page_mark(PageMark::First, top);
            stores.set_page_mark(PageMark::Bot, top);
        }
    }
}

fn output_penalty_and_rewrite_break(stores: &mut Universe, after_break: &mut Vec<Node>) -> i32 {
    if let Some(Node::Penalty(value)) = after_break.first_mut() {
        let penalty = *value;
        *value = INF_PENALTY;
        return penalty;
    }

    if let Some(Node::Penalty(penalty)) = stores.page_contribution_front().cloned() {
        let _ = stores.pop_page_contribution_front();
        after_break.push(Node::Penalty(INF_PENALTY));
        return penalty;
    }

    INF_PENALTY
}

fn run_output_routine<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
    output: tex_state::ids::TokenListId,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let base_input_depth = input.depth();
    stores.enter_group_with_kind(GroupKind::Simple);
    nest.push(Mode::InternalVertical);
    nest.current_list_mut().set_prev_depth(IGNORE_DEPTH);
    let output_frame = delimited_output_tokens(stores, output);
    input.push_token_list(output_frame, TokenListReplayKind::OutputRoutine);

    while input.contains_token_list_frame(output_frame, TokenListReplayKind::OutputRoutine)
        || input.depth() > base_input_depth
    {
        if pop_finished_output_frame(input, stores, output_frame) {
            break;
        }
        sync_engine_state::<S, _>(hooks, nest, stores);
        let Some(token) = ({
            let mut expansion = ExpansionContext::new(stores);
            get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks)?
        }) else {
            if !input.contains_token_list_frame(output_frame, TokenListReplayKind::OutputRoutine) {
                break;
            }
            return Err(ExecError::MissingToken {
                context: "output routine",
            });
        };
        match dispatch_delivered_token_with_recorder(nest, token, input, stores, recorder, hooks)? {
            DispatchAction::Continue => {}
            DispatchAction::Shipout(artifact) => stats.shipped_artifacts.push(artifact),
            DispatchAction::End => {
                return Err(ExecError::UnimplementedTypesetting {
                    mode: nest.current_mode(),
                    token,
                    operation: "\\end inside \\output",
                });
            }
            DispatchAction::NotConsumed => {
                return Err(ExecError::UnimplementedTypesetting {
                    mode: nest.current_mode(),
                    token,
                    operation: "output routine",
                });
            }
        }
    }

    assignments::flush_pending_hchars(nest, stores)?;
    let output_level = nest.pop()?;
    leave_group(input, stores, GroupKind::Simple)?;
    stores.set_page_integer(PageInteger::InsertPenalties, 0);
    if stores.box_reg(255).is_some() {
        let _ = stores.take_box_reg_same_level(255);
        return Err(ExecError::OutputRoutineBox255NotVoid);
    }
    stores.prepend_page_contributions(output_level.list().nodes().to_vec());
    Ok(())
}

fn pop_finished_output_frame<S>(
    input: &mut InputStack<S>,
    stores: &Universe,
    output: tex_state::ids::TokenListId,
) -> bool {
    let Some((token_list, replay_kind, index)) = input.current_token_list_frame() else {
        return false;
    };
    if token_list == output
        && replay_kind == TokenListReplayKind::OutputRoutine
        && index >= stores.tokens(token_list).len()
    {
        input.pop_current_token_list_frame(token_list, replay_kind);
        return true;
    }
    false
}

fn delimited_output_tokens(
    stores: &mut Universe,
    output: tex_state::ids::TokenListId,
) -> tex_state::ids::TokenListId {
    let mut tokens = stores.tokens(output).to_vec();
    let relax = stores.intern("relax");
    tokens.push(Token::Cs(relax));
    stores.intern_token_list(&tokens)
}

fn take_box255_node(stores: &mut Universe) -> Result<Node, ExecError> {
    let id = stores
        .take_box_reg_same_level(255)
        .ok_or(ExecError::MissingToken { context: "box" })?;
    stores
        .nodes(id)
        .first()
        .cloned()
        .ok_or(ExecError::MissingToken { context: "box" })
}

fn append_end_cleanup_contributions(stores: &mut Universe) {
    let empty = stores.freeze_node_list(&[]);
    stores.append_page_contribution(Node::HList(BoxNode::new(BoxNodeFields {
        width: stores.dimen_param(DimenParam::H_SIZE),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty,
    })));
    let fill = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(0),
        stretch: Scaled::from_raw(Scaled::UNITY),
        stretch_order: Order::Fill,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    stores.append_page_contribution(Node::Glue {
        spec: fill,
        kind: GlueKind::Normal,
    });
    stores.append_page_contribution(Node::Penalty(EJECT_PENALTY));
}

fn job_is_quiescent(stores: &Universe) -> bool {
    stores.current_page_nodes().is_empty()
        && stores.page_contributions().is_empty()
        && stores.page_fire_up().is_none()
        && stores.page_dimension(PageDimension::Total).raw() == 0
        && stores.page_integer(PageInteger::DeadCycles) == 0
}
