use tex_expand::ExpansionHooks;
use tex_lex::{InputSource, InputStack};
use tex_state::glue::Order;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Token, TracedTokenWord};
use tex_state::{BoxDimension, Universe};

use super::*;
use crate::vertical::{
    append_node_to_current_list, append_vertical_contribution, build_page_if_outer_vertical,
    is_outer_vertical,
};
use crate::{ExecError, Mode, ModeNest};

mod leaders;
mod packaging;
mod vsplit;

use leaders::{leader_glue_kind, scan_leader_glue, scan_leader_payload};
pub(crate) use packaging::hpack_with_overfull_rule;
pub(super) use packaging::scan_required_box_node;
use packaging::{
    ScannedBoxValue, first_box_node, kind_for_primitive, scan_box_node, scan_box_value,
    take_last_box,
};
pub(crate) use packaging::{scan_box_group, scan_pack_spec};
use vsplit::scan_vsplit_node;

pub(super) fn execute_make_box<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    _global: bool,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let node = if primitive == UnexpandablePrimitive::VSplit {
        scan_vsplit_node(input, stores, hooks, context)?
    } else {
        Some(scan_box_node(
            kind_for_primitive(primitive)?,
            input,
            stores,
            hooks,
            context,
        )?)
    };
    if let Some(node) = node {
        append_box_node_to_current_list(nest, stores, node)?;
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

pub(crate) fn scan_math_box<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Option<Node>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let node = match primitive {
        UnexpandablePrimitive::HBox | UnexpandablePrimitive::VBox | UnexpandablePrimitive::VTop => {
            Some(scan_box_node(
                kind_for_primitive(primitive)?,
                input,
                stores,
                hooks,
                context,
            )?)
        }
        UnexpandablePrimitive::VSplit => scan_vsplit_node(input, stores, hooks, context)?,
        UnexpandablePrimitive::Box | UnexpandablePrimitive::Copy => {
            let index = scan_register_index(input, stores, hooks, context)?;
            let id = if primitive == UnexpandablePrimitive::Box {
                stores.take_box_reg_same_level(index)
            } else {
                stores.box_reg(index)
            };
            first_box_node(stores, id).map(|node| stores.clone_node_to_epoch(node))
        }
        UnexpandablePrimitive::Raise | UnexpandablePrimitive::Lower => {
            let amount = scan_scaled(input, stores, hooks, context)?;
            let mut node = packaging::scan_required_box_node(input, stores, hooks, context)?;
            apply_shift(&mut node, primitive, amount)?;
            Some(node)
        }
        _ => unreachable!("caller restricts math box commands"),
    };
    let _ = nest;
    Ok(node)
}

pub(super) fn execute_setbox<S, H>(
    global: bool,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let index = scan_register_index(input, stores, hooks, context)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let mut execution = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let (nest, stores) = execution.parts();
    let mut construction = stores.begin_box_build();
    let value = match scan_box_value(Some(nest), input, &mut construction, hooks, context) {
        Ok(Some(ScannedBoxValue::Fresh(node))) => {
            let list = construction.freeze_node_list(&[node]);
            Some(list)
        }
        Ok(Some(ScannedBoxValue::Shared(node))) => {
            let node = construction.clone_node_to_epoch(node);
            let list = construction.freeze_node_list(&[node]);
            Some(list)
        }
        Ok(None) => None,
        Err(err) => return Err(err),
    };
    construction.finish(index, value, global);
    execution.commit();
    Ok(())
}

pub(super) fn execute_box_dimension_assignment<S, H>(
    primitive: UnexpandablePrimitive,
    global: bool,
    context: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let index = scan_register_index(input, stores, hooks, context)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let value = scan_scaled(input, stores, hooks, context)?;
    let dimension = box_dimension(primitive)?;
    if global {
        if let Some(id) = stores.box_reg(index) {
            let epoch_id = stores.clone_node_list_to_epoch(id);
            let mut nodes = stores.nodes(epoch_id).to_vec();
            rewrite_box_dimension(&mut nodes, dimension, value);
            let rewritten = stores.freeze_node_list(&nodes);
            stores.set_box_reg_global(index, rewritten);
        }
    } else {
        stores.set_box_dimension(index, dimension, value);
    }
    Ok(())
}

