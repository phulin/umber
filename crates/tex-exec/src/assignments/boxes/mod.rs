use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Token, TracedTokenWord};

use super::*;
use crate::vertical::{append_vertical_contribution, build_page_if_outer_vertical};
use crate::{ExecError, Mode, ModeNest};

mod leaders;
mod packaging;
#[cfg(test)]
mod tests;
pub(crate) mod vsplit;

use crate::canonical_box_runtime::hmode::infinite_glue;
use crate::canonical_box_runtime::{
    acquire_box_register, append_box_node_to_current_list, append_box_register,
    append_node_to_current_list, apply_box_shift_delta, assign_box_dimension,
    box_dimension_for_primitive, execute_scanned_saved_vertical_discards, execute_scanned_unbox,
    first_box_node, take_last_box,
};
use leaders::{scan_leader_glue, scan_leader_payload};
pub(super) use packaging::scan_box_value_node;
use packaging::{
    BoxScanContext, ScannedBoxValue, kind_for_primitive, scan_box_node, scan_box_value,
};
pub(crate) use packaging::{scan_box_group, scan_pack_spec};
use vsplit::scan_vsplit_node;

pub(super) fn execute_make_box(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    _global: bool,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let node = if primitive == UnexpandablePrimitive::VSplit {
        scan_vsplit_node(input, stores, execution, context)?
    } else {
        Some(scan_box_node(
            kind_for_primitive(primitive)?,
            input,
            stores,
            execution,
            context,
        )?)
    };
    if let Some(node) = node {
        append_box_node_to_current_list(nest, stores, node, execution.command_fuel())?;
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

pub(crate) fn scan_math_box(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<Node>, ExecError> {
    let node = match primitive {
        UnexpandablePrimitive::HBox | UnexpandablePrimitive::VBox | UnexpandablePrimitive::VTop => {
            Some(scan_box_node(
                kind_for_primitive(primitive)?,
                input,
                stores,
                execution,
                context,
            )?)
        }
        UnexpandablePrimitive::VSplit => scan_vsplit_node(input, stores, execution, context)?,
        UnexpandablePrimitive::Box | UnexpandablePrimitive::Copy => {
            let index = scan_register_index(input, stores, execution, context)?;
            let source_proven = execution.paragraph_box_is_source_proven(index);
            let id = acquire_box_register(stores, index, primitive == UnexpandablePrimitive::Copy);
            account_external_box_access(execution, index, source_proven, primitive, id.is_some());
            first_box_node(stores, id)
        }
        UnexpandablePrimitive::Raise | UnexpandablePrimitive::Lower => {
            let amount = scan_scaled(input, stores, execution, context)?;
            let Some(mut node) = packaging::scan_box_value_node(input, stores, execution, context)?
            else {
                return Ok(None);
            };
            apply_shift(&mut node, primitive, amount)?;
            Some(node)
        }
        _ => unreachable!("caller restricts math box commands"),
    };
    let _ = nest;
    Ok(node)
}

pub(super) fn execute_setbox(
    global: bool,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<u16, ExecError> {
    let index = scan_setbox_target(input, stores, execution, context)?;
    let mut transaction = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let (nest, stores) = transaction.parts();
    let mut construction = stores.begin_box_build();
    let value = match scan_box_value(
        Some(nest),
        input,
        &mut construction,
        execution,
        context,
        BoxScanContext::SetBox,
    ) {
        Ok(Some(ScannedBoxValue::Fresh(node))) => {
            let list = construction.freeze_node_list(&[node]);
            Some(list)
        }
        Ok(Some(ScannedBoxValue::Shared(node))) => {
            let list = construction.freeze_node_list(&[node]);
            Some(list)
        }
        Ok(None) => None,
        Err(err) => return Err(err),
    };
    construction.finish(index, value, global);
    transaction.commit();
    Ok(index)
}

/// Scans the portion of TeX82 §1241's `\setbox` assignment that is owned
/// even when box construction is forbidden: the register and optional equals.
pub(crate) fn scan_setbox_target(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<u16, ExecError> {
    let index = scan_register_index(input, stores, execution, context)?;
    skip_optional_equals_x(input, stores, execution)?;
    Ok(index)
}

pub(super) fn execute_box_dimension_assignment(
    primitive: UnexpandablePrimitive,
    global: bool,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let index = scan_register_index(input, stores, execution, context)?;
    skip_optional_equals_x(input, stores, execution)?;
    let value = scan_scaled(input, stores, execution, context)?;
    let dimension = box_dimension_for_primitive(primitive)?;
    assign_box_dimension(stores, index, dimension, value, global);
    Ok(())
}

pub(super) fn execute_box_list_command(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    match primitive {
        UnexpandablePrimitive::Box | UnexpandablePrimitive::Copy => {
            let index = scan_register_index(input, stores, execution, context)?;
            execute_box_register_read(primitive, index, nest, stores, execution)?;
        }
        UnexpandablePrimitive::UnHBox
        | UnexpandablePrimitive::UnHCopy
        | UnexpandablePrimitive::UnVBox
        | UnexpandablePrimitive::UnVCopy => {
            let index = scan_register_index(input, stores, execution, context)?;
            let source_proven = execution.paragraph_box_is_source_proven(index);
            if !source_proven {
                execution.record_paragraph_box_read(index);
                if matches!(
                    primitive,
                    UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnVBox
                ) && stores.box_reg(index).is_some()
                {
                    execution.mark_paragraph_barrier(
                        tex_state::ParagraphBarrierReason::UnsupportedEscapingWrite,
                    );
                }
            }
            execute_scanned_unbox(primitive, index, nest, stores, execution.command_fuel())?;
        }
        UnexpandablePrimitive::PageDiscards | UnexpandablePrimitive::SplitDiscards => {
            execute_scanned_saved_vertical_discards(
                primitive,
                nest,
                stores,
                execution.command_fuel(),
            )?;
        }
        UnexpandablePrimitive::LastBox => {
            if let Some(node) = take_last_box(nest, stores, execution.command_fuel())? {
                append_box_node_to_current_list(nest, stores, node, execution.command_fuel())?;
            }
        }
        UnexpandablePrimitive::Raise
        | UnexpandablePrimitive::Lower
        | UnexpandablePrimitive::MoveLeft
        | UnexpandablePrimitive::MoveRight => {
            let amount = scan_scaled(input, stores, execution, context)?;
            if let Some(mut node) = scan_box_value_node(input, stores, execution, context)? {
                apply_shift(&mut node, primitive, amount)?;
                append_box_node_to_current_list(nest, stores, node, execution.command_fuel())?;
            }
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

/// Applies an already command-scanned `\\box` or `\\copy` register read.
///
/// The command-core owns `scan_int`; this stomach helper receives only its
/// completed index and applies the TeX82 box-list mutation.
pub(crate) fn execute_box_register_read(
    primitive: UnexpandablePrimitive,
    index: u16,
    nest: &mut ModeNest,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let source_proven = execution.paragraph_box_is_source_proven(index);
    let id = acquire_box_register(stores, index, primitive == UnexpandablePrimitive::Copy);
    account_external_box_access(execution, index, source_proven, primitive, id.is_some());
    append_box_register(nest, stores, id, execution.command_fuel())
}

/// Replays a command-scanned unboxing register operation without reopening
/// the input stream.  Ownership remains in `Universe` so a failed aggregate
/// replay rolls back both the register and its child-list liveness together.
fn account_external_box_access(
    execution: &mut crate::ExecutionContext<'_>,
    index: u16,
    source_proven: bool,
    primitive: UnexpandablePrimitive,
    present: bool,
) {
    if source_proven {
        return;
    }
    execution.record_paragraph_box_read(index);
    if present && primitive == UnexpandablePrimitive::Box {
        execution
            .mark_paragraph_barrier(tex_state::ParagraphBarrierReason::UnsupportedEscapingWrite);
    }
}

pub(super) fn execute_kern_or_skip(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    match primitive {
        UnexpandablePrimitive::Kern => {
            let amount = scan_scaled(input, stores, execution, context)?;
            append_node_to_current_list(
                nest,
                stores,
                Node::Kern {
                    amount,
                    kind: KernKind::Explicit,
                },
                execution.command_fuel(),
            )?;
        }
        UnexpandablePrimitive::HSkip => {
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                ensure_horizontal_for_character(nest, input, stores, execution.command_fuel())?;
            }
            let spec = scan_glue_id(input, stores, execution, false, context)?;
            append_node_to_current_list(
                nest,
                stores,
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                execution.command_fuel(),
            )?;
        }
        UnexpandablePrimitive::VSkip
        | UnexpandablePrimitive::VFil
        | UnexpandablePrimitive::VFill
        | UnexpandablePrimitive::VSs
        | UnexpandablePrimitive::VFilNeg => {
            execute_vertical_skip(primitive, nest, input, stores, execution, context)?
        }
        _ => unreachable!("caller restricts kern/skip primitives"),
    }
    Ok(())
}

pub(super) fn execute_leaders(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let leader = scan_leader_payload(input, stores, execution, context)?;
    let spec = match scan_leader_glue(input, stores, execution, nest.current_mode(), context) {
        Ok(spec) => spec,
        Err(ExecError::LeadersNotFollowedByProperGlue { .. }) => {
            // TeX.web §1078 discards the scanned leader payload and resumes
            // main control. The scanner has already backed the unsuitable
            // command up, so `error` alone completes §1078's `back_error`.
            crate::error_report::report_input_error(
                input,
                stores,
                "Leaders not followed by proper glue",
                &[
                    "You should say `\\leaders <box or rule><hskip or vskip>'.",
                    "I found the <box or rule>, but there's no suitable",
                    "<hskip or vskip>, so I'm ignoring these leaders.",
                ],
            )?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    crate::canonical_box_runtime::append_leader_contribution(
        nest,
        stores,
        crate::canonical_box_runtime::leader_glue_kind(primitive),
        leader,
        spec,
        execution.command_fuel(),
    )?;
    Ok(())
}

pub(super) fn execute_hrule(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    match nest.current_mode() {
        Mode::Vertical | Mode::InternalVertical => {}
        Mode::Horizontal => end_paragraph_with_fuel(nest, stores, execution.command_fuel())?,
        Mode::RestrictedHorizontal => {
            // TeX.web §1095's `head_for_vmode`: an `\hrule` in restricted
            // horizontal mode cannot start a vertical list, so it is dropped
            // rather than turned into a mode change.
            let report_context = crate::diagnostics::show_context(stores, &input.summary());
            let mut report = stores.print_err("You can't use `");
            report
                .print_esc("hrule")
                .print("' here except with leaders")
                .help(&[
                    "To put a horizontal rule in an hbox or an alignment,",
                    "you should use \\leaders or \\hrulefill (see The TeXbook).",
                ])
                .context(report_context);
            report.error().jump_out()?;
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
    let node = scan_rule_node(
        input,
        stores,
        execution,
        UnexpandablePrimitive::HRule,
        context,
    )?;
    append_vertical_contribution(nest, stores, node);
    nest.current_list_mutation()
        .set_prev_depth(crate::mode::ignored_depth(stores));
    Ok(())
}

/// TeX82 §1105's `delete_last`. Shared by the legacy dispatcher
/// (`assignments::mod`) and canonical main control's `ScannedStep::DeleteLast`
/// handler, since `\unpenalty`/`\unkern`/`\unskip` need no operand scan --
/// only this mode/list-sensitive removal.
fn execute_vertical_skip(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<(), ExecError> {
    if nest.current_mode() == Mode::Horizontal {
        end_paragraph_with_fuel(nest, stores, execution.command_fuel())?;
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
        UnexpandablePrimitive::VSkip => scan_glue_id(input, stores, execution, false, context)?,
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

fn apply_shift(
    node: &mut Node,
    primitive: UnexpandablePrimitive,
    amount: Scaled,
) -> Result<(), ExecError> {
    let delta = match primitive {
        UnexpandablePrimitive::Lower | UnexpandablePrimitive::MoveRight => amount,
        UnexpandablePrimitive::Raise | UnexpandablePrimitive::MoveLeft => -amount,
        _ => unreachable!("caller restricts shift primitives"),
    };
    apply_box_shift_delta(node, delta)
}
