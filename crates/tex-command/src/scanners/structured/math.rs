use super::*;
use std::ops::ControlFlow;
use tex_state::meaning::{MeaningWord, StaticCommandClass};

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Completes one TeX82 §1151 `scan_math` field.
    ///
    /// §1151 is a classification, not an absorption:
    ///
    /// ```text
    /// begin restart:<Get the next non-blank non-relax non-call token>;
    /// reswitch: case cur_cmd of
    /// letter,other_char,char_given: begin c:=ho(math_code(cur_chr));
    ///     if c=@'100000 then begin <Treat cur_chr as an active character>;
    ///       goto restart; end; end;
    /// char_num: begin scan_char_num; cur_chr:=cur_val; cur_cmd:=char_given;
    ///   goto reswitch; end;
    /// math_char_num: begin scan_fifteen_bit_int; c:=cur_val; end;
    /// math_given: c:=cur_chr;
    /// delim_num: begin scan_twenty_seven_bit_int; c:=cur_val div @'10000; end;
    /// othercases <Scan a subformula enclosed in braces and return>
    /// endcases;
    /// ```
    ///
    /// Every scalar case ends holding a math code and nothing else: no input
    /// level is pushed, no token is backed up, and the command that carried
    /// the case is never delivered a second time. The only `back_input` in
    /// the whole procedure belongs to §1152's active-character restart and to
    /// §1153's braced field.
    ///
    /// `othercases` is the *entire* rest of the vocabulary, not just a left
    /// brace: §1153's `back_input; scan_left_brace` runs §403, which either
    /// consumes a real `{` or reports ``Missing { inserted``, backs the
    /// rejected command up, and behaves as though a brace had been read. The
    /// `math_group` opens either way, so a rejected command becomes the first
    /// token of the subformula body rather than being silently dropped.
    fn scan_math_field_restricted(
        &mut self,
        provenance: StructuredProvenance,
        kind: MathFieldRestrictedKind,
    ) -> Result<Option<MathFieldEpisode>, CommandError> {
        let class = match kind {
            MathFieldRestrictedKind::Character => RestrictedIntegerClass::CharacterCode,
            MathFieldRestrictedKind::MathCharacter => RestrictedIntegerClass::FifteenBit,
            MathFieldRestrictedKind::Delimiter => RestrictedIntegerClass::TwentySevenBit,
        };
        let result = self.scan_restricted_integer(class);
        let scanned = result?;
        let (code, provenance) = match kind {
            MathFieldRestrictedKind::Character => {
                let ch = char::from_u32(scanned.value as u32)
                    .expect("recovered character number is in range");
                let code = self.state.mathcode(ch);
                if code == 0o100000 {
                    self.treat_as_active_character(ch, provenance.primary)?;
                    return Ok(None);
                }
                (code as u16, provenance)
            }
            MathFieldRestrictedKind::MathCharacter => (
                scanned.value as u16,
                StructuredProvenance {
                    primary: scanned.provenance.primary,
                },
            ),
            MathFieldRestrictedKind::Delimiter => (
                (scanned.value as u32 / 0o10000) as u16,
                StructuredProvenance {
                    primary: scanned.provenance.primary,
                },
            ),
        };
        Ok(Some(MathFieldEpisode {
            body: MathFieldBody::Character(code),
            provenance,
        }))
    }

    pub fn scan_math_field_episode(&mut self) -> Result<MathFieldEpisode, CommandError> {
        let mut field = None;
        self.consume_expanded_words(&mut |processor, spelling, meaning, _, exceptional| {
            let provenance = StructuredProvenance {
                primary: spelling.origin(),
            };
            if let MeaningWord::Static(word) = meaning {
                let character = Meaning::runtime_word_character(*word);
                if matches!(character, Some((_, Catcode::Space))) || *word == Meaning::RELAX_WORD {
                    return Ok(ControlFlow::Continue(()));
                }
                let character = match character {
                    Some((ch, Catcode::Letter | Catcode::Other)) => Some(ch),
                    _ if Meaning::runtime_word_control_code(*word)
                        == Meaning::runtime_word_control_code(Meaning::CharGiven('A').encode()) =>
                    {
                        Some(
                            char::from_u32(Meaning::runtime_word_operand(*word) as u32)
                                .expect("validated character-definition meaning"),
                        )
                    }
                    _ => None,
                };
                if let Some(ch) = character {
                    let code = processor.state.mathcode(ch);
                    if code == 0o100000 {
                        processor.treat_as_active_character(ch, provenance.primary)?;
                        return Ok(ControlFlow::Continue(()));
                    }
                    field = Some(MathFieldEpisode {
                        body: MathFieldBody::Character(code as u16),
                        provenance,
                    });
                    return Ok(ControlFlow::Break(()));
                }
                if Meaning::runtime_word_control_code(*word)
                    == Meaning::runtime_word_control_code(Meaning::MathCharGiven(0).encode())
                {
                    field = Some(MathFieldEpisode {
                        body: MathFieldBody::Character(Meaning::runtime_word_operand(*word) as u16),
                        provenance,
                    });
                    return Ok(ControlFlow::Break(()));
                }
                if Meaning::runtime_word_class(*word) == StaticCommandClass::Unexpandable
                    && let Some(
                        primitive @ (UnexpandablePrimitive::Char
                        | UnexpandablePrimitive::MathChar
                        | UnexpandablePrimitive::Delimiter),
                    ) = UnexpandablePrimitive::from_operand(Meaning::runtime_word_operand(*word))
                {
                    let kind = match primitive {
                        UnexpandablePrimitive::Char => MathFieldRestrictedKind::Character,
                        UnexpandablePrimitive::MathChar => MathFieldRestrictedKind::MathCharacter,
                        _ => MathFieldRestrictedKind::Delimiter,
                    };
                    field = processor.scan_math_field_restricted(provenance, kind)?;
                    return Ok(if field.is_some() {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    });
                }
            }
            // §1153 backs up before §403, including a real left brace.
            processor.back_consumed_word(spelling, exceptional)?;
            let primary = processor.scan_left_brace()?.unwrap_or(OriginId::UNKNOWN);
            field = Some(MathFieldEpisode {
                body: MathFieldBody::OpenGroup,
                provenance: StructuredProvenance { primary },
            });
            Ok(ControlFlow::Break(()))
        })?;
        Ok(field.unwrap_or(MathFieldEpisode {
            body: MathFieldBody::Missing,
            provenance: StructuredProvenance {
                primary: OriginId::UNKNOWN,
            },
        }))
    }

    /// Consumes the mandatory opening brace of one `\mathchoice` branch.
    ///
    /// TeX82 §1172's `append_choices` and §1174's `build_choices` both end in
    /// `push_math(math_choice_group); scan_left_brace`, so all four branches
    /// go through this one scan. Nothing is absorbed: like §1153's braced
    /// math field, a branch body is ordinary input that main control reads
    /// live, closed by §1174's `math_choice_group` arm of
    /// `handle_right_brace`.
    ///
    /// §403's recovery is to behave as though a `{` had been read, so the
    /// group opens either way and the rejected command -- already backed up
    /// by `scan_left_brace` -- becomes the first thing the branch body
    /// reads. The returned flag reports only whether that recovery ran.
    pub fn scan_math_choice_group(&mut self) -> Result<bool, CommandError> {
        Ok(self.scan_left_brace()?.is_none())
    }

    /// Completes the delimiter immediately following a structural math
    /// boundary (`\left`, `\right`, or `\middle`).
    pub fn scan_math_delimiter_boundary(
        &mut self,
        kind: MathDelimiterBoundaryKind,
    ) -> Result<MathDelimiterBoundary, CommandError> {
        Ok(MathDelimiterBoundary {
            kind,
            delimiter: self.scan_delimiter(false)?,
        })
    }

    /// Scans TeX82 §436's `scan_fifteen_bit_int` math-character number.
    pub fn scan_math_character(&mut self) -> Result<ScannedMathCharacter, CommandError> {
        let result = self.scan_restricted_integer(RestrictedIntegerClass::FifteenBit);
        let scanned = result?;
        Ok(ScannedMathCharacter {
            code: scanned.value as u16,

            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// Scans TeX82 §437's `scan_twenty_seven_bit_int` delimiter number: an
    /// ordinary `scan_int` whose result is replaced by zero when it leaves
    /// the 27-bit range.
    ///
    /// This is the whole delimiter operand only where tex.web calls
    /// `scan_twenty_seven_bit_int` directly -- §1154's `mmode+delim_num` and
    /// §1151's `scan_math` `delim_num` case -- and the `r=true` half of
    /// §1160. Every other delimiter position goes through
    /// [`Self::scan_delimiter`].
    pub fn scan_delimiter_number(&mut self) -> Result<ScannedMathDelimiter, CommandError> {
        let result = self.scan_restricted_integer(RestrictedIntegerClass::TwentySevenBit);
        let scanned = result?;
        Ok(ScannedMathDelimiter {
            code: scanned.value as u32,

            missing_delimiter: false,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// TeX82 §1160's `scan_delimiter(p, r)`.
    ///
    /// `radical` is tex.web's `r`, "tells if this delimiter follows
    /// `\radical` or not". Only §1163's `math_radical` passes `true`, and
    /// only then is the operand a bare `scan_twenty_seven_bit_int`. Every
    /// other delimiter position -- §1191's `\left`/`\right`, §1192's
    /// `\right` recovery, §1182's `\abovewithdelims` family and §1183's
    /// ambiguous-fraction recovery -- passes `false`, where §1160 instead
    /// fetches §404's next non-blank non-relax non-call token and classifies
    /// it:
    ///
    /// ```text
    /// letter,other_char: cur_val:=del_code(cur_chr);
    /// delim_num: scan_twenty_seven_bit_int;
    /// othercases cur_val:=-1
    /// ```
    ///
    /// so `\left(` reads `(`'s `\delcode` and `\left\delimiter"426830A`
    /// consumes the already-delivered `\delimiter` in place. Scanning the
    /// `r=false` positions as if they were `r=true` made the fetched command
    /// the first token of a `scan_int` instead: `\delimiter` is not a numeric
    /// constant, so §444's `vacuous` case backed it up, published a zero
    /// delimiter, and then re-delivered `\delimiter` to main control as an
    /// independent §1154 math character.
    ///
    /// §1161 owns the negative result: `back_error` returns the rejected
    /// token to the input and the delimiter becomes null. The `delim_num`
    /// branch cannot reach it, because §437 has already clamped an
    /// out-of-range code to zero.
    pub fn scan_delimiter(&mut self, radical: bool) -> Result<ScannedMathDelimiter, CommandError> {
        if radical {
            return self.scan_delimiter_number();
        }
        let mut delimiter = None;
        self.consume_expanded_words(&mut |processor,
                                          spelling,
                                          meaning,
                                          control_sequence,
                                          exceptional| {
            let code = if let MeaningWord::Static(word) = meaning {
                match Meaning::runtime_word_character(*word) {
                    Some((_, Catcode::Space)) => return Ok(ControlFlow::Continue(())),
                    Some((ch, Catcode::Letter | Catcode::Other)) => processor.state.delcode(ch),
                    _ if *word == Meaning::RELAX_WORD => return Ok(ControlFlow::Continue(())),
                    _ if Meaning::runtime_word_class(*word) == StaticCommandClass::Unexpandable
                        && UnexpandablePrimitive::from_operand(Meaning::runtime_word_operand(
                            *word,
                        )) == Some(UnexpandablePrimitive::Delimiter) =>
                    {
                        delimiter = Some(processor.scan_delimiter_number()?);
                        return Ok(ControlFlow::Break(()));
                    }
                    _ => -1,
                }
            } else {
                -1
            };
            let primary = spelling.origin();
            if code < 0 {
                // §1161 captures the rejected delivery, then backs it up before
                // reporting. The following source token remains untouched.
                let site = processor.capture_consumed_site(
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
                processor.back_consumed_word(spelling, exceptional)?;
                processor.missing_delimiter_error(Some(site))?;
            }
            delimiter = Some(ScannedMathDelimiter {
                code: if code < 0 { 0 } else { code as u32 },
                missing_delimiter: code < 0,
                provenance: StructuredProvenance { primary },
            });
            Ok(ControlFlow::Break(()))
        })?;
        Ok(delimiter.unwrap_or(ScannedMathDelimiter {
            code: 0,
            missing_delimiter: true,
            provenance: StructuredProvenance {
                primary: OriginId::UNKNOWN,
            },
        }))
    }

    /// TeX82 §1161's invalid-delimiter `back_error` report.
    ///
    /// This belongs to the scanner episode, immediately after the rejected
    /// token is backed up. In particular, §1182 must report both delimiter
    /// recoveries before §448 starts scanning `\abovewithdelims`'s rule
    /// thickness. Deferring the reports to stomach application reverses that
    /// order when the thickness itself takes §446's missing-number recovery.
    fn missing_delimiter_error(
        &mut self,
        site: Option<tex_state::diagnostic::DiagnosticSite>,
    ) -> Result<(), CommandError> {
        let context = self.command.output_open_context(self.state);
        let site =
            Some(self.complete_diagnostic_site(
                site.unwrap_or_else(|| self.capture_diagnostic_site(None)),
            ));
        if !self.command.semantic_diagnostics.is_empty() || self.command.expanding_deferred_write()
        {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: MISSING_DELIMITER_DIAGNOSTIC,
                    runaway: None,
                    message: "Missing delimiter (. inserted)".into(),
                    help: MISSING_DELIMITER_HELP,
                    context,
                    integer_error: None,
                    site,
                });
            return Ok(());
        }
        let mut report = self.state.print_err("Missing delimiter (. inserted)");
        report.help(MISSING_DELIMITER_HELP).context(context);
        let outcome = report.error_with_effects_at(self.diagnostic_effects, site);
        self.finish_error_outcome(outcome)?;
        Ok(())
    }

    /// Collects the command-owned scalar prefix of TeX82's generalized
    /// fraction forms. Numerator/denominator mlist construction stays in the
    /// executor and is deliberately absent from this scanner boundary.
    pub fn scan_math_fraction(
        &mut self,
        kind: MathFractionKind,
        with_delimiters: bool,
    ) -> Result<ScannedMathFraction, CommandError> {
        let (left_delimiter, right_delimiter) = if with_delimiters {
            (
                Some(self.scan_delimiter(false)?),
                Some(self.scan_delimiter(false)?),
            )
        } else {
            (None, None)
        };
        let thickness = match kind {
            MathFractionKind::Above => {
                let result = self.scan_dimension();
                Some(result?)
            }
            MathFractionKind::Atop => Some(Scaled::from_raw(0)),
            MathFractionKind::Over => None,
        };
        Ok(ScannedMathFraction {
            kind,
            left_delimiter,
            right_delimiter,
            thickness,
        })
    }

    /// Scans the `mu`-unit material operands used only in math mode.
    pub fn scan_math_mu_material(
        &mut self,
        glue: bool,
    ) -> Result<ScannedMathMuMaterial, CommandError> {
        if glue {
            let result = self.scan_glue(true);
            let spec = result?;
            Ok(ScannedMathMuMaterial::Glue {
                spec,
                shared_zero: self.scanned_glue_shared_zero,
            })
        } else {
            let result = self.scan_mu_dimension();
            Ok(ScannedMathMuMaterial::Kern(result?))
        }
    }

    /// Completes the scalar portion of one math-mode command.  Any following
    /// field or braced list is intentionally represented by a later opaque
    /// replay episode, so the stomach never receives a source cursor.
    ///
    /// The table is keyed on the delivered [`Meaning`], not on
    /// [`UnexpandablePrimitive`], because TeX82's math vocabulary is not
    /// exclusively primitive-shaped: §1154's `mmode+math_given` case carries
    /// its math code in the delivered command itself (a `\\mathchardef`
    /// target, §1224), exactly as `mmode+math_char_num` carries it in the
    /// integer `\\mathchar` scans. Keying on the primitive alone silently
    /// excluded `math_given` from the whole mmode table.
    pub fn scan_math_request(
        &mut self,
        command: &crate::CurrentCommand<G>,
    ) -> Result<Option<MathRequest>, CommandError> {
        use MathRequest as Request;
        use MathTextFieldKind as Field;
        // TeX82 §1154's `mmode+math_given: set_math_char(cur_chr)`. Unlike
        // `mmode+math_char_num`, which reaches the same `set_math_char`
        // (§1155) through §436's `scan_fifteen_bit_int`, the code is already
        // complete in the delivered command, so nothing is scanned and the
        // math char's provenance is the delivering token's own origin.
        if let Some(Meaning::MathCharGiven(code)) = static_meaning(command.meaning()) {
            return Ok(Some(Request::Character(ScannedMathCharacter {
                code,

                provenance: StructuredProvenance {
                    primary: command.origin(),
                },
            })));
        }
        let Some(Meaning::UnexpandablePrimitive(primitive)) = static_meaning(command.meaning())
        else {
            return Ok(None);
        };
        let request = match primitive {
            UnexpandablePrimitive::MathChar => Request::Character(self.scan_math_character()?),
            UnexpandablePrimitive::Delimiter => Request::Delimiter(self.scan_delimiter_number()?),
            UnexpandablePrimitive::MathOrd => Request::TextField(Field::Ord),
            UnexpandablePrimitive::MathOp => Request::TextField(Field::Op),
            UnexpandablePrimitive::MathBin => Request::TextField(Field::Bin),
            UnexpandablePrimitive::MathRel => Request::TextField(Field::Rel),
            UnexpandablePrimitive::MathOpen => Request::TextField(Field::Open),
            UnexpandablePrimitive::MathClose => Request::TextField(Field::Close),
            UnexpandablePrimitive::MathPunct => Request::TextField(Field::Punct),
            UnexpandablePrimitive::MathInner => Request::TextField(Field::Inner),
            UnexpandablePrimitive::Underline => Request::TextField(Field::Underline),
            UnexpandablePrimitive::Overline => Request::TextField(Field::Overline),
            UnexpandablePrimitive::Limits => Request::Limits(MathLimitKind::Limits),
            UnexpandablePrimitive::NoLimits => Request::Limits(MathLimitKind::NoLimits),
            UnexpandablePrimitive::DisplayLimits => Request::Limits(MathLimitKind::DisplayLimits),
            UnexpandablePrimitive::Over => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Over, false)?)
            }
            UnexpandablePrimitive::Atop => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Atop, false)?)
            }
            UnexpandablePrimitive::Above => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Above, false)?)
            }
            UnexpandablePrimitive::OverWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Over, true)?)
            }
            UnexpandablePrimitive::AtopWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Atop, true)?)
            }
            UnexpandablePrimitive::AboveWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Above, true)?)
            }
            UnexpandablePrimitive::Radical => Request::Radical(self.scan_delimiter(true)?),
            // TeX82 §1110 diagnoses a text `\accent` before `math_ac`
            // reaches §436's `scan_fifteen_bit_int`. Keep that operand
            // pending so §82's `show_context` still sees the input level
            // that delivered the command. A real `\mathaccent` has no
            // intervening error and can complete its scalar scan here.
            UnexpandablePrimitive::Accent => Request::Accent { character: None },
            UnexpandablePrimitive::MathAccent => Request::Accent {
                character: Some(self.scan_math_character()?),
            },
            UnexpandablePrimitive::MSkip => Request::MuMaterial(self.scan_math_mu_material(true)?),
            UnexpandablePrimitive::MKern => Request::MuMaterial(self.scan_math_mu_material(false)?),
            UnexpandablePrimitive::MathChoice => Request::Choice,
            UnexpandablePrimitive::EqNo => Request::EquationNumber(ScannedEquationNumber {
                side: EquationNumberSide::Right,
            }),
            UnexpandablePrimitive::LeftEqNo => Request::EquationNumber(ScannedEquationNumber {
                side: EquationNumberSide::Left,
            }),
            _ => return Ok(None),
        };
        Ok(Some(request))
    }
}
