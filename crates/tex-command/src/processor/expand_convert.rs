//! TeX and e-TeX conversion and mark primitives.

use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::token::{OriginId, Token};

use crate::command::{CommandClass, HotCommand};
use crate::input::{PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, TokenBehavior};
use crate::observation::{CommandObservation, InputReason, InputRecord, InputTransition};
use crate::{CommandError, CurrentCommand};

use super::expand_render::{
    append_scaled_without_unit, format_scaled, meaning_text, page_mark, render_the_value,
    roman_numeral, string_text,
};
use super::{CommandProcessor, DeliveryStatus};

impl<G> CommandProcessor<'_, '_, G> {
    /// Advances the compact register-index phase of `\the`.  Register
    /// selectors are the common internal-value form that used to re-enter
    /// `get_x_token` from `scan_something_internal`; keeping their decimal
    /// index in the same control record makes chains such as
    /// `\the\count\the\count0` stackless as well.
    pub(super) fn advance_the_index_continuation(
        &mut self,
        command: HotCommand<G>,
    ) -> Result<bool, CommandError> {
        use crate::expansion_work::control::ThePhase;

        let control = self
            .command
            .scratch
            .top_the_control()
            .map_err(crate::scan_toks::scratch_command_error)?
            .ok_or_else(CommandError::input_invariant)?;
        let ThePhase::Index {
            target,
            negative,
            value,
            seen_digit,
        } = control.phase
        else {
            return Err(CommandError::input_invariant());
        };
        let character = command.character_token();
        let is_space = command.character_catcode() == Some(tex_state::token::Catcode::Space);
        let digit = character
            .filter(|character| character.is_ascii_digit())
            .map(|character| i64::from(character as u8 - b'0'));
        let accumulate = |value: i64, digit: i64| {
            value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .unwrap_or(i64::from(i32::MAX))
                .min(i64::from(i32::MAX))
        };
        let finish = |this: &mut Self, value: i64, negative: bool| -> Result<(), CommandError> {
            let value = value.min(i64::from(i32::MAX));
            let value = if negative {
                value.saturating_neg()
            } else {
                value
            };
            let value = i32::try_from(value).unwrap_or_else(|_| {
                if value.is_negative() {
                    i32::MIN
                } else {
                    i32::MAX
                }
            });
            let limit = if this.command.profile().capabilities().supports_etex() {
                32_767
            } else {
                i32::from(u8::MAX)
            };
            let index = u16::try_from(value.clamp(0, limit)).unwrap_or(0);
            let opener = this
                .command
                .scratch
                .pop_the_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            let value = this.scan_the_register_value(target, index)?;
            this.expand_the_value(opener, value)
        };

        match character {
            _ if is_space && !seen_digit => {
                self.command.scratch.set_the_phase(ThePhase::Index {
                    target,
                    negative,
                    value,
                    seen_digit,
                })?;
                Ok(false)
            }
            Some('+') | Some('-') if !seen_digit => {
                self.command.scratch.set_the_phase(ThePhase::Index {
                    target,
                    negative: character == Some('-'),
                    value,
                    seen_digit,
                })?;
                Ok(false)
            }
            Some(_) if digit.is_some() => {
                self.command.scratch.set_the_phase(ThePhase::Index {
                    target,
                    negative,
                    value: accumulate(value, digit.expect("digit matched")),
                    seen_digit: true,
                })?;
                Ok(false)
            }
            _ if !seen_digit => {
                self.back_input(command.materialize())?;
                self.missing_number_error()?;
                finish(self, 0, false)?;
                Ok(true)
            }
            _ => {
                if !is_space {
                    self.back_input(command.materialize())?;
                }
                finish(self, value, negative)?;
                Ok(true)
            }
        }
    }

