//! The sole interpreter switch; no eligibility dispatcher precedes it.
use super::*;
use crate::main_control::direct::command_codes as codes;
use tex_state::meaning::MeaningWord;

pub(super) fn execute<'state, 'host, G>(
    processor: &mut Processor<'state, 'host, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<ReplayStep, ExecError> {
    let code = match meaning {
        MeaningWord::Static(word) => Meaning::runtime_word_control_code(*word),
        MeaningWord::Font(_) => codes::FONT,
        MeaningWord::Macro { .. } => unreachable!("expanded reader delivers a terminal"),
    };
    match code {
        codes::RELAX => {
            processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                let pending = execution.modes.current_list().pending_hchars().is_some();
                semantic::execute_relax(
                    context,
                    pending.then_some(&mut execution.modes),
                    effects,
                    fuel,
                )
            })
        }
        codes::CHAR_LETTER | codes::CHAR_OTHER | codes::CHAR_GIVEN => {
            let MeaningWord::Static(word) = meaning else {
                unreachable!("scalar character meaning")
            };
            let ch = if code == codes::CHAR_GIVEN {
                char::from_u32(Meaning::runtime_word_operand(*word) as u32)
                    .expect("admitted character definition")
            } else {
                Meaning::runtime_word_character(*word)
                    .expect("admitted literal character")
                    .0
            };
            let origin = processor.active_macro_origin().unwrap_or(spelling.origin());
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Math | Mode::DisplayMath => {
                    direct::characters::append_native_math_character(processor, ch, origin)?;
                }
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    let etex_extended = processor.profile() == CommandProfile::ETEX26;
                    processor.with_execution_owners(
                        |context, _, effects, fuel, _, execution, _, _, _| {
                            direct::characters::append_native_character(
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
                    return start_paragraph_and_reswitch(
                        processor,
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                }
            }
            Ok(ReplayStep::Continue)
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
                Ok(ReplayStep::Continue)
            })
        }
        codes::FONT => {
            let MeaningWord::Font(font) = meaning else {
                unreachable!("font capability")
            };
            let global = effective_global(processor.int_param(IntParam::GLOBAL_DEFS), false);
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, _, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_font_select(context, effects, *font, global)
                },
            )?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::CHAR_BEGIN_GROUP | codes::PRIMITIVE_BEGIN_GROUP => {
            let kind = if code == codes::CHAR_BEGIN_GROUP {
                GroupKind::Simple
            } else {
                GroupKind::SemiSimple
            };
            report_scan(processor)?;
            processor.with_execution_owners(
                |context, command, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    direct::apply_groups::begin_native_group(
                        context,
                        command,
                        effects,
                        fuel,
                        &mut execution.modes,
                        kind,
                    )
                },
            )
        }
        codes::CHAR_END_GROUP | codes::PRIMITIVE_END_GROUP => close_tex_group(
            processor,
            code,
            spelling,
            meaning,
            control_sequence,
            exceptional,
        ),
        codes::PRIMITIVE_COUNT | codes::COUNT_REGISTER => {
            let global = effective_global(processor.int_param(IntParam::GLOBAL_DEFS), false);
            let index = if code == codes::COUNT_REGISTER {
                let MeaningWord::Static(word) = meaning else {
                    unreachable!("count register operand")
                };
                Meaning::runtime_word_operand(*word) as u16
            } else {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            };
            processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, _, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_count(
                        context,
                        effects,
                        observation_receipts,
                        index,
                        value,
                        global,
                    )
                },
            )?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::PRIMITIVE_DIMEN | codes::DIMEN_REGISTER => {
            let global = effective_global(processor.int_param(IntParam::GLOBAL_DEFS), false);
            let index = if code == codes::DIMEN_REGISTER {
                let MeaningWord::Static(word) = meaning else {
                    unreachable!("dimension register operand")
                };
                Meaning::runtime_word_operand(*word) as u16
            } else {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            };
            processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?;
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, _, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_dimen(
                        context,
                        effects,
                        observation_receipts,
                        index,
                        value,
                        global,
                    )
                },
            )?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        _ => execute_selected_command(
            processor,
            code,
            spelling,
            meaning,
            control_sequence,
            exceptional,
        ),
    }
}
