//! Canonical raw command delivery.
//!
//! This is the sole scalar path from input levels to `CurrentCommand<G>`, after
//! TeX.web §341 (`get_next`).  Later scanner and alignment milestones extend
//! the two explicit entry points below; they do not add another lexical path.

use tex_state::env::banks::{IntParam, TokParam};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::command::{CurrentCommand, DeliveryStamp, ResolvedCommand};
use crate::error::CommandError;
use crate::input::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, InputRetirementAction,
    InputTopTransition, PackedTokenSources, PackedTokenSpanHandle, ReplayTrace, RetirementBehavior,
    StoredReplayReason, TokenBehavior, TokenCursor,
};
// tex.web §303's `name` classification only reaches an observation payload.
use crate::input::SourceNameClass;
use crate::{AlignmentDelivery, AlignmentDeliveryEvent, CommandReplayDelivery};

use super::CommandProcessor;
use super::expand::{ExpandedFetch, ProtectedMacroHandling, UndefinedHandling};
use super::status::{EofLegality, RecoveryContext, ScannerStatus};
use super::{
    AlignmentInterceptionPolicy, DeliveryMode, DeliveryPolicy, ExpandedDeliveryPolicy,
    ExpandedObservationPolicy, FirstCommandPolicy, ReplayCompletionPolicy,
};

/// TeX82 §336's `Incomplete \if...; all text was ignored after line N`.
///
/// Distinct from `conditionals`'s own incomplete-`\if` recovery identity:
/// this one is `check_outer_validity` finding a live skipping episode, not
/// §500 inserting frozen `\relax` in the middle of a condition.
const INCOMPLETE_CONDITIONAL_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0336;

/// TeX82 §338's `File ended` / `Forbidden control sequence found` while
/// scanning a definition, use, preamble or text.
pub(crate) const RUNAWAY_SCAN_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0338;

/// TeX82 §345's invalid source-character report.
///
/// The tokenizer has already consumed the character when this is recorded;
/// raw delivery reports it with deletions disabled and then restarts at the
/// following character instead of producing a token for it.
const INVALID_SOURCE_CHARACTER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0345;

use super::alignment::AlignmentDeliveryState;
use super::alignment::CELL_ALIGN_STATE;

use crate::input::InputRetirementReason;
use crate::observation::{
    AlignmentRecord, CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation,
    CommandProvenance, DiagnosticArgument, DiagnosticRecord, InputReason, InputRecord,
    InputTransition, RecoveryKind, RecoveryRecord, observed_token,
};

