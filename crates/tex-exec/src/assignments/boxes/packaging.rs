use tex_expand::get_x_token_with_context;
use tex_lex::InputStack;
use tex_state::TokenListReplayKind;
use tex_state::env::banks::TokParam;
use tex_state::ids::NodeListId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::Node;
use tex_state::token::{Catcode, TracedTokenWord};
use tex_state::{ExpansionState, GeometryObservation, GroupKind, Universe};
use tex_typeset::{PackDiagnostic, PackSpec, plan_hpack_nodes};

use crate::packing_params::{
    hpack, hpack_params, recover_texxet_directions, vpack, vpack_params, vtop,
};
use crate::{ExecError, Mode, ModeNest, leave_group, push_traced_tokens};

use super::super::{
    fire_afterassignment, flush_pending_hchars, has_catcode_meaning, next_non_space_traced_x,
    normal_paragraph, scan_optional_keyword_x, scan_register_index, scan_scaled,
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

#[derive(Clone, Copy)]
pub(super) enum BoxScanContext {
    BoxExpected,
    SetBox,
}

impl ScannedBoxValue {
    pub(super) fn into_node(self) -> Node {
        match self {
            Self::Fresh(node) | Self::Shared(node) => node,
        }
    }
}

pub(in crate::assignments) fn scan_box_value_node(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<Option<Node>, ExecError> {
    scan_box_value(
        None,
        input,
        stores,
        execution,
        context,
        BoxScanContext::BoxExpected,
    )
    .map(|value| value.map(ScannedBoxValue::into_node))
}

pub(super) fn scan_box_value(
    nest: Option<&mut ModeNest>,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
    scan_context: BoxScanContext,
) -> Result<Option<ScannedBoxValue>, ExecError> {
    // TeX82's scan_box starts with get_x_token's "next non-blank non-relax"
    // loop (tex.web §1084).  This matters for format code that deliberately
    // leaves a compatibility hook equal to \relax before the real box command.
    let traced = loop {
        let traced = next_non_space_traced_x(input, stores, execution)?
            .ok_or(ExecError::MissingTracedToken { context })?;
        let Token::Cs(symbol) = traced.semantic_token() else {
            break traced;
        };
        if stores.meaning(symbol) != Meaning::Relax {
            break traced;
        }
    };
    let token = traced.semantic_token();
    let Token::Cs(symbol) = token else {
        return recover_missing_box(input, stores, traced, scan_context);
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
            let source_proven = execution.paragraph_box_is_source_proven(index);
            let destructive = matches!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
            );
            let id = if destructive {
                stores.take_box_reg_same_level(index)
            } else {
                stores.box_reg(index)
            };
            super::account_external_box_access(
                execution,
                index,
                source_proven,
                if destructive {
                    UnexpandablePrimitive::Box
                } else {
                    UnexpandablePrimitive::Copy
                },
                id.is_some(),
            );
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
            execution.mark_paragraph_barrier(
                tex_state::ParagraphBarrierReason::UnsupportedEscapingWrite,
            );
            scan_vsplit_node(input, stores, execution, traced)
                .map(|value| value.map(ScannedBoxValue::Fresh))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox) => {
            execution.mark_paragraph_barrier(
                tex_state::ParagraphBarrierReason::UnsupportedEscapingWrite,
            );
            let nest = nest.ok_or(ExecError::MissingToken { context: "box" })?;
            take_last_box(nest, stores, execution.command_fuel())
                .map(|value| value.map(ScannedBoxValue::Shared))
        }
        _ => recover_missing_box(input, stores, traced, scan_context),
    }
}

