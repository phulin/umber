use tex_expand::{ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack};
use tex_state::glue::Order;
use tex_state::ids::NodeListId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::Token;
use tex_state::{BoxDimension, Universe};
use tex_typeset::{HpackParams, PackDiagnostic, PackSpec, VpackParams, hpack, vpack, vtop};

use super::*;
use crate::vertical::{
    append_node_to_current_list, append_vertical_contribution, build_page_if_outer_vertical,
    is_outer_vertical,
};
use crate::{ExecError, Mode, ModeNest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoxKind {
    HBox,
    VBox,
    VTop,
}

pub(super) fn execute_make_box<S, H>(
    primitive: UnexpandablePrimitive,
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
    let node = scan_box_node(kind_for_primitive(primitive)?, input, stores, hooks)?;
    append_node_to_current_list(nest, stores, node)?;
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

pub(super) fn execute_setbox<S, H>(
    global: bool,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let index = scan_register_index(input, stores, hooks)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let node = scan_required_box_node(input, stores, hooks)?;
    let node = stores.clone_node_to_epoch(node);
    let list = stores.freeze_node_list(&[node]);
    if global {
        stores.set_box_reg_global(index, list);
    } else {
        stores.set_box_reg(index, list);
    }
    Ok(())
}

pub(super) fn execute_box_dimension_assignment<S, H>(
    primitive: UnexpandablePrimitive,
    global: bool,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let index = scan_register_index(input, stores, hooks)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let value = scan_scaled(input, stores, hooks)?;
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
            let index = scan_register_index(input, stores, hooks)?;
            let id = if primitive == UnexpandablePrimitive::Box {
                stores.take_box_reg(index)
            } else {
                stores.box_reg(index)
            };
            append_box_register(nest, stores, id)?;
        }
        UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnVBox => {
            let index = scan_register_index(input, stores, hooks)?;
            let id = stores.take_box_reg(index);
            append_unboxed(nest, stores, id, primitive)?;
        }
        UnexpandablePrimitive::LastBox => {
            if let Some(node) = nest.current_list_mut().pop_box() {
                let list = stores.freeze_node_list(&[node]);
                stores.set_box_reg(255, list);
            } else {
                let empty = stores.freeze_node_list(&[]);
                stores.set_box_reg(255, empty);
            }
        }
        UnexpandablePrimitive::Raise
        | UnexpandablePrimitive::Lower
        | UnexpandablePrimitive::MoveLeft
        | UnexpandablePrimitive::MoveRight => {
            let amount = scan_scaled(input, stores, hooks)?;
            let mut node = scan_required_box_node(input, stores, hooks)?;
            apply_shift(&mut node, primitive, amount)?;
            append_node_to_current_list(nest, stores, node)?;
        }
        _ => unreachable!("caller restricts box list commands"),
    }
    if primitive != UnexpandablePrimitive::LastBox {
        build_page_if_outer_vertical(nest, stores)?;
    }
    Ok(())
}

pub(super) fn execute_kern_or_skip<S, H>(
    primitive: UnexpandablePrimitive,
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
            let amount = scan_scaled(input, stores, hooks)?;
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
            let spec = scan_glue_id(input, stores, hooks, false)?;
            append_node_to_current_list(
                nest,
                stores,
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                },
            )?;
        }
        UnexpandablePrimitive::VSkip
        | UnexpandablePrimitive::VFil
        | UnexpandablePrimitive::VFill
        | UnexpandablePrimitive::VSs
        | UnexpandablePrimitive::VFilNeg => {
            execute_vertical_skip(primitive, nest, input, stores, hooks)?
        }
        _ => unreachable!("caller restricts kern/skip primitives"),
    }
    Ok(())
}

pub(super) fn execute_hrule<S, H>(
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
        Mode::RestrictedHorizontal => return Err(ExecError::HRuleHereExceptLeaders),
        mode => {
            return Err(ExecError::UnimplementedTypesetting {
                mode,
                token: Token::Cs(stores.intern("hrule")),
                operation: "\\hrule",
            });
        }
    }
    let node = scan_rule_node(input, stores, hooks, UnexpandablePrimitive::HRule)?;
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
            token: Token::Cs(stores.intern("vskip")),
            operation: "\\vskip",
        });
    }
    let spec = match primitive {
        UnexpandablePrimitive::VSkip => scan_glue_id(input, stores, hooks, false)?,
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
        },
    );
    Ok(())
}

pub(super) fn scan_required_box_node<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Node, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let token = next_non_space_x(input, stores, hooks)?
        .ok_or(ExecError::MissingToken { context: "box" })?;
    let Token::Cs(symbol) = token else {
        return Err(ExecError::MissingToken { context: "box" });
    };
    match stores.meaning(symbol) {
        Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::HBox)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VBox)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VTop) => {
            scan_box_node(kind_for_primitive(primitive)?, input, stores, hooks)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
            let index = scan_register_index(input, stores, hooks)?;
            let id = if matches!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
            ) {
                stores.take_box_reg(index)
            } else {
                stores.box_reg(index)
            };
            first_box_node(stores, id).ok_or(ExecError::MissingToken { context: "box" })
        }
        _ => Err(ExecError::MissingToken { context: "box" }),
    }
}