    pub(super) fn compact_the_register_target(meaning: Meaning) -> bool {
        matches!(
            meaning,
            Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Count
                    | UnexpandablePrimitive::Dimen
                    | UnexpandablePrimitive::Skip
                    | UnexpandablePrimitive::Muskip
                    | UnexpandablePrimitive::Toks
            )
        )
    }

    /// Starts the compact `\fontname` operand protocol.  The opener is kept
    /// as an origin in the generation-owned control lane, so a chain of font
    /// name conversions never nests a scanner call frame.
    pub(super) fn begin_fontname_continuation(
        &mut self,
        opener: OriginId,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_fontname_control(opener)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Completes one compact `\fontname` operand from the command currently
    /// owned by the expanded-delivery loop.  Only a valid font selector is
    /// rendered directly; the invalid-selector branch materializes once at
    /// the diagnostic/backup boundary, exactly as §577's `back_error` does.
    pub(super) fn complete_fontname_continuation(
        &mut self,
        target: HotCommand<G>,
    ) -> Result<(), CommandError> {
        let control = self
            .command
            .scratch
            .pop_fontname_control()
            .map_err(crate::scan_toks::scratch_command_error)?;
        let font = match target.command_word().class() {
            CommandClass::Font => target.font_id(),
            CommandClass::Unexpandable
                if target.command_word().unexpandable_primitive()
                    == Some(tex_state::meaning::UnexpandablePrimitive::Font) =>
            {
                Some(self.state.current_font())
            }
            _ => None,
        };
        let Some(font) = font else {
            let command = target.materialize();
            let deferred = {
                let mut report = self.state.print_err("Missing font identifier");
                report.help(&[
                    "I was looking for a control sequence whose",
                    "current meaning has been defined by \\font.",
                ]);
                report.defer()
            };
            self.back_input(command)?;
            let context = self.command.output_open_context(self.state);
            let mut report = self.state.resume_error_report(deferred);
            report.context(context);
            let outcome = report.error();
            self.finish_error_outcome(outcome)?;
            self.push_font_name(tex_state::font::NULL_FONT, control.opener)?;
            return Ok(());
        };
        self.push_font_name(font, control.opener)
    }

    fn push_font_name(
        &mut self,
        font: tex_state::ids::FontId,
        opener: OriginId,
    ) -> Result<(), CommandError> {
        let mut name = self.state.font_name(font);
        let size = self.state.font_size(font);
        if size != self.state.font_design_size(font) {
            name.push_str(" at ");
            append_scaled_without_unit(size, &mut name);
            name.push_str("pt");
        }
        self.push_rendered_text(&name, opener);
        Ok(())
    }

    /// Starts the compact literal operand path for `\number` and
    /// `\romannumeral`.  The opener survives only as provenance in the
    /// generation-owned control lane.
    pub(super) fn begin_number_continuation(
        &mut self,
        opener: OriginId,
        roman: bool,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_number_control(opener, roman)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Consumes one settled character of a hot number conversion.  A nested
    /// expandable operand never enters another delivery function: it returns
    /// as the next command to this compact accumulator.
    pub(super) fn advance_number_continuation(
        &mut self,
        command: HotCommand<G>,
    ) -> Result<bool, CommandError> {
        use crate::expansion_work::control::SynchronousNumberPhase as Phase;
        let control = self
            .command
            .scratch
            .top_number_control()
            .map_err(crate::scan_toks::scratch_command_error)?
            .ok_or_else(CommandError::input_invariant)?;
        let character = command.character_token();
        let is_space = command.character_catcode() == Some(tex_state::token::Catcode::Space);
        let digit = character
            .filter(|ch| ch.is_ascii_digit())
            .map(|ch| i64::from(ch as u8 - b'0'));
        let saturating_digit = |value: i64, digit: i64| {
            value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .unwrap_or(i64::from(i32::MAX))
                .min(i64::from(i32::MAX))
        };
        let finish = |this: &mut Self,
                      control: crate::expansion_work::control::SynchronousNumberControl,
                      value: i64,
                      negative: bool|
         -> Result<(), CommandError> {
            let value = value.min(i64::from(i32::MAX));
            let value = if negative { -value } else { value };
            let value = i32::try_from(value).unwrap_or_else(|_| {
                if value.is_negative() {
                    i32::MIN
                } else {
                    i32::MAX
                }
            });
            let _ = this
                .command
                .scratch
                .pop_number_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            let text = if control.roman {
                roman_numeral(value)
            } else {
                value.to_string()
            };
            this.push_rendered_text(&text, control.opener);
            Ok(())
        };
        match control.phase {
            Phase::Need => {
                if is_space {
                    return Ok(false);
                }
                if character == Some('+') || character == Some('-') {
                    self.command.scratch.set_number_phase(Phase::Accumulating {
                        negative: character == Some('-'),
                        value: 0,
                        seen_digit: false,
                    })?;
                    return Ok(false);
                }
                if let Some(digit) = digit {
                    self.command.scratch.set_number_phase(Phase::Accumulating {
                        negative: false,
                        value: digit,
                        seen_digit: true,
                    })?;
                    return Ok(false);
                }
                self.back_input(command.materialize())?;
                self.missing_number_error()?;
                finish(self, control, 0, false)?;
                Ok(true)
            }
            Phase::Await { .. } => Err(CommandError::input_invariant()),
            Phase::Accumulating {
                negative,
                value,
                seen_digit,
            } => {
                if let Some(digit) = digit {
                    self.command.scratch.set_number_phase(Phase::Accumulating {
                        negative,
                        value: saturating_digit(value, digit),
                        seen_digit: true,
                    })?;
                    return Ok(false);
                }
                if !seen_digit {
                    self.back_input(command.materialize())?;
                    self.missing_number_error()?;
                    finish(self, control, 0, negative)?;
                } else {
                    if !is_space {
                        self.back_input(command.materialize())?;
                    }
                    finish(self, control, value, negative)?;
                }
                Ok(true)
            }
        }
    }

    /// Starts the iterative `\the` operand request.  The opener is reduced to
    /// its packed origin before the request enters the shared expansion-work
    /// control lane; no rich command is retained while the operand expands.
    pub(super) fn begin_the_continuation(&mut self, opener: OriginId) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_the_control(opener)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Completes one iterative `\the` operand after the expanded loop has
    /// settled its target command.  This entry point deliberately accepts the
    /// already-delivered command, so it never calls `get_x_token_into` and
    /// cannot recursively re-enter expanded delivery.
    pub(super) fn complete_the_continuation(
        &mut self,
        target: &CurrentCommand<G>,
        opener: OriginId,
    ) -> Result<(), CommandError> {
        let scanned = self.scan_internal_value_or_zero_from_target(target)?;
        self.expand_the_value(opener, scanned.value)
    }

    /// e-TeX 2.6 etex.ch §53a's `\detokenize`.
    ///
    /// `scan_general_text` collects without expansion, `token_show` renders
    /// the frozen spelling exactly as for `\scantokens`, and `str_toks`
    /// projects the resulting string to category-10 spaces and category-12
    /// other characters.
    pub(super) fn expand_detokenize(
        &mut self,
        opener: &CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let text = self.attempt_token_list_string_text(scanned.replacement_text)?;
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    /// pdftex.web §§495 and 1535's `\expanded` conversion.
    ///
    /// `scan_pdf_ext_toks` is exactly `scan_toks(false, true)`: it expands one
    /// balanced general-text argument and returns the resulting token list via
    /// `ins_list`. The inserted list therefore reenters the caller's current
    /// expansion loop instead of being rendered to characters.
    pub(super) fn expand_expanded(&mut self) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let replacement = scanned.replacement_text;
        let words = self
            .command
            .attempt
            .arena()
            .token_words(replacement)
            .map_err(crate::scan_toks::attempt_command_error)?
            .to_vec();
        let first = words.first().map(|word| word.semantic_token());
        self.insert_expansion_list(PackedTokenSpanHandle::transient(words), first);
        Ok(())
    }

    /// `\\string` observes spelling, never an effective control-sequence meaning.
    pub(super) fn expand_string(&mut self, opener: &CurrentCommand<G>) -> Result<(), CommandError> {
        let mut destination = None;
        match self.get_token_with_normal_scanner_status_into(&mut destination)? {
            DeliveryStatus::End => return Err(CommandError::input_invariant()),
            DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let target = destination
            .take()
            .expect("command status initializes destination");
        self.push_rendered_text(
            &string_text(self.state, target.spelling().semantic_token()),
            opener.origin(),
        );
        Ok(())
    }

    pub(super) fn expand_meaning(
        &mut self,
        opener: &CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let mut destination = None;
        match self.get_token_with_normal_scanner_status_into(&mut destination)? {
            DeliveryStatus::End => return Err(CommandError::input_invariant()),
            DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let target = destination
            .take()
            .expect("command status initializes destination");
        let text = meaning_text(self.state, &target);
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    pub(super) fn expand_number(
        &mut self,
        opener: &CurrentCommand<G>,
        roman: bool,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::Number {
                roman: retained_roman,
            } if retained_roman == roman => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let value = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::Number { roman },
                suspended,
            )?
            .value;
        let text = if roman {
            roman_numeral(value)
        } else {
            value.to_string()
        };
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    /// Installs one TeX82 §467 `ins_the_toks` result.
    ///
    /// §465's `the_toks` produces a token list for every `cur_val_level`: the
    /// scalar levels through `@<Convert |cur_val| to a token list@>`, `ident_val`
    /// as the font's own control-sequence token, and `tok_val` as a copy of the
    /// register or parameter. §467 then hands _all_ of them to the same
    /// `ins_list`, so none of the three may install a differently classified
    /// input level.
    pub(crate) fn expand_the_value(
        &mut self,
        opener: OriginId,
        value: crate::InternalValue,
    ) -> Result<(), CommandError> {
        if let Some(text) = render_the_value(&value) {
            self.push_rendered_text(&text, opener);
        } else {
            match value {
                // §466 copies the register's list rather than sharing its
                // durable source. The operation-local copy remains in the
                // attempt until this inserted level has copied its words.
                crate::InternalValue::Tokens { tokens } => {
                    let words = self
                        .command
                        .attempt
                        .arena()
                        .token_words(tokens)
                        .map_err(crate::scan_toks::attempt_command_error)?
                        .to_vec();
                    let first = words.first().map(|word| word.semantic_token());
                    self.insert_expansion_list(PackedTokenSpanHandle::transient(words), first);
                }
                crate::InternalValue::Font(symbol) => {
                    self.push_rendered_tokens([Token::Cs(symbol)], opener);
                }
                _ => unreachable!("non-token internal values are rendered above"),
            }
        }
        Ok(())
    }

    /// TeX82 §471's `font_name_code: scan_font_ident` and §472's
    /// `print(font_name[cur_val])`.
    ///
    /// `\fontname` owns no operand reading of its own: §577's
    /// `scan_font_ident` is the only routine that turns a command into a
    /// font, including its invalid-identifier recovery to `nullfont`.
    pub(super) fn expand_fontname(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::FontName
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_font_selector_retained();
        let font = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::FontName,
            suspended,
        )?;
        let mut name = self.state.font_name(font);
        let size = self.state.font_size(font);
        if size != self.state.font_design_size(font) {
            // TeX82 §472 appends `at <size>pt` whenever the selected size
            // differs from the TFM design size. This text is inserted as
            // catcode-12/space tokens by `str_toks`, so it must be complete
            // before an enclosing `\edef` captures it.
            name.push_str(" at ");
            append_scaled_without_unit(size, &mut name);
            name.push_str("pt");
        }
        self.push_rendered_text(&name, opener.origin());
        Ok(())
    }

    pub(super) fn expand_pdf_font_size(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::PdfFontSize
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_font_selector_retained();
        let font = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::PdfFontSize,
            suspended,
        )?;
        let size = format_scaled(self.state.tracked_font_size(font));
        self.push_rendered_text(&size, opener.origin());
        Ok(())
    }

    pub(super) fn expand_margin_kern(
        &mut self,
        opener: CurrentCommand<G>,
        primitive: ExpandablePrimitive,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::PdfMarginKern {
                primitive: retained,
            } if retained == primitive => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_extended_register_index_retained();
        let index = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::PdfMarginKern { primitive },
            suspended,
        )?;
        let side = match primitive {
            ExpandablePrimitive::LeftMarginKern => tex_state::node::MarginKernSide::Left,
            ExpandablePrimitive::RightMarginKern => tex_state::node::MarginKernSide::Right,
            _ => return Err(CommandError::input_invariant()),
        };
        let Some(amount) = self.state.box_margin_kern(index, side) else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (marginkern): a non-empty hbox expected",
            ));
        };
        self.push_rendered_text(&format_scaled(amount), opener.origin());
        Ok(())
    }

    pub(super) fn expand_mark(
        &mut self,
        primitive: ExpandablePrimitive,
    ) -> Result<(), CommandError> {
        if let Some(tokens) = self.state.page_mark_value(page_mark(primitive)).cloned() {
            self.push_mark_text(&tokens);
        }
        Ok(())
    }

    pub(super) fn expand_mark_class(
        &mut self,
        primitive: ExpandablePrimitive,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        // e-TeX 2.6 `etex.ch` [26.1178] uses the same
        // `scan_register_num` as numbered marks and sparse registers.
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::MarkClass {
                primitive: retained,
            } if retained == primitive => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_extended_register_index_retained();
        let class = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::MarkClass { primitive },
            suspended,
        )?;
        // e-TeX 2.6 etex.ch [25.386] makes class zero an exact alias for
        // TeX82's `cur_mark`, including its null-versus-empty pointer state.
        let tokens = self
            .state
            .page_mark_class_value(page_mark(primitive), class);
        if let Some(tokens) = tokens.copied() {
            self.push_mark_text(&tokens);
        }
        Ok(())
    }

    fn push_mark_text(&mut self, tokens: &tex_state::node::NodeTokenList) {
        let words = self
            .state
            .node_token_words(*tokens)
            .expect("page mark token key belongs to the admitted generation");
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::stored_semantic(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(crate::input::StoredReplayReason::Mark),
        );
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::Mark,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }),
        );
    }
}
