//! TeX's scan_box selects operands directly into their actual destination.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Destination {
    List(Scaled),
    Register(SetBoxTarget),
    Shipout,
    Leader(GlueKind),
}

pub(super) fn set_box<G>(
    processor: &mut Processor<'_, '_, G>,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let index = processor
        .scan_profile_register_index()
        .map_err(command_error)?;
    processor.scan_optional_equals().map_err(command_error)?;
    report_scan(processor)?;
    let forbidden = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        execution.set_box_forbidden_depth != 0
    });
    if forbidden {
        processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
            report_improper_setbox(command.output_open_context(context), context, effects)
        })?;
        recovery::finish_error_dialogue(processor)?;
        assignments::complete(processor, receipts)?;
        return Ok(MainControlStep::Continue);
    }
    let step = scan(
        processor,
        Destination::Register(SetBoxTarget { index, global }),
        receipts,
    )?;
    assignments::complete(processor, receipts)?;
    Ok(step)
}

pub(super) fn scan<G>(
    processor: &mut Processor<'_, '_, G>,
    destination: Destination,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let mut result = None;
    let status = processor
        .consume_main_control(&mut tex_command::terminal_consumer::<
            Processor<'_, '_, G>,
            G,
            _,
        >(
            |processor, spelling, meaning, _code, _, exceptional| {
                let code = meaning.control_code();
                if matches!(code, codes::RELAX | codes::CHAR_SPACE) {
                    return Ok(ControlFlow::Continue(()));
                }
                result = Some((|| match code {
                    codes::PRIMITIVE_H_BOX | codes::PRIMITIVE_V_BOX | codes::PRIMITIVE_V_TOP => {
                        let primitive = match code {
                            codes::PRIMITIVE_H_BOX => UnexpandablePrimitive::HBox,
                            codes::PRIMITIVE_V_BOX => UnexpandablePrimitive::VBox,
                            _ => UnexpandablePrimitive::VTop,
                        };
                        construction(processor, primitive, destination, receipts)
                    }
                    codes::PRIMITIVE_BOX | codes::PRIMITIVE_COPY => register(
                        processor,
                        code == codes::PRIMITIVE_COPY,
                        destination,
                        receipts,
                    ),
                    codes::PRIMITIVE_LAST_BOX => last(processor, destination, receipts),
                    codes::PRIMITIVE_V_SPLIT => split(processor, destination, receipts),
                    codes::PRIMITIVE_H_RULE | codes::PRIMITIVE_V_RULE
                        if matches!(destination, Destination::Leader(_)) =>
                    {
                        let primitive = if code == codes::PRIMITIVE_H_RULE {
                            UnexpandablePrimitive::HRule
                        } else {
                            UnexpandablePrimitive::VRule
                        };
                        let rule = processor.scan_rule_spec(primitive).map_err(command_error)?;
                        report_scan(processor)?;
                        processor.with_execution_owners(
                            |context, command, effects, fuel, _, execution, _, _, _| {
                                finish(
                                    destination,
                                    Some(Node::Rule {
                                        width: rule.width,
                                        height: rule.height,
                                        depth: rule.depth,
                                    }),
                                    context,
                                    command,
                                    effects,
                                    fuel,
                                    execution,
                                    receipts,
                                )
                            },
                        )?;
                        leaders::finish_leader(processor)
                    }
                    _ => {
                        processor
                            .back_consumed_word(spelling, exceptional)
                            .map_err(command_error)?;
                        missing(processor)
                    }
                })());
                Ok(ControlFlow::Break(()))
            },
        ))
        .map_err(command_error)?;
    match result {
        Some(result) => result,
        None if status == tex_command::DeliveryStatus::End => missing(processor),
        None => finish_input_boundary(processor, status),
    }
}

fn missing<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        report_missing_box(command, effects, context)
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

