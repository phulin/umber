use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    fn scan_pdf_action_owned_text(&mut self) -> Result<ScannedBalancedText, CommandError> {
        self.scan_balanced_text(true)
    }

    pub(super) fn scan_pdf_action_for_owner(
        &mut self,
        owner: PendingPdfActionOwner,
    ) -> Result<(PendingPdfActionOwner, PdfActionSpec), CommandError> {
        let progress = PdfActionScalarProgress {
            goto: None,
            file: None,
            structure: None,
            target: None,
            phase: PdfActionScalarPhase::UserKeyword,
        };
        self.scan_pdf_action_scalar(owner, progress)
    }

    fn scan_pdf_action_scalar(
        &mut self,
        owner: PendingPdfActionOwner,
        mut progress: PdfActionScalarProgress,
    ) -> Result<(PendingPdfActionOwner, PdfActionSpec), CommandError> {
        use tex_state::PdfActionWindow;
        loop {
            match progress.phase {
                PdfActionScalarPhase::UserKeyword => {
                    let result = self.scan_keyword_retained("user");
                    if result.into_result()?.value {
                        let text = self.scan_pdf_action_owned_text()?;
                        return Ok((owner, PdfActionSpec::User(text.tokens)));
                    }
                    progress.phase = PdfActionScalarPhase::GotoKeyword;
                }
                PdfActionScalarPhase::GotoKeyword => {
                    let result = self.scan_keyword_retained("goto");
                    if result.into_result()?.value {
                        progress.goto = Some(true);
                        progress.phase = PdfActionScalarPhase::FileKeyword;
                    } else {
                        progress.phase = PdfActionScalarPhase::ThreadKeyword;
                    }
                }
                PdfActionScalarPhase::ThreadKeyword => {
                    let result = self.scan_keyword_retained("thread");
                    if !result.into_result()?.value {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): action type missing",
                        ));
                    }
                    progress.goto = Some(false);
                    progress.phase = PdfActionScalarPhase::FileKeyword;
                }
                PdfActionScalarPhase::FileKeyword => {
                    let result = self.scan_keyword_retained("file");
                    if result.into_result()?.value {
                        let _ = progress.goto.ok_or(CommandError::input_invariant())?;
                        progress.file = Some(self.scan_pdf_action_owned_text()?.tokens);
                    }
                    progress.phase = PdfActionScalarPhase::StructureKeyword;
                }
                PdfActionScalarPhase::StructureKeyword => {
                    let result = self.scan_keyword_retained("struct");
                    if result.into_result()?.value {
                        let goto = progress.goto.ok_or(CommandError::input_invariant())?;
                        if !goto {
                            return Err(CommandError::PdfNavigation(
                                "pdfTeX error (ext1): only GoTo action can be used with `struct'",
                            ));
                        }
                        if progress.file.is_some() {
                            progress.structure = Some(PdfActionIdentifier::Raw(
                                self.scan_pdf_action_owned_text()?.tokens,
                            ));
                            progress.phase = PdfActionScalarPhase::PageKeyword;
                        } else {
                            progress.phase = PdfActionScalarPhase::StructureNameKeyword;
                        }
                    } else {
                        progress.phase = PdfActionScalarPhase::PageKeyword;
                    }
                }
                PdfActionScalarPhase::StructureNameKeyword => {
                    let result = self.scan_keyword_retained("name");
                    if result.into_result()?.value {
                        let _ = progress.goto.ok_or(CommandError::input_invariant())?;
                        progress.structure = Some(PdfActionIdentifier::Name(
                            self.scan_pdf_action_owned_text()?.tokens,
                        ));
                        progress.phase = PdfActionScalarPhase::PageKeyword;
                    } else {
                        progress.phase = PdfActionScalarPhase::StructureNumberKeyword;
                    }
                }
                PdfActionScalarPhase::StructureNumberKeyword => {
                    let result = self.scan_keyword_retained("num");
                    if !result.into_result()?.value {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): identifier type missing",
                        ));
                    }
                    progress.phase = PdfActionScalarPhase::StructureNumber;
                }
                PdfActionScalarPhase::StructureNumber => {
                    let result = self.scan_integer_retained();
                    let value = result.into_result()?.value;
                    progress.structure = Some(PdfActionIdentifier::Number(
                        Self::finish_pdf_positive(value, "struct identifier", false)?,
                    ));
                    progress.phase = PdfActionScalarPhase::PageKeyword;
                }
                PdfActionScalarPhase::PageKeyword => {
                    let result = self.scan_keyword_retained("page");
                    if result.into_result()?.value {
                        if !progress.goto.ok_or(CommandError::input_invariant())? {
                            return Err(CommandError::PdfNavigation(
                                "pdfTeX error (ext1): only GoTo action can be used with `page'",
                            ));
                        }
                        progress.phase = PdfActionScalarPhase::PageNumber;
                    } else {
                        progress.phase = PdfActionScalarPhase::NameKeyword;
                    }
                }
                PdfActionScalarPhase::PageNumber => {
                    let result = self.scan_integer_retained();
                    let number = Self::finish_pdf_positive(
                        result.into_result()?.value,
                        "page number",
                        false,
                    )?;
                    let _ = progress.goto.ok_or(CommandError::input_invariant())?;
                    let view = self.scan_pdf_action_owned_text()?.tokens;
                    progress.target = Some(PdfActionTarget::Page { number, view });
                    progress.phase = PdfActionScalarPhase::NewWindowKeyword;
                }
                PdfActionScalarPhase::NameKeyword => {
                    let result = self.scan_keyword_retained("name");
                    if result.into_result()?.value {
                        let _ = progress.goto.ok_or(CommandError::input_invariant())?;
                        let name = self.scan_pdf_action_owned_text()?.tokens;
                        progress.target = Some(PdfActionTarget::Destination(
                            PdfActionIdentifier::Name(name),
                        ));
                        progress.phase = PdfActionScalarPhase::NewWindowKeyword;
                    } else {
                        progress.phase = PdfActionScalarPhase::NumberKeyword;
                    }
                }
                PdfActionScalarPhase::NumberKeyword => {
                    let result = self.scan_keyword_retained("num");
                    if !result.into_result()?.value {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): identifier type missing",
                        ));
                    }
                    let goto = progress.goto.ok_or(CommandError::input_invariant())?;
                    if goto && progress.file.is_some() {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): `goto' option cannot be used with both `file' and `num'",
                        ));
                    }
                    progress.phase = PdfActionScalarPhase::Number;
                }
                PdfActionScalarPhase::Number => {
                    let result = self.scan_integer_retained();
                    let value = Self::finish_pdf_positive(
                        result.into_result()?.value,
                        "num identifier",
                        false,
                    )?;
                    progress.target = Some(PdfActionTarget::Destination(
                        PdfActionIdentifier::Number(value),
                    ));
                    progress.phase = PdfActionScalarPhase::NewWindowKeyword;
                }
                PdfActionScalarPhase::NewWindowKeyword => {
                    let result = self.scan_keyword_retained("newwindow");
                    if result.into_result()?.value {
                        return self.finish_pdf_action(owner, progress, PdfActionWindow::New);
                    }
                    progress.phase = PdfActionScalarPhase::NoNewWindowKeyword;
                }
                PdfActionScalarPhase::NoNewWindowKeyword => {
                    let result = self.scan_keyword_retained("nonewwindow");
                    let same = result.into_result()?.value;
                    return self.finish_pdf_action(
                        owner,
                        progress,
                        if same {
                            PdfActionWindow::Same
                        } else {
                            PdfActionWindow::Unspecified
                        },
                    );
                }
            }
        }
    }

    fn finish_pdf_action(
        &self,
        owner: PendingPdfActionOwner,
        progress: PdfActionScalarProgress,
        window: tex_state::PdfActionWindow,
    ) -> Result<(PendingPdfActionOwner, PdfActionSpec), CommandError> {
        let goto = progress.goto.ok_or(CommandError::input_invariant())?;
        if window != tex_state::PdfActionWindow::Unspecified && (!goto || progress.file.is_none()) {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): `newwindow'/`nonewwindow' must be used with `goto' and `file' option",
            ));
        }
        let action = PdfActionDestination {
            file: progress.file,
            structure: progress.structure,
            target: progress.target.ok_or(CommandError::input_invariant())?,
            window,
        };
        Ok((
            owner,
            if goto {
                PdfActionSpec::GoTo(action)
            } else {
                PdfActionSpec::Thread(action)
            },
        ))
    }
}