/// TeX82's `scan_box` backs up a non-box command after reporting the error
/// (tex.web §1084), leaving the destination box void while normal command
/// processing resumes with the rejected token.
fn recover_missing_box(
    input: &mut InputStack,
    stores: &mut Universe,
    traced: TracedTokenWord,
    scan_context: BoxScanContext,
) -> Result<Option<ScannedBoxValue>, ExecError> {
    let (message, help): (&str, &[&str]) = match scan_context {
        // TeX82 §1084 selects this branch when `box_context < box_flag`;
        // `\setbox` supplies its destination register as that context.
        BoxScanContext::SetBox => (
            "Improper \\setbox",
            &[
                "Sorry, \\setbox is not allowed after \\halign in a display,",
                "or between \\accent and an accented character.",
            ],
        ),
        BoxScanContext::BoxExpected => (
            "A <box> was supposed to be here",
            &[
                "I was expecting to see \\hbox or \\vbox or \\copy or \\box or",
                "something like that. So you might find something missing in",
                "your output. But keep trying; you can fix this later.",
            ],
        ),
    };
    crate::error_report::back_error(input, stores, traced, message, help)?;
    Ok(None)
}

pub(crate) fn take_last_box(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Option<Node>, ExecError> {
    flush_pending_hchars(nest, stores, fuel)?;
    match nest.current_mode() {
        Mode::Math | Mode::DisplayMath => {
            report_cannot_take_last_box(
                stores,
                "math mode",
                &["Sorry; this \\lastbox will be void."],
            )?;
            Ok(None)
        }
        Mode::Vertical
            if nest.current_list().is_empty() && stores.page_contributions().is_empty() =>
        {
            report_cannot_take_last_box(
                stores,
                "vertical mode",
                &[
                    "Sorry...I usually can't take things from the current page.",
                    "This \\lastbox will therefore be void.",
                ],
            )?;
            Ok(None)
        }
        Mode::Vertical => {
            let Some(tail) =
                crate::effective_tail::EffectiveTail::find(stores.page_contributions().iter())
            else {
                return Ok(None);
            };
            if !matches!(tail.node(), Node::HList(_) | Node::VList(_)) {
                return Ok(None);
            }
            let range = tail.removal_range();
            let mut removed = stores.remove_page_contribution_range(range);
            Ok(reset_removed_box_shift(&mut removed))
        }
        Mode::InternalVertical | Mode::Horizontal | Mode::RestrictedHorizontal => {
            let Some(tail) =
                crate::effective_tail::EffectiveTail::find(nest.current_list().nodes().iter())
            else {
                return Ok(None);
            };
            if !matches!(tail.node(), Node::HList(_) | Node::VList(_)) {
                return Ok(None);
            }
            let range = tail.removal_range();
            let mut removed = nest.current_list_mutation().remove_node_range(range);
            Ok(reset_removed_box_shift(&mut removed))
        }
    }
}

fn reset_removed_box_shift(removed: &mut [Node]) -> Option<Node> {
    let node = removed
        .iter_mut()
        .find(|node| matches!(node, Node::HList(_) | Node::VList(_)))?;
    match node {
        Node::HList(box_node) | Node::VList(box_node) => {
            box_node.shift = tex_state::scaled::Scaled::from_raw(0);
        }
        _ => unreachable!("node was selected as a box"),
    }
    Some(node.clone())
}

/// TeX.web §370's `<Complain about an undefined macro>`.
///
/// The offending name is deliberately absent from the message: §82's context
/// display ends the top line with it, which is what the third help line means
/// by "the control sequence at the end of the top line".
fn report_undefined_control_sequence(
    input: &InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    crate::diagnostics::report_undefined_control_sequence_in_input(input, stores)?;
    Ok(())
}

/// TeX.web §1080's two `\lastbox` refusals, opened by §72's `you_cant`
/// (`print_err("You can't use `"); print_cmd_chr; print("' in "); print_mode`).
///
/// `take_last_box` is also reached from canonical replay and from page
/// building, neither of which holds a live `InputStack`, so §82's display
/// comes from the last published input summary.
fn report_cannot_take_last_box(
    stores: &mut Universe,
    mode: &str,
    help: &[&str],
) -> Result<(), ExecError> {
    let context = crate::diagnostics::show_context(stores, stores.input_summary());
    let mut report = stores.print_err("You can't use `");
    report
        .print_esc("lastbox")
        .print("' in ")
        .print(mode)
        .help(help)
        .context(context);
    report.error().jump_out()?;
    Ok(())
}

pub(super) fn scan_box_node(
    kind: BoxKind,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<Node, ExecError> {
    let spec = scan_pack_spec(input, stores, execution, context)?;
    let opener =
        next_non_space_traced_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
            context: "box group",
        })?;
    if !has_catcode_meaning(stores, opener.semantic_token(), Catcode::BeginGroup) {
        // TeX.web §403 `scan_left_brace` backs up the first body token and
        // proceeds with an inserted opening brace.
        crate::error_report::back_error(
            input,
            stores,
            opener,
            "Missing { inserted",
            &[
                "A left brace was mandatory here, so I've put one in.",
                "You might want to delete and/or insert some corrections",
                "so that I will find a matching right brace soon.",
                "(If you're confused by all this, try typing `I}' now.)",
            ],
        )?;
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
    inner.push(mode)?;
    let (hook, replay_kind) = match kind {
        BoxKind::HBox => (TokParam::EVERY_HBOX, TokenListReplayKind::EveryHBox),
        BoxKind::VBox | BoxKind::VTop => (TokParam::EVERY_VBOX, TokenListReplayKind::EveryVBox),
    };
    let hook = stores.tok_param(hook);
    if !stores.tokens(hook).is_empty() {
        input.push_token_list(hook, replay_kind);
    }
    // TeX82 inserts a pending `\afterassignment` token before the every-box
    // list when this box is the value of `\setbox`. Input frames are LIFO, so
    // push the hook first and the one-token afterassignment replay second.
    if fire_afterassignment(input, stores) {
        execution
            .mark_paragraph_barrier(tex_state::ParagraphBarrierReason::UnsupportedEscapingWrite);
    }
    scan_box_group(&mut inner, input, stores, execution, box_group_depth)?;
    if kind != BoxKind::HBox && inner.current_mode() == Mode::Horizontal {
        // TeX82's vbox_group/vtop_group right-brace handler runs end_graf
        // before package. This matters when display math has resumed an empty
        // paragraph immediately before the box's closing brace: packaging the
        // horizontal level would otherwise discard the completed vertical
        // list beneath it.
        crate::assignments::end_paragraph_with_fuel(&mut inner, stores, execution.command_fuel())?;
    }
    let level =
        crate::assignments::commit_current_list(&mut inner, stores, execution.command_fuel())?;
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
    execution.paragraph_group_exited(stores);
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
    mut diagnostic_nodes: Option<&mut Vec<Node>>,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let lr_problems = recover_texxet_directions(stores, nodes);
    if let Some(diagnostic_nodes) = diagnostic_nodes.as_deref_mut() {
        let _ = recover_texxet_directions(stores, diagnostic_nodes);
    }
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
        if let Some(diagnostic_nodes) = diagnostic_nodes.as_deref_mut() {
            diagnostic_nodes.push(Node::Rule {
                width: Some(params.overfull_rule),
                height: None,
                depth: None,
            });
        }
    }
    let short_diagnostic_nodes = diagnostic_nodes
        .as_deref()
        .map(|physical| project_short_diagnostic_discs(physical, nodes));
    let diagnostic_list_layout = if short_diagnostic_nodes.is_some() {
        crate::pack_report::DiagnosticListLayout::DetachedProjection
    } else {
        crate::pack_report::DiagnosticListLayout::FrozenList
    };
    let children = stores.freeze_node_list_owned(nodes);
    let mut packed = plan.finish(children);
    stores.set_last_badness(packed.badness);
    stores.record_geometry_observation(GeometryObservation::Hpack {
        width_sp: i64::from(packed.node.width.raw()),
        height_sp: i64::from(packed.node.height.raw()),
        depth_sp: i64::from(packed.node.depth.raw()),
    });
    let diagnostic_box = diagnostic_nodes.map_or(packed.node, |nodes| {
        let diagnostic_children = stores.freeze_node_list(nodes);
        let children = stores.freeze_node_list(
            short_diagnostic_nodes
                .as_deref()
                .expect("physical diagnostics have a short-display projection"),
        );
        packed.node.diagnostic_children = Some(diagnostic_children);
        tex_state::node::BoxNode {
            children,
            ..packed.node
        }
    });
    crate::pack_report::report_pack_diagnostics(
        stores,
        crate::pack_report::PackedDirection::Horizontal,
        &packed.diagnostics,
        &tex_state::node::Node::HList(diagnostic_box),
        diagnostic_list_layout,
    );
    if let Some((missing, extra)) = lr_problems {
        crate::pack_report::report_lr_problems(
            stores,
            missing,
            extra,
            &tex_state::node::Node::HList(diagnostic_box),
            diagnostic_list_layout,
        );
    }
    packed.node
}