pub(super) fn execute_box_list_command<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::Box | UnexpandablePrimitive::Copy => {
            let index = scan_register_index(input, stores, hooks, context)?;
            let id = if primitive == UnexpandablePrimitive::Box {
                stores.take_box_reg_same_level(index)
            } else {
                stores.box_reg(index)
            };
            append_box_register(nest, stores, id)?;
        }
        UnexpandablePrimitive::UnHBox
        | UnexpandablePrimitive::UnHCopy
        | UnexpandablePrimitive::UnVBox
        | UnexpandablePrimitive::UnVCopy => {
            let index = scan_register_index(input, stores, hooks, context)?;
            let id = stores.box_reg(index);
            if let Some(node) = first_box_node(stores, id)
                && !unbox_kind_matches(primitive, &node)
            {
                report_incompatible_unbox(stores);
                return Ok(());
            }
            let id = if matches!(
                primitive,
                UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnVBox
            ) {
                stores.take_box_reg_same_level(index)
            } else {
                id
            };
            append_unboxed(nest, stores, id, primitive)?;
        }
        UnexpandablePrimitive::PageDiscards | UnexpandablePrimitive::SplitDiscards => {
            let nodes = if primitive == UnexpandablePrimitive::PageDiscards {
                stores.take_page_discards()
            } else {
                stores.take_split_discards()
            };
            flush_pending_hchars(nest, stores)?;
            for node in nodes {
                if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                    append_vertical_contribution(nest, stores, node);
                } else {
                    nest.current_list_mut().push(node);
                }
            }
        }
        UnexpandablePrimitive::LastBox => {
            if let Some(node) = take_last_box(nest, stores)? {
                append_box_node_to_current_list(nest, stores, node)?;
            }
        }
        UnexpandablePrimitive::Raise
        | UnexpandablePrimitive::Lower
        | UnexpandablePrimitive::MoveLeft
        | UnexpandablePrimitive::MoveRight => {
            let amount = scan_scaled(input, stores, hooks, context)?;
            let mut node = scan_required_box_node(input, stores, hooks, context)?;
            apply_shift(&mut node, primitive, amount)?;
            append_box_node_to_current_list(nest, stores, node)?;
        }
        _ => unreachable!("caller restricts box list commands"),
    }
    // TeX82 routes `\lastbox` back through `box_end`, which immediately
    // invokes the page builder when the box is re-appended in outer vmode.
    // Unboxing alone only splices contributions and does not catch them up.
    if !matches!(
        primitive,
        UnexpandablePrimitive::UnHBox
            | UnexpandablePrimitive::UnHCopy
            | UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy
            | UnexpandablePrimitive::PageDiscards
            | UnexpandablePrimitive::SplitDiscards
    ) {
        build_page_if_outer_vertical(nest, stores)?;
    }
    Ok(())
}

pub(super) fn execute_kern_or_skip<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::Kern => {
            let amount = scan_scaled(input, stores, hooks, context)?;
            append_node_to_current_list(
                nest,
                stores,
                Node::Kern {
                    amount,
                    kind: KernKind::Explicit,
                },
            )?;
        }
        UnexpandablePrimitive::HSkip => {
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                ensure_horizontal_for_character(nest, input, stores)?;
            }
            let spec = scan_glue_id(input, stores, hooks, false, context)?;
            append_node_to_current_list(
                nest,
                stores,
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                    leader: None,
                },
            )?;
        }
        UnexpandablePrimitive::VSkip
        | UnexpandablePrimitive::VFil
        | UnexpandablePrimitive::VFill
        | UnexpandablePrimitive::VSs
        | UnexpandablePrimitive::VFilNeg => {
            execute_vertical_skip(primitive, nest, input, stores, hooks, context)?
        }
        _ => unreachable!("caller restricts kern/skip primitives"),
    }
    Ok(())
}

pub(super) fn execute_leaders<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let leader = scan_leader_payload(input, stores, hooks, context)?;
    let spec = match scan_leader_glue(input, stores, hooks, nest.current_mode(), context) {
        Ok(spec) => spec,
        Err(ExecError::LeadersNotFollowedByProperGlue { .. }) => {
            // TeX.web §1077 backs up the unsuitable command, discards the
            // scanned leader payload, and resumes main control.
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Leaders not followed by proper glue.\nYou should say `\\leaders <box or rule><hskip or vskip>'.\nI'm ignoring these leaders.\n",
            );
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    append_node_to_current_list(
        nest,
        stores,
        Node::Glue {
            spec,
            kind: leader_glue_kind(primitive),
            leader: Some(leader),
        },
    )?;
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

pub(super) fn execute_hrule<S, H>(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match nest.current_mode() {
        Mode::Vertical | Mode::InternalVertical => {}
        Mode::Horizontal => end_paragraph(nest, stores)?,
        Mode::RestrictedHorizontal => {
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! You can't use `\\hrule' here except with leaders.\nTo put a horizontal rule in an hbox or an alignment,\nyou should use \\leaders or \\hrulefill.\n",
            );
            return Ok(());
        }
        mode => {
            return Err(ExecError::UnimplementedTypesetting {
                mode,
                token: Token::Cs(stores.intern("hrule").symbol()),
                origin: OriginId::UNKNOWN,
                operation: "\\hrule",
            });
        }
    }
    let node = scan_rule_node(input, stores, hooks, UnexpandablePrimitive::HRule, context)?;
    append_vertical_contribution(nest, stores, node);
    nest.current_list_mut()
        .set_prev_depth(crate::mode::IGNORE_DEPTH);
    Ok(())
}

