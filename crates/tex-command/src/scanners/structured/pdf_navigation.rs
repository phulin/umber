use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    pub(super) fn scan_pdf_navigation_text(&mut self) -> Result<ScannedBalancedText, CommandError> {
        self.scan_balanced_text(true)
    }

    fn finish_pdf_outline(
        &mut self,
        attributes: Option<ScannedBalancedText>,
        action: PdfActionSpec,
    ) -> Result<PdfNavigationRequest, CommandError> {
        let result = self.scan_keyword_retained("count");
        let count = if result.into_result()?.value {
            let result = self.scan_integer_retained();
            result.into_result()?.value
        } else {
            0
        };
        let title = self.scan_pdf_navigation_text()?;
        Ok(PdfNavigationRequest::Outline(PdfOutlineRequest {
            attributes,
            action,
            count,
            title,
        }))
    }

    fn scan_pdf_thread_identifier_owned(
        &mut self,
        primitive: UnexpandablePrimitive,
        dimensions: tex_state::PdfAnnotationDimensions,
        attributes: Option<ScannedBalancedText>,
    ) -> Result<PdfNavigationRequest, CommandError> {
        let result = self.scan_keyword_retained("name");
        let name = result.into_result()?.value;
        if name {
            let text = self.scan_pdf_navigation_text()?;
            return Ok(PdfNavigationRequest::Thread(PdfThreadRequest {
                dimensions,
                attributes,
                identifier: PdfActionIdentifier::Name(text.tokens),
                running: primitive == UnexpandablePrimitive::PdfStartThread,
            }));
        }

        let result = self.scan_keyword_retained("num");
        let num = result.into_result()?.value;
        let identifier = if num {
            let result = self.scan_integer_retained();
            let value = result.into_result()?.value;
            PdfActionIdentifier::Number(Self::finish_pdf_positive(
                value,
                "thread identifier",
                true,
            )?)
        } else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext4): thread identifier type missing",
            ));
        };
        Ok(PdfNavigationRequest::Thread(PdfThreadRequest {
            dimensions,
            attributes,
            identifier,
            running: primitive == UnexpandablePrimitive::PdfStartThread,
        }))
    }

    /// Scans the pdfTeX annotation/link/destination/thread family (pdftex.web
    /// 34847--35208).  `scan_alt_rule` deliberately resets all dimensions on
    /// each invocation and accepts repeated fields, with the last one winning.
    pub fn scan_pdf_navigation_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfNavigationRequest, CommandError> {
        use PdfNavigationRequest as Request;

        let phase = match primitive {
            UnexpandablePrimitive::PdfAnnot => PdfNavigationScalarPhase::AnnotationReserve,
            UnexpandablePrimitive::PdfStartLink
            | UnexpandablePrimitive::PdfThread
            | UnexpandablePrimitive::PdfStartThread => PdfNavigationScalarPhase::WidthKeyword,
            UnexpandablePrimitive::PdfOutline => PdfNavigationScalarPhase::AttributeKeyword,
            UnexpandablePrimitive::PdfDest => PdfNavigationScalarPhase::DestinationStructure,
            UnexpandablePrimitive::PdfEndLink => return Ok(Request::EndLink),
            UnexpandablePrimitive::PdfEndThread => return Ok(Request::EndThread),
            _ => return Err(CommandError::input_invariant()),
        };
        self.scan_pdf_navigation_scalar(PdfNavigationScalarProgress {
            primitive,
            use_object: None,
            dimensions: tex_state::PdfAnnotationDimensions::RUNNING,
            attributes: None,
            structure: None,
            identifier: None,
            phase,
        })
    }

    fn scan_pdf_navigation_scalar(
        &mut self,
        mut progress: PdfNavigationScalarProgress,
    ) -> Result<PdfNavigationRequest, CommandError> {
        use tex_state::node::PdfDestinationKind as Kind;
        loop {
            match progress.phase {
                PdfNavigationScalarPhase::AnnotationReserve => {
                    let result = self.scan_keyword_retained("reserveobjnum");
                    if result.into_result()?.value {
                        return Ok(PdfNavigationRequest::Annotation(
                            PdfAnnotationRequest::Reserve,
                        ));
                    }
                    progress.phase = PdfNavigationScalarPhase::AnnotationUse;
                }
                PdfNavigationScalarPhase::AnnotationUse => {
                    let result = self.scan_keyword_retained("useobjnum");
                    progress.phase = if result.into_result()?.value {
                        PdfNavigationScalarPhase::AnnotationUseObject
                    } else {
                        PdfNavigationScalarPhase::WidthKeyword
                    };
                }
                PdfNavigationScalarPhase::AnnotationUseObject => {
                    let result = self.scan_integer_retained();
                    progress.use_object = Some(result.into_result()?.value);
                    progress.phase = PdfNavigationScalarPhase::WidthKeyword;
                }
                PdfNavigationScalarPhase::WidthKeyword
                | PdfNavigationScalarPhase::FitRWidthKeyword => {
                    let result = self.scan_keyword_retained("width");
                    let fitr = progress.phase == PdfNavigationScalarPhase::FitRWidthKeyword;
                    progress.phase = if result.into_result()?.value {
                        if fitr {
                            PdfNavigationScalarPhase::FitRWidthDimension
                        } else {
                            PdfNavigationScalarPhase::WidthDimension
                        }
                    } else if fitr {
                        PdfNavigationScalarPhase::FitRHeightKeyword
                    } else {
                        PdfNavigationScalarPhase::HeightKeyword
                    };
                }
                PdfNavigationScalarPhase::WidthDimension
                | PdfNavigationScalarPhase::FitRWidthDimension => {
                    let result = self.scan_dimension_retained();
                    progress.dimensions.width = Some(result.into_result()?.value);
                    progress.phase =
                        if progress.phase == PdfNavigationScalarPhase::FitRWidthDimension {
                            PdfNavigationScalarPhase::FitRWidthKeyword
                        } else {
                            PdfNavigationScalarPhase::WidthKeyword
                        };
                }
                PdfNavigationScalarPhase::HeightKeyword
                | PdfNavigationScalarPhase::FitRHeightKeyword => {
                    let result = self.scan_keyword_retained("height");
                    let fitr = progress.phase == PdfNavigationScalarPhase::FitRHeightKeyword;
                    progress.phase = if result.into_result()?.value {
                        if fitr {
                            PdfNavigationScalarPhase::FitRHeightDimension
                        } else {
                            PdfNavigationScalarPhase::HeightDimension
                        }
                    } else if fitr {
                        PdfNavigationScalarPhase::FitRDepthKeyword
                    } else {
                        PdfNavigationScalarPhase::DepthKeyword
                    };
                }
                PdfNavigationScalarPhase::HeightDimension
                | PdfNavigationScalarPhase::FitRHeightDimension => {
                    let result = self.scan_dimension_retained();
                    progress.dimensions.height = Some(result.into_result()?.value);
                    progress.phase =
                        if progress.phase == PdfNavigationScalarPhase::FitRHeightDimension {
                            PdfNavigationScalarPhase::FitRWidthKeyword
                        } else {
                            PdfNavigationScalarPhase::WidthKeyword
                        };
                }
                PdfNavigationScalarPhase::DepthKeyword
                | PdfNavigationScalarPhase::FitRDepthKeyword => {
                    let result = self.scan_keyword_retained("depth");
                    let fitr = progress.phase == PdfNavigationScalarPhase::FitRDepthKeyword;
                    if result.into_result()?.value {
                        progress.phase = if fitr {
                            PdfNavigationScalarPhase::FitRDepthDimension
                        } else {
                            PdfNavigationScalarPhase::DepthDimension
                        };
                    } else if fitr {
                        let dimensions = progress.dimensions;
                        return self
                            .finish_pdf_destination(progress, Kind::FitRectangle(dimensions));
                    } else {
                        match progress.primitive {
                            UnexpandablePrimitive::PdfAnnot => {
                                let entries = self.scan_pdf_navigation_text()?;
                                return Ok(PdfNavigationRequest::Annotation(
                                    PdfAnnotationRequest::Define {
                                        use_object: progress.use_object,
                                        dimensions: progress.dimensions,
                                        entries,
                                    },
                                ));
                            }
                            UnexpandablePrimitive::PdfStartLink
                            | UnexpandablePrimitive::PdfThread
                            | UnexpandablePrimitive::PdfStartThread => {
                                progress.phase = PdfNavigationScalarPhase::AttributeKeyword;
                            }
                            _ => return Err(CommandError::input_invariant()),
                        }
                    }
                }
                PdfNavigationScalarPhase::DepthDimension
                | PdfNavigationScalarPhase::FitRDepthDimension => {
                    let result = self.scan_dimension_retained();
                    progress.dimensions.depth = Some(result.into_result()?.value);
                    progress.phase =
                        if progress.phase == PdfNavigationScalarPhase::FitRDepthDimension {
                            PdfNavigationScalarPhase::FitRWidthKeyword
                        } else {
                            PdfNavigationScalarPhase::WidthKeyword
                        };
                }
                PdfNavigationScalarPhase::AttributeKeyword => {
                    let result = self.scan_keyword_retained("attr");
                    let has_attr = result.into_result()?.value;
                    match progress.primitive {
                        UnexpandablePrimitive::PdfStartLink => {
                            if has_attr {
                                progress.attributes = Some(self.scan_pdf_navigation_text()?);
                            }
                            let (owner, action) =
                                self.scan_pdf_action_for_owner(PendingPdfActionOwner::StartLink {
                                    dimensions: progress.dimensions,
                                    attributes: progress.attributes,
                                })?;
                            let PendingPdfActionOwner::StartLink {
                                dimensions,
                                attributes,
                            } = owner
                            else {
                                return Err(CommandError::input_invariant());
                            };
                            return Ok(PdfNavigationRequest::StartLink(PdfStartLinkRequest {
                                dimensions,
                                attributes,
                                action,
                            }));
                        }
                        UnexpandablePrimitive::PdfOutline => {
                            if has_attr {
                                progress.attributes = Some(self.scan_pdf_navigation_text()?);
                            }
                            let (owner, action) =
                                self.scan_pdf_action_for_owner(PendingPdfActionOwner::Outline {
                                    attributes: progress.attributes,
                                })?;
                            let PendingPdfActionOwner::Outline { attributes } = owner else {
                                return Err(CommandError::input_invariant());
                            };
                            return self.finish_pdf_outline(attributes, action);
                        }
                        primitive @ (UnexpandablePrimitive::PdfThread
                        | UnexpandablePrimitive::PdfStartThread) => {
                            if has_attr {
                                progress.attributes = Some(self.scan_pdf_navigation_text()?);
                            }
                            return self.scan_pdf_thread_identifier_owned(
                                primitive,
                                progress.dimensions,
                                progress.attributes,
                            );
                        }
                        _ => return Err(CommandError::input_invariant()),
                    }
                }
                PdfNavigationScalarPhase::DestinationStructure => {
                    let result = self.scan_keyword_retained("struct");
                    progress.phase = if result.into_result()?.value {
                        PdfNavigationScalarPhase::DestinationStructureValue
                    } else {
                        PdfNavigationScalarPhase::DestinationName
                    };
                }
                PdfNavigationScalarPhase::DestinationStructureValue => {
                    let result = self.scan_integer_retained();
                    progress.structure = Some(Self::finish_pdf_positive(
                        result.into_result()?.value,
                        "struct identifier",
                        false,
                    )?);
                    progress.phase = PdfNavigationScalarPhase::DestinationName;
                }
                PdfNavigationScalarPhase::DestinationName => {
                    let result = self.scan_keyword_retained("name");
                    if result.into_result()?.value {
                        let identifier = self.scan_pdf_navigation_text()?;
                        progress.identifier = Some(PdfActionIdentifier::Name(identifier.tokens));
                        progress.phase = PdfNavigationScalarPhase::DestinationXyz;
                    } else {
                        progress.phase = PdfNavigationScalarPhase::DestinationNumber;
                    }
                }
                PdfNavigationScalarPhase::DestinationNumber => {
                    let result = self.scan_keyword_retained("num");
                    if !result.into_result()?.value {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): identifier type missing",
                        ));
                    }
                    progress.phase = PdfNavigationScalarPhase::DestinationNumberValue;
                }
                PdfNavigationScalarPhase::DestinationNumberValue => {
                    let result = self.scan_integer_retained();
                    progress.identifier =
                        Some(PdfActionIdentifier::Number(Self::finish_pdf_positive(
                            result.into_result()?.value,
                            "destination identifier",
                            true,
                        )?));
                    progress.phase = PdfNavigationScalarPhase::DestinationXyz;
                }
                PdfNavigationScalarPhase::DestinationXyz => {
                    let result = self.scan_keyword_retained("xyz");
                    if result.into_result()?.value {
                        progress.phase = PdfNavigationScalarPhase::DestinationZoom;
                    } else {
                        progress.phase = PdfNavigationScalarPhase::DestinationFitBh;
                    }
                }
                PdfNavigationScalarPhase::DestinationZoom => {
                    let result = self.scan_keyword_retained("zoom");
                    if result.into_result()?.value {
                        progress.phase = PdfNavigationScalarPhase::DestinationZoomValue;
                    } else {
                        return self.finish_pdf_destination(progress, Kind::Xyz { zoom: None });
                    }
                }
                PdfNavigationScalarPhase::DestinationZoomValue => {
                    let result = self.scan_integer_retained();
                    let zoom = result.into_result()?.value;
                    if zoom > 1_073_741_823 {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): number too big",
                        ));
                    }
                    return self.finish_pdf_destination(progress, Kind::Xyz { zoom: Some(zoom) });
                }
                PdfNavigationScalarPhase::DestinationFitBh
                | PdfNavigationScalarPhase::DestinationFitBv
                | PdfNavigationScalarPhase::DestinationFitB
                | PdfNavigationScalarPhase::DestinationFitH
                | PdfNavigationScalarPhase::DestinationFitV
                | PdfNavigationScalarPhase::DestinationFitR
                | PdfNavigationScalarPhase::DestinationFit => {
                    let (keyword, kind, next) = match progress.phase {
                        PdfNavigationScalarPhase::DestinationFitBh => (
                            "fitbh",
                            Some(Kind::FitBoundingBoxHorizontal),
                            PdfNavigationScalarPhase::DestinationFitBv,
                        ),
                        PdfNavigationScalarPhase::DestinationFitBv => (
                            "fitbv",
                            Some(Kind::FitBoundingBoxVertical),
                            PdfNavigationScalarPhase::DestinationFitB,
                        ),
                        PdfNavigationScalarPhase::DestinationFitB => (
                            "fitb",
                            Some(Kind::FitBoundingBox),
                            PdfNavigationScalarPhase::DestinationFitH,
                        ),
                        PdfNavigationScalarPhase::DestinationFitH => (
                            "fith",
                            Some(Kind::FitHorizontal),
                            PdfNavigationScalarPhase::DestinationFitV,
                        ),
                        PdfNavigationScalarPhase::DestinationFitV => (
                            "fitv",
                            Some(Kind::FitVertical),
                            PdfNavigationScalarPhase::DestinationFitR,
                        ),
                        PdfNavigationScalarPhase::DestinationFitR => {
                            ("fitr", None, PdfNavigationScalarPhase::DestinationFit)
                        }
                        PdfNavigationScalarPhase::DestinationFit => (
                            "fit",
                            Some(Kind::Fit),
                            PdfNavigationScalarPhase::DestinationFit,
                        ),
                        _ => unreachable!(),
                    };
                    let result = self.scan_keyword_retained(keyword);
                    if result.into_result()?.value {
                        if progress.phase == PdfNavigationScalarPhase::DestinationFitR {
                            progress.dimensions = tex_state::PdfAnnotationDimensions::RUNNING;
                            progress.phase = PdfNavigationScalarPhase::FitRWidthKeyword;
                        } else {
                            return self.finish_pdf_destination(
                                progress,
                                kind.expect("non-fitr destination has a kind"),
                            );
                        }
                    } else if progress.phase == PdfNavigationScalarPhase::DestinationFit {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): destination type missing",
                        ));
                    } else {
                        progress.phase = next;
                    }
                }
            }
        }
    }

    fn finish_pdf_destination(
        &self,
        progress: PdfNavigationScalarProgress,
        kind: tex_state::node::PdfDestinationKind,
    ) -> Result<PdfNavigationRequest, CommandError> {
        Ok(PdfNavigationRequest::Destination(PdfDestinationRequest {
            structure: progress.structure,
            identifier: progress.identifier.ok_or(CommandError::input_invariant())?,
            kind,
        }))
    }

    pub(super) fn finish_pdf_positive(
        value: i32,
        kind: &'static str,
        bounded_by_halfword: bool,
    ) -> Result<u32, CommandError> {
        if value <= 0 {
            return Err(CommandError::PdfNavigation(match kind {
                "struct identifier" => "pdfTeX error (ext1): struct identifier must be positive",
                "page number" => "pdfTeX error (ext1): page number must be positive",
                _ => "pdfTeX error (ext1): num identifier must be positive",
            }));
        }
        if bounded_by_halfword && value > 1_073_741_823 {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): number too big",
            ));
        }
        Ok(value as u32)
    }
}