impl<G> CommandProcessor<'_, '_, G> {
    /// Reports TeX82 §1096's `hmode+par_end` recovery predicate.
    ///
    /// `align_state` belongs to raw command delivery, so the stomach asks
    /// this command-owned question instead of mirroring the brace counter.
    #[must_use]
    pub fn paragraph_end_needs_alignment_recovery(&self) -> bool {
        self.command.alignment.align_state < 0
    }

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
            RetirementHandoff::Continue | RetirementHandoff::Completed => Ok(()),
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
            | RetirementHandoff::Completed
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
                RetirementHandoff::Continue | RetirementHandoff::Completed => {}
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
                RetirementHandoff::Continue | RetirementHandoff::Completed => {
                    retired = retired.saturating_add(1);
                }
                RetirementHandoff::Stop | RetirementHandoff::EndV(_) => {
                    return Err(CommandError::input_invariant());
                }
            }
        }
    }

    /// Delivers one expanded command, separating an intercepted alignment
    /// delimiter from ordinary main-control delivery.
    ///
    /// No executor-side classifier is involved: `get_next` has already made
    /// the canonical `align_state` decision before this method observes the
    /// frozen `end_template` meaning.
    ///
    /// This must use the completion-aware raw fetch, not plain `get_next`.
    /// An alignment cell's body can itself contain an executor-owned replay
    /// episode (a math field, math-group/choice branch, or discretionary
    /// part -- for example `\vphantom`'s `\mathchoice` inside an inline `$#$`
    /// cell template). Plain `get_next` silently swallows that episode's
    /// retirement and keeps cascading to whatever real token follows, which
    /// can belong to the *enclosing* context rather than the episode; the
    /// caller (`scan_alignment_delivery_step`) needs `Completed` surfaced so
    /// it can report `ColdOperation::ReplayCompleted` exactly as ordinary
    /// (non-alignment) `scan_step` already does via
    /// `get_x_token_with_replay_completion`.
    ///
    /// `main_loop_active` reports whether `main_control` is parked at TeX82
    /// §1034's `main_loop_lookahead` rather than at §1030's `big_switch`. An
    /// alignment cell body is ordinary `main_control` material, so the same
    /// §1038 rule holds inside it: the first fetch is a bare `get_next`, and
    /// a `letter`/`other_char`/`char_given` never reaches `x_token`.
    ///
    /// It also selects which of §380's two expanded fetches this is, and the
    /// two disagree about the `end_template` that closes a cell's ⟨v_j⟩
    /// template -- see [`ExpandedFetch`].
    pub fn get_x_alignment_delivery(
        &mut self,
        main_loop_active: bool,
    ) -> Result<Option<AlignmentDelivery<G>>, CommandError> {
        let mut destination = None;
        let delivery = self.get_x_alignment_delivery_into(main_loop_active, &mut destination)?;
        Ok(match delivery {
            super::DeliveryStatus::End => None,
            super::DeliveryStatus::Command => Some(AlignmentDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            super::DeliveryStatus::ReplayCompleted(episode) => {
                Some(AlignmentDelivery::Completed(episode))
            }
            super::DeliveryStatus::AlignmentEndTemplate => Some(AlignmentDelivery::Event(
                AlignmentDeliveryEvent::EndTemplate(
                    destination.expect("alignment status initializes destination"),
                ),
            )),
            super::DeliveryStatus::AlignmentClosingBrace => Some(AlignmentDelivery::Event(
                AlignmentDeliveryEvent::ClosingBrace(
                    destination.expect("alignment status initializes destination"),
                ),
            )),
            super::DeliveryStatus::PendingExpanded => {
                unreachable!("alignment delivery commits terminal observations")
            }
        })
    }

    /// Delivers active-cell input into caller-provided command storage.
    pub fn get_x_alignment_delivery_into(
        &mut self,
        main_loop_active: bool,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        let fetch = if main_loop_active {
            ExpandedFetch::XToken
        } else {
            ExpandedFetch::GetXToken
        };
        let delivery = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch,
                    protected_macros: ProtectedMacroHandling::Expand,
                    undefined: UndefinedHandling::Diagnose,
                    observation: ExpandedObservationPolicy::Commit,
                    first_command: if main_loop_active {
                        FirstCommandPolicy::MainLoopCharacter
                    } else {
                        FirstCommandPolicy::Ordinary
                    },
                }),
                replay_completion: ReplayCompletionPolicy::Surface,
                alignment_interception: AlignmentInterceptionPolicy::Surface,
            },
            destination,
        )?;
        Ok(delivery)
    }

    /// Hands an intercepted delimiter from `end_template` main control back
    /// to canonical input, then starts the active cell's v-template above it.
    /// The delimiter is never classified by an executor-side loop: after the
    /// suffix retires, raw `get_next` sees it again with its original spelling.
    pub fn begin_alignment_v_template(
        &mut self,
        alignment: crate::AlignmentIdentity,
        event: AlignmentDeliveryEvent<G>,
    ) -> Result<(), CommandError> {
        let (saved_delimiter, delimiter_line) = match event {
            AlignmentDeliveryEvent::EndTemplate(delimiter) => {
                if self.last_delivery != Some(delimiter.delivery_stamp())
                    || !matches!(
                        delimiter.meaning(),
                        tex_state::ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                            ExpandablePrimitive::EndTemplate
                        ))
                    )
                {
                    return Err(CommandError::StaleDelivery);
                }
                self.last_delivery = None;
                (
                    Self::saved_alignment_delimiter(&delimiter)?,
                    delimiter
                        .direct_source_line_number()
                        .unwrap_or_else(|| self.command.current_file_line_number()),
                )
            }
            AlignmentDeliveryEvent::ClosingBrace(_) => {
                return Err(CommandError::input_invariant());
            }
        };
        self.start_alignment_v_template(alignment, saved_delimiter, delimiter_line)
    }

    /// Completes TeX82 §1131's `do_endv` input-stack proof and cell
    /// transition. The processor performs the proof because `loc_field=null`
    /// for stored token lists can only be decided while the immutable token
    /// store is borrowed; the persistent command state deliberately owns no
    /// such store capability.
    pub fn finish_alignment_cell(
        &mut self,
        alignment: crate::AlignmentIdentity,
    ) -> Result<crate::FinishedAlignmentCell, CommandError> {
        let v_level = self
            .command
            .alignment
            .active_v_template_level(alignment)
            .map_err(|_| CommandError::input_invariant())?;
        let mut found = false;
        for level in self.command.input.levels.iter().rev() {
            // TeX82 §1131 walks downward only while `state=token_list` and
            // `loc=null`. A live token either in the v-template itself or in
            // an interposed token-list frame is the canonical interwoven-
            // preamble fatal path, not an internal Rust invariant failure.
            match level {
                InputLevel::Tokens(cursor) => {
                    if cursor
                        .token_at(PackedTokenSources::new(
                            &self.command.input.replay,
                            self.command.attempt.arena(),
                        ))
                        .is_some()
                    {
                        break;
                    }
                    if cursor.identity() == v_level
                        && matches!(cursor.behavior, TokenBehavior::VTemplate)
                        && matches!(
                            cursor.retirement,
                            RetirementBehavior::AwaitingVTemplateRetirement
                        )
                    {
                        found = true;
                        break;
                    }
                }
                InputLevel::MacroArgument(cursor) => {
                    if !cursor.is_exhausted() {
                        break;
                    }
                }
                InputLevel::Source(_) => break,
            }
        }
        if !found {
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                "(interwoven alignment preambles are not allowed)",
            )));
        }
        // TeX82 §791 performs this independently of §1131's input-stack
        // proof: another preamble may leave the expected exhausted frame in
        // place while its brace sentinel is interwoven with the active cell.
        if self.command.alignment.align_state < 500_000 {
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                "(interwoven alignment preambles are not allowed)",
            )));
        }
        self.command
            .finish_alignment_cell_after_input_proof(alignment)
            .map_err(|_| CommandError::input_invariant())
    }

    /// Performs TeX82 §1127's `align_error`, the whole of §1126's
    /// `any_mode(car_ret), any_mode(tab_mark)` action.
    ///
    /// `Ok(None)` is §1128's `@<Express consternation over the fact that no
    /// alignment is in progress@>` branch (`abs(align_state)>2`): the
    /// delimiter is reported and dropped, with no backup and no insertion.
    /// `Ok(Some(brace))` is the `abs(align_state)<=2` branch: the delimiter is
    /// backed up and the missing brace is inserted above it by `ins_error`.
    /// Both backup levels and every `align_state` adjustment remain
    /// command-owned.
    ///
    /// The caller passes the delimiter exactly as main control received it;
    /// this is not an alignment-delivery event, because tex.web reaches
    /// `align_error` from `main_control`, not from `get_next`.
    pub fn recover_align_error(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<Option<Token>, CommandError> {
        let previous = self.command.alignment.align_state;
        if previous.unsigned_abs() > 2 {
            return Ok(None);
        }
        self.back_input(command)?;
        let recovery = self
            .command
            .alignment
            .correct_unbalanced_delimiter()
            .ok_or(CommandError::input_invariant())?;
        let recovery_name = match recovery {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => "missing_left_brace",
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => "missing_right_brace",
            _ => return Err(CommandError::input_invariant()),
        };
        self.observe_unbalanced_delimiter_correction(recovery_name, previous);
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient([TracedTokenWord::pack(recovery, OriginId::UNKNOWN)]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        self.observe_inserted_token_recovery(level, recovery);
        let before_backup = self.command.alignment.align_state;
        self.command
            .alignment
            .correct_inserted_brace_backup(recovery);
        self.observe_alignment_backup_correction(before_backup);
        Ok(Some(recovery))
    }

    /// Performs TeX82 §1132's `align_group` `handle_right_brace` recovery.
    ///
    /// The executor selects this structural branch only after it has observed
    /// its active entry `align_group`; this command-core operation retains the
    /// delivered brace's raw backup, its alignment correction, and insertion
    /// of the inaccessible frozen `\\cr`. It does not close the group: the
    /// inserted row terminator reaches alignment delivery before brace replay.
    pub fn recover_alignment_closing_brace(
        &mut self,
        event: AlignmentDeliveryEvent<G>,
    ) -> Result<(), CommandError> {
        let AlignmentDeliveryEvent::ClosingBrace(command) = event else {
            return Err(CommandError::input_invariant());
        };
        if !matches!(
            command.meaning(),
            tex_state::ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        ) {
            return Err(CommandError::input_invariant());
        }
        // The brace arrived by replaying the next-cell opener backup. §325's
        // stack-conservation loop retires that exhausted backup before §1132
        // makes its own backup.
        self.back_input(command)?;
        let frozen_cr = self
            .state
            .primitive_token("cr")
            .ok_or(CommandError::input_invariant())?;
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient([TracedTokenWord::pack(frozen_cr, OriginId::UNKNOWN)]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        if self.is_observed() {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }));
            self.observe(CommandObservation::Recovery(RecoveryRecord {
                // `ins_error` records this as a token insertion even though
                // the concrete recovery token is TeX82's frozen `\\cr`.
                kind: RecoveryKind::InsertedToken,
                // TeX82's inaccessible frozen control sequence retains the
                // canonical `\\cr` spelling in observer transport.
                tokens: vec![crate::observation::ObservedToken::ControlSequence(
                    "cr".into(),
                )],
            }));
        }
        Ok(())
    }

    fn saved_alignment_delimiter(
        command: &CurrentCommand<G>,
    ) -> Result<crate::AlignmentCellDelimiter, CommandError> {
        match command.alignment_adjustment() {
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(
                crate::processor::alignment::AlignmentDelimiter::Tab,
            ) => Ok(crate::AlignmentCellDelimiter::Tab),
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(
                crate::processor::alignment::AlignmentDelimiter::Span,
            ) => Ok(crate::AlignmentCellDelimiter::Span),
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(
                crate::processor::alignment::AlignmentDelimiter::Cr
                | crate::processor::alignment::AlignmentDelimiter::CrCr,
            ) => Ok(crate::AlignmentCellDelimiter::Row),
            _ => Err(CommandError::input_invariant()),
        }
    }

    /// TeX82 saves this typed outcome in `extra_info(cur_align)` for
    /// `fin_col`; after `do_endv` it selects structural continuation without
    /// re-entering raw delimiter delivery.
    fn start_alignment_v_template(
        &mut self,
        alignment: crate::AlignmentIdentity,
        saved_delimiter: crate::AlignmentCellDelimiter,
        delimiter_line: u32,
    ) -> Result<(), CommandError> {
        self.command
            .begin_alignment_v_template(self.state, alignment, saved_delimiter, delimiter_line)
            .map_err(|_| CommandError::input_invariant())?;
        if let Some(input) = self
            .command
            .alignment_v_template_push_observation(alignment)
        {
            self.observe(CommandObservation::Input(input));
            if let Some(template) = self
                .command
                .alignment_v_template_push_alignment_observation(alignment)
            {
                self.observe(CommandObservation::Alignment(template));
            }
            self.observe(CommandObservation::Alignment(AlignmentRecord {
                transition: "state_change",
                alignment: Some(alignment.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: Some(CELL_ALIGN_STATE),
            }));
        }
        Ok(())
    }

    /// Delivers one unexpanded raw command through canonical `get_next`.
    pub fn get_next(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_next_into(&mut destination)? {
            super::DeliveryStatus::End => Ok(None),
            super::DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary raw delivery returns only commands"),
        }
    }

    /// Delivers one raw command directly into caller-provided final storage.
    pub fn get_next_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        let delivery = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Consume,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        )?;
        debug_assert!(matches!(
            delivery,
            super::DeliveryStatus::End | super::DeliveryStatus::Command
        ));
        Ok(delivery)
    }

    /// Delivers one raw command or an executor-owned stored-episode
    /// completion. This is the raw counterpart of
    /// [`Self::get_x_token_with_replay_completion`].
    pub fn get_next_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let delivery = self.get_next_with_replay_completion_into(&mut destination)?;
        Ok(match delivery {
            super::DeliveryStatus::End => None,
            super::DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            super::DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("raw replay-aware delivery has no expanded event"),
        })
    }

    /// Delivers raw replay-aware input into caller-provided command storage.
    pub fn get_next_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        let delivery = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Surface,
                alignment_interception: AlignmentInterceptionPolicy::None,
            },
            destination,
        )?;
        debug_assert!(matches!(
            delivery,
            super::DeliveryStatus::End
                | super::DeliveryStatus::Command
                | super::DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(delivery)
    }

    /// Delivers the raw token following TeX's backtick character-code
    /// introducer.
    ///
    /// §442 reads it with `get_token`, so the delivery is an ordinary raw
    /// command whose identity is its own category code -- the *scanner's*
    /// later interpretation of `cur_chr` is category-independent, the
    /// delivery is not. This observed nothing of its own until
    /// `umber2-johp.141`: it used to force the observed spelling to
    /// `other_char`, which existed only to feed a spelling-derived command
    /// name in the transport and silently masked whatever category code the
    /// engine actually held.
    ///
    /// It is `get_token`, not `get_next`, for the further reason §365 gives:
    /// `get_token` is one of the two places TeX82 clears
    /// `no_new_control_sequence`, so `` \`\newname `` enters `newname` in the
    /// hash table exactly as any other `get_token` reader would.
    pub(crate) fn get_next_character_code(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_token_into(&mut destination)? {
            super::DeliveryStatus::End => return Ok(None),
            super::DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        if let Some(command) = &destination
            && matches!(
                command.spelling().semantic_token(),
                Token::Char {
                    cat: Catcode::BeginGroup | Catcode::EndGroup,
                    ..
                }
            )
        {
            // TeX82 §442 immediately cancels `get_next`'s brace update
            // when a brace token supplies an alphabetic character constant.
            // The token is consumed as a character code, not as grouping
            // material, so a following alignment delimiter must still see
            // the entry's original `align_state`.
            self.command
                .alignment
                .undo_delivery(command.alignment_adjustment());
        }
        Ok(destination)
    }

    /// Delivers one raw token for consumers which canonically permit a new
    /// source control-sequence spelling.
    pub fn get_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_token_into(&mut destination)? {
            super::DeliveryStatus::End => Ok(None),
            super::DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
    }

    /// Delivers one raw token directly into caller-provided final storage.
    pub fn get_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        debug_assert!(!self.create_source_control_sequences);
        self.create_source_control_sequences = true;
        let delivery = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Consume,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        );
        self.create_source_control_sequences = false;
        let delivery = delivery?;
        debug_assert!(matches!(
            delivery,
            super::DeliveryStatus::End | super::DeliveryStatus::Command
        ));
        Ok(delivery)
    }

    /// Applies tex.web §§84/87's ErrorStop input mutation at the sole raw
    /// command/input ownership boundary.
    #[doc(hidden)]
    pub fn apply_error_stop_recovery(
        &mut self,
        mut request: tex_state::print::ErrorRecoveryRequest,
    ) -> Result<(), CommandError> {
        loop {
            match request {
                tex_state::print::ErrorRecoveryRequest::Delete(count) => {
                    let mut deleted = None;
                    for _ in 0..count {
                        match self.get_token_into(&mut deleted)? {
                            super::DeliveryStatus::End => break,
                            super::DeliveryStatus::Command => deleted = None,
                            _ => unreachable!("ordinary token delivery returns only commands"),
                        }
                    }
                    let context = self.error_context();
                    self.state.printer().print_rendered(&context);
                    match self.state.continue_error_stop_dialog(&context) {
                        tex_state::print::ErrorOutcome::Continue => return Ok(()),
                        tex_state::print::ErrorOutcome::Recovery(next) => request = next,
                        tex_state::print::ErrorOutcome::JumpOut(jump) => return Err(jump.into()),
                    }
                }
                tex_state::print::ErrorRecoveryRequest::Insert(line) => {
                    self.command
                        .open_error_insert_line(line.into_bytes())
                        .map_err(|_| CommandError::input_invariant())?;
                    return Ok(());
                }
            }
        }
    }

    /// Completes one synchronous error dialogue while this processor owns the
    /// sole mutable command-input route.
    pub(crate) fn finish_error_outcome(
        &mut self,
        outcome: tex_state::print::ErrorOutcome,
    ) -> Result<(), CommandError> {
        match outcome {
            tex_state::print::ErrorOutcome::Continue => Ok(()),
            tex_state::print::ErrorOutcome::Recovery(request) => {
                self.apply_error_stop_recovery(request)
            }
            tex_state::print::ErrorOutcome::JumpOut(jump) => Err(jump.into()),
        }
    }

    /// Restores the immediately preceding raw delivery to TeX's input.
    ///
    /// This is TeX.web's `back_input`: token equality is insufficient because
    /// equal spellings can be delivered by distinct input transitions. The
    /// consumed command proves the exact live transition and ensures literal
    /// brace accounting is undone at most once.
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
        let mut destination = None;
        match self.get_token_into(&mut destination)? {
            super::DeliveryStatus::End => return Ok(false),
            super::DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let next = destination
            .take()
            .expect("command status initializes destination");
        if outer_horizontal && is_math_shift(&next) {
            Ok(true)
        } else {
            self.back_input(next)?;
            Ok(false)
        }
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
        let mut destination = None;
        match self.get_x_token_into(&mut destination)? {
            super::DeliveryStatus::End => return Ok(false),
            super::DeliveryStatus::Command => {}
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
        let next = destination
            .take()
            .expect("command status initializes destination");
        if is_math_shift(&next) {
            Ok(true)
        } else {
            self.back_input(next)?;
            Ok(false)
        }
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
            PackedTokenSpanHandle::backed_up(tokens),
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
    /// and pushes a one-token `backed_up` list. No delivery stamp is involved,
    /// so this serves every caller whose token is not the last raw delivery:
    ///
    /// - §372's `\\csname`: `cur_tok:=cur_cs+cs_token_flag; back_input` backs
    ///   up a control sequence that was never delivered at all.
    /// - §282's `unsave`, through §326: each `insert_token` entry left by
    ///   `\\aftergroup` is backed up as the group's save-stack level is
    ///   cleared off, long after that token was scanned.
    ///
    /// [`Self::back_input_saved`] is the sibling for a caller that still holds
    /// the `CurrentCommand<G>`: §342's alignment interception records transitions
    /// that set `align_state` outright rather than stepping it, so a delivery
    /// that is available must have its own adjustment reversed, not one
    /// recomputed from the token.
    pub fn back_input_token(&mut self, spelling: TracedTokenWord) -> Result<(), CommandError> {
        self.conserve_input_stack()?;
        self.command
            .alignment
            .undo_delivery(AlignmentDeliveryState::<G>::back_input_adjustment(
                spelling.semantic_token(),
            ));
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::backed_up([BackedUpToken {
                spelling,
                source_provenance: None,
            }]),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        if self.is_observed() {
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
                tokens: vec![self.observed_token(spelling)],
            }));
        }
        Ok(())
    }

    /// Replays one group's `\aftergroup` tokens in save order.
    ///
    /// TeX82 §282 invokes §326 once for every `insert_token` save entry.
    /// e-TeX 2.6 etex.ch [15.282] optimizes the second and later entries:
    /// after the first `back_input`, it links each token directly onto that
    /// same `backed_up` list. Those direct links adjust `align_state`, but do
    /// not push or observe another input level.
    pub fn back_input_aftergroup_tokens(
        &mut self,
        tokens: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<(), CommandError> {
        let mut tokens = tokens.into_iter().collect::<Vec<_>>();
        let Some(last) = tokens.pop() else {
            return Ok(());
        };
        self.back_input_token(last)?;
        if self.profile().capabilities().supports_etex() {
            let prepended = tokens.len();
            for spelling in tokens.iter().rev() {
                self.command.record_alignment_phase();
                self.command.alignment.undo_delivery(
                    AlignmentDeliveryState::<G>::back_input_adjustment(spelling.semantic_token()),
                );
            }
            let Some(InputLevel::Tokens(cursor)) = self.command.input.levels.last() else {
                unreachable!("back_input above installed a token-list level");
            };
            assert_eq!(
                cursor.position(),
                0,
                "no delivery occurs while e-TeX links aftergroup tokens"
            );
            let PackedTokenSpanHandle::Replay { replay, .. } = cursor.span else {
                unreachable!("back_input above installed a replay payload");
            };
            let admitted = self
                .command
                .input
                .replay
                .prepend_backed_up(
                    replay,
                    tokens.into_iter().map(|spelling| BackedUpToken {
                        spelling,
                        source_provenance: None,
                    }),
                )
                .map_err(|_| CommandError::input_invariant())?;
            let Ok(prepended) = u32::try_from(prepended) else {
                return Err(CommandError::input_invariant());
            };
            debug_assert_eq!(admitted, prepended);
            let Some(InputLevel::Tokens(cursor)) = self.command.input.levels.last_mut() else {
                unreachable!("back_input above installed a token-list level");
            };
            if cursor.frame.extend_limit(prepended).is_none() {
                return Err(CommandError::input_invariant());
            }
        } else {
            for spelling in tokens.into_iter().rev() {
                self.back_input_token(spelling)?;
            }
        }
        Ok(())
    }

    /// Performs TeX82 §1095's `head_for_vmode` replay for a stop command.
    ///
    /// In outer horizontal mode, `\\end` is not yet eligible for final
    /// termination.  TeX backs the stop command up, sets `cur_tok` to the
    /// primitive `\\par`, and backs that synthesized token up with inserted
    /// token-list ownership.  The executor subsequently applies only the
    /// typed paragraph transition; all raw input and recovery observations
    /// remain here.
    pub fn recover_stop_for_vertical_mode(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        self.insert_par_before(command)
    }

    /// Performs Web2C `partoken.ch`'s boundary replay for a positive
    /// `\partokencontext`.
    ///
    /// The boundary command is backed up first and the current `\par`
    /// control sequence is installed above it with `inserted` ownership.
    /// This is the same raw-input transition as TeX82 §1095's
    /// `head_for_vmode`; only the caller's boundary predicate differs.
    pub fn insert_partoken_before(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        self.insert_par_before(command)
    }

    fn insert_par_before(&mut self, command: CurrentCommand<G>) -> Result<(), CommandError> {
        self.back_input(command)?;
        let par = TracedTokenWord::pack(
            Token::Cs(
                self.state
                    .symbol("par")
                    .ok_or(CommandError::input_invariant())?,
            ),
            OriginId::UNKNOWN,
        );
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient([par]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        if self.is_observed() {
            // `head_for_vmode` calls `back_input` after assigning `cur_tok`;
            // the push is therefore observed as backup even though its
            // inserted ownership makes retirement a recovery transition.
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
                tokens: vec![self.observed_token(par)],
            }));
        }
        Ok(())
    }

    /// Installs TeX82 §395's `par_token` after an extra right brace in a macro
    /// argument. The offending brace has already been backed up.
    ///
    /// Unlike §336's frozen outer-validity recovery, §395 assigns the token
    /// for the ordinary `\par` control sequence. Its meaning is therefore
    /// resolved when the inserted token is delivered, including a meaning
    /// that the user has reassigned since INITEX installed the primitive.
    pub(crate) fn insert_macro_argument_recovery_par(&mut self) -> Result<(), CommandError> {
        let par = Token::Cs(
            self.state
                .symbol("par")
                .ok_or(CommandError::input_invariant())?,
        );
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient([TracedTokenWord::pack(par, OriginId::UNKNOWN)]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        self.observe_inserted_token_recovery(level, par);
        Ok(())
    }

    /// Performs TeX82 §1047's `insert_dollar_sign` replay for a command that
    /// §1046 lists among the "math-only cases in non-math modes, or vice
    /// versa" (for example `mmode+hrule`). TeX backs the offending command
    /// up, sets `cur_tok` to an inserted `$`, and backs that synthesized
    /// token up too, so the next two deliveries close the current math (or
    /// non-math) mode and then replay the original command in the resulting
    /// mode. The executor is responsible for the accompanying "Missing $
    /// inserted" diagnostic text; only the raw input recovery lives here.
    pub fn recover_missing_math_shift(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        self.back_input(command)?;
        let dollar_token = Token::Char {
            ch: '$',
            cat: Catcode::MathShift,
        };
        let dollar = TracedTokenWord::pack(dollar_token, OriginId::UNKNOWN);
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient([dollar]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        // TeX82 §1047 calls `ins_error`, whose §323 `token_type:=inserted`
        // reclassifies the level created by `back_input`. Observe that final
        // canonical ownership, not the helper used to allocate the level.
        self.observe_inserted_token_recovery(level, dollar_token);
        Ok(())
    }

    /// Pushes TeX82 §327's synthesized `ins_error` token above any
    /// ordinary input that the caller has already backed up.
    ///
    /// `ins_error` is not merely a diagnostic annotation: it runs
    /// `back_input`, then changes that new level's `token_type` to `inserted`
    /// before §82 calls `show_context`. Keeping this as a distinct live level
    /// is what gives the recovery token its `<inserted text>` context while an
    /// enclosing §325 backup remains `<to be read again>`.
    pub(crate) fn push_inserted_error_token(&mut self, token: Token) {
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient([TracedTokenWord::pack(token, OriginId::UNKNOWN)]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        self.observe_inserted_token_recovery(level, token);
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
            RetirementHandoff::Stop | RetirementHandoff::EndV(_) | RetirementHandoff::Completed => {
                Err(CommandError::input_invariant())
            }
        }
    }

    /// Performs TeX82 §1064 `off_save` input recovery for a command.
    ///
    /// `closing` holds the one or more tokens §1065 prepares to match the
    /// current group (a single `}`/`$` character, or the two-token
    /// `\right.` a `math_left_group` needs); they replay in order, ahead of
    /// the backed-up command. The executor selects `closing` from its actual
    /// group, but raw backup, inserted-token ownership, and observer order
    /// remain in the command core. This is used by §1131 `do_endv` and by
    /// ordinary main control when an inaccessible closer must first end an
    /// intervening group.
    pub fn recover_off_save(
        &mut self,
        command: CurrentCommand<G>,
        closing: &[Token],
    ) -> Result<(), CommandError> {
        observe!(
            self,
            CommandObservation::Diagnostic(crate::observation::DiagnosticRecord {
                severity: "error",
                diagnostic: "off_save_replay",
                arguments: vec![DiagnosticArgument::Token(
                    self.observed_command_spelling(&command),
                )],
            },),
        );
        self.back_input(command)?;
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient(
                closing
                    .iter()
                    .map(|&token| TracedTokenWord::pack(token, OriginId::UNKNOWN)),
            ),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        if self.is_observed() {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }));
            self.observe(CommandObservation::Recovery(RecoveryRecord {
                kind: RecoveryKind::InsertedToken,
                tokens: closing
                    .iter()
                    .map(|&token| {
                        self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN))
                    })
                    .collect(),
            }));
        }
        Ok(())
    }

    /// Publishes TeX82 §§1064 and 1066's bottom-level `off_save` drop.
    ///
    /// The command remains on its existing backup level, which raw input
    /// retires on the following delivery attempt. Keeping the diagnostic here
    /// makes that observer order command-owned just like replay recovery.
    pub fn report_off_save_bottom_drop(&mut self, command: &CurrentCommand<G>) {
        self.observe_command_diagnostic("off_save_bottom_drop", command);
    }

    /// Performs TeX82 §1131's end-v instance of [`Self::recover_off_save`].
    pub fn recover_endv_off_save(
        &mut self,
        command: CurrentCommand<G>,
        closing: Token,
    ) -> Result<(), CommandError> {
        self.recover_off_save(command, &[closing])
    }

    /// Looks up the frozen (redefinition-proof) control-sequence token for a
    /// primitive by its canonical name, e.g. TeX82's `frozen_end_group` /
    /// `frozen_right`, the tokens §1065's `off_save` inserts for a
    /// `semi_simple_group` / `math_left_group` respectively so that a
    /// user redefinition of `\endgroup` or `\right` cannot change what the
    /// recovery closes.
    pub fn frozen_primitive_token(&self, name: &str) -> Result<Token, CommandError> {
        self.state
            .primitive_token(name)
            .ok_or(CommandError::input_invariant())
    }

    /// Restores a command and records the diagnostic selected by `back_error`.
    ///
    /// Full diagnostic-text rendering for the identities this records remains
    /// a later milestone; keeping its accounting here ensures recovery input
    /// remains ordinary input after the one backup transition.
    pub(crate) fn back_error(
        &mut self,
        command: CurrentCommand<G>,
        diagnostic: u64,
    ) -> Result<(), CommandError> {
        self.back_input(command)?;
        self.command.timeline.record_expansion_diagnostic_push();
        self.command.expansion.pending_diagnostics.push(diagnostic);
        Ok(())
    }

    /// [`Self::back_error`] that also composes the report §82 will render.
    ///
    /// TeX82's `back_error` is `back_input` *then* `error`, so the context is
    /// captured with the backed-up level already on the stack -- which is
    /// exactly what makes the display's `<to be read again>` line name the
    /// offending token.
    pub(crate) fn back_error_reporting(
        &mut self,
        command: CurrentCommand<G>,
        diagnostic: u64,
        message: String,
        help: &'static [&'static str],
    ) -> Result<(), CommandError> {
        self.back_error(command, diagnostic)?;
        let context = self.command.output_open_context(self.state);
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: diagnostic,
                runaway: None,
                message,
                help,
                context,
                integer_error: None,
            });
        Ok(())
    }

    /// The plain-`error` sibling of [`Self::back_error_reporting`], for a
    /// §82 call that backs nothing up.
    pub(crate) fn report_recoverable(
        &mut self,
        diagnostic: u64,
        message: String,
        help: &'static [&'static str],
    ) {
        self.command.timeline.record_expansion_diagnostic_push();
        self.command.expansion.pending_diagnostics.push(diagnostic);
        let context = self.command.output_open_context(self.state);
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: diagnostic,
                runaway: None,
                message,
                help,
                context,
                integer_error: None,
            });
    }

    /// Canonical backing operation used by `\\noexpand` for one replayed
    /// command. The treatment belongs to the backed-up level, not the token
    /// or the returned command.
    pub(crate) fn back_input_with_treatment(
        &mut self,
        command: CurrentCommand<G>,
        treatment: BackupTreatment,
    ) -> Result<(), CommandError> {
        let stamp = command.delivery_stamp();
        if self.last_delivery != Some(stamp) {
            return Err(CommandError::StaleDelivery);
        }
        self.back_input_unchecked(command, treatment)
    }

    /// TeX82 §326's `@<Insert token |p| into \TeX's input@>`:
    /// `t:=cur_tok; cur_tok:=p; back_input; cur_tok:=t`.
    ///
    /// This is a full §325 `back_input` -- stack-conservation loop, literal
    /// brace `align_state` reversal, backup push, and recovery record -- run
    /// against a raw delivery the caller saved earlier instead of against the
    /// live one. §325 requires only that `cur_tok` hold the token to replace,
    /// and §326 exists precisely so a caller may point `cur_tok` at a saved
    /// token first, so the delivery stamp is not part of the mechanism.
    ///
    /// §1221's `\futurelet` is the canonical caller: `get_token; q:=cur_tok;
    /// get_token; back_input; cur_tok:=q; back_input`. The second token is
    /// restored by the ordinary `back_input` above and the saved first token
    /// by this one, so the pair is reread in its original order from two
    /// separate backup levels.
    pub(crate) fn back_input_saved(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        self.back_input_unchecked(command, BackupTreatment::Ordinary)
    }

    fn back_input_unchecked(
        &mut self,
        command: CurrentCommand<G>,
        treatment: BackupTreatment,
    ) -> Result<(), CommandError> {
        self.last_delivery = None;
        // §325 runs the stack-conservation loop before it touches
        // `align_state` and before it pushes the `backed_up` list, so every
        // depleted level retires ahead of the backup.
        self.conserve_input_stack()?;
        let previous_align_state = self.command.alignment.align_state;
        let adjustment = command.alignment_adjustment();
        self.undo_alignment_delivery(&command);

        let level = self.command.push_token_level(
            PackedTokenSpanHandle::backed_up([BackedUpToken {
                spelling: command.spelling(),
                source_provenance: command.source_provenance(),
            }]),
            TokenBehavior::BackedUp(treatment),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        if self.is_observed() {
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
                tokens: vec![self.observed_command_spelling(&command)],
            }));
            if self.command.alignment.active_alignment.is_some()
                && matches!(
                    adjustment,
                    crate::processor::AlignmentDeliveryAdjustment::BeginGroup
                        | crate::processor::AlignmentDeliveryAdjustment::EndGroup
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
                    previous_align_state: Some(previous_align_state),
                }));
            }
        }
        Ok(())
    }

    /// Completes TeX82 §760's v-template insertion when a scalar
    /// `get_x_token` lookahead reaches an active-cell delimiter. `get_next`
    /// has already made the delimiter decision, so this accepts only that
    /// typed adjustment and never reclassifies its spelling.
    pub(crate) fn begin_scalar_alignment_v_template(
        &mut self,
        command: &CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let alignment = self
            .command
            .alignment
            .active_alignment
            .ok_or(CommandError::input_invariant())?;
        let delimiter = Self::saved_alignment_delimiter(command)?;
        let delimiter_line = command
            .direct_source_line_number()
            .unwrap_or_else(|| self.command.current_file_line_number());
        if self.last_delivery != Some(command.delivery_stamp()) {
            return Err(CommandError::StaleDelivery);
        }
        self.last_delivery = None;
        self.start_alignment_v_template(alignment, delimiter, delimiter_line)
    }

    /// Runs the one TeX82 §341 next-command pipeline in the caller's final
    /// slot: authoritative raw input, in-place meaning resolution, then one
    /// delivery-policy settlement. Cold input transitions re-enter this loop
    /// only after their slot typestate borrow has ended.
    pub(super) fn next_command_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        // Expanded delivery keeps this same caller-owned value across every
        // synchronous expansion. Raw input overwrites all delivery facts and
        // meaning resolution overwrites the prior meaning, so rebuilding an
        // empty command between tokens would only duplicate state movement.
        if destination.is_none() {
            *destination = Some(CurrentCommand::empty());
        }
        let result = (|| loop {
            if let Some(episode) = self.take_ready_replay_completion() {
                destination.take();
                return Ok(super::DeliveryStatus::ReplayCompleted(episode));
            }
            let transition = self
                .command
                .next_raw_into(
                    self.state,
                    self.create_source_control_sequences,
                    destination
                        .as_mut()
                        .expect("next-command pipeline owns its reusable command slot")
                        .empty_for_raw_delivery(),
                )
                .map_err(|()| CommandError::input_invariant());
            let transition = match transition {
                Ok(transition) => transition,
                Err(error) => {
                    destination.take();
                    return Err(error);
                }
            };
            let raw = match transition {
                InputTopTransition::Empty => {
                    observe!(
                        self,
                        CommandObservation::Input(InputRecord {
                            transition: InputTransition::Stop,
                            reason: InputReason::Source,
                            source_name: Some(SourceNameClass::Terminal),
                            source: None,
                            level: 0,
                            position: 0,
                        }),
                    );
                    if self.raw_end_restarts()? {
                        continue;
                    }
                    destination.take();
                    return Ok(super::DeliveryStatus::End);
                }
                InputTopTransition::Delivered(raw) => raw,
                InputTopTransition::ParameterPushed(parameter_level) => {
                    observe!(
                        self,
                        CommandObservation::Input(InputRecord {
                            transition: InputTransition::Push,
                            reason: InputReason::Parameter,
                            source_name: None,
                            source: None,
                            level: parameter_level.0,
                            position: 0,
                        }),
                    );
                    continue;
                }
                InputTopTransition::InvalidCharacter => {
                    self.report_recoverable(
                        INVALID_SOURCE_CHARACTER_DIAGNOSTIC,
                        "Text line contains an invalid character".into(),
                        &[
                            "A funny symbol that I can't read has just been input.",
                            "Continue, and I'll forget that it ever happened.",
                        ],
                    );
                    continue;
                }
                InputTopTransition::NeedLine(identity) => {
                    if self.acquire_source_line(true)?.is_none()
                        && matches!(
                            self.finish_exhausted_source(identity)?,
                            SourceExhaustionStatus::End
                        )
                    {
                        if self.raw_end_restarts()? {
                            continue;
                        }
                        destination.take();
                        return Ok(super::DeliveryStatus::End);
                    }
                    continue;
                }
                InputTopTransition::SourceExhausted(identity) => {
                    if matches!(
                        self.finish_exhausted_source(identity)?,
                        SourceExhaustionStatus::End
                    ) {
                        if self.raw_end_restarts()? {
                            continue;
                        }
                        destination.take();
                        return Ok(super::DeliveryStatus::End);
                    }
                    continue;
                }
                InputTopTransition::TokenExhausted(identity) => {
                    let index = self
                        .command
                        .input
                        .levels
                        .last()
                        .and_then(|level| match level {
                            InputLevel::Tokens(cursor) if cursor.identity() == identity => {
                                Some(cursor.frame.position())
                            }
                            InputLevel::MacroArgument(cursor) if cursor.identity() == identity => {
                                Some(cursor.frame.position())
                            }
                            _ => None,
                        })
                        .ok_or_else(CommandError::input_invariant)?;
                    match self.retire_input_top(identity)? {
                        RetirementHandoff::Stop => {
                            if self.raw_end_restarts()? {
                                continue;
                            }
                            destination.take();
                            return Ok(super::DeliveryStatus::End);
                        }
                        RetirementHandoff::Completed => continue,
                        RetirementHandoff::Continue => continue,
                        RetirementHandoff::EndV(level) => destination
                            .as_mut()
                            .expect("next-command pipeline owns its reusable command slot")
                            .empty_for_raw_delivery()
                            .write_raw_delivery(
                                TracedTokenWord::pack(
                                    self.state.frozen_end_template_token(),
                                    OriginId::UNKNOWN,
                                ),
                                level.0,
                                u64::from(index),
                                None,
                                false,
                                None,
                                false,
                            ),
                    }
                }
            };

            let (input_level, position) = raw.delivery_coordinate();
            let delivery_stamp =
                DeliveryStamp::new(input_level, position, self.next_delivery_sequence);
            self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
            let scanner = !matches!(
                self.command.scanner.status(),
                crate::processor::ScannerStatus::Normal
            );
            let (resolved, meaning_lookup) =
                raw.resolve_in_place(delivery_stamp.sequence(), self.state);
            self.record_raw_delivery(scanner, meaning_lookup);
            if let Err(error) = self.apply_delivery_rules(resolved, delivery_stamp) {
                destination.take();
                return Err(error);
            }
            return Ok(super::DeliveryStatus::Command);
        })();
        if result.is_err() {
            destination.take();
        }
        result
    }

    /// Settles an input-end transition after every raw typestate borrow has
    /// ended. `true` means outer-validity recovery installed new input and the
    /// caller must re-enter the pipeline.
    fn raw_end_restarts(&mut self) -> Result<bool, CommandError> {
        // §360: a `\read` pseudo-file's line has ended, which is
        // `cur_cmd:=0; cur_chr:=0; return` -- an ordinary end of line inside
        // live `read_toks`, so no runaway recovery may run.
        if std::mem::take(&mut self.read_line_ended) {
            return Ok(false);
        }
        self.recover_runaway_eof()
    }

    /// Applies the remaining §341 delivery rules to one resolved command.
    /// Resolution has ended its dense meaning borrow before this function can
    /// perform recovery, alignment mutation, or observation.
    fn apply_delivery_rules(
        &mut self,
        mut resolved: ResolvedCommand<'_, G>,
        delivery_stamp: DeliveryStamp,
    ) -> Result<(), CommandError> {
        if resolved.as_ref().suppresses_expandable_control_sequence() {
            resolved.as_mut().suppress_expandable();
        }
        // Outer-validity recovery canonically backs up this exact raw
        // delivery before substituting its recovery space.
        self.last_delivery = Some(delivery_stamp);
        self.check_outer_validity_entry(resolved.as_mut())?;
        let previous_align_state = self.command.alignment.align_state;
        self.command.record_alignment_phase();
        self.command.alignment.classify_delivery(resolved.as_mut());
        let command = resolved.as_ref();
        let adjustment = command.alignment_adjustment();
        if self.command.alignment.active_alignment.is_some()
            && !matches!(
                adjustment,
                crate::processor::AlignmentDeliveryAdjustment::None
            )
        {
            self.observe(CommandObservation::Alignment(AlignmentRecord {
                transition: match adjustment {
                    crate::processor::AlignmentDeliveryAdjustment::BeginGroup => "begin_group",
                    crate::processor::AlignmentDeliveryAdjustment::EndGroup => "end_group",
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_) => "delimiter",
                    crate::processor::AlignmentDeliveryAdjustment::None => unreachable!(),
                },
                alignment: self
                    .command
                    .alignment
                    .active_alignment
                    .map(|identity| identity.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: if matches!(
                    adjustment,
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                ) {
                    previous_align_state
                } else {
                    self.command.alignment.align_state
                },
                delimiter: match adjustment {
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(delimiter) => {
                        Some(delimiter.observation_name())
                    }
                    _ => None,
                },
                previous_align_state: matches!(
                    adjustment,
                    crate::processor::AlignmentDeliveryAdjustment::BeginGroup
                        | crate::processor::AlignmentDeliveryAdjustment::EndGroup
                )
                .then_some(previous_align_state),
            }));
        }
        if !matches!(
            adjustment,
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
        ) {
            self.observe_raw_delivery(command);
        }
        Ok(())
    }

    pub(crate) fn acquire_source_line(
        &mut self,
        firm: bool,
    ) -> Result<Option<crate::PhysicalLine>, CommandError> {
        self.acquire_source_line_with_pending(firm, false)
    }

    fn finish_exhausted_source(
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
            RetirementHandoff::Completed => Ok(SourceExhaustionStatus::Continue),
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

    fn take_ready_replay_completion(&mut self) -> Option<crate::CommandReplayEpisode> {
        self.command.take_ready_replay_completion()
    }

    /// Retires the validated top input row and returns only the scalar phase
    /// the caller's existing delivery loop must advance to. The caller-owned
    /// command destination remains in place; retirement does not reconstruct
    /// or redispatch a command.
    fn retire_input_top(
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
        if !matches!(action, InputRetirementAction::VTemplateRetained) {
            let reason = if self.take_immediate_write_retirement(identity) {
                InputReason::Write
            } else {
                observed_retirement_reason(action, retirement.reason)
            };
            self.observe(CommandObservation::Input(InputRecord {
                transition: if matches!(action, InputRetirementAction::TerminalStop) {
                    InputTransition::Stop
                } else {
                    InputTransition::Retire
                },
                reason,
                source_name: retirement.name_class,
                source: retirement.source,
                level: identity.0,
                position: 0,
            }));
        }
        // The pinned observer names a retiring alignment template from the
        // token list the level holds, exactly as tex.web's `end_token_list`
        // distinguishes `start=omit_template` from a column's ⟨v_j⟩ part.
        // A retained v-template has not left the stack yet, so it is not
        // named here.
        if let Some(transition) = match retirement.reason {
            _ if matches!(action, InputRetirementAction::VTemplateRetained) => None,
            InputRetirementReason::AlignmentUTemplate => Some("u_template_retire"),
            InputRetirementReason::AlignmentVTemplate => Some("v_template_retire"),
            InputRetirementReason::AlignmentOmitTemplate => Some("omit_template_retire"),
            _ => None,
        } {
            self.observe(CommandObservation::Alignment(AlignmentRecord {
                transition,
                alignment: self
                    .command
                    .alignment
                    .active_alignment
                    .map(|alignment| alignment.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            }));
        }
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
                let previous_align_state = self.command.alignment.align_state;
                self.command.record_alignment_phase();
                if self.command.alignment.finish_u_template(identity) {
                    observe!(
                        self,
                        CommandObservation::Alignment(AlignmentRecord {
                            transition: "state_change",
                            alignment: self
                                .command
                                .alignment
                                .active_alignment
                                .map(|alignment| alignment.raw()),
                            nesting: self.command.alignment_observation_nesting(),
                            align_state: self.command.alignment.align_state,
                            delimiter: None,
                            previous_align_state: Some(previous_align_state),
                        }),
                    );
                }
                if self.command.complete_replay(identity).is_some() {
                    Ok(RetirementHandoff::Completed)
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
            let retirement = self.retire_input_top(identity)?;
            match retirement {
                // Finished stored replay episodes queue their completion in
                // command state. Draining continues so the whole depleted run
                // is cleaned off; delivery surfaces each ready ownership
                // boundary before any enclosing source.
                RetirementHandoff::Continue | RetirementHandoff::Completed => {}
                RetirementHandoff::Stop | RetirementHandoff::EndV(_) => {
                    return Err(CommandError::input_invariant());
                }
            }
        }
    }

    fn check_outer_validity_entry(
        &mut self,
        command: &mut CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        if !command.is_outer() || matches!(self.command.scanner.status(), ScannerStatus::Normal) {
            return Ok(());
        }

        let recovery = self.command.scanner.recovery_context();
        // The pinned TeX82 observer instruments `check_outer_validity` at
        // entry, before §23 backs up the forbidden outer control sequence.
        // Keep this command-owned: the backup still belongs to raw delivery,
        // but the diagnostic describes the live scanner episode that selected
        // its recovery.
        self.observe_outer_validity_diagnostic(&recovery.status, false);
        if matches!(recovery.status, ScannerStatus::Matching(_)) {
            self.outer_recovered_while_matching = true;
        }
        if matches!(recovery.status, ScannerStatus::Absorbing(_)) {
            self.outer_recovered_while_absorbing = true;
        }
        // TeX82 §336 deliberately does not back up an outer control
        // sequence delivered by a `\read` pseudo-file (`name=1..17`).  The
        // one-line source remains live until §483's `end_file_reading`, so
        // the recovery context must continue to name that read frame rather
        // than manufacture a `<to be read again>` token-list level.
        let delivered_by_read = self.command.input.levels.iter().any(|level| {
            matches!(
                level,
                InputLevel::Source(source)
                    if source.identity().0 == command.delivery_stamp().input_level()
                        && matches!(source.name_class, SourceNameClass::ReadStream(_))
            )
        });
        if !delivered_by_read {
            self.back_input(command.copy_for_backup())?;
        }
        self.install_outer_recovery(recovery, false)?;
        command.recover_as_space();
        Ok(())
    }

    /// Recovers terminal input only while a scanner episode is live. The
    /// inserted tokens are then delivered through this same raw loop.
    fn recover_runaway_eof(&mut self) -> Result<bool, CommandError> {
        let recovery = self.command.scanner.recovery_context();
        if matches!(self.command.scanner.eof_legality(), EofLegality::Legal) {
            return Ok(false);
        }
        if matches!(recovery.status, ScannerStatus::Matching(_)) {
            // TeX82 §23 calls `check_outer_validity` after retiring an input
            // file at EOF. Its frozen `\par` ends the failed §394 match; it
            // is not an ordinary paragraph that `back_error` replays after
            // the expansion returns.
            self.eof_recovered_while_matching = true;
        }
        self.observe_outer_validity_diagnostic(&recovery.status, true);
        self.install_outer_recovery(recovery, true)?;
        Ok(true)
    }

    /// TeX.web's `check_outer_validity` recovery table. Primitive insertions
    /// are frozen tokens, retaining their original meanings if user code has
    /// reassigned their visible spellings.
    fn install_outer_recovery(
        &mut self,
        recovery: RecoveryContext,
        at_file_end: bool,
    ) -> Result<(), CommandError> {
        let RecoveryContext { status, warning } = recovery;
        if matches!(status, ScannerStatus::Aligning(_)) {
            // TeX82 §23's `check_outer_validity` reports the aligning
            // recovery before `ins_error` inserts inaccessible frozen `\cr`.
            // This remains a command-owned observation: the executor has no
            // raw scanner-status or token-list recovery capability.
            self.observe(CommandObservation::Alignment(AlignmentRecord {
                transition: "outer_validity",
                alignment: self
                    .command
                    .alignment
                    .active_alignment
                    .map(|identity| identity.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            }));
        }
        let (frozen_recovery_name, recovery_kind) = match &status {
            ScannerStatus::Skipping(_) => (Some("fi"), RecoveryKind::InsertedToken),
            ScannerStatus::Matching(_) => (Some("par"), RecoveryKind::InsertedControlSequence),
            // TeX82's `check_outer_validity` inserts frozen `\\cr` before
            // its required follow-up right brace. The recovery event denotes
            // the inaccessible control sequence alone; raw delivery owns
            // the whole displayed insertion.
            ScannerStatus::Aligning(_) => (Some("cr"), RecoveryKind::InsertedControlSequence),
            ScannerStatus::Normal | ScannerStatus::Defining(_) | ScannerStatus::Absorbing(_) => {
                (None, RecoveryKind::InsertedToken)
            }
        };
        let (first_token, second_token) = match status {
            ScannerStatus::Normal => return Ok(()),
            ScannerStatus::Skipping(_) => (self.frozen_primitive("fi")?, None),
            ScannerStatus::Defining(_) | ScannerStatus::Absorbing(_) => (right_brace(), None),
            ScannerStatus::Matching(_) => (self.frozen_primitive("par")?, None),
            ScannerStatus::Aligning(_) => (self.frozen_primitive("cr")?, Some(right_brace())),
        };
        // TeX82 §23 leaves `scanner_status := aligning` live while its
        // inserted frozen `\cr` finishes `init_align`'s preamble scan.
        // `get_preamble_token` therefore owns the one aligning-to-normal
        // transition at typed preamble completion. Every other recovery
        // remains an immediate scanner-episode exit before its inserted input
        // is delivered.
        let retains_aligning_until_preamble_completion =
            matches!(status, ScannerStatus::Aligning(_))
                && self.command.alignment.active_alignment.is_some();
        if !retains_aligning_until_preamble_completion {
            self.command.scanner.clear_for_recovery();
        }
        if let Some(warning) = warning {
            self.command.timeline.record_expansion_diagnostic_push();
            self.command.expansion.pending_diagnostics.push(warning.0);
        }
        let observed_tokens = std::iter::once(first_token)
            .chain(second_token)
            .enumerate()
            .filter_map(|(index, token)| {
                if index != 0 && matches!(&status, ScannerStatus::Aligning(_)) {
                    return None;
                }
                (index == 0)
                    .then_some(frozen_recovery_name)
                    .flatten()
                    .map(|name| crate::observation::ObservedToken::ControlSequence(name.into()))
                    .or_else(|| Some(self.observed_token(token)))
            })
            .collect();
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient(std::iter::once(first_token).chain(second_token)),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        if matches!(&status, ScannerStatus::Aligning(_)) {
            self.command.alignment.pending_outer_recovery_cr = Some(self.frozen_primitive("cr")?);
        }
        // §336 ends with `ins_error`, which is `back_input` as an *inserted*
        // level and only then `error`. The context therefore renders with the
        // frozen recovery token already on the stack, which is what puts the
        // `<inserted text>` line above the source line.
        // §338's `<Tell the user what has run away and try to recover>`, the
        // sibling branch of §336's incomplete-conditional report. `runaway`'s
        // own heading and partial list are still owed (umber2-2iil); this is
        // the `error` §338 ends with.
        if let Some((kind, warning_index)) = match &status {
            ScannerStatus::Defining(context) => Some(("definition", context.target)),
            ScannerStatus::Matching(context) => Some(("use", Some(context.macro_name))),
            ScannerStatus::Aligning(context) => Some(("preamble", context.owner)),
            ScannerStatus::Absorbing(context) => Some(("text", context.owner)),
            ScannerStatus::Normal | ScannerStatus::Skipping(_) => None,
        } {
            // §338 chooses its opening on `cur_cs`: a forbidden `\outer`
            // control sequence rather than the file simply running out.
            let opening = if at_file_end {
                "File ended"
            } else {
                "Forbidden control sequence found"
            };
            let name = warning_index.map_or_else(String::new, |symbol| {
                let spelling = self.state.resolve(symbol).to_owned();
                super::expand::print_esc_text(self.state, &spelling)
            });
            let context = self.command.output_open_context(self.state);
            let heading = match &status {
                ScannerStatus::Defining(_) => "Runaway definition?",
                ScannerStatus::Matching(_) => "Runaway argument?",
                ScannerStatus::Aligning(_) => "Runaway preamble?",
                ScannerStatus::Absorbing(_) => "Runaway text?",
                ScannerStatus::Normal | ScannerStatus::Skipping(_) => unreachable!(),
            };
            let partial = match &status {
                ScannerStatus::Aligning(context) => self
                    .command
                    .transient
                    .builders
                    .iter()
                    .find(|builder| builder.identity == context.builder.0)
                    .and_then(|builder| {
                        self.command
                            .attempt
                            .arena()
                            .token_buffer(builder.tokens)
                            .ok()
                    })
                    .map_or_else(String::new, |tokens| {
                        tokens.iter().fold(String::new(), |mut text, token| {
                            super::expand::append_token_list_token_text(
                                self.state,
                                token.semantic_token(),
                                &mut text,
                            );
                            text
                        })
                    }),
                _ => String::new(),
            };
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: RUNAWAY_SCAN_DIAGNOSTIC,
                    runaway: Some(crate::state::RunawayPrelude { heading, partial }),
                    message: format!("{opening} while scanning {kind} of {name}"),
                    help: &[
                        "I suspect you have forgotten a `}', causing me",
                        "to read past where you wanted me to stop.",
                        "I'll try to recover; but if the error is serious,",
                        "you'd better type `E' or `X' now and fix your file.",
                    ],
                    context,
                    integer_error: None,
                });
        }
        if let ScannerStatus::Skipping(skipping) = &status {
            let name =
                super::expand::print_esc_text(self.state, skipping.conditional.canonical_name());
            let message = format!(
                "Incomplete {name}; all text was ignored after line {}",
                skipping.skip_line
            );
            let help: &'static [&'static str] = if at_file_end {
                // §336 replaces only the first help line when the failure was
                // the file running out rather than a forbidden control
                // sequence appearing in the skipped text.
                &[
                    "The file ended while I was skipping conditional text.",
                    "This kind of error happens when you say `\\if...' and forget",
                    "the matching `\\fi'. I've inserted a `\\fi'; this might work.",
                ]
            } else {
                &[
                    "A forbidden control sequence occurred in skipped text.",
                    "This kind of error happens when you say `\\if...' and forget",
                    "the matching `\\fi'. I've inserted a `\\fi'; this might work.",
                ]
            };
            let context = self.command.output_open_context(self.state);
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: INCOMPLETE_CONDITIONAL_DIAGNOSTIC,
                    runaway: None,
                    message,
                    help,
                    context,
                    integer_error: None,
                });
        }
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }),
        );
        observe!(
            self,
            CommandObservation::Recovery(RecoveryRecord {
                kind: recovery_kind,
                tokens: observed_tokens,
            }),
        );
        Ok(())
    }

    pub(crate) fn set_runaway_partial(&mut self, identity: u64, tokens: &[TracedTokenWord]) {
        // TeX82 §262 carries `match_chr` from each match token into later
        // out-parameter rendering. The compact representation stores a
        // nonstandard match character immediately before its slot token.
        let mut raw = String::new();
        let mut match_marker = '#';
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index].semantic_token();
            if let Token::Char {
                ch,
                cat: Catcode::Parameter,
            } = token
                && let Some(Token::Param(slot)) =
                    tokens.get(index + 1).map(|word| word.semantic_token())
            {
                match_marker = ch;
                raw.push(ch);
                raw.push(char::from(b'0' + slot));
                index += 2;
                continue;
            }
            if let Token::Param(slot) = token {
                raw.push(match_marker);
                raw.push(char::from(b'0' + slot));
            } else {
                super::expand::append_token_list_token_text(self.state, token, &mut raw);
            }
            index += 1;
        }
        let mut rendered = String::new();
        self.state.append_selector_string_text(&raw, &mut rendered);
        // TeX82 §306 reads the current scanner's live list synchronously. A
        // command trace discovered by the same raw-delivery episode can
        // already be queued behind the deferred report, so update the newest
        // report owned by this scanner episode rather than assuming it is the
        // queue tail. In particular, a later scan must not overwrite a
        // completed §396 runaway-argument pseudoprint still awaiting output.
        let Some(runaway) =
            self.command
                .semantic_diagnostics
                .iter_mut()
                .rev()
                .find_map(|diagnostic| match diagnostic {
                    crate::CommandSemanticDiagnostic::Recoverable {
                        identity: diagnostic_identity,
                        runaway: Some(runaway),
                        ..
                    } if *diagnostic_identity == identity => Some(runaway),
                    _ => None,
                })
        else {
            return;
        };
        runaway.partial = rendered;
    }

    fn observe_outer_validity_diagnostic(&mut self, status: &ScannerStatus, at_eof: bool) {
        let arguments = if self.command.profile().capabilities().supports_etex() {
            // The e-TeX 2.6 oracle's §336 seam records the outer-validity
            // boundary without scanner metadata. Scanner status remains a
            // separate canonical transition.
            Vec::new()
        } else {
            vec![DiagnosticArgument::Name(
                match status {
                    ScannerStatus::Normal => "normal",
                    ScannerStatus::Skipping(_) => "skipping",
                    ScannerStatus::Defining(_) => "defining",
                    ScannerStatus::Matching(_) => "matching",
                    ScannerStatus::Aligning(_) => "aligning",
                    ScannerStatus::Absorbing(_) => "absorbing",
                }
                .into(),
            )]
        };
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic: if at_eof {
                "outer_validity_eof"
            } else {
                "outer_validity_control_sequence"
            },
            arguments,
        }));
        if at_eof && matches!(status, ScannerStatus::Skipping(_)) {
            self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
                severity: "error",
                diagnostic: "conditional_incomplete",
                arguments: Vec::new(),
            }));
        }
    }

    fn frozen_primitive(&self, name: &str) -> Result<TracedTokenWord, CommandError> {
        Ok(TracedTokenWord::pack(
            self.frozen_primitive_token(name)?,
            OriginId::UNKNOWN,
        ))
    }

    pub(crate) fn observed_token(
        &self,
        token: TracedTokenWord,
    ) -> crate::observation::ObservedToken {
        observed_token(
            token,
            |symbol| self.state.resolve(symbol).to_owned(),
            |frozen| self.state.frozen_primitive_name(frozen).map(str::to_owned),
        )
    }

    pub(crate) fn observed_command_spelling(
        &self,
        command: &CurrentCommand<G>,
    ) -> crate::observation::ObservedToken {
        if let Some(symbol) = command.control_sequence() {
            // §353's `get_next` resolves an active character through its own
            // `active_base + c` control-sequence cell and records that cell
            // in `cur_cs`, so §365's `cur_tok` is `cs_token_flag + cur_cs`.
            // Observations expose that identity at the current-command
            // boundary, just as they do for escaped control sequences.  The
            // raw token spelling remains available on `CurrentCommand<G>` for
            // token-sensitive consumers.
            crate::observation::ObservedToken::ControlSequence(
                self.state.resolve(symbol).to_owned(),
            )
        } else if command.spelling().semantic_token().is_frozen_end_template()
            || command.spelling().semantic_token().is_frozen_endv()
        {
            // TeX82 stores both inaccessible template sentinels in distinct
            // frozen control-sequence slots whose texts are `endtemplate`
            // (TeX.web §780). `get_next` therefore exposes that control
            // sequence identity at the raw boundary, while §380's
            // `get_x_token` changes only its effective command to `endv` --
            // and §380's `x_token` does not even do that, reaching §375's
            // separate `frozen_endv` token through §366 `expand` instead.
            crate::observation::ObservedToken::ControlSequence("endtemplate".into())
        } else if matches!(command.spelling().semantic_token(), Token::Frozen(_))
            && matches!(
                command.meaning(),
                tex_state::ResolvedMeaning::Static(Meaning::Relax)
            )
        {
            // TeX82's observer presents the inaccessible frozen `\relax`
            // inserted by incomplete-conditional recovery as `\relax`.
            // A `\noexpand` target has the same effective meaning but retains
            // its original control-sequence spelling.
            crate::observation::ObservedToken::ControlSequence("relax".into())
        } else if matches!(command.spelling().semantic_token(), Token::Frozen(_))
            && let tex_state::ResolvedMeaning::Static(meaning) = command.meaning()
            && let Some(name) = self.state.primitive_name(meaning)
        {
            crate::observation::ObservedToken::ControlSequence(name.into())
        } else {
            self.observed_token(command.spelling())
        }
    }

    fn observe_raw_delivery(&mut self, command: &CurrentCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let (command_name, command_operand) =
                crate::observation::canonical_current_command_identity_for_profile(
                    self.command.profile(),
                    command,
                );
            let spelling = self.observed_command_spelling(command);
            let semantic_operand = crate::observation::canonical_sparse_register_operand(
                self.command.profile(),
                command.meaning(),
            );
            CommandObservation::Command(CommandDeliveryRecord {
                boundary: CommandDeliveryBoundary::Raw,
                spelling,
                command: command_name,
                command_operand,
                semantic_operand,
                provenance: CommandProvenance::from_stamp(
                    command.delivery_stamp(),
                    command.origin(),
                    command.direct_source_provenance(),
                ),
            })
        });
    }

    pub(crate) fn undo_alignment_delivery(&mut self, command: &CurrentCommand<G>) {
        self.command.record_alignment_phase();
        self.command
            .alignment
            .undo_delivery(command.alignment_adjustment());
    }

    /// Cancels raw brace accounting for a matched `#{` delimiter. The opening
    /// brace was delivered as parameter text, so scalar macro matching must
    /// not leave a group entry for replacement replay to balance later.
    pub(crate) fn undo_delimiter_begin_group_delivery(&mut self) {
        self.command.record_alignment_phase();
        self.command.alignment.undo_delimiter_begin_group_delivery();
    }
}