pub(super) fn execute_delete_last(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores)?;
    if is_outer_vertical(nest) {
        return execute_delete_last_outer_vertical(primitive, stores);
    }
    if nest.current_mode() == Mode::Vertical && nest.current_list().is_empty() {
        return match primitive {
            UnexpandablePrimitive::UnSkip => Ok(()),
            UnexpandablePrimitive::UnPenalty => Err(ExecError::CannotDeleteFromCurrentPage {
                command: "\\unpenalty",
            }),
            UnexpandablePrimitive::UnKern => Err(ExecError::CannotDeleteFromCurrentPage {
                command: "\\unkern",
            }),
            _ => unreachable!("caller restricts delete_last primitives"),
        };
    }
    let matches_target = matches!(
        (primitive, nest.current_list().nodes().last()),
        (UnexpandablePrimitive::UnSkip, Some(Node::Glue { .. }))
            | (UnexpandablePrimitive::UnPenalty, Some(Node::Penalty(_)))
            | (UnexpandablePrimitive::UnKern, Some(Node::Kern { .. }))
    );
    if matches_target {
        let _ = nest.current_list_mut().pop_last_node();
    }
    Ok(())
}

fn execute_delete_last_outer_vertical(
    primitive: UnexpandablePrimitive,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let Some(tail) = stores.page_contribution_tail() else {
        return match primitive {
            UnexpandablePrimitive::UnSkip => Ok(()),
            UnexpandablePrimitive::UnPenalty => Err(ExecError::CannotDeleteFromCurrentPage {
                command: "\\unpenalty",
            }),
            UnexpandablePrimitive::UnKern => Err(ExecError::CannotDeleteFromCurrentPage {
                command: "\\unkern",
            }),
            _ => unreachable!("caller restricts delete_last primitives"),
        };
    };
    let matches_target = matches!(
        (primitive, tail),
        (UnexpandablePrimitive::UnSkip, Node::Glue { .. })
            | (UnexpandablePrimitive::UnPenalty, Node::Penalty(_))
            | (UnexpandablePrimitive::UnKern, Node::Kern { .. })
    );
    if matches_target {
        let _ = stores.pop_page_contribution_tail();
    }
    Ok(())
}

fn execute_vertical_skip<S, H>(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    if nest.current_mode() == Mode::Horizontal {
        end_paragraph(nest, stores)?;
    }
    if !matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        return Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: Token::Cs(stores.intern("vskip").symbol()),
            origin: OriginId::UNKNOWN,
            operation: "\\vskip",
        });
    }
    let spec = match primitive {
        UnexpandablePrimitive::VSkip => scan_glue_id(input, stores, hooks, false, context)?,
        UnexpandablePrimitive::VFil => stores.intern_glue(infinite_glue(Order::Fil, false, false)),
        UnexpandablePrimitive::VFill => {
            stores.intern_glue(infinite_glue(Order::Fill, false, false))
        }
        UnexpandablePrimitive::VSs => stores.intern_glue(infinite_glue(Order::Fil, false, true)),
        UnexpandablePrimitive::VFilNeg => {
            stores.intern_glue(infinite_glue(Order::Fil, true, false))
        }
        _ => unreachable!("caller restricts vertical skip primitives"),
    };
    append_vertical_contribution(
        nest,
        stores,
        Node::Glue {
            spec,
            kind: GlueKind::Normal,
            leader: None,
        },
    );
    Ok(())
}

fn append_box_register(
    nest: &mut ModeNest,
    stores: &mut Universe,
    id: Option<tex_state::ids::NodeListId>,
) -> Result<(), ExecError> {
    if let Some(node) = first_box_node(stores, id) {
        let node = stores.clone_node_to_epoch(node);
        append_box_node_to_current_list(nest, stores, node)?;
    }
    Ok(())
}

