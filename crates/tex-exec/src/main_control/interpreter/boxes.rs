//! Package live boxes after native unsave, directly into their final owner.
use super::*;

pub(super) fn begin<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let construction = processor
        .scan_box_construction(primitive)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            begin_replay_box(
                construction,
                None,
                None,
                &mut execution.modes,
                context,
                &mut execution.boxes,
                command,
                effects,
                fuel,
                receipts.as_mut(),
            )?;
            command.publish_named_token_list_pushes(context, effects, observer);
            Ok(MainControlStep::Continue)
        },
    )
}

pub(super) fn close<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let (kind, group, packing) =
        processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
            let active = execution
                .boxes
                .active_boxes
                .last()
                .expect("selected live box");
            (active.kind, active.group_kind, active.packing)
        });
    if let ReplayBoxKind::Insert(class, pre) = kind {
        return close_insertion(processor, class, pre);
    }
    let (children, max_depth) = processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            if !kind.horizontal() {
                let diagnostic =
                    command_diagnostic_context(command, execution.boxes.output_routine_active);
                let mut geometry = pack_geometry_sink(command, observer);
                crate::paragraph_end::end_paragraph_with_fuel(
                    &mut execution.modes,
                    context,
                    effects,
                    &mut geometry,
                    diagnostic,
                    fuel,
                )?;
            }
            let mut level = crate::box_runtime::commit_current_list(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            Ok::<_, ExecError>((
                level.list_mutation().take_nodes(),
                context.dimen_param(DimenParam::BOX_MAX_DEPTH),
            ))
        },
    )?;
    groups::finish_native_group_leave(processor, group, "box group")?;
    let step = processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            let diagnostic = crate::pack_report::PackDiagnosticContext {
                current_line: command
                    .current_file_line_number()
                    .try_into()
                    .unwrap_or(i32::MAX),
                pack_begin_line: 0,
                output_routine_active: execution.boxes.output_routine_active,
            };
            let mut geometry = pack_geometry_sink(command, observer);
            let node = if kind.horizontal() {
                Node::HList(crate::box_runtime::hpack_with_overfull_rule(
                    context,
                    effects,
                    &mut geometry,
                    &diagnostic,
                    children,
                    packing,
                ))
            } else {
                let mut params = crate::packing_params::vpack_params(context);
                params.box_max_depth = max_depth;
                Node::VList(match kind {
                    ReplayBoxKind::VBox | ReplayBoxKind::VCenter => {
                        crate::packing_params::vpack(
                            context,
                            effects,
                            &mut geometry,
                            &diagnostic,
                            children,
                            packing,
                            params,
                        )
                        .node
                    }
                    ReplayBoxKind::VTop => {
                        crate::packing_params::vtop(
                            context,
                            effects,
                            &mut geometry,
                            &diagnostic,
                            children,
                            packing,
                            params,
                        )
                        .node
                    }
                    _ => unreachable!("selected vertical package"),
                })
            };
            drop(geometry);
            if kind == ReplayBoxKind::VCenter {
                let boxed = context.publish_page_nodes([node]);
                execution.modes.current_list_mutation().push(
                    context,
                    Node::MathNoad(MathNoad::new(NoadKind::VCenter, MathField::SubBox(boxed))),
                );
                execution
                    .boxes
                    .pop_box(context)
                    .expect("live vcenter owner");
                return Ok::<_, ExecError>(MainControlStep::Continue);
            }
            let active = execution
                .boxes
                .active_boxes
                .last()
                .expect("live package destination");
            if let Some(leader) = active.leader_kind {
                let payload =
                    crate::box_runtime::payload_from_node(node).ok_or(ExecError::MissingToken {
                        context: "leader box payload",
                    })?;
                execution.boxes.pending_leader = Some((leader, payload));
                execution.boxes.pop_box(context).expect("live leader owner");
            } else if active.shipout_region.is_some() {
                let active = execution
                    .boxes
                    .pop_box(context)
                    .expect("live shipout owner");
                debug_assert!(execution.prepared_shipout.is_none());
                execution.prepared_shipout = Some(PreparedShipout {
                    source: PreparedShipoutSource::Page(node),
                    region: active.shipout_region,
                });
            } else if active.target.is_some() {
                let active = execution
                    .boxes
                    .pop_box(context)
                    .expect("live assignment owner");
                let boxed = context.publish_page_nodes([node]);
                commit_set_box_target(
                    active.target.expect("live box target"),
                    Some(boxed),
                    context,
                    effects,
                    receipts.as_mut(),
                );
            } else {
                let active = execution.boxes.pop_box(context).expect("live append owner");
                let mut node = node;
                if let Some(shift) = active.shift {
                    crate::box_runtime::apply_box_shift_delta(&mut node, shift.delta)?;
                }
                if let Some(start) = active.box_segment_start {
                    crate::box_runtime::append_box_node_with_segment(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                        node,
                        start,
                        active.migration_segments,
                    )?;
                } else {
                    crate::box_runtime::append_box_node_to_current_list(
                        &mut execution.modes,
                        context,
                        effects,
                        node,
                        fuel,
                    )?;
                }
                crate::vertical::build_page_if_outer_vertical(
                    &mut execution.modes,
                    context,
                    effects,
                    command,
                )?;
            }
            Ok(MainControlStep::Continue)
        },
    )?;
    let leader = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        execution.boxes.pending_leader.is_some()
    });
    if leader {
        leaders::finish_leader(processor)
    } else {
        Ok(step)
    }
}

