//! TeX.web output routine fire-up and end-of-job cleanup.

use std::collections::BTreeMap;

use tex_expand::{ExpansionHooks, ReadRecorder};
use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::env::banks::{DimenParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign};
use tex_state::page::{
    EJECT_PENALTY, INF_PENALTY, PageDimension, PageFireUp, PageInsertionStatus, PageInteger,
    PageMark,
};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::Token;
use tex_state::{GroupKind, Universe};
use tex_typeset::{INF_BAD, PackSpec, VpackParams};

use crate::assignments::{self, shipout_node};
use crate::executor::{MainControlExit, run_main_control_until};
use crate::mode::IGNORE_DEPTH;
use crate::packing_params::vpack;
use crate::page_builder::build_page;
use crate::splitting::{natural_vlist_size, prune_page_top, vpack_natural};
use crate::{ExecError, ExecutionStats, Mode, ModeNest, leave_group, push_traced_tokens};

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
        prepend_output_heldover(stores, Vec::new());
        let node = take_box255_node(stores)?;
        let artifact = shipout_node(node, input, stores, recorder)?;
        let _ = artifact;
        build_page(stores)?;
        return Ok(());
    }

    let dead_cycles = stores.page_integer(PageInteger::DeadCycles);
    let max_dead_cycles = stores.int_param(IntParam::MAX_DEAD_CYCLES);
    if dead_cycles >= max_dead_cycles {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            &format!(
                "\n! Output loop---{dead_cycles} consecutive dead cycles.\nI've concluded that your \\output is awry; it never does a\n\\shipout, so I'm shipping \\box255 out myself. Next time\nincrease \\maxdeadcycles if you want me to be more patient!\n"
            ),
        );
        prepend_output_heldover(stores, Vec::new());
        let node = take_box255_node(stores)?;
        let _artifact = shipout_node(node, input, stores, recorder)?;
        build_page(stores)?;
        return Ok(());
    }
    stores.set_page_integer(PageInteger::DeadCycles, dead_cycles.saturating_add(1));
    run_output_routine(nest, input, stores, recorder, hooks, stats, output)?;
    build_page(stores)?;
    Ok(())
}

fn prepare_box255(stores: &mut Universe, fire_up: PageFireUp) -> Result<(), ExecError> {
    if stores.box_reg(255).is_some() {
        let _ = stores.take_box_reg_same_level(255);
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! \\box255 is not void.\nYou shouldn't use \\box255 except in \\output routines.\nProceed, and I'll discard its present contents.\n",
        );
    }

    let split_index = fire_up.best_break().index();
    let page_max_depth = stores.page_max_depth();
    let (page_nodes, mut after_break) = stores.take_current_page_prefix(split_index);
    let output_penalty = output_penalty_and_rewrite_break(stores, &mut after_break, fire_up);
    stores.set_int_param_global(IntParam::OUTPUT_PENALTY, output_penalty);
    stores.prepend_page_contributions(after_break);
    let distributed = distribute_insertions(stores, page_nodes)?;
    update_page_marks_at_fire_up(stores, &distributed.page_nodes);

    let page_list = stores.freeze_node_list(&distributed.page_nodes);
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
    for node in distributed.heldover {
        stores.push_current_page_node(node);
    }
    stores.set_page_integer(
        PageInteger::InsertPenalties,
        i32::try_from(distributed.heldover_count).map_err(|_| ExecError::ArithmeticOverflow)?,
    );
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

struct DistributedInsertions {
    page_nodes: Vec<Node>,
    heldover: Vec<Node>,
    heldover_count: usize,
}

struct InsertionQueue {
    nodes: Vec<Node>,
    best_ins_index: usize,
    status: PageInsertionStatus,
}

#[derive(Clone, Copy)]
struct SplitInsertionContext {
    insertion_start: usize,
    page_index: usize,
    class: u16,
    split_top_skip: tex_state::ids::GlueId,
    split_max_depth: Scaled,
    floating_penalty: i32,
}

