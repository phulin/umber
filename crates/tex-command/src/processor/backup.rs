//! Canonical command backup and replay insertion transitions.

use tex_state::meaning::{Meaning, MeaningWord};
use tex_state::token::{Catcode, TracedTokenWord};

use crate::command::CurrentCommand;
use crate::error::CommandError;
use crate::input::{
    BackedUpToken, BackupTreatment, ReplayTrace, RetirementBehavior, TokenBehavior,
    backed_up_replay,
};
use crate::observation::{
    AlignmentRecord, CommandObservation, InputReason, InputRecord, InputTransition, RecoveryKind,
    RecoveryRecord,
};

use super::CommandProcessor;
use super::alignment::AlignmentDeliveryState;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// TeX82 §325's `back_input`: pushes the command's token back onto
    /// input and reverses its literal brace accounting.
    ///
    /// §326's `cur_tok:=p; back_input` restores a saved token the same way,
    /// so a command superseded by a later delivery is backed up through this
    /// same call.
    pub fn back_input(&mut self, command: CurrentCommand<G>) -> Result<(), CommandError> {
        self.back_input_with_treatment(command, BackupTreatment::Ordinary)
    }

    /// Performs TeX82 §1138 `init_math`'s opening probe: the lookahead that
    /// decides whether a `$` seen in horizontal mode opens display math.
    ///
    /// §1138 reads the second token with `get_token`, and states the reason on
    /// that very line -- "`get_x_token` would fail on `\ifmmode`". The probe
    /// therefore must not expand the peeked token, and must not observe an
    /// expanded delivery for it.
    ///
    /// The pair is consumed only when `outer_horizontal` holds, matching
    /// §1138's `(cur_cmd=math_shift)and(mode>0)`: in restricted horizontal
    /// mode `mode<0`, so even a genuine second `$` is backed up and reread as
    /// the immediate end of an empty inline formula. Every other outcome runs
    /// §325 `back_input`, so exactly one raw delivery is ever consumed without
    /// a backup level.
    pub fn scan_init_math_display_pair(
        &mut self,
        outer_horizontal: bool,
    ) -> Result<bool, CommandError> {
        self.consume_raw_words::<true, _>(&mut |processor, spelling, meaning, exceptional| {
            let shift = outer_horizontal
                && matches!(
                    meaning.expect("math probe requested native meaning"),
                    MeaningWord::Static(word)
                        if Meaning::runtime_word_character(*word)
                            .is_some_and(|(_, cat)| cat == Catcode::MathShift)
                );
            if !shift {
                processor.back_consumed_word(spelling, exceptional)?;
            }
            Ok(std::ops::ControlFlow::Break(shift))
        })
        .map(|shift| shift.unwrap_or(false))
    }
    /// Performs TeX82 §1197's `@<Check that another \.\$ follows@>`, the probe
    /// §1194 `after_math` runs when a display, or a display's equation number,
    /// is closing.
    ///
    /// Unlike §1138's opener this one _is_ `get_x_token`, so the peeked token
    /// is expanded and observed as an expanded delivery. A non-shift reaches
    /// §327 `back_error`, whose backup half lives here; the executor owns the
    /// accompanying ``Display math should end with $$`` diagnostic.
    pub fn scan_display_end_math_shift(&mut self) -> Result<bool, CommandError> {
        let mut shift = false;
        self.consume_expanded_words(&mut |processor, spelling, meaning, _, exceptional| {
            shift = matches!(meaning, MeaningWord::Static(word)
                if Meaning::runtime_word_character(*word)
                    .is_some_and(|(_, cat)| cat == Catcode::MathShift));
            if !shift {
                processor.back_consumed_word(spelling, exceptional)?;
            }
            Ok(std::ops::ControlFlow::Break(()))
        })?;
        Ok(shift)
    }
    /// TeX82 §323's `back_list`: `begin_token_list(p,backed_up)`.
    ///
    /// This is not §325's `back_input`, and the difference is structural
    /// rather than cosmetic. `back_input` undoes *the* preceding delivery: it
    /// runs §325's stack-conservation loop, reverses that delivery's literal
    /// brace `align_state` adjustment, and is observed together with the
    /// token it is undoing. `back_list` merely pushes a token list the caller
    /// assembled, so it does none of those things -- the instrumented
    /// `begin_token_list` observes a `backed_up` push with no recovery record
    /// at all.
    ///
    /// §407's `scan_keyword` is why both exist: a failed match backs the
    /// offending token up with `back_input` and then pushes the
    /// already-matched prefix as a second, separate level, so the prefix is
    /// reread first and the offender after it. Collapsing the two into one
    /// level loses a push transition the oracle records, and merging the
    /// prefix into `back_input`'s level would additionally claim the prefix
    /// as part of the undone delivery.
    ///
    /// §407 guards its call with `if p<>backup_head`, so an empty list is the
    /// caller's business; pushing one here would observe a level that retires
    /// without ever delivering a token.
    pub(crate) fn back_list(&mut self, tokens: impl IntoIterator<Item = BackedUpToken>) {
        let level = self.command.push_token_level(
            backed_up_replay(tokens),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Backup,
                reason: InputReason::Backup,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }),
        );
    }
    /// TeX82 §325's `back_input` driven by a token rather than by the live
    /// delivery: the §326 shape `cur_tok:=p; back_input`, where `p` is a
    /// token the caller holds instead of one it just consumed.
    ///
    /// §325 requires only that `cur_tok` name the token to be reread. It runs
    /// the stack-conservation loop, derives its `align_state` change from
    /// `cur_tok`'s own category ([`AlignmentDeliveryState::back_input_adjustment`]),
    /// and pushes a one-token `backed_up` list. It serves every caller whose
    /// token is not a delivered command:
    ///
    /// - §372's `\\csname`: `cur_tok:=cur_cs+cs_token_flag; back_input` backs
    ///   up a control sequence that was never delivered at all.
    /// - §282's `unsave`, through §326: each `insert_token` entry left by
    ///   `\\aftergroup` is backed up as the group's save-stack level is
    ///   cleared off, long after that token was scanned.
    ///
    /// [`Self::back_input`] is the sibling for a caller that still holds
    /// the `CurrentCommand<G>`: §342's alignment interception records transitions
    /// that set `align_state` outright rather than stepping it, so a delivery
    /// that is available must have its own adjustment reversed, not one
    /// recomputed from the token.
    pub fn back_input_token(&mut self, spelling: TracedTokenWord) -> Result<(), CommandError> {
        self.push_backed_up_token(spelling, spelling)
    }

    /// Shared §326 insertion for [`Self::back_input_token`]. The level's
    /// storage is the caller's: an inline word, or a replay-lane list when
    /// e-TeX will link further `\aftergroup` tokens onto it.
    #[inline(always)] // pure forwarding must not add a native backup call frame
    fn push_backed_up_token(
        &mut self,
        spelling: TracedTokenWord,
        storage: impl crate::input::PackedTokenSpanSource<G>,
    ) -> Result<(), CommandError> {
        let adjustment =
            AlignmentDeliveryState::<G>::back_input_adjustment(spelling.semantic_token());
        let level = self.push_backed_up_input(storage, BackupTreatment::Ordinary, adjustment)?;
        if self.is_observed() {
            self.observe_backed_up_input(level, self.observed_token(spelling), adjustment);
        }
        Ok(())
    }

    /// Reads the actual spelling and backup facts that must survive another
    /// operand read or a temporary scanner-status scope.
    #[inline]
    pub(crate) fn read_backup_operand(
        &mut self,
        adjustment: &mut super::AlignmentDeliveryAdjustment,
        observed: &mut Option<crate::observation::ObservedToken>,
    ) -> Result<TracedTokenWord, CommandError> {
        self.consume_raw_words::<false, _>(&mut |processor, spelling, _, command| {
            *adjustment = command.map_or_else(
                || AlignmentDeliveryState::<G>::back_input_adjustment(spelling.semantic_token()),
                CurrentCommand::alignment_adjustment,
            );
            if processor.is_observed() {
                *observed = Some(processor.observed_command_spelling(
                    command.expect("observed raw operand has a command"),
                ));
            }
            Ok(std::ops::ControlFlow::Break(spelling))
        })?
        .ok_or_else(CommandError::input_invariant)
    }

    /// Backs a real token retained across another operand read. Only its
    /// original adjustment and demanded observed spelling outlive delivery.
    #[inline(always)] // pure forwarding must not add a native backup call frame
    pub(crate) fn back_retained_word(
        &mut self,
        spelling: TracedTokenWord,
        adjustment: super::AlignmentDeliveryAdjustment,
        observed_spelling: Option<crate::observation::ObservedToken>,
        treatment: BackupTreatment,
    ) -> Result<(), CommandError> {
        let level = self.push_backed_up_input(spelling, treatment, adjustment)?;
        if self.is_observed() {
            self.observe_backed_up_input(
                level,
                observed_spelling.expect("observed retained token has its original spelling"),
                adjustment,
            );
        }
        Ok(())
    }

    #[inline]
    fn push_backed_up_input(
        &mut self,
        storage: impl crate::input::PackedTokenSpanSource<G>,
        treatment: BackupTreatment,
        adjustment: super::AlignmentDeliveryAdjustment,
    ) -> Result<crate::input::InputLevelId, CommandError> {
        // §325 conserves depleted input before undoing delivery and pushing.
        self.conserve_input_stack_for_descendant()?;
        self.command.alignment.undo_delivery(adjustment);
        let level = self.command.push_token_level(
            storage,
            TokenBehavior::BackedUp(treatment),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        Ok(level)
    }

    #[cold]
    fn observe_backed_up_input(
        &mut self,
        level: crate::input::InputLevelId,
        spelling: crate::observation::ObservedToken,
        adjustment: super::AlignmentDeliveryAdjustment,
    ) {
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Backup,
            reason: InputReason::Backup,
            source_name: None,
            source: None,
            level: level.0,
            position: 0,
        }));
        self.observe(CommandObservation::Recovery(RecoveryRecord {
            kind: RecoveryKind::Backup,
            tokens: vec![spelling],
        }));
        if self.command.alignment.active_alignment.is_some()
            && matches!(
                adjustment,
                super::AlignmentDeliveryAdjustment::BeginGroup
                    | super::AlignmentDeliveryAdjustment::EndGroup
            )
        {
            self.observe(CommandObservation::Alignment(AlignmentRecord {
                transition: "backup_correction",
                alignment: self
                    .command
                    .alignment
                    .active_alignment
                    .map(|identity| identity.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: Some(match adjustment {
                    // Input insertion leaves the just-adjusted state unchanged.
                    super::AlignmentDeliveryAdjustment::BeginGroup => {
                        self.command.alignment.align_state + 1
                    }
                    super::AlignmentDeliveryAdjustment::EndGroup => {
                        self.command.alignment.align_state - 1
                    }
                    _ => unreachable!("only unit brace adjustments emit backup correction"),
                }),
            }));
        }
    }
    /// Restores a native save level and admits aftergroup tokens directly to
    /// final input. The callback observes each value before its saved owner drops.
    pub fn end_group(
        &mut self,
        kind: tex_state::GroupKind,
        mut restore: impl FnMut(
            &mut tex_state::CommandContext<'_, G>,
            &mut tex_state::diagnostic::DiagnosticEffects,
            &tex_state::GroupRestorationEntry<G>,
        ),
    ) -> Result<tex_state::GroupFrame, CommandError> {
        let etex = self.profile().capabilities().supports_etex();
        let command = &mut *self.command;
        let host = &mut self.host;
        let fuel = &mut *self.fuel;
        let observer = &mut self.observer;
        let effects = &mut *self.diagnostic_effects;
        let mut linked = None;
        let mut failure = None;
        let frame = self
            .state
            .end_group_observed(kind, |state, event| match event {
                tex_state::GroupUnsaveEvent::Restoration(entry) => restore(state, effects, entry),
                tex_state::GroupUnsaveEvent::AfterGroup(word) if failure.is_none() => {
                    if let Some(identity) = linked {
                        command.alignment.undo_delivery(
                            AlignmentDeliveryState::<G>::back_input_adjustment(
                                word.semantic_token(),
                            ),
                        );
                        command.input.levels.link_aftergroup_word(identity, word);
                    } else {
                        let observed = observer
                            .as_mut()
                            .map(|observer| &mut **observer as &mut dyn crate::CommandObserver);
                        let mut processor = CommandProcessor::new(
                            command,
                            state,
                            host.reborrow(),
                            fuel,
                            observed,
                            effects,
                        );
                        let result = if etex {
                            processor.push_backed_up_token(
                                word,
                                backed_up_replay(core::iter::once(BackedUpToken {
                                    spelling: word,
                                })),
                            )
                        } else {
                            processor.back_input_token(word)
                        };
                        match result {
                            Ok(()) if etex => {
                                linked = processor
                                    .command
                                    .input
                                    .levels
                                    .rows
                                    .last()
                                    .map(|row| row.identity())
                            }
                            Ok(()) => {}
                            Err(error) => failure = Some(error),
                        }
                    }
                }
                tex_state::GroupUnsaveEvent::AfterGroup(_) => {}
            })
            .map_err(|_| CommandError::input_invariant())?;
        if let Some(identity) = linked {
            command.input.levels.finish_aftergroup_links(identity);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(frame)
    }

    /// Canonical backing operation used by `\\noexpand` for one replayed
    /// command. The treatment belongs to the backed-up level, not the token
    /// or the returned command.
    #[inline(always)] // pure forwarding must not add a native backup call frame
    pub(crate) fn back_input_with_treatment(
        &mut self,
        command: CurrentCommand<G>,
        treatment: BackupTreatment,
    ) -> Result<(), CommandError> {
        let adjustment = command.alignment_adjustment();
        let level = self.push_backed_up_input(command.spelling(), treatment, adjustment)?;
        if self.is_observed() {
            self.observe_backed_up_input(
                level,
                self.observed_command_spelling(&command),
                adjustment,
            );
        }
        Ok(())
    }
    /// Cancels raw brace accounting for a matched `#{` delimiter. The opening
    /// brace was delivered as parameter text, so scalar macro matching must
    /// not leave a group entry for replacement replay to balance later.
    pub(crate) fn undo_delimiter_begin_group_delivery(&mut self) {
        self.command.alignment.undo_delimiter_begin_group_delivery();
    }
}
