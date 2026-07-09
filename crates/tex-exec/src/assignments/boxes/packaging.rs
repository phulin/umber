use tex_expand::{ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack};
use tex_state::ids::NodeListId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::Node;
use tex_state::{GroupKind, Universe};
use tex_typeset::{PackDiagnostic, PackSpec};

use crate::packing_params::{hpack, hpack_params, vpack, vpack_params, vtop};
use crate::{ExecError, Mode, ModeNest, leave_group};

use super::super::{
    flush_pending_hchars, is_begin_group, is_end_group, next_non_space_x, scan_optional_keyword_x,
    scan_register_index, scan_scaled,
};
use super::vsplit::scan_vsplit_node;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BoxKind {
    HBox,
    VBox,
    VTop,
}

pub(in crate::assignments) fn scan_required_box_node<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Node, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    scan_box_value(input, stores, hooks)?.ok_or(ExecError::MissingToken { context: "box" })
}

pub(super) fn scan_box_value<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Option<Node>, ExecError>
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
            scan_box_node(kind_for_primitive(primitive)?, input, stores, hooks).map(Some)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
            let index = scan_register_index(input, stores, hooks)?;
            let id = if matches!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
            ) {
                stores.take_box_reg_same_level(index)
            } else {
                stores.box_reg(index)
            };
            Ok(first_box_node(stores, id))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit) => {
            scan_vsplit_node(input, stores, hooks)
        }
        _ => Err(ExecError::MissingToken { context: "box" }),
    }
}

pub(super) fn scan_box_node<S, H>(
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
    stores.enter_group_with_kind(GroupKind::Simple);
    let mode = if kind == BoxKind::HBox {
        Mode::RestrictedHorizontal
    } else {
        Mode::InternalVertical
    };
    let mut inner = ModeNest::new();
    inner.push(mode);
    scan_box_group(&mut inner, input, stores, hooks)?;
    let level = inner.pop()?;
    let nodes = if kind == BoxKind::HBox {
        crate::math::finish_math_lists(stores, level.list().nodes(), false)
    } else {
        level.list().nodes().to_vec()
    };
    let children = stores.freeze_node_list(&nodes);
    let node = match kind {
        BoxKind::HBox => Node::HList(hpack_with_overfull_rule(stores, children, spec)),
        BoxKind::VBox => Node::VList(vpack(stores, children, spec, vpack_params(stores)).node),
        BoxKind::VTop => Node::VList(vtop(stores, children, spec, vpack_params(stores)).node),
    };
    leave_group(input, stores, GroupKind::Simple)?;
    Ok(node)
}

pub(in crate::assignments) fn hpack_with_overfull_rule(
    stores: &mut Universe,
    children: NodeListId,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
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

pub(crate) fn scan_box_group<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    stores.with_hash_only_checkpoints(|stores| {
        let mut brace_depth = 1usize;
        loop {
            crate::executor::sync_engine_state::<S, _>(hooks, nest, stores);
            let token = {
                let mut recorder = NoopRecorder;
                get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
            }
            .ok_or(ExecError::MissingToken {
                context: "box closing brace",
            })?;
            let math_mode = matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath);
            if !math_mode && is_begin_group(token) {
                brace_depth += 1;
            }
            if !math_mode && is_end_group(token) {
                brace_depth -= 1;
                if brace_depth == 0 {
                    flush_pending_hchars(nest, stores)?;
                    return Ok(());
                }
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
    })
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

pub(super) fn first_box_node(stores: &Universe, id: Option<NodeListId>) -> Option<Node> {
    let id = id?;
    stores.nodes(id).first().and_then(|node| match node {
        Node::HList(_) | Node::VList(_) => Some(node.clone()),
        _ => None,
    })
}

pub(super) fn kind_for_primitive(primitive: UnexpandablePrimitive) -> Result<BoxKind, ExecError> {
    match primitive {
        UnexpandablePrimitive::HBox => Ok(BoxKind::HBox),
        UnexpandablePrimitive::VBox => Ok(BoxKind::VBox),
        UnexpandablePrimitive::VTop => Ok(BoxKind::VTop),
        _ => Err(ExecError::MissingToken { context: "box" }),
    }
}

use tex_state::token::Token;
