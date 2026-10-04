//! TeX82 section 1211's actual prefix loop, inside the same scanner owner.
use super::*;
use tex_state::meaning::MeaningWord;

// Disjoint from the actual LONG/OUTER/PROTECTED meaning flags. This bit is
// call-local scope, never an eqtb meaning flag or retained command column.
pub(super) const GLOBAL: u8 = 1 << 7;

pub(super) fn scan<G>(
    processor: &mut Processor<'_, '_, G>,
    mut prefixes: u8,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let mut completion = None;
    processor
        .consume_expanded_words(&mut |processor,
                                      spelling,
                                      meaning,
                                      control_sequence,
                                      exceptional| {
            let code = meaning.control_code();
            match code {
                codes::RELAX | codes::CHAR_SPACE => return Ok(ControlFlow::Continue(())),
                codes::PRIMITIVE_GLOBAL => prefixes |= GLOBAL,
                codes::PRIMITIVE_LONG => prefixes |= MeaningFlags::LONG.bits(),
                codes::PRIMITIVE_OUTER => prefixes |= MeaningFlags::OUTER.bits(),
                codes::PRIMITIVE_PROTECTED => prefixes |= MeaningFlags::PROTECTED.bits(),
                _ => {
                    let static_value = static_meaning(meaning.resolve());
                    if !tex_command::exceeds_max_non_prefixed_command(static_value) {
                        completion = Some(discard_prefixes(
                            processor,
                            spelling,
                            meaning,
                            control_sequence,
                            exceptional,
                            true,
                        ));
                        return Ok(ControlFlow::Break(()));
                    }
                    let definition = matches!(
                        code,
                        codes::PRIMITIVE_DEF
                            | codes::PRIMITIVE_EDEF
                            | codes::PRIMITIVE_GDEF
                            | codes::PRIMITIVE_XDEF
                    );
                    if prefixes & !GLOBAL != 0 && !definition {
                        if let Err(error) = discard_prefixes(
                            processor,
                            spelling,
                            meaning,
                            control_sequence,
                            exceptional,
                            false,
                        ) {
                            completion = Some(Err(error));
                            return Ok(ControlFlow::Break(()));
                        }
                        prefixes &= GLOBAL;
                    }
                    completion = Some(dispatch::reswitch(
                        processor,
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                        receipts,
                        prefixes,
                        code,
                    ));
                    return Ok(ControlFlow::Break(()));
                }
            }
            Ok(ControlFlow::Continue(()))
        })
        .map_err(command_error)?;
    completion.ok_or(ExecError::MissingToken {
        context: "prefixed assignment",
    })?
}

fn discard_prefixes<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    invalid_command: bool,
) -> Result<MainControlStep, ExecError> {
    let site = processor.capture_consumed_site(spelling, meaning, control_sequence, exceptional);
    if invalid_command {
        processor
            .back_consumed_word(spelling, exceptional)
            .map_err(command_error)?;
    }
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
        let print = tex_command::PrintCommand::from_native(meaning, control_sequence);
        let display = command.output_open_context(context);
        let etex = command.profile().capabilities().supports_etex();
        let diagnostic = if invalid_command {
            PendingDiagnostic::PrefixOnNonPrefixedCommand(print, display, etex, Some(site))
        } else {
            PendingDiagnostic::IrrelevantLongOuterPrefix(print, display, etex, Some(site))
        };
        report_pending_diagnostics_in_context(context, effects, vec![diagnostic])?;
        Ok::<_, ExecError>(())
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}
