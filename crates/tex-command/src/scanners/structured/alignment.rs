use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    fn push_alignment_live_token(
        &mut self,
        builder: TokenBuilderId,
        spelling: TracedTokenWord,
    ) -> Result<(), CommandError> {
        let live_tokens = self
            .command
            .transient
            .builders
            .iter()
            .find(|live| live.identity == builder.0)
            .ok_or(CommandError::input_invariant())?
            .tokens;
        self.command
            .attempt
            .arena_mut()
            .push_buffer_token(live_tokens, spelling)
            .map_err(|_| CommandError::input_invariant())
    }

    pub(crate) fn abort_alignment_preamble(
        &mut self,
        pending: AlignmentPreambleState<G>,
    ) -> Result<(), CommandError> {
        self.finish_scanner_episode(pending.scanner_episode);
        self.command
            .transient
            .builders
            .retain(|live| live.identity != pending.builder.0);
        Ok(())
    }
    /// Runs TeX82 §774 `init_align`'s `scan_spec(align_group,false)`: §645's
    /// optional `to`/`spread` clause followed by §403's mandatory left brace.
    ///
    /// `\halign`/`\valign` take the same specification as `\hbox`, and §805
    /// packages the preamble prototype box with `hpack(preamble, saved(1),
    /// saved(0))` -- the very values §645 scanned here -- so the clause is
    /// returned rather than discarded.
    ///
    /// The brace is *consumed*, not backed up, exactly as §645 leaves it: the
    /// following `@<Scan the preamble...@>` starts from the token after it.
    /// The two input backups an oracle trace shows here are §407
    /// `scan_keyword`'s own, one per failed keyword, and they are produced by
    /// running the real keyword scans rather than by replaying the brace.
    pub fn scan_alignment_preamble_opening(&mut self) -> Result<ScannedPackingSpec, CommandError> {
        let packing = self.scan_spec_packing()?;
        let _ = self.scan_left_brace(true)?;
        Ok(packing)
    }

    /// Delivers the first alignment cell's lookahead, then backs it up before
    /// the selected u-template is installed.
    ///
    /// This is TeX82's `init_col` lookahead ordering: every non-`\omit`
    /// command, including an ordinary unbraced cell such as `\vrule`, changes
    /// and then restores `align_state` through command-owned backup. `\omit`
    /// instead remains consumed and selects the typed template-free path.
    /// TeX82 §765 does not require the backed-up lookahead to be a left brace.
    pub fn scan_alignment_cell_opening(&mut self) -> Result<AlignmentCellOpening, CommandError> {
        let mut destination = None;
        loop {
            if self.request_expanded_token(&mut destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let opening = destination.take().ok_or(CommandError::input_invariant())?;
            match static_meaning(opening.meaning()) {
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }) => continue,
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit)) => {
                    self.command
                        .prepare_alignment_cell_lookahead()
                        .map_err(|_| CommandError::input_invariant())?;
                    return Ok(AlignmentCellOpening::Omit);
                }
                _ => {
                    self.back_input(opening)?;
                    return Ok(AlignmentCellOpening::Template);
                }
            }
        }
    }

    /// Performs TeX82 §791 `fin_col`'s next-entry lookahead. TeX82 uses
    /// `get_x_token`; e-TeX 2.6 change section [37.791] and pdfTeX use
    /// `get_x_or_protected`. The profile-aware fetch and pending observation
    /// ownership are shared with §785's post-row `align_peek`.
    pub fn scan_alignment_next_cell_opening(
        &mut self,
    ) -> Result<AlignmentCellOpening, CommandError> {
        self.command
            .prepare_alignment_cell_lookahead()
            .map_err(|_| CommandError::input_invariant())?;
        let lookahead = self
            .next_alignment_lookahead()?
            .ok_or(CommandError::input_invariant())?;
        {
            if matches!(
                static_meaning(lookahead.command().meaning()),
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit))
            ) {
                let _ = self.commit_alignment_lookahead_delivery(lookahead);
                return Ok(AlignmentCellOpening::Omit);
            }
            self.back_alignment_lookahead(lookahead)?;
        }
        Ok(AlignmentCellOpening::Template)
    }

    /// Consumes the compulsory opener following `align_peek`'s `\\noalign`.
    ///
    /// TeX82 §37 sets `align_state := 1000000`, recognizes the expanded
    /// `no_align` command, then calls `scan_left_brace` before the executor
    /// creates `no_align_group`.  Unlike an `init_col` lookahead, this brace
    /// is not backed up: its raw delivery is the canonical `1000000 ->
    /// 1000001` transition.
    pub fn scan_alignment_noalign_opening(&mut self) -> Result<(), CommandError> {
        let _ = self.scan_left_brace(true)?;
        Ok(())
    }

    /// Installs TeX82 §37's `align_peek` sentinel before its expanded
    /// lookahead.  The command processor owns this state because it is raw
    /// token-delivery state, not executor group state.
    pub fn begin_alignment_peek(&mut self, restarting: bool) -> Result<(), CommandError> {
        let changed = self.command.alignment.align_state != 1_000_000;
        self.command
            .prepare_alignment_cell_lookahead()
            .map_err(|_| CommandError::input_invariant())?;
        // TeX82 §785's `restart` label assigns the sentinel on every pass.
        // The initial pass is already represented when its caller changed
        // the value; an ignored `\crcr` returns to the label and must publish
        // the otherwise-idempotent assignment too.
        self.observe_alignment_peek_sentinel(changed || restarting);
        Ok(())
    }

    /// Enters TeX82's live alignment-preamble scanner episode.
    ///
    /// `init_align` establishes `scanner_status := aligning` after its
    /// required brace has been replayed and backed up, but before the first
    /// `get_preamble_token` retires that backup.  The status therefore belongs
    /// to the command-owned input transition, rather than to executor replay
    /// or the preamble parser.
    pub fn begin_alignment_preamble_scan(
        &mut self,
        owner: Option<tex_state::interner::Symbol>,
    ) -> Result<(), CommandError> {
        let mut pending = {
            // TeX82 §776's preamble scan begins with the opener already
            // consumed. It owns both template sinks before its first token
            // demand, so a nested expansion can suspend without moving either
            // result out of the attempt arena.
            let alignment = self
                .command
                .alignment
                .active_alignment
                .ok_or(CommandError::input_invariant())?;
            self.command
                .alignment
                .set_preamble_phase(alignment)
                .map_err(|_| CommandError::input_invariant())?;
            let builder = TokenBuilderId(self.command.transient.next_builder_identity);
            self.command.transient.next_builder_identity =
                self.command.transient.next_builder_identity.wrapping_add(1);
            let live_tokens = self
                .command
                .attempt
                .arena_mut()
                .allocate_token_buffer()
                .map_err(|_| CommandError::input_invariant())?;
            self.command
                .transient
                .builders
                .push(crate::state::LiveTokenBuilder {
                    identity: builder.0,
                    tokens: live_tokens,
                });
            let scanner_episode = self.begin_scanner_episode(
                ScannerStatus::Aligning(AlignmentScanContext {
                    alignment: AlignmentId(alignment.raw()),
                    builder,
                    owner,
                    warning: ScannerWarning(0),
                }),
                ScannerStatusVisibility::Observed,
            );
            observe!(
                self,
                crate::CommandObservation::Alignment(crate::AlignmentRecord {
                    transition: "preamble_start",
                    alignment: Some(alignment.raw()),
                    nesting: self.command.alignment_observation_nesting(),
                    align_state: self.command.alignment.align_state,
                    delimiter: None,
                    previous_align_state: None,
                },),
            );
            let current_tabskip = self
                .state
                .glue_param(GlueParam::TAB_SKIP)
                .map_or_else(|| GlueSpec::ZERO, |id| self.state.glue(id));
            AlignmentPreambleState {
                alignment,
                builder,
                scanner_episode,
                columns: Vec::new(),
                tabskips: vec![current_tabskip],
                current_tabskip,
                repeat_start: None,
                u_template: self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_token_buffer()
                    .map_err(|_| CommandError::input_invariant())?,
                v_template: self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_token_buffer()
                    .map_err(|_| CommandError::input_invariant())?,
                phase: AlignmentPreamblePhase::UTemplate,
                scalar_scan: None,
                _generation: PhantomData,
            }
        };
        loop {
            if let Some(scalar) = pending.scalar_scan.take() {
                match scalar.phase {
                    AlignmentPreambleScalarPhase::TabskipEquals => {
                        match self.scan_optional_equals_retained() {
                            crate::RetainedScalarScan::Complete(_) => {
                                pending.scalar_scan = Some(AlignmentPreambleScalar {
                                    phase: AlignmentPreambleScalarPhase::TabskipGlue,
                                    _generation: PhantomData,
                                });
                                continue;
                            }
                            crate::RetainedScalarScan::Failed(error) => {
                                self.abort_alignment_preamble(pending)?;
                                return Err(error);
                            }
                        }
                    }
                    AlignmentPreambleScalarPhase::TabskipGlue => {
                        match self.scan_glue_retained(false) {
                            crate::RetainedScalarScan::Complete(value) => {
                                pending.current_tabskip = value.value;
                                let global = self.state.int_param(IntParam::GLOBAL_DEFS) > 0;
                                self.state
                                    .define_preamble_tabskip(pending.current_tabskip, global);
                                continue;
                            }
                            crate::RetainedScalarScan::Failed(error) => {
                                self.abort_alignment_preamble(pending)?;
                                return Err(error);
                            }
                        }
                    }
                }
            }
            let mut destination = None;
            let command = match self.get_preamble_token(&mut destination) {
                Ok(DeliveryStatus::Command) => {
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                Ok(DeliveryStatus::End) => {
                    self.abort_alignment_preamble(pending)?;
                    return Err(CommandError::input_invariant());
                }
                Ok(_) => {
                    self.abort_alignment_preamble(pending)?;
                    return Err(CommandError::input_invariant());
                }
                Err(error) if error.is_resource_suspension() => {
                    self.abort_alignment_preamble(pending)?;
                    return Err(error);
                }
                Err(error) => {
                    self.abort_alignment_preamble(pending)?;
                    return Err(error);
                }
            };
            if matches!(
                static_meaning(command.meaning()),
                Some(Meaning::GlueParam(index)) if index == GlueParam::TAB_SKIP.raw()
            ) {
                pending.scalar_scan = Some(AlignmentPreambleScalar {
                    phase: AlignmentPreambleScalarPhase::TabskipEquals,
                    _generation: PhantomData,
                });
                continue;
            }

            match pending.phase {
                AlignmentPreamblePhase::UTemplate => {
                    if is_character_command(&command, Catcode::Parameter) {
                        pending.phase = AlignmentPreamblePhase::VTemplate;
                        continue;
                    }
                    let tab = is_character_command(&command, Catcode::AlignmentTab);
                    let terminator = tab
                        || matches!(
                            static_meaning(command.meaning()),
                            Some(Meaning::UnexpandablePrimitive(
                                UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                            ))
                        );
                    if terminator && self.command.alignment.align_state == PREAMBLE_ALIGN_STATE {
                        if tab
                            && self
                                .command
                                .attempt
                                .arena()
                                .token_buffer(pending.u_template)
                                .map_err(|_| CommandError::input_invariant())?
                                .is_empty()
                            && pending.repeat_start.is_none()
                        {
                            pending.repeat_start = Some(pending.columns.len());
                            continue;
                        }
                        observe!(
                            self,
                            crate::CommandObservation::Alignment(crate::AlignmentRecord {
                                transition: "missing_parameter",
                                alignment: Some(pending.alignment.raw()),
                                nesting: self.command.alignment_observation_nesting(),
                                align_state: self.command.alignment.align_state,
                                delimiter: None,
                                previous_align_state: None,
                            },),
                        );
                        self.back_error_reporting(
                            command,
                            MISSING_PARAMETER_DIAGNOSTIC,
                            "Missing # inserted in alignment preamble".to_owned(),
                            &[
                                "There should be exactly one # between &'s, when an",
                                "\\halign or \\valign is being set up. In this case you had",
                                "none, so I've put one in; maybe that will work.",
                            ],
                        )?;
                        pending.phase = AlignmentPreamblePhase::VTemplate;
                        continue;
                    }
                    if !matches!(
                        static_meaning(command.meaning()),
                        Some(Meaning::CharToken {
                            cat: Catcode::Space,
                            ..
                        })
                    ) || !self
                        .command
                        .attempt
                        .arena()
                        .token_buffer(pending.u_template)
                        .map_err(|_| CommandError::input_invariant())?
                        .is_empty()
                    {
                        self.command
                            .attempt
                            .arena_mut()
                            .push_buffer_token(pending.u_template, command.spelling())
                            .map_err(|_| CommandError::input_invariant())?;
                        self.push_alignment_live_token(pending.builder, command.spelling())?;
                    }
                }
                AlignmentPreamblePhase::VTemplate => {
                    let ends_column = is_character_command(&command, Catcode::AlignmentTab);
                    let ends_preamble = matches!(
                        static_meaning(command.meaning()),
                        Some(Meaning::UnexpandablePrimitive(
                            UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                        ))
                    );
                    if (ends_column || ends_preamble)
                        && self.command.alignment.align_state == PREAMBLE_ALIGN_STATE
                    {
                        let u_template = self
                            .command
                            .attempt
                            .arena_mut()
                            .finish_token_buffer(pending.u_template)
                            .map_err(|_| CommandError::input_invariant())?;
                        let v_template = self
                            .command
                            .attempt
                            .arena_mut()
                            .finish_token_buffer(pending.v_template)
                            .map_err(|_| CommandError::input_invariant())?;
                        pending.columns.push(AlignmentCellTemplates {
                            u_template: Some(u_template),
                            v_template,
                        });
                        pending.tabskips.push(pending.current_tabskip);
                        if ends_preamble {
                            break;
                        }
                        pending.u_template = self
                            .command
                            .attempt
                            .arena_mut()
                            .allocate_token_buffer()
                            .map_err(|_| CommandError::input_invariant())?;
                        pending.v_template = self
                            .command
                            .attempt
                            .arena_mut()
                            .allocate_token_buffer()
                            .map_err(|_| CommandError::input_invariant())?;
                        pending.phase = AlignmentPreamblePhase::UTemplate;
                        continue;
                    }
                    if is_character_command(&command, Catcode::Parameter) {
                        observe!(
                            self,
                            crate::CommandObservation::Alignment(crate::AlignmentRecord {
                                transition: "extra_parameter",
                                alignment: Some(pending.alignment.raw()),
                                nesting: self.command.alignment_observation_nesting(),
                                align_state: self.command.alignment.align_state,
                                delimiter: None,
                                previous_align_state: None,
                            },),
                        );
                        self.report_recoverable(
                            EXTRA_PARAMETER_DIAGNOSTIC,
                            "Only one # is allowed per tab".to_owned(),
                            &[
                                "There should be exactly one # between &'s, when an",
                                "\\halign or \\valign is being set up. In this case you had",
                                "more than one, so I'm ignoring all but the first.",
                            ],
                        );
                        continue;
                    }
                    self.command
                        .attempt
                        .arena_mut()
                        .push_buffer_token(pending.v_template, command.spelling())
                        .map_err(|_| CommandError::input_invariant())?;
                    self.push_alignment_live_token(pending.builder, command.spelling())?;
                }
            }
        }
        self.command
            .alignment
            .complete_preamble(
                pending.alignment,
                AlignmentPreamble {
                    columns: pending.columns,
                    tabskips: pending.tabskips,
                    default_tabskip: pending.current_tabskip,
                    repeat_start: pending.repeat_start,
                },
            )
            .map_err(|_| CommandError::input_invariant())?;
        observe!(
            self,
            crate::CommandObservation::Alignment(crate::AlignmentRecord {
                transition: "preamble_finish",
                alignment: Some(pending.alignment.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },),
        );
        // TeX's `fin_align` boundary becomes observable before `scanner_status`
        // returns to normal. Retain the live aligning episode while publishing
        // its completion, then restore normal status; otherwise an exit record
        // loses its `aligning` identity and reverses the canonical ordering.
        self.finish_scanner_episode(pending.scanner_episode);
        self.command
            .transient
            .builders
            .retain(|live| live.identity != pending.builder.0);
        Ok(())
    }

    /// TeX82 §759's `get_preamble_token`.
    ///
    /// A `\span` is not template material: it fetches the following token,
    /// expands that token exactly once when expandable, and repeats if the
    /// resulting raw token is another `\span`. Ordinary template tokens stay
    /// raw so their meanings are resolved when each cell is executed.
    fn get_preamble_token(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let delivery = self.get_token_into(destination)?;
        if delivery == DeliveryStatus::End {
            return Ok(DeliveryStatus::End);
        }
        if delivery != DeliveryStatus::Command {
            return Err(CommandError::input_invariant());
        }
        while destination.as_ref().is_some_and(|command| {
            matches!(
                static_meaning(command.meaning()),
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Span))
            )
        }) {
            destination.take();
            match self.get_token_into(destination)? {
                DeliveryStatus::End => return Ok(DeliveryStatus::End),
                DeliveryStatus::Command => {}
                _ => return Err(CommandError::input_invariant()),
            }
            if destination
                .as_ref()
                .is_some_and(crate::processor::expand::is_expandable_command)
            {
                self.request_expansion_into(destination, true)?;
                match self.continue_preamble_after_span_expansion(destination)? {
                    DeliveryStatus::End => return Ok(DeliveryStatus::End),
                    DeliveryStatus::Command => {}
                    _ => return Err(CommandError::input_invariant()),
                }
            }
        }
        if destination.as_ref().is_some_and(|command| {
            matches!(
                command.spelling().semantic_token(),
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                }
            ) && self.command.alignment.align_state == PREAMBLE_ALIGN_STATE
        }) && let Some(cr) = self.command.alignment.pending_outer_recovery_cr.take()
        {
            // §336's first inserted `\cr` was seen while the runaway brace
            // was still open. Once the follow-up `}` restores the preamble
            // sentinel, replay the owned delimiter/brace tail before the
            // backed-up forbidden command can open a second runaway episode.
            self.conserve_input_stack_for_descendant()?;
            self.invalidate_delivery_freshness();
            self.command.push_token_level(
                PackedTokenSpanHandle::transient([
                    cr,
                    TracedTokenWord::pack(
                        Token::Char {
                            ch: '}',
                            cat: Catcode::EndGroup,
                        },
                        OriginId::UNKNOWN,
                    ),
                ]),
                TokenBehavior::Recovery,
                RetirementBehavior::Pop,
                ReplayTrace::Inserted,
            );
        }
        Ok(DeliveryStatus::Command)
    }

    /// Completes TeX82 §759's `expand; get_token` preamble transition.
    ///
    /// Successful expansion has consumed the current command semantically but
    /// leaves its physical owner in the caller slot for the fused expanded
    /// driver to reuse. Section 759 instead crosses into a fresh raw
    /// `get_token`, so retire that settled owner before raw delivery fills the
    /// same destination with the following token.
    fn continue_preamble_after_span_expansion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        // `request_expansion_into` consumes the expanded command on success;
        // §759 immediately crosses to a fresh raw `get_token` for the span
        // operand, so the destination must already be empty here.
        debug_assert!(destination.is_none());
        self.get_token_into(destination)
    }
}
