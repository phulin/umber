//! Direct boxes semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_v_split<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    prepared_dvi_pages: &mut PreparedDviPages,
    split: &mut ScannedVSplit,
) -> Result<ReplayStep, ExecError> {
    {
        if let Some(context) = &split.missing_to_context {
            report_missing_vsplit_to(context, command.diagnostic_effects, stores)?;
        }
        let isolated_append = boxes.pending_setbox.is_none()
            && matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal | Mode::InternalVertical
            )
            && stores.box_register(split.index).is_some();
        if isolated_append {
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
        }
        let start = isolated_append.then(|| stores.begin_page_node_region());
        let diagnostic_context =
            command_diagnostic_context(command.state, command.output_routine_active);
        let mut geometry =
            pack_geometry_sink(command.state, reborrow_observer(&mut command.observations));
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
        match (start, node) {
            (Some(start), Some(node)) => {
                debug_assert!(matches!(context, BoxContext::Append(_)));
                crate::box_runtime::append_box_node_with_segment(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                    node,
                    start,
                    Vec::new(),
                )?;
            }
            (Some(start), None) => {
                stores
                    .release_page_node_region(start)
                    .expect("void split releases its construction boundary");
                box_end(context, None, modes, stores, prepared_dvi_pages, command)?;
            }
            (None, node) => {
                box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
            }
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_box_dimension_assignment<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    dimension: &mut tex_state::BoxDimension,
    value: &mut Scaled,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    {
        // `Universe::set_box_dimension{,_global}` share one body: TeX82
        // §1055's `alter_box_dimen` mutates the visible box node
        // directly rather than through the save stack, so the assignment
        // prefix does not change which binding level is affected.
        let _ = global;
        AssignmentCommitter::new(stores, command.diagnostic_effects)
            .unscoped(|stores| assign_box_dimension(stores, *index, *dimension, *value));
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_show_box<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
) -> Result<ReplayStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::execute_showbox(
            stores,
            command.diagnostic_effects,
            *index,
            command.state.profile(),
        );
        command.defer_show_completion(true, context);
        Ok(ReplayStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
pub(in crate::main_control) fn execute_box_register<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    prepared_dvi_pages: &mut PreparedDviPages,
    index: &mut u16,
    copy: &mut bool,
    ships_out: &mut bool,
) -> Result<ReplayStep, ExecError> {
    {
        let isolated_append = !*ships_out
            && boxes.pending_setbox.is_none()
            && matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal | Mode::InternalVertical
            )
            && stores.box_register(*index).is_some();
        if isolated_append {
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
        }
        let start = isolated_append.then(|| stores.begin_page_node_region());
        // `\shipout\copy` reads the register in place: shipout only
        // expands deferred token lists, so the register cannot change
        // before the staged page is released.
        let pinned = (*ships_out && *copy)
            .then(|| stores.pin_box_for_shipout(*index))
            .flatten();
        let node = match pinned {
            Some(node) => Some(node),
            None => {
                let id = read_box_register(*index, *copy, stores, command);
                crate::box_runtime::first_box_node(stores, id)
            }
        };
        let context = boxes.take_box_context(*ships_out);
        if let Some(start) = start {
            debug_assert!(matches!(context, BoxContext::Append(_)));
            crate::box_runtime::append_box_node_with_segment(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                node.expect("nonvoid box register retains its wrapper"),
                start,
                Vec::new(),
            )?;
            crate::vertical::build_page_if_outer_vertical(
                modes,
                stores,
                command.diagnostic_effects,
                &*command.state,
            )?;
        } else {
            box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_unbox<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
    index: &mut u16,
    error_context: &mut tex_command::DiagnosticContextCoordinate,
) -> Result<ReplayStep, ExecError> {
    {
        let state = &*command.state;
        crate::box_runtime::execute_scanned_unbox_with_error_context(
            *primitive,
            *index,
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            |stores| {
                state
                    .render_diagnostic_context(*error_context, stores)
                    .map_err(|_| {
                        ExecError::Command(tex_command::CommandError::StaleDiagnosticContext)
                    })
            },
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_saved_vertical_discards<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
) -> Result<ReplayStep, ExecError> {
    {
        crate::box_runtime::execute_scanned_saved_vertical_discards(
            *primitive,
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_last_box<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    prepared_dvi_pages: &mut PreparedDviPages,
    error_context: &mut tex_command::DiagnosticContextCoordinate,
) -> Result<ReplayStep, ExecError> {
    {
        let state = &*command.state;
        let node = crate::box_runtime::take_last_box(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            |stores| {
                state
                    .render_diagnostic_context(*error_context, stores)
                    .map_err(|_| {
                        ExecError::Command(tex_command::CommandError::StaleDiagnosticContext)
                    })
            },
        )?;
        let context = boxes.take_box_context(false);
        box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
        Ok(ReplayStep::Continue)
    }
}
