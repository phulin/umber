//! Native box construction and group closure.
use super::super::*;

pub(in crate::main_control) fn execute_illegal_box_shift<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<MainControlStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_begin_shipout<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    boxes: &mut ReplayBoxes<G>,
) -> Result<MainControlStep, ExecError> {
    {
        boxes.pending_shipout = Some(stores.begin_page_node_region());
        Ok(MainControlStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
pub(in crate::main_control) fn execute_set_box<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    active_discretionaries: &[ActiveDiscretionary],
    active_math_choices: &[usize],
    active_math_fields: &mut [ActiveMathFieldTarget],
    prepared_dvi_pages: &mut PreparedDviPages,
    target: &mut SetBoxTarget,
    path: &mut ScannedSetBoxPath,
) -> Result<MainControlStep, ExecError> {
    {
        // §1214's `<Adjust for the setting of \globaldefs>` runs inside
        // `prefixed_command`, before §1241 scans the box, so `global` in
        // §1241's `if global then n:=256+cur_val` is the *effective*
        // scope. Resolving it at `box_end` instead would read
        // `\globaldefs` as the box body left it.
        // TeX82 §§1074 and 1077 remove `\lastbox` from the current
        // list before storing that box in the destination register. Keep
        // the same ownership boundary: the source-list rewrite must
        // settle before the destination construction suffix opens, or
        // sealing that suffix would transfer the live source descriptor
        // into durable box ownership along with the selected box.
        if let ScannedSetBoxPath::Payload(ScannedBoxShiftPayload::LastBox { error_context }) = path
        {
            let state = &*command.state;
            let removed: Option<crate::box_runtime::RemovedLastBox> =
                crate::box_runtime::take_last_box_with_segment(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                    |stores| render_diagnostic_context(state, stores, **error_context),
                )?;
            if let Some(removed) = &removed
                && let (Some(root), Some(metadata)) =
                    (removed.root, removed.migration_metadata.as_ref())
                && stores.can_transfer_interleaved_page_box(root, metadata, removed.shift_reset)
            {
                commit_interleaved_set_box_target(
                    PendingSetBox {
                        target: *target,
                        region: stores.begin_page_node_region(),
                        origin: tex_state::page_node_arena::BuiltBoxOrigin::LastBox,
                    },
                    root,
                    metadata.clone(),
                    removed.shift_reset,
                    stores,
                    command,
                );
                return Ok(MainControlStep::Continue);
            }
            let node = removed.map(|removed| removed.node);
            boxes.pending_setbox = Some(PendingSetBox {
                target: *target,
                region: stores.begin_page_node_region(),
                origin: tex_state::page_node_arena::BuiltBoxOrigin::LastBox,
            });
            let context = boxes.take_box_context(false);
            box_end(
                context,
                node,
                modes,
                stores,
                command.state,
                command.diagnostic_effects,
                command.fuel,
                command.prepared_shipout,
                command.assignment_receipts.as_deref_mut(),
            )?;
            return Ok(MainControlStep::Continue);
        }

        boxes.pending_setbox = Some(PendingSetBox {
            target: *target,
            region: stores.begin_page_node_region(),
            origin: tex_state::page_node_arena::BuiltBoxOrigin::SetBoxConstruction,
        });
        match path {
            ScannedSetBoxPath::Forbidden { error_context } => {
                if let BoxContext::SetBox(pending) = boxes.take_box_context(false) {
                    stores
                        .release_page_node_region(pending.region)
                        .expect("rejected setbox releases its empty page region");
                }
                report_improper_setbox(
                    std::mem::take(error_context),
                    stores,
                    command.diagnostic_effects,
                )?;
            }
            ScannedSetBoxPath::Payload(payload) => match payload {
                ScannedBoxShiftPayload::Missing => {
                    if let BoxContext::SetBox(pending) = boxes.take_box_context(false) {
                        stores
                            .release_page_node_region(pending.region)
                            .expect("missing setbox payload releases its page region");
                    }
                    let context = command.state.output_open_context(&*stores);
                    report_improper_setbox(context, stores, command.diagnostic_effects)?;
                }
                ScannedBoxShiftPayload::BoxRegister { index, copy } => {
                    let moves = if *copy {
                        let target = &boxes
                            .pending_setbox
                            .as_ref()
                            .expect("setbox target remains pending")
                            .target;
                        target.index == *index
                            && stores.self_copy_moves_box_register(
                                *index,
                                assignment_scope(target.global),
                            )
                    } else {
                        stores.can_take_unique_box_source(*index)
                    };
                    if moves {
                        let pending = boxes
                            .pending_setbox
                            .take()
                            .expect("setbox target remains pending");
                        commit_unique_register_take_target(pending, *index, stores, command);
                        return Ok(MainControlStep::Continue);
                    }
                    if !*copy
                        && stores.can_assign_output_carrier_take(*index)
                        && boxes.active_boxes.is_empty()
                        && active_alignment.is_none()
                        && active_discretionaries.is_empty()
                        && active_math_choices.is_empty()
                        && active_math_fields.is_empty()
                        && let Some(modes) = modes.preflight_page_region_succession(stores)
                    {
                        let pending = boxes
                            .pending_setbox
                            .take()
                            .expect("setbox target remains pending");
                        match commit_output_carrier_take_target(
                                pending, modes, stores, command,
                            ) {
                                Ok(()) => return Ok(MainControlStep::Continue),
                                Err(declined) => match *declined {
                                    (pending, tex_state::page_node_arena::OutputCarrierTakeError::UnsupportedGeometry) => {
                                        boxes.pending_setbox = Some(pending);
                                    }
                                    (_pending, tex_state::page_node_arena::OutputCarrierTakeError::Promotion(_error)) => {
                                        return Err(ExecError::Fatal(FatalError::confusion(
                                            "output carrier transfer",
                                        )));
                                    }
                                },
                            }
                    }
                    boxes
                        .pending_setbox
                        .as_mut()
                        .expect("setbox target remains pending")
                        .origin = if *copy {
                        tex_state::page_node_arena::BuiltBoxOrigin::SetBoxRegisterCopy
                    } else {
                        #[cfg(feature = "profiling")]
                        {
                            let mode_roots_clear =
                                modes.preflight_page_region_succession(stores).is_some();
                            stores.profile_box_register_take_decline(
                                *index,
                                mode_roots_clear,
                                boxes.active_boxes.is_empty(),
                            )
                        }
                        #[cfg(not(feature = "profiling"))]
                        {
                            tex_state::page_node_arena::BuiltBoxOrigin::SetBoxRegisterTake
                        }
                    };
                    let id = read_box_register(*index, *copy, stores);
                    let node = crate::box_runtime::first_box_node(stores, id);
                    let context = boxes.take_box_context(false);
                    box_end(
                        context,
                        node,
                        modes,
                        stores,
                        command.state,
                        command.diagnostic_effects,
                        command.fuel,
                        command.prepared_shipout,
                        command.assignment_receipts.as_deref_mut(),
                    )?;
                }
                ScannedBoxShiftPayload::LastBox { .. } => {
                    unreachable!("setbox lastbox is ordered before its destination suffix")
                }
                ScannedBoxShiftPayload::VSplit(split) => {
                    boxes
                        .pending_setbox
                        .as_mut()
                        .expect("setbox target remains pending")
                        .origin = tex_state::page_node_arena::BuiltBoxOrigin::SetBoxVSplit;
                    if let Some(context) = &split.missing_to_context {
                        report_missing_vsplit_to(context, command.diagnostic_effects, stores)?;
                    }
                    let diagnostic_context =
                        command_diagnostic_context(command.state, command.output_routine_active);
                    let mut geometry = pack_geometry_sink(
                        command.state,
                        reborrow_observer(&mut command.observations),
                    );
                    let node = crate::box_runtime::split_vbox_register(
                        stores,
                        command.diagnostic_effects,
                        &mut geometry,
                        &diagnostic_context,
                        split.index,
                        split.height,
                        &split.split_context,
                    )?;
                    let context = boxes.take_box_context(false);
                    box_end(
                        context,
                        node,
                        modes,
                        stores,
                        command.state,
                        command.diagnostic_effects,
                        command.fuel,
                        command.prepared_shipout,
                        command.assignment_receipts.as_deref_mut(),
                    )?;
                }
                ScannedBoxShiftPayload::Construction(construction) => begin_replay_box(
                    *construction,
                    boxes.pending_setbox.take(),
                    None,
                    modes,
                    stores,
                    boxes,
                    command.state,
                    command.diagnostic_effects,
                    command.fuel,
                    command.assignment_receipts.as_deref_mut(),
                )?,
            },
        }
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_box_shift<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    shift: &mut ScannedBoxShift,
) -> Result<MainControlStep, ExecError> {
    apply_box_shift(shift, command, modes, stores, boxes)
}
