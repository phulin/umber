use tex_expand::{ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack};
use tex_state::ids::NodeListId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::Node;
use tex_state::token::{Catcode, TracedTokenWord};
use tex_state::{GroupKind, Universe};
use tex_typeset::{PackDiagnostic, PackSpec};

use crate::packing_params::{hpack, hpack_params, vpack, vpack_params, vtop};
use crate::{ExecError, Mode, ModeNest, leave_group};

use super::super::{
    flush_pending_hchars, has_catcode_meaning, next_non_space_traced_x, scan_optional_keyword_x,
    scan_register_index, scan_scaled,
};
use super::vsplit::scan_vsplit_node;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BoxKind {
    HBox,
    VBox,
    VTop,
}

/// A scanned box value together with whether its child lists already belong to
/// the current construction epoch.
pub(super) enum ScannedBoxValue {
    Fresh(Node),
    Shared(Node),
}

impl ScannedBoxValue {
    pub(super) fn into_epoch_node(self, stores: &mut Universe) -> Node {
        match self {
            Self::Fresh(node) => node,
            Self::Shared(node) => stores.clone_node_to_epoch(node),
        }
    }
}

pub(in crate::assignments) fn scan_required_box_node<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<Node, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    scan_box_value(None, input, stores, hooks, context)?
        .map(|value| value.into_epoch_node(stores))
        .ok_or(ExecError::MissingToken { context: "box" })
}

pub(super) fn scan_box_value<S, H>(
    nest: Option<&mut ModeNest>,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<Option<ScannedBoxValue>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let traced = next_non_space_traced_x(input, stores, hooks)?
        .ok_or(ExecError::MissingTracedToken { context })?;
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        return Err(ExecError::MissingToken { context: "box" });
    };
    match stores.meaning(symbol) {
        Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::HBox)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VBox)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VTop) => {
            scan_box_node(kind_for_primitive(primitive)?, input, stores, hooks, traced)
                .map(ScannedBoxValue::Fresh)
                .map(Some)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
            let index = scan_register_index(input, stores, hooks, traced)?;
            let id = if matches!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
            ) {
                stores.take_box_reg_same_level(index)
            } else {
                stores.box_reg(index)
            };
            Ok(first_box_node(stores, id).map(ScannedBoxValue::Shared))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit) => {
            scan_vsplit_node(input, stores, hooks, traced)
                .map(|value| value.map(ScannedBoxValue::Fresh))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox) => {
            let nest = nest.ok_or(ExecError::MissingToken { context: "box" })?;
            take_last_box(nest, stores).map(|value| value.map(ScannedBoxValue::Shared))
        }
        _ => Err(ExecError::MissingToken { context: "box" }),
    }
}

pub(super) fn take_last_box(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<Option<Node>, ExecError> {
    flush_pending_hchars(nest, stores)?;
    match nest.current_mode() {
        Mode::Math | Mode::DisplayMath => {
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! You can't use `\\lastbox' in math mode.\nSorry; this \\lastbox will be void.\n",
            );
            Ok(None)
        }
        Mode::Vertical
            if nest.current_list().is_empty() && stores.page_contributions().is_empty() =>
        {
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! You can't use `\\lastbox' in vertical mode.\nSorry...I usually can't take things from the current page.\nThis \\lastbox will therefore be void.\n",
            );
            Ok(None)
        }
        Mode::Vertical => Ok(stores.take_page_contribution_last_box()),
        Mode::InternalVertical | Mode::Horizontal | Mode::RestrictedHorizontal => {
            Ok(nest.current_list_mut().take_last_box())
        }
    }
}

pub(super) fn scan_box_node<S, H>(
    kind: BoxKind,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<Node, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let spec = scan_pack_spec(input, stores, hooks, context)?;
    let opener = next_non_space_traced_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "box group",
    })?;
    if !has_catcode_meaning(
        stores,
        tex_expand::semantic_token(opener),
        Catcode::BeginGroup,
    ) {
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
            let semantic = tex_expand::semantic_token(token);
            let math_mode = matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath);
            if !math_mode && has_catcode_meaning(stores, semantic, Catcode::BeginGroup) {
                brace_depth += 1;
            }
            if !math_mode && has_catcode_meaning(stores, semantic, Catcode::EndGroup) {
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
                        token: semantic,
                        origin: token.origin(),
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
    context: TracedTokenWord,
) -> Result<PackSpec, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    if scan_optional_keyword_x(input, stores, hooks, "to")? {
        Ok(PackSpec::Exactly(scan_scaled(
            input, stores, hooks, context,
        )?))
    } else if scan_optional_keyword_x(input, stores, hooks, "spread")? {
        Ok(PackSpec::Spread(scan_scaled(
            input, stores, hooks, context,
        )?))
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
