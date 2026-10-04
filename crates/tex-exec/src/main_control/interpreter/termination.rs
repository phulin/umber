//! Stop commands enter page ejection or the actual one-time job boundary.
use super::*;

pub(super) fn stop<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    dump: bool,
) -> Result<MainControlStep, ExecError> {
    let all_over = processor.with_execution_owners(|context, _, _, _, _, _, _, _, _| {
        crate::page_output::job_is_all_over(context)
    });
    if !all_over {
        processor
            .back_consumed_word(spelling, exceptional)
            .map_err(command_error)?;
        report_scan(processor)?;
        return processor.with_execution_owners(
            |context, command, effects, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                semantic::execute_eject_residual_page(context, command, effects)
            },
        );
    }
    let incomplete = processor.final_cleanup();
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let result = semantic::execute_end(context, execution.initex, dump)?;
        execution.pending_job_end = Some((dump, incomplete));
        Ok(result)
    })
}
