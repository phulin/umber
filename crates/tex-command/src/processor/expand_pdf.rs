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
    ) -> Result<(), CommandError> {
        let mut restore_at_page_start = false;
        let mode = if self.scan_keyword_retained("page").into_result()?.value {
            restore_at_page_start = true;
            let direct = self.scan_keyword_retained("direct").into_result()?.value;
            if direct {
                tex_state::PdfColorStackMode::Direct
            } else if self.scan_keyword_retained("page").into_result()?.value {
                tex_state::PdfColorStackMode::Page
            } else {
                tex_state::PdfColorStackMode::Origin
            }
        } else if self.scan_keyword_retained("direct").into_result()?.value {
            tex_state::PdfColorStackMode::Direct
        } else if self.scan_keyword_retained("page").into_result()?.value {
            tex_state::PdfColorStackMode::Page
        } else {
            tex_state::PdfColorStackMode::Origin
        };
        let initial = self.scan_balanced_text(true)?.tokens;
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
    ) -> Result<(), CommandError> {
        let scan = self.scan_integer_retained();
        let bound = scan.into_result()?.value;
        let value = self.state.pdf_uniform_deviate(bound);
        self.push_rendered_text(&value.to_string(), opener.origin());
        Ok(())
    }

    pub(super) fn expand_pdf_ximage_bbox(
        &mut self,
        opener: &CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let scan = self.scan_integer_retained();
        let object = u32::try_from(scan.into_result()?.value).ok();
        let id = object.and_then(|raw| tex_state::PdfExternalImageId::new(raw).ok());
        let Some(id) = id.filter(|id| self.state.pdf_external_image(*id).is_some()) else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): cannot find referenced object.",
            ));
        };
        let scan = self.scan_integer_retained();
        let index = scan.into_result()?.value;
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
    ) -> Result<(), CommandError> {
        let scan = self.scan_integer_retained();
        let object = scan.into_result()?.value;
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
    ) -> Result<(), CommandError> {
        let scan = self.scan_integer_retained();
        let page = scan.into_result()?.value;
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
    ) -> Result<(), CommandError> {
        let scan = self.scan_integer_retained();
        let mut index = scan.into_result()?.value;
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
    ) -> Result<(), CommandError> {
        let scan = self.scan_extended_register_index_retained();
        let class = scan.into_result()?;
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