fn scan_box_node<S, H>(
    kind: BoxKind,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Node, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let spec = scan_pack_spec(input, stores, hooks)?;
    let opener = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "box group",
    })?;
    if !is_begin_group(opener) {
        return Err(ExecError::MissingToken {
            context: "box group",
        });
    }
    let mode = if kind == BoxKind::HBox {
        Mode::RestrictedHorizontal
    } else {
        Mode::InternalVertical
    };
    let mut inner = ModeNest::new();
    inner.push(mode);
    scan_box_group(&mut inner, input, stores, hooks)?;
    let level = inner.pop()?;
    let children = stores.freeze_node_list(level.list().nodes());
    let node = match kind {
        BoxKind::HBox => Node::HList(hpack_with_overfull_rule(stores, children, spec)),
        BoxKind::VBox => Node::VList(vpack(stores, children, spec, VpackParams::read(stores)).node),
        BoxKind::VTop => Node::VList(vtop(stores, children, spec, VpackParams::read(stores)).node),
    };
    Ok(node)
}

pub(super) fn hpack_with_overfull_rule(
    stores: &mut Universe,
    children: NodeListId,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = HpackParams::read(stores);
    let mut packed = hpack(stores, children, spec, params);
    if params.overfull_rule.raw() > 0
        && packed
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, PackDiagnostic::Overfull { .. }))
    {
        let mut nodes = stores.nodes(children).to_vec();
        nodes.push(Node::Rule {
            width: Some(params.overfull_rule),
            height: None,
            depth: None,
        });
        packed.node.children = stores.freeze_node_list(&nodes);
    }
    packed.node
}

pub(super) fn scan_box_group<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    loop {
        crate::executor::sync_engine_state::<S, _>(hooks, nest, stores);
        let token = {
            let mut recorder = NoopRecorder;
            get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
        }
        .ok_or(ExecError::MissingToken {
            context: "box closing brace",
        })?;
        if is_end_group(token) {
            flush_pending_hchars(nest, stores)?;
            return Ok(());
        }
        match crate::dispatch_delivered_token(nest, token, input, stores, hooks)? {
            crate::DispatchAction::Continue => {}
            crate::DispatchAction::Shipout(_) => {}
            crate::DispatchAction::End => return Ok(()),
            crate::DispatchAction::NotConsumed => {
                return Err(ExecError::UnimplementedTypesetting {
                    mode: nest.current_mode(),
                    token,
                    operation: "box content",
                });
            }
        }
    }
}

fn scan_pack_spec<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<PackSpec, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    if scan_optional_keyword_x(input, stores, hooks, "to")? {
        Ok(PackSpec::Exactly(scan_scaled(input, stores, hooks)?))
    } else if scan_optional_keyword_x(input, stores, hooks, "spread")? {
        Ok(PackSpec::Spread(scan_scaled(input, stores, hooks)?))
    } else {
        Ok(PackSpec::Natural)
    }
}

fn append_box_register(
    nest: &mut ModeNest,
    stores: &mut Universe,
    id: Option<tex_state::ids::NodeListId>,
) -> Result<(), ExecError> {
    if let Some(node) = first_box_node(stores, id) {
        let node = stores.clone_node_to_epoch(node);
        append_node_to_current_list(nest, stores, node)?;
    }
    Ok(())
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
        (UnexpandablePrimitive::UnHBox, Node::HList(box_node))
        | (UnexpandablePrimitive::UnVBox, Node::VList(box_node)) => {
            let children = stores.clone_node_list_to_epoch(box_node.children);
            for node in stores.nodes(children).to_vec() {
                append_node_to_current_list(nest, stores, node)?;
            }
            Ok(())
        }
        (_, node) => {
            append_node_to_current_list(nest, stores, node)?;
            Ok(())
        }
    }
}

fn first_box_node(stores: &Universe, id: Option<tex_state::ids::NodeListId>) -> Option<Node> {
    let id = id?;
    stores.nodes(id).first().and_then(|node| match node {
        Node::HList(_) | Node::VList(_) => Some(node.clone()),
        _ => None,
    })
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
        UnexpandablePrimitive::Raise | UnexpandablePrimitive::MoveRight => amount,
        UnexpandablePrimitive::Lower | UnexpandablePrimitive::MoveLeft => -amount,
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

fn kind_for_primitive(primitive: UnexpandablePrimitive) -> Result<BoxKind, ExecError> {
    match primitive {
        UnexpandablePrimitive::HBox => Ok(BoxKind::HBox),
        UnexpandablePrimitive::VBox => Ok(BoxKind::VBox),
        UnexpandablePrimitive::VTop => Ok(BoxKind::VTop),
        _ => Err(ExecError::MissingToken { context: "box" }),
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