fn distribute_insertions(
    stores: &mut Universe,
    page_nodes: Vec<Node>,
) -> Result<DistributedInsertions, ExecError> {
    if stores.int_param(IntParam::HOLDING_INSERTS) > 0 {
        return Ok(DistributedInsertions {
            page_nodes,
            heldover: Vec::new(),
            heldover_count: 0,
        });
    }

    let mut queues = BTreeMap::new();
    let insertions = stores.page_insertions().to_vec();
    for insertion in insertions {
        if let Some(best_ins_index) = insertion.best_ins_index() {
            queues.insert(
                insertion.class(),
                InsertionQueue {
                    nodes: insertion_box_nodes(stores, insertion.class())?,
                    best_ins_index,
                    status: insertion.status(),
                },
            );
        }
    }

    let mut retained = Vec::new();
    let mut heldover = Vec::new();
    let mut heldover_count = 0usize;
    for (index, node) in page_nodes.into_iter().enumerate() {
        match node {
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => {
                let mut wait = Some(Node::Ins {
                    class,
                    size,
                    split_top_skip,
                    split_max_depth,
                    floating_penalty,
                    content,
                });
                if let Some(queue) = queues.get_mut(&class) {
                    wait = None;
                    let start = queue.nodes.len();
                    queue.nodes.extend(
                        stores
                            .nodes(content)
                            .into_iter()
                            .map(|node| node.to_owned()),
                    );
                    if queue.best_ins_index == index {
                        if let Some(remainder) = split_insertion_remainder(
                            stores,
                            queue,
                            SplitInsertionContext {
                                insertion_start: start,
                                page_index: index,
                                class,
                                split_top_skip,
                                split_max_depth,
                                floating_penalty,
                            },
                        )? {
                            heldover.push(remainder);
                            heldover_count += 1;
                        }
                        let boxed_nodes = std::mem::take(&mut queue.nodes);
                        package_insertion_box(stores, class, boxed_nodes);
                    }
                }
                if let Some(node) = wait {
                    heldover.push(node);
                    heldover_count += 1;
                }
            }
            node => retained.push(node),
        }
    }

    Ok(DistributedInsertions {
        page_nodes: retained,
        heldover,
        heldover_count,
    })
}

fn insertion_box_nodes(stores: &mut Universe, class: u16) -> Result<Vec<Node>, ExecError> {
    let Some(list) = stores.box_reg(class) else {
        return Ok(Vec::new());
    };
    let Some(node) = stores.nodes(list).first().map(|node| node.to_owned()) else {
        return Ok(Vec::new());
    };
    match node {
        Node::VList(box_node) => {
            let children = stores.clone_node_list_to_epoch(box_node.children);
            Ok(stores.nodes(children).to_vec())
        }
        Node::HList(_) => Err(ExecError::UnsupportedShipoutNode {
            node: "hbox insertion box",
        }),
        _ => Ok(Vec::new()),
    }
}

fn split_insertion_remainder(
    stores: &mut Universe,
    queue: &mut InsertionQueue,
    context: SplitInsertionContext,
) -> Result<Option<Node>, ExecError> {
    let PageInsertionStatus::SplitUp {
        broken_ins_index,
        broken_at: Some(broken_at),
    } = queue.status
    else {
        return Ok(None);
    };
    if broken_ins_index != context.page_index {
        return Ok(None);
    }

    let split_at = context
        .insertion_start
        .checked_add(broken_at)
        .ok_or(ExecError::ArithmeticOverflow)?
        .min(queue.nodes.len());
    let remainder = queue.nodes.split_off(split_at);
    let pruned = prune_page_top(stores, remainder, context.split_top_skip);
    if pruned.is_empty() {
        return Ok(None);
    }
    let content = stores.freeze_node_list(&pruned);
    let size = natural_vlist_size(stores, content)?;
    Ok(Some(Node::Ins {
        class: context.class,
        size,
        split_top_skip: context.split_top_skip,
        split_max_depth: context.split_max_depth,
        floating_penalty: context.floating_penalty,
        content,
    }))
}

fn package_insertion_box(stores: &mut Universe, class: u16, nodes: Vec<Node>) {
    let list = stores.freeze_node_list(&nodes);
    let packed = vpack_natural(stores, list);
    let boxed = stores.freeze_node_list(&[Node::VList(packed)]);
    stores.set_box_reg_global(class, boxed);
}

fn prepend_output_heldover(stores: &mut Universe, output_nodes: Vec<Node>) {
    let (mut heldover, _) = stores.take_current_page_prefix(stores.current_page_len());
    heldover.extend(output_nodes);
    stores.start_new_page();
    stores.set_page_integer(PageInteger::InsertPenalties, 0);
    stores.prepend_page_contributions(heldover);
}

