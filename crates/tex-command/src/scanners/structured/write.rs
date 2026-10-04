use super::*;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
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
        let result = self.scan_integer();
        let value = result?.value;
        Ok(if value < 0 {
            WriteStreamSelector::Negative
        } else if value > 15 {
            WriteStreamSelector::AboveRange
        } else {
            WriteStreamSelector::Stream(value as u8)
        })
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
            .ok_or_else(|| CommandError::input_invariant())?;
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
                .token_words(&tokens)
                .map_err(|_| CommandError::input_invariant())?
                .len();
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
                    .token_words(&tokens)
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
            .token_words(&expanded)
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
        let mut stopper = destination
            .take()
            .ok_or_else(|| CommandError::input_invariant())?;
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
            stopper = destination
                .take()
                .ok_or_else(|| CommandError::input_invariant())?;
        }
        self.retire_delivery_level()?;
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
    pub fn scan_immediate_write_text(&mut self) -> Result<AttemptTokenListId, CommandError> {
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
        let level = self.command.push_token_level(
            crate::input::transient_replay(
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
