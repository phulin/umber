//! Direct borrowed character admission into the live horizontal builder.
use super::*;
/// The one main-loop admission owner for ordinary source text and its scalar
/// boundaries. The command processor invokes `admit` once for a borrowed
/// prefix and again only for exceptional scalar events; this concrete owner
/// keeps hmode state, diagnostics, and the compact outcome together.
pub(super) struct MainSourceAdmission<'a> {
    pub(super) modes: &'a mut ModeNest,
    pub(super) etex_extended: bool,
    pub(super) run_error: &'a mut Option<ExecError>,
    pub(super) borrowed_applied: &'a mut bool,
    pub(super) count: &'a mut usize,
    pub(super) continues: &'a mut bool,
}

impl<G> tex_command::MainCharacterConsumer<G> for MainSourceAdmission<'_> {
    #[inline(always)]
    fn admit<'state, 'admission, 'fuel, 'effects, 'run>(
        &mut self,
        context: &'state mut CommandContext<'admission, G>,
        fuel: &'fuel mut tex_command::CommandFuel,
        diagnostic_effects: &'effects mut DiagnosticEffects,
        input: tex_command::MainCharacterInput<'run>,
    ) -> tex_command::CharacterRunAdmission {
        match input {
            tex_command::MainCharacterInput::Borrowed(run) => {
                let result = crate::box_runtime::append_character_run_with_fuel(
                    self.modes,
                    context,
                    &mut *diagnostic_effects,
                    run,
                    self.etex_extended,
                    fuel,
                );
                match result {
                    Ok(admitted) => {
                        *self.borrowed_applied |= admitted.count != 0;
                        // A lexical tail still belongs to §1038's main-loop
                        // label: the next scalar delivery may be a
                        // `char_given`/`char_num` command that must not gain a
                        // trace merely because the borrowed prefix stopped.
                        // Missing metrics, in contrast, leave the loop for
                        // the existing scalar fallback.
                        *self.continues = admitted.continue_run
                            || (admitted.count != 0
                                && matches!(
                                    admitted.fallback,
                                    Some(tex_command::CharacterRunFallback::LexicalBoundary)
                                ));
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
                        *self.run_error = Some(error);
                        tex_command::CharacterRunAdmission::new(0, false)
                    }
                }
            }
            tex_command::MainCharacterInput::Scalar { ch, origin } => {
                *self.count = (*self.count).saturating_add(1);
                if let Err(error) = crate::box_runtime::append_character_with_fuel(
                    self.modes,
                    context,
                    &mut *diagnostic_effects,
                    ch,
                    origin,
                    self.etex_extended,
                    fuel,
                ) {
                    *self.run_error = Some(error);
                    *self.continues = false;
                    return tex_command::CharacterRunAdmission::new(0, false);
                }
                *self.continues = u8::try_from(u32::from(ch)).ok().is_some_and(|code| {
                    context
                        .font_char_metrics(context.current_font(), code)
                        .is_some()
                });
                tex_command::CharacterRunAdmission::new(1, *self.continues)
            }
        }
    }
}
