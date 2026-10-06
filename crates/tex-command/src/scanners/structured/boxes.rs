use super::*;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Scans TeX82 §1082's `\\vsplit <number> to <dimen>` prefix.
    ///
    /// e-TeX 2.6 [47.1082] widens the source box selector from
    /// `scan_eight_bit_int` to `scan_register_num`.
    pub fn scan_vsplit(&mut self) -> Result<ScannedVSplit, CommandError> {
        let index = self.scan_profile_register_index()?;
        let found = self.scan_keyword("to")?;
        let missing_to_context = (!found).then(|| self.command.output_open_context(self.state));
        let height = self.scan_dimension()?;
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
        let command = destination
            .take()
            .ok_or_else(|| CommandError::input_invariant())?;
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
        let result = self.scan_internal_value_or_zero();
        let value = result?;
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
                    .attempt_token_words(&tokens)
                    .map_err(crate::scan_toks::attempt_command_error)?;
                for token in words {
                    self.state
                        .append_token_selector_text(token.semantic_token(), &mut text);
                }
                self.command
                    .attempt
                    .discard_token_list(tokens)
                    .map_err(crate::scan_toks::attempt_command_error)?;
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
                    let result = self.scan_keyword("width");
                    if result? {
                        phase = RuleScalarPhase::WidthDimension;
                    } else {
                        phase = RuleScalarPhase::HeightKeyword;
                    }
                }
                RuleScalarPhase::WidthDimension => {
                    let result = self.scan_dimension();
                    width = Some(result?);
                    phase = RuleScalarPhase::WidthKeyword;
                }
                RuleScalarPhase::HeightKeyword => {
                    let result = self.scan_keyword("height");
                    if result? {
                        phase = RuleScalarPhase::HeightDimension;
                    } else {
                        phase = RuleScalarPhase::DepthKeyword;
                    }
                }
                RuleScalarPhase::HeightDimension => {
                    let result = self.scan_dimension();
                    height = Some(result?);
                    phase = RuleScalarPhase::WidthKeyword;
                }
                RuleScalarPhase::DepthKeyword => {
                    let result = self.scan_keyword("depth");
                    if result? {
                        phase = RuleScalarPhase::DepthDimension;
                    } else {
                        break;
                    }
                }
                RuleScalarPhase::DepthDimension => {
                    let result = self.scan_dimension();
                    depth = Some(result?);
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

    /// Scans the `to`/`spread` clause of TeX82 §645's `scan_spec`.
    ///
    /// §645 is the single routine every specification-taking group opener
    /// runs: `if scan_keyword("to") then spec_code:=exactly else if
    /// scan_keyword("spread") then spec_code:=additional else begin
    /// spec_code:=additional; cur_val:=0; goto found end; scan_normal_dimen`.
    /// An absent clause is `spread 0pt`, which packs at natural size.
    ///
    /// Alignment's §774 caller keeps its existing group protocol. Box entry
    /// consumes its first word directly in `scan_box_spec` below, sharing
    /// §407's keyword matcher for a real packing clause.
    pub(super) fn scan_spec_packing(&mut self) -> Result<ScannedPackingSpec, CommandError> {
        let exactly = if self.scan_keyword("to")? {
            true
        } else if self.scan_keyword("spread")? {
            false
        } else {
            return Ok(ScannedPackingSpec::Natural);
        };
        let value = self.scan_dimension()?;
        Ok(if exactly {
            ScannedPackingSpec::Exactly(value)
        } else {
            ScannedPackingSpec::Spread(value)
        })
    }

    /// TeX82 §§403/645/1083: packing expands outside the new group,
    /// mandatory-brace expansion inside it, before the executor enters its mode.
    pub fn scan_box_spec(
        &mut self,
        group: tex_state::GroupKind,
    ) -> Result<ScannedPackingSpec, CommandError> {
        let mut packing = None;
        self.consume_expanded_words(&mut crate::terminal_consumer::<Self, G>(
            &mut |processor, spelling, meaning, _code, control_sequence, exceptional| {
                let character = meaning.character();
                let keyword = if control_sequence.is_none() {
                    match character.map(|(ch, _)| ch.to_ascii_lowercase()) {
                        Some('t') => Some("to"),
                        Some('s') => Some("spread"),
                        _ => None,
                    }
                } else {
                    None
                };
                if keyword.is_none() && matches!(character, Some((_, Catcode::Space))) {
                    return Ok(std::ops::ControlFlow::Continue(()));
                }
                let value = if let Some(keyword) = keyword {
                    if processor.scan_keyword_tail(keyword, spelling)? {
                        let size = processor.scan_dimension()?;
                        if keyword == "to" {
                            ScannedPackingSpec::Exactly(size)
                        } else {
                            ScannedPackingSpec::Spread(size)
                        }
                    } else {
                        ScannedPackingSpec::Natural
                    }
                } else {
                    ScannedPackingSpec::Natural
                };
                processor
                    .command
                    .begin_group(processor.state, processor.diagnostic_effects, group);
                if keyword.is_some()
                    || processor
                        .consume_left_brace(spelling, meaning, exceptional)?
                        .is_none()
                {
                    processor.scan_left_brace()?;
                }
                packing = Some(value);
                Ok(std::ops::ControlFlow::Break(()))
            },
        ))?;
        packing.ok_or_else(CommandError::input_invariant)
    }
}