fn construction<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    destination: Destination,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let construction = processor
        .scan_box_construction(primitive)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            if let Destination::Leader(kind) = destination {
                let box_kind = ReplayBoxKind::from_scanned(construction.kind);
                let packing = match construction.packing {
                    ScannedPackingSpec::Natural => PackSpec::Natural,
                    ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
                    ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
                };
                enter_group(context, command, effects, box_kind.group_kind());
                execution.modes.push_at_line(
                    if box_kind.horizontal() {
                        Mode::RestrictedHorizontal
                    } else {
                        Mode::InternalVertical
                    },
                    i32::try_from(command.current_file_line_number()).unwrap_or(i32::MAX),
                )?;
                if !box_kind.horizontal() {
                    commit_box_normal_paragraph(
                        &mut execution.modes,
                        context,
                        effects,
                        receipts.as_mut(),
                    );
                }
                execution.boxes.push_box(
                    context,
                    ActiveReplayBox {
                        target: None,
                        shipout_region: None,
                        box_segment_start: None,
                        migration_segment_start: None,
                        migration_segments: Vec::new(),
                        kind: box_kind,
                        group_kind: box_kind.group_kind(),
                        packing,
                        leader_kind: Some(kind),
                        shift: None,
                    },
                );
                schedule_everybox(command, context, box_kind.horizontal());
            } else {
                let target = match destination {
                    Destination::Register(target) => Some(PendingSetBox {
                        target,
                        region: context.begin_page_node_region(),
                        origin: tex_state::page_node_arena::BuiltBoxOrigin::SetBoxConstruction,
                    }),
                    _ => None,
                };
                let shipout = matches!(destination, Destination::Shipout)
                    .then(|| context.begin_page_node_region());
                let axis = if matches!(
                    execution.modes.current_mode(),
                    Mode::Vertical | Mode::InternalVertical
                ) {
                    BoxShiftAxis::Horizontal
                } else {
                    BoxShiftAxis::Vertical
                };
                begin_replay_box(
                    construction,
                    target,
                    shipout,
                    &mut execution.modes,
                    context,
                    &mut execution.boxes,
                    command,
                    effects,
                    fuel,
                    receipts.as_mut(),
                )?;
                if let Destination::List(delta) = destination
                    && delta != Scaled::from_raw(0)
                {
                    execution
                        .boxes
                        .active_boxes
                        .last_mut()
                        .expect("constructed box destination")
                        .shift = Some(ReplayBoxShift { delta, axis });
                }
            }
            command.publish_named_token_list_pushes(context, effects, observer);
            Ok(MainControlStep::Continue)
        },
    )
}

pub(super) fn register<G>(
    processor: &mut Processor<'_, '_, G>,
    copy: bool,
    destination: Destination,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let index = processor
        .scan_profile_register_index()
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        if let Destination::Register(target) = destination {
            if (!copy && context.can_take_unique_box_source(index))
                || (copy
                    && target.index == index
                    && context.self_copy_moves_box_register(index, assignment_scope(target.global)))
            {
                let receipt = AssignmentCommitter::new(context, effects).box_register_unique_take(
                    index,
                    target.index,
                    target.global,
                    |context, taken| {
                        context
                            .finish_unique_box_register_take(
                                taken,
                                target.index,
                                assignment_scope(target.global),
                            )
                            .expect("exclusive register source belongs to the actual destination");
                    },
                );
                record_assignment(receipts, target, receipt);
                return Ok(());
            }
            if !copy
                && context.can_assign_output_carrier_take(index)
                && execution.boxes.active_boxes.is_empty()
                && execution.active_alignment.is_none()
                && execution.active_discretionaries.is_empty()
                && execution.active_math_choices.is_empty()
                && execution.active_math_fields.is_empty()
                && let Some(modes) = execution.modes.preflight_page_region_succession(context)
            {
                match AssignmentCommitter::new(context, effects).box_register_output_carrier(
                    target.index,
                    target.global,
                    modes,
                ) {
                    Ok(receipt) => {
                        record_assignment(receipts, target, receipt);
                        return Ok(());
                    }
                    Err(
                        tex_state::page_node_arena::OutputCarrierTakeError::UnsupportedGeometry,
                    ) => {}
                    Err(tex_state::page_node_arena::OutputCarrierTakeError::Promotion(_)) => {
                        return Err(ExecError::Fatal(FatalError::confusion(
                            "output carrier transfer",
                        )));
                    }
                }
            }
        }
        let origin = if copy {
            tex_state::page_node_arena::BuiltBoxOrigin::SetBoxRegisterCopy
        } else {
            tex_state::page_node_arena::BuiltBoxOrigin::SetBoxRegisterTake
        };
        let region = begin_operand_region(destination, index, context, effects, fuel, execution)?;
        let node = if matches!(destination, Destination::Shipout) && copy {
            context.pin_box_for_shipout(index).or_else(|| {
                let root = read_box_register(index, copy, context);
                crate::box_runtime::first_box_node(context, root)
            })
        } else {
            let root = read_box_register(index, copy, context);
            crate::box_runtime::first_box_node(context, root)
        };
        finish_region(
            destination,
            node,
            region,
            origin,
            context,
            command,
            effects,
            fuel,
            execution,
            receipts,
        )
    })?;
    if matches!(destination, Destination::Leader(_)) {
        leaders::finish_leader(processor)
    } else {
        Ok(MainControlStep::Continue)
    }
}

