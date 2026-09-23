use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    /// Scans TeX82 §1350's `new_write_whatsit` stream number for a
    /// `write_node_size` extension.
    ///
    /// `new_write_whatsit` normalizes the scanned number *before* it reaches
    /// `write_stream(tail)`:
    ///
    /// ```text
    /// else begin scan_int;
    ///   if cur_val<0 then cur_val:=17
    ///   else if cur_val>15 then cur_val:=16;
    ///   end;
    /// write_stream(tail):=cur_val;
    /// ```
    ///
    /// §1342 explains the two extra slots: `write_open[16]` stands for every
    /// stream number above 15 and `write_open[17]` for every negative one, so
    /// the recorded stream is always in `0..=17`. This is deliberately *not*
    /// §433's `scan_four_bit_int`, which `new_write_whatsit` uses only for
    /// the `open_node_size` case (`\openout`) and which reports "Bad number"
    /// and recovers as stream zero instead.
    pub fn scan_write_stream(&mut self) -> Result<WriteStreamSelector, CommandError> {
        let result = self.scan_integer_retained();
        let value = result.into_result()?.value;
        Ok(if value < 0 {
            WriteStreamSelector::Negative
        } else if value > 15 {
            WriteStreamSelector::AboveRange
        } else {
            WriteStreamSelector::Stream(value as u8)
        })
    }

    /// Scans TeX82 §53's one-token `\immediate` extension execution.
    ///
    /// `do_extension` calls `get_x_token`, executes only `openout`, `write`,
    /// and `closeout`, and backs every other expanded command up for ordinary
    /// main control.  The integer, optional-equals, filename, and write-text
    /// scans remain in this command-owned episode.
    pub fn scan_immediate_extension(
        &mut self,
        pdf_output_enabled: bool,
    ) -> Result<ImmediateExtension, CommandError> {
        let mut destination = None;
        let command = loop {
            if self.request_expanded_token(&mut destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let command = destination.take().ok_or(CommandError::input_invariant())?;
            if !matches!(
                static_meaning(command.meaning()),
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                break command;
            }
        };
        match static_meaning(command.meaning()) {
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OpenOut)) => {
                self.finish_immediate_open_out()
            }
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write)) => {
                let stream = self.scan_immediate_write_stream_selector()?;
                self.finish_immediate_write(stream)
            }
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut)) => {
                let stream = self.scan_immediate_write_stream_selector()?;
                Ok(ImmediateExtension::CloseOut { stream })
            }
            Some(Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::PdfObject
                | UnexpandablePrimitive::PdfXForm
                | UnexpandablePrimitive::PdfXImage),
            )) => self.finish_immediate_pdf(command, primitive, pdf_output_enabled),
            _ => {
                self.back_input(command)?;
                Ok(ImmediateExtension::Continue)
            }
        }
    }

    fn scan_immediate_write_stream_selector(
        &mut self,
    ) -> Result<WriteStreamSelector, CommandError> {
        let result = self.scan_integer_retained();
        let value = result.into_result()?.value;
        Ok(if value < 0 {
            WriteStreamSelector::Negative
        } else if value > 15 {
            WriteStreamSelector::AboveRange
        } else {
            WriteStreamSelector::Stream(value as u8)
        })
    }

    fn finish_immediate_write(
        &mut self,
        stream: WriteStreamSelector,
    ) -> Result<ImmediateExtension, CommandError> {
        // TeX82 §53 first saves write text without expansion, then
        // `write_out` replays it under an outer `\\endwrite` stopper.
        let tokens = match self.scan_immediate_write_text() {
            Ok(tokens) => tokens,
            Err(error) => {
                return Err(error);
            }
        };
        let expanded = match self.expand_write_text(tokens) {
            Ok(expanded) => expanded,
            Err(error) => {
                return Err(error);
            }
        };
        Ok(ImmediateExtension::Write {
            stream,
            tokens: expanded.tokens,
        })
    }

    fn finish_immediate_pdf(
        &mut self,
        command: CurrentCommand<G>,
        primitive: UnexpandablePrimitive,
        pdf_output_enabled: bool,
    ) -> Result<ImmediateExtension, CommandError> {
        if !pdf_output_enabled {
            // pdftex.web §1623 leaves the looked-ahead primitive current after
            // `check_pdfoutput`. The executor also restores the outer
            // `\immediate`, so preserving this inner delivery keeps the two
            // commands in source order for a later PDF-mode retry.
            self.back_input(command)?;
            return Ok(ImmediateExtension::PdfExtensionInDviMode(primitive));
        }

        match primitive {
            UnexpandablePrimitive::PdfObject => self
                .scan_pdf_object_request()
                .map(ImmediateExtension::PdfObject),
            UnexpandablePrimitive::PdfXForm => self
                .scan_pdf_form_request(UnexpandablePrimitive::PdfXForm)
                .map(ImmediateExtension::PdfForm),
            UnexpandablePrimitive::PdfXImage => self
                .scan_pdf_image_request()
                .map(ImmediateExtension::PdfImage),
            _ => Err(CommandError::input_invariant()),
        }
    }

    fn finish_immediate_open_out(&mut self) -> Result<ImmediateExtension, CommandError> {
        let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::FourBit);
        let stream = result.into_result()?.value as u8;
        let result = self.scan_optional_equals_retained();
        result.into_result()?;
        let result = self.scan_file_name_retained();
        let file_name = result.into_result()?;
        Ok(ImmediateExtension::OpenOut { stream, file_name })
    }

    /// Expands TeX82 §§1369--1372 write text inside the artificial brace and
    /// frozen-`\endwrite` input episode installed by `write_out`.
    ///
    /// The returned recovery flag is §1371's `cur_tok<>end_write_token`
    /// test. In that case TeX reports "Unbalanced write command" and consumes
    /// through the inaccessible sentinel, never through the surrounding
    /// source input.
    pub fn expand_write_text(
        &mut self,
        tokens: AttemptTokenListId,
    ) -> Result<ExpandedWriteText, CommandError> {
        self.write_expansion_depth = self
            .write_expansion_depth
            .checked_add(1)
            .ok_or_else(|| CommandError::input_invariant())?;
        let result = self.expand_write_text_inner(tokens);
        self.write_expansion_depth -= 1;
        result
    }

    /// Expands one generation-durable write payload without exposing the
    /// operation-local coordinate used by the write scanner.
    ///
    /// The durable words are copied into the current attempt before the
    /// artificial brace and frozen-`\endwrite` episode is installed. The
    /// attempt id remains entirely command-owned.
    pub fn expand_durable_write_text(
        &mut self,
        tokens: tex_state::TokenListId<G>,
    ) -> Result<ExpandedWriteText, CommandError> {
        let tokens = self.copy_durable_token_list_into_attempt(Some(tokens))?;
        self.expand_write_text(tokens)
    }

    fn expand_write_text_inner(
        &mut self,
        tokens: AttemptTokenListId,
    ) -> Result<ExpandedWriteText, CommandError> {
        let endwrite = self
            .state
            .primitive_token("endwrite")
            .ok_or(CommandError::input_invariant())?;
        let right_brace = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let left_brace = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let (stopper_level, write_words) = {
            let write_words = self
                .command
                .attempt
                .arena()
                .token_words(tokens)
                .map_err(|_| CommandError::input_invariant())?
                .len();
            // The bottom stopper delivers the synthetic closing brace followed
            // by frozen outer `\\endwrite`; the write list and opening brace sit
            // above it exactly as TeX82's three `ins_list` calls do.
            let stopper_level = self.push_write_recovery([right_brace, endwrite], right_brace);
            let write_level = self
                .command
                .push_attempt_list_level(
                    tokens,
                    u32::try_from(write_words).map_err(|_| CommandError::input_invariant())?,
                    TokenBehavior::Ordinary,
                    RetirementBehavior::Pop,
                    ReplayTrace::Stored(StoredReplayReason::Write),
                )
                .map_err(|_| CommandError::input_invariant())?;
            // TeX82 §§323 and 1370 trace the named write_text list at
            // begin_token_list, before the opening-brace insertion and expanded
            // scan_toks can report an error.
            if self
                .state
                .int_param(tex_state::env::banks::IntParam::TRACING_MACROS)
                > 1
            {
                let mut text = String::new();
                crate::processor::expand_render::append_print_esc_text(
                    self.state, "write", &mut text,
                );
                text.push_str("->");
                let words = self
                    .command
                    .attempt
                    .arena()
                    .token_words(tokens)
                    .map_err(|_| CommandError::input_invariant())?;
                for word in words.iter() {
                    crate::processor::expand_render::append_token_list_token_text(
                        self.state,
                        word.semantic_token(),
                        &mut text,
                    );
                }
                self.command
                    .semantic_diagnostics
                    .push(crate::CommandSemanticDiagnostic::Trace {
                        text,
                        force_newline: false,
                    });
            }
            self.observe_write_list_push(write_level);
            self.push_write_recovery([left_brace], left_brace);
            (stopper_level, write_words)
        };

        self.outer_recovered_while_absorbing = false;

        let expanded = match self.scan_balanced_text(true) {
            Ok(expanded) => expanded.tokens,
            Err(error) => {
                return Err(error);
            }
        };
        let transient_words = self.command.transient_dynamic_words();
        let expanded_words = self
            .command
            .attempt
            .arena()
            .token_words(expanded)
            .map_err(|_| CommandError::input_invariant())?
            .len();
        // TeX82 §1370 keeps the source list, expanded result, live command
        // buffers, and three artificial tokens until the stopper is read.
        self.state.observe_transient_token_words(
            write_words
                .saturating_add(expanded_words)
                .saturating_add(transient_words)
                .saturating_add(4),
        );
        let mut destination = None;
        if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
            return Err(CommandError::input_invariant());
        }
        let mut stopper = destination.take().ok_or(CommandError::input_invariant())?;
        let unbalanced =
            self.outer_recovered_while_absorbing || stopper.spelling().semantic_token() != endwrite;
        self.outer_recovered_while_absorbing = false;
        // §1372 calls `error` before its recovery loop consumes through the
        // frozen stopper. Preserve that instant: the write and inserted-list
        // levels are gone by the time shipout can render the queued report.
        let error_context = unbalanced.then(|| self.command.output_open_context(self.state));
        while stopper.spelling().semantic_token() != endwrite {
            if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            stopper = destination.take().ok_or(CommandError::input_invariant())?;
        }
        self.retire_delivery_level(stopper.delivery_stamp())?;
        if unbalanced {
            self.retire_exhausted_through(stopper_level)?;
        }
        Ok(ExpandedWriteText {
            tokens: expanded,
            unbalanced,
            error_context,
        })
    }

    /// Freezes the ordinary `\\write` text after TeX82's `scan_int`
    /// terminator has been validated and backed up. Unlike general-text
    /// callers, §53's `new_write_whatsit` enters the absorbing collection at
    /// that already-backed-up brace.
    fn scan_immediate_write_text(&mut self) -> Result<AttemptTokenListId, CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralAfterOpening {
            expanded: false,
            primary: OriginId::UNKNOWN,
            owner: None,
        })?;
        Ok(scanned.replacement_text)
    }

    fn push_write_recovery(
        &mut self,
        tokens: impl IntoIterator<Item = Token>,
        observed: Token,
    ) -> InputLevelId {
        self.invalidate_delivery_freshness();
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient(
                tokens
                    .into_iter()
                    .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN)),
            ),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        self.observe_inserted_token_recovery(level, observed);
        level
    }
}
