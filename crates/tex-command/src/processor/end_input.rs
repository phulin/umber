//! Canonical end-of-input, source exhaustion, and retirement transitions.

use tex_state::env::banks::{IntParam, TokParam};
use tex_state::token::{Catcode, Token};

use crate::error::CommandError;
use crate::input::SourceNameClass;
use crate::input::{
    InputLevel, InputLevelId, InputRetirementAction, PackedTokenSources, PackedTokenSpanHandle,
    ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior, TokenCursor,
    observed_retirement_reason,
};
use crate::observation::{CommandObservation, InputReason, InputRecord, InputTransition};

use super::CommandProcessor;

impl<G> CommandProcessor<'_, '_, G> {
    /// Runs TeX82 §1335's `final_cleanup` input unwinding.
    ///
    /// `main_control` returns as soon as §1054's `its_all_over` is true, so
    /// every input level still on the stack -- the root file at end of text,
    /// an unfinished macro body, a `\\output` token list -- is discarded
    /// without being read: `while input_ptr>0 do if state=token_list then
    /// end_token_list else end_file_reading`.  §1335 stops at `input_ptr=0`,
    /// the terminal, whose own stop is reported by the job-termination
    /// boundary; Umber's terminal level is already gone by then (it supplied
    /// only the startup filename), so the terminal stop is reported here once
    /// the stack is empty, exactly as ordinary exhaustion reports it.
    pub fn final_cleanup(&mut self) -> Vec<crate::IncompleteCondition> {
        let mut terminal_stopped = false;
        while let Some(retirement) = self.command.pop_input_level_at_end_of_job() {
            let terminal = matches!(retirement.action, InputRetirementAction::TerminalStop);
            terminal_stopped |= terminal;
            observe!(
                self,
                CommandObservation::Input(InputRecord {
                    transition: if terminal {
                        InputTransition::Stop
                    } else {
                        InputTransition::Retire
                    },
                    reason: observed_retirement_reason(retirement.action, retirement.reason),
                    source_name: retirement.name_class,
                    source: retirement.source,
                    level: retirement.identity.0,
                    position: 0,
                }),
            );
        }
        if !terminal_stopped {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Stop,
                reason: InputReason::Source,
                // tex.web §1335 unwinds down to `input_ptr=0`, which §331
                // established as the `name=0` terminal level.
                source_name: Some(SourceNameClass::Terminal),
                source: None,
                level: 0,
                position: 0,
            }));
        }
        self.command.conditions.drain_incomplete()
    }
    /// Retires the exhausted level that supplied the immediately preceding
    /// raw delivery without reading a subsequent token.
    ///
    /// TeX82's `write_out` consumes its artificial `\\endwrite` stopper
    /// after `scan_toks` has restored scanner status.  The stopper's level
    /// must retire before the caller publishes the write effect, but the
    /// following source token must remain untouched (§53).
    pub(crate) fn retire_last_delivery_level(&mut self) -> Result<(), CommandError> {
        let stamp = self.last_delivery.ok_or(CommandError::input_invariant())?;
        match self.retire_input_top(InputLevelId(stamp.input_level()))? {
            RetirementHandoff::Continue | RetirementHandoff::Completed(_) => Ok(()),
            RetirementHandoff::Stop | RetirementHandoff::EndV(_) => {
                Err(CommandError::input_invariant())
            }
        }
    }
    /// Retires the exact one-line source opened by TeX82 §483.
    ///
    /// e-TeX `\readline` consumes bytes directly instead of calling
    /// `get_token`, so it must still cross the ordinary retirement boundary
    /// that publishes §483's matching `end_file_reading` transition.
    pub(crate) fn retire_read_line_level(
        &mut self,
        level: InputLevelId,
    ) -> Result<(), CommandError> {
        match self.retire_input_top(level)? {
            RetirementHandoff::Stop if std::mem::take(&mut self.read_line_ended) => Ok(()),
            RetirementHandoff::Continue
            | RetirementHandoff::Completed(_)
            | RetirementHandoff::Stop
            | RetirementHandoff::EndV(_) => Err(CommandError::input_invariant()),
        }
    }
    pub(crate) fn retire_exhausted_through(
        &mut self,
        level: InputLevelId,
    ) -> Result<(), CommandError> {
        loop {
            let top = self
                .command
                .top_input_level_identity()
                .ok_or(CommandError::input_invariant())?;
            if top < level {
                return Ok(());
            }
            let reached = top == level;
            match self.retire_input_top(top)? {
                RetirementHandoff::Continue | RetirementHandoff::Completed(_) => {}
                RetirementHandoff::Stop | RetirementHandoff::EndV(_) => {
                    return Err(CommandError::input_invariant());
                }
            }
            if reached {
                return Ok(());
            }
        }
    }
    /// Retires depleted token-list levels without fetching the next command.
    ///
    /// Named executor boundaries use this after the command that ended an
    /// outer paragraph. TeX normally performs the same conservation loop at
    /// the next `get_next`, but a checkpoint must become quiescent before it
    /// can fetch (and therefore semantically cross) that next command.
    pub fn retire_exhausted_token_levels_for_named_boundary(
        &mut self,
    ) -> Result<usize, CommandError> {
        let mut retired = 0_usize;
        loop {
            let Some(identity) = self
                .command
                .input
                .levels
                .last()
                .and_then(|level| match level {
                    InputLevel::Tokens(cursor) => {
                        (!matches!(cursor.behavior, TokenBehavior::VTemplate)
                            && cursor
                                .token_at(PackedTokenSources::new(
                                    &self.command.input.replay,
                                    self.command.attempt.arena(),
                                ))
                                .is_none())
                        .then(|| cursor.identity())
                    }
                    InputLevel::MacroArgument(cursor) => {
                        cursor.is_exhausted().then(|| cursor.identity())
                    }
                    InputLevel::Source(_) => None,
                })
            else {
                return Ok(retired);
            };
            match self.retire_input_top(identity)? {
                RetirementHandoff::Continue | RetirementHandoff::Completed(_) => {
                    retired = retired.saturating_add(1);
                }
                RetirementHandoff::Stop | RetirementHandoff::EndV(_) => {
                    return Err(CommandError::input_invariant());
                }
            }
        }
    }
    /// Starts TeX82 §1025's already-selected output token list.
    ///
    /// Page selection and `\box255` packing belong to the stomach (§1012's
    /// `fire_up`), but the resulting token-list ownership never leaves
    /// command control.  This is the *only* way `\output` is ever entered:
    /// §1054's `its_all_over` never starts it directly, it only appends the
    /// end-job contribution trio and lets §994's `build_page` decide.
    pub fn begin_selected_output_routine(&mut self) -> Result<(), CommandError> {
        let output = self
            .state
            .token_parameter(TokParam::OUTPUT)
            .expect("output is an admitted token parameter")
            .expect("output routine entry requires a configured token list");
        self.report_named_token_list("output", output.clone());
        let words = self.state.token_list(output.clone());
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::durable(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(crate::input::StoredReplayReason::OutputRoutine),
        );
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::OutputRoutine,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }),
        );
        Ok(())
    }
    /// Implements TeX82 §1026's input-side output-routine completion.
    ///
    /// The brace that reaches `handle_right_brace` can come from inside the
    /// output token list (for example through a macro) while `output_text`
    /// still has unread tokens. TeX diagnoses that case and uses raw
    /// `get_token` delivery to discard the remainder before it calls
    /// `end_token_list`; it must not let those tokens resume in main control.
    /// Returns whether the diagnostic is required.
    pub fn finish_selected_output_routine(&mut self) -> Result<bool, CommandError> {
        let Some(output_index) = self.command.input.levels.iter().rposition(|level| {
            matches!(
                level,
                InputLevel::Tokens(TokenCursor {
                    trace: ReplayTrace::Stored(StoredReplayReason::OutputRoutine),
                    ..
                })
            )
        }) else {
            // Ordinary expanded delivery can retire a depleted output level
            // while returning its final brace. In that balanced case there
            // is no remainder for §1026 to inspect or discard.
            return Ok(false);
        };

        let output_has_remaining = match &self.command.input.levels[output_index] {
            InputLevel::Tokens(cursor) => cursor
                .token_at(PackedTokenSources::new(
                    &self.command.input.replay,
                    self.command.attempt.arena(),
                ))
                .is_some(),
            InputLevel::Source(_) => unreachable!("output replay is a token level"),
            InputLevel::MacroArgument(_) => unreachable!("output replay is not an argument"),
        };
        let levels_above_are_depleted_backups = self.command.input.levels[output_index + 1..]
            .iter()
            .all(|level| {
                matches!(
                    level,
                    InputLevel::Tokens(cursor)
                        if matches!(cursor.behavior, TokenBehavior::BackedUp(_))
                            && cursor
                                .token_at(PackedTokenSources::new(
                                    &self.command.input.replay,
                                    self.command.attempt.arena(),
                                ))
                            .is_none()
                )
            });
        let unbalanced = output_has_remaining || !levels_above_are_depleted_backups;

        if unbalanced {
            let mut discarded = None;
            while self
                .command
                .input
                .levels
                .iter()
                .find_map(|level| match level {
                    InputLevel::Tokens(cursor)
                        if cursor.identity()
                            == match &self.command.input.levels[output_index] {
                                InputLevel::Tokens(output) => output.identity(),
                                InputLevel::Source(_) => unreachable!("output token level"),
                                InputLevel::MacroArgument(_) => {
                                    unreachable!("output token level is not an argument")
                                }
                            } =>
                    {
                        Some(
                            cursor
                                .token_at(PackedTokenSources::new(
                                    &self.command.input.replay,
                                    self.command.attempt.arena(),
                                ))
                                .is_some(),
                        )
                    }
                    InputLevel::Source(_)
                    | InputLevel::Tokens(_)
                    | InputLevel::MacroArgument(_) => None,
                })
                .unwrap_or(false)
            {
                match self.get_token_into(&mut discarded)? {
                    super::DeliveryStatus::End => {
                        return Err(CommandError::input_invariant());
                    }
                    super::DeliveryStatus::Command => discarded = None,
                    _ => unreachable!("ordinary token delivery returns only commands"),
                }
            }
        }

        if unbalanced {
            self.conserve_input_stack()?;
        }
        Ok(unbalanced)
    }
    /// Retires a depleted §325 right-brace backup before synchronous output.
    ///
    /// TeX82 §1085's box-group closer can synchronously reach §1025's output
    /// hand-off through `box_end` and `build_page`. The consumed closer's
    /// one-token backup has ended before that hand-off and must be retired
    /// without fetching whatever lies beneath it.
    pub fn retire_completed_right_brace_backup(&mut self) -> Result<(), CommandError> {
        let Some(InputLevel::Tokens(cursor)) = self.command.input.levels.last() else {
            return Ok(());
        };
        if !matches!(cursor.behavior, TokenBehavior::BackedUp(_))
            || cursor
                .token_at(PackedTokenSources::new(
                    &self.command.input.replay,
                    self.command.attempt.arena(),
                ))
                .is_some()
            || !matches!(
                match cursor.span {
                    PackedTokenSpanHandle::Replay { replay, .. } => self
                        .command
                        .input
                        .replay
                        .get(replay, 0)
                        .map(|(spelling, _)| spelling.semantic_token()),
                    _ => None,
                },
                Some(Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                })
            )
        {
            return Ok(());
        }
        match self.retire_input_top(cursor.identity())? {
            RetirementHandoff::Continue => Ok(()),
            RetirementHandoff::Stop
            | RetirementHandoff::EndV(_)
            | RetirementHandoff::Completed(_) => Err(CommandError::input_invariant()),
        }
    }
    /// Settles an input-end transition after every raw typestate borrow has
    /// ended. `true` means outer-validity recovery installed new input and the
    /// caller must re-enter the pipeline.
    pub(super) fn raw_end_restarts(&mut self) -> Result<bool, CommandError> {
        // §360: a `\read` pseudo-file's line has ended, which is
        // `cur_cmd:=0; cur_chr:=0; return` -- an ordinary end of line inside
        // live `read_toks`, so no runaway recovery may run.
        if std::mem::take(&mut self.read_line_ended) {
            return Ok(false);
        }
        self.recover_runaway_eof()
    }
    pub(crate) fn acquire_source_line(
        &mut self,
        firm: bool,
    ) -> Result<Option<crate::PhysicalLine>, CommandError> {
        self.acquire_source_line_with_pending(firm, false)
    }
    pub(super) fn finish_exhausted_source(
        &mut self,
        identity: InputLevelId,
    ) -> Result<SourceExhaustionStatus, CommandError> {
        self.command
            .register_exhausted_source_backings(self.state, identity)
            .map_err(|()| CommandError::input_invariant())?;
        if let Some(level) = self.command.begin_pending_every_eof(self.state, identity) {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::EveryEof,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }));
            return Ok(SourceExhaustionStatus::Continue);
        }
        if self.state.int_param(IntParam::TRACING_NESTING) > 1 {
            let context = match self.command.input.levels.last() {
                Some(InputLevel::Source(source)) if source.identity() == identity => self
                    .command
                    .output_retiring_source_context(source, self.state),
                _ => return Err(CommandError::input_invariant()),
            };
            self.pending_file_warning_context = Some((identity, context));
        }
        match self.retire_input_top(identity)? {
            RetirementHandoff::Stop => Ok(SourceExhaustionStatus::End),
            RetirementHandoff::Completed(_) => Ok(SourceExhaustionStatus::Continue),
            RetirementHandoff::Continue => {
                let _ = self.recover_runaway_eof()?;
                Ok(SourceExhaustionStatus::Continue)
            }
            RetirementHandoff::EndV(_) => Err(CommandError::input_invariant()),
        }
    }
    fn acquire_source_line_with_pending(
        &mut self,
        firm: bool,
        pending_acquired_line: bool,
    ) -> Result<Option<crate::PhysicalLine>, CommandError> {
        let endlinechar = self.state.int_param(IntParam::END_LINE_CHAR);
        self.command
            .acquire_input_top_line(
                self.state,
                self.create_source_control_sequences,
                endlinechar,
                firm,
                pending_acquired_line,
            )
            .map_err(|()| CommandError::input_invariant())
    }
    pub(crate) fn prepare_started_input(&mut self) -> Result<crate::PhysicalLine, CommandError> {
        self.acquire_source_line_with_pending(true, true)?
            .ok_or_else(CommandError::input_invariant)
    }
    /// Retires the validated top input row and returns only the scalar phase
    /// the caller's existing delivery loop must advance to. The caller-owned
    /// command destination remains in place; retirement does not reconstruct
    /// or redispatch a command.
    pub(super) fn retire_input_top(
        &mut self,
        identity: InputLevelId,
    ) -> Result<RetirementHandoff, CommandError> {
        let nesting_context = self
            .pending_file_warning_context
            .take()
            .and_then(|(level, context)| (level == identity).then_some(context));
        let file_warning_boundary = self.prepare_file_warning_boundary(identity);
        let retirement = self
            .command
            .retire_exhausted_input_with_file_warning(identity, file_warning_boundary)
            .map_err(|_| CommandError::input_invariant())?;
        let action = retirement.action;
        let file_warning_boundary = retirement.file_warning_boundary;
        let closes_file_frame = retirement.closes_file_frame;
        // e-TeX 2.6 [23.328]'s `file_warning`: `end_file_reading` retiring a
        // real source level (never a `\read` pseudo-file's `EndReadLine`, and
        // never a token-list level) is the one point this level's recorded
        // group/conditional open depth can be compared against the live one.
        if matches!(action, InputRetirementAction::SourcePopped)
            && let Some(boundary) = file_warning_boundary
        {
            self.warn_file_boundary_incomplete(boundary, nesting_context);
        }
        // TeX82 §362 closes the file after `file_warning` and before the next
        // `check_outer_validity` diagnostic. Render the call-local retirement
        // transition at that exact point; there is no cross-step effect state.
        if closes_file_frame {
            self.state.print_file_close();
        }
        let completed = self.command.settle_input_retirement(
            retirement,
            &mut self.observer,
            &mut self.immediate_write_retirement,
        );
        match action {
            // §360's `\read` line end is `cur_cmd:=cur_chr:=0; return`: the
            // level is gone and delivery stops, rather than resuming whatever
            // §483's `begin_file_reading` buried.
            InputRetirementAction::TerminalStop => Ok(RetirementHandoff::Stop),
            InputRetirementAction::ReadLineEnded => {
                self.read_line_ended = true;
                Ok(RetirementHandoff::Stop)
            }
            InputRetirementAction::VTemplateRetained => {
                // The exhausted frame remains live while `get_next` delivers
                // frozen end-template. Its expanded `endv` is then handled by
                // typed `do_endv`, which retires this exact frame.
                Ok(RetirementHandoff::EndV(identity))
            }
            InputRetirementAction::SourcePopped
            | InputRetirementAction::TokenListPopped
            | InputRetirementAction::VTemplatePopped => {
                if let Some(episode) = completed {
                    Ok(RetirementHandoff::Completed(episode))
                } else {
                    Ok(RetirementHandoff::Continue)
                }
            }
        }
    }
    /// TeX82 §§325 and 390 clean off *every* recently depleted token list
    /// before a new one is pushed. Both sections spell the same loop, and
    /// §390's comment states its purpose for both:
    ///
    /// ```text
    /// while (state=token_list)and(loc=null)and(token_type<>v_template) do
    ///   end_token_list; {conserve stack space}
    /// ```
    ///
    /// §390 runs it before `macro_call` installs a macro body; §325 runs it as
    /// `back_input`'s first act, before the one-token `backed_up` list. The
    /// loop is generic over token-list kind by construction -- exhausted macro
    /// bodies, replayed parameters, backups, recovery insertions, and stored
    /// replay episodes all drain here -- so a macro that ends with a call to
    /// itself cannot grow the input stack without bound, and every resulting
    /// retirement is observable *before* the new level's push. `v_template` is
    /// the sections' sole exception: an exhausted v-part stays live until
    /// `do_endv` retires it.
    ///
    /// Because the condition is `loc=null` alone, this must never be narrowed
    /// to a particular token-list kind or to the level that made the last
    /// delivery: a run of levels can be depleted at once, and the whole run
    /// retires before the push.
    pub(crate) fn conserve_input_stack(&mut self) -> Result<(), CommandError> {
        self.conserve_input_stack_with_owner(None)
    }

    /// Runs §325/§390 stack conservation for a transition that will
    /// immediately install one newer input level. Every completion retired by
    /// the drained run transfers directly to that exact future owner.
    pub(crate) fn conserve_input_stack_for_descendant(&mut self) -> Result<(), CommandError> {
        let owner = InputLevelId(self.command.input.next_level_identity);
        self.conserve_input_stack_with_owner(Some(owner))
    }

    fn conserve_input_stack_with_owner(
        &mut self,
        descendant: Option<InputLevelId>,
    ) -> Result<(), CommandError> {
        loop {
            let depleted = match self.command.input.levels.last() {
                Some(InputLevel::Tokens(cursor))
                    if drains_for_stack_conservation(&cursor.behavior)
                        && cursor
                            .token_at(PackedTokenSources::new(
                                &self.command.input.replay,
                                self.command.attempt.arena(),
                            ))
                            .is_none() =>
                {
                    Some(cursor.identity())
                }
                Some(InputLevel::MacroArgument(cursor)) if cursor.is_exhausted() => {
                    Some(cursor.identity())
                }
                Some(InputLevel::Tokens(_))
                | Some(InputLevel::MacroArgument(_))
                | Some(InputLevel::Source(_))
                | None => None,
            };
            let Some(identity) = depleted else {
                return Ok(());
            };
            #[cfg(test)]
            {
                self.command
                    .raw_delivery_path_counters
                    .conservation_retirements = self
                    .command
                    .raw_delivery_path_counters
                    .conservation_retirements
                    .saturating_add(1);
            }
            let retirement = self.retire_input_top(identity)?;
            match retirement {
                // Finished stored replay episodes queue their completion in
                // command state. Draining continues so the whole depleted run
                // is cleaned off; delivery surfaces each ready ownership
                // boundary before any enclosing source.
                RetirementHandoff::Continue => {}
                RetirementHandoff::Completed(episode) => {
                    if let Some(owner) = descendant {
                        self.command.defer_replay_completion(Some(episode), owner);
                    }
                }
                RetirementHandoff::Stop | RetirementHandoff::EndV(_) => {
                    return Err(CommandError::input_invariant());
                }
            }
        }
    }
}

