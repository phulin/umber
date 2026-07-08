use tex_expand::{ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack};
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::GlueSpec;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{BoxDimension, Universe};
use tex_typeset::{HpackParams, PackSpec, VpackParams, hpack, vpack, vtop};

use super::*;
use crate::mode::IGNORE_DEPTH;
use crate::{ExecError, Mode, ModeNest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoxKind {
    HBox,
    VBox,
    VTop,
}

pub(crate) fn try_append_character(
    nest: &mut ModeNest,
    token: Token,
    stores: &mut Universe,
) -> Result<bool, ExecError> {
    match (nest.current_mode(), token) {
        (Mode::RestrictedHorizontal | Mode::Horizontal, Token::Char { ch, cat }) => {
            if cat == Catcode::Space {
                let id = stores.intern_glue(GlueSpec::ZERO);
                nest.current_list_mut().push(Node::Glue {
                    spec: id,
                    kind: GlueKind::Normal,
                });
            } else {
                nest.current_list_mut().push(Node::Char {
                    font: stores.current_font(),
                    ch,
                });
            }
            Ok(true)
        }
        _ => Ok(false),
    }
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
            nest.current_list_mut().push(Node::Kern {
                amount,
                kind: KernKind::Explicit,
            });
        }
        UnexpandablePrimitive::HSkip | UnexpandablePrimitive::VSkip => {
            let spec = scan_glue_id(input, stores, hooks, false)?;
            nest.current_list_mut().push(Node::Glue {
                spec,
                kind: GlueKind::Normal,
            });
        }
        _ => unreachable!("caller restricts kern/skip primitives"),
    }
    Ok(())
}

fn scan_required_box_node<S, H>(
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
        BoxKind::HBox => Node::HList(hpack(stores, children, spec, HpackParams::read(stores)).node),
        BoxKind::VBox => Node::VList(vpack(stores, children, spec, VpackParams::read(stores)).node),
        BoxKind::VTop => Node::VList(vtop(stores, children, spec, VpackParams::read(stores)).node),
    };
    Ok(node)
}

fn scan_box_group<S, H>(
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
        let token = {
            let mut recorder = NoopRecorder;
            get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
        }
        .ok_or(ExecError::MissingToken {
            context: "box closing brace",
        })?;
        if is_end_group(token) {
            return Ok(());
        }
        match crate::dispatch_delivered_token(nest, token, input, stores, hooks)? {
            crate::DispatchAction::Continue => {}
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
            nest.current_list_mut()
                .append(stores.nodes(children).iter().cloned());
            Ok(())
        }
        (_, node) => {
            append_node_to_current_list(nest, stores, node)?;
            Ok(())
        }
    }
}

fn append_node_to_current_list(
    nest: &mut ModeNest,
    stores: &mut Universe,
    node: Node,
) -> Result<(), ExecError> {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        append_node_to_vertical_list(nest, stores, node)
    } else {
        nest.current_list_mut().push(node);
        Ok(())
    }
}

fn append_node_to_vertical_list(
    nest: &mut ModeNest,
    stores: &mut Universe,
    node: Node,
) -> Result<(), ExecError> {
    let Some((height, depth)) = vertical_baseline_dimensions(&node) else {
        nest.current_list_mut().push(node);
        return Ok(());
    };
    if let Some(prev_depth) = nest.current_list().prev_depth()
        && prev_depth.raw() > IGNORE_DEPTH.raw()
    {
        let baseline = stores.glue(stores.glue_param(GlueParam::BASELINE_SKIP));
        let requested = baseline
            .width
            .checked_sub(prev_depth)
            .and_then(|value| value.checked_sub(height))
            .ok_or(ExecError::ArithmeticOverflow)?;
        let (spec, kind) =
            if requested.raw() < stores.dimen_param(DimenParam::LINE_SKIP_LIMIT).raw() {
                (stores.glue_param(GlueParam::LINE_SKIP), GlueKind::LineSkip)
            } else {
                (
                    stores.intern_glue(GlueSpec {
                        width: requested,
                        stretch: baseline.stretch,
                        stretch_order: baseline.stretch_order,
                        shrink: baseline.shrink,
                        shrink_order: baseline.shrink_order,
                    }),
                    GlueKind::BaselineSkip,
                )
            };
        nest.current_list_mut().push(Node::Glue { spec, kind });
    }
    let list = nest.current_list_mut();
    list.push(node);
    list.set_prev_depth(depth);
    Ok(())
}

fn vertical_baseline_dimensions(node: &Node) -> Option<(Scaled, Scaled)> {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => Some((box_node.height, box_node.depth)),
        Node::Rule { height, depth, .. } => Some((
            height.unwrap_or(Scaled::from_raw(0)),
            depth.unwrap_or(Scaled::from_raw(0)),
        )),
        _ => None,
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
