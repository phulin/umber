use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    /// Scans TeX82 §1241's complete `\setbox` assignment operand.
    ///
    /// TeX.web's `prefixed_command` dispatches `set_box` to §433's
    /// `scan_eight_bit_int` then `scan_optional_equals`, followed immediately
    /// by the `set_box_allowed` test. Its false branch reports directly,
    /// without fetching a box command; its true branch enters §1084's
    /// `scan_box`. None of the scanned operand returns to main control.
    /// e-TeX 2.6 [49.1241] widens only the target to `scan_register_num` while
    /// retaining the same complete operand ownership and backup transitions.
    pub fn scan_setbox_assignment(
        &mut self,
        set_box_allowed: bool,
    ) -> Result<ScannedSetBoxAssignment, CommandError> {
        let index = self.scan_profile_register_index_retained().into_result()?;
        let result = self.scan_optional_equals_retained();
        result.into_result()?;
        let path = if set_box_allowed {
            ScannedSetBoxPath::Payload(self.scan_box_payload()?)
        } else {
            ScannedSetBoxPath::Forbidden {
                error_context: self.command.output_open_context(self.state),
            }
        };
        Ok(ScannedSetBoxAssignment { index, path })
    }

    /// Scans the register operand of TeX82 §1079's `make_box(box_code)` and
    /// e-TeX 2.6 [47.1079]'s sparse-array replacement.
    pub fn scan_box_register(&mut self) -> Result<ScannedBoxRegister, CommandError> {
        let result = self.scan_profile_register_index_retained();
        Ok(ScannedBoxRegister {
            index: result.into_result()?,
        })
    }

    /// Scans TeX82 §1082's `\\vsplit <number> to <dimen>` prefix.
    ///
    /// e-TeX 2.6 [47.1082] widens the source box selector from
    /// `scan_eight_bit_int` to `scan_register_num`.
    pub fn scan_vsplit(&mut self) -> Result<ScannedVSplit, CommandError> {
        let index = self.scan_profile_register_index_retained().into_result()?;
        let found = self.scan_keyword_retained("to").into_result()?;
        let missing_to_context =
            (!found.value).then(|| self.command.output_open_context(self.state));
        let height = self.scan_dimension_retained().into_result()?.value;
        let split_context = self.command.output_open_context(self.state);
        Ok(ScannedVSplit {
            index,
            height,
            missing_to_context,
            split_context,
        })
    }

    /// TeX82 §296's `print_meaning` as `\\show` reaches it.
    ///
    /// A macro's meaning is `print_cmd_chr`, then `print_char(":")`, then
    /// `print_ln`, then `token_show` of the body -- so `\\show\\cs` puts the
    /// replacement text on its own line. `\\meaning` and `\\showthe` share
    /// `print_meaning` but run it under §471's `new_string` selector, where
    /// §57's `print_ln` does nothing, which is why only this caller breaks
    /// the line.
    fn shown_meaning_text(
        state: &mut tex_state::CommandContext<'_, G>,
        command: &crate::CurrentCommand<G>,
    ) -> String {
        let text = selector_meaning_text(state, command);
        let breaks_after_colon = matches!(command.meaning(), ResolvedMeaning::Macro { .. })
            || matches!(
                static_meaning(command.meaning()),
                Some(Meaning::ExpandablePrimitive(
                    tex_state::meaning::ExpandablePrimitive::EndTemplate
                ))
            )
            || matches!(
                static_meaning(command.meaning()),
                Some(Meaning::ExpandablePrimitive(
                    tex_state::meaning::ExpandablePrimitive::TopMark
                        | tex_state::meaning::ExpandablePrimitive::FirstMark
                        | tex_state::meaning::ExpandablePrimitive::BotMark
                        | tex_state::meaning::ExpandablePrimitive::SplitFirstMark
                        | tex_state::meaning::ExpandablePrimitive::SplitBotMark
                ))
            );
        if breaks_after_colon {
            text.replacen(':', ":\n", 1)
        } else {
            text
        }
    }

    /// TeX82 §46's raw `\\show` operand scan.
    pub fn scan_show(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let mut destination = None;
        if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
            return Err(CommandError::input_invariant());
        }
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        let token = command.spelling().semantic_token();
        let content = match token {
            Token::Cs(_)
            | Token::Char {
                cat: Catcode::Active,
                ..
            } => {
                let raw = string_text(self.state, token);
                let mut shown = String::new();
                self.state.append_selector_string_text(&raw, &mut shown);
                format!(
                    "> {shown}={}",
                    Self::shown_meaning_text(self.state, &command)
                )
            }
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {
                format!("> {}", Self::shown_meaning_text(self.state, &command))
            }
        };
        Ok(ScannedDisplayDiagnostic {
            content,
            provenance: StructuredProvenance {
                primary: command.origin(),
            },
        })
    }

    /// TeX82 §46's `\\showthe` internal-value scan.
    pub fn scan_showthe(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let result = self.scan_internal_value_or_zero_retained();
        let value = result.into_result()?;
        let text = match value.value {
            value @ (InternalValue::Integer(_)
            | InternalValue::Dimension(_)
            | InternalValue::Glue(_)
            | InternalValue::MuGlue(_)) => {
                render_the_value(&value).expect("non-token values render")
            }
            // TeX82 §§262/1297: `the_toks` turns an `ident_val` into a
            // control-sequence token, then `token_show` uses `print_cs`.
            // Its control-word delimiter therefore precedes §1293's period.
            InternalValue::Font(symbol) => print_cs_text(self.state, symbol),
            InternalValue::Tokens { tokens, .. } => {
                let mut text = String::new();
                let words = self
                    .command
                    .attempt_token_words(tokens)
                    .map_err(crate::scan_toks::attempt_command_error)?
                    .to_vec();
                for token in words {
                    self.state
                        .append_token_selector_text(token.semantic_token(), &mut text);
                }
                text
            }
        };
        Ok(ScannedDisplayDiagnostic {
            content: format!("> {text}"),
            provenance: StructuredProvenance {
                primary: value.provenance.primary,
            },
        })
    }

    /// Scans e-TeX 2.6 `etex.ch` [17.3623--3660]'s unexpanded general text
    /// operand for `\\showtokens`.
    ///
    /// The compulsory braces are removed and the balanced interior is
    /// retained verbatim; expansion is never entered.
    pub fn scan_showtokens(&mut self) -> Result<ScannedBalancedText, CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let provenance = provenance(&scanned);
        Ok(ScannedBalancedText {
            tokens: scanned.replacement_text,
            provenance,
        })
    }

    /// e-TeX 2.6 `etex.ch` [49.1296]'s extended box-register scan for
    /// `\\showbox`.
    ///
    /// The change from TeX82's `scan_eight_bit_int` to `scan_register_num`
    /// retains the restricted scanner's invalid-to-zero recovery before the
    /// box lookup.
    pub fn scan_showbox(&mut self) -> Result<(u16, StructuredProvenance), CommandError> {
        let class = if self.command.profile().capabilities().supports_etex() {
            RestrictedIntegerClass::Register
        } else {
            RestrictedIntegerClass::EightBit
        };
        let result = self.scan_restricted_integer_retained(class);
        let index = result.into_result()?;
        Ok((
            u16::try_from(index.value).expect("recovered register number is in range"),
            StructuredProvenance {
                primary: index.provenance.primary,
            },
        ))
    }

    /// Scans the payload prefix of TeX82 §1090's leader commands.
    pub fn scan_leader_payload(&mut self) -> Result<ScannedLeaderPayload, CommandError> {
        let mut destination = None;
        match self.request_expanded_token(&mut destination)? {
            DeliveryStatus::End => return Ok(ScannedLeaderPayload::Missing),
            DeliveryStatus::Command => {}
            _ => return Err(CommandError::input_invariant()),
        };
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        match static_meaning(command.meaning()) {
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)) => {
                let result = self.scan_eight_bit_register_index_retained();
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: result.into_result()?,
                    copy: false,
                })
            }
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy)) => {
                let result = self.scan_eight_bit_register_index_retained();
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: result.into_result()?,
                    copy: true,
                })
            }
            Some(Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HBox
                | UnexpandablePrimitive::VBox
                | UnexpandablePrimitive::VTop),
            )) => Ok(ScannedLeaderPayload::Construction(
                self.scan_box_construction(primitive)?,
            )),
            Some(Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HRule | UnexpandablePrimitive::VRule),
            )) => Ok(ScannedLeaderPayload::Rule(self.scan_rule_spec(primitive)?)),
            _ => {
                self.back_input(command)?;
                Ok(ScannedLeaderPayload::Missing)
            }
        }
    }

    /// Scans TeX82's named glue-parameter assignment operand.
    ///
    /// This follows `scan_optional_equals` and `scan_glue`, retaining their
    /// canonical backup and alignment-delivery transitions before replay
    /// applies the aggregate mutation.
    pub fn scan_glue_parameter_assignment(
        &mut self,
        index: u16,
        mu: bool,
    ) -> Result<ScannedGlueParameterAssignment, CommandError> {
        let result = self.scan_optional_equals_retained();
        result.into_result()?;

        let result = self.scan_glue_retained(mu);
        let value = result.into_result()?.value;
        Ok(ScannedGlueParameterAssignment { index, value, mu })
    }

    /// Scans the complete expanded specification of a TeX82 rule.
    ///
    /// This is TeX.web's `scan_rule_spec`: keyword recognition and dimension
    /// scanning stay in command control, including failed-keyword replay.
    pub fn scan_rule_spec(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedRuleSpec, CommandError> {
        let default_rule = Scaled::from_raw(26_214);
        // TeX82 §463 starts every rule node with null dimensions, then gives
        // `\vrule` only its default width and `\hrule` only its default
        // height/depth. A null height/depth on a vertical rule is a running
        // dimension resolved by alignment or packing; materializing the
        // horizontal defaults here changes that geometry before those owners
        // can apply their canonical values.
        let (mut width, mut height, mut depth, mut phase) =
            if matches!(primitive, UnexpandablePrimitive::VRule) {
                (
                    Some(default_rule),
                    None,
                    None,
                    RuleScalarPhase::WidthKeyword,
                )
            } else {
                (
                    None,
                    Some(default_rule),
                    Some(Scaled::from_raw(0)),
                    RuleScalarPhase::WidthKeyword,
                )
            };
        loop {
            match phase {
                RuleScalarPhase::WidthKeyword => {
                    let result = self.scan_keyword_retained("width");
                    if result.into_result()?.value {
                        phase = RuleScalarPhase::WidthDimension;
                    } else {
                        phase = RuleScalarPhase::HeightKeyword;
                    }
                }
                RuleScalarPhase::WidthDimension => {
                    let result = self.scan_dimension_retained();
                    width = Some(result.into_result()?.value);
                    phase = RuleScalarPhase::WidthKeyword;
                }
                RuleScalarPhase::HeightKeyword => {
                    let result = self.scan_keyword_retained("height");
                    if result.into_result()?.value {
                        phase = RuleScalarPhase::HeightDimension;
                    } else {
                        phase = RuleScalarPhase::DepthKeyword;
                    }
                }
                RuleScalarPhase::HeightDimension => {
                    let result = self.scan_dimension_retained();
                    height = Some(result.into_result()?.value);
                    phase = RuleScalarPhase::WidthKeyword;
                }
                RuleScalarPhase::DepthKeyword => {
                    let result = self.scan_keyword_retained("depth");
                    if result.into_result()?.value {
                        phase = RuleScalarPhase::DepthDimension;
                    } else {
                        break;
                    }
                }
                RuleScalarPhase::DepthDimension => {
                    let result = self.scan_dimension_retained();
                    depth = Some(result.into_result()?.value);
                    phase = RuleScalarPhase::WidthKeyword;
                }
            }
        }
        Ok(ScannedRuleSpec {
            width,
            height,
            depth,
        })
    }

    /// Reads a box body's mandatory opening brace: TeX82 §403's
    /// `scan_left_brace`, which every box-opening site reaches through
    /// §645's `scan_spec` (`new_save_level(c); scan_left_brace`) or §1099's
    /// `begin_insert_or_adjust` (`new_save_level(insert_group);
    /// scan_left_brace`).
    ///
    /// §403 *consumes* that brace; the save level it belongs to was already
    /// opened by the caller, so nothing is delivered to main control on its
    /// behalf. The brace is therefore never backed up here: replay opens the
    /// group when it receives the construction, exactly as `new_save_level`
    /// runs before `scan_left_brace`.
    ///
    /// When the mandatory brace is absent, §403 recovers by backing up the
    /// offending command and behaving as though a `{` had been read.
    /// `scan_left_brace` has already performed that backup, so this returns
    /// on the same footing: the brace is accounted for either way.
    fn scan_box_group_opening(&mut self) -> Result<(), CommandError> {
        let _ = self.scan_left_brace(true)?;
        Ok(())
    }

    /// Scans the `to`/`spread` clause of TeX82 §645's `scan_spec`.
    ///
    /// §645 is the single routine every specification-taking group opener
    /// runs: `if scan_keyword("to") then spec_code:=exactly else if
    /// scan_keyword("spread") then spec_code:=additional else begin
    /// spec_code:=additional; cur_val:=0; goto found end; scan_normal_dimen`.
    /// An absent clause is `spread 0pt`, which packs at natural size.
    ///
    /// Both call sites in this crate -- §1083's box construction and §774's
    /// `init_align` -- must share it: `\halign to <dimen>{`, `\halign spread
    /// <dimen>{`, and `\hbox to <dimen>{` are the same scan, and a site that
    /// skipped straight to §403's mandatory left brace would reject the `t`
    /// of `to` as a missing brace.
    pub(super) fn scan_spec_packing(&mut self) -> Result<ScannedPackingSpec, CommandError> {
        let mut phase = PackingScalarPhase::ToKeyword;
        loop {
            match phase {
                PackingScalarPhase::ToKeyword => {
                    let result = self.scan_keyword_retained("to");
                    if result.into_result()?.value {
                        phase = PackingScalarPhase::Dimension { exactly: true };
                    } else {
                        phase = PackingScalarPhase::SpreadKeyword;
                    }
                }
                PackingScalarPhase::SpreadKeyword => {
                    let result = self.scan_keyword_retained("spread");
                    if result.into_result()?.value {
                        phase = PackingScalarPhase::Dimension { exactly: false };
                    } else {
                        return Ok(ScannedPackingSpec::Natural);
                    }
                }
                PackingScalarPhase::Dimension { exactly } => {
                    let result = self.scan_dimension_retained();
                    let value = result.into_result()?.value;
                    return Ok(if exactly {
                        ScannedPackingSpec::Exactly(value)
                    } else {
                        ScannedPackingSpec::Spread(value)
                    });
                }
            }
        }
    }

    /// Scans TeX82 §1083's complete box-construction prefix: §645's
    /// `scan_spec`, whose optional `to`/`spread` clause and mandatory left
    /// brace are both consumed before replay enters the box group.
    ///
    /// §1167's `mmode+vcenter` runs the identical prefix
    /// (`scan_spec(vcenter_group,false)`), so `\vcenter` is scanned here
    /// rather than as a math text field: its body is an internal vertical
    /// list, not an mlist, and only §1168's closing action distinguishes it
    /// from `\vbox`.
    pub fn scan_box_construction(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedBoxConstruction, CommandError> {
        let kind = match primitive {
            UnexpandablePrimitive::HBox => ScannedBoxKind::HBox,
            UnexpandablePrimitive::VBox => ScannedBoxKind::VBox,
            UnexpandablePrimitive::VTop => ScannedBoxKind::VTop,
            UnexpandablePrimitive::VCenter => ScannedBoxKind::VCenter,
            _ => return Err(CommandError::input_invariant()),
        };
        let packing = self.scan_spec_packing()?;
        self.scan_box_group_opening()?;
        Ok(ScannedBoxConstruction { kind, packing })
    }

    /// Scans TeX82 §1099's `begin_insert_or_adjust` prefix, the one routine
    /// both `\insert` and `\vadjust` enter: `if cur_cmd=vadjust then
    /// cur_val:=255 else scan_eight_bit_int`, then
    /// `new_save_level(insert_group); scan_left_brace`.
    ///
    /// `scan_eight_bit_int` owns its range clamp and queues §433's diagnostic
    /// for the executor's canonical error channel. The effective value is
    /// carried to replay; `\vadjust` skips the scan entirely, so its fixed
    /// 255 is never subject to that diagnostic.
    pub fn scan_insert_construction(
        &mut self,
        is_vadjust: bool,
    ) -> Result<ScannedInsertConstruction, CommandError> {
        let pre = if is_vadjust && self.command.profile().capabilities().supports_pdftex() {
            self.scan_keyword_retained("pre").into_result()?.value
        } else {
            false
        };
        let (class, reserved_class_context) = if is_vadjust {
            (255, None)
        } else {
            let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::EightBit);
            let class = result.into_result()?.value;
            let context = (class == 255).then(|| self.command.output_open_context(self.state));
            (class, context)
        };
        self.scan_box_group_opening()?;
        Ok(ScannedInsertConstruction {
            class,
            is_vadjust,
            pre,
            reserved_class_context,
        })
    }

    /// Scans TeX82 §1073's box-shift prefix (`\raise`, `\lower`, `\moveleft`,
    /// `\moveright`) once the caller has already validated `abs(mode)+cur_cmd`
    /// legality (tex.web's "Forbidden cases": `vmode+vmove`, `hmode+hmove`,
    /// and `mmode+hmove` never reach `scan_normal_dimen` at all).
    ///
    /// The main-control case reads: `t:=cur_chr; scan_normal_dimen; if t=0
    /// then scan_box(cur_val) else scan_box(-cur_val)`. `\lower`/`\moveright`
    /// have `chr_code=0` and keep the scanned dimension; `\raise`/`\moveleft`
    /// have `chr_code=1` and negate it. This is `box_context`, later stored
    /// verbatim as `shift_amount(cur_box)`.
    pub fn scan_box_shift(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedBoxShift, CommandError> {
        let result = self.scan_dimension_retained();
        let amount = result.into_result()?.value;
        let delta = match primitive {
            UnexpandablePrimitive::Lower | UnexpandablePrimitive::MoveRight => amount,
            UnexpandablePrimitive::Raise | UnexpandablePrimitive::MoveLeft => -amount,
            _ => return Err(CommandError::input_invariant()),
        };
        let payload = self.scan_box_payload()?;
        Ok(ScannedBoxShift { delta, payload })
    }

    /// Scans TeX82 §1084's `scan_box` operand for a box-shift prefix: `scan_box`
    /// begins with "the next non-blank non-relax" token (§1084's own
    /// `get_x_token` loop), then requires `cur_cmd=make_box`. Since `box_context`
    /// here is always a signed dimension (bounded by `max_dimen`), it can never
    /// reach `leader_flag`, so `scan_box`'s rule-spec branch never applies to a
    /// box-shift operand -- only `\hbox`/`\vbox`/`\vtop`, `\box`, `\copy`,
    /// `\lastbox`, and `\vsplit` are accepted, matching `scan_box_value`'s
    /// `make_box` family exactly. Anything else is `scan_box`'s "A <box> was
    /// supposed to be here" recovery: the rejected command is backed up
    /// (`back_error`) for ordinary replay, and replay alone reports the
    /// diagnostic since it needs a `Universe` sink.
    fn scan_box_payload(&mut self) -> Result<ScannedBoxShiftPayload, CommandError> {
        let mut destination = None;
        loop {
            match self.request_expanded_token(&mut destination)? {
                DeliveryStatus::End => return Ok(ScannedBoxShiftPayload::Missing),
                DeliveryStatus::Command => {}
                _ => return Err(CommandError::input_invariant()),
            };
            let command = destination.take().ok_or(CommandError::input_invariant())?;
            match static_meaning(command.meaning()) {
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
                | Some(Meaning::Relax) => continue,
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)) => {
                    return Ok(ScannedBoxShiftPayload::BoxRegister {
                        index: self.scan_box_register()?.index,
                        copy: false,
                    });
                }
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy)) => {
                    return Ok(ScannedBoxShiftPayload::BoxRegister {
                        index: self.scan_box_register()?.index,
                        copy: true,
                    });
                }
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox)) => {
                    return Ok(ScannedBoxShiftPayload::LastBox {
                        error_context: self.error_context(),
                    });
                }
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit)) => {
                    return Ok(ScannedBoxShiftPayload::VSplit(self.scan_vsplit()?));
                }
                Some(Meaning::UnexpandablePrimitive(
                    primitive @ (UnexpandablePrimitive::HBox
                    | UnexpandablePrimitive::VBox
                    | UnexpandablePrimitive::VTop),
                )) => {
                    return Ok(ScannedBoxShiftPayload::Construction(
                        self.scan_box_construction(primitive)?,
                    ));
                }
                _ => {
                    self.back_input(command)?;
                    return Ok(ScannedBoxShiftPayload::Missing);
                }
            }
        }
    }
}
