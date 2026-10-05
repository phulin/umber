//! Text accents run §1270 assignments inside the retained processor.
use super::*;

pub(super) fn apply<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let scanned = processor.scan_integer().map_err(command_error)?;
    report_scan(processor)?;
    let accent = u8::try_from(scanned.value).map_err(|_| ExecError::InvalidCode {
        context: "\\accent",
        value: scanned.value,
    })?;
    let extended = processor.profile() == CommandProfile::ETEX26;
    let (font, metrics) =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            execution.main_loop_active = false;
            crate::box_runtime::flush_pending_hchars(&mut execution.modes, context, effects, fuel)?;
            let font = context.current_font();
            let metrics = context.font_char_metrics(font, accent);
            if metrics.is_none() {
                crate::diagnostics::report_missing_character_warning(
                    context,
                    effects,
                    font,
                    char::from(accent),
                    extended,
                );
            }
            Ok::<_, ExecError>((font, metrics))
        })?;
    let Some(metrics) = metrics else {
        return Ok(MainControlStep::Continue);
    };
    let base = scan_base(processor, receipts)?;
    processor.with_execution_owners(
        |context, command, effects, _, observer, execution, _, _, _| {
            let mut geometry = pack_geometry_sink(command, observer);
            apply_accent_nodes(
                &mut execution.modes,
                context,
                effects,
                &mut geometry,
                extended,
                AccentPlacement {
                    accent,
                    accent_font: font,
                    accent_metrics: metrics,
                    accent_origin: scanned.provenance.primary,
                    base,
                },
            )
        },
    )
}

fn scan_base<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<Option<(u8, OriginId)>, ExecError> {
    let mut base = Ok(None);
    processor
        .consume_expanded_words(&mut tex_command::terminal_consumer::<
            Processor<'_, '_, G>,
            G,
            _,
        >(
            |processor, spelling, meaning, code, symbol, exceptional| {
                let value = static_meaning(meaning.resolve());
                match value {
                    Meaning::Relax
                    | Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } => {
                        return Ok(ControlFlow::Continue(()));
                    }
                    Meaning::CharToken {
                        ch,
                        cat: Catcode::Letter | Catcode::Other | Catcode::Active,
                    }
                    | Meaning::CharGiven(ch) => {
                        base = u8::try_from(ch as u32)
                            .map(|ch| Some((ch, spelling.origin())))
                            .map_err(|_| ExecError::InvalidCode {
                                context: "accent base",
                                value: ch as i32,
                            });
                    }
                    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
                        base = processor
                            .scan_integer()
                            .map_err(command_error)
                            .and_then(|value| {
                                u8::try_from(value.value)
                                    .map(|ch| Some((ch, spelling.origin())))
                                    .map_err(|_| ExecError::InvalidCode {
                                        context: "accent base",
                                        value: value.value,
                                    })
                            });
                    }
                    _ if tex_command::exceeds_max_non_prefixed_command(value) => {
                        processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                            execution.set_box_forbidden_depth += 1;
                        });
                        let step = dispatch::execute(
                            processor,
                            spelling,
                            meaning,
                            symbol,
                            exceptional,
                            receipts,
                            code,
                        );
                        processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                            execution.set_box_forbidden_depth -= 1;
                            execution.main_loop_active = false;
                        });
                        match step {
                            Ok(MainControlStep::Continue) => return Ok(ControlFlow::Continue(())),
                            Ok(_) => {}
                            Err(error) => base = Err(error),
                        }
                    }
                    _ => {
                        processor.back_consumed_word(spelling, exceptional)?;
                    }
                }
                Ok(ControlFlow::Break(()))
            },
        ))
        .map_err(command_error)?;
    report_scan(processor)?;
    base
}
