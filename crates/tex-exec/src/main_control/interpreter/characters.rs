//! Direct borrowed character admission into the live horizontal builder.
use super::*;

pub(super) fn no_boundary<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let (mode, right) = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        (execution.modes.current_mode(), execution.main_loop_active)
    });
    if matches!(mode, Mode::Math | Mode::DisplayMath) {
        return Ok(MainControlStep::Continue);
    }
    if right {
        return processor.with_execution_owners(
            |context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                crate::box_runtime::flush_pending_hchars_without_right_boundary(
                    &mut execution.modes,
                    context,
                    effects,
                    fuel,
                )?;
                Ok(MainControlStep::Continue)
            },
        );
    }
    let mut completion = None;
    let status = processor
        .consume_main_control(&mut tex_command::terminal_consumer::<
            Processor<'_, '_, G>,
            G,
            _,
        >(
            |processor, spelling, meaning, code, symbol, exceptional| {
                if code == codes::PRIMITIVE_NO_BOUNDARY {
                    trace::command(processor, code, meaning, symbol);
                    return Ok(ControlFlow::Continue(()));
                }
                if matches!(
                    code,
                    codes::CHAR_LETTER
                        | codes::CHAR_OTHER
                        | codes::CHAR_GIVEN
                        | codes::PRIMITIVE_CHAR
                ) {
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution
                            .modes
                            .current_list_mutation()
                            .set_no_boundary(true)
                    });
                }
                completion = Some(dispatch::execute(
                    processor,
                    spelling,
                    meaning,
                    symbol,
                    exceptional,
                    receipts,
                    code,
                ));
                Ok(ControlFlow::Break(()))
            },
        ))
        .map_err(command_error)?;
    match completion {
        Some(step) => step,
        None => finish_input_boundary(processor, status),
    }
}
pub(super) fn ignore_spaces<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let mut completion = None;
    let status = processor
        .consume_main_control(&mut tex_command::terminal_consumer::<
            Processor<'_, '_, G>,
            G,
            _,
        >(
            |processor, spelling, meaning, code, symbol, exceptional| {
                if code == codes::CHAR_SPACE {
                    return Ok(ControlFlow::Continue(()));
                }
                if code == codes::PRIMITIVE_IGNORE_SPACES {
                    trace::command(processor, code, meaning, symbol);
                    return Ok(ControlFlow::Continue(()));
                }
                completion = Some(dispatch::execute(
                    processor,
                    spelling,
                    meaning,
                    symbol,
                    exceptional,
                    receipts,
                    code,
                ));
                Ok(ControlFlow::Break(()))
            },
        ))
        .map_err(command_error)?;
    match completion {
        Some(step) => step,
        None => finish_input_boundary(processor, status),
    }
}

/// The one main-loop admission owner for ordinary source text and its scalar
/// boundaries. The command processor invokes `admit` once for a borrowed
/// prefix and again only for exceptional scalar events; this concrete owner
/// keeps hmode state, diagnostics, and the compact outcome together.
pub(super) struct MainSourceAdmission {
    pub(super) etex_extended: bool,
    pub(super) error: Option<ExecError>,
}

impl<G> tex_command::MainCharacterConsumer<G, &mut ExecutionState<G>> for MainSourceAdmission {
    #[inline(always)]
    fn admit<'state, 'admission, 'fuel, 'effects, 'run>(
        &mut self,
        context: &'state mut CommandContext<'admission, G>,
        fuel: &'fuel mut tex_command::CommandFuel,
        diagnostic_effects: &'effects mut DiagnosticEffects,
        execution: &mut &mut ExecutionState<G>,
        input: tex_command::MainCharacterInput<'run>,
    ) -> tex_command::CharacterRunAdmission {
        match input {
            tex_command::MainCharacterInput::Borrowed(run) => {
                let result = crate::box_runtime::append_character_run_with_fuel(
                    &mut execution.modes,
                    context,
                    &mut *diagnostic_effects,
                    run,
                    fuel,
                );
                match result {
                    Ok(admitted) => {
                        // A lexical tail still belongs to §1038's main-loop
                        // label: the next scalar delivery may be a
                        // `char_given`/`char_num` command that must not gain a
                        // trace merely because the borrowed prefix stopped.
                        // Missing metrics, in contrast, leave the loop for
                        // the existing scalar fallback.
                        if admitted.count != 0 {
                            execution.main_loop_active = admitted.continue_run
                                || matches!(
                                    admitted.fallback,
                                    Some(tex_command::CharacterRunFallback::LexicalBoundary)
                                );
                        }
                        if admitted.count == 0 {
                            match admitted.fallback {
                                Some(tex_command::CharacterRunFallback::LexicalBoundary) => {
                                    tex_command::CharacterRunAdmission::tokenizer_fallback()
                                }
                                Some(tex_command::CharacterRunFallback::MissingMetric) => {
                                    tex_command::CharacterRunAdmission::scalar_fallback()
                                }
                                None => tex_command::CharacterRunAdmission::new(0, false),
                            }
                        } else {
                            match admitted.fallback {
                                Some(fallback) => {
                                    tex_command::CharacterRunAdmission::with_fallback(
                                        admitted.count,
                                        admitted.continue_run,
                                        fallback,
                                    )
                                }
                                None => tex_command::CharacterRunAdmission::new(
                                    admitted.count,
                                    admitted.continue_run,
                                ),
                            }
                        }
                    }
                    Err(error) => {
                        self.error = Some(error);
                        tex_command::CharacterRunAdmission::new(0, false)
                    }
                }
            }
            tex_command::MainCharacterInput::Scalar { ch, origin } => {
                if let Err(error) = append_native_character(
                    context,
                    diagnostic_effects,
                    fuel,
                    execution,
                    ch,
                    origin,
                    self.etex_extended,
                ) {
                    self.error = Some(error);
                    return tex_command::CharacterRunAdmission::new(0, false);
                }
                tex_command::CharacterRunAdmission::new(1, execution.main_loop_active)
            }
        }
    }
}

/// Append an actual scalar glyph and select the next fetch role in its owner.
pub(in crate::main_control) fn append_native_character<G>(
    context: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    execution: &mut ExecutionState<G>,
    ch: char,
    origin: OriginId,
    etex_extended: bool,
) -> Result<(), ExecError> {
    crate::box_runtime::append_character_with_fuel(
        &mut execution.modes,
        context,
        effects,
        ch,
        origin,
        etex_extended,
        fuel,
    )?;
    execution.main_loop_active = u8::try_from(u32::from(ch)).ok().is_some_and(|code| {
        context
            .font_char_metrics(context.current_font(), code)
            .is_some()
    });
    Ok(())
}

/// Append an actual math noad, or retain §1155's active-character treatment.
pub(in crate::main_control) fn append_native_math_character<'state, 'host, G>(
    processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
    ch: char,
    origin: OriginId,
) -> Result<(), ExecError> {
    let code = processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        math_character_code(context, ch)
    });
    if code >= 0x8000 {
        processor
            .treat_as_active_character(ch, origin)
            .map_err(command_error)
    } else {
        processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
            append_math_char(
                execution.modes.current_list_mutation(),
                context,
                code,
                origin,
            )
        })
    }
}
