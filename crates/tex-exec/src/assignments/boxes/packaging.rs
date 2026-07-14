use tex_expand::{NoopRecorder, get_x_token_with_recorder_and_context};
use tex_lex::{InputSource, InputStack};
use tex_state::ids::NodeListId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::Node;
use tex_state::token::{Catcode, TracedTokenWord};
use tex_state::{ExpansionState, GroupKind, Universe};
use tex_typeset::{PackDiagnostic, PackSpec, plan_hpack_nodes};

use crate::packing_params::{hpack, hpack_params, vpack, vpack_params, vtop};
use crate::{ExecError, Mode, ModeNest, leave_group, push_traced_tokens};

use super::super::{
    flush_pending_hchars, has_catcode_meaning, next_non_space_traced_x, normal_paragraph,
    scan_optional_keyword_x, scan_register_index, scan_scaled,
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
    pub(super) fn into_node(self) -> Node {
        match self {
            Self::Fresh(node) | Self::Shared(node) => node,
        }
    }
}

pub(in crate::assignments) fn scan_required_box_node<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
    context: TracedTokenWord,
) -> Result<Node, ExecError>
where
    S: InputSource,
{
    scan_box_value(None, input, stores, execution, context)?
        .map(ScannedBoxValue::into_node)
        .ok_or(ExecError::MissingToken { context: "box" })
}

pub(super) fn scan_box_value<S>(
    nest: Option<&mut ModeNest>,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
    context: TracedTokenWord,
) -> Result<Option<ScannedBoxValue>, ExecError>
where
    S: InputSource,
{
    let traced = next_non_space_traced_x(input, stores, execution)?
        .ok_or(ExecError::MissingTracedToken { context })?;
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        return recover_missing_box(input, stores, traced);
    };
    match stores.meaning(symbol) {
        Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::HBox)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VBox)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VTop) => scan_box_node(
            kind_for_primitive(primitive)?,
            input,
            stores,
            execution,
            traced,
        )
        .map(ScannedBoxValue::Fresh)
        .map(Some),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
            let index = scan_register_index(input, stores, execution, traced)?;
            let id = if matches!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
            ) {
                stores.take_box_reg_same_level(index)
            } else {
                stores.box_reg(index)
            };
            if !matches!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
            ) && let Some(id) = id
            {
                stores.pin_survivor(id);
            }
            Ok(first_box_node(stores, id).map(ScannedBoxValue::Shared))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit) => {
            scan_vsplit_node(input, stores, execution, traced)
                .map(|value| value.map(ScannedBoxValue::Fresh))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox) => {
            let nest = nest.ok_or(ExecError::MissingToken { context: "box" })?;
            take_last_box(nest, stores).map(|value| value.map(ScannedBoxValue::Shared))
        }
        _ => recover_missing_box(input, stores, traced),
    }
}

