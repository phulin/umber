//! Persistent interpretation over the actual scanner and execution owners.
//!
//! Fetch roles change at TeX's own labels. They do not end the processor loan.
//! Every terminal enters the same dispatch; only exceptional delivery carries
//! the reader's diagnostic/observation record.

use super::*;
use std::ops::ControlFlow;

mod accent;
mod arithmetic;
mod assignments;
mod boxes;
mod characters;
mod codes;
mod diagnostics;
mod discretionary;
mod dispatch;
mod driver;
mod fonts;
mod glue_identity;
mod groups;
mod material;
mod math;
mod pdf;
mod prefixes;
mod publication;
mod recovery;
mod streams;
mod termination;
mod trace;

type Processor<'state, 'host, G> =
    CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>;

pub(super) fn run<G>(
    processor: &mut Processor<'_, '_, G>,
    remaining_operations: usize,
    completed_operations: &mut usize,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    while *completed_operations < remaining_operations {
        let lookahead = processor
            .with_execution_owners(|_, _, _, _, _, execution, _, _, _| execution.main_loop_active);
        trace::prepare(processor);
        let mut completion = None;
        let observing = processor.is_observed();
        let mut consume =
            |processor: &mut Processor<'_, '_, G>,
             spelling: TracedTokenWord,
             meaning: &tex_state::meaning::MeaningWord<G>,
             control_sequence: Option<Symbol>,
             exceptional: Option<&tex_command::CurrentCommand<G>>| {
                match dispatch::execute(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                    observation_receipts,
                    0,
                ) {
                    Ok(MainControlStep::Continue) => {
                        *completed_operations += 1;
                        trace::prepare(processor);
                        let (next_lookahead, host_boundary) = processor.with_execution_owners(
                            |context, _, effects, _, _, execution, _, _, _| {
                                (
                                    execution.main_loop_active,
                                    execution.paragraph_checkpoint_cut
                                        || execution.prepared_shipout.is_some()
                                        || effects.has_first_recoverable()
                                        || (!execution.boxes.output_routine_active
                                            && (context.page_fire_up().is_some()
                                                || context
                                                    .page_builder_resume_after_output_pending())),
                                )
                            },
                        );
                        if host_boundary {
                            completion = Some(Ok(MainControlStep::Continue));
                            return Ok(ControlFlow::Break(()));
                        }
                        if !observing
                            && next_lookahead == lookahead
                            && *completed_operations < remaining_operations
                        {
                            return Ok(ControlFlow::Continue(()));
                        }
                    }
                    result => {
                        if result.is_ok() {
                            *completed_operations += 1;
                        }
                        completion = Some(result);
                    }
                }
                Ok(ControlFlow::Break(()))
            };
        let status = if lookahead {
            processor.consume_main_loop_lookahead(&mut consume)
        } else {
            processor.consume_main_control(&mut consume)
        }
        .map_err(command_error)?;
        if let Some(completion) = completion {
            return completion;
        }
        if observing && status == tex_command::DeliveryStatus::Command {
            return Ok(MainControlStep::Continue);
        }
        match status {
            tex_command::DeliveryStatus::Command => {}
            status => return finish_input_boundary(processor, status),
        }
    }
    Ok(MainControlStep::Continue)
}

/// Scanner reports publish before the selected arm mutates semantic state.
/// Successful ordinary scans never enter this demanded diagnostic boundary.
fn report_scan<G>(processor: &mut Processor<'_, '_, G>) -> Result<(), ExecError> {
    if processor.has_pending_reports() {
        let diagnostics = processor
            .take_semantic_diagnostics()
            .into_iter()
            .map(PendingDiagnostic::Command)
            .collect();
        processor.with_execution_owners(
            |context, command, effects, _, _, execution, _, _, _| {
                host_boundaries::report_scan_on_owners(
                    context,
                    command,
                    execution,
                    effects,
                    diagnostics,
                )
            },
        )?;
        recovery::finish_error_dialogue(processor)?;
    }
    Ok(())
}

fn finish_input_boundary<G>(
    _processor: &mut Processor<'_, '_, G>,
    status: tex_command::DeliveryStatus,
) -> Result<MainControlStep, ExecError> {
    match status {
        tex_command::DeliveryStatus::End => Ok(MainControlStep::EndOfInput),
        tex_command::DeliveryStatus::ReplayCompleted(_) => Ok(MainControlStep::Continue),
        _ => unreachable!("canonical terminal reader handles alignment and scalar boundaries"),
    }
}
