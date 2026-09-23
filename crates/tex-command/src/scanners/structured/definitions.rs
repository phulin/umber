use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    /// Scans TeX's balanced general text through the canonical `scan_toks`
    /// collector. `expanded` controls its TeX82 expanded-collection mode.
    pub fn scan_balanced_text(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedBalancedText, CommandError> {
        // TeX82 §473's `scan_toks` sets `scanner_status` *before* §403's
        // `scan_left_brace` removes the compulsory opening brace, for both
        // collection modes. Callers that reach here -- §1288's `shift_case`,
        // §1352's `\write`, `\special`, and the pdfTeX graphics family --
        // enter `scan_toks` directly, so the brace must not be scanned and
        // backed up here first: that produced a raw delivery and a
        // backup/recovery pair ahead of the absorbing transition, and cost
        // the redelivered brace its source location. The one call site where
        // TeX really does look the brace up first, §1227's token-list
        // assignment, states that explicitly through `GeneralAfterOpening`.
        let scanned = self.scan_toks(ScanToksMode::General { expanded })?;
        let provenance = provenance(&scanned);
        Ok(ScannedBalancedText {
            tokens: scanned.replacement_text,
            provenance,
        })
    }

    pub fn scan_balanced_text_retained(
        &mut self,
        expanded: bool,
    ) -> crate::RetainedScalarScan<ScannedBalancedText> {
        let result = self.scan_balanced_text(expanded);
        self.detach_retained_scalar(result)
    }

    /// Performs TeX82 §1288's complete `shift_case`.
    ///
    /// `\uppercase`/`\lowercase` are `any_mode` main-control cases that never
    /// reach the stomach: §1288 collects a general text with `scan_toks`,
    /// rewrites each token through the current `\uccode`/`\lccode` table, and
    /// hands the result straight back to the input stack with
    /// `back_list(link(def_ref))`. §323's `back_list` is
    /// `begin_token_list(p, backed_up)`, so the resulting level is a
    /// backed-up token list -- one observed input push, and a retirement that
    /// reports backup rather than a stored token-list replay. Keeping the
    /// whole section here makes the observed command-processor path the only
    /// path: no executor-side step re-pushes this list behind the observer.
    pub fn shift_case(&mut self, uppercase: bool) -> Result<(), CommandError> {
        // §1288 changes only tokens below `cs_token_flag+single_base`, i.e.
        // character tokens and active characters, and leaves the category
        // alone. The canonical collector applies that mapping as each accepted
        // spelling enters this final generation-owned replay span. It never
        // creates an attempt list or walks a completed source list.
        let scanned =
            self.scan_toks_buffers(crate::scan_toks::ScanToksMode::CaseShift { uppercase })?;
        let replay = scanned
            .replay_input()
            .ok_or_else(CommandError::input_invariant)?;
        self.invalidate_delivery_freshness();
        let level = self.command.push_token_level(
            replay,
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        // `back_list` is a plain `begin_token_list`, not §325's `back_input`:
        // it pushes a backed-up level without the accompanying recovery
        // record that a backed-up raw delivery reports.
        self.observe(crate::CommandObservation::Input(crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::Backup,
            source_name: None,
            source: None,
            level: level.0,
            position: 0,
        }));
        Ok(())
    }

    /// Scans `\special`, including pdfTeX's optional `shipout` keyword.
    ///
    /// The ordinary form expands its general text immediately, as TeX82 does.
    /// The `shipout` form retains the unexpanded balanced tokens so traversal
    /// can expand them against the state current when their box is shipped.
    pub fn scan_special(&mut self) -> Result<(bool, ScannedBalancedText), CommandError> {
        // TeX82 §473 enters `scan_toks` immediately. The preceding optional
        // keyword probe belongs only to pdfTeX 1.40.29 §1534; in particular,
        // an e-TeX job must enter `absorbing` before delivering the opening
        // brace instead of speculatively backing it up and replaying it.
        let deferred = if self.profile().capabilities().supports_pdftex() {
            let result = self.scan_keyword_retained("shipout");
            result.into_result()?.value
        } else {
            false
        };
        match self.scan_balanced_text(!deferred) {
            Ok(text) => Ok((deferred, text)),
            Err(error) => Err(error),
        }
    }

    /// Scans a macro parameter text and replacement text without exposing the
    /// temporary macro-argument matcher or its input frames.
    pub fn scan_macro_definition(
        &mut self,
        expanded: bool,
        global: bool,
    ) -> Result<ScannedMacroDefinition<G>, CommandError> {
        let target = self.scan_definition_target()?;
        let scanned = self.scan_toks_buffers(ScanToksMode::MacroDefinitionFor {
            expanded,
            target,
            global,
        })?;
        let provenance = StructuredProvenance {
            primary: scanned.primary,
        };
        let definition = scanned
            .definition()
            .ok_or(CommandError::input_invariant())?;
        Ok(ScannedMacroDefinition {
            target,
            definition,
            provenance,
        })
    }

    /// Scans TeX82 §1221's raw `\let` operand sequence.
    ///
    /// `future` selects `future_let`, whose §1221 body is `get_token;
    /// q:=cur_tok; get_token; back_input; cur_tok:=q; back_input`. Both halves
    /// are ordinary §325 `back_input` calls, so the two tokens are restored on
    /// two separate backup levels -- the second token's level pushed first and
    /// the saved first token's on top of it, which rereads them in their
    /// original order. The meaning defined afterwards is the second token's,
    /// because §325 "doesn't affect `cur_cmd`, `cur_chr`".
    pub fn scan_let_assignment(
        &mut self,
        future: bool,
    ) -> Result<(Symbol, ResolvedMeaning<G>), CommandError> {
        let mut destination = None;
        let target = self.scan_definition_target()?;
        let meaning = if future {
            let mut first_destination = None;
            if self.get_token_into(&mut first_destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let mut second_destination = None;
            if self.get_token_into(&mut second_destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let second = second_destination
                .take()
                .ok_or(CommandError::input_invariant())?;
            let meaning = second.meaning();
            self.back_input(second)?;
            let first = first_destination
                .take()
                .ok_or(CommandError::input_invariant())?;
            self.back_input_saved(first)?;
            meaning
        } else {
            let mut source = loop {
                if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
                    return Err(CommandError::input_invariant());
                }
                let source = destination.take().ok_or(CommandError::input_invariant())?;
                if !matches!(
                    source.meaning_ref(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    })
                ) {
                    break source;
                }
            };
            if matches!(
                source.spelling().semantic_token(),
                Token::Char {
                    ch: '=',
                    cat: Catcode::Other
                }
            ) {
                if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
                    return Err(CommandError::input_invariant());
                }
                source = destination.take().ok_or(CommandError::input_invariant())?;
                if matches!(
                    source.meaning_ref(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    })
                ) {
                    if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
                        return Err(CommandError::input_invariant());
                    }
                    source = destination.take().ok_or(CommandError::input_invariant())?;
                }
            }
            source.into_meaning()
        };
        Ok((target, meaning))
    }
}