/// TeX82's `scan_box` backs up a non-box command after reporting the error
/// (tex.web §1076), leaving the destination box void while normal command
/// processing resumes with the rejected token.
fn recover_missing_box<S: InputSource>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    traced: TracedTokenWord,
) -> Result<Option<ScannedBoxValue>, ExecError> {
    crate::push_traced_tokens(input, stores, [traced]);
    stores.world_mut().write_text(
        tex_state::PrintSink::TerminalAndLog,
        "\n! A <box> was supposed to be here.\nI was expecting to see \\hbox or \\vbox or \\copy or \\box or\nsomething like that. So you might find something missing in\nyour output. But keep trying; you can fix this later.\n",
    );
    Ok(None)
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

pub(super) fn scan_box_node<S>(
    kind: BoxKind,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
    context: TracedTokenWord,
) -> Result<Node, ExecError>
where
    S: InputSource,
{
    let spec = scan_pack_spec(input, stores, execution, context)?;
    let opener =
        next_non_space_traced_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
            context: "box group",
        })?;
    if !has_catcode_meaning(
        stores,
        tex_expand::semantic_token(opener),
        Catcode::BeginGroup,
    ) {
        // TeX.web §403 `scan_left_brace` backs up the first body token and
        // proceeds with an inserted opening brace.
        push_traced_tokens(input, stores, [opener]);
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing { inserted.\nA left brace was mandatory here, so I've put one in.\n",
        );
    }
    let group_kind = match kind {
        BoxKind::HBox => GroupKind::HBox,
        BoxKind::VBox => GroupKind::VBox,
        BoxKind::VTop => GroupKind::VTop,
    };
    stores.enter_group_with_kind(group_kind);
    let box_group_depth = stores.execution_group_depth();
    let mode = if kind == BoxKind::HBox {
        Mode::RestrictedHorizontal
    } else {
        Mode::InternalVertical
    };
    let mut inner = ModeNest::new();
    if kind != BoxKind::HBox {
        // TeX82 begin_box normalizes paragraph-scoped parameters after the
        // vbox/vtop group has opened, so the defaults are local to the box.
        // In particular, stale outer parshape data must not determine a
        // display started in this internal vertical list.
        normal_paragraph(&mut inner, stores);
    }
    inner.push(mode);
    scan_box_group(&mut inner, input, stores, execution, box_group_depth)?;
    if kind != BoxKind::HBox && inner.current_mode() == Mode::Horizontal {
        // TeX82's vbox_group/vtop_group right-brace handler runs end_graf
        // before package. This matters when display math has resumed an empty
        // paragraph immediately before the box's closing brace: packaging the
        // horizontal level would otherwise discard the completed vertical
        // list beneath it.
        crate::assignments::end_paragraph(&mut inner, stores)?;
    }
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
    leave_group(input, stores, group_kind)?;
    Ok(node)
}

pub(crate) fn hpack_with_overfull_rule(
    stores: &mut Universe,
    children: NodeListId,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let mut packed = hpack(stores, children, spec, params);
    // TeX's hpack overfull branch is guarded by list_ptr(r) <> null. An
    // explicitly negative-width empty hbox is therefore not decorated even
    // when \overfullrule is positive.
    if !stores.nodes(children).is_empty()
        && params.overfull_rule.raw() > 0
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

pub(crate) fn hpack_owned_with_overfull_rule(
    stores: &mut Universe,
    nodes: &mut Vec<Node>,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let plan = plan_hpack_nodes(stores, nodes, spec, params);
    if !nodes.is_empty()
        && params.overfull_rule.raw() > 0
        && plan
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, PackDiagnostic::Overfull { .. }))
    {
        nodes.push(Node::Rule {
            width: Some(params.overfull_rule),
            height: None,
            depth: None,
        });
    }
    let children = stores.freeze_node_list_owned(nodes);
    plan.finish(children).node
}

