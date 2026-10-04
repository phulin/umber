//! Concrete operand scans and mutations in the actual persistent processor.
use super::*;

pub(super) fn count<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_integer().map_err(command_error)?.value;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_count(context, effects, receipts, index, value, global)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn dimension<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_dimension().map_err(command_error)?;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_dimen(context, effects, receipts, index, value, global)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn integer_parameter<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_integer().map_err(command_error)?.value;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_int_param(context, effects, command, receipts, index, value, global)
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn dimension_parameter<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_dimension().map_err(command_error)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_dimen_param(context, effects, command, receipts, index, value, global)
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn glue_register<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    mu: bool,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_glue(mu).map_err(command_error)?;
    let source_identity = processor.scanned_glue_identity();
    let source_register = processor.scanned_glue_register();
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            let reassigning = glue_identity::glue_pointer_reassigned(
                context,
                mu,
                index,
                value,
                source_identity,
                source_register,
                global,
                &execution.skip_pointer_sources,
                &execution.muskip_pointer_sources,
            );
            let redundant = context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 && reassigning;
            let step = semantic::execute_skip(
                context,
                effects,
                receipts,
                index,
                value,
                global,
                mu,
                redundant,
                reassigning,
            )?;
            if !redundant {
                let physical = if mu {
                    context.muskip(index)
                } else {
                    context.glue_register(index).ok().flatten()
                };
                if let Some(physical) = physical {
                    let pointers = if mu {
                        &mut execution.muskip_pointer_sources
                    } else {
                        &mut execution.skip_pointer_sources
                    };
                    if pointers.len() <= usize::from(index) {
                        pointers.resize(usize::from(index) + 1, None);
                    }
                    pointers[usize::from(index)] = Some((physical, source_identity));
                }
            }
            Ok::<ReplayStep, ExecError>(step)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn definition<G>(
    processor: &mut Processor<'_, '_, G>,
    expanded: bool,
    flags: MeaningFlags,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    let definition = processor
        .scan_macro_definition(expanded)
        .map_err(command_error)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, _, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            definition_assignments::apply_macro_definition(
                definition.target,
                definition.definition,
                flags,
                global,
                context,
                effects,
                observer,
                receipts,
            )
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn bind<G>(
    processor: &mut Processor<'_, '_, G>,
    target: Symbol,
    meaning: tex_state::MeaningWord<G>,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            execution.main_loop_active = false;
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            definition_assignments::apply_let(target, meaning, global, context, effects, receipts)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}
