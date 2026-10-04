//! Scanned material is published directly into the actual current list.
use super::*;

pub(super) fn rule<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
) -> Result<MainControlStep, ExecError> {
    let rule = processor.scan_rule_spec(primitive).map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            apply_scanned_rule(
                command,
                effects,
                fuel,
                observer,
                execution.boxes.output_routine_active,
                &mut execution.modes,
                context,
                rule.width,
                rule.height,
                rule.depth,
                primitive == UnexpandablePrimitive::HRule,
            )
        },
    )
}

pub(super) fn fixed_glue<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::append_node_to_current_list(
            &mut execution.modes,
            context,
            effects,
            Node::Glue {
                origin: tex_state::node::GlueSpecOrigin::Owned,
                spec: crate::box_runtime::fixed_infinite_glue(primitive),
                kind: GlueKind::Normal,
                leader: None,
            },
            fuel,
        )?;
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn control_space<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        if matches!(
            execution.modes.current_mode(),
            Mode::Math | Mode::DisplayMath
        ) {
            let spec = crate::box_runtime::control_space_glue_spec(context);
            execution.modes.current_list_mutation().push(
                context,
                Node::Glue {
                    origin: tex_state::node::GlueSpecOrigin::Owned,
                    spec,
                    kind: GlueKind::Normal,
                    leader: None,
                },
            );
        } else {
            crate::box_runtime::append_control_space_with_fuel(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
        }
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn italic_correction<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        if matches!(
            execution.modes.current_mode(),
            Mode::Math | Mode::DisplayMath
        ) {
            execution.modes.current_list_mutation().push(
                context,
                Node::Kern {
                    amount: Scaled::from_raw(0),
                    kind: KernKind::Font,
                },
            );
        } else {
            crate::box_runtime::append_italic_correction_with_fuel(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
        }
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn non_script<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        execution.modes.current_list_mutation().push(
            context,
            Node::Glue {
                origin: tex_state::node::GlueSpecOrigin::SharedZero,
                spec: GlueSpec::ZERO,
                kind: GlueKind::NonScript,
                leader: None,
            },
        );
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn horizontal_glue<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let value = processor.scan_glue(false).map_err(command_error)?;
    let shared_zero = processor.scanned_glue_shared_zero();
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        semantic::execute_horizontal_skip(
            context,
            &mut execution.modes,
            effects,
            fuel,
            value,
            shared_zero,
        )
    })
}

pub(super) fn vertical_glue<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let value = processor.scan_glue(false).map_err(command_error)?;
    let shared_zero = processor.scanned_glue_shared_zero();
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        semantic::execute_vertical_skip(
            context,
            &mut execution.modes,
            effects,
            fuel,
            value,
            shared_zero,
        )
    })
}

pub(super) fn kern<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    let amount = processor.scan_dimension().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        semantic::execute_kern(context, &mut execution.modes, effects, fuel, amount)
    })
}

pub(super) fn penalty<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let amount = processor.scan_integer().map_err(command_error)?.value;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::flush_pending_hchars_with_fuel(
            &mut execution.modes,
            context,
            effects,
            fuel,
        )?;
        crate::vertical::append_vertical_contribution(
            &mut execution.modes,
            context,
            Node::Penalty(amount),
        );
        crate::vertical::build_page_if_outer_vertical(&execution.modes, context, effects, command)?;
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn mu_glue<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let spec = processor.scan_glue(true).map_err(command_error)?;
    let shared_zero = processor.scanned_glue_shared_zero();
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        execution.modes.current_list_mutation().push(
            context,
            Node::Glue {
                origin: if shared_zero {
                    tex_state::node::GlueSpecOrigin::SharedZero
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
                spec,
                kind: GlueKind::MuSkip,
                leader: None,
            },
        );
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn mu_kern<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let amount = processor.scan_mu_dimension().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        execution.modes.current_list_mutation().push(
            context,
            Node::Kern {
                amount,
                kind: KernKind::Mu,
            },
        );
        Ok(MainControlStep::Continue)
    })
}
