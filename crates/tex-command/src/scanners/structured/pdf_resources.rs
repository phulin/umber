use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    /// Scans pdfTeX's raw-object, form, and document-fragment extensions.
    ///
    /// This is the command boundary corresponding to pdftex.web's extension
    /// cases: `scan_keyword` and expanded `scan_pdf_ext_toks` are complete
    /// before the executor mutates its PDF ledger or mode list.
    pub fn scan_pdf_object_request(&mut self) -> Result<PdfObjectRequest, CommandError> {
        let mut progress = PdfObjectScalarProgress {
            use_object: None,
            stream: false,
            stream_attr: None,
            phase: PdfObjectScalarPhase::ReserveKeyword,
        };
        loop {
            match progress.phase {
                PdfObjectScalarPhase::ReserveKeyword => {
                    let result = self.scan_keyword_retained("reserveobjnum");
                    if result.into_result()?.value {
                        return Ok(PdfObjectRequest::Reserve);
                    }
                    progress.phase = PdfObjectScalarPhase::UseKeyword;
                }
                PdfObjectScalarPhase::UseKeyword => {
                    let result = self.scan_keyword_retained("useobjnum");
                    progress.phase = if result.into_result()?.value {
                        PdfObjectScalarPhase::UseObject
                    } else {
                        PdfObjectScalarPhase::StreamKeyword
                    };
                }
                PdfObjectScalarPhase::UseObject => {
                    let result = self.scan_integer_retained();
                    progress.use_object = Some(result.into_result()?.value);
                    progress.phase = PdfObjectScalarPhase::StreamKeyword;
                }
                PdfObjectScalarPhase::StreamKeyword => {
                    let result = self.scan_keyword_retained("stream");
                    progress.stream = result.into_result()?.value;
                    progress.phase = if progress.stream {
                        PdfObjectScalarPhase::AttributeKeyword
                    } else {
                        PdfObjectScalarPhase::FileKeyword
                    };
                }
                PdfObjectScalarPhase::AttributeKeyword => {
                    let result = self.scan_keyword_retained("attr");
                    if result.into_result()?.value {
                        progress.stream_attr = match self.scan_balanced_text(true) {
                            Ok(value) => Some(value),
                            Err(error) => {
                                return Err(error);
                            }
                        };
                    }
                    progress.phase = PdfObjectScalarPhase::FileKeyword;
                }
                PdfObjectScalarPhase::FileKeyword => {
                    let result = self.scan_keyword_retained("file");
                    let file = result.into_result()?.value;
                    let data = match self.scan_balanced_text(true) {
                        Ok(data) => data,
                        Err(error) => {
                            return Err(error);
                        }
                    };
                    return Ok(PdfObjectRequest::Define {
                        use_object: progress.use_object,
                        stream: progress.stream,
                        stream_attr: progress.stream_attr,
                        file,
                        data,
                    });
                }
            }
        }
    }

    pub fn scan_pdf_form_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfFormRequest, CommandError> {
        if primitive == UnexpandablePrimitive::PdfRefXForm {
            let result = self.scan_integer_retained();
            return Ok(PdfFormRequest::Reference {
                object: result.into_result()?.value,
            });
        }
        let mut progress = PdfFormScalarProgress {
            attr: None,
            resources: None,
            phase: PdfFormScalarPhase::AttributeKeyword,
        };
        loop {
            match progress.phase {
                PdfFormScalarPhase::AttributeKeyword => {
                    let result = self.scan_keyword_retained("attr");
                    if result.into_result()?.value {
                        progress.attr = match self.scan_balanced_text(true) {
                            Ok(attr) => Some(attr),
                            Err(error) => {
                                return Err(error);
                            }
                        };
                    }
                    progress.phase = PdfFormScalarPhase::ResourcesKeyword;
                }
                PdfFormScalarPhase::ResourcesKeyword => {
                    let result = self.scan_keyword_retained("resources");
                    if result.into_result()?.value {
                        let resources = match self.scan_balanced_text(true) {
                            Ok(resources) => resources,
                            Err(error) => {
                                return Err(error);
                            }
                        };
                        let result = self.scan_extended_register_index_retained();
                        let box_register = result.into_result()?;
                        return Ok(PdfFormRequest::Create {
                            attr: progress.attr,
                            resources: Some(resources),
                            box_register,
                        });
                    }
                    progress.phase = PdfFormScalarPhase::BoxRegister;
                }
                PdfFormScalarPhase::BoxRegister => {
                    let result = self.scan_extended_register_index_retained();
                    let box_register = result.into_result()?;
                    return Ok(PdfFormRequest::Create {
                        attr: progress.attr,
                        resources: progress.resources,
                        box_register,
                    });
                }
            }
        }
    }

    pub fn scan_pdf_reference_object_request(
        &mut self,
    ) -> Result<PdfReferenceObjectRequest, CommandError> {
        let result = self.scan_integer_retained();
        Ok(PdfReferenceObjectRequest {
            object: result.into_result()?.value,
        })
    }

    pub fn scan_pdf_font_action(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedPdfFontAction, CommandError> {
        let needs_font = matches!(
            primitive,
            UnexpandablePrimitive::PdfFontAttr
                | UnexpandablePrimitive::PdfIncludeChars
                | UnexpandablePrimitive::PdfNoBuiltinToUnicode
        );
        let font = if needs_font {
            let scan = self.scan_font_selector_retained();
            Some(scan.into_result()?)
        } else {
            None
        };
        if primitive == UnexpandablePrimitive::PdfNoBuiltinToUnicode {
            return Ok(ScannedPdfFontAction {
                font,
                first: None,
                second: None,
            });
        }
        let first = match self.scan_balanced_text(true) {
            Ok(first) => first.tokens,
            Err(error) => {
                return Err(error);
            }
        };
        let second = if primitive == UnexpandablePrimitive::PdfGlyphToUnicode {
            match self.scan_balanced_text(true) {
                Ok(second) => Some(second.tokens),
                Err(error) => {
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok(ScannedPdfFontAction {
            font,
            first: Some(first),
            second,
        })
    }

    pub fn scan_pdf_document_fragment_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfDocumentFragmentRequest, CommandError> {
        use tex_state::PdfDocumentFragmentKind as Kind;
        let (kind, text) = {
            let kind = match primitive {
                UnexpandablePrimitive::PdfInfo => Kind::Info,
                UnexpandablePrimitive::PdfCatalog => Kind::Catalog,
                UnexpandablePrimitive::PdfNames => Kind::Names,
                UnexpandablePrimitive::PdfTrailer => Kind::Trailer,
                UnexpandablePrimitive::PdfTrailerId => Kind::TrailerId,
                _ => return Err(CommandError::input_invariant()),
            };
            let text = self.scan_pdf_navigation_text()?;
            (kind, text)
        };
        let open_action = if kind == Kind::Catalog && {
            let result = self.scan_keyword_retained("openaction");

            result.into_result()?.value
        } {
            let (owner, action) =
                self.scan_pdf_action_for_owner(PendingPdfActionOwner::DocumentFragment {
                    kind,
                    text,
                })?;
            let PendingPdfActionOwner::DocumentFragment { kind, text } = owner else {
                return Err(CommandError::input_invariant());
            };
            return Ok(PdfDocumentFragmentRequest {
                kind,
                text,
                open_action: Some(action),
            });
        } else {
            None
        };
        Ok(PdfDocumentFragmentRequest {
            kind,
            text,
            open_action,
        })
    }
}
