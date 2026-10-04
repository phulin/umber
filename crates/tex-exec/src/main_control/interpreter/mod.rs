//! Persistent interpretation over the actual scanner and execution owners.
//!
//! Fetch roles change at TeX's own labels. They do not end the processor loan.
//! Every terminal enters the same dispatch; only exceptional delivery carries
//! the reader's diagnostic/observation record.

use super::*;
use std::ops::ControlFlow;

mod assignments;
mod characters;
mod codes;
mod discretionary;
mod dispatch;
mod driver;
mod glue_identity;
mod groups;
mod material;
mod math;
mod prefixes;
mod publication;
mod recovery;

type Processor<'state, 'host, G> =
    CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>;

pub(super) fn run<G>(
    processor: &mut Processor<'_, '_, G>,
    remaining_operations: usize,
    completed_operations: &mut usize,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    while *completed_operations < remaining_operations {
        let lookahead = processor
            .with_execution_owners(|_, _, _, _, _, execution, _, _, _| execution.main_loop_active);
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
                    Ok(ReplayStep::Continue) => {
                        *completed_operations += 1;
                        let next_lookahead =
                            processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                                execution.main_loop_active
                            });
                        if !observing
                            && next_lookahead == lookahead
                            && *completed_operations < remaining_operations
                        {
                            return Ok(ControlFlow::Continue(()));
                        }
                    }
                    result => completion = Some(result),
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
            return Ok(ReplayStep::Continue);
        }
        match status {
            tex_command::DeliveryStatus::Command => {}
            status => return finish_input_boundary(processor, status),
        }
    }
    Ok(ReplayStep::Continue)
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
) -> Result<ReplayStep, ExecError> {
    match status {
        tex_command::DeliveryStatus::End => Ok(ReplayStep::EndOfInput),
        tex_command::DeliveryStatus::ReplayCompleted(_) => Ok(ReplayStep::Continue),
        _ => unreachable!("canonical terminal reader handles alignment and scalar boundaries"),
    }
}
