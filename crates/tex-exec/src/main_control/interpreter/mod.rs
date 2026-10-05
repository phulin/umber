//! Persistent interpretation over the actual scanner and execution owners.
//!
//! Fetch roles change at TeX's own labels. They do not end the processor loan.
//! Every terminal enters the same dispatch; only exceptional delivery carries
//! the reader's diagnostic/observation record.

use super::*;
use std::ops::ControlFlow;

mod accent;
mod alignment;
mod alignment_columns;
mod alignment_recovery;
mod arithmetic;
mod assignments;
mod box_inputs;
mod boxes;
mod characters;
mod codes;
mod delimiters;
mod diagnostics;
mod discretionary;
mod dispatch;
mod display_math;
mod driver;
mod fonts;
mod glue_identity;
mod groups;
mod leaders;
mod material;
mod math;
mod math_shift;
mod pdf;
mod prefixes;
mod publication;
mod recovery;
mod resources;
mod streams;
mod terminal;
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
    let mut completed = *completed_operations;
    let result = (|| {
        let initial_effect_pos = processor.effect_pos();
        while completed < remaining_operations {
            let lookahead = processor.host_facts().main_loop_active;

            let mut completion = None;
            let observing = processor.is_observed();
            let mut consume = terminal::InterpreterTerminals {
                remaining_operations,
                completed,
                observation_receipts,
                completion: &mut completion,
                observing,
                lookahead,
                initial_effect_pos,
                resident_clean: false,
            };
            let status = if lookahead {
                processor.consume_main_loop_lookahead(&mut consume)
            } else {
                processor.consume_main_control(&mut consume)
            };
            completed = consume.completed;
            let status = status.map_err(command_error)?;
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
    })();
    *completed_operations = completed;
    result
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
                )?;
                context.publish_diagnostic_effects_before_synchronous_print(effects);
                Ok::<(), ExecError>(())
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
        tex_command::DeliveryStatus::ReplayCompleted(_)
        | tex_command::DeliveryStatus::AlignmentEndTemplate => Ok(MainControlStep::Continue),
        _ => unreachable!("canonical terminal reader handles alignment and scalar boundaries"),
    }
}