fn record_assignment(
    receipts: &mut Option<Vec<MutationRecord>>,
    target: SetBoxTarget,
    receipt: bool,
) {
    if receipt && let Some(receipts) = receipts {
        receipts.push(MutationRecord {
            target: MutationTarget::Register,
            key: ObservationValue::Name(format!("box:{}", target.index)),
            value: ObservationValue::Name("occupied".into()),
            global: target.global,
        });
    }
}

pub(super) fn last<G>(
    processor: &mut Processor<'_, '_, G>,
    destination: Destination,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let coordinate = processor.diagnostic_context_coordinate();
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let removed = crate::box_runtime::take_last_box_with_segment(
            &mut execution.modes,
            context,
            effects,
            fuel,
            |context| {
                command
                    .render_diagnostic_context(coordinate, context)
                    .map_err(|_| {
                        ExecError::Command(tex_command::CommandError::StaleDiagnosticContext)
                    })
            },
        )?;
        if let Destination::Register(target) = destination
            && let Some(removed) = &removed
            && let (Some(root), Some(metadata)) =
                (removed.root, removed.migration_metadata.as_ref())
            && context.can_transfer_interleaved_page_box(root, metadata, removed.shift_reset)
        {
            let region = context.begin_page_node_region();
            let receipt = AssignmentCommitter::new(context, effects).box_register(
                target.index,
                Some(&root),
                target.global,
                |context| {
                    context
                        .assign_interleaved_page_box(
                            target.index,
                            root,
                            metadata.clone(),
                            removed.shift_reset,
                            region,
                            assignment_scope(target.global),
                        )
                        .expect("preflighted lastbox transfers into its actual register");
                },
            );
            record_assignment(receipts, target, receipt);
            return Ok(());
        }
        finish(
            destination,
            removed.map(|removed| removed.node),
            context,
            command,
            effects,
            fuel,
            execution,
            receipts,
        )
    })?;
    recovery::finish_error_dialogue(processor)?;
    if matches!(destination, Destination::Leader(_)) {
        leaders::finish_leader(processor)
    } else {
        Ok(MainControlStep::Continue)
    }
}

