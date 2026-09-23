use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    /// Scans pdfTeX's `scan_image` request prefix.
    ///
    /// The ordering follows pdfTeX 1.40.29's `scan_image`: a repeated rule
    /// specification, optional `attr` general text, mutually exclusive
    /// `named` expanded general text or `page` integer, optional `colorspace`
    /// integer, then one page-box selector and the filename operand. The
    /// filename uses TeX82 §§511–520's expanded
    /// filename scanner, so braces are optional, quotes protect spaces, and
    /// the first unquoted space or noncharacter remains the request boundary.
    /// Resource acquisition is expressly outside this scanner.
    pub fn scan_pdf_image_request(&mut self) -> Result<PdfImageRequest, CommandError> {
        let mut progress = PdfImageScalarProgress {
            width: None,
            height: None,
            depth: None,
            attr: None,
            page: PendingPdfImagePage::Unset,
            color_space_object: 0,
            page_box: None,
            phase: PdfImageScalarPhase::WidthKeyword,
        };
        loop {
            match progress.phase {
                PdfImageScalarPhase::WidthKeyword => {
                    let result = self.scan_keyword_retained("width");
                    progress.phase = if result.into_result()?.value {
                        PdfImageScalarPhase::WidthDimension
                    } else {
                        PdfImageScalarPhase::HeightKeyword
                    };
                }
                PdfImageScalarPhase::WidthDimension => {
                    let result = self.scan_dimension_retained();
                    progress.width = Some(result.into_result()?.value);
                    progress.phase = PdfImageScalarPhase::WidthKeyword;
                }
                PdfImageScalarPhase::HeightKeyword => {
                    let result = self.scan_keyword_retained("height");
                    progress.phase = if result.into_result()?.value {
                        PdfImageScalarPhase::HeightDimension
                    } else {
                        PdfImageScalarPhase::DepthKeyword
                    };
                }
                PdfImageScalarPhase::HeightDimension => {
                    let result = self.scan_dimension_retained();
                    progress.height = Some(result.into_result()?.value);
                    progress.phase = PdfImageScalarPhase::WidthKeyword;
                }
                PdfImageScalarPhase::DepthKeyword => {
                    let result = self.scan_keyword_retained("depth");
                    progress.phase = if result.into_result()?.value {
                        PdfImageScalarPhase::DepthDimension
                    } else {
                        PdfImageScalarPhase::AttributeKeyword
                    };
                }
                PdfImageScalarPhase::DepthDimension => {
                    let result = self.scan_dimension_retained();
                    progress.depth = Some(result.into_result()?.value);
                    progress.phase = PdfImageScalarPhase::WidthKeyword;
                }
                PdfImageScalarPhase::AttributeKeyword => {
                    let result = self.scan_keyword_retained("attr");
                    if result.into_result()?.value {
                        let attr = self.scan_pdf_navigation_text()?;
                        progress.attr = Some(attr.tokens);
                    }
                    progress.phase = PdfImageScalarPhase::NamedKeyword;
                }
                PdfImageScalarPhase::NamedKeyword => {
                    let result = self.scan_keyword_retained("named");
                    if result.into_result()?.value {
                        let text = self.scan_pdf_navigation_text()?;
                        progress.page = PendingPdfImagePage::Named(text.tokens);
                        progress.phase = PdfImageScalarPhase::ColorSpaceKeyword;
                    } else {
                        progress.phase = PdfImageScalarPhase::PageKeyword;
                    }
                }
                PdfImageScalarPhase::PageKeyword => {
                    let result = self.scan_keyword_retained("page");
                    if result.into_result()?.value {
                        progress.phase = PdfImageScalarPhase::PageNumber;
                    } else {
                        progress.page = PendingPdfImagePage::Number(1);
                        progress.phase = PdfImageScalarPhase::ColorSpaceKeyword;
                    }
                }
                PdfImageScalarPhase::PageNumber => {
                    let result = self.scan_integer_retained();
                    progress.page = PendingPdfImagePage::Number(result.into_result()?.value);
                    progress.phase = PdfImageScalarPhase::ColorSpaceKeyword;
                }
                PdfImageScalarPhase::ColorSpaceKeyword => {
                    let result = self.scan_keyword_retained("colorspace");
                    progress.phase = if result.into_result()?.value {
                        PdfImageScalarPhase::ColorSpaceObject
                    } else {
                        PdfImageScalarPhase::MediaBox
                    };
                }
                PdfImageScalarPhase::ColorSpaceObject => {
                    let result = self.scan_integer_retained();
                    progress.color_space_object = result.into_result()?.value;
                    progress.phase = PdfImageScalarPhase::MediaBox;
                }
                PdfImageScalarPhase::MediaBox
                | PdfImageScalarPhase::CropBox
                | PdfImageScalarPhase::BleedBox
                | PdfImageScalarPhase::TrimBox
                | PdfImageScalarPhase::ArtBox => {
                    let (keyword, selected, next) = match progress.phase {
                        PdfImageScalarPhase::MediaBox => (
                            "mediabox",
                            PdfImagePageBox::Media,
                            PdfImageScalarPhase::CropBox,
                        ),
                        PdfImageScalarPhase::CropBox => (
                            "cropbox",
                            PdfImagePageBox::Crop,
                            PdfImageScalarPhase::BleedBox,
                        ),
                        PdfImageScalarPhase::BleedBox => (
                            "bleedbox",
                            PdfImagePageBox::Bleed,
                            PdfImageScalarPhase::TrimBox,
                        ),
                        PdfImageScalarPhase::TrimBox => (
                            "trimbox",
                            PdfImagePageBox::Trim,
                            PdfImageScalarPhase::ArtBox,
                        ),
                        PdfImageScalarPhase::ArtBox => (
                            "artbox",
                            PdfImagePageBox::Art,
                            PdfImageScalarPhase::FileName,
                        ),
                        _ => unreachable!(),
                    };
                    let result = self.scan_keyword_retained(keyword);
                    if result.into_result()?.value {
                        progress.page_box = Some(selected);
                        progress.phase = PdfImageScalarPhase::FileName;
                    } else {
                        progress.phase = next;
                    }
                }
                PdfImageScalarPhase::FileName => {
                    let result = self.scan_file_name_retained();
                    let name = result.into_result()?.packed();
                    let page = match progress.page {
                        PendingPdfImagePage::Unset => PdfImagePageSelection::Number(1),
                        PendingPdfImagePage::Number(page) => PdfImagePageSelection::Number(page),
                        PendingPdfImagePage::Named(tokens) => {
                            let semantic = self
                                .command
                                .attempt
                                .arena()
                                .token_words(tokens)
                                .map_err(|_| CommandError::input_invariant())?
                                .iter()
                                .map(|word| word.semantic_token())
                                .collect::<Vec<_>>();
                            PdfImagePageSelection::Named(
                                crate::processor::expand_render::token_slice_string_text(
                                    self.state, &semantic,
                                )
                                .into_bytes(),
                            )
                        }
                    };
                    let page_box = progress.page_box;
                    return Ok(PdfImageRequest {
                        name,
                        width: progress.width,
                        height: progress.height,
                        depth: progress.depth,
                        page,
                        color_space_object: progress.color_space_object,
                        page_box_explicit: page_box.is_some(),
                        page_box: page_box.unwrap_or(PdfImagePageBox::Crop),
                        resolution: 0,
                        attr: progress.attr,
                    });
                }
            }
        }
    }
}
