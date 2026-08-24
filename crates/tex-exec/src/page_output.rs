//! Input-free TeX.web page-output selection, packaging, and end-job state.

use std::collections::{BTreeMap, BTreeSet};

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{DimenParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign};
use tex_state::node_arena::PageListId;
use tex_state::page::{
    AWFUL_BAD, INF_PENALTY, PageFireUp, PageInsertionStatus, PageInteger, PageMark,
};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_typeset::{INF_BAD, PackSpec, VpackParams};

use crate::ExecError;
use crate::pack_report::ExecutionDiagnosticContext;
use crate::packing_params::vpack;
use crate::splitting::{natural_vlist_size, prune_page_top, vpack_natural};

/// TeX.web's `-1073741824` end-job penalty from `its_all_over`.
const END_JOB_PENALTY: i32 = -AWFUL_BAD - 1;

/// The typed result of TeX82 §1016 packing, before either the default
/// routine or command-owned `\\output` replay begins.
#[derive(Debug)]
pub(crate) enum SelectedPageOutput {
    Default(Node),
    UserRoutine,
}

/// Selects and packages one pending page without accessing input. Main control
/// main control owns the subsequent mode/group transition; command control
/// owns replay of `\\output` itself.
pub(crate) fn select_pending_page_output<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    fire_up: PageFireUp,
    diagnostic_context: ExecutionDiagnosticContext,
) -> Result<SelectedPageOutput, ExecError> {
    prepare_box255(
        stores,
        diagnostic_effects,
        geometry,
        fire_up,
        &diagnostic_context,
    )?;
    let output_is_empty = stores
        .token_parameter(TokParam::OUTPUT)
        .expect("output token parameter is admitted")
        .is_none_or(|output| stores.token_list(output).is_empty());
    if output_is_empty {
        prepend_output_heldover(stores, Vec::new(), true);
        let page = take_box255_node(stores)?;
        stores.clear_page_discards();
        return Ok(SelectedPageOutput::Default(page));
    }
    let dead_cycles = stores.page_integer(PageInteger::DeadCycles);
    if dead_cycles >= stores.int_param(IntParam::MAX_DEAD_CYCLES) {
        report_output_loop(
            stores,
            diagnostic_effects,
            dead_cycles,
            diagnostic_context.output_context.clone(),
        )?;
        prepend_output_heldover(stores, Vec::new(), true);
        let page = take_box255_node(stores)?;
        stores.clear_page_discards();
        return Ok(SelectedPageOutput::Default(page));
    }
    stores.set_page_integer(PageInteger::DeadCycles, dead_cycles + 1);
    Ok(SelectedPageOutput::UserRoutine)
}

/// TeX82 §1026's input-free tail after the command-owned output list has
/// closed.  `output_nodes` is the internal-vertical list built by the routine.
pub(crate) fn resume_page_builder_after_output<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    output_nodes: Vec<Node>,
    diagnostic_context: ExecutionDiagnosticContext,
) -> Result<(), ExecError> {
    if let Some(box255) = stores.copy_box_to_page(255) {
        stores.clear_box_preserving_level(255);
        report_box255_not_emptied(
            stores,
            diagnostic_effects,
            box255,
            diagnostic_context.output_context.clone(),
        )?;
    }
    stores.clear_page_discards();
    prepend_output_heldover(stores, output_nodes, false);
    crate::page_builder::build_page_with_diagnostic_context(
        stores,
        diagnostic_effects,
        &diagnostic_context,
    )
}

pub(crate) fn prepare_box255<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    fire_up: PageFireUp,
    diagnostic_context: &ExecutionDiagnosticContext,
) -> Result<(), ExecError> {
    if let Some(box255) = stores.copy_box_to_page(255) {
        stores.clear_box_preserving_level(255);
        report_box255_not_void(
            stores,
            diagnostic_effects,
            box255,
            Some(&diagnostic_context.output_context),
        )?;
    }

    let split_index = fire_up.best_break().index();
    let page_max_depth = stores.page_max_depth();
    let (page_nodes, mut after_break) = stores.take_current_page_prefix(split_index);
    let output_penalty = output_penalty_and_rewrite_break(stores, &mut after_break, fire_up);
    stores
        .assign_int_param(
            IntParam::OUTPUT_PENALTY,
            output_penalty,
            tex_state::AssignmentScope::Global,
        )
        .expect("output penalty assignment targets admitted state");
    stores.prepend_page_contributions(after_break);
    let distributed = distribute_insertions(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        page_nodes,
    )?;
    update_page_marks_at_fire_up(stores, &distributed.page_nodes);

    let page_list = stores.publish_page_nodes(distributed.page_nodes);
    let packed = vpack(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        page_list,
        PackSpec::Exactly(fire_up.best_size()),
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: Scaled::MAX_DIMEN,
            box_max_depth: page_max_depth,
        },
    );
    let box255 = stores.publish_page_nodes(vec![Node::VList(packed.node)]);
    stores
        .assign_page_box_global(255, box255)
        .expect("output box stays in admitted page storage");
    stores.start_page_after_output();
    for node in distributed.heldover {
        stores.push_current_page_node(node);
    }
    stores.set_page_integer(
        PageInteger::InsertPenalties,
        i32::try_from(distributed.heldover_count).map_err(|_| ExecError::ArithmeticOverflow)?,
    );
    Ok(())
}

