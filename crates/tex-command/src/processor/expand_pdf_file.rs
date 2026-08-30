//! pdfTeX immutable-file enquiry expansion primitives.

use crate::{CommandError, CurrentCommand};

use super::CommandProcessor;
use super::expand_render::format_pdf_date;

impl<G> CommandProcessor<'_, '_, G> {
    /// pdftex.web §1590's `pdf_file_dump_code` conversion.
    ///
    /// The filename is scanned before the immutable input capability is
    /// consulted. An absent capability retains the corrected range and typed
    /// request, so the host retry neither repeats diagnostics nor rescans the
    /// consumed operands.
    pub(super) fn expand_pdf_file_dump(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let (pending, scanned_range, scanned_options) =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => (None, None, Some((0, 0, 0))),
                crate::state::PendingExpansionResume::PdfFileDumpOptions {
                    offset,
                    length,
                    phase,
                } => (None, None, Some((offset, length, phase))),
                crate::state::PendingExpansionResume::PdfFileDumpText { offset, length } => {
                    (None, Some((offset, length)), None)
                }
                crate::state::PendingExpansionResume::PdfFileDump(pending) => {
                    (Some(pending), None, None)
                }
                _ => return Err(CommandError::input_invariant()),
            };
        let (request, offset, length) = if let Some(pending) = pending {
            (pending.request, pending.offset, pending.length)
        } else {
            let (offset, length) = if let Some(range) = scanned_range {
                range
            } else {
                let (mut offset, mut length, mut phase) =
                    scanned_options.expect("unscanned dump options retain their cursor");
                loop {
                    match phase {
                        0 => {
                            let scan = self.scan_keyword_retained("offset");
                            if self
                                .retain_expansion_scalar(
                                    scan,
                                    crate::state::PendingExpansionResume::PdfFileDumpOptions {
                                        offset,
                                        length,
                                        phase,
                                    },
                                    suspended,
                                )?
                                .value
                            {
                                phase = 1;
                            } else {
                                phase = 2;
                            }
                        }
                        1 => {
                            let scan = self.scan_integer_retained();
                            offset = self
                                .retain_expansion_scalar(
                                    scan,
                                    crate::state::PendingExpansionResume::PdfFileDumpOptions {
                                        offset,
                                        length,
                                        phase,
                                    },
                                    suspended,
                                )?
                                .value;
                            if offset < 0 {
                                self.pdftex_file_range_diagnostic("offset", offset);
                                offset = 0;
                            }
                            phase = 2;
                        }
                        2 => {
                            let scan = self.scan_keyword_retained("length");
                            if self
                                .retain_expansion_scalar(
                                    scan,
                                    crate::state::PendingExpansionResume::PdfFileDumpOptions {
                                        offset,
                                        length,
                                        phase,
                                    },
                                    suspended,
                                )?
                                .value
                            {
                                phase = 3;
                            } else {
                                break;
                            }
                        }
                        3 => {
                            let scan = self.scan_integer_retained();
                            length = self
                                .retain_expansion_scalar(
                                    scan,
                                    crate::state::PendingExpansionResume::PdfFileDumpOptions {
                                        offset,
                                        length,
                                        phase,
                                    },
                                    suspended,
                                )?
                                .value;
                            if length < 0 {
                                self.pdftex_file_range_diagnostic("length", length);
                                length = 0;
                            }
                            break;
                        }
                        _ => return Err(CommandError::input_invariant()),
                    }
                }
                (offset, length)
            };
            let name = match self.scan_balanced_text(true) {
                Ok(name) => name.tokens,
                Err(error) => {
                    if error.is_resource_suspension() {
                        *suspended = Some(crate::state::PendingExpansionResume::PdfFileDumpText {
                            offset,
                            length,
                        });
                    }
                    return Err(error);
                }
            };
            let name = self
                .attempt_token_list_bytes(name)?
                .into_iter()
                .map(char::from)
                .collect::<String>();
            (
                crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::Dump),
                offset,
                length,
            )
        };
        self.state.unsupported_host_capability();
        let Some(source) = self.host.input_probe(&request.name) else {
            return if self.host.input_probe_is_unavailable(&request.name) {
                Ok(())
            } else {
                *suspended = Some(crate::state::PendingExpansionResume::PdfFileDump(
                    crate::state::PendingFileEnquiry {
                        request: request.clone(),
                        offset,
                        length,
                    },
                ));
                Err(CommandError::MissingInputProbe(request))
            };
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
        self.push_rendered_text(&rendered, opener.origin());
        Ok(())
    }

    /// pdftex.web §1590's `pdf_file_size_code` conversion.
    pub(super) fn expand_pdf_file_size(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let pending =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => None,
                crate::state::PendingExpansionResume::PdfFileSize(pending) => Some(pending),
                _ => return Err(CommandError::input_invariant()),
            };
        let request = if let Some(pending) = pending {
            pending.request
        } else {
            let name = self.scan_balanced_text(true)?.tokens;
            let name = self
                .attempt_token_list_bytes(name)?
                .into_iter()
                .map(char::from)
                .collect::<String>();
            crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::Size)
        };
        self.state.unsupported_host_capability();
        let Some(source) = self.host.input_probe(&request.name) else {
            return if self.host.input_probe_is_unavailable(&request.name) {
                Ok(())
            } else {
                *suspended = Some(crate::state::PendingExpansionResume::PdfFileSize(
                    crate::state::PendingFileEnquiry {
                        request: request.clone(),
                        offset: 0,
                        length: 0,
                    },
                ));
                Err(CommandError::MissingInputProbe(request))
            };
        };
        self.push_rendered_text(&source.source().bytes().len().to_string(), opener.origin());
        Ok(())
    }

    /// pdftex.web §1590's `pdf_file_mod_date_code` conversion.
    pub(super) fn expand_pdf_file_modification_date(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let pending =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => None,
                crate::state::PendingExpansionResume::PdfFileModificationDate(pending) => {
                    Some(pending)
                }
                _ => return Err(CommandError::input_invariant()),
            };
        let request = if let Some(pending) = pending {
            pending.request
        } else {
            crate::FileEnquiryRequest::new(
                self.scan_pdf_file_name()?,
                crate::FileEnquiryIntent::ModificationDate,
            )
        };
        self.state.unsupported_host_capability();
        let Some(resource) = self.host.input_probe(&request.name) else {
            return if self.host.input_probe_is_unavailable(&request.name) {
                Ok(())
            } else {
                *suspended = Some(
                    crate::state::PendingExpansionResume::PdfFileModificationDate(
                        crate::state::PendingFileEnquiry {
                            request: request.clone(),
                            offset: 0,
                            length: 0,
                        },
                    ),
                );
                Err(CommandError::MissingInputProbe(request))
            };
        };
        if let Some(date) = resource.modification_date() {
            self.push_rendered_text(
                &format_pdf_date(date.clock, date.utc_offset_minutes),
                opener.origin(),
            );
        }
        Ok(())
    }

    /// pdftex.web §1590's string/file MD5 conversion.
    pub(super) fn expand_pdf_md_five_sum(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        use md5::{Digest, Md5};
        let (pending, scanned_file, scan_file_keyword) =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => (None, None, true),
                crate::state::PendingExpansionResume::PdfMdFiveSumFile => (None, None, true),
                crate::state::PendingExpansionResume::PdfMdFiveSumText { file } => {
                    (None, Some(file), false)
                }
                crate::state::PendingExpansionResume::PdfMdFiveSum(pending) => {
                    (Some(pending), None, false)
                }
                _ => return Err(CommandError::input_invariant()),
            };
        let file = if pending.is_some() {
            true
        } else if let Some(file) = scanned_file {
            file
        } else if scan_file_keyword {
            let scan = self.scan_keyword_retained("file");
            self.retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfMdFiveSumFile,
                suspended,
            )?
            .value
        } else {
            return Err(CommandError::input_invariant());
        };
        let mut bytes = if let Some(pending) = &pending {
            pending.request.name.as_bytes().to_vec()
        } else {
            let tokens = match self.scan_balanced_text(true) {
                Ok(tokens) => tokens.tokens,
                Err(error) => {
                    if error.is_resource_suspension() {
                        *suspended =
                            Some(crate::state::PendingExpansionResume::PdfMdFiveSumText { file });
                    }
                    return Err(error);
                }
            };
            self.attempt_token_list_bytes(tokens)?
        };
        if file {
            let request = pending.map_or_else(
                || {
                    let name = bytes.iter().copied().map(char::from).collect::<String>();
                    crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::MdFiveSum)
                },
                |pending| pending.request,
            );
            self.state.unsupported_host_capability();
            let Some(resource) = self.host.input_probe(&request.name) else {
                return if self.host.input_probe_is_unavailable(&request.name) {
                    Ok(())
                } else {
                    *suspended = Some(crate::state::PendingExpansionResume::PdfMdFiveSum(
                        crate::state::PendingFileEnquiry {
                            request: request.clone(),
                            offset: 0,
                            length: 0,
                        },
                    ));
                    Err(CommandError::MissingInputProbe(request))
                };
            };
            bytes = resource.source().bytes().to_vec();
        }
        let digest = Md5::digest(bytes);
        let rendered = digest
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();
        self.push_rendered_text(&rendered, opener.origin());
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
