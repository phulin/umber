use super::*;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Scans TeX82's `\\openin`, `\\closein`, `\\read`, and e-TeX's
    /// `\\readline` operands without exposing a raw delivery to replay.
    pub fn scan_input_stream_request(
        &mut self,
        primitive: tex_state::meaning::UnexpandablePrimitive,
        read_global: bool,
    ) -> Result<InputStreamRequest<G>, CommandError> {
        use tex_state::meaning::UnexpandablePrimitive;
        match primitive {
            // §§1272--1275's `in_stream` command scans §435's
            // `scan_four_bit_int`. Recovery is complete before the request is
            // committed; §435 reports from the original integer here,
            // before filename acquisition and stream mutation.
            UnexpandablePrimitive::OpenIn => {
                let result = self.scan_restricted_integer(RestrictedIntegerClass::FourBit);
                let scanned = result?;
                let result = self.scan_optional_equals();
                result?;
                let result = self.scan_file_name();
                let file_name = result?;
                Ok(InputStreamRequest::Open {
                    stream: scanned.value,

                    file_name,
                })
            }
            UnexpandablePrimitive::CloseIn => {
                let result = self.scan_restricted_integer(RestrictedIntegerClass::FourBit);
                let scanned = result?;
                Ok(InputStreamRequest::Close {
                    stream: scanned.value,
                })
            }
            UnexpandablePrimitive::Read | UnexpandablePrimitive::ReadLine => {
                // §1225's `read_to_cs` scans a plain `scan_int`, *not*
                // §435's four-bit selector: §482 answers an out-of-range
                // stream with `if (n<0)or(n>15) then m:=16`, reading from the
                // terminal, and no error is reported at all.
                let result = self.scan_integer();
                let stream = result?.value;
                // tex.web §1225 reports a missing `to` and inserts it, then
                // runs `get_r_token` regardless: the keyword is recovered,
                // not required. §1225 reports it *here*, between the failed
                // keyword and `get_r_token`, so §82's context still shows the
                // target as `<to be read again>` and no `read_toks` prompt has
                // been printed yet.

                let result = self.scan_keyword("to");
                let found_to = result?;
                if !found_to {
                    let context = self.command.output_open_context(self.state);
                    let mut report = self.state.print_err("Missing `to' inserted");
                    report.help(&[
                        "You should have said `\\read<number> to \\cs'.",
                        "I'm going to look for the \\cs now.",
                    ]);
                    report.context(context);
                    let outcome = report.error();
                    self.finish_error_outcome(outcome)?;
                }
                // §1215's `get_r_token` backs a rejected ordinary target up
                // immediately. Its §325 stack-conservation step first retires
                // the exhausted keyword-mismatch backup, leaving the rejected
                // target live below §483's temporary read-line source.
                let target = self.scan_definition_target()?;
                // TeX82 §1225: `\\read` scans `n`, `to`, and `r`, then runs
                // §482's `read_toks(n,r)` on the spot. The collector needs
                // live input levels, category codes, `align_state`, and
                // `scanner_status`, all of which are the command core's.
                let definition =
                    self.read_toks(stream, target, primitive == UnexpandablePrimitive::ReadLine)?;
                Ok(InputStreamRequest::Read {
                    stream,
                    target,
                    global: read_global,
                    definition,
                })
            }
            _ => Err(CommandError::input_invariant()),
        }
    }

    /// Scans a complete ordinary font definition without retaining a raw
    /// command, cursor, or host capability.
    ///
    /// TeX82 §1257's `new_font` runs `define(u,set_font,null_font)` on the
    /// `get_r_token` target *before* `scan_optional_equals` and
    /// `scan_file_name`, exactly as §1224 gives a `\\chardef` target a
    /// provisional `\\relax`. The identifier therefore already denotes the
    /// null font while the file name and `at`/`scaled` size are scanned, and
    /// §1257's `common_ending: equiv(u):=f` later overwrites that equivalent
    /// in place rather than through a second `eq_define`.
    pub fn scan_font_definition(
        &mut self,
        provisional_global: bool,
    ) -> Result<FontLoadRequest, CommandError> {
        let target = self.scan_definition_target()?;
        self.state.set_provisional_meaning(
            target,
            Meaning::Font(tex_state::font::NULL_FONT),
            provisional_global,
        );
        observe!(
            self,
            crate::CommandObservation::Mutation(crate::MutationRecord {
                target: crate::MutationTarget::Meaning,
                key: crate::ObservationValue::Name(self.state.resolve(target).to_owned()),
                value: crate::ObservationValue::Name("set_font".into()),
                global: provisional_global,
            }),
        );
        let result = self.scan_optional_equals();
        result?;
        let result = self.scan_file_name();
        let file_name = result?;
        let mut size_recovery = None;
        let has_at = self.scan_keyword("at")?;
        let size = if has_at {
            let result = self.scan_dimension();
            let requested = result?;
            // §1259's `if (s<=0)or(s>=@'1000000000)`.
            FontSizeSpec::At(
                if requested.raw() > 0 && requested.raw() < 2048 * Scaled::UNITY {
                    requested
                } else {
                    size_recovery = Some(FontSizeRecovery::ImproperAtSize {
                        size: requested,
                        context: self.command.output_open_context(self.state),
                    });
                    Scaled::from_raw(10 * Scaled::UNITY)
                },
            )
        } else {
            let result = self.scan_keyword("scaled");
            let scaled = result?;
            if scaled {
                let result = self.scan_integer();
                let requested = result?.value;
                // §1258's `if (cur_val<=0)or(cur_val>32768)`.
                FontSizeSpec::Scale(if (1..=32_768).contains(&requested) {
                    requested
                } else {
                    size_recovery = Some(FontSizeRecovery::IllegalMagnification {
                        value: requested,
                        context: self.command.output_open_context(self.state),
                    });
                    1000
                })
            } else {
                FontSizeSpec::Design
            }
        };
        Ok(FontLoadRequest {
            target,
            name: file_name.packed(),
            size,
            size_recovery,
            error_context: self.command.output_open_context(self.state),
        })
    }

    /// Scans pdfTeX's `\pdfcopyfont` and `\letterspacefont` definitions.
    ///
    /// Like TeX82 §1257's `new_font`, pdfTeX installs `nullfont` before it
    /// scans any operand following the target. This is significant for a
    /// self-referential definition and for every later failure path.
    pub fn scan_generated_font_definition(
        &mut self,
        kind: GeneratedFontKind,
        provisional_global: bool,
    ) -> Result<ScannedGeneratedFontDefinition, CommandError> {
        let target = self.scan_definition_target()?;
        self.state.set_provisional_meaning(
            target,
            Meaning::Font(tex_state::font::NULL_FONT),
            provisional_global,
        );
        observe!(
            self,
            crate::CommandObservation::Mutation(crate::MutationRecord {
                target: crate::MutationTarget::Meaning,
                key: crate::ObservationValue::Name(self.state.resolve(target).to_owned()),
                value: crate::ObservationValue::Name("set_font".into()),
                global: provisional_global,
            }),
        );
        let result = self.scan_optional_equals();
        result?;
        let result = self.scan_font_selector();
        let source = result?;
        let (amount, no_ligatures) = match kind {
            GeneratedFontKind::Copy => (0, false),
            GeneratedFontKind::Letterspace => {
                let result = self.scan_integer();
                let amount = result?.value.clamp(-1000, 1000) as i16;

                let result = self.scan_keyword("nolig");
                let no_ligatures = result?;
                (amount, no_ligatures)
            }
        };
        Ok(ScannedGeneratedFontDefinition {
            kind,
            target,
            source,
            amount,
            no_ligatures,
        })
    }
}
