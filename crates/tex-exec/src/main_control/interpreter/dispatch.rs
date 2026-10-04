//! The sole interpreter switch; no eligibility dispatcher precedes it.
use super::*;
use tex_state::meaning::MeaningWord;

pub(super) fn execute<'state, 'host, G>(
    processor: &mut Processor<'state, 'host, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
    prefixes: u8,
) -> Result<ReplayStep, ExecError> {
    let code = match meaning {
        MeaningWord::Static(word) => Meaning::runtime_word_control_code(*word),
        MeaningWord::Font(_) => codes::FONT,
        MeaningWord::Macro { .. } => unreachable!("expanded reader delivers a terminal"),
    };
    match code {
        codes::PRIMITIVE_GLOBAL
        | codes::PRIMITIVE_LONG
        | codes::PRIMITIVE_OUTER
        | codes::PRIMITIVE_PROTECTED => {
            let flag = match code {
                codes::PRIMITIVE_GLOBAL => prefixes::GLOBAL,
                codes::PRIMITIVE_LONG => MeaningFlags::LONG.bits(),
                codes::PRIMITIVE_OUTER => MeaningFlags::OUTER.bits(),
                _ => MeaningFlags::PROTECTED.bits(),
            };
            prefixes::scan(processor, prefixes | flag, observation_receipts)
        }
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
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
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
            if code == codes::CHAR_BEGIN_GROUP && math_mode(processor) {
                processor
                    .back_consumed_word(spelling, exceptional)
                    .map_err(command_error)?;
                return math::nucleus(processor, NoadKind::Normal(NoadClass::Ord));
            }
            let kind = if code == codes::CHAR_BEGIN_GROUP {
                GroupKind::Simple
            } else {
                GroupKind::SemiSimple
            };
            report_scan(processor)?;
            processor.with_execution_owners(
                |context, command, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    groups::begin_native_group(
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
        codes::CHAR_END_GROUP | codes::PRIMITIVE_END_GROUP => groups::close_tex_group(
            processor,
            code,
            spelling,
            meaning,
            control_sequence,
            exceptional,
        ),
        codes::PRIMITIVE_COUNT | codes::COUNT_REGISTER => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let index = if code == codes::COUNT_REGISTER {
                operand(meaning) as u16
            } else {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            };
            assignments::count(processor, index, global, observation_receipts)
        }
        codes::PRIMITIVE_DIMEN | codes::DIMEN_REGISTER => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let index = if code == codes::DIMEN_REGISTER {
                operand(meaning) as u16
            } else {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            };
            assignments::dimension(processor, index, global, observation_receipts)
        }
        codes::INT_PARAM => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            assignments::integer_parameter(
                processor,
                operand(meaning) as u16,
                global,
                observation_receipts,
            )
        }
        codes::DIMEN_PARAM => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            assignments::dimension_parameter(
                processor,
                operand(meaning) as u16,
                global,
                observation_receipts,
            )
        }
        codes::PRIMITIVE_SKIP
        | codes::SKIP_REGISTER
        | codes::PRIMITIVE_MUSKIP
        | codes::MUSKIP_REGISTER => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let mu = matches!(code, codes::PRIMITIVE_MUSKIP | codes::MUSKIP_REGISTER);
            let index = if matches!(code, codes::SKIP_REGISTER | codes::MUSKIP_REGISTER) {
                operand(meaning) as u16
            } else {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            };
            assignments::glue_register(processor, index, mu, global, observation_receipts)
        }
        codes::PRIMITIVE_DEF
        | codes::PRIMITIVE_EDEF
        | codes::PRIMITIVE_GDEF
        | codes::PRIMITIVE_XDEF => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0
                    || matches!(code, codes::PRIMITIVE_GDEF | codes::PRIMITIVE_XDEF),
            );
            let expanded = matches!(code, codes::PRIMITIVE_EDEF | codes::PRIMITIVE_XDEF);
            assignments::definition(
                processor,
                expanded,
                MeaningFlags::from_bits(prefixes & !prefixes::GLOBAL),
                global,
                observation_receipts,
            )
        }
        codes::PRIMITIVE_LET => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let (target, meaning) = processor.scan_let_assignment().map_err(command_error)?;
            assignments::bind(processor, target, meaning, global, observation_receipts)
        }
        codes::PRIMITIVE_FUTURE_LET => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let (target, meaning) = processor
                .scan_futurelet_assignment()
                .map_err(command_error)?;
            assignments::bind(processor, target, meaning, global, observation_receipts)
        }
        codes::PRIMITIVE_AFTER_GROUP => {
            let mut token = processor
                .get_token_spelling()
                .map_err(command_error)?
                .ok_or(ExecError::MissingToken {
                    context: "aftergroup operand",
                })?;
            report_scan(processor)?;
            processor.with_execution_owners(|context, command, _, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                semantic::execute_after_group(context, command, &mut token)
            })
        }
        codes::PRIMITIVE_AFTER_ASSIGNMENT => {
            let mut token = processor
                .get_token_spelling()
                .map_err(command_error)?
                .ok_or(ExecError::MissingToken {
                    context: "afterassignment operand",
                })?;
            report_scan(processor)?;
            processor.with_execution_owners(|context, command, _, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                semantic::execute_after_assignment(context, command, &mut token)
            })
        }
        codes::MATH_GIVEN => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            math::character(processor, operand(meaning) as u32, spelling.origin())
        }
        codes::PRIMITIVE_MATH_CHAR | codes::PRIMITIVE_DELIMITER => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let class = if code == codes::PRIMITIVE_MATH_CHAR {
                RestrictedIntegerClass::FifteenBit
            } else {
                RestrictedIntegerClass::TwentySevenBit
            };
            let scanned = processor
                .scan_restricted_integer(class)
                .map_err(command_error)?;
            let value = if code == codes::PRIMITIVE_DELIMITER {
                scanned.value as u32 >> 12
            } else {
                scanned.value as u32
            };
            math::character(processor, value, scanned.provenance.primary)
        }
        codes::PRIMITIVE_DISPLAY_STYLE
        | codes::PRIMITIVE_TEXT_STYLE
        | codes::PRIMITIVE_SCRIPT_STYLE
        | codes::PRIMITIVE_SCRIPT_SCRIPT_STYLE => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let style = match code {
                codes::PRIMITIVE_DISPLAY_STYLE => MathStyle::Display,
                codes::PRIMITIVE_TEXT_STYLE => MathStyle::Text,
                codes::PRIMITIVE_SCRIPT_STYLE => MathStyle::Script,
                _ => MathStyle::ScriptScript,
            };
            math::style(processor, style)
        }
        codes::PRIMITIVE_MATH_ORD
        | codes::PRIMITIVE_MATH_OP
        | codes::PRIMITIVE_MATH_BIN
        | codes::PRIMITIVE_MATH_REL
        | codes::PRIMITIVE_MATH_OPEN
        | codes::PRIMITIVE_MATH_CLOSE
        | codes::PRIMITIVE_MATH_PUNCT
        | codes::PRIMITIVE_MATH_INNER => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = match code {
                codes::PRIMITIVE_MATH_ORD => NoadKind::Normal(NoadClass::Ord),
                codes::PRIMITIVE_MATH_OP => NoadKind::Normal(NoadClass::Op),
                codes::PRIMITIVE_MATH_BIN => NoadKind::Normal(NoadClass::Bin),
                codes::PRIMITIVE_MATH_REL => NoadKind::Normal(NoadClass::Rel),
                codes::PRIMITIVE_MATH_OPEN => NoadKind::Normal(NoadClass::Open),
                codes::PRIMITIVE_MATH_CLOSE => NoadKind::Normal(NoadClass::Close),
                codes::PRIMITIVE_MATH_PUNCT => NoadKind::Normal(NoadClass::Punct),
                _ => NoadKind::Normal(NoadClass::Inner),
            };
            math::nucleus(processor, kind)
        }
        codes::CHAR_SUPERSCRIPT | codes::CHAR_SUBSCRIPT => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = if code == codes::CHAR_SUPERSCRIPT {
                MathScriptKind::Superscript
            } else {
                MathScriptKind::Subscript
            };
            math::script(processor, kind)
        }
        codes::PRIMITIVE_LIMITS | codes::PRIMITIVE_NO_LIMITS | codes::PRIMITIVE_DISPLAY_LIMITS => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = match code {
                codes::PRIMITIVE_LIMITS => LimitType::Limits,
                codes::PRIMITIVE_NO_LIMITS => LimitType::NoLimits,
                _ => LimitType::DisplayLimits,
            };
            math::limits(processor, kind)
        }
        codes::PRIMITIVE_OVER
        | codes::PRIMITIVE_ATOP
        | codes::PRIMITIVE_ABOVE
        | codes::PRIMITIVE_OVER_WITH_DELIMS
        | codes::PRIMITIVE_ATOP_WITH_DELIMS
        | codes::PRIMITIVE_ABOVE_WITH_DELIMS => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = match code {
                codes::PRIMITIVE_OVER | codes::PRIMITIVE_OVER_WITH_DELIMS => MathFractionKind::Over,
                codes::PRIMITIVE_ATOP | codes::PRIMITIVE_ATOP_WITH_DELIMS => MathFractionKind::Atop,
                _ => MathFractionKind::Above,
            };
            let delimiters = matches!(
                code,
                codes::PRIMITIVE_OVER_WITH_DELIMS
                    | codes::PRIMITIVE_ATOP_WITH_DELIMS
                    | codes::PRIMITIVE_ABOVE_WITH_DELIMS
            );
            math::fraction(processor, kind, delimiters)
        }
        codes::PRIMITIVE_UNDERLINE | codes::PRIMITIVE_OVERLINE => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = if code == codes::PRIMITIVE_UNDERLINE {
                NoadKind::Underline
            } else {
                NoadKind::Overline
            };
            math::nucleus(processor, kind)
        }
        codes::PRIMITIVE_RADICAL => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let delimiter = processor.scan_delimiter(true).map_err(command_error)?;
            math::nucleus(
                processor,
                NoadKind::Radical {
                    delimiter: delimiter.code,
                },
            )
        }
        codes::PRIMITIVE_MATH_ACCENT => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let scanned = processor.scan_math_character().map_err(command_error)?;
            let accent = processor
                .with_execution_owners(|context, _, _, _, _, _, _, _, _| {
                    math_char(context, u32::from(scanned.code), scanned.provenance.primary)
                })?
                .1;
            math::nucleus(processor, NoadKind::Accent { accent })
        }
        codes::PRIMITIVE_DISCRETIONARY | codes::PRIMITIVE_DISCRETIONARY_HYPHEN => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            if code == codes::PRIMITIVE_DISCRETIONARY {
                discretionary::begin(processor)
            } else {
                discretionary::hyphen(
                    processor,
                    processor.active_macro_origin().unwrap_or(spelling.origin()),
                )
            }
        }
        codes::PRIMITIVE_MATH_CHOICE => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            math::choice(processor)
        }
        codes::PRIMITIVE_KERN => material::kern(processor),
        codes::PRIMITIVE_PENALTY => material::penalty(processor),
        codes::PRIMITIVE_H_SKIP => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            material::horizontal_glue(processor)
        }
        codes::PRIMITIVE_V_SKIP => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Math | Mode::DisplayMath => recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                ),
                Mode::Horizontal => {
                    processor
                        .recover_stop_for_vertical_mode(spelling, exceptional)
                        .map_err(command_error)?;
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution.main_loop_active = false
                    });
                    Ok(ReplayStep::Continue)
                }
                Mode::Vertical | Mode::InternalVertical => material::vertical_glue(processor),
                Mode::RestrictedHorizontal => {
                    recovery::off_save(processor, spelling, meaning, control_sequence, exceptional)
                }
            }
        }
        codes::PRIMITIVE_M_SKIP | codes::PRIMITIVE_M_KERN => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            if code == codes::PRIMITIVE_M_SKIP {
                material::mu_glue(processor)
            } else {
                material::mu_kern(processor)
            }
        }
        codes::PRIMITIVE_ACCENT if math_mode(processor) => math::text_accent(processor),
        codes::PRIMITIVE_EQ_NO | codes::PRIMITIVE_LEFT_EQ_NO => {
            let side = if code == codes::PRIMITIVE_LEFT_EQ_NO {
                crate::mode::EqNoSide::Left
            } else {
                crate::mode::EqNoSide::Right
            };
            math::equation_number(processor, side, spelling)
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

fn operand<G>(meaning: &MeaningWord<G>) -> u64 {
    let MeaningWord::Static(word) = meaning else {
        unreachable!("selected static operand")
    };
    Meaning::runtime_word_operand(*word)
}

fn math_mode<G>(processor: &mut Processor<'_, '_, G>) -> bool {
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        matches!(
            execution.modes.current_mode(),
            Mode::Math | Mode::DisplayMath
        )
    })
}
