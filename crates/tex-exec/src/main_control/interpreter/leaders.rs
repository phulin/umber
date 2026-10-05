//! Consume the actual following glue before contributing leaders.
use super::*;

pub(super) fn finish_leader<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let (payload, horizontal) =
        processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
            (
                execution.boxes.pending_leader.take(),
                matches!(
                    execution.modes.current_mode(),
                    Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath
                ),
            )
        });
    let mut result = None;
    let status = processor
        .consume_main_control(&mut tex_command::terminal_consumer::<
            Processor<'_, '_, G>,
            G,
            _,
        >(
            |processor, spelling, _meaning, code, _, exceptional| {
                if matches!(code, codes::CHAR_SPACE | codes::RELAX) {
                    return Ok(ControlFlow::Continue(()));
                }
                result = Some((|| {
                    let glue = if (horizontal && code == codes::PRIMITIVE_H_SKIP)
                        || (!horizontal && code == codes::PRIMITIVE_V_SKIP)
                    {
                        let spec = processor.scan_glue(false).map_err(command_error)?;
                        Some((spec, processor.scanned_glue_shared_zero()))
                    } else {
                        let infinite = match (horizontal, code) {
                            (true, codes::PRIMITIVE_H_FIL) | (false, codes::PRIMITIVE_V_FIL) => {
                                Some((Order::Fil, false, false))
                            }
                            (true, codes::PRIMITIVE_H_FILL) | (false, codes::PRIMITIVE_V_FILL) => {
                                Some((Order::Fill, false, false))
                            }
                            (true, codes::PRIMITIVE_H_SS) | (false, codes::PRIMITIVE_V_SS) => {
                                Some((Order::Fil, false, true))
                            }
                            (true, codes::PRIMITIVE_H_FIL_NEG)
                            | (false, codes::PRIMITIVE_V_FIL_NEG) => {
                                Some((Order::Fil, true, false))
                            }
                            _ => None,
                        };
                        infinite.map(|(order, negative, shrink)| {
                            let unit = Scaled::from_raw(if negative {
                                -Scaled::UNITY
                            } else {
                                Scaled::UNITY
                            });
                            let zero = Scaled::from_raw(0);
                            (
                                GlueSpec {
                                    width: zero,
                                    stretch: if shrink { zero } else { unit },
                                    stretch_order: if shrink { Order::Normal } else { order },
                                    shrink: if shrink { unit } else { zero },
                                    shrink_order: if shrink { order } else { Order::Normal },
                                },
                                false,
                            )
                        })
                    };
                    let Some((glue, shared_zero)) = glue else {
                        processor
                            .back_consumed_word(spelling, exceptional)
                            .map_err(command_error)?;
                        return bad_leader_glue(processor);
                    };
                    report_scan(processor)?;
                    processor.with_execution_owners(
                        |context, command, effects, fuel, _, execution, _, _, _| {
                            execution.main_loop_active = false;
                            if let Some((kind, payload)) = payload {
                                crate::box_runtime::append_leader_contribution(
                                    &mut execution.modes,
                                    context,
                                    effects,
                                    kind,
                                    payload,
                                    glue,
                                    if shared_zero {
                                        tex_state::node::GlueSpecOrigin::SharedZero
                                    } else {
                                        tex_state::node::GlueSpecOrigin::Owned
                                    },
                                    fuel,
                                )?;
                                crate::vertical::build_page_if_outer_vertical(
                                    &execution.modes,
                                    context,
                                    effects,
                                    command,
                                )?;
                            }
                            Ok(MainControlStep::Continue)
                        },
                    )
                })());
                Ok(ControlFlow::Break(()))
            },
        ))
        .map_err(command_error)?;
    match result {
        Some(result) => result,
        None if status == tex_command::DeliveryStatus::End => bad_leader_glue(processor),
        None => finish_input_boundary(processor, status),
    }
}

fn bad_leader_glue<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::error_report::report_error(
            context,
            effects,
            "Leaders not followed by proper glue",
            &[
                "You should say `\\leaders <box or rule><hskip or vskip>'.",
                "I found the <box or rule>, but there's no suitable",
                "<hskip or vskip>, so I'm ignoring these leaders.",
            ],
            command.output_open_context(context),
        )
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}