/// Combines TeX's physical discretionary topology with semantic side lists.
///
/// Physical paragraph lists expand ligatures, so their node indices do not
/// align with the semantic list. Discretionaries themselves retain source
/// order across that projection; pair them by that order rather than by a
/// positional zip. The physical node continues to own `replace_count` and
/// its detached replacement list, while §174 renders the corresponding
/// semantic pre/post branches.
pub(super) fn project_short_diagnostic_discs(physical: &[Node], semantic: &[Node]) -> Vec<Node> {
    let mut semantic_discs = semantic.iter().filter_map(|node| match node {
        Node::Disc { pre, post, .. } => Some((*pre, *post)),
        _ => None,
    });
    physical
        .iter()
        .map(|node| match node {
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => {
                let (pre, post) = semantic_discs.next().unwrap_or((*pre, *post));
                Node::Disc {
                    kind: *kind,
                    pre,
                    post,
                    replace: *replace,
                    physical_replace_count: *physical_replace_count,
                }
            }
            _ => node.clone(),
        })
        .collect()
}

pub(crate) fn scan_box_group(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    box_group_depth: u32,
) -> Result<(), ExecError> {
    {
        loop {
            crate::executor::sync_engine_state(execution, nest, stores);
            let token = {
                match get_x_token_with_context(
                    input,
                    &mut tex_state::ExpansionContext::new(stores),
                    execution,
                ) {
                    Ok(token) => token,
                    Err(error) => match error.into_conditional_recovery() {
                        Ok(tex_state::ExpansionRecovery::UndefinedControlSequence) => {
                            report_undefined_control_sequence(input, stores)?;
                            continue;
                        }
                        Ok(tex_state::ExpansionRecovery::ExtraConditionalControl { name }) => {
                            crate::diagnostics::report_extra_conditional(stores, name)?;
                            continue;
                        }
                        Ok(_) => unreachable!("conditional recovery has a closed vocabulary"),
                        Err(error) => return Err(error.into()),
                    },
                }
            }
            .ok_or(ExecError::MissingToken {
                context: "box closing brace",
            })?;
            let semantic = token.semantic_token();
            if semantic.is_frozen_endv() {
                // TeX.web §1131 routes every end-v marker through
                // `do_endv`. A box group cannot finish an alignment entry, so
                // `do_endv` takes §1064's `off_save` branch: back up end-v,
                // insert the token that closes the current box group, and let
                // main control retry end-v at the alignment entry level.
                crate::assignments::off_save_alignment(token, input, stores)?;
                continue;
            }
            // TeX.web §1084 packages on the right brace for the active box
            // save-stack group. Scanners such as \message consume their own
            // balanced braces, so delivered-token brace counting is insufficient.
            if stores.execution_group_depth() == box_group_depth
                && has_catcode_meaning(stores, semantic, Catcode::EndGroup)
            {
                flush_pending_hchars(nest, stores, execution.command_fuel())?;
                return Ok(());
            }
            let action =
                match crate::dispatch_delivered_token(nest, token, input, stores, execution) {
                    Ok(action) => action,
                    Err(error) if error.is_undefined_control_sequence() => {
                        report_undefined_control_sequence(input, stores)?;
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
                        crate::diagnostics::report_extra_conditional(stores, name)?;
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

pub(crate) fn scan_pack_spec(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<PackSpec, ExecError> {
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

pub(crate) fn first_box_node(stores: &Universe, id: Option<NodeListId>) -> Option<Node> {
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