fn update_page_marks_at_fire_up<G>(stores: &mut CommandContext<'_, G>, page_nodes: &[Node]) {
    let mut classes = stores.page_mark_classes().collect::<BTreeSet<_>>();
    classes.insert(0);
    for node in page_nodes {
        if let Node::Mark { class, .. } = node {
            classes.insert(*class);
        }
    }

    for class in classes {
        // e-TeX 2.6 `etex.ch` [26.1396] `fire_up_init` first discards the
        // previous top and first marks. The previous bot mark becomes the new
        // top mark unless its token list is empty; an empty bot mark is made
        // null so the sparse mark-class node can eventually disappear.
        let old_bot = stores.page_mark_class_value(PageMark::Bot, class).cloned();
        stores.clear_page_mark_class(PageMark::Top, class);
        stores.clear_page_mark_class(PageMark::First, class);
        // TeX82 §1012 copies class zero's `bot_mark` pointer even when its
        // token list is empty. e-TeX 2.6 `etex.ch` [26.1396] adds the empty
        // list deletion only for sparse mark-class nodes.
        let top = if class == 0 {
            old_bot
        } else {
            old_bot.filter(|tokens| !tokens.is_empty())
        };
        match top.as_ref() {
            Some(top) => stores.set_page_mark_class(PageMark::Top, class, top.clone()),
            None => stores.clear_page_mark_class(PageMark::Bot, class),
        }

        let mut first = None;
        let mut bot = None;
        for node in page_nodes {
            if let Node::Mark {
                class: node_class,
                tokens,
            } = node
                && *node_class == class
            {
                if first.is_none() {
                    first = Some(tokens.clone());
                }
                bot = Some(tokens.clone());
            }
        }

        match (first, bot) {
            (Some(first), Some(bot)) => {
                stores.set_page_mark_class(PageMark::First, class, first);
                stores.set_page_mark_class(PageMark::Bot, class, bot);
            }
            _ => {
                if let Some(top) = top.as_ref() {
                    // e-TeX 2.6 `etex.ch` [26.1397] `fire_up_done`.
                    stores.set_page_mark_class(PageMark::First, class, top.clone());
                    stores.set_page_mark_class(PageMark::Bot, class, top.clone());
                } else {
                    stores.clear_page_mark_class(PageMark::First, class);
                    stores.clear_page_mark_class(PageMark::Bot, class);
                }
            }
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
    accepting: bool,
}

#[derive(Clone)]
struct SplitInsertionContext {
    insertion_start: usize,
    page_index: usize,
    class: u16,
    split_top_skip: tex_state::glue::GlueSpec,
    split_max_depth: Scaled,
    floating_penalty: i32,
}

fn distribute_insertions<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &ExecutionDiagnosticContext,
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
                    nodes: insertion_box_nodes(
                        stores,
                        diagnostic_effects,
                        insertion.class(),
                        diagnostic_context,
                    )?,
                    best_ins_index,
                    status: insertion.status(),
                    accepting: true,
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
                if let Some(queue) = queues.get_mut(&class)
                    && queue.accepting
                {
                    wait = None;
                    let start = queue.nodes.len();
                    queue.nodes.extend(
                        stores
                            .page_node_list(content)
                            .expect("insertion content belongs to the live page arena")
                            .nodes()
                            .iter()
                            .cloned(),
                    );
                    if queue.best_ins_index == index {
                        if let Some(remainder) = split_insertion_remainder(
                            stores,
                            diagnostic_effects,
                            geometry,
                            diagnostic_context,
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
                        package_insertion_box(
                            stores,
                            diagnostic_effects,
                            geometry,
                            diagnostic_context,
                            class,
                            boxed_nodes,
                        );
                        queue.accepting = false;
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

fn insertion_box_nodes<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    class: u16,
    diagnostic_context: &ExecutionDiagnosticContext,
) -> Result<Vec<Node>, ExecError> {
    // TeX82 §1018 calls §993's `ensure_vbox` again here because an output
    // routine or assignment can replace the class register after page setup.
    let Some(list) = crate::page_builder::ensure_insertion_vbox(
        stores,
        diagnostic_effects,
        class,
        diagnostic_context,
    )?
    else {
        return Ok(Vec::new());
    };
    let Some(node) = stores
        .page_node_list(list)
        .expect("insertion box belongs to the live page arena")
        .nodes()
        .first()
    else {
        return Ok(Vec::new());
    };
    match node {
        Node::VList(box_node) => Ok(stores
            .page_node_list(box_node.children)
            .expect("insertion box children belong to the live page arena")
            .nodes()
            .to_vec()),
        Node::HList(_) => unreachable!("ensure_insertion_vbox rejected the hbox"),
        _ => Ok(Vec::new()),
    }
}

fn split_insertion_remainder<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &ExecutionDiagnosticContext,
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
    let content = stores.publish_page_nodes(pruned);
    let size = natural_vlist_size(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        content,
    )?;
    Ok(Some(Node::Ins {
        class: context.class,
        size,
        split_top_skip: context.split_top_skip,
        split_max_depth: context.split_max_depth,
        floating_penalty: context.floating_penalty,
        content,
    }))
}

fn package_insertion_box<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &ExecutionDiagnosticContext,
    class: u16,
    nodes: Vec<Node>,
) {
    let list = stores.publish_page_nodes(nodes);
    let packed = vpack_natural(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        list,
    );
    let boxed = stores.publish_page_nodes(vec![Node::VList(packed)]);
    stores
        .assign_page_box_global(class, boxed)
        .expect("insertion box stays in admitted page storage");
}

pub(crate) fn prepend_output_heldover<G>(
    stores: &mut CommandContext<'_, G>,
    output_nodes: Vec<Node>,
    discard_rewritten_break: bool,
) {
    let (mut heldover, _) = stores.take_current_page_prefix(stores.current_page_len());
    // TeX82 §§994/1012 resume the page builder after `fire_up`; without an
    // output routine that continuation discards the chosen penalty after
    // §1013 rewrites it to `inf_penalty`. Main control defers the
    // output tail to the command-step boundary, so complete that one-token
    // continuation only when the rewritten break is the entire suffix.
    // Material contributed after the fire-up belongs to a later builder
    // invocation (notably §1196's post-display penalty) and must remain behind
    // the sentinel in canonical order. User output retains the sentinel for
    // §1026's ordinary builder resumption.
    if discard_rewritten_break {
        let heldover_is_rewritten_break = heldover.len() == 1
            && matches!(heldover.first(), Some(Node::Penalty(value)) if *value == INF_PENALTY)
            && stores.page_contributions().is_empty();
        let contribution_is_rewritten_break = heldover.is_empty()
            && stores.page_contributions().len() == 1
            && matches!(stores.page_contribution_front(), Some(Node::Penalty(value)) if *value == INF_PENALTY);
        if heldover_is_rewritten_break {
            heldover.clear();
        } else if contribution_is_rewritten_break {
            let _ = stores.pop_page_contribution_front();
        }
    }
    heldover.extend(output_nodes);
    stores.start_page_after_output();
    stores.set_page_integer(PageInteger::InsertPenalties, 0);
    stores.prepend_page_contributions(heldover);
}

fn output_penalty_and_rewrite_break<G>(
    stores: &mut CommandContext<'_, G>,
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

/// TeX.web §1024's `<Explain that too many dead cycles have occurred...>`.
///
/// Page output is driven by the page builder, not by a scanner, so its caller
/// supplies §82's context. Main control renders its live command
/// stack; only the explicit source-free test seam renders the last published
/// input summary.
pub(crate) fn report_output_loop<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    dead_cycles: i32,
    context: String,
) -> Result<(), ExecError> {
    // TeX82 §§1006, 1012, and 1024 print the completed page-break
    // diagnostic before `fire_up` enters this synchronous error dialogue.
    stores.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
    let mut report = stores.print_err("Output loop---");
    report
        .print_int(dead_cycles)
        .print(" consecutive dead cycles")
        .help(&[
            "I've concluded that your \\output is awry; it never does a",
            "\\shipout, so I'm shipping \\box255 out myself. Next time",
            "increase \\maxdeadcycles if you want me to be more patient!",
        ])
        .context(context);
    report.error().jump_out()?;
    Ok(())
}

/// TeX.web §1015's `<Ensure that box 255 is empty before output>`.
fn report_box255_not_void<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    deleted: PageListId,
    error_context: Option<&str>,
) -> Result<(), ExecError> {
    let context = error_context.unwrap_or_default().to_owned();
    stores.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
    let mut report = stores.print_err("");
    report
        .print_esc("box")
        .print("255 is not void")
        .help(&[
            "You shouldn't use \\box255 except in \\output routines.",
            "Proceed, and I'll discard its present contents.",
        ])
        .context(context);
    report.error().jump_out()?;
    report_deleted_box(stores, diagnostic_effects, deleted);
    Ok(())
}

/// TeX.web §1028's `<Ensure that box 255 is empty after output>`.
pub(crate) fn report_box255_not_emptied<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    deleted: PageListId,
    context: String,
) -> Result<(), ExecError> {
    stores.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
    let mut report = stores.print_err("Output routine didn't use all of ");
    report
        .print_esc("box")
        .print_int(255)
        .help(&[
            "Your \\output commands should empty \\box255,",
            "e.g., by saying `\\shipout\\box255'.",
            "Proceed; I'll discard its present contents.",
        ])
        .context(context);
    report.error().jump_out()?;
    report_deleted_box(stores, diagnostic_effects, deleted);
    Ok(())
}

