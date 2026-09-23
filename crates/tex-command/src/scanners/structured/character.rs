use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    /// Scans TeX82 §1123's `make_accent` accent code.
    ///
    /// §1123 is `scan_char_num; f:=cur_font; p:=new_character(f,cur_val)` and
    /// only then `do_assignments`, so the accent code is the whole of what the
    /// command layer owns before the executor takes over.
    pub fn scan_accent(&mut self) -> Result<ScannedAccent, CommandError> {
        let result = self.scan_integer_retained();
        let accent = result.into_result()?;
        Ok(ScannedAccent {
            accent: accent.value,
            accent_provenance: StructuredProvenance {
                primary: accent.provenance.primary,
            },
        })
    }

    /// Delivers one step of TeX82 §1123's post-`scan_char_num` lookahead.
    ///
    /// §404's `<Get the next non-blank non-relax non-call token>` is shared by
    /// §1270's `do_assignments` and §1124's base-character classification --
    /// §1270 leaves the token it stops on in `cur_cmd`, and §1124 classifies
    /// exactly that token. This therefore performs the fetch and §1124's
    /// classification, and hands any other command back to the executor.
    ///
    /// A `prefixed_command` must not be replayed. §1270 executes it in place,
    /// with no `back_input` at all, so backing it up would push a backup
    /// level, emit a recovery record and deliver the command a second time,
    /// none of which tex.web does (`umber2-johp.196`, `umber2-johp.264`). It
    /// is handed to the executor still delivered; only §1124's own `else`
    /// branch replays, and it does so here, inside the delivery episode that
    /// owns the command.
    pub fn scan_accent_base(&mut self) -> Result<ScannedAccentBase<G>, CommandError> {
        let mut destination = None;
        match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
            DeliveryStatus::End => return Ok(ScannedAccentBase::Missing),
            DeliveryStatus::Command => {}
            _ => return Err(CommandError::input_invariant()),
        };
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        let provenance = StructuredProvenance {
            primary: command.origin(),
        };
        match static_meaning(command.meaning()) {
            Some(Meaning::CharToken {
                ch,
                cat: Catcode::Letter | Catcode::Other,
            })
            | Some(Meaning::CharGiven(ch))
            | Some(Meaning::CharToken {
                ch,
                cat: Catcode::Active,
            }) => {
                let character =
                    u8::try_from(ch as u32).map_err(|_| CommandError::input_invariant())?;
                Ok(ScannedAccentBase::Character {
                    character,
                    provenance,
                })
            }
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)) => {
                let result = self.scan_integer_retained();
                let character = u8::try_from(result.into_result()?.value)
                    .map_err(|_| CommandError::input_invariant())?;
                Ok(ScannedAccentBase::Character {
                    character,
                    provenance,
                })
            }
            Some(meaning) if crate::primitives::is_prefixed_command(meaning) => {
                Ok(ScannedAccentBase::Assignment(command))
            }
            _ => {
                self.back_input(command)?;
                Ok(ScannedAccentBase::Missing)
            }
        }
    }

    /// Delivers one command from TeX82 §1270's `do_assignments` fetch.
    ///
    /// The fetch itself is §404's "next non-blank non-relax non-call token".
    /// Callers must dispatch the returned command in place: `do_assignments`
    /// neither backs up assignments nor refetches the first non-assignment it
    /// stops on. This boundary is also used by §1206 after `fin_align`, where
    /// blanks before the display-closing command must not reach main control.
    pub fn next_do_assignments_command(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => Err(CommandError::input_invariant()),
        }
    }

    /// Consumes only §1117/§1120's opening brace. The body remains on the
    /// live input stack and returns to main control in restricted horizontal
    /// mode; in particular, no macro or conditional from the body is expanded
    /// before the executor has installed `disc_group`.
    pub fn scan_discretionary_opening(
        &mut self,
    ) -> Result<ScannedDiscretionaryOpening, CommandError> {
        let opening = self.scan_left_brace(true)?;
        Ok(ScannedDiscretionaryOpening {
            provenance: StructuredProvenance {
                primary: opening.origin(),
            },
        })
    }
}
