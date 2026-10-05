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
            let mut consume = {
                #[inline(always)]
                |processor: &mut Processor<'_, '_, G>,
                 spelling: TracedTokenWord,
                 meaning: &tex_state::meaning::MeaningWord<G>,
                 control_sequence: Option<Symbol>,
                 exceptional: Option<&tex_command::CurrentCommand<G>>| {
                    // Native delivery never lends an observation record.
                    // Fold that actual ownership fact into its continuation.
                    let observing = exceptional.is_some() && observing;
                    let code = codes::of(meaning);
                    if let Err(error) =
                        dispatch::enter(processor, spelling, meaning, control_sequence, code)
                    {
                        completion = Some(Err(error));
                        return Ok(ControlFlow::Break(()));
                    }
                    if code != codes::RELAX {
                        match dispatch::reswitch(
                            processor,
                            spelling,
                            meaning,
                            control_sequence,
                            exceptional,
                            observation_receipts,
                            0,
                            code,
                        ) {
                            Ok(MainControlStep::Continue) => {}
                            result => {
                                if result.is_ok() {
                                    completed += 1;
                                }
                                completion = Some(result);
                                return Ok(ControlFlow::Break(()));
                            }
                        }
                    }
                    completed += 1;
                    // Relax can flush character shaping or publish a demanded
                    // diagnostic; it cannot close a list or prepare page output.
                    let (mut next_lookahead, host_boundary) = if code == codes::RELAX {
                        (
                            false,
                            processor.has_diagnostic_boundary_since(initial_effect_pos),
                        )
                    } else {
                        processor.with_execution_owners(
                            |context, _, effects, _, _, execution, _, _, _| {
                                let diagnostic_boundary = context.effect_pos()
                                    != initial_effect_pos
                                    || effects.has_first_recoverable();
                                (
                                    execution.main_loop_active,
                                    diagnostic_boundary
                                        || execution.paragraph_checkpoint_cut
                                        || execution.prepared_shipout.is_some()
                                        || execution.prepared_pdf_form.is_some()
                                        || (execution.page_region_succession_pending
                                            && !execution.modes.retains_page_node_handles())
                                        || (!execution.boxes.output_routine_active
                                            && (context.page_fire_up().is_some()
                                                || context
                                                    .page_builder_resume_after_output_pending())),
                                )
                            },
                        )
                    };
                    if host_boundary {
                        completion = Some(Ok(MainControlStep::Continue));
                        return Ok(ControlFlow::Break(()));
                    }
                    if next_lookahead && !observing {
                        let mut characters = characters::MainSourceAdmission {
                            etex_extended: processor.profile() == CommandProfile::ETEX26,
                            error: None,
                        };
                        let appended = match processor.consume_main_source_characters(
                            remaining_operations - completed,
                            &mut characters,
                        ) {
                            Ok(appended) => appended,
                            Err(error) => {
                                completion = Some(Err(command_error(error)));
                                return Ok(ControlFlow::Break(()));
                            }
                        };
                        completed += appended as usize;
                        if appended != 0 {
                            next_lookahead = processor.host_facts().main_loop_active;
                        }
                        if let Some(error) = characters.error {
                            completion = Some(Err(error));
                            return Ok(ControlFlow::Break(()));
                        }
                    }
                    if !observing && next_lookahead == lookahead && completed < remaining_operations
                    {
                        return Ok(ControlFlow::Continue(()));
                    }
                    Ok(ControlFlow::Break(()))
                }
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
