//! pdfTeX state and object enquiry expansion primitives.

use crate::{CommandError, CurrentCommand};

use super::CommandProcessor;
use super::expand_render::format_scaled;

/// Stable pending-diagnostic identity for pdftex.web §495's color-stack
/// capacity recovery.
pub(crate) const TOO_MANY_COLOR_STACKS_DIAGNOSTIC: u64 = 0x7064_6663_7300_0495;

impl<G> CommandProcessor<'_, '_, G> {
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

    fn pdftex_match_number_diagnostic(&mut self, value: i32) {
        self.command.semantic_diagnostics.push(
            crate::CommandSemanticDiagnostic::PdfExpansionMessage {
                text: format!("! Bad match number ({value})."),
            },
        );
    }
}