/// TeX82 §199's `box_error` tail after the caller's recoverable error.
fn report_deleted_box<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    deleted: PageListId,
) {
    let dump = crate::node_dump::dump_page_list(
        stores,
        deleted,
        crate::node_dump::DumpConfig::read(stores),
    );
    // §199 enters `begin_diagnostic` after the caller's live §82 report, so
    // with nonpositive `\tracingonline` the deleted box is log-only even in
    // an interactive mode. The next synchronous page-output reporter crosses
    // the diagnostic-effects bridge before printing, preserving this tail's
    // order without changing its canonical selector.
    let mut diagnostic = stores.begin_diagnostic(diagnostic_effects);
    diagnostic
        .print_nl("The following box has been deleted:")
        .print_ln()
        .print_rendered(&dump);
    diagnostic.end(true);
}

pub(crate) fn take_box255_node<G>(stores: &mut CommandContext<'_, G>) -> Result<Node, ExecError> {
    let owner = stores
        .take_box_to_page(255)
        .ok_or(ExecError::MissingToken { context: "box" })?;
    stores
        .page_node_list(owner)
        .expect("box 255 was copied into the live page arena")
        .get(0)
        .map(|node| node.to_owned_with(|id| id))
        .ok_or(ExecError::MissingToken { context: "box" })
}