pub(crate) fn scan_box_group<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
    box_group_depth: u32,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    {
        loop {
            crate::executor::sync_engine_state::<S>(execution, nest, stores);
            let token = {
                let mut recorder = NoopRecorder;
                match get_x_token_with_recorder_and_context(input, stores, &mut recorder, execution)
                {
                    Ok(token) => token,
                    Err(tex_expand::ExpandError::UndefinedControlSequence { name, .. }) => {
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            &format!("\n! Undefined control sequence \\{name}.\n"),
                        );
                        continue;
                    }
                    Err(tex_expand::ExpandError::ExtraConditionalControl { name, .. }) => {
                        crate::diagnostics::report_extra_conditional(stores, name);
                        continue;
                    }
                    Err(err) => return Err(err.into()),
                }
            }
            .ok_or(ExecError::MissingToken {
                context: "box closing brace",
            })?;
            let semantic = tex_expand::semantic_token(token);
            // TeX.web §1084 packages on the right brace for the active box
            // save-stack group. Scanners such as \message consume their own
            // balanced braces, so delivered-token brace counting is insufficient.
            if stores.execution_group_depth() == box_group_depth
                && has_catcode_meaning(stores, semantic, Catcode::EndGroup)
            {
                flush_pending_hchars(nest, stores)?;
                return Ok(());
            }
            let action =
                match crate::dispatch_delivered_token(nest, token, input, stores, execution) {
                    Ok(action) => action,
                    Err(ExecError::Expand(tex_expand::ExpandError::UndefinedControlSequence {
                        name,
                        ..
                    })) => {
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            &format!("\n! Undefined control sequence \\{name}.\n"),
                        );
                        continue;
                    }
                    Err(ExecError::Expand(tex_expand::ExpandError::Captured { error, .. }))
                        if matches!(
                            error.as_ref(),
                            tex_expand::ExpandError::UndefinedControlSequence { .. }
                        ) =>
                    {
                        let tex_expand::ExpandError::UndefinedControlSequence { name, .. } = *error
                        else {
                            unreachable!("guard restricts captured expansion error")
                        };
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            &format!("\n! Undefined control sequence \\{name}.\n"),
                        );
                        continue;
                    }
                    // Recursive box scanning is still TeX's main-control loop. A
                    // recoverable assignment error must consume the bad command
                    // and continue inside the box, just as the outer executor
                    // does, rather than aborting the construction transaction and
                    // replaying the remaining body on the enclosing list.
                    Err(ExecError::UnsupportedAssignmentTarget) => {
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            "\n! Improper assignment target; this assignment is ignored.\n",
                        );
                        continue;
                    }
                    Err(ExecError::ExtraConditionalControl { primitive, .. }) => {
                        let name = match primitive {
                            tex_state::meaning::ExpandablePrimitive::Else => "else",
                            tex_state::meaning::ExpandablePrimitive::Fi => "fi",
                            tex_state::meaning::ExpandablePrimitive::Or => "or",
                            _ => {
                                unreachable!("error variant is restricted to conditional controls")
                            }
                        };
                        crate::diagnostics::report_extra_conditional(stores, name);
                        continue;
                    }
                    Err(
                        ExecError::ExtraRightBraceOrForgottenEndgroup { .. }
                        | ExecError::ExtraRightBraceOrForgottenDollar { .. }
                        | ExecError::TooManyRightBraces { .. }
                        | ExecError::ExtraEndGroup { .. }
                        | ExecError::EndGroupMismatch { .. }
                        | ExecError::MathShiftGroupMismatch { .. },
                    ) => continue,
                    Err(err) => return Err(err),
                };
            match action {
                crate::DispatchAction::Continue => {}
                crate::DispatchAction::Shipout(_) => {}
                crate::DispatchAction::End => {
                    // A stop command cannot terminate TeX from inside an
                    // unfinished box. Close this recovery scan and replay it
                    // so outer main control can perform the ordinary final
                    // page-builder cleanup in vertical mode.
                    push_traced_tokens(input, stores, [token]);
                    return Ok(());
                }
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
    }
}

pub(crate) fn scan_pack_spec<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
    context: TracedTokenWord,
) -> Result<PackSpec, ExecError>
where
    S: InputSource,
{
    if scan_optional_keyword_x(input, stores, execution, "to")? {
        Ok(PackSpec::Exactly(scan_scaled(
            input, stores, execution, context,
        )?))
    } else if scan_optional_keyword_x(input, stores, execution, "spread")? {
        Ok(PackSpec::Spread(scan_scaled(
            input, stores, execution, context,
        )?))
    } else {
        Ok(PackSpec::Natural)
    }
}

pub(super) fn first_box_node(stores: &Universe, id: Option<NodeListId>) -> Option<Node> {
    let id = id?;
    stores.nodes(id).first().and_then(|node| match node {
        tex_state::node_arena::NodeRef::HList(_) | tex_state::node_arena::NodeRef::VList(_) => {
            Some(node.to_owned())
        }
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
