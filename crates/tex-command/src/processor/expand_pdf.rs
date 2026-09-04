//! pdfTeX state and object enquiry expansion primitives.

use crate::command::HotCommand;
use crate::{CommandError, CurrentCommand};
use tex_state::token::OriginId;

use super::CommandProcessor;
use super::expand_render::format_scaled;

/// Stable pending-diagnostic identity for pdftex.web §495's color-stack
/// capacity recovery.
pub(crate) const TOO_MANY_COLOR_STACKS_DIAGNOSTIC: u64 = 0x7064_6663_7300_0495;

impl<G> CommandProcessor<'_, '_, G> {
    pub(super) fn begin_pdf_ximage_bbox_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_pdf_ximage_bbox_control_with_parent(opener, parent)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Consumes one settled integer of `\pdfximagebbox` without entering the
    /// retained scalar scanner.  The two operands are deliberately kept in a
    /// copy-small control: object validation happens before the coordinate is
    /// read, matching pdfTeX's diagnostic order.
    pub(super) fn advance_pdf_ximage_bbox_continuation(
        &mut self,
        command: HotCommand<G>,
        at_end: bool,
    ) -> Result<bool, CommandError> {
        use crate::expansion_work::control::SynchronousPdfXImageBBoxPhase as Phase;

        let control = self
            .command
            .scratch
            .top_pdf_ximage_bbox_control()
            .map_err(crate::scan_toks::scratch_command_error)?
            .ok_or_else(CommandError::input_invariant)?;
        let character = command.character_value();
        let is_space = command.character_catcode() == Some(tex_state::token::Catcode::Space);
        let digit = character
            .filter(|ch| ch.is_ascii_digit())
            .map(|ch| i64::from(ch as u8 - b'0'));
        let accumulate = |value: i64, digit: i64| {
            value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .unwrap_or(i64::from(i32::MAX))
                .min(i64::from(i32::MAX))
        };
        let signed = |value: i64, negative: bool| {
            let value = value.min(i64::from(i32::MAX));
            if negative { -value } else { value }
        };

        match control.phase {
            Phase::Object {
                negative,
                value,
                seen_digit,
            } => {
                if is_space && !seen_digit {
                    return Ok(false);
                }
                if (character == Some('+') || character == Some('-')) && !seen_digit {
                    self.command
                        .scratch
                        .set_pdf_ximage_bbox_phase(Phase::Object {
                            negative: character == Some('-'),
                            value,
                            seen_digit,
                        })?;
                    return Ok(false);
                }
                if let Some(digit) = digit {
                    self.command
                        .scratch
                        .set_pdf_ximage_bbox_phase(Phase::Object {
                            negative,
                            value: accumulate(value, digit),
                            seen_digit: true,
                        })?;
                    return Ok(false);
                }
                if !seen_digit {
                    let site = (!at_end).then(|| self.capture_hot_diagnostic_site(&command));
                    if !at_end {
                        self.back_input(command.materialize())?;
                    }
                    self.missing_number_error_at(site)?;
                } else if !at_end && !is_space {
                    self.back_input(command.materialize())?;
                }
                let object = signed(value, negative);
                let id = u32::try_from(object)
                    .ok()
                    .and_then(|raw| tex_state::PdfExternalImageId::new(raw).ok())
                    .filter(|id| self.state.pdf_external_image(*id).is_some())
                    .ok_or(CommandError::PdfNavigation(
                        "pdfTeX error (ext1): cannot find referenced object.",
                    ))?;
                self.command
                    .scratch
                    .set_pdf_ximage_bbox_phase(Phase::Coordinate {
                        object: id.raw(),
                        negative: false,
                        value: 0,
                        seen_digit: false,
                    })?;
                Ok(false)
            }
            Phase::Coordinate {
                object,
                negative,
                value,
                seen_digit,
            } => {
                if is_space && !seen_digit {
                    return Ok(false);
                }
                if (character == Some('+') || character == Some('-')) && !seen_digit {
                    self.command
                        .scratch
                        .set_pdf_ximage_bbox_phase(Phase::Coordinate {
                            object,
                            negative: character == Some('-'),
                            value,
                            seen_digit,
                        })?;
                    return Ok(false);
                }
                if let Some(digit) = digit {
                    self.command
                        .scratch
                        .set_pdf_ximage_bbox_phase(Phase::Coordinate {
                            object,
                            negative,
                            value: accumulate(value, digit),
                            seen_digit: true,
                        })?;
                    return Ok(false);
                }
                if !seen_digit {
                    let site = (!at_end).then(|| self.capture_hot_diagnostic_site(&command));
                    if !at_end {
                        self.back_input(command.materialize())?;
                    }
                    self.missing_number_error_at(site)?;
                } else if !at_end && !is_space {
                    self.back_input(command.materialize())?;
                }
                let coordinate = signed(value, negative);
                let id = tex_state::PdfExternalImageId::new(object).map_err(|_| {
                    CommandError::PdfNavigation(
                        "pdfTeX error (ext1): cannot find referenced object.",
                    )
                })?;
                let metadata =
                    self.state
                        .pdf_external_image(id)
                        .ok_or(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): cannot find referenced object.",
                        ))?;
                let coordinate = u8::try_from(coordinate)
                    .ok()
                    .and_then(|index| metadata.bbox_coordinate(index))
                    .ok_or(CommandError::PdfNavigation(
                        "pdfTeX error (pdfximagebbox): invalid parameter.",
                    ))?;
                let control = self
                    .command
                    .scratch
                    .pop_pdf_ximage_bbox_control()
                    .map_err(crate::scan_toks::scratch_command_error)?;
                self.push_rendered_text(&format_scaled(coordinate), control.opener);
                Ok(true)
            }
        }
    }

    pub(super) fn finish_pdf_ximage_bbox_continuation_at_end(
        &mut self,
    ) -> Result<bool, CommandError> {
        if self
            .command
            .scratch
            .top_pdf_ximage_bbox_control()
            .map_err(crate::scan_toks::scratch_command_error)?
            .is_none()
        {
            return Ok(false);
        }
        self.advance_pdf_ximage_bbox_continuation(HotCommand::empty(), true)
    }

    /// pdftex.web §495's `pdf_colorstack_init_code` conversion.
    pub(super) fn expand_pdf_color_stack_init(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let (mut restore_at_page_start, mut option_phase, retained_mode) =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => (false, Some(0_u8), None),
                crate::state::PendingExpansionResume::PdfColorStackInitOptions {
                    restore_at_page_start,
                    phase,
                } => (restore_at_page_start, Some(phase), None),
                crate::state::PendingExpansionResume::PdfColorStackInitText {
                    restore_at_page_start,
                    mode,
                } => (restore_at_page_start, None, Some(mode)),
                _ => return Err(CommandError::input_invariant()),
            };
        let mode = if let Some(mut phase) = option_phase.take() {
            loop {
                let keyword = match phase {
                    0 => "page",
                    1 => "direct",
                    2 => "page",
                    _ => return Err(CommandError::input_invariant()),
                };
                let scan = self.scan_keyword_retained(keyword);
                let matched = self
                    .retain_expansion_scalar(
                        scan,
                        crate::state::PendingExpansionResume::PdfColorStackInitOptions {
                            restore_at_page_start,
                            phase,
                        },
                        suspended,
                    )?
                    .value;
                match phase {
                    0 => {
                        restore_at_page_start = matched;
                        phase = 1;
                    }
                    1 if matched => break tex_state::PdfColorStackMode::Direct,
                    1 => phase = 2,
                    2 if matched => break tex_state::PdfColorStackMode::Page,
                    2 => break tex_state::PdfColorStackMode::Origin,
                    _ => unreachable!(),
                }
            }
        } else {
            retained_mode.expect("completed color-stack options retain their mode")
        };
        let initial = match self.scan_balanced_text(true) {
            Ok(initial) => initial.tokens,
            Err(error) => {
                if error.is_resource_suspension() {
                    *suspended = Some(
                        crate::state::PendingExpansionResume::PdfColorStackInitText {
                            restore_at_page_start,
                            mode,
                        },
                    );
                }
                return Err(error);
            }
        };
        let initial = self.attempt_token_list_bytes(initial)?;
        let id = match self
            .state
            .allocate_pdf_color_stack(mode, restore_at_page_start, initial)
        {
            Ok(id) => id,
            Err(_) => {
                self.report_recoverable(
                    TOO_MANY_COLOR_STACKS_DIAGNOSTIC,
                    "Too many color stacks".to_owned(),
                    &[
                        "The number of color stacks is limited to 32768.",
                        "I'll use the default color stack 0 here.",
                    ],
                );
                0
            }
        };
        self.push_rendered_text(&id.to_string(), opener.origin());
        Ok(())
    }

    pub(super) fn expand_pdf_uniform_deviate(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfUniformDeviate => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let bound = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfUniformDeviate,
                suspended,
            )?
            .value;
        let value = self.state.pdf_uniform_deviate(bound);
        self.push_rendered_text(&value.to_string(), opener.origin());
        Ok(())
    }

    pub(super) fn expand_pdf_ximage_bbox(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let object = match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch)
        {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfXImageObject => {
                let scan = self.scan_integer_retained();
                let object = self
                    .retain_expansion_scalar(
                        scan,
                        crate::state::PendingExpansionResume::PdfXImageObject,
                        suspended,
                    )?
                    .value;
                u32::try_from(object).ok()
            }
            crate::state::PendingExpansionResume::PdfXImageCoordinate { object } => Some(object),
            _ => return Err(CommandError::input_invariant()),
        };
        let id = object.and_then(|raw| tex_state::PdfExternalImageId::new(raw).ok());
        let Some(id) = id.filter(|id| self.state.pdf_external_image(*id).is_some()) else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): cannot find referenced object.",
            ));
        };
        let object = id.raw();
        let scan = self.scan_integer_retained();
        let index = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfXImageCoordinate { object },
                suspended,
            )?
            .value;
        let metadata = self
            .state
            .pdf_external_image(id)
            .expect("validated external image remains present");
        let Some(coordinate) = u8::try_from(index)
            .ok()
            .and_then(|index| metadata.bbox_coordinate(index))
        else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (pdfximagebbox): invalid parameter.",
            ));
        };
        self.push_rendered_text(&format_scaled(coordinate), opener.origin());
        Ok(())
    }

    pub(super) fn expand_pdf_xform_name(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfXFormName => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let object = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfXFormName,
                suspended,
            )?
            .value;
        let resource = u32::try_from(object)
            .ok()
            .and_then(|object| self.state.pdf_form_resource(object))
            .unwrap_or(0);
        self.push_rendered_text(&resource.to_string(), opener.origin());
        Ok(())
    }

    pub(super) fn expand_pdf_page_ref(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfPageRef => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let page = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfPageRef,
                suspended,
            )?
            .value;
        if page <= 0 {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (pageref): invalid page number",
            ));
        }
        let object = u32::try_from(page)
            .ok()
            .and_then(|page| self.state.pdf_page_object(page))
            .unwrap_or(0);
        self.push_rendered_text(&object.to_string(), opener.origin());
        Ok(())
    }

    pub(super) fn expand_pdf_last_match(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfLastMatch => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let mut index = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfLastMatch,
                suspended,
            )?
            .value;
        if index < 0 {
            self.pdftex_match_number_diagnostic(index);
            index = 1;
        }
        let capture = u32::try_from(index)
            .ok()
            .and_then(|index| self.state.pdf_match_capture(index))
            .map(|(offset, bytes)| (offset, bytes.to_vec()));
        let mut rendered = match capture {
            Some((offset, _)) => format!("{offset}->"),
            None => "-1->".to_owned(),
        };
        if let Some((_, bytes)) = capture {
            rendered.extend(bytes.into_iter().map(char::from));
        }
        self.push_rendered_text(&rendered, opener.origin());
        Ok(())
    }

    /// pdftex.web §1590's `pdf_insert_ht_code` conversion reads the height
    /// accumulated in the live page-builder insertion record. Missing classes
    /// use pdfTeX's literal `0pt`; present zero heights use `print_scaled` and
    /// therefore remain distinguishable as `0.0pt`.
    pub(super) fn expand_pdf_insert_height(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::PdfInsertHeight
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_extended_register_index_retained();
        let class = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::PdfInsertHeight,
            suspended,
        )?;
        let rendered = self
            .state
            .page_insertion(class)
            .map(|insertion| insertion.height())
            .map_or_else(|| "0pt".to_owned(), format_scaled);
        self.push_rendered_text(&rendered, opener.origin());
        Ok(())
    }

    pub(super) fn pdftex_match_number_diagnostic(&mut self, value: i32) {
        self.command.semantic_diagnostics.push(
            crate::CommandSemanticDiagnostic::PdfExpansionMessage {
                text: format!("! Bad match number ({value})."),
            },
        );
    }
}
