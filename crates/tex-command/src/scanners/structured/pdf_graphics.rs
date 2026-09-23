use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    /// Scans the unexpandable pdfTeX graphics whatsit family.
    ///
    /// This follows pdftex.web's `pdfliteral` through `pdfsnapycomp` scanners:
    /// `shipout` is recognized before the literal mode, immediate literals
    /// and setters expand their balanced text now, and a shipout literal
    /// retains its unexpanded token list for traversal-time expansion.
    pub fn scan_pdf_graphics_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<Option<PdfGraphicsRequest>, CommandError> {
        use PdfGraphicsRequest as Request;

        let request = match primitive {
            UnexpandablePrimitive::PdfLiteral => {
                return self
                    .scan_pdf_graphics_scalar(primitive, PdfGraphicsScalarPhase::LiteralShipout);
            }
            UnexpandablePrimitive::PdfSetMatrix => Request::SetMatrix {
                text: self.scan_balanced_text(true)?,
            },
            UnexpandablePrimitive::PdfSave => Request::Save,
            UnexpandablePrimitive::PdfRestore => Request::Restore,
            UnexpandablePrimitive::PdfColorStack => {
                return self.scan_pdf_graphics_scalar(primitive, PdfGraphicsScalarPhase::ColorId);
            }
            UnexpandablePrimitive::PdfSavePos => Request::SavePosition,
            UnexpandablePrimitive::PdfSnapRefPoint => Request::SnapReferencePoint,
            UnexpandablePrimitive::PdfSnapY => {
                return self.scan_pdf_graphics_scalar(primitive, PdfGraphicsScalarPhase::SnapY);
            }
            UnexpandablePrimitive::PdfSnapYComp => {
                return self.scan_pdf_graphics_scalar(primitive, PdfGraphicsScalarPhase::SnapYComp);
            }
            _ => return Ok(None),
        };
        Ok(Some(request))
    }

    fn scan_pdf_graphics_scalar(
        &mut self,
        _primitive: UnexpandablePrimitive,
        mut phase: PdfGraphicsScalarPhase,
    ) -> Result<Option<PdfGraphicsRequest>, CommandError> {
        use PdfColorStackActionRequest as Action;
        use PdfGraphicsRequest as Request;
        loop {
            match phase {
                PdfGraphicsScalarPhase::LiteralShipout => {
                    let result = self.scan_keyword_retained("shipout");
                    let deferred = result.into_result()?.value;
                    phase = PdfGraphicsScalarPhase::LiteralDirect { deferred };
                }
                PdfGraphicsScalarPhase::LiteralDirect { deferred } => {
                    let result = self.scan_keyword_retained("direct");
                    if result.into_result()?.value {
                        return self.finish_pdf_graphics_literal(
                            tex_state::node::PdfLiteralMode::Direct,
                            deferred,
                        );
                    }
                    phase = PdfGraphicsScalarPhase::LiteralPage { deferred };
                }
                PdfGraphicsScalarPhase::LiteralPage { deferred } => {
                    let result = self.scan_keyword_retained("page");
                    let page = result.into_result()?.value;
                    let mode = if page {
                        tex_state::node::PdfLiteralMode::Page
                    } else {
                        tex_state::node::PdfLiteralMode::Origin
                    };
                    return self.finish_pdf_graphics_literal(mode, deferred);
                }
                PdfGraphicsScalarPhase::ColorId => {
                    let result = self.scan_integer_retained();
                    let id = result.into_result()?.value;
                    phase = PdfGraphicsScalarPhase::ColorSet { id };
                }
                PdfGraphicsScalarPhase::ColorSet { id } => {
                    let result = self.scan_keyword_retained("set");
                    if result.into_result()?.value {
                        return self
                            .finish_pdf_color_stack_text(id, PendingPdfColorStackAction::Set);
                    }
                    phase = PdfGraphicsScalarPhase::ColorPush { id };
                }
                PdfGraphicsScalarPhase::ColorPush { id } => {
                    let result = self.scan_keyword_retained("push");
                    if result.into_result()?.value {
                        return self
                            .finish_pdf_color_stack_text(id, PendingPdfColorStackAction::Push);
                    }
                    phase = PdfGraphicsScalarPhase::ColorPop { id };
                }
                PdfGraphicsScalarPhase::ColorPop { id } => {
                    let result = self.scan_keyword_retained("pop");
                    if result.into_result()?.value {
                        return Ok(Some(Request::ColorStack {
                            id,
                            action: Some(Action::Pop),
                        }));
                    }
                    phase = PdfGraphicsScalarPhase::ColorCurrent { id };
                }
                PdfGraphicsScalarPhase::ColorCurrent { id } => {
                    let result = self.scan_keyword_retained("current");
                    let current = result.into_result()?.value;
                    return Ok(Some(Request::ColorStack {
                        id,
                        action: current.then_some(Action::Current),
                    }));
                }
                PdfGraphicsScalarPhase::SnapY => {
                    let result = self.scan_glue_retained(false);
                    let glue = result.into_result()?.value;
                    return Ok(Some(Request::SnapY { glue }));
                }
                PdfGraphicsScalarPhase::SnapYComp => {
                    let result = self.scan_integer_retained();
                    let ratio = result.into_result()?.value.clamp(0, 1000) as u16;
                    return Ok(Some(Request::SnapYComp { ratio }));
                }
            }
        }
    }

    fn finish_pdf_graphics_literal(
        &mut self,
        mode: tex_state::node::PdfLiteralMode,
        deferred: bool,
    ) -> Result<Option<PdfGraphicsRequest>, CommandError> {
        match self.scan_balanced_text(!deferred) {
            Ok(text) => Ok(Some(PdfGraphicsRequest::Literal {
                mode,
                deferred,
                text,
            })),
            Err(error) => Err(error),
        }
    }

    fn finish_pdf_color_stack_text(
        &mut self,
        id: i32,
        action: PendingPdfColorStackAction,
    ) -> Result<Option<PdfGraphicsRequest>, CommandError> {
        match self.scan_balanced_text(true) {
            Ok(text) => Ok(Some(PdfGraphicsRequest::ColorStack {
                id,
                action: Some(match action {
                    PendingPdfColorStackAction::Set => PdfColorStackActionRequest::Set(text),
                    PendingPdfColorStackAction::Push => PdfColorStackActionRequest::Push(text),
                }),
            })),
            Err(error) => Err(error),
        }
    }
}
