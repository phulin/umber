//! TeX and e-TeX conversion and mark primitives.

use tex_state::meaning::{ExpandablePrimitive, Meaning, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::scaled::{PhysicalUnit, scaled_from_decimal_parts};
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
    /// Advances the compact integer-expression form used by `\the`.  The
    /// ordinary expression scanner already has an explicit parenthesis stack,
    /// but its scalar factors still request expanded tokens synchronously.
    /// This small no-parenthesis lane covers the common nested conversion
    /// path and keeps each factor in the generation-owned control record.
    pub(super) fn advance_the_expression_continuation(
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
        let ThePhase::Expression {
            target,
            expression,
            expression_sign,
            term,
            term_operator,
            term_active,
            negative,
            value,
            seen_digit,
        } = control.phase
        else {
            return Err(CommandError::input_invariant());
        };
        if target != Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NumExpr) {
            return Err(CommandError::input_invariant());
        }
        let character = command.character_value();
        let is_space = command.character_catcode() == Some(tex_state::token::Catcode::Space);
        let digit = character
            .filter(|character| character.is_ascii_digit())
            .map(|character| i64::from(character as u8 - b'0'));
        let is_relax = matches!(
            command.resolved_meaning(),
            ResolvedMeaning::Static(Meaning::Relax)
        );
        let accumulate = |value: i64, digit: i64| {
            value
                .saturating_mul(10)
                .saturating_add(digit)
                .min(i64::from(i32::MAX))
        };
        let apply_term = |term: i64, operator: u8, factor: i64| match operator {
            1 => term.saturating_mul(factor),
            2 if factor != 0 => term / factor,
            2 => 0,
            _ => factor,
        };
        let finish = |this: &mut Self, expression: i64, term: i64| {
            let result = expression.saturating_add(i64::from(expression_sign).saturating_mul(term));
            let result = result.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
            let result = i32::try_from(result).expect("clamped expression fits i32");
            let opener = this
                .command
                .scratch
                .pop_the_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            this.expand_the_value(opener, crate::InternalValue::Integer(result))
        };
        let reset_factor = |this: &mut Self,
                            expression,
                            expression_sign,
                            term,
                            term_operator,
                            term_active|
         -> Result<(), CommandError> {
            this.command
                .scratch
                .set_the_phase(ThePhase::Expression {
                    target,
                    expression,
                    expression_sign,
                    term,
                    term_operator,
                    term_active,
                    negative: false,
                    value: 0,
                    seen_digit: false,
                })
                .map_err(crate::scan_toks::scratch_command_error)
        };

        if is_space && !seen_digit {
            return Ok(false);
        }
        if (character == Some('+') || character == Some('-')) && !seen_digit {
            self.command.scratch.set_the_phase(ThePhase::Expression {
                target,
                expression,
                expression_sign,
                term,
                term_operator,
                term_active,
                negative: character == Some('-'),
                value,
                seen_digit,
            })?;
            return Ok(false);
        }
        if let Some(digit) = digit {
            self.command.scratch.set_the_phase(ThePhase::Expression {
                target,
                expression,
                expression_sign,
                term,
                term_operator,
                term_active,
                negative,
                value: accumulate(value, digit),
                seen_digit: true,
            })?;
            return Ok(false);
        }

        if !seen_digit {
            if !is_relax {
                self.back_input(command.materialize())?;
            }
            self.missing_number_error()?;
            finish(self, expression, if term_active { term } else { 0 })?;
            return Ok(true);
        }
        let factor = if negative {
            value.saturating_neg()
        } else {
            value
        };
        let term = if term_active {
            apply_term(term, term_operator, factor)
        } else {
            factor
        };
        if character == Some('*') || character == Some('/') {
            reset_factor(
                self,
                expression,
                expression_sign,
                term,
                if character == Some('*') { 1 } else { 2 },
                true,
            )?;
            return Ok(false);
        }
        if character == Some('+') || character == Some('-') {
            let expression =
                expression.saturating_add(i64::from(expression_sign).saturating_mul(term));
            reset_factor(
                self,
                expression,
                if character == Some('+') { 1 } else { -1 },
                0,
                0,
                false,
            )?;
            return Ok(false);
        }
        if !is_relax && seen_digit {
            self.back_input(command.materialize())?;
        }
        finish(self, expression, term)?;
        Ok(true)
    }

    pub(super) fn compact_the_expression_target(meaning: Meaning) -> bool {
        meaning == Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NumExpr)
    }

    /// Advances the common literal `\the\dimexpr` form.  The full e-TeX
    /// expression scanner remains the semantic fallback for parenthesized,
    /// glue, and internal-unit expressions; this compact path handles the
    /// fixed-point `pt` stream used by nested conversion chains without
    /// retaining a scalar scanner call frame.
    pub(super) fn advance_the_dimension_expression_continuation(
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
        let ThePhase::DimensionExpression {
            target,
            as_number,
            expression,
            expression_sign,
            term,
            term_operator,
            term_active,
            negative,
            value,
            fraction,
            fraction_digits,
            decimal,
            unit,
            seen_digit,
        } = control.phase
        else {
            return Err(CommandError::input_invariant());
        };
        if target != Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DimExpr) {
            return Err(CommandError::input_invariant());
        }

        let character = command.character_value();
        let is_space = command.character_catcode() == Some(tex_state::token::Catcode::Space);
        let is_relax = matches!(
            command.resolved_meaning(),
            ResolvedMeaning::Static(Meaning::Relax)
        );
        let digit = character
            .filter(|character| character.is_ascii_digit())
            .map(|character| i32::from(character as u8 - b'0'));
        let accumulate = |value: i32, digit: i32| {
            value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .unwrap_or(i32::MAX)
        };
        let accumulate_fraction = |fraction: i32, digits: u8, digit: i32| {
            if digits >= 9 {
                (fraction, digits)
            } else {
                (
                    fraction.saturating_mul(10).saturating_add(digit),
                    digits + 1,
                )
            }
        };
        let factor = |value: i32, fraction: i32, digits: u8, decimal: bool, negative: bool| {
            let fraction = if decimal && digits != 0 {
                let mut denominator = 1_i64;
                for _ in 0..digits.min(9) {
                    denominator = denominator.saturating_mul(10);
                }
                let rounded = (i64::from(fraction)
                    .saturating_mul(i64::from(tex_state::scaled::Scaled::UNITY))
                    .saturating_add(denominator / 2))
                    / denominator;
                i32::try_from(rounded).unwrap_or(tex_state::scaled::Scaled::UNITY - 1)
            } else {
                0
            };
            let result = scaled_from_decimal_parts(value, fraction, PhysicalUnit::Pt)
                .map(|value| value.raw())
                .unwrap_or(i32::MAX);
            if negative {
                result.saturating_neg()
            } else {
                result
            }
        };
        let finish = |this: &mut Self,
                      expression: i32,
                      expression_sign: i8,
                      term: i32,
                      term_active: bool|
         -> Result<(), CommandError> {
            let result = expression.saturating_add(
                i32::from(expression_sign).saturating_mul(if term_active { term } else { 0 }),
            );
            let opener = this
                .command
                .scratch
                .pop_the_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if as_number {
                this.push_rendered_text(&result.to_string(), opener);
                Ok(())
            } else {
                this.expand_the_value(
                    opener,
                    crate::InternalValue::Dimension(tex_state::scaled::Scaled::from_raw(result)),
                )
            }
        };
        let reset = |this: &mut Self,
                     expression,
                     expression_sign,
                     term,
                     term_operator,
                     term_active|
         -> Result<(), CommandError> {
            this.command
                .scratch
                .set_the_phase(ThePhase::DimensionExpression {
                    target,
                    as_number,
                    expression,
                    expression_sign,
                    term,
                    term_operator,
                    term_active,
                    negative: false,
                    value: 0,
                    fraction: 0,
                    fraction_digits: 0,
                    decimal: false,
                    unit: 0,
                    seen_digit: false,
                })?;
            Ok(())
        };

        if unit == 0 {
            if is_space && !seen_digit {
                return Ok(false);
            }
            if (character == Some('+') || character == Some('-')) && !seen_digit {
                self.command
                    .scratch
                    .set_the_phase(ThePhase::DimensionExpression {
                        target,
                        as_number,
                        expression,
                        expression_sign,
                        term,
                        term_operator,
                        term_active,
                        negative: character == Some('-'),
                        value,
                        fraction,
                        fraction_digits,
                        decimal,
                        unit,
                        seen_digit,
                    })?;
                return Ok(false);
            }
            if let Some(digit) = digit {
                let value = if decimal {
                    value
                } else {
                    accumulate(value, digit)
                };
                let (fraction, fraction_digits) = if decimal {
                    accumulate_fraction(fraction, fraction_digits, digit)
                } else {
                    (fraction, fraction_digits)
                };
                self.command
                    .scratch
                    .set_the_phase(ThePhase::DimensionExpression {
                        target,
                        as_number,
                        expression,
                        expression_sign,
                        term,
                        term_operator,
                        term_active,
                        negative,
                        value,
                        fraction,
                        fraction_digits,
                        decimal,
                        unit,
                        seen_digit: true,
                    })?;
                return Ok(false);
            }
            if (character == Some('.') || character == Some(',')) && !decimal {
                self.command
                    .scratch
                    .set_the_phase(ThePhase::DimensionExpression {
                        target,
                        as_number,
                        expression,
                        expression_sign,
                        term,
                        term_operator,
                        term_active,
                        negative,
                        value,
                        fraction,
                        fraction_digits,
                        decimal: true,
                        unit,
                        seen_digit,
                    })?;
                return Ok(false);
            }
            if character == Some('p') && seen_digit {
                self.command
                    .scratch
                    .set_the_phase(ThePhase::DimensionExpression {
                        target,
                        as_number,
                        expression,
                        expression_sign,
                        term,
                        term_operator,
                        term_active,
                        negative,
                        value,
                        fraction,
                        fraction_digits,
                        decimal,
                        unit: 1,
                        seen_digit,
                    })?;
                return Ok(false);
            }
        } else if unit == 1 && character == Some('t') {
            self.command
                .scratch
                .set_the_phase(ThePhase::DimensionExpression {
                    target,
                    as_number,
                    expression,
                    expression_sign,
                    term,
                    term_operator,
                    term_active,
                    negative,
                    value,
                    fraction,
                    fraction_digits,
                    decimal,
                    unit: 2,
                    seen_digit,
                })?;
            return Ok(false);
        }

        if unit != 2 {
            if !is_relax {
                self.back_input(command.materialize())?;
            }
            self.missing_number_error()?;
            finish(self, expression, expression_sign, term, term_active)?;
            return Ok(true);
        }

        let current = factor(value, fraction, fraction_digits, decimal, negative);
        if is_space {
            // Spaces after a complete factor are scanner separators. Keep
            // the completed unit marker so the next operator can commit it.
            return Ok(false);
        }
        if character == Some('+') || character == Some('-') {
            let expression = expression.saturating_add(
                i32::from(expression_sign).saturating_mul(if term_active { term } else { current }),
            );
            reset(
                self,
                expression,
                if character == Some('+') { 1 } else { -1 },
                0,
                0,
                false,
            )?;
            return Ok(false);
        }
        if is_relax {
            finish(
                self,
                expression,
                expression_sign,
                if term_active { term } else { current },
                true,
            )?;
            return Ok(true);
        }
        self.back_input(command.materialize())?;
        finish(
            self,
            expression,
            expression_sign,
            if term_active { term } else { current },
            true,
        )?;
        Ok(true)
    }

    pub(super) fn compact_the_dimension_expression_target(meaning: Meaning) -> bool {
        meaning == Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DimExpr)
    }

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
        let character = command.character_value();
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
    pub(super) fn begin_fontname_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_fontname_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Starts the compact `\pdffontsize` operand protocol.  It shares the
    /// expanded font-selector request and differs only in the rendering step,
    /// so a nested selector follows the same generation-owned control lane.
    pub(super) fn begin_pdf_font_size_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_font_size_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_pdf_font_name_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_font_name_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_pdf_font_object_number_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_font_object_number_control_with_parent(opener, parent)
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
        match control.purpose {
            crate::expansion_work::control::SynchronousFontPurpose::Name => {
                self.push_font_name(font, control.opener)
            }
            crate::expansion_work::control::SynchronousFontPurpose::Size => {
                let size = format_scaled(self.state.tracked_font_size(font));
                self.push_rendered_text(&size, control.opener);
                Ok(())
            }
            crate::expansion_work::control::SynchronousFontPurpose::PdfName => {
                let name = self.state.font_name(font);
                self.push_rendered_text(&name, control.opener);
                Ok(())
            }
            crate::expansion_work::control::SynchronousFontPurpose::PdfObjectNumber => {
                let object = self
                    .state
                    .ensure_pdf_font_resource(font)
                    .map_err(|_| {
                        CommandError::PdfNavigation("pdfTeX error (font): too many PDF objects")
                    })?
                    .object_number();
                self.push_rendered_text(&object.to_string(), control.opener);
                Ok(())
            }
        }
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
    pub(super) fn begin_number_continuation_with_parent(
        &mut self,
        opener: OriginId,
        roman: bool,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_number_control_with_parent(opener, roman, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_pdf_uniform_deviate_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_uniform_deviate_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_pdf_margin_kern_continuation_with_parent(
        &mut self,
        opener: OriginId,
        side: tex_state::node::MarginKernSide,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_margin_kern_control_with_parent(opener, side, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_pdf_insert_height_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_insert_height_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_pdf_xform_name_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_xform_name_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_pdf_page_ref_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_page_ref_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_pdf_last_match_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_last_match_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn begin_mark_class_continuation_with_parent(
        &mut self,
        opener: OriginId,
        primitive: ExpandablePrimitive,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_mark_class_control_with_parent(opener, primitive, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    fn finish_number_output(
        &mut self,
        control: crate::expansion_work::control::SynchronousNumberControl,
        value: i32,
    ) -> Result<(), CommandError> {
        use crate::expansion_work::control::SynchronousNumberPurpose;

        let mark_primitive = match control.purpose {
            SynchronousNumberPurpose::TopMarkClass => Some(ExpandablePrimitive::TopMarks),
            SynchronousNumberPurpose::FirstMarkClass => Some(ExpandablePrimitive::FirstMarks),
            SynchronousNumberPurpose::BotMarkClass => Some(ExpandablePrimitive::BotMarks),
            SynchronousNumberPurpose::SplitFirstMarkClass => {
                Some(ExpandablePrimitive::SplitFirstMarks)
            }
            SynchronousNumberPurpose::SplitBotMarkClass => Some(ExpandablePrimitive::SplitBotMarks),
            _ => None,
        };
        if let Some(primitive) = mark_primitive {
            let class = u16::try_from(value.clamp(0, 32_767)).unwrap_or(0);
            if let Some(tokens) = self
                .state
                .page_mark_class_value(page_mark(primitive), class)
                .copied()
            {
                self.push_mark_text(&tokens);
            }
            return Ok(());
        }

        let text = match control.purpose {
            SynchronousNumberPurpose::Decimal => value.to_string(),
            SynchronousNumberPurpose::Roman => roman_numeral(value),
            SynchronousNumberPurpose::PdfUniformDeviate => {
                self.state.pdf_uniform_deviate(value).to_string()
            }
            SynchronousNumberPurpose::PdfInsertHeight => {
                let class = u16::try_from(value.clamp(0, 32_767)).unwrap_or(0);
                self.state
                    .page_insertion(class)
                    .map(|insertion| insertion.height())
                    .map_or_else(|| "0pt".to_owned(), format_scaled)
            }
            SynchronousNumberPurpose::PdfXFormName => u32::try_from(value)
                .ok()
                .and_then(|object| self.state.pdf_form_resource(object))
                .unwrap_or(0)
                .to_string(),
            SynchronousNumberPurpose::PdfPageRef => {
                if value <= 0 {
                    return Err(CommandError::PdfNavigation(
                        "pdfTeX error (pageref): invalid page number",
                    ));
                }
                u32::try_from(value)
                    .ok()
                    .and_then(|page| self.state.pdf_page_object(page))
                    .unwrap_or(0)
                    .to_string()
            }
            SynchronousNumberPurpose::PdfLastMatch => {
                let mut index = value;
                if index < 0 {
                    self.pdftex_match_number_diagnostic(index);
                    index = 1;
                }
                let mut rendered = String::new();
                if let Some((offset, bytes)) = u32::try_from(index)
                    .ok()
                    .and_then(|index| self.state.pdf_match_capture(index))
                {
                    use std::fmt::Write as _;
                    write!(rendered, "{offset}->").expect("writing to String cannot fail");
                    rendered.extend(bytes.iter().copied().map(char::from));
                } else {
                    rendered.push_str("-1->");
                }
                rendered
            }
            SynchronousNumberPurpose::PdfMarginKernLeft
            | SynchronousNumberPurpose::PdfMarginKernRight => {
                unreachable!("margin-kern controls use their scaled output path")
            }
            SynchronousNumberPurpose::TopMarkClass
            | SynchronousNumberPurpose::FirstMarkClass
            | SynchronousNumberPurpose::BotMarkClass
            | SynchronousNumberPurpose::SplitFirstMarkClass
            | SynchronousNumberPurpose::SplitBotMarkClass => {
                unreachable!("mark-class controls use their token-list output path")
            }
        };
        self.push_rendered_text(&text, control.opener);
        Ok(())
    }

    /// Consumes one settled character of a hot number conversion.  A nested
    /// expandable operand never enters another delivery function: it returns
    /// as the next command to this compact accumulator.
    pub(super) fn advance_number_continuation(
        &mut self,
        command: HotCommand<G>,
    ) -> Result<bool, CommandError> {
        self.advance_number_continuation_impl(command, false)
    }

    fn advance_number_continuation_impl(
        &mut self,
        command: HotCommand<G>,
        at_end: bool,
    ) -> Result<bool, CommandError> {
        use crate::expansion_work::control::SynchronousNumberPhase as Phase;
        let control = self
            .command
            .scratch
            .top_number_control()
            .map_err(crate::scan_toks::scratch_command_error)?
            .ok_or_else(CommandError::input_invariant)?;
        let character = command.character_value();
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

        // pdfTeX's margin-kern conversions use the e-TeX extended register
        // selector grammar.  Keep the selector in this existing compact
        // integer lane; the only operation-specific work is the final box
        // lookup and scaled rendering.
        if matches!(
            control.purpose,
            crate::expansion_work::control::SynchronousNumberPurpose::PdfMarginKernLeft
                | crate::expansion_work::control::SynchronousNumberPurpose::PdfMarginKernRight
        ) {
            let finish_margin_kern =
                |this: &mut Self,
                 control: crate::expansion_work::control::SynchronousNumberControl,
                 value: i64,
                 negative: bool|
                 -> Result<(), CommandError> {
                    let value = if negative {
                        value.saturating_neg()
                    } else {
                        value
                    };
                    let index = u16::try_from(value.clamp(0, 32_767)).unwrap_or(0);
                    let side = match control.purpose {
                    crate::expansion_work::control::SynchronousNumberPurpose::PdfMarginKernLeft => {
                        tex_state::node::MarginKernSide::Left
                    }
                    crate::expansion_work::control::SynchronousNumberPurpose::PdfMarginKernRight => {
                        tex_state::node::MarginKernSide::Right
                    }
                    _ => unreachable!("margin-kern branch validates its purpose"),
                };
                    let Some(amount) = this.state.box_margin_kern(index, side) else {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (marginkern): a non-empty hbox expected",
                        ));
                    };
                    let _ = this
                        .command
                        .scratch
                        .pop_number_control()
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    this.push_rendered_text(&format_scaled(amount), control.opener);
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
                    if !at_end {
                        self.back_input(command.materialize())?;
                    }
                    self.missing_number_error()?;
                    finish_margin_kern(self, *control, 0, false)?;
                    return Ok(true);
                }
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
                        if !at_end {
                            self.back_input(command.materialize())?;
                        }
                        self.missing_number_error()?;
                        finish_margin_kern(self, *control, 0, negative)?;
                    } else {
                        if !at_end && !is_space {
                            self.back_input(command.materialize())?;
                        }
                        finish_margin_kern(self, *control, value, negative)?;
                    }
                    return Ok(true);
                }
                Phase::Await { .. }
                | Phase::RegisterIndex { .. }
                | Phase::RegisterIndexAwait { .. } => {
                    return Err(CommandError::input_invariant());
                }
            }
        }

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
            this.finish_number_output(control, value)
        };

        // A register primitive is itself the first operand token of
        // `scan_int`; only its selector remains to be consumed.  Keep that
        // selector in the same compact number record instead of falling back
        // to `scan_something_internal`, whose legacy selector scanner would
        // re-enter expanded delivery for a macro-valued index.
        if matches!(control.phase, Phase::Need)
            && let ResolvedMeaning::Static(meaning) = command.resolved_meaning()
        {
            if control.purpose != crate::expansion_work::control::SynchronousNumberPurpose::Roman
                && meaning == Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NumExpr)
            {
                let (number, parent) = self
                    .command
                    .scratch
                    .pop_number_control_with_parent()
                    .map_err(crate::scan_toks::scratch_command_error)?;
                self.begin_the_continuation_with_parent(number.opener, parent)?;
                self.command.scratch.set_the_phase(
                    crate::expansion_work::control::ThePhase::Expression {
                        target: meaning,
                        expression: 0,
                        expression_sign: 1,
                        term: 0,
                        term_operator: 0,
                        term_active: false,
                        negative: false,
                        value: 0,
                        seen_digit: false,
                    },
                )?;
                return Ok(false);
            }
            if control.purpose != crate::expansion_work::control::SynchronousNumberPurpose::Roman
                && meaning == Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DimExpr)
            {
                let (number, parent) = self
                    .command
                    .scratch
                    .pop_number_control_with_parent()
                    .map_err(crate::scan_toks::scratch_command_error)?;
                self.begin_the_continuation_with_parent(number.opener, parent)?;
                self.command.scratch.set_the_phase(
                    crate::expansion_work::control::ThePhase::DimensionExpression {
                        target: meaning,
                        as_number: true,
                        expression: 0,
                        expression_sign: 1,
                        term: 0,
                        term_operator: 0,
                        term_active: false,
                        negative: false,
                        value: 0,
                        fraction: 0,
                        fraction_digits: 0,
                        decimal: false,
                        unit: 0,
                        seen_digit: false,
                    },
                )?;
                return Ok(false);
            }
            if Self::compact_number_register_target(meaning) {
                self.command
                    .scratch
                    .set_number_phase(Phase::RegisterIndex {
                        target: meaning,
                        negative: false,
                        value: 0,
                        seen_digit: false,
                    })?;
                return Ok(false);
            }
            if let Some(value) = self.scan_the_direct_value(meaning)?
                && let Some(value) = Self::number_from_internal_value(&value)
            {
                let _ = self
                    .command
                    .scratch
                    .pop_number_control()
                    .map_err(crate::scan_toks::scratch_command_error)?;
                self.finish_number_output(*control, value)?;
                return Ok(true);
            }
        }

        if let Phase::RegisterIndex {
            target,
            negative,
            value,
            seen_digit,
        } = control.phase
        {
            let finish_register =
                |this: &mut Self,
                 control: crate::expansion_work::control::SynchronousNumberControl,
                 target: Meaning,
                 value: i64,
                 negative: bool|
                 -> Result<(), CommandError> {
                    let value = if negative {
                        value.saturating_neg()
                    } else {
                        value
                    };
                    let limit = if this.command.profile().capabilities().supports_etex() {
                        32_767
                    } else {
                        i64::from(u8::MAX)
                    };
                    let index = u16::try_from(value.clamp(0, limit)).unwrap_or(0);
                    let internal = this.scan_the_register_value(target, index)?;
                    let number = Self::number_from_internal_value(&internal).unwrap_or(0);
                    let _ = this
                        .command
                        .scratch
                        .pop_number_control()
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    this.finish_number_output(control, number)
                };
            match character {
                _ if is_space && !seen_digit => {
                    self.command
                        .scratch
                        .set_number_phase(Phase::RegisterIndex {
                            target,
                            negative,
                            value,
                            seen_digit,
                        })?;
                    return Ok(false);
                }
                Some('+') | Some('-') if !seen_digit => {
                    self.command
                        .scratch
                        .set_number_phase(Phase::RegisterIndex {
                            target,
                            negative: character == Some('-'),
                            value,
                            seen_digit,
                        })?;
                    return Ok(false);
                }
                Some(digit) if digit.is_ascii_digit() => {
                    let value = value
                        .checked_mul(10)
                        .and_then(|value| value.checked_add(i64::from(digit as u8 - b'0')))
                        .unwrap_or(i64::from(i32::MAX))
                        .min(i64::from(i32::MAX));
                    self.command
                        .scratch
                        .set_number_phase(Phase::RegisterIndex {
                            target,
                            negative,
                            value,
                            seen_digit: true,
                        })?;
                    return Ok(false);
                }
                _ if !seen_digit => {
                    if !at_end {
                        self.back_input(command.materialize())?;
                    }
                    self.missing_number_error()?;
                    finish_register(self, *control, target, 0, false)?;
                    return Ok(true);
                }
                _ => {
                    if !at_end && !is_space {
                        self.back_input(command.materialize())?;
                    }
                    finish_register(self, *control, target, value, negative)?;
                    return Ok(true);
                }
            }
        }
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
                if !at_end {
                    self.back_input(command.materialize())?;
                }
                self.missing_number_error()?;
                finish(self, *control, 0, false)?;
                Ok(true)
            }
            Phase::Await { .. } => Err(CommandError::input_invariant()),
            Phase::RegisterIndex { .. } => Err(CommandError::input_invariant()),
            Phase::RegisterIndexAwait { .. } => Err(CommandError::input_invariant()),
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
                    if !at_end {
                        self.back_input(command.materialize())?;
                    }
                    self.missing_number_error()?;
                    finish(self, *control, 0, negative)?;
                } else {
                    if !at_end && !is_space {
                        self.back_input(command.materialize())?;
                    }
                    finish(self, *control, value, negative)?;
                }
                Ok(true)
            }
        }
    }

    /// Lets a pending integer conversion observe end-of-input as the
    /// scanner's implicit terminator.  TeX's integer scanner accepts a
    /// completed value immediately before EOF; the ordinary delivery loop
    /// otherwise has no settled command to hand to the hot accumulator.
    pub(super) fn finish_number_continuation_at_end(&mut self) -> Result<bool, CommandError> {
        let Some(control) = self
            .command
            .scratch
            .top_number_control()
            .map_err(crate::scan_toks::scratch_command_error)?
        else {
            return Ok(false);
        };
        if matches!(
            control.phase,
            crate::expansion_work::control::SynchronousNumberPhase::Await { .. }
                | crate::expansion_work::control::SynchronousNumberPhase::RegisterIndexAwait { .. }
        ) {
            return Ok(false);
        }
        self.advance_number_continuation_impl(HotCommand::empty(), true)
    }

    fn compact_number_register_target(meaning: Meaning) -> bool {
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

    fn number_from_internal_value(value: &crate::InternalValue) -> Option<i32> {
        match value {
            crate::InternalValue::Integer(value) => Some(*value),
            crate::InternalValue::Dimension(value) => Some(value.raw()),
            crate::InternalValue::Glue(value) | crate::InternalValue::MuGlue(value) => {
                Some(value.width.raw())
            }
            crate::InternalValue::Font(_) | crate::InternalValue::Tokens { .. } => None,
        }
    }

    /// Starts the iterative `\the` operand request.  The opener is reduced to
    /// its packed origin before the request enters the shared expansion-work
    /// control lane; no rich command is retained while the operand expands.
    pub(super) fn begin_the_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_the_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Starts the compact expanded-token collector used by e-TeX's
    /// `\expanded` conversion.  Its attempt buffer is admitted before the
    /// control, so a failed control push rolls back the complete local suffix
    /// without leaving a half-open collector behind.
    pub(super) fn begin_expanded_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        let attempt_opening = self.command.attempt.arena().mark();
        let writer = self
            .command
            .attempt
            .arena_mut()
            .allocate_token_buffer()
            .map_err(crate::scan_toks::attempt_command_error)?;
        if let Err(error) = self.command.scratch.push_expanded_control_with_parent(
            opener,
            attempt_opening,
            writer,
            parent,
        ) {
            self.command
                .attempt
                .arena_mut()
                .truncate(attempt_opening)
                .map_err(crate::scan_toks::attempt_command_error)?;
            return Err(crate::scan_toks::scratch_command_error(error));
        }
        Ok(())
    }

    /// Starts the raw balanced child of an active `\expanded` collector.
    /// Its destination is the parent's already-open token buffer, so the
    /// child can retire without copying or installing an intermediate list.
    pub(super) fn begin_unexpanded_continuation_with_parent(
        &mut self,
        opener: OriginId,
        writer: crate::attempt::AttemptTokenBufferId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        let attempt_opening = self.command.attempt.arena().mark();
        self.command
            .scratch
            .push_unexpanded_control_with_parent(opener, attempt_opening, writer, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Starts the raw balanced child of an active `\expanded` collector for
    /// `\detokenize`. Rendered characters are written directly to the
    /// parent's token buffer as the child settles each source word.
    pub(super) fn begin_detokenize_continuation_with_parent(
        &mut self,
        opener: OriginId,
        writer: crate::attempt::AttemptTokenBufferId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        let attempt_opening = self.command.attempt.arena().mark();
        self.command
            .scratch
            .push_detokenize_control_with_parent(opener, attempt_opening, writer, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Applies TeX82 §403's missing-left-brace recovery to an expanded
    /// collector. The rejected command is backed up as the first body token;
    /// the synthetic opening brace exists only in alignment and cursor state,
    /// so no second input owner is needed.
    pub(super) fn recover_expanded_opening(
        &mut self,
        command: HotCommand<G>,
    ) -> Result<(), CommandError> {
        let command = command.materialize();
        let deferred = {
            let mut report = self.state.print_err("Missing { inserted");
            report.help(&[
                "A left brace was mandatory here, so I've put one in.",
                "You might want to delete and/or insert some corrections",
                "so that I will find a matching right brace soon.",
                "(If you're confused by all this, try typing `I}' now.)",
            ]);
            report.defer()
        };
        self.back_input(command)?;
        let context = self.command.output_open_context(self.state);
        let mut report = self.state.resume_error_report(deferred);
        report.context(context);
        let outcome = report.error();
        self.finish_error_outcome(outcome)?;
        self.command.record_alignment_phase();
        self.command.alignment.align_state += 1;
        self.command
            .scratch
            .begin_expanded_body()
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Appends one settled unexpandable token to the active `\expanded`
    /// body.  Balance is updated from the literal spelling, as required by
    /// `scan_toks`; the closing delimiter is consumed rather than stored.
    /// Returning `true` retires the collector and inserts its attempt-owned
    /// result into the same input stack used by ordinary `ins_list` output.
    pub(super) fn append_expanded_word(
        &mut self,
        command: &HotCommand<G>,
    ) -> Result<bool, CommandError> {
        let control = self
            .command
            .scratch
            .top_expanded_control()
            .map_err(crate::scan_toks::scratch_command_error)?
            .ok_or_else(CommandError::input_invariant)?;
        let kind = control.kind;
        let writer = control.writer;
        let closes = self
            .command
            .scratch
            .settle_expanded_word(command.spelling_word())
            .map_err(crate::scan_toks::scratch_command_error)?;
        let control = if closes {
            Some(
                self.command
                    .scratch
                    .pop_expanded_control_with_parent()
                    .map_err(crate::scan_toks::scratch_command_error)?,
            )
        } else {
            None
        };
        if !closes {
            if kind == crate::expansion_work::control::SynchronousExpandedKind::Detokenize {
                let token = command.spelling_word().semantic_token();
                let mut failure = None;
                {
                    let state = &*self.state;
                    let arena = self.command.attempt.arena_mut();
                    tex_state::token_show::for_each_token_string_char(state, token, |ch| {
                        if failure.is_none()
                            && let Err(error) = arena.push_buffer_token(
                                writer,
                                tex_state::token::TracedTokenWord::pack(
                                    tex_state::token::Token::Char {
                                        ch,
                                        cat: if ch == ' ' {
                                            tex_state::token::Catcode::Space
                                        } else {
                                            tex_state::token::Catcode::Other
                                        },
                                    },
                                    OriginId::UNKNOWN,
                                ),
                            )
                        {
                            failure = Some(error);
                        }
                    });
                }
                if let Some(error) = failure {
                    return Err(crate::scan_toks::attempt_command_error(error));
                }
                return Ok(false);
            }
            let word = tex_state::token::TracedTokenWord::pack(
                command.spelling_word().semantic_token(),
                command.origin(),
            );
            self.command
                .attempt
                .arena_mut()
                .push_buffer_token(writer, word)
                .map_err(crate::scan_toks::attempt_command_error)?;
            return Ok(false);
        }
        let (control, parent) = control.expect("closing expanded word retires its control");
        if matches!(
            control.kind,
            crate::expansion_work::control::SynchronousExpandedKind::Unexpanded
                | crate::expansion_work::control::SynchronousExpandedKind::Detokenize
        ) {
            return Ok(true);
        }
        let list = self
            .command
            .attempt
            .arena_mut()
            .finish_token_buffer(control.writer)
            .map_err(crate::scan_toks::attempt_command_error)?;
        if control.kind
            == crate::expansion_work::control::SynchronousExpandedKind::PdfStringCompareLeft
        {
            let writer = self
                .command
                .attempt
                .arena_mut()
                .allocate_token_buffer()
                .map_err(crate::scan_toks::attempt_command_error)?;
            self.command
                .scratch
                .push_pdf_string_control_with_parent(
                    control.opener,
                    crate::expansion_work::control::SynchronousExpandedKind::PdfStringCompareRight,
                    control.attempt_opening,
                    writer,
                    Some(list),
                    parent,
                )
                .map_err(crate::scan_toks::scratch_command_error)?;
            return Ok(true);
        }
        if control.kind
            == crate::expansion_work::control::SynchronousExpandedKind::PdfStringCompareRight
        {
            let left = control.left.ok_or_else(CommandError::input_invariant)?;
            self.finish_pdf_string_compare_continuation(left, list, control.opener)?;
            if let Some(parent) = parent {
                self.command
                    .scratch
                    .resume_expansion_control_parent(parent)
                    .map_err(crate::scan_toks::scratch_command_error)?;
            }
            return Ok(true);
        }
        if matches!(
            control.kind,
            crate::expansion_work::control::SynchronousExpandedKind::PdfEscapeString
                | crate::expansion_work::control::SynchronousExpandedKind::PdfEscapeHex
                | crate::expansion_work::control::SynchronousExpandedKind::PdfUnescapeHex
        ) {
            self.finish_pdf_string_continuation(control.kind, list, control.opener)?;
            return Ok(true);
        }
        let len = self
            .command
            .attempt_token_words(list)
            .map_err(crate::scan_toks::attempt_command_error)?
            .len();
        let first = if len == 0 {
            None
        } else {
            Some(
                self.command
                    .attempt
                    .arena()
                    .token_word(list, 0)
                    .map_err(crate::scan_toks::attempt_command_error)?
                    .semantic_token(),
            )
        };
        self.insert_expansion_list(
            crate::input::PackedTokenSpanHandle::AttemptList {
                list,
                len: u32::try_from(len).map_err(|_| CommandError::input_invariant())?,
            },
            first,
        );
        Ok(true)
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
        self.invalidate_delivery_freshness();
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