fn observed_retirement_reason(
    action: InputRetirementAction,
    reason: InputRetirementReason,
) -> InputReason {
    match (action, reason) {
        (
            InputRetirementAction::SourcePopped
            | InputRetirementAction::TerminalStop
            | InputRetirementAction::ReadLineEnded,
            _,
        ) => InputReason::Source,
        (_, InputRetirementReason::Backup) => InputReason::Backup,
        (_, InputRetirementReason::Macro) => InputReason::Macro,
        (_, InputRetirementReason::Parameter) => InputReason::Parameter,
        (_, InputRetirementReason::AlignmentUTemplate) => InputReason::AlignmentUTemplate,
        (
            _,
            InputRetirementReason::AlignmentVTemplate
            | InputRetirementReason::AlignmentOmitTemplate,
        ) => InputReason::AlignmentVTemplate,
        (_, InputRetirementReason::Recovery) => InputReason::Recovery,
        (_, InputRetirementReason::TokenList(stored)) => stored_input_reason(stored),
        (_, InputRetirementReason::Source) => InputReason::Source,
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

enum SourceExhaustionStatus {
    Continue,
    End,
}

enum RetirementHandoff {
    Stop,
    Continue,
    EndV(InputLevelId),
    Completed,
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

/// The live engine reads TeX82 §341's `get_next` makes while it reads source.
///
/// One borrow of live state answers both: §207's category codes, and §363's
/// `firm_up_the_line`. Splitting them into two closures would need
/// [`tex_state::CommandContext`] borrowed mutably twice at once, which is
/// what forced them into one trait rather than any relationship between the
/// two reads.
/// Whether a delivered command is TeX82's `math_shift` command code -- the
/// single test both §1138's opener and §1197's closer apply to their peeked
/// token, and the reason neither may grow a private notion of "a `$`".
fn is_math_shift<G>(command: &CurrentCommand<G>) -> bool {
    matches!(
        command.meaning(),
        tex_state::ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::MathShift,
            ..
        })
    )
}

fn right_brace() -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
        OriginId::UNKNOWN,
    )
}
