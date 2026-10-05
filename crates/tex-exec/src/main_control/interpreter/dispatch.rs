//! The sole interpreter switch; no eligibility dispatcher precedes it.
use super::*;
use tex_state::meaning::MeaningWord;

mod cold;

#[inline(always)]
pub(super) fn execute<'state, 'host, G>(
    processor: &mut Processor<'state, 'host, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
    code: u64,
) -> Result<MainControlStep, ExecError> {
    enter(processor, spelling, meaning, control_sequence, code)?;
    if code == codes::RELAX {
        return Ok(MainControlStep::Continue);
    }
    reswitch(
        processor,
        spelling,
        meaning,
        control_sequence,
        exceptional,
        observation_receipts,
        0,
        code,
    )
}

/// Work demanded before the actual terminal opcode executes.
#[inline(always)]
pub(super) fn enter<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    code: u64,
) -> Result<bool, ExecError> {
    if processor.has_pending_reports() {
        report_scan(processor)?;
    }
    if !matches!(
        code,
        codes::CHAR_LETTER
            | codes::CHAR_OTHER
            | codes::CHAR_GIVEN
            | codes::PRIMITIVE_CHAR
            | codes::PRIMITIVE_NO_BOUNDARY
    ) {
        let execution = processor.host_facts();
        if execution.main_loop_active || execution.modes.current_list().pending_hchars().is_some() {
            processor
                .with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    crate::box_runtime::flush_pending_hchars_with_fuel(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                    )
                })
                .map_err(|error| capture_error(processor, spelling, error))?;
        }
    }
    Ok(trace::command(processor, code, meaning, control_sequence))
}

#[inline(always)]
#[allow(clippy::too_many_arguments)] // Actual word operands, prefix bits and decoded selector.
pub(super) fn reswitch<'state, 'host, G>(
    processor: &mut Processor<'state, 'host, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
    prefixes: u8,
    code: u64,
) -> Result<MainControlStep, ExecError> {
    match code {
        codes::CHAR_LETTER | codes::CHAR_OTHER | codes::CHAR_GIVEN => {
            let Some(word) = meaning.scalar_word() else {
                unreachable!("scalar character meaning")
            };
            let ch = if code == codes::CHAR_GIVEN {
                char::from_u32(Meaning::runtime_word_operand(word) as u32)
                    .expect("admitted character definition")
            } else {
                Meaning::runtime_word_character(word)
                    .expect("admitted literal character")
                    .0
            };
            let origin = processor.active_macro_origin().unwrap_or(spelling.origin());
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Math | Mode::DisplayMath => {
                    characters::append_native_math_character(processor, ch, origin)?;
                }
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    let etex_extended = processor.profile() == CommandProfile::ETEX26;
                    processor.with_execution_owners(
                        |context, _, effects, fuel, _, execution, _, _, _| {
                            characters::append_native_character(
                                context,
                                effects,
                                fuel,
                                execution,
                                ch,
                                origin,
                                etex_extended,
                            )
                        },
                    )?;
                }
                Mode::Vertical | Mode::InternalVertical => {
                    return recovery::start_paragraph_and_reswitch(
                        processor,
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                }
            }
            Ok(MainControlStep::Continue)
        }
        codes::CHAR_SPACE => {
            processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                if matches!(
                    execution.modes.current_mode(),
                    Mode::Horizontal | Mode::RestrictedHorizontal
                ) {
                    crate::box_runtime::append_space_with_fuel(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                    )?;
                }
                Ok(MainControlStep::Continue)
            })
        }
        _ => cold::execute(
            processor,
            spelling,
            meaning,
            control_sequence,
            exceptional,
            observation_receipts,
            prefixes,
            code,
        ),
    }
    .map_err(|error| capture_error(processor, spelling, error))
}

#[cold]
#[inline(never)]
fn capture_error<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    error: ExecError,
) -> ExecError {
    let error = error.capture_command_origin(spelling.origin());
    processor.with_execution_owners(|context, command, _, _, _, _, _, _, _| {
        error.freeze_diagnostic_origin(context, command.diagnostic_input_context(8))
    })
}

fn operand<G>(meaning: &MeaningWord<G>) -> u64 {
    let Some(word) = meaning.scalar_word() else {
        unreachable!("selected static operand")
    };
    Meaning::runtime_word_operand(word)
}

fn math_mode<G>(processor: &mut Processor<'_, '_, G>) -> bool {
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        matches!(
            execution.modes.current_mode(),
            Mode::Math | Mode::DisplayMath
        )
    })
}
