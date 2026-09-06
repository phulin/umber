//! pdfTeX immutable-file enquiry expansion primitives.

use tex_state::token::OriginId;

use crate::CommandError;

use super::CommandProcessor;
use super::expand_render::format_pdf_date;

impl<G> CommandProcessor<'_, '_, G> {
    /// pdftex.web §1590's `pdf_file_dump_code` conversion.
    ///
    /// The filename is scanned before the immutable input capability is
    /// consulted. An absent capability retains the corrected range and typed
    /// request, so the host retry neither repeats diagnostics nor rescans the
    /// consumed operands.
    pub(super) fn expand_pdf_file_dump(&mut self, opener: OriginId) -> Result<(), CommandError> {
        let mut offset = 0;
        if self.scan_keyword_retained("offset").into_result()?.value {
            offset = self.scan_integer_retained().into_result()?.value;
            if offset < 0 {
                self.pdftex_file_range_diagnostic("offset", offset);
                offset = 0;
            }
        }
        let mut length = 0;
        if self.scan_keyword_retained("length").into_result()?.value {
            length = self.scan_integer_retained().into_result()?.value;
            if length < 0 {
                self.pdftex_file_range_diagnostic("length", length);
                length = 0;
            }
        }
        let tokens = self.scan_balanced_text(true)?.tokens;
        let name = self
            .attempt_token_list_bytes(tokens)?
            .into_iter()
            .map(char::from)
            .collect::<String>();
        let request = crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::Dump);
        let Some(source) = self.resolve_input_probe(&request)? else {
            return Ok(());
        };
        let start = usize::try_from(offset).expect("recovered file offset is nonnegative");
        let bytes = source.source().bytes();
        if start >= bytes.len() || length == 0 {
            return Ok(());
        }
        let end = start
            .saturating_add(usize::try_from(length).expect("recovered dump length is nonnegative"))
            .min(bytes.len());
        let mut rendered = String::with_capacity((end - start) * 2);
        for byte in &bytes[start..end] {
            use std::fmt::Write as _;
            write!(rendered, "{byte:02X}").expect("writing to a String cannot fail");
        }
        self.push_rendered_text(&rendered, opener);
        Ok(())
    }

    /// pdftex.web §1590's `pdf_file_size_code` conversion.
    pub(super) fn expand_pdf_file_size(&mut self, opener: OriginId) -> Result<(), CommandError> {
        let tokens = self.scan_balanced_text(true)?.tokens;
        let name = self
            .attempt_token_list_bytes(tokens)?
            .into_iter()
            .map(char::from)
            .collect::<String>();
        let request = crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::Size);
        let Some(source) = self.resolve_input_probe(&request)? else {
            return Ok(());
        };
        self.push_rendered_text(&source.source().bytes().len().to_string(), opener);
        Ok(())
    }

    /// pdftex.web §1590's `pdf_file_mod_date_code` conversion.
    pub(super) fn expand_pdf_file_modification_date(
        &mut self,
        opener: OriginId,
    ) -> Result<(), CommandError> {
        let request = crate::FileEnquiryRequest::new(
            self.scan_pdf_file_name()?,
            crate::FileEnquiryIntent::ModificationDate,
        );
        let Some(resource) = self.resolve_input_probe(&request)? else {
            return Ok(());
        };
        if let Some(date) = resource.modification_date() {
            self.push_rendered_text(
                &format_pdf_date(date.clock, date.utc_offset_minutes),
                opener,
            );
        }
        Ok(())
    }

    /// pdftex.web §1590's string/file MD5 conversion.
    pub(super) fn expand_pdf_md_five_sum(&mut self, opener: OriginId) -> Result<(), CommandError> {
        use md5::{Digest, Md5};
        let file = self.scan_keyword_retained("file").into_result()?.value;
        let tokens = self.scan_balanced_text(true)?.tokens;
        let mut bytes = self.attempt_token_list_bytes(tokens)?;
        if file {
            let name = bytes.iter().copied().map(char::from).collect::<String>();
            let request = crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::MdFiveSum);
            let Some(resource) = self.resolve_input_probe(&request)? else {
                return Ok(());
            };
            bytes = resource.source().bytes().to_vec();
        }
        let digest = Md5::digest(bytes);
        let rendered = digest
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();
        self.push_rendered_text(&rendered, opener);
        Ok(())
    }

    fn scan_pdf_file_name(&mut self) -> Result<String, CommandError> {
        let tokens = self.scan_balanced_text(true)?.tokens;
        Ok(self
            .attempt_token_list_bytes(tokens)?
            .into_iter()
            .map(char::from)
            .collect())
    }

    fn resolve_input_probe(
        &mut self,
        request: &crate::FileEnquiryRequest,
    ) -> Result<Option<crate::FileEnquiryResource>, CommandError> {
        self.state.unsupported_host_capability();
        let provider_settled = if self.host.input_probe(&request.name).is_none()
            && !self.host.input_probe_is_unavailable(&request.name)
        {
            let need = crate::ResourceNeed::InputProbe {
                request: request.clone(),
            };
            matches!(
                self.host
                    .resolve_and_install_resource(self.state, &need, false)?,
                Some(true)
            )
        } else {
            false
        };
        if !provider_settled {
            self.record_input_probe_dependencies(&request.name)?;
        }
        if let Some(resource) = self.host.input_probe(&request.name) {
            Ok(Some(resource))
        } else if self.host.input_probe_is_unavailable(&request.name) {
            Ok(None)
        } else {
            Err(CommandError::MissingInputProbe(request.clone()))
        }
    }

    fn record_input_probe_dependencies(&mut self, name: &str) -> Result<(), CommandError> {
        let dependencies = self.host.input_probe_dependencies(name);
        self.state
            .record_input_dependencies(&dependencies)
            .map_err(|_| CommandError::input_invariant())
    }

    fn pdftex_file_range_diagnostic(&mut self, kind: &str, value: i32) {
        let label = if kind == "offset" {
            "file offset"
        } else {
            "dump length"
        };
        self.command.semantic_diagnostics.push(
            crate::CommandSemanticDiagnostic::PdfExpansionMessage {
                text: format!("! Bad {label} ({value})."),
            },
        );
    }
}