fn close_insertion<G>(
    processor: &mut Processor<'_, '_, G>,
    class: u16,
    pre: bool,
) -> Result<MainControlStep, ExecError> {
    let (page_context, pack_context, split_top_skip, split_max_depth, floating_penalty) = processor
        .with_execution_owners(
            |context, command, effects, fuel, observer, execution, _, _, _| {
                let page_context = command.output_open_context(context);
                let diagnostic =
                    command_diagnostic_context(command, execution.boxes.output_routine_active);
                let pack_context = diagnostic.packing();
                let mut geometry = pack_geometry_sink(command, observer);
                crate::paragraph_end::end_paragraph_with_fuel(
                    &mut execution.modes,
                    context,
                    effects,
                    &mut geometry,
                    diagnostic,
                    fuel,
                )?;
                let skip = context
                    .glue_param(GlueParam::SPLIT_TOP_SKIP)
                    .map(|id| context.glue(id))
                    .unwrap_or(GlueSpec::ZERO);
                Ok::<_, ExecError>((
                    page_context,
                    pack_context,
                    skip,
                    context.dimen_param(DimenParam::SPLIT_MAX_DEPTH),
                    context.int_param(IntParam::FLOATING_PENALTY),
                ))
            },
        )?;
    groups::finish_native_group_leave(processor, GroupKind::Insert, "insert group")?;
    processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            let mut level = crate::box_runtime::commit_current_list(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            let content = level.list_mutation().take_nodes();
            let params = tex_typeset::VpackParams {
                box_max_depth: Scaled::MAX_DIMEN,
                ..crate::packing_params::vpack_params(context)
            };
            let mut geometry = pack_geometry_sink(command, observer);
            let packed = crate::packing_params::vpack(
                context,
                effects,
                &mut geometry,
                &pack_context,
                content,
                PackSpec::Natural,
                params,
            );
            drop(geometry);
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            let node = if class == 255 {
                Node::Adjust(tex_state::node::AdjustNode { content, pre })
            } else {
                Node::Ins {
                    class,
                    size: packed
                        .node
                        .height
                        .checked_add(packed.node.depth)
                        .ok_or(ExecError::ArithmeticOverflow)?,
                    split_top_skip,
                    split_max_depth,
                    floating_penalty,
                    content,
                }
            };
            crate::vertical::append_vertical_contribution(&mut execution.modes, context, node);
            let inserted = execution
                .boxes
                .pop_box(context)
                .expect("live insertion owner");
            if let Some(start) = inserted.migration_segment_start {
                let segment = context.close_page_box_segment(start);
                if !segment.is_empty() {
                    execution
                        .boxes
                        .active_boxes
                        .last_mut()
                        .expect("enclosing hbox")
                        .migration_segments
                        .push(segment);
                }
            }
            execution.main_loop_active = false;
            context.publish_diagnostic_effects_before_synchronous_print(effects);
            crate::vertical::build_page_if_outer_vertical_with_error_context(
                &mut execution.modes,
                context,
                effects,
                &page_context,
            )?;
            Ok(MainControlStep::Continue)
        },
    )
}

pub(super) fn unbox<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
) -> Result<MainControlStep, ExecError> {
    let index = processor
        .scan_profile_register_index()
        .map_err(command_error)?;
    let coordinate = processor.diagnostic_context_coordinate();
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::execute_scanned_unbox_with_error_context(
            primitive,
            index,
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
        )
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

pub(super) fn discards<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::execute_scanned_saved_vertical_discards(
            primitive,
            &mut execution.modes,
            context,
            effects,
            fuel,
        )?;
        Ok(MainControlStep::Continue)
    })
}