/// Names the tex.web §307 `token_type` one stored replay level reports.
pub(crate) fn stored_input_reason(reason: crate::input::StoredReplayReason) -> InputReason {
    use crate::input::StoredReplayReason as Stored;
    use crate::observation::UmberReplayKind;

    match reason {
        Stored::OutputRoutine => InputReason::OutputRoutine,
        Stored::EveryPar => InputReason::EveryPar,
        Stored::EveryMath => InputReason::EveryMath,
        Stored::EveryDisplay => InputReason::EveryDisplay,
        Stored::EveryHBox => InputReason::EveryHBox,
        Stored::EveryVBox => InputReason::EveryVBox,
        Stored::EveryJob => InputReason::EveryJob,
        Stored::EveryCr => InputReason::EveryCr,
        Stored::EveryEof => InputReason::EveryEof,
        Stored::Mark => InputReason::Mark,
        Stored::Write => InputReason::Write,
        Stored::Discretionary => InputReason::UmberReplay(UmberReplayKind::Discretionary),
    }
}

pub(super) enum SourceExhaustionStatus {
    Continue,
    End,
}

pub(super) enum RetirementHandoff {
    Stop,
    Continue,
    EndV(InputLevelId),
    Completed(crate::CommandReplayEpisode),
}

/// Exhaustive over [`TokenBehavior`]: TeX82 §§325 and 390's stack-conservation
/// loop excludes exactly one token type, `v_template`. A new token-list kind
/// must state which side of that rule it is on rather than inherit a default.
fn drains_for_stack_conservation(behavior: &TokenBehavior) -> bool {
    match behavior {
        TokenBehavior::Ordinary
        | TokenBehavior::Recovery
        | TokenBehavior::MacroBody(_)
        | TokenBehavior::Parameter
        | TokenBehavior::BackedUp(_)
        | TokenBehavior::UTemplate => true,
        TokenBehavior::VTemplate => false,
    }
}
