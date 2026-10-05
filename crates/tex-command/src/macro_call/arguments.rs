//! Canonical macro argument grammar over final native spelling.

use super::*;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// §395 backs up the actual extra brace before inserting and reporting par.
    fn recover_extra_right_brace_argument(
        &mut self,
        word: TracedTokenWord,
        command: Option<&CurrentCommand<G>>,
    ) -> Result<(), CommandError> {
        self.back_consumed_word(word, command)?;
        self.insert_macro_argument_recovery_par()?;
        self.report_extra_right_brace_argument();
        self.consume_raw_words::<false, _>(&mut |processor, par, _, command| {
            processor
                .back_consumed_word(par, command)
                .map(|()| std::ops::ControlFlow::Break(()))
        })?
        .ok_or(CommandError::ParagraphInMacroArgument)?;
        self.report_paragraph_ended_before_complete(&[]);
        Err(CommandError::ParagraphInMacroArgument)
    }

    pub(super) fn scan_undelimited_argument(
        &mut self,
        tokens: &mut ArgumentCursor,
        flags: MeaningFlags,
        paragraph_token: Option<TokenWord>,
    ) -> Result<(), CommandError> {
        // §394 keeps the opening brace until completion, so runaway context
        // still sees the unmatched group. One grammar handles its first word
        // and interior words directly from their final spelling.
        loop {
            if !self.is_observed() {
                let run = self.consume_argument_run(tokens, paragraph_token, None)?;
                if run.consumed != 0 && tokens.brace_depth() == 0 {
                    strip_argument_outer_group(tokens)?;
                    return Ok(());
                }
                if run.at_transition && self.settle_resident_run_boundary()? {
                    continue;
                }
            }
            let appended = self
                .consume_raw_words::<false, _>(&mut |processor, word, _, command| {
                    if command.is_some_and(CurrentCommand::is_outer_recovery_space) {
                        // §23's returned spacer is not linked into the argument.
                        return Ok(std::ops::ControlFlow::Break(false));
                    }
                    if processor.outer_recovered_while_matching
                        && command.is_some_and(CurrentCommand::is_effective_paragraph)
                    {
                        if tokens
                            .visible_len()
                            .map_err(|_| CommandError::input_invariant())?
                            != 0
                        {
                            let partial = processor
                                .command
                                .input
                                .levels
                                .arguments
                                .match_words(tokens)
                                .map_err(|_| CommandError::input_invariant())?
                                .collect::<Vec<_>>();
                            processor.set_runaway_partial(
                                crate::processor::RUNAWAY_SCAN_DIAGNOSTIC,
                                &partial,
                            );
                        }
                        return Err(CommandError::OuterInMacroArgument);
                    }
                    let catcode = word.token_word().literal_catcode();
                    if tokens.brace_depth() == 0 {
                        if catcode == Some(Catcode::Space) {
                            return Ok(std::ops::ControlFlow::Break(false));
                        }
                        if catcode == Some(Catcode::EndGroup) {
                            return processor
                                .recover_extra_right_brace_argument(word, command)
                                .map(|_| std::ops::ControlFlow::Break(false));
                        }
                    }
                    processor.check_argument_paragraph(
                        word,
                        command,
                        paragraph_token,
                        flags,
                        Some(tokens),
                    )?;
                    let input = &mut processor.command.input.levels;
                    input
                        .arguments
                        .append_from_input(tokens, word, input.current.as_ref(), false)
                        .map_err(|_| CommandError::input_invariant())?;
                    Ok(std::ops::ControlFlow::Break(true))
                })?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            if appended && tokens.brace_depth() == 0 {
                strip_argument_outer_group(tokens)?;
                return Ok(());
            }
        }
    }

    pub(super) fn scan_delimited_argument(
        &mut self,
        tokens: &mut ArgumentCursor,
        flags: MeaningFlags,
        delimiter: MacroDelimiter<'_>,
        paragraph_token: Option<TokenWord>,
    ) -> Result<(), CommandError> {
        loop {
            // Delimiters are inactive inside a balanced group, and at depth
            // zero with no pending delimiter prefix a word other than the
            // delimiter's first token cannot start a match. Consume such
            // ordinary words in place and return to scalar delivery only at a
            // semantic boundary. A held prefix exists only at depth zero.
            if !self.is_observed()
                && self
                    .command
                    .input
                    .levels
                    .arguments
                    .delimiter_prefix_len(tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    == 0
            {
                let stop_word = delimiter.words()[0];
                let run = self.consume_argument_run(tokens, paragraph_token, Some(stop_word))?;
                if run.at_transition && self.settle_resident_run_boundary()? {
                    continue;
                }
            }
            let completed = self
                .consume_raw_words::<false, _>(&mut |processor, word, _, command| {
                    if command.is_some_and(CurrentCommand::is_outer_recovery_space) {
                        return Ok(std::ops::ControlFlow::Break(false));
                    }
                    if processor.outer_recovered_while_matching
                        && command.is_some_and(CurrentCommand::is_effective_paragraph)
                    {
                        let mut partial = processor
                            .command
                            .input
                            .levels
                            .arguments
                            .match_words(tokens)
                            .map_err(|_| CommandError::input_invariant())?
                            .collect::<Vec<_>>();
                        partial.extend(
                            processor
                                .command
                                .input
                                .levels
                                .arguments
                                .delimiter_prefix_words(tokens)
                                .map_err(|_| CommandError::input_invariant())?,
                        );
                        processor.set_runaway_partial(
                            crate::processor::RUNAWAY_SCAN_DIAGNOSTIC,
                            &partial,
                        );
                        return Err(CommandError::OuterInMacroArgument);
                    }

                    let spelling = word.token_word();
                    let prefix_len = processor
                        .command
                        .input
                        .levels
                        .arguments
                        .delimiter_prefix_len(tokens)
                        .map_err(|_| CommandError::input_invariant())?;
                    if tokens.brace_depth() == 0
                        && prefix_len < delimiter.words().len()
                        && spelling == delimiter.words()[prefix_len]
                    {
                        if prefix_len + 1 == delimiter.words().len() {
                            // `#{` consumes the opening brace as parameter text. Raw
                            // delivery has accounted for it, but no replacement-body
                            // replay exists yet to provide the balancing delivery.
                            if word.token_word().literal_catcode() == Some(Catcode::BeginGroup) {
                                processor.undo_delimiter_begin_group_delivery();
                            }
                            if prefix_len != 0 {
                                processor
                                    .command
                                    .input
                                    .levels
                                    .arguments
                                    .truncate_delimiter_suffix(tokens)
                                    .map_err(|_| CommandError::input_invariant())?;
                            }
                            return Ok(std::ops::ControlFlow::Break(true));
                        }
                        let input = &mut processor.command.input.levels;
                        input
                            .arguments
                            .append_from_input(tokens, word, input.current.as_ref(), true)
                            .map_err(|_| CommandError::input_invariant())?;
                        return Ok(std::ops::ControlFlow::Break(false));
                    }

                    if prefix_len != 0 {
                        let retained = delimiter.overlap(prefix_len, spelling);
                        // If the current token is retained, every already-held word
                        // before that suffix is now committed.  If no suffix remains,
                        // all held words are committed and the current token is
                        // handled as ordinary argument material below.
                        let committed = if retained == 0 {
                            prefix_len
                        } else {
                            prefix_len + 1 - retained
                        };
                        if processor.is_observed() {
                            for index in 0..committed {
                                let prefix_token = processor
                                    .command
                                    .input
                                    .levels
                                    .arguments
                                    .delimiter_prefix_word(tokens, index)
                                    .map_err(|_| CommandError::input_invariant())?;
                                observe!(
                                    processor,
                                    CommandObservation::TokenList(TokenListRecord {
                                        transition: "splice",
                                        purpose: "macro_delimiter_recovery",
                                        tokens: vec![processor.observed_token(prefix_token)],
                                    }),
                                );
                            }
                        }
                        processor
                            .command
                            .input
                            .levels
                            .arguments
                            .reveal_delimiter_words(tokens, committed as u32)
                            .map_err(|_| CommandError::input_invariant())?;
                        if retained != 0 {
                            // The current spelling is the final word of the retained
                            // suffix. It is unpublished until the next mismatch or
                            // until the complete delimiter truncates the holdback.
                            let input = &mut processor.command.input.levels;
                            input
                                .arguments
                                .append_from_input(tokens, word, input.current.as_ref(), true)
                                .map_err(|_| CommandError::input_invariant())?;
                            return Ok(std::ops::ControlFlow::Break(false));
                        }
                    }

                    // The failed delimiter prefix is revealed before §395 examines
                    // the current token. A held opening brace can consequently make a
                    // closing brace ordinary argument material.
                    if tokens.brace_depth() == 0
                        && word.token_word().literal_catcode() == Some(Catcode::EndGroup)
                    {
                        return processor
                            .recover_extra_right_brace_argument(word, command)
                            .map(|_| std::ops::ControlFlow::Break(false));
                    }
                    processor.check_argument_paragraph(
                        word,
                        command,
                        paragraph_token,
                        flags,
                        Some(tokens),
                    )?;
                    let input = &mut processor.command.input.levels;
                    input
                        .arguments
                        .append_from_input(tokens, word, input.current.as_ref(), false)
                        .map_err(|_| CommandError::input_invariant())?;
                    Ok(std::ops::ControlFlow::Break(false))
                })?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            if completed {
                strip_argument_outer_group(tokens)?;
                return Ok(());
            }
        }
    }

    /// Consumes one batched run of ordinary argument words. Control
    /// sequences are classified only while no alignment interception or outer
    /// recovery can apply to them.
    fn consume_argument_run(
        &mut self,
        tokens: &mut ArgumentCursor,
        paragraph_token: Option<TokenWord>,
        stop_word: Option<TokenWord>,
    ) -> Result<crate::input::ResidentRun, CommandError> {
        // An undelimited argument (no delimiter stop word) ends at its own
        // closing brace, which its run admits last; a delimited argument may
        // close any group it opened.
        let close_floor = u32::from(stop_word.is_none());
        let admission = ArgumentRunAdmission::with_commands(
            self.state,
            paragraph_token,
            stop_word,
            close_floor,
        );
        let mut admission = if stop_word.is_none() {
            admission.completing_undelimited()
        } else {
            admission
        };
        if self.command.delivery_mode.alignment_active() || self.outer_recovered_while_matching {
            admission.restrict_to_characters();
        }
        self.command
            .consume_plain_resident_run(tokens, self.fuel, admission)
    }

    fn check_argument_paragraph(
        &mut self,
        word: TracedTokenWord,
        command: Option<&CurrentCommand<G>>,
        paragraph_token: Option<TokenWord>,
        flags: MeaningFlags,
        partial: Option<&ArgumentCursor>,
    ) -> Result<(), CommandError> {
        if self.eof_recovered_while_matching
            && command.is_some_and(CurrentCommand::is_effective_paragraph)
        {
            // TeX82 §23 calls `check_outer_validity` after source EOF and
            // changes `long_state` to `outer_call`, even for a `\long` macro.
            // Its inserted frozen `\par` terminates the match but is consumed
            // by the failed expansion instead of being replayed by §396.
            let partial = partial
                .map(|buffer| {
                    self.command
                        .input
                        .levels
                        .arguments
                        .match_words(buffer)
                        .map(|words| words.collect::<Vec<_>>())
                        .map_err(|_| CommandError::input_invariant())
                })
                .transpose()?
                .unwrap_or_default();
            self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        if (Some(word.token_word()) == paragraph_token) && !flags.contains(MeaningFlags::LONG) {
            // TeX82 §394 reports this through `back_error` while the macro
            // matcher is still live.  The caller will then restore its
            // enclosing scanner status, so retain the exact `\par` input
            // ahead of that restoration rather than merely returning an
            // error from the scalar matcher.
            self.back_consumed_word(word, command)?;
            // §396 ends with `back_error`, so §82 renders the context with the
            // replayed `\par` already on the stack.
            let partial = partial
                .map(|buffer| {
                    self.command
                        .input
                        .levels
                        .arguments
                        .match_words(buffer)
                        .map(|words| words.collect::<Vec<_>>())
                        .map_err(|_| CommandError::input_invariant())
                })
                .transpose()?
                .unwrap_or_default();
            self.report_paragraph_ended_before_complete(&partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        Ok(())
    }
}

// TeX82 §394 strips a single enclosing group at this actual completion,
// borrowing the caller's cursor without producing another cursor value.
fn strip_argument_outer_group(buffer: &mut ArgumentCursor) -> Result<(), CommandError> {
    if buffer.removable_outer_group() {
        buffer
            .strip_outer_group()
            .map_err(|_| CommandError::input_invariant())?;
    }
    Ok(())
}
