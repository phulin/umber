//! Assignments execute at the canonical reader's terminal boundary.
use super::*;

pub(super) fn accepts_assignment(code: u64) -> bool {
    matches!(
        code,
        codes::PRIMITIVE_TEXT_FONT
            | codes::PRIMITIVE_SCRIPT_FONT
            | codes::PRIMITIVE_SCRIPT_SCRIPT_FONT
            | codes::PRIMITIVE_COUNT
            | codes::COUNT_REGISTER
            | codes::PRIMITIVE_DIMEN
            | codes::DIMEN_REGISTER
            | codes::PRIMITIVE_SKIP
            | codes::SKIP_REGISTER
            | codes::PRIMITIVE_MUSKIP
            | codes::MUSKIP_REGISTER
            | codes::INT_PARAM
            | codes::DIMEN_PARAM
            | codes::GLUE_PARAM
            | codes::MU_GLUE_PARAM
            | codes::PRIMITIVE_DEF
            | codes::PRIMITIVE_EDEF
            | codes::PRIMITIVE_GDEF
            | codes::PRIMITIVE_XDEF
            | codes::PRIMITIVE_LET
            | codes::PRIMITIVE_FUTURE_LET
            | codes::PRIMITIVE_TOKS
            | codes::TOKS_REGISTER
            | codes::TOK_PARAM
            | codes::PRIMITIVE_ADVANCE
            | codes::PRIMITIVE_MULTIPLY
            | codes::PRIMITIVE_DIVIDE
    )
}

