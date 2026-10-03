//! Scalar assignments execute at the canonical reader's terminal boundary.
use super::*;

pub(super) fn accepts_scalar_assignment(code: u64) -> bool {
    matches!(
        code,
        codes::PRIMITIVE_COUNT
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
    )
}

pub(super) fn execute_scalar_assignment<'state, 'host, G>(
    processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
    word: u64,
    code: u64,
    global: bool,
    scan_origin: Option<OriginId>,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
    pending_diagnostics: Option<&mut Vec<PendingDiagnostic<G>>>,
) -> Result<ReplayStep, ExecError> {
    let global = effective_global(processor.int_param(IntParam::GLOBAL_DEFS), global);
    macro_rules! apply_assignment {
        ($receipts:ident, |$context:ident, $state:ident, $effects:ident, $fuel:ident, $observer:ident, $execution:ident| $apply:expr) => {{
            if processor.has_pending_reports() {
                let mut diagnostics = pending_diagnostics.map(std::mem::take).unwrap_or_default();
                diagnostics.extend(
                    processor
                        .take_semantic_diagnostics()
                        .into_iter()
                        .map(PendingDiagnostic::Command),
                );
                processor.with_execution_owners(
                    |context, command, effects, _, _, execution, _, _, _| {
                        super::super::host_boundaries::report_scan_on_owners(
                            context,
                            command,
                            execution,
                            effects,
                            diagnostics,
                        )
                    },
                )?;
            }
            let $receipts = &mut *observation_receipts;
            processor.with_execution_owners(
                |$context, $state, $effects, $fuel, $observer, $execution, _, _, _| {
                    $execution.main_loop_active = false;
                    $apply
                },
            )
        }};
    }
    match code {
        codes::PRIMITIVE_COUNT | codes::COUNT_REGISTER => {
            let index = if code == codes::COUNT_REGISTER {
                Meaning::runtime_word_operand(word) as u16
            } else {
                take_native_scalar!(
                    processor.scan_profile_register_index_retained(),
                    scan_origin
                )
            };
            let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
            let value = take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
            let step = apply_assignment!(
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
                take_native_scalar!(
                    processor.scan_profile_register_index_retained(),
                    scan_origin
                )
            };
            let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
            let value = take_native_scalar!(processor.scan_dimension_retained(), scan_origin);
            let step = apply_assignment!(
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
                take_native_scalar!(
                    processor.scan_profile_register_index_retained(),
                    scan_origin
                )
            };
            let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
            let value = take_native_scalar!(processor.scan_glue_retained(mu), scan_origin);
            let source_identity = processor.scanned_glue_identity();
            let source_register = processor.scanned_glue_register();
            let step = apply_assignment!(
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
            let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
            let value = take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
            let step = apply_assignment!(
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
            let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
            let value = take_native_scalar!(processor.scan_dimension_retained(), scan_origin);
            let step = apply_assignment!(
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
            let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
            let value = take_native_scalar!(processor.scan_glue_retained(mu), scan_origin);
            let step = apply_assignment!(
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
        _ => unreachable!("scalar assignment selection"),
    }
}