fn append_box_node_to_current_list(
    nest: &mut ModeNest,
    stores: &mut Universe,
    mut node: Node,
) -> Result<(), ExecError> {
    let migrated = if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        extract_box_migrations(stores, &mut node)
    } else {
        Vec::new()
    };
    append_node_to_current_list(nest, stores, node)?;
    for node in migrated {
        append_vertical_contribution(nest, stores, node);
    }
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) {
        nest.current_list_mut().set_space_factor(1000);
    }
    Ok(())
}

fn extract_box_migrations(stores: &mut Universe, node: &mut Node) -> Vec<Node> {
    let Node::HList(boxed) = node else {
        return Vec::new();
    };
    let mut retained = Vec::new();
    let mut migrated = Vec::new();
    for child in stores.nodes(boxed.children) {
        match child.to_owned() {
            child @ (Node::Mark { .. } | Node::Ins { .. }) => migrated.push(child),
            Node::Adjust(list) => {
                migrated.extend(stores.nodes(list).into_iter().map(|node| node.to_owned()));
            }
            child => retained.push(child),
        }
    }
    if !migrated.is_empty() {
        boxed.children = stores.freeze_node_list(&retained);
    }
    migrated
}

fn append_unboxed(
    nest: &mut ModeNest,
    stores: &mut Universe,
    id: Option<tex_state::ids::NodeListId>,
    primitive: UnexpandablePrimitive,
) -> Result<(), ExecError> {
    let Some(node) = first_box_node(stores, id) else {
        return Ok(());
    };
    match (primitive, node) {
        (UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnHCopy, Node::HList(box_node))
        | (UnexpandablePrimitive::UnVBox | UnexpandablePrimitive::UnVCopy, Node::VList(box_node)) =>
        {
            let children = stores.clone_node_list_to_epoch(box_node.children);
            flush_pending_hchars(nest, stores)?;
            for node in stores.nodes(children).to_vec() {
                if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                    append_vertical_contribution(nest, stores, node);
                } else {
                    nest.current_list_mut().push(node);
                }
            }
            Ok(())
        }
        _ => unreachable!("unbox compatibility is validated before register mutation"),
    }
}

fn unbox_kind_matches(primitive: UnexpandablePrimitive, node: &Node) -> bool {
    matches!(
        (primitive, node),
        (
            UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnHCopy,
            Node::HList(_)
        ) | (
            UnexpandablePrimitive::UnVBox | UnexpandablePrimitive::UnVCopy,
            Node::VList(_)
        )
    )
}

fn report_incompatible_unbox(stores: &mut Universe) {
    stores.world_mut().write_text(
        tex_state::PrintSink::TerminalAndLog,
        "\n! Incompatible list can't be unboxed.\nSorry, Pandora. (You sneaky devil.)\nI refuse to unbox an \\hbox in vertical mode or vice versa.\nAnd I can't open any boxes in math mode.\n",
    );
}

fn apply_shift(
    node: &mut Node,
    primitive: UnexpandablePrimitive,
    amount: Scaled,
) -> Result<(), ExecError> {
    let box_node = match node {
        Node::HList(box_node) | Node::VList(box_node) => box_node,
        _ => return Err(ExecError::MissingToken { context: "box" }),
    };
    let delta = match primitive {
        UnexpandablePrimitive::Lower | UnexpandablePrimitive::MoveRight => amount,
        UnexpandablePrimitive::Raise | UnexpandablePrimitive::MoveLeft => -amount,
        _ => unreachable!("caller restricts shift primitives"),
    };
    box_node.shift = box_node
        .shift
        .checked_add(delta)
        .ok_or(ExecError::ArithmeticOverflow)?;
    Ok(())
}

fn rewrite_box_dimension(nodes: &mut [Node], dimension: BoxDimension, value: Scaled) {
    let box_node = match nodes {
        [Node::HList(box_node)] | [Node::VList(box_node)] => box_node,
        _ => return,
    };
    match dimension {
        BoxDimension::Width => box_node.width = value,
        BoxDimension::Height => box_node.height = value,
        BoxDimension::Depth => box_node.depth = value,
    }
}

fn box_dimension(primitive: UnexpandablePrimitive) -> Result<BoxDimension, ExecError> {
    match primitive {
        UnexpandablePrimitive::Wd => Ok(BoxDimension::Width),
        UnexpandablePrimitive::Ht => Ok(BoxDimension::Height),
        UnexpandablePrimitive::Dp => Ok(BoxDimension::Depth),
        _ => Err(ExecError::UnsupportedAssignmentTarget),
    }
}