#[allow(clippy::too_many_arguments)] // actual processor, packed selector, scope and demanded owners
pub(super) fn execute_assignment<'state, 'host, G>(
    processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
    word: u64,
    code: u64,
    global: bool,
    flags: MeaningFlags,
    control_sequence: Option<tex_state::interner::Symbol>,
    scan_origin: Option<OriginId>,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
    pending_diagnostics: Option<&mut Vec<PendingDiagnostic<G>>>,
) -> Result<ReplayStep, ExecError> {
    let global = effective_global(
        processor.int_param(IntParam::GLOBAL_DEFS),
        global || matches!(code, codes::PRIMITIVE_GDEF | codes::PRIMITIVE_XDEF),
    );
    match code {
        codes::PRIMITIVE_TEXT_FONT
        | codes::PRIMITIVE_SCRIPT_FONT
        | codes::PRIMITIVE_SCRIPT_SCRIPT_FONT => {
            let size = match code {
                codes::PRIMITIVE_TEXT_FONT => MathFontSize::Text,
                codes::PRIMITIVE_SCRIPT_FONT => MathFontSize::Script,
                _ => MathFontSize::ScriptScript,
            };
            let family = take_native_scalar!(
                processor.scan_restricted_integer(tex_command::RestrictedIntegerClass::FourBit),
                scan_origin
            )
            .value as u8;
            let _ = take_native_scalar!(processor.scan_optional_equals(), scan_origin);
            let font = take_native_scalar!(processor.scan_font_selector(), scan_origin);
            with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                _receipts,
                |context, _state, effects, _fuel, _observer, _execution| {
                    AssignmentCommitter::new(context, effects).try_unscoped(|context| {
                        assign_math_family_font(context, size, family, font, global)
                    })?;
                    Ok(ReplayStep::Continue)
                }
            )
        }
        codes::PRIMITIVE_COUNT | codes::COUNT_REGISTER => {
            let index = if code == codes::COUNT_REGISTER {
                Meaning::runtime_word_operand(word) as u16
            } else {
                take_native_scalar!(processor.scan_profile_register_index(), scan_origin)
            };
            let _ = take_native_scalar!(processor.scan_optional_equals(), scan_origin);
            let value = take_native_scalar!(processor.scan_integer(), scan_origin).value;
            let step = with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |native_context,
                 _native_state,
                 native_effects,
                 _native_fuel,
                 _native_observer,
                 native_execution| {
                    semantic::execute_count(
                        native_context,
                        native_effects,
                        receipts,
                        index,
                        value,
                        global,
                    )
                }
            )?;

            Ok(step)
        }
        codes::PRIMITIVE_DIMEN | codes::DIMEN_REGISTER => {
            let index = if code == codes::DIMEN_REGISTER {
                Meaning::runtime_word_operand(word) as u16
            } else {
                take_native_scalar!(processor.scan_profile_register_index(), scan_origin)
            };
            let _ = take_native_scalar!(processor.scan_optional_equals(), scan_origin);
            let value = take_native_scalar!(processor.scan_dimension(), scan_origin);
            let step = with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |native_context,
                 _native_state,
                 native_effects,
                 _native_fuel,
                 _native_observer,
                 native_execution| {
                    semantic::execute_dimen(
                        native_context,
                        native_effects,
                        receipts,
                        index,
                        value,
                        global,
                    )
                }
            )?;

            Ok(step)
        }
        codes::PRIMITIVE_SKIP
        | codes::SKIP_REGISTER
        | codes::PRIMITIVE_MUSKIP
        | codes::MUSKIP_REGISTER => {
            let mu = matches!(code, codes::PRIMITIVE_MUSKIP | codes::MUSKIP_REGISTER);
            let index = if matches!(code, codes::SKIP_REGISTER | codes::MUSKIP_REGISTER) {
                Meaning::runtime_word_operand(word) as u16
            } else {
                take_native_scalar!(processor.scan_profile_register_index(), scan_origin)
            };
            let _ = take_native_scalar!(processor.scan_optional_equals(), scan_origin);
            let value = take_native_scalar!(processor.scan_glue(mu), scan_origin);
            let source_identity = processor.scanned_glue_identity();
            let source_register = processor.scanned_glue_register();
            let step = with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |native_context,
                 _native_state,
                 native_effects,
                 _native_fuel,
                 _native_observer,
                 native_execution| {
                    let reassigning = MainControl::direct_glue_pointer_reassigned(
                        native_context,
                        mu,
                        index,
                        value,
                        source_identity,
                        source_register,
                        global,
                        &native_execution.skip_pointer_sources,
                        &native_execution.muskip_pointer_sources,
                    );
                    let redundant =
                        native_context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 && reassigning;
                    let result = semantic::execute_skip(
                        native_context,
                        native_effects,
                        receipts,
                        index,
                        value,
                        global,
                        mu,
                        redundant,
                        reassigning,
                    );
                    if result.is_ok() && !redundant {
                        let physical = if mu {
                            native_context.muskip(index)
                        } else {
                            native_context.glue_register(index).ok().flatten()
                        };
                        if let Some(physical) = physical {
                            let pointers = if mu {
                                &mut native_execution.muskip_pointer_sources
                            } else {
                                &mut native_execution.skip_pointer_sources
                            };
                            if pointers.len() <= usize::from(index) {
                                pointers.resize(usize::from(index) + 1, None);
                            }
                            pointers[usize::from(index)] = Some((physical, source_identity));
                        }
                    }
                    result
                }
            )?;

            Ok(step)
        }
        codes::INT_PARAM => {
            let index = Meaning::runtime_word_operand(word) as u16;
            let _ = take_native_scalar!(processor.scan_optional_equals(), scan_origin);
            let value = take_native_scalar!(processor.scan_integer(), scan_origin).value;
            let step = with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |native_context,
                 native_state,
                 native_effects,
                 _native_fuel,
                 _native_observer,
                 native_execution| {
                    semantic::execute_int_param(
                        native_context,
                        native_effects,
                        native_state,
                        receipts,
                        index,
                        value,
                        global,
                    )
                }
            )?;

            Ok(step)
        }
        codes::DIMEN_PARAM => {
            let index = Meaning::runtime_word_operand(word) as u16;
            let _ = take_native_scalar!(processor.scan_optional_equals(), scan_origin);
            let value = take_native_scalar!(processor.scan_dimension(), scan_origin);
            let step = with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |native_context,
                 native_state,
                 native_effects,
                 _native_fuel,
                 _native_observer,
                 native_execution| {
                    semantic::execute_dimen_param(
                        native_context,
                        native_effects,
                        native_state,
                        receipts,
                        index,
                        value,
                        global,
                    )
                }
            )?;

            Ok(step)
        }
        codes::GLUE_PARAM | codes::MU_GLUE_PARAM => {
            let index = Meaning::runtime_word_operand(word) as u16;
            let mu = code == codes::MU_GLUE_PARAM;
            let _ = take_native_scalar!(processor.scan_optional_equals(), scan_origin);
            let value = take_native_scalar!(processor.scan_glue(mu), scan_origin);
            let step = with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |native_context,
                 native_state,
                 native_effects,
                 _native_fuel,
                 _native_observer,
                 native_execution| {
                    semantic::execute_glue_param(
                        native_context,
                        native_effects,
                        native_state,
                        receipts,
                        index,
                        value,
                        global,
                    )
                }
            )?;

            Ok(step)
        }
        codes::PRIMITIVE_DEF
        | codes::PRIMITIVE_EDEF
        | codes::PRIMITIVE_GDEF
        | codes::PRIMITIVE_XDEF => {
            let definition = processor
                .scan_macro_definition(matches!(
                    code,
                    codes::PRIMITIVE_EDEF | codes::PRIMITIVE_XDEF
                ))
                .map_err(command_error)
                .map_err(|error| scan_error(error, scan_origin))?;
            let step = with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |context, _state, effects, fuel, observer, native_execution| {
                    if native_execution
                        .modes
                        .current_list()
                        .pending_hchars()
                        .is_some()
                    {
                        crate::box_runtime::flush_pending_hchars_with_fuel(
                            &mut native_execution.modes,
                            context,
                            effects,
                            fuel,
                        )?;
                    }
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
                }
            )?;

            Ok(step)
        }
        codes::PRIMITIVE_LET | codes::PRIMITIVE_FUTURE_LET => {
            let (target, meaning) = (if code == codes::PRIMITIVE_FUTURE_LET {
                processor.scan_futurelet_assignment()
            } else {
                processor.scan_let_assignment()
            })
            .map_err(command_error)
            .map_err(|error| scan_error(error, scan_origin))?;
            let step = with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |context, _state, effects, fuel, _observer, native_execution| {
                    if native_execution
                        .modes
                        .current_list()
                        .pending_hchars()
                        .is_some()
                    {
                        crate::box_runtime::flush_pending_hchars_with_fuel(
                            &mut native_execution.modes,
                            context,
                            effects,
                            fuel,
                        )?;
                    }
                    definition_assignments::apply_let(
                        target, meaning, global, context, effects, receipts,
                    )
                }
            )?;

            Ok(step)
        }

        codes::PRIMITIVE_TOKS | codes::TOKS_REGISTER | codes::TOK_PARAM => {
            let owner = control_sequence.ok_or_else(|| {
                scan_error(
                    ExecError::MissingToken {
                        context: "token-list assignment owner",
                    },
                    scan_origin,
                )
            })?;
            let index = if code == codes::PRIMITIVE_TOKS {
                take_native_scalar!(processor.scan_profile_register_index(), scan_origin)
            } else {
                Meaning::runtime_word_operand(word) as u16
            };
            let parameter = code == codes::TOK_PARAM;
            let (tokens, source) = processor
                .scan_token_list_value(owner, parameter && index == TokParam::OUTPUT.raw())
                .map_err(command_error)
                .map_err(|error| scan_error(error, scan_origin))?;
            with_native_assignment!(processor, observation_receipts, pending_diagnostics;
                receipts,
                |context, state, effects, _fuel, _observer, execution| {
                    let tokens = match source {
                        Some(source) => Some(source),
                        None => publication::publish_tokens(state, context, tokens)?,
                    };
                    semantic::execute_toks(
                        context, effects, state, receipts, index, tokens, global, parameter,
                    )
                }
            )
        }
        codes::PRIMITIVE_ADVANCE | codes::PRIMITIVE_MULTIPLY | codes::PRIMITIVE_DIVIDE => {
            let primitive = match code {
                codes::PRIMITIVE_ADVANCE => UnexpandablePrimitive::Advance,
                codes::PRIMITIVE_MULTIPLY => UnexpandablePrimitive::Multiply,
                _ => UnexpandablePrimitive::Divide,
            };
            scan_arithmetic::execute_arithmetic(
                processor,
                primitive,
                global,
                scan_origin,
                observation_receipts,
                pending_diagnostics,
            )
        }
        _ => unreachable!("native assignment selection"),
    }
}