/// Appends TeX82 §1054's end-job contribution trio to the contribution
/// list: `tail_append(new_null_box); width(tail):=hsize;
/// tail_append(new_glue(fill_glue)); tail_append(new_penalty(-'10000000000))`.
///
/// `tail_append` is a plain list append, so none of §679's `append_to_vlist`
/// baselineskip interposition applies and `prev_depth` is left alone.
pub(crate) fn append_end_job_contributions<G>(stores: &mut CommandContext<'_, G>) {
    let empty = tex_state::node_arena::PageListId::empty();
    stores.append_page_contribution(Node::HList(BoxNode::new(BoxNodeFields {
        width: stores.dimen_param(DimenParam::H_SIZE),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty,
    })));
    let fill = GlueSpec {
        width: Scaled::from_raw(0),
        stretch: Scaled::from_raw(Scaled::UNITY),
        stretch_order: Order::Fill,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    };
    stores.append_page_contribution(Node::Glue {
        spec: fill,
        kind: GlueKind::Normal,
        leader: None,
    });
    stores.append_page_contribution(Node::Penalty(END_JOB_PENALTY));
}

/// TeX82 §1054's `its_all_over` test: `(page_head=page_tail) and (head=tail)
/// and (dead_cycles=0)`.
///
/// §1051's `privileged` has already restricted this to outer vertical mode,
/// where `head`/`tail` *is* the contribution list, so `head=tail` is exactly
/// "no contributions are waiting for `build_page`".
pub(crate) fn job_is_all_over<G>(stores: &CommandContext<'_, G>) -> bool {
    stores.current_page_len() == 0
        && stores.page_contributions().is_empty()
        && stores.page_integer(PageInteger::DeadCycles) == 0
}

#[cfg(test)]
mod tests;
