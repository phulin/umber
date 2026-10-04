//! Scanned material is published directly into the actual current list.
use super::*;

pub(super) fn horizontal_glue<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
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
) -> Result<ReplayStep, ExecError> {
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

pub(super) fn kern<G>(processor: &mut Processor<'_, '_, G>) -> Result<ReplayStep, ExecError> {
    let amount = processor.scan_dimension().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        semantic::execute_kern(context, &mut execution.modes, effects, fuel, amount)
    })
}

pub(super) fn penalty<G>(processor: &mut Processor<'_, '_, G>) -> Result<ReplayStep, ExecError> {
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
        crate::vertical::build_page_if_outer_vertical(
            &mut execution.modes,
            context,
            effects,
            command,
        )?;
        Ok(ReplayStep::Continue)
    })
}

pub(super) fn mu_glue<G>(processor: &mut Processor<'_, '_, G>) -> Result<ReplayStep, ExecError> {
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
        Ok(ReplayStep::Continue)
    })
}

pub(super) fn mu_kern<G>(processor: &mut Processor<'_, '_, G>) -> Result<ReplayStep, ExecError> {
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
        Ok(ReplayStep::Continue)
    })
}
