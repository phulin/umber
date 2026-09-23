use super::*;

impl<G> CommandProcessor<'_, '_, G> {
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
        let result = self.scan_restricted_integer_retained(class);
        let scanned = result.into_result()?;
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
        let mut destination = None;
        loop {
            // §1151's `restart` label: §404's shared "next non-blank
            // non-relax non-call token", the same fetch §403 opens with.
            match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
                DeliveryStatus::End => {
                    return Ok(MathFieldEpisode {
                        body: MathFieldBody::Missing,
                        provenance: StructuredProvenance {
                            primary: OriginId::UNKNOWN,
                        },
                    });
                }
                DeliveryStatus::Command => {}
                _ => return Err(CommandError::input_invariant()),
            };
            let command = destination.take().ok_or(CommandError::input_invariant())?;
            let provenance = StructuredProvenance {
                primary: command.origin(),
            };
            // §1151's `reswitch`: `char_num` scans its selector and re-enters
            // the table as `char_given`, so both reach one `math_code` read.
            let character = match static_meaning(command.meaning()) {
                Some(Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                }) => Some(ch),
                Some(Meaning::CharGiven(ch)) => Some(ch),
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)) => {
                    if let Some(field) = self.scan_math_field_restricted(
                        provenance,
                        MathFieldRestrictedKind::Character,
                    )? {
                        return Ok(field);
                    }
                    continue;
                }
                _ => None,
            };
            if let Some(ch) = character {
                let code = self.state.mathcode(ch);
                // §1151 tests `c=@'100000` exactly, and §1152 then resolves
                // the character's active meaning, expands it once with
                // `x_token`, backs the result up, and restarts the field.
                if code == 0o100000 {
                    self.treat_as_active_character(ch, provenance.primary)?;
                    continue;
                }
                return Ok(MathFieldEpisode {
                    body: MathFieldBody::Character(code as u16),
                    provenance,
                });
            }
            let (code, provenance) = match static_meaning(command.meaning()) {
                // §1224's `\mathchardef` target carries its own code.
                Some(Meaning::MathCharGiven(code)) => (code, provenance),
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathChar)) => {
                    return self
                        .scan_math_field_restricted(
                            provenance,
                            MathFieldRestrictedKind::MathCharacter,
                        )?
                        .ok_or_else(|| CommandError::input_invariant());
                }
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter)) => {
                    return self
                        .scan_math_field_restricted(provenance, MathFieldRestrictedKind::Delimiter)?
                        .ok_or_else(|| CommandError::input_invariant());
                }
                // §1153's `othercases`, verbatim: `back_input;
                // scan_left_brace; ... push_math(math_group); return`. The
                // brace is re-read rather than consumed from `command`
                // because §403's skipped spaces and its missing-brace
                // recovery are both observable.
                _ => {
                    self.back_input(command)?;
                    return Ok(match self.scan_left_brace(true)? {
                        crate::scan_toks::ScannedLeftBrace::Consumed(opening) => MathFieldEpisode {
                            body: MathFieldBody::OpenGroup,
                            provenance: StructuredProvenance {
                                primary: opening.origin(),
                            },
                        },
                        // §403's recovery reaches §1153 with `cur_cmd =
                        // left_brace`, so `push_math(math_group)` runs
                        // unconditionally; the rejected command is already
                        // backed up and opens the body.
                        crate::scan_toks::ScannedLeftBrace::Inserted => MathFieldEpisode {
                            body: MathFieldBody::OpenGroup,
                            provenance: StructuredProvenance {
                                primary: OriginId::UNKNOWN,
                            },
                        },
                    });
                }
            };
            return Ok(MathFieldEpisode {
                body: MathFieldBody::Character(code),
                provenance,
            });
        }
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
        Ok(matches!(
            self.scan_left_brace(true)?,
            crate::scan_toks::ScannedLeftBrace::Inserted
        ))
    }

    /// Completes the delimiter immediately following a structural math
    /// boundary (`\left`, `\right`, or `\middle`).
    pub fn scan_math_delimiter_boundary(
        &mut self,
        kind: MathDelimiterBoundaryKind,
    ) -> Result<MathDelimiterBoundary, CommandError> {
        let mut destination = None;
        match self.next_non_blank_non_relax_x_token_hot(&mut destination)? {
            DeliveryStatus::End => {
                return Ok(MathDelimiterBoundary {
                    kind,
                    delimiter: ScannedMathDelimiter {
                        code: 0,
                        recovered: true,
                        missing_delimiter: true,
                        provenance: StructuredProvenance {
                            primary: OriginId::UNKNOWN,
                        },
                    },
                });
            }
            DeliveryStatus::Command
            | DeliveryStatus::PendingExpanded
            | DeliveryStatus::AlignmentClosingBrace => {}
            DeliveryStatus::AlignmentEndTemplate
            | DeliveryStatus::CharacterRun
            | DeliveryStatus::CharacterRunBoundary => {
                return Err(CommandError::input_invariant());
            }
            DeliveryStatus::ReplayCompleted(_) => unreachable!(),
        }
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        let primary = command.origin();
        let code = match command.command_word().static_meaning() {
            Some(Meaning::CharToken {
                ch,
                cat: Catcode::Letter | Catcode::Other,
            }) => self.state.delcode(ch),
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter)) => {
                return Ok(MathDelimiterBoundary {
                    kind,
                    delimiter: self.scan_delimiter_number()?,
                });
            }
            // `char_given` and `math_given` are distinct TeX command codes;
            // neither is one of §1160's accepted delimiter cases. Keep them
            // explicit here so a compact terminal command cannot fall
            // through to a second expanded fetch.
            Some(Meaning::CharGiven(_) | Meaning::MathCharGiven(_)) => -1,
            _ => -1,
        };
        if code < 0 {
            // TeX82 §1161: back up exactly the one rejected delivery before
            // reporting the recovery. The following source token has not
            // entered the scanner and remains untouched.
            let site = self.capture_hot_diagnostic_site(&command);
            self.back_input_hot(command)?;
            self.missing_delimiter_error(Some(site))?;
            return Ok(MathDelimiterBoundary {
                kind,
                delimiter: ScannedMathDelimiter {
                    code: 0,
                    recovered: true,
                    missing_delimiter: true,
                    provenance: StructuredProvenance { primary },
                },
            });
        }
        Ok(MathDelimiterBoundary {
            kind,
            delimiter: ScannedMathDelimiter {
                code: code as u32,
                recovered: false,
                missing_delimiter: false,
                provenance: StructuredProvenance { primary },
            },
        })
    }

    /// Scans TeX82 §436's `scan_fifteen_bit_int` math-character number.
    pub fn scan_math_character(&mut self) -> Result<ScannedMathCharacter, CommandError> {
        let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::FifteenBit);
        let scanned = result.into_result()?;
        Ok(ScannedMathCharacter {
            code: scanned.value as u16,
            recovered: scanned.recovered,
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
        let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::TwentySevenBit);
        let scanned = result.into_result()?;
        Ok(ScannedMathDelimiter {
            code: scanned.value as u32,
            recovered: scanned.recovered,
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
        let mut destination = None;
        match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
            DeliveryStatus::End => {
                return Ok(ScannedMathDelimiter {
                    code: 0,
                    recovered: true,
                    missing_delimiter: true,
                    provenance: StructuredProvenance {
                        primary: OriginId::UNKNOWN,
                    },
                });
            }
            DeliveryStatus::Command => {}
            _ => return Err(CommandError::input_invariant()),
        };
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        let primary = command.origin();
        let code = match static_meaning(command.meaning()) {
            Some(Meaning::CharToken {
                ch,
                cat: Catcode::Letter | Catcode::Other,
            }) => self.state.delcode(ch),
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter)) => {
                return self.scan_delimiter_number();
            }
            _ => -1,
        };
        if code < 0 {
            // TeX82 §1161: "Missing delimiter (. inserted)" reports through
            // `back_error`, which returns the offending token to the input
            // before the error and leaves the null delimiter behind.
            let site = self.capture_diagnostic_site(Some(&command));
            self.back_input(command)?;
            self.missing_delimiter_error(Some(site))?;
            return Ok(ScannedMathDelimiter {
                code: 0,
                recovered: true,
                missing_delimiter: true,
                provenance: StructuredProvenance { primary },
            });
        }
        Ok(ScannedMathDelimiter {
            code: code as u32,
            recovered: false,
            missing_delimiter: false,
            provenance: StructuredProvenance { primary },
        })
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

    /// Scans TeX82 §435's `scan_four_bit_int` family index, the prefix common
    /// to the three math-font assignment primitives (§1234's `def_family`).
    /// The later font-meaning scan is intentionally not part of this request.
    pub fn scan_math_family(
        &mut self,
        size: MathFamilySize,
    ) -> Result<ScannedMathFamily, CommandError> {
        let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::FourBit);
        let scanned = result.into_result()?;
        Ok(ScannedMathFamily {
            size,
            family: scanned.value as u8,
            recovered: scanned.recovered,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    pub fn scan_math_family_retained(
        &mut self,
        size: MathFamilySize,
    ) -> crate::RetainedScalarScan<ScannedMathFamily> {
        let result = self.scan_math_family(size);
        self.detach_retained_scalar(result)
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
                let result = self.scan_dimension_retained();
                Some(result.into_result()?.value)
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
            let result = self.scan_glue_retained(true);
            Ok(ScannedMathMuMaterial::Glue(result.into_result()?.value))
        } else {
            let result = self.scan_mu_dimension_retained();
            Ok(ScannedMathMuMaterial::Kern(result.into_result()?.value))
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
                recovered: false,
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
            UnexpandablePrimitive::DisplayStyle => Request::Style(MathStyleKind::Display),
            UnexpandablePrimitive::TextStyle => Request::Style(MathStyleKind::Text),
            UnexpandablePrimitive::ScriptStyle => Request::Style(MathStyleKind::Script),
            UnexpandablePrimitive::ScriptScriptStyle => Request::Style(MathStyleKind::ScriptScript),
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