fn output_penalty_and_rewrite_break(
    stores: &mut Universe,
    after_break: &mut Vec<Node>,
    fire_up: PageFireUp,
) -> i32 {
    if let Some(Node::Penalty(value)) = after_break.first_mut() {
        let penalty = *value;
        *value = INF_PENALTY;
        return penalty;
    }

    if fire_up.trigger() == fire_up.best_break()
        && let Some(Node::Penalty(penalty)) = stores.page_contribution_front().cloned()
    {
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
    let mut transaction = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let mut replay = None;
    let result = {
        let (nest, stores) = transaction.parts();
        run_output_routine_inner(
            nest,
            input,
            stores,
            recorder,
            hooks,
            stats,
            output,
            &mut replay,
        )
    };
    if result.is_ok() {
        transaction.commit();
    } else if let Some(replay) = replay {
        let _ = input.abort_token_list_replay(replay);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_output_routine_inner<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
    output: tex_state::ids::TokenListId,
    replay: &mut Option<tex_lex::TokenListReplayMarker>,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    stores.enter_group_with_kind(GroupKind::Simple);
    nest.push(Mode::InternalVertical);
    nest.current_list_mut().set_prev_depth(IGNORE_DEPTH);
    let output_frame = delimited_output_tokens(stores, output);
    *replay = Some(input.push_token_list(output_frame, TokenListReplayKind::OutputRoutine));

    match run_main_control_until(
        nest,
        input,
        stores,
        recorder,
        hooks,
        stats,
        |input, stores| pop_finished_output_frame(input, stores, output_frame),
    )? {
        MainControlExit::Stopped => {}
        MainControlExit::EndOfInput => {
            if !input.contains_token_list_frame(output_frame, TokenListReplayKind::OutputRoutine) {
                // Expansion can discard the exhausted output frame while
                // looking for the next token; that is still a normal stop.
            } else {
                return Err(ExecError::MissingToken {
                    context: "output routine",
                });
            }
        }
        MainControlExit::End { token } => {
            // TeX's off_save recovery closes the output group before a stop
            // command can be reconsidered by outer vertical main control.
            // Preserve the command for that reconsideration while completing
            // the ordinary output-routine teardown below.
            push_traced_tokens(input, stores, [token]);
        }
        MainControlExit::NotConsumed { token } => {
            return Err(ExecError::UnimplementedTypesetting {
                mode: nest.current_mode(),
                token: tex_expand::semantic_token(token),
                origin: token.origin(),
                operation: "output routine",
            });
        }
    }

    assignments::flush_pending_hchars(nest, stores)?;
    let output_level = nest.pop()?;
    leave_group(input, stores, GroupKind::Simple)?;
    if stores.box_reg(255).is_some() {
        let _ = stores.take_box_reg_same_level(255);
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Output routine didn't use all of \\box255.\nYour \\output commands should empty \\box255,\ne.g., by saying `\\shipout\\box255'.\nProceed; I'll discard its present contents.\n",
        );
    }
    prepend_output_heldover(stores, output_level.list().nodes().to_vec());
    Ok(())
}

fn pop_finished_output_frame<S>(
    input: &mut InputStack<S>,
    stores: &Universe,
    output: tex_state::ids::TokenListId,
) -> bool {
    while let Some((token_list, replay_kind, index)) = input.current_token_list_frame() {
        if index < stores.tokens(token_list).len() {
            return false;
        }
        input.pop_current_token_list_frame(token_list, replay_kind);
        if token_list == output && replay_kind == TokenListReplayKind::OutputRoutine {
            return true;
        }
    }
    false
}

fn delimited_output_tokens(
    stores: &mut Universe,
    output: tex_state::ids::TokenListId,
) -> tex_state::ids::TokenListId {
    let mut tokens = stores.tokens(output).to_vec();
    let relax = stores.intern("relax");
    tokens.push(Token::Cs(relax.symbol()));
    stores.intern_token_list(&tokens)
}

fn take_box255_node(stores: &mut Universe) -> Result<Node, ExecError> {
    let id = stores
        .take_box_reg_same_level(255)
        .ok_or(ExecError::MissingToken { context: "box" })?;
    stores
        .nodes(id)
        .first()
        .map(|node| node.to_owned())
        .ok_or(ExecError::MissingToken { context: "box" })
}

fn append_end_cleanup_contributions(stores: &mut Universe) {
    let empty = stores.freeze_node_list(&[]);
    stores.append_page_contribution(Node::HList(BoxNode::new(BoxNodeFields {
        width: stores.dimen_param(DimenParam::H_SIZE),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
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
        leader: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::page::PageBreak;

    fn fire_up(best_break: usize, trigger: usize) -> PageFireUp {
        PageFireUp::new(
            PageBreak::new(best_break),
            Scaled::from_raw(0),
            PageBreak::new(trigger),
        )
    }

    #[test]
    fn earlier_break_preserves_unrelated_pending_penalty() {
        let mut stores = Universe::new();
        stores.append_page_contribution(Node::Penalty(EJECT_PENALTY));
        let glue = stores.intern_glue(GlueSpec {
            width: Scaled::from_raw(0),
            stretch: Scaled::from_raw(0),
            stretch_order: Order::Normal,
            shrink: Scaled::from_raw(0),
            shrink_order: Order::Normal,
        });
        let chosen_break = Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        };
        let mut after_break = vec![chosen_break.clone()];

        let penalty =
            output_penalty_and_rewrite_break(&mut stores, &mut after_break, fire_up(1, 2));

        assert_eq!(penalty, INF_PENALTY);
        assert_eq!(after_break, [chosen_break]);
        assert_eq!(stores.page_contributions().len(), 1);
        assert_eq!(
            stores.page_contributions().front(),
            Some(&Node::Penalty(EJECT_PENALTY))
        );
    }

    #[test]
    fn chosen_pending_penalty_is_rewritten() {
        let mut stores = Universe::new();
        stores.append_page_contribution(Node::Penalty(EJECT_PENALTY));
        let mut after_break = Vec::new();

        let penalty =
            output_penalty_and_rewrite_break(&mut stores, &mut after_break, fire_up(1, 1));

        assert_eq!(penalty, EJECT_PENALTY);
        assert_eq!(after_break, [Node::Penalty(INF_PENALTY)]);
        assert!(stores.page_contributions().is_empty());
    }
}