pub(super) fn split<G>(
    processor: &mut Processor<'_, '_, G>,
    destination: Destination,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let split = processor.scan_vsplit().map_err(command_error)?;
    report_scan(processor)?;
    if let Some(display) = &split.missing_to_context {
        processor.with_execution_owners(|context, _, effects, _, _, _, _, _, _| {
            report_missing_vsplit_to(display, effects, context)
        })?;
        recovery::finish_error_dialogue(processor)?;
    }
    processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            let region =
                begin_operand_region(destination, split.index, context, effects, fuel, execution)?;
            let diagnostic =
                command_diagnostic_context(command, execution.boxes.output_routine_active);
            let mut geometry = pack_geometry_sink(command, observer);
            let node = crate::box_runtime::split_vbox_register(
                context,
                effects,
                &mut geometry,
                &diagnostic,
                split.index,
                split.height,
                &split.split_context,
            )?;
            finish_region(
                destination,
                node,
                region,
                tex_state::page_node_arena::BuiltBoxOrigin::SetBoxVSplit,
                context,
                command,
                effects,
                fuel,
                execution,
                receipts,
            )
        },
    )?;
    if matches!(destination, Destination::Leader(_)) {
        leaders::finish_leader(processor)
    } else {
        Ok(MainControlStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // borrows actual owners for a selected box destination
fn finish<G>(
    destination: Destination,
    node: Option<Node>,
    context: &mut CommandContext<'_, G>,
    command: &CommandState<G>,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    execution: &mut ExecutionState<G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<(), ExecError> {
    let region = matches!(destination, Destination::Register(_) | Destination::Shipout)
        .then(|| context.begin_page_node_region());
    finish_region(
        destination,
        node,
        region,
        tex_state::page_node_arena::BuiltBoxOrigin::LastBox,
        context,
        command,
        effects,
        fuel,
        execution,
        receipts,
    )
}

#[allow(clippy::too_many_arguments)] // borrows actual owners and the construction interval
fn finish_region<G>(
    destination: Destination,
    node: Option<Node>,
    region: Option<tex_state::node_region::PageClosureBuildMark>,
    origin: tex_state::page_node_arena::BuiltBoxOrigin,
    context: &mut CommandContext<'_, G>,
    command: &CommandState<G>,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    execution: &mut ExecutionState<G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<(), ExecError> {
    let destination = match destination {
        Destination::List(delta) => {
            if let Some(region) = region {
                if let Some(mut node) = node {
                    crate::box_runtime::apply_box_shift_delta(&mut node, delta)?;
                    crate::box_runtime::append_box_node_with_segment(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                        node,
                        region,
                        Vec::new(),
                    )?;
                    crate::vertical::build_page_if_outer_vertical(
                        &execution.modes,
                        context,
                        effects,
                        command,
                    )?;
                } else {
                    context
                        .release_page_node_region(region)
                        .expect("void box operand interval");
                }
                return Ok(());
            }
            BoxContext::Append(delta)
        }
        Destination::Register(target) => BoxContext::SetBox(PendingSetBox {
            target,
            region: region.expect("actual register operand interval"),
            origin,
        }),
        Destination::Shipout => {
            BoxContext::ShipOut(region.expect("actual shipout operand interval"))
        }
        Destination::Leader(kind) => {
            if let Some(node) = node {
                execution.boxes.pending_leader = Some((
                    kind,
                    crate::box_runtime::payload_from_node(node).expect("admitted box or rule"),
                ));
            }
            return Ok(());
        }
    };
    box_end(
        destination,
        node,
        &mut execution.modes,
        context,
        command,
        effects,
        fuel,
        &mut execution.prepared_shipout,
        receipts.as_mut(),
    )
}

fn begin_operand_region<G>(
    destination: Destination,
    index: u16,
    context: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    execution: &mut ExecutionState<G>,
) -> Result<Option<tex_state::node_region::PageClosureBuildMark>, ExecError> {
    let isolated = matches!(destination, Destination::List(_))
        && matches!(
            execution.modes.current_mode(),
            Mode::Horizontal | Mode::RestrictedHorizontal | Mode::InternalVertical
        )
        && context.box_register(index).is_some();
    if isolated {
        crate::box_runtime::flush_pending_hchars(&mut execution.modes, context, effects, fuel)?;
    }
    Ok(
        (isolated || matches!(destination, Destination::Register(_) | Destination::Shipout))
            .then(|| context.begin_page_node_region()),
    )
}
