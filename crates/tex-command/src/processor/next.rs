//! Canonical raw command delivery.
//!
//! This is the sole scalar path from input levels to `CurrentCommand`, after
//! TeX.web §341 (`get_next`).  Later scanner and alignment milestones extend
//! the two explicit entry points below; they do not add another lexical path.

use tex_state::env::banks::{IntParam, TokParam};
use tex_state::input::TracedTokenList;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::command::{CurrentCommand, DeliveryStamp};
use crate::error::CommandError;
use crate::input::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, InputRetirementAction,
    OutParameterReplay, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior,
    TokenCursor, TokenPayload,
};
// tex.web §303's `name` classification only reaches an observation payload.
use crate::input::SourceNameClass;
use crate::input::{RegisteredSourceKind, SourceRegistration};
use crate::profile::{CharacterCode, CharacterMode};
use crate::{
    AlignmentDelivery, AlignmentDeliveryEvent, CommandReplayDelivery, SourceControlSequenceKind,
    SourceProvenance, SourceToken, SourceTokenizationStep,
};
use tex_state::CommandLineSource;

use super::CommandProcessor;
use super::expand::{ExpandedFetch, ProtectedMacroHandling, UndefinedHandling};
use super::status::{EofLegality, RecoveryContext, ScannerStatus};
use super::{
    AlignmentInterceptionPolicy, ControlSequenceCreation, DeliveryEvent, DeliveryMode,
    DeliveryPolicy, ExpandedDeliveryPolicy, ExpandedObservationPolicy, FirstCommandPolicy,
    ReplayCompletionPolicy,
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

impl CommandProcessor<'_> {
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
        match self.retire_and_restart(InputLevelId(stamp.input_level()))? {
            RetirementRestart::Continue | RetirementRestart::Completed => Ok(()),
            RetirementRestart::Stop | RetirementRestart::EndV(_) => {
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
        match self.retire_and_restart(level)? {
            RetirementRestart::Stop if std::mem::take(&mut self.read_line_ended) => Ok(()),
            RetirementRestart::Continue
            | RetirementRestart::Completed
            | RetirementRestart::Stop
            | RetirementRestart::EndV(_) => Err(CommandError::input_invariant()),
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
            match self.retire_and_restart(top)? {
                RetirementRestart::Continue | RetirementRestart::Completed => {}
                RetirementRestart::Stop | RetirementRestart::EndV(_) => {
                    return Err(CommandError::input_invariant());
                }
            }
            if reached {
                return Ok(());
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
    ) -> Result<Option<AlignmentDelivery>, CommandError> {
        let fetch = if main_loop_active {
            ExpandedFetch::XToken
        } else {
            ExpandedFetch::GetXToken
        };
        let delivery = self.delivery_driver(
            None,
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
                control_sequence_creation: ControlSequenceCreation::Forbid,
                alignment_interception: AlignmentInterceptionPolicy::Surface,
            },
        )?;
        Ok(delivery.map(|event| match event {
            DeliveryEvent::Command(command) => AlignmentDelivery::Command(command),
            DeliveryEvent::ReplayCompleted(episode) => AlignmentDelivery::Completed(episode),
            DeliveryEvent::Alignment(event) => AlignmentDelivery::Event(event),
            DeliveryEvent::PendingExpanded(_) => {
                unreachable!("alignment delivery commits terminal observations")
            }
        }))
    }

    /// Hands an intercepted delimiter from `end_template` main control back
    /// to canonical input, then starts the active cell's v-template above it.
    /// The delimiter is never classified by an executor-side loop: after the
    /// suffix retires, raw `get_next` sees it again with its original spelling.
    pub fn begin_alignment_v_template(
        &mut self,
        alignment: crate::AlignmentIdentity,
        event: AlignmentDeliveryEvent,
    ) -> Result<(), CommandError> {
        let saved_delimiter = match event {
            AlignmentDeliveryEvent::EndTemplate(delimiter) => {
                if self.last_delivery != Some(delimiter.delivery_stamp())
                    || !matches!(
                        delimiter.meaning(),
                        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
                    )
                {
                    return Err(CommandError::StaleDelivery);
                }
                self.last_delivery = None;
                Self::saved_alignment_delimiter(&delimiter)?
            }
            AlignmentDeliveryEvent::ClosingBrace(_) => {
                return Err(CommandError::input_invariant());
            }
        };
        self.start_alignment_v_template(alignment, saved_delimiter)
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
            let InputLevel::Tokens(cursor) = level else {
                break;
            };
            // TeX82 §1131 walks downward only while `state=token_list` and
            // `loc=null`. A live token either in the v-template itself or in
            // an interposed token-list frame is the canonical interwoven-
            // preamble fatal path, not an internal Rust invariant failure.
            if Self::next_stored_token(cursor, &self.command.parameters, &self.state).is_some() {
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
        command: CurrentCommand,
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
            TokenPayload::transient([TracedTokenWord::pack(recovery, OriginId::UNKNOWN)]),
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
        event: AlignmentDeliveryEvent,
    ) -> Result<(), CommandError> {
        let AlignmentDeliveryEvent::ClosingBrace(command) = event else {
            return Err(CommandError::input_invariant());
        };
        if !matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }
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
            TokenPayload::transient([TracedTokenWord::pack(frozen_cr, OriginId::UNKNOWN)]),
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
        command: &CurrentCommand,
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
    ) -> Result<(), CommandError> {
        self.command
            .begin_alignment_v_template(
                &self.state,
                alignment,
                saved_delimiter,
                self.state
                    .token_list_ref(tex_state::ids::TokenListId::EMPTY),
            )
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

    /// TeX82 §342's `@<If an alignment entry has just ended, take appropriate
    /// action@>`, the last statement of §341's `get_next` before its `exit`.
    ///
    /// Because it lives inside `get_next`, §789's ⟨v_j⟩ insertion is
    /// transparent to *every* reader that pulls a raw command: `get_token`,
    /// `get_x_token`, `macro_call`'s §392 parameter matcher, and §473's
    /// `scan_toks` all restart on the ⟨v_j⟩ template and never see the tab
    /// mark, `\span`, or `\cr` that ended the entry as ordinary content. A
    /// reader that does see it captures the delimiter's spelling as material
    /// -- and, worse, leaves the cell's own `\endv` undelivered, so the
    /// alignment never advances. Umber classifies the delimiter in
    /// [`crate::processor::alignment::AlignmentDelivery`] during delivery;
    /// this is where §342's structural consequence is applied, so the
    /// classification is made exactly once and consumed exactly once.
    ///
    /// `Ok(None)` means the ⟨v_j⟩ template is now the live input and the
    /// caller must restart its fetch, which is §789's `goto restart`.
    ///
    /// The one reader that deliberately does not run this is main control's
    /// [`Self::get_x_alignment_delivery`]: TeX82 §789 stores the delimiter in
    /// `extra_info(cur_align)` for §791's `fin_col`, and that alignment
    /// identity is executor-owned in Umber, so main control surfaces a typed
    /// [`AlignmentDeliveryEvent::EndTemplate`] and the executor calls
    /// [`Self::begin_alignment_v_template`] to perform the same insertion.
    pub(super) fn insert_alignment_entry_v_template(
        &mut self,
        command: CurrentCommand,
    ) -> Result<Option<CurrentCommand>, CommandError> {
        if matches!(
            command.alignment_adjustment(),
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
        ) {
            self.begin_scalar_alignment_v_template(command)?;
            return Ok(None);
        }
        Ok(Some(command))
    }

    /// Delivers one unexpanded raw command through canonical `get_next`.
    pub fn get_next(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.apply_error_stop_recovery()?;
        let delivery = self.delivery_driver(
            None,
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Consume,
                control_sequence_creation: ControlSequenceCreation::Forbid,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
        )?;
        Ok(delivery.map(|event| match event {
            DeliveryEvent::Command(command) => command,
            _ => unreachable!("ordinary raw delivery returns only commands"),
        }))
    }

    /// Delivers one raw command or an executor-owned stored-episode
    /// completion. This is the raw counterpart of
    /// [`Self::get_x_token_with_replay_completion`].
    pub fn get_next_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery>, CommandError> {
        let delivery = self.delivery_driver(
            None,
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Surface,
                control_sequence_creation: ControlSequenceCreation::Forbid,
                alignment_interception: AlignmentInterceptionPolicy::None,
            },
        )?;
        Ok(delivery.map(|event| match event {
            DeliveryEvent::Command(command) => CommandReplayDelivery::Command(command),
            DeliveryEvent::ReplayCompleted(episode) => CommandReplayDelivery::Completed(episode),
            _ => unreachable!("raw replay-aware delivery has no expanded event"),
        }))
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
    ) -> Result<Option<CurrentCommand>, CommandError> {
        let command = self.get_token()?;
        if let Some(command) = &command
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
        Ok(command)
    }

    /// Delivers one raw token for consumers which canonically permit a new
    /// control-sequence spelling. The present interner records a spelling
    /// without assigning it a meaning, so the policy boundary is explicit
    /// even before diagnostic-only interning is separated further.
    pub fn get_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.apply_error_stop_recovery()?;
        let delivery = self.delivery_driver(
            None,
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Consume,
                control_sequence_creation: ControlSequenceCreation::Allow,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
        )?;
        Ok(delivery.map(|event| match event {
            DeliveryEvent::Command(command) => command,
            _ => unreachable!("ordinary token delivery returns only commands"),
        }))
    }

    /// Applies tex.web §§84/87's ErrorStop input mutation at the sole raw
    /// command/input ownership boundary.
    #[doc(hidden)]
    pub fn apply_error_stop_recovery(&mut self) -> Result<(), CommandError> {
        while let Some(request) = self.state.take_error_recovery_request() {
            match request {
                tex_state::print::ErrorRecoveryRequest::Delete(count) => {
                    for _ in 0..count {
                        if self.get_token()?.is_none() {
                            break;
                        }
                    }
                    let context = self.error_context();
                    self.state.printer().print_rendered(&context);
                    self.state.continue_error_stop_dialog(&context).jump_out()?;
                }
                tex_state::print::ErrorRecoveryRequest::Insert(line) => {
                    self.command
                        .open_error_insert_line(line.into_bytes())
                        .map_err(|_| CommandError::input_invariant())?;
                }
            }
        }
        Ok(())
    }

    /// Restores the immediately preceding raw delivery to TeX's input.
    ///
    /// This is TeX.web's `back_input`: token equality is insufficient because
    /// equal spellings can be delivered by distinct input transitions. The
    /// consumed command proves the exact live transition and ensures literal
    /// brace accounting is undone at most once.
    pub fn back_input(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
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
        let Some(next) = self.get_token()? else {
            return Ok(false);
        };
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
        let Some(next) = self.get_x_token()? else {
            return Ok(false);
        };
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
    pub(crate) fn back_list(&mut self, tokens: Vec<crate::input::RootedBackedUpToken>) {
        debug_assert!(
            !tokens.is_empty(),
            "TeX82 §407 guards back_list with `p<>backup_head`"
        );
        let level = self.command.push_token_level(
            TokenPayload::backed_up_rooted(tokens),
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
    /// the `CurrentCommand`: §342's alignment interception records transitions
    /// that set `align_state` outright rather than stepping it, so a delivery
    /// that is available must have its own adjustment reversed, not one
    /// recomputed from the token.
    pub fn back_input_token(&mut self, spelling: TracedTokenWord) -> Result<(), CommandError> {
        self.back_input_rooted_token(tex_state::token::RootedTracedTokenWord::unowned(spelling))
    }

    /// Rooted form of [`Self::back_input_token`] for synthesized or saved
    /// arena-backed positions.
    pub fn back_input_rooted_token(
        &mut self,
        spelling: tex_state::token::RootedTracedTokenWord,
    ) -> Result<(), CommandError> {
        self.conserve_input_stack()?;
        let word = spelling.word();
        self.command
            .alignment
            .undo_delivery(AlignmentDeliveryState::back_input_adjustment(
                word.semantic_token(),
            ));
        let level = self.command.push_token_level(
            TokenPayload::backed_up_rooted([crate::input::RootedBackedUpToken::new(
                BackedUpToken {
                    spelling: word,
                    source_provenance: None,
                },
                spelling.into_parts().1,
            )]),
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
                tokens: vec![self.observed_token(word)],
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
        tokens: impl IntoIterator<Item = tex_state::token::RootedTracedTokenWord>,
    ) -> Result<(), CommandError> {
        let mut tokens = tokens.into_iter().collect::<Vec<_>>();
        let Some(last) = tokens.pop() else {
            return Ok(());
        };
        self.back_input_rooted_token(last)?;
        if self.profile().capabilities().supports_etex() {
            let prepended = tokens.len();
            for spelling in tokens.iter().rev() {
                self.command.alignment.undo_delivery(
                    AlignmentDeliveryState::back_input_adjustment(spelling.word().semantic_token()),
                );
            }
            let Some(InputLevel::Tokens(cursor)) = self.command.input.levels.last_mut() else {
                unreachable!("back_input above installed a token-list level");
            };
            assert_eq!(
                cursor.position(),
                0,
                "no delivery occurs while e-TeX links aftergroup tokens"
            );
            if cursor
                .payload
                .prepend_backed_up(tokens.into_iter().map(|spelling| {
                    let (word, root) = spelling.into_parts();
                    crate::input::RootedBackedUpToken::new(
                        BackedUpToken {
                            spelling: word,
                            source_provenance: None,
                        },
                        root,
                    )
                }))
                .is_none()
            {
                unreachable!("back_input above installed a backed-up payload");
            }
            let Ok(prepended) = u32::try_from(prepended) else {
                return Err(CommandError::input_invariant());
            };
            if cursor.frame.extend_limit(prepended).is_none() {
                return Err(CommandError::input_invariant());
            }
        } else {
            for spelling in tokens.into_iter().rev() {
                self.back_input_rooted_token(spelling)?;
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
        command: CurrentCommand,
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
    pub fn insert_partoken_before(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
        self.insert_par_before(command)
    }

    fn insert_par_before(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
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
            TokenPayload::transient([par]),
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
            TokenPayload::transient([TracedTokenWord::pack(par, OriginId::UNKNOWN)]),
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
        command: CurrentCommand,
    ) -> Result<(), CommandError> {
        self.back_input(command)?;
        let dollar_token = Token::Char {
            ch: '$',
            cat: Catcode::MathShift,
        };
        let dollar = TracedTokenWord::pack(dollar_token, OriginId::UNKNOWN);
        let level = self.command.push_token_level(
            TokenPayload::transient([dollar]),
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
            TokenPayload::transient([TracedTokenWord::pack(token, OriginId::UNKNOWN)]),
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
        let output_id = self.state.tok_param(TokParam::OUTPUT);
        let output = TracedTokenList::synthetic(self.state.token_list_ref(output_id));
        self.report_named_token_list("output", output.token_list());
        let words = self.state.tokens(output_id);
        let origins = self.state.origin_list(output.origin_ref());
        let level = self.command.push_token_level(
            TokenPayload::stored(&words, origins.iter()),
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
            InputLevel::Tokens(cursor) => {
                Self::next_stored_token(cursor, &self.command.parameters, &self.state).is_some()
            }
            InputLevel::Source(_) => unreachable!("output replay is a token level"),
        };
        let levels_above_are_depleted_backups = self.command.input.levels[output_index + 1..]
            .iter()
            .all(|level| {
                matches!(
                    level,
                    InputLevel::Tokens(cursor)
                        if matches!(cursor.behavior, TokenBehavior::BackedUp(_))
                            && Self::next_stored_token(
                                cursor,
                                &self.command.parameters,
                                &self.state,
                            )
                            .is_none()
                )
            });
        let unbalanced = output_has_remaining || !levels_above_are_depleted_backups;

        if unbalanced {
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
                            } =>
                    {
                        Some(
                            Self::next_stored_token(cursor, &self.command.parameters, &self.state)
                                .is_some(),
                        )
                    }
                    InputLevel::Source(_) | InputLevel::Tokens(_) => None,
                })
                .unwrap_or(false)
            {
                self.get_token()?.ok_or(CommandError::input_invariant())?;
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
            || Self::next_stored_token(cursor, &self.command.parameters, &self.state).is_some()
            || !matches!(
                cursor
                    .payload
                    .backed_up_get(0)
                    .map(|token| token.spelling.semantic_token()),
                Some(Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                })
            )
        {
            return Ok(());
        }
        match self.retire_and_restart(cursor.identity())? {
            RetirementRestart::Continue => Ok(()),
            RetirementRestart::Stop | RetirementRestart::EndV(_) | RetirementRestart::Completed => {
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
        command: CurrentCommand,
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
            TokenPayload::transient(
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
    pub fn report_off_save_bottom_drop(&mut self, command: &CurrentCommand) {
        self.observe_command_diagnostic("off_save_bottom_drop", command);
    }

    /// Performs TeX82 §1131's end-v instance of [`Self::recover_off_save`].
    pub fn recover_endv_off_save(
        &mut self,
        command: CurrentCommand,
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
        command: CurrentCommand,
        diagnostic: u64,
    ) -> Result<(), CommandError> {
        self.back_input(command)?;
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
        command: CurrentCommand,
        diagnostic: u64,
        message: String,
        help: &'static [&'static str],
    ) -> Result<(), CommandError> {
        self.back_error(command, diagnostic)?;
        let context = self.command.output_open_context(&self.state);
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: diagnostic,
                runaway: None,
                message,
                help,
                context,
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
        self.command.expansion.pending_diagnostics.push(diagnostic);
        let context = self.command.output_open_context(&self.state);
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: diagnostic,
                runaway: None,
                message,
                help,
                context,
            });
    }

    /// Canonical backing operation used by `\\noexpand` for one replayed
    /// command. The treatment belongs to the backed-up level, not the token
    /// or the returned command.
    pub(crate) fn back_input_with_treatment(
        &mut self,
        command: CurrentCommand,
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
    pub(crate) fn back_input_saved(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
        self.back_input_unchecked(command, BackupTreatment::Ordinary)
    }

    fn back_input_unchecked(
        &mut self,
        command: CurrentCommand,
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
            TokenPayload::backed_up_rooted([crate::input::RootedBackedUpToken::new(
                BackedUpToken {
                    spelling: command.spelling(),
                    source_provenance: command.source_provenance(),
                },
                command.origin_ref().clone(),
            )]),
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
        command: CurrentCommand,
    ) -> Result<(), CommandError> {
        let alignment = self
            .command
            .alignment
            .active_alignment
            .ok_or(CommandError::input_invariant())?;
        let delimiter = Self::saved_alignment_delimiter(&command)?;
        if self.last_delivery != Some(command.delivery_stamp()) {
            return Err(CommandError::StaleDelivery);
        }
        self.last_delivery = None;
        self.start_alignment_v_template(alignment, delimiter)
    }

    pub(super) fn get_next_with_control_sequence_creation(
        &mut self,
        allow_control_sequence_creation: bool,
    ) -> Result<Option<CommandReplayDelivery>, CommandError> {
        loop {
            if let Some(episode) = self.take_ready_replay_completion() {
                return Ok(Some(CommandReplayDelivery::Completed(episode)));
            }
            let Some(delivery) = self.take_input_token(allow_control_sequence_creation)? else {
                if let Some(episode) = self.take_ready_replay_completion() {
                    return Ok(Some(CommandReplayDelivery::Completed(episode)));
                }
                // §360: a `\read` pseudo-file's line has ended, which is
                // `cur_cmd:=0; cur_chr:=0; return` -- an ordinary end of
                // line inside a live `read_toks`, not end of input, so
                // `check_outer_validity` must not run and no runaway may be
                // reported.
                if std::mem::take(&mut self.read_line_ended) {
                    return Ok(None);
                }
                if self.recover_runaway_eof()? {
                    continue;
                }
                return Ok(None);
            };
            let DeliveredToken {
                spelling,
                level,
                position,
                behavior,
                source_provenance,
                direct_source,
            } = delivery;

            if let Token::Param(slot) = spelling.semantic_token() {
                let replay = self
                    .command
                    .replay_out_parameter(level, slot)
                    .map_err(|_| CommandError::input_invariant())?;
                if let OutParameterReplay::Pushed(_parameter_level) = replay {
                    observe!(
                        self,
                        CommandObservation::Input(InputRecord {
                            transition: InputTransition::Push,
                            reason: InputReason::Parameter,
                            source_name: None,
                            source: None,
                            level: _parameter_level.0,
                            position: 0,
                        }),
                    );
                    continue;
                }
            }

            let delivery_stamp = DeliveryStamp::new(level.0, position, self.next_delivery_sequence);
            self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
            if matches!(
                spelling.semantic_token(),
                Token::Cs(_)
                    | Token::Char {
                        cat: Catcode::Active,
                        ..
                    }
            ) {
                self.record_meaning_lookup();
            }
            let mut command = CurrentCommand::resolve(
                spelling,
                delivery_stamp,
                source_provenance,
                direct_source,
                &mut self.state,
            );
            self.record_token_frame(!matches!(
                self.command.scanner.status(),
                crate::processor::ScannerStatus::Normal
            ));
            if matches!(
                behavior,
                TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence)
            ) {
                command.suppress_expandable();
            }
            // Outer-validity recovery canonically backs up this exact raw
            // delivery before substituting its recovery space.
            self.last_delivery = Some(delivery_stamp);
            self.check_outer_validity_entry(&mut command)?;
            let previous_align_state = self.command.alignment.align_state;
            let adjustment = self.command.alignment.classify_delivery(&mut command);
            command.set_alignment_adjustment(adjustment);
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
                self.observe_raw_delivery(&command);
            }
            return Ok(Some(CommandReplayDelivery::Command(command)));
        }
    }

    fn take_input_token(
        &mut self,
        allow_control_sequence_creation: bool,
    ) -> Result<Option<DeliveredToken>, CommandError> {
        enum ActiveInput {
            Source {
                identity: InputLevelId,
                position: u64,
                backing: crate::input::RegisteredSource,
            },
            Tokens {
                identity: InputLevelId,
                index: usize,
            },
        }

        loop {
            if self.command.has_ready_replay_completion() {
                return Ok(None);
            }
            let Some(level) = self.command.input.levels.last().map(|level| match level {
                // Delivery needs only the stable level coordinates and the
                // immutable physical registration. Cloning `SourceLevel`
                // here used to allocate a fresh `Box` and copy the complete
                // mutable lexer/line cursor for every source token, even
                // though `next_source_step` immediately advanced the live
                // cursor instead. Keep the live level canonical and retain
                // only the cheap Arc-backed registration across that call.
                InputLevel::Source(source) => ActiveInput::Source {
                    identity: source.identity(),
                    position: source.cursor.next_physical_offset,
                    backing: source.cursor.backing.clone(),
                },
                InputLevel::Tokens(cursor) => ActiveInput::Tokens {
                    identity: cursor.identity(),
                    index: cursor.position(),
                },
            }) else {
                observe!(
                    self,
                    CommandObservation::Input(InputRecord {
                        transition: InputTransition::Stop,
                        reason: InputReason::Source,
                        // Nothing is left on the stack, so what has stopped is
                        // §331's `name=0` base terminal level.
                        source_name: Some(SourceNameClass::Terminal),
                        source: None,
                        level: 0,
                        position: 0,
                    }),
                );
                return Ok(None);
            };
            match level {
                ActiveInput::Source {
                    identity,
                    position,
                    backing,
                } => {
                    self.ensure_source_registration(&backing);
                    match self.next_source_step() {
                        SourceTokenizationStep::Token(token) => {
                            self.ensure_replacement_line_registration();
                            let spelling =
                                self.source_spelling(&token, allow_control_sequence_creation);
                            let Some(InputLevel::Source(source)) =
                                self.command.input.levels.last_mut()
                            else {
                                return Err(CommandError::input_invariant());
                            };
                            if source.frame.identity() != identity.0
                                || source.frame.advance().is_none()
                            {
                                return Err(CommandError::input_invariant());
                            }
                            return Ok(Some(DeliveredToken {
                                spelling,
                                level: identity,
                                position,
                                behavior: TokenBehavior::Ordinary,
                                source_provenance: Some(token.provenance()),
                                direct_source: true,
                            }));
                        }
                        SourceTokenizationStep::InvalidCharacter(_) => {
                            // TeX82 §345 temporarily sets
                            // `deletions_allowed:=false`, calls `error`, restores
                            // it, and goes to `restart`. Umber's error channel
                            // deliberately has deletion disabled (tex-state's
                            // print contract), so queuing this one report before
                            // continuing has the same recovery boundary: the
                            // offending character is consumed exactly once and
                            // no later token is silently discarded.
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
                        SourceTokenizationStep::End => {
                            // e-TeX 2.6 etex.ch §24.362 inserts a non-null
                            // `\everyeof` above the still-live source. Its
                            // token-list level must therefore push and retire
                            // before §329 retires the pseudo-file.
                            if let Some(level) =
                                self.command.begin_pending_every_eof(&self.state, identity)
                            {
                                self.observe(CommandObservation::Input(InputRecord {
                                    transition: InputTransition::Push,
                                    reason: InputReason::EveryEof,
                                    source_name: None,
                                    source: None,
                                    level: level.0,
                                    position: 0,
                                }));
                                continue;
                            }
                            // TeX82 §343 checks outer validity immediately
                            // after `end_file_reading`, before `get_next`
                            // resumes the caller's input level.  In
                            // particular, a skipped conditional that reaches
                            // EOF in a nested `\\input` must insert frozen
                            // `\\fi` above the parent, rather than allowing
                            // the parent's next token to escape `pass_text`.
                            if self
                                .state
                                .int_param(tex_state::env::banks::IntParam::TRACING_NESTING)
                                > 1
                            {
                                let context = match self.command.input.levels.last() {
                                    Some(InputLevel::Source(source))
                                        if source.identity() == identity =>
                                    {
                                        self.command
                                            .output_retiring_source_context(source, &self.state)
                                    }
                                    _ => return Err(CommandError::input_invariant()),
                                };
                                self.pending_file_warning_context = Some((identity, context));
                            }
                            let restart = self.retire_and_restart(identity)?;
                            // §362 prints its `)` *before* `end_file_reading`
                            // and `check_outer_validity`, so the bracket the
                            // retirement just queued has to reach the
                            // transcript here, not when the step ends: the
                            // very next thing `recover_runaway_eof` may print
                            // is `Incomplete \if...` or a runaway report,
                            // which tex.web puts outside the file it has
                            // already closed.
                            if self.command.semantic_diagnostics.is_empty() {
                                self.command.render_file_framing_events(&mut self.state);
                            }
                            match restart {
                                RetirementRestart::Stop => return Ok(None),
                                RetirementRestart::Continue => {
                                    // TeX82 §343 calls `check_outer_validity`
                                    // after every real source retirement. The
                                    // scanner episode may predate this source:
                                    // EOF still makes that unfinished scan a
                                    // runaway before parent input can resume.
                                    if self.recover_runaway_eof()? {
                                        continue;
                                    }
                                }
                                RetirementRestart::EndV(_) => {
                                    return Err(CommandError::input_invariant());
                                }
                                RetirementRestart::Completed => {
                                    return Ok(None);
                                }
                            }
                        }
                    }
                }
                ActiveInput::Tokens { identity, index } => {
                    let next = {
                        let Some(InputLevel::Tokens(cursor)) = self.command.input.levels.last()
                        else {
                            unreachable!("inspected token level remains a token level")
                        };
                        debug_assert_eq!(cursor.identity(), identity);
                        debug_assert_eq!(cursor.position(), index);
                        Self::next_stored_token(cursor, &self.command.parameters, &self.state)
                    };
                    if let Some((spelling, position, behavior, source_provenance)) = next {
                        let InputLevel::Tokens(cursor) = self
                            .command
                            .input
                            .levels
                            .last_mut()
                            .expect("inspected input level remains live")
                        else {
                            unreachable!("inspected token level remains a token level");
                        };
                        if cursor.frame.advance().map(|position| position as usize) != Some(index) {
                            return Err(CommandError::input_invariant());
                        }
                        return Ok(Some(DeliveredToken {
                            spelling,
                            level: identity,
                            position,
                            behavior,
                            source_provenance,
                            direct_source: false,
                        }));
                    }
                    match self.retire_and_restart(identity)? {
                        RetirementRestart::Stop => return Ok(None),
                        RetirementRestart::Continue => {}
                        RetirementRestart::EndV(level) => {
                            return Ok(Some(DeliveredToken {
                                spelling: TracedTokenWord::pack(
                                    self.state.frozen_end_template_token(),
                                    tex_state::token::OriginId::UNKNOWN,
                                ),
                                level,
                                position: u64::try_from(index)
                                    .map_err(|_| CommandError::input_invariant())?,
                                behavior: TokenBehavior::VTemplate,
                                source_provenance: None,
                                direct_source: false,
                            }));
                        }
                        RetirementRestart::Completed => return Ok(None),
                    }
                }
            }
        }
    }

    fn take_ready_replay_completion(&mut self) -> Option<crate::CommandReplayEpisode> {
        self.command.take_ready_replay_completion()
    }

    fn retire_and_restart(
        &mut self,
        identity: InputLevelId,
    ) -> Result<RetirementRestart, CommandError> {
        let open_depths = self.command.source_open_depths(identity);
        let nesting_context = self
            .pending_file_warning_context
            .take()
            .and_then(|(level, context)| (level == identity).then_some(context));
        let retirement = self
            .command
            .retire_exhausted_input(identity)
            .map_err(|_| CommandError::input_invariant())?;
        let action = retirement.action;
        // e-TeX 2.6 [23.328]'s `file_warning`: `end_file_reading` retiring a
        // real source level (never a `\read` pseudo-file's `EndReadLine`, and
        // never a token-list level) is the one point this level's recorded
        // group/conditional open depth can be compared against the live one.
        if matches!(action, InputRetirementAction::SourcePopped)
            && let Some(open_depths) = open_depths
        {
            self.warn_file_boundary_incomplete(open_depths, nesting_context);
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
            InputRetirementAction::TerminalStop => Ok(RetirementRestart::Stop),
            InputRetirementAction::ReadLineEnded => {
                self.read_line_ended = true;
                Ok(RetirementRestart::Stop)
            }
            InputRetirementAction::VTemplateRetained => {
                // The exhausted frame remains live while `get_next` delivers
                // frozen end-template. Its expanded `endv` is then handled by
                // typed `do_endv`, which retires this exact frame.
                Ok(RetirementRestart::EndV(identity))
            }
            InputRetirementAction::SourcePopped
            | InputRetirementAction::TokenListPopped
            | InputRetirementAction::VTemplatePopped => {
                let previous_align_state = self.command.alignment.align_state;
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
                    Ok(RetirementRestart::Completed)
                } else {
                    Ok(RetirementRestart::Continue)
                }
            }
        }
    }

    fn next_source_step(&mut self) -> SourceTokenizationStep {
        self.command
            .observe_active_source_dependencies(&mut self.state);
        let profile = self.command.profile();
        // TeX82's `firm_up_the_line` captures `end_line_char` when it loads
        // each physical line.  The cursor keeps that captured value through
        // the line, so assignments affect the next refill but cannot rewrite
        // a partially consumed line.
        let endlinechar = self.state.int_param(IntParam::END_LINE_CHAR);
        let step = {
            let mut queries = LiveSourceQueries {
                state: &mut self.state,
            };
            match profile.character_mode() {
                CharacterMode::EightBitExact => self
                    .command
                    .next_exact_source_step(endlinechar, &mut queries),
                CharacterMode::UnicodeExtended => self
                    .command
                    .next_unicode_source_step(endlinechar, &mut queries),
            }
        };
        self.command
            .observe_active_source_dependencies(&mut self.state);
        step
    }

    fn ensure_source_registration(&mut self, source: &crate::input::RegisteredSource) {
        let _ = self
            .state
            .register_source(source.id, source.source_descriptor());
    }

    /// Registers the backing TeX82 §363 installed over the active line.
    ///
    /// The line the file supplied is registered before tokenization starts;
    /// a `\pausing` replacement only exists once §363 has run inside that
    /// step, so its identity is registered as soon as the step yields a token
    /// located in it.
    fn ensure_replacement_line_registration(&mut self) {
        if let Some((source, descriptor)) = self.command.active_line_backing() {
            let _ = self.state.register_source(source, descriptor);
        }
    }

    /// Resolves one scanned source token into its semantic spelling.
    ///
    /// `allow_control_sequence_creation` is TeX82's `no_new_control_sequence`
    /// inverted. §257 sets that flag, §365 clears it only around `get_token`,
    /// and §374 clears it only around `\csname`'s `id_lookup`, so a raw
    /// `get_next` may not enter a new name into the hash table: §259's
    /// `id_lookup` hands it §222's dummy `undefined_control_sequence`
    /// instead. Section 356 sends only multiletter control words to the hash;
    /// it resolves a one-letter word or control symbol to `single_base+c` and
    /// an escape at line end to `null_cs`. Section 372 applies the same length
    /// split to `\csname`, and §351 gives a blank line's `\par` `par_loc`;
    /// all of those fixed eqtb locations exist before any scan.
    fn source_spelling(
        &mut self,
        source_token: &SourceToken,
        allow_control_sequence_creation: bool,
    ) -> TracedTokenWord {
        let token = match source_token {
            SourceToken::Character { code, catcode, .. } => Token::Char {
                ch: character_from_code(*code),
                cat: *catcode,
            },
            SourceToken::ControlSequence { name, kind, .. } => match kind {
                SourceControlSequenceKind::Active => Token::Char {
                    ch: character_from_code(name[0]),
                    cat: Catcode::Active,
                },
                SourceControlSequenceKind::Word
                | SourceControlSequenceKind::Symbol
                | SourceControlSequenceKind::Paragraph
                | SourceControlSequenceKind::Null => {
                    let hashed = *kind == SourceControlSequenceKind::Word && name.len() > 1;
                    name.with_text(|name| {
                        if hashed && !allow_control_sequence_creation {
                            self.state
                                .known_control_sequence(name)
                                .map_or_else(Token::undefined_control_sequence, Token::Cs)
                        } else if hashed {
                            Token::Cs(self.state.intern_hash_control_sequence(name))
                        } else {
                            Token::Cs(self.state.intern_control_sequence(name))
                        }
                    })
                }
            },
        };
        let range = source_token.range();
        let origin = if range.end().saturating_sub(range.start()) == 1 {
            self.state
                .source_token_origin(range.source(), range.start(), range.end())
        } else {
            self.state
                .source_range_origin(range.source(), range.start(), range.end())
        };
        TracedTokenWord::pack(token, origin)
    }

    fn next_stored_token(
        cursor: &TokenCursor,
        parameters: &crate::macro_call::ParameterState,
        stores: &tex_state::CommandContext<'_>,
    ) -> Option<(
        TracedTokenWord,
        u64,
        TokenBehavior,
        Option<SourceProvenance>,
    )> {
        let index = cursor.position();
        let position = u64::try_from(index).ok()?;
        let spelling = match &cursor.payload {
            TokenPayload::Packed(chunk) => chunk.get(index),
            TokenPayload::MacroReplacement {
                admitted,
                definition,
                ..
            } => {
                debug_assert_eq!(parameters.admitted_macro(*admitted), *definition);
                stores
                    .macro_definition(*definition)
                    .replacement_traced_word(index)
                    .map(|spelling| (spelling, None))
            }
            TokenPayload::ArgumentRange { arguments, range } => (index
                < range.end().saturating_sub(range.start()))
            .then(|| parameters.argument_traced_word(*arguments, range.start() + index))
            .flatten()
            .map(|spelling| (spelling, None)),
        }?;
        Some((spelling.0, position, cursor.behavior.clone(), spelling.1))
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
                        && Self::next_stored_token(
                            cursor,
                            &self.command.parameters,
                            &self.state,
                        )
                        .is_none() =>
                {
                    Some(cursor.identity())
                }
                Some(InputLevel::Tokens(_)) | Some(InputLevel::Source(_)) | None => None,
            };
            let Some(identity) = depleted else {
                return Ok(());
            };
            match self.retire_and_restart(identity)? {
                // Finished stored replay episodes queue their completion in
                // command state. Draining continues so the whole depleted run
                // is cleaned off; delivery surfaces each ready ownership
                // boundary before any enclosing source.
                RetirementRestart::Continue | RetirementRestart::Completed => {}
                RetirementRestart::Stop | RetirementRestart::EndV(_) => {
                    return Err(CommandError::input_invariant());
                }
            }
        }
    }

    fn check_outer_validity_entry(
        &mut self,
        command: &mut CurrentCommand,
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
            TokenPayload::transient(std::iter::once(first_token).chain(second_token)),
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
                super::expand::print_esc_text(&self.state, &spelling)
            });
            let context = self.command.output_open_context(&self.state);
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: RUNAWAY_SCAN_DIAGNOSTIC,
                    runaway: Some(crate::state::RunawayPrelude {
                        heading: match &status {
                            ScannerStatus::Defining(_) => "Runaway definition?",
                            ScannerStatus::Matching(_) => "Runaway argument?",
                            ScannerStatus::Aligning(_) => "Runaway preamble?",
                            ScannerStatus::Absorbing(_) => "Runaway text?",
                            ScannerStatus::Normal | ScannerStatus::Skipping(_) => unreachable!(),
                        },
                        partial: match &status {
                            ScannerStatus::Aligning(context) => self
                                .command
                                .transient
                                .builders
                                .iter()
                                .find(|builder| builder.identity == context.builder.0)
                                .map_or_else(String::new, |builder| {
                                    builder
                                        .tokens
                                        .iter()
                                        .fold(String::new(), |mut text, token| {
                                            super::expand::append_token_list_token_text(
                                                &self.state,
                                                token.semantic_token(),
                                                &mut text,
                                            );
                                            text
                                        })
                                }),
                            _ => String::new(),
                        },
                    }),
                    message: format!("{opening} while scanning {kind} of {name}"),
                    help: &[
                        "I suspect you have forgotten a `}', causing me",
                        "to read past where you wanted me to stop.",
                        "I'll try to recover; but if the error is serious,",
                        "you'd better type `E' or `X' now and fix your file.",
                    ],
                    context,
                });
        }
        if let ScannerStatus::Skipping(skipping) = &status {
            let name =
                super::expand::print_esc_text(&self.state, skipping.conditional.canonical_name());
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
            let context = self.command.output_open_context(&self.state);
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: INCOMPLETE_CONDITIONAL_DIAGNOSTIC,
                    runaway: None,
                    message,
                    help,
                    context,
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
                super::expand::append_token_list_token_text(&self.state, token, &mut raw);
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
            |frozen| {
                self.state
                    .frozen_primitive_meaning(frozen)
                    .and_then(|meaning| self.state.primitive_name(meaning))
                    .map(str::to_owned)
            },
        )
    }

    pub(crate) fn observed_command_spelling(
        &self,
        command: &CurrentCommand,
    ) -> crate::observation::ObservedToken {
        if let Some(symbol) = command.control_sequence() {
            // §353's `get_next` resolves an active character through its own
            // `active_base + c` control-sequence cell and records that cell
            // in `cur_cs`, so §365's `cur_tok` is `cs_token_flag + cur_cs`.
            // Observations expose that identity at the current-command
            // boundary, just as they do for escaped control sequences.  The
            // raw token spelling remains available on `CurrentCommand` for
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
            && matches!(command.meaning(), Meaning::Relax)
        {
            // TeX82's observer presents the inaccessible frozen `\relax`
            // inserted by incomplete-conditional recovery as `\relax`.
            // A `\noexpand` target has the same effective meaning but retains
            // its original control-sequence spelling.
            crate::observation::ObservedToken::ControlSequence("relax".into())
        } else if matches!(command.spelling().semantic_token(), Token::Frozen(_))
            && let Some(name) = self.state.primitive_name(command.meaning())
        {
            crate::observation::ObservedToken::ControlSequence(name.into())
        } else {
            self.observed_token(command.spelling())
        }
    }

    fn observe_raw_delivery(&mut self, command: &CurrentCommand) {
        observe!(self, {
            #[cfg(test)]
            {
                self.observation_payloads_built += 1;
            }
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

    pub(crate) fn undo_alignment_delivery(&mut self, command: &CurrentCommand) {
        self.command
            .alignment
            .undo_delivery(command.alignment_adjustment());
    }

    /// Cancels raw brace accounting for a matched `#{` delimiter. The opening
    /// brace was delivered as parameter text, so scalar macro matching must
    /// not leave a group entry for replacement replay to balance later.
    pub(crate) fn undo_delimiter_begin_group_delivery(&mut self) {
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

struct DeliveredToken {
    spelling: TracedTokenWord,
    level: InputLevelId,
    position: u64,
    behavior: TokenBehavior,
    source_provenance: Option<SourceProvenance>,
    /// True only for a token read directly from a physical source cursor.
    /// Backups preserve their diagnostic range while remaining replay input.
    direct_source: bool,
}

enum RetirementRestart {
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
struct LiveSourceQueries<'a, 'b> {
    state: &'a mut tex_state::CommandContext<'b>,
}

impl crate::SourceStepQueries for LiveSourceQueries<'_, '_> {
    fn catcode(&mut self, code: CharacterCode) -> Catcode {
        self.state.catcode(character_from_code(code))
    }

    /// TeX82 §363's `firm_up_the_line`.
    ///
    /// `if pausing>0 then if interaction>nonstop_mode then begin
    /// wake_up_terminal; print_ln; if start<limit then print the buffered
    /// line; first:=limit; prompt_input("=>"); if last>first then move the
    /// typed line down into the buffer ... end`. §71's `prompt_input(#)` is
    /// `print(#); term_input`, so the `print_ln`, the echoed line, and the
    /// `=>` are one printed prompt and one acquired line -- exactly what the
    /// single line-acquisition capability performs.
    fn firm_up_the_line(&mut self, line: &str) -> Option<SourceRegistration> {
        if self.state.int_param(IntParam::PAUSING) <= 0
            || !self.state.interaction_permits_terminal_input()
        {
            return None;
        }
        let prompt = format!("\n{line}=>");
        let replacement = self
            .state
            .input_ln(CommandLineSource::Terminal { prompt: &prompt })?;
        // §363's `if last>first`: a bare carriage return types no line at
        // all, and the line the file supplied stands as it is.
        if replacement.is_empty() {
            return None;
        }
        Some(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            replacement.into_bytes(),
        ))
    }
}

fn character_from_code(code: CharacterCode) -> char {
    crate::profile::token_character(code)
}

/// Whether a delivered command is TeX82's `math_shift` command code -- the
/// single test both §1138's opener and §1197's closer apply to their peeked
/// token, and the reason neither may grow a private notion of "a `$`".
fn is_math_shift(command: &CurrentCommand) -> bool {
    matches!(
        command.meaning(),
        Meaning::CharToken {
            cat: Catcode::MathShift,
            ..
        }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tex_state::Universe;
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive};
    use tex_state::token::{OriginId, Token, TracedTokenWord};

    use super::*;
    use crate::input::{ReplayTrace, RetirementBehavior, SharedTokenBuffer};
    use crate::observation::{
        CommandDeliveryBoundary, CommandObservation, CommandObserver, InputTransition,
    };
    use crate::processor::{
        AbsorbingContext, AlignmentId, AlignmentScanContext, ArgumentBuilderId, ConditionId,
        DefinitionContext, MatchingContext, ScannerWarning, SkippingContext, TokenBuilderId,
    };
    use crate::{
        CommandHostCapabilities, CommandHostContext, CommandState, RegisteredSourceKind,
        SourceRegistration,
    };

    #[derive(Default)]
    struct Recorder(Vec<CommandObservation>);

    impl CommandObserver for Recorder {
        fn committed(&mut self, observation: CommandObservation) {
            self.0.push(observation);
        }
    }

    fn processor<'a>(
        command: &'a mut CommandState,
        universe: &'a mut Universe,
        capabilities: &'a mut CommandHostCapabilities,
    ) -> CommandProcessor<'a> {
        CommandProcessor::new(
            command,
            universe.command_context(),
            CommandHostContext::new(capabilities),
        )
    }

    fn templates() -> crate::AlignmentCellTemplates {
        let universe = Universe::new();
        crate::AlignmentCellTemplates {
            u_template: None,
            v_template: tex_state::input::TracedTokenList::synthetic(
                universe.token_list_ref(tex_state::ids::TokenListId::EMPTY),
            ),
        }
    }

    #[test]
    fn invalid_source_character_reports_and_restarts_at_following_token() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"\x7fA"[..]),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        universe.set_catcode('\x7f', tex_state::token::Catcode::Invalid);
        let mut capabilities = CommandHostCapabilities::default();

        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let following = processor
            .get_next()
            .expect("invalid-character recovery succeeds")
            .expect("following token remains live");
        assert!(matches!(
            following.meaning(),
            Meaning::CharToken {
                ch: 'A',
                cat: tex_state::token::Catcode::Letter,
            }
        ));

        let diagnostics = processor.take_semantic_diagnostics();
        let [
            crate::CommandSemanticDiagnostic::Recoverable {
                identity: INVALID_SOURCE_CHARACTER_DIAGNOSTIC,
                message,
                help,
                context,
                ..
            },
        ] = diagnostics.as_slice()
        else {
            panic!("TeX82 §345 reports once before restart: {diagnostics:?}");
        };
        assert_eq!(message, "Text line contains an invalid character");
        assert_eq!(
            *help,
            [
                "A funny symbol that I can't read has just been input.",
                "Continue, and I'll forget that it ever happened.",
            ]
        );
        assert!(
            context.contains("^^?"),
            "the report captures the consumed invalid character's source context: {context:?}"
        );
        assert!(
            processor.take_semantic_diagnostics().is_empty(),
            "the consumed invalid character is not diagnosed twice"
        );
    }

    #[test]
    fn unbalanced_alignment_delimiter_recovery_keeps_backup_before_inserted_brace() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"{&".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let alignment = crate::AlignmentIdentity::new(17);
        command.begin_alignment(alignment);
        command
            .apply_alignment_request(
                &universe.command_context(),
                crate::AlignmentRequest::BeginCell {
                    alignment,
                    templates: templates(),
                },
            )
            .expect("cell begins");
        command
            .apply_alignment_request(
                &universe.command_context(),
                crate::AlignmentRequest::InstallCellTemplate(alignment),
            )
            .expect("empty template installs");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        assert!(
            matches!(processor.get_x_alignment_delivery(false).expect("brace delivers"), Some(crate::AlignmentDelivery::Command(command)) if matches!(command.meaning(), Meaning::CharToken { cat: Catcode::BeginGroup, .. }))
        );
        // §1126 routes the delimiter to `main_control`, so it is delivered as
        // an ordinary command; `get_next` never intercepts it here.
        let delimiter = match processor
            .get_x_alignment_delivery(false)
            .expect("unbalanced delimiter delivers")
        {
            Some(crate::AlignmentDelivery::Command(command))
                if matches!(
                    command.meaning(),
                    Meaning::CharToken {
                        cat: Catcode::AlignmentTab,
                        ..
                    }
                ) =>
            {
                command
            }
            other => panic!("expected a main-control tab-mark delivery, got {other:?}"),
        };
        assert!(
            matches!(
                processor
                    .recover_align_error(delimiter)
                    .expect("TeX82 align_error recovery is command-owned"),
                Some(Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                })
            ),
            "§1127 inserts the missing right brace at align_state 1"
        );
        assert!(
            matches!(processor.get_x_alignment_delivery(false).expect("inserted brace delivers"), Some(crate::AlignmentDelivery::Command(command)) if matches!(command.meaning(), Meaning::CharToken { cat: Catcode::EndGroup, .. }))
        );
        assert!(matches!(
            processor
                .get_x_alignment_delivery(false)
                .expect("replayed tab intercepts"),
            Some(crate::AlignmentDelivery::Event(
                crate::AlignmentDeliveryEvent::EndTemplate(_)
            ))
        ));

        let backup = recorder.0.iter().position(|record| matches!(record, CommandObservation::Input(input) if input.transition == InputTransition::Backup)).expect("delimiter backup is observed");
        let recovery = recorder.0.iter().position(|record| matches!(record, CommandObservation::Input(input) if input.transition == InputTransition::Recovery)).expect("inserted brace recovery is observed");
        assert!(
            backup < recovery,
            "§1127 backs up the delimiter before ins_error inserts its brace"
        );
    }

    /// TeX82 §1127 selects §1128's `@<Express consternation@>` branch --
    /// report and drop, with no `back_input` and no `ins_error` -- whenever
    /// `abs(align_state)>2`.  That is the branch every delimiter outside an
    /// alignment takes, because §331 starts `align_state` at 1000000 and §772
    /// restores it across every alignment.
    #[test]
    fn align_error_drops_the_delimiter_when_no_alignment_entry_is_in_progress() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"&".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        assert_eq!(
            command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE,
            "§331 initializes align_state far above the recovery window"
        );
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        let delimiter = processor
            .get_x_token()
            .expect("tab mark delivers")
            .expect("tab mark token");
        assert!(
            processor
                .recover_align_error(delimiter)
                .expect("align_error is command-owned")
                .is_none(),
            "§1128 drops the delimiter instead of inserting a brace"
        );
        let next = processor
            .get_x_token()
            .expect("input continues")
            .expect("the synthetic end-line space is still there");
        assert!(
            matches!(
                next.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ),
            "the dropped delimiter is not backed up, so the line's end-line \
             space follows it directly"
        );
    }

    /// TeX82 §772's `push_alignment`/`pop_alignment` save and restore
    /// `align_state` around *every* alignment, so material after `fin_align`
    /// sees the running brace count that was live at `\halign`, not zero.
    #[test]
    fn finishing_an_alignment_restores_the_saved_outer_align_state() {
        let mut command = CommandState::default();
        let mut universe = Universe::new_with_plain_catcodes();
        command.alignment.align_state = crate::processor::TOP_LEVEL_ALIGN_STATE + 1;
        let alignment = crate::AlignmentIdentity::new(23);
        command.begin_alignment(alignment);
        assert_eq!(
            command.alignment.align_state,
            crate::processor::alignment::PREAMBLE_ALIGN_STATE
        );
        command
            .apply_alignment_request(
                &universe.command_context(),
                crate::AlignmentRequest::Finish(alignment),
            )
            .expect("alignment finishes");
        assert_eq!(
            command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE + 1
        );
        assert!(command.alignment.align_stack.is_empty());
    }

    #[test]
    fn align_group_closing_brace_recovery_backs_up_before_frozen_cr() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"}".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let alignment = crate::AlignmentIdentity::new(18);
        command.begin_alignment(alignment);
        command
            .apply_alignment_request(
                &universe.command_context(),
                crate::AlignmentRequest::BeginCell {
                    alignment,
                    templates: templates(),
                },
            )
            .expect("empty-template cell begins");
        recovery_primitives(&mut universe);
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        let event = match processor
            .get_x_alignment_delivery(false)
            .expect("closing brace delivers")
        {
            Some(crate::AlignmentDelivery::Event(
                event @ crate::AlignmentDeliveryEvent::ClosingBrace(_),
            )) => event,
            other => panic!("expected typed align-group closing brace, got {other:?}"),
        };
        processor
            .recover_alignment_closing_brace(event)
            .expect("TeX82 §1103 recovery is command-owned");
        assert!(matches!(
            processor
                .get_x_alignment_delivery(false)
                .expect("frozen cr delivers"),
            Some(crate::AlignmentDelivery::Event(
                crate::AlignmentDeliveryEvent::EndTemplate(_)
            ))
        ));

        let backup = recorder
            .0
            .iter()
            .position(|record| matches!(record, CommandObservation::Input(input) if input.transition == InputTransition::Backup))
            .expect("brace backup is observed");
        let recovery = recorder
            .0
            .iter()
            .position(|record| matches!(record, CommandObservation::Recovery(recovery)
                if recovery.kind == RecoveryKind::InsertedToken
                    && recovery.tokens == vec![crate::observation::ObservedToken::ControlSequence("cr".into())]))
            .expect("frozen cr insertion is observed canonically");
        assert!(
            backup < recovery,
            "§1103 backs up the brace before ins_error"
        );
    }

    #[test]
    fn source_delivery_restarts_after_retirement_and_accounts_literal_braces() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"{x}".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let first = processor
            .get_next()
            .expect("opening brace delivers")
            .expect("input is live");
        assert_eq!(
            first.spelling().semantic_token(),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup
            }
        );
        assert_eq!(first.delivery_stamp().position(), 0);
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE + 1
        );
        assert_eq!(
            processor
                .get_next()
                .expect("letter delivers")
                .expect("input is live")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert_eq!(
            processor
                .get_next()
                .expect("closing brace delivers")
                .expect("input is live")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup
            }
        );
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE
        );
        assert_eq!(
            processor
                .get_next()
                .expect("synthetic endline space delivers")
                .expect("input is live")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space
            }
        );
        assert!(
            processor
                .get_next()
                .expect("source retirement succeeds")
                .is_none()
        );
    }

    #[test]
    fn observer_preserves_active_control_sequence_identity_at_raw_delivery() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"~".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        universe.set_catcode('~', Catcode::Active);
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        let delivered = processor
            .get_next()
            .expect("active token delivers")
            .expect("input is live");

        assert!(matches!(
            delivered.spelling().semantic_token(),
            Token::Char {
                ch: '~',
                cat: Catcode::Active,
            }
        ));
        assert!(matches!(
            recorder.0.as_slice(),
            [CommandObservation::Command(delivery)]
                if delivery.boundary == CommandDeliveryBoundary::Raw
                    && delivery.command == "undefined_cs"
                    && delivery.spelling
                        == crate::ObservedToken::ControlSequence("~".into())
        ));
    }

    fn open_generated(command: &mut CommandState, bytes: &'static [u8]) {
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(bytes),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
    }

    /// TeX82 §365 clears `no_new_control_sequence` only inside `get_token`,
    /// so §259's `id_lookup` gives a raw `get_next` §222's shared dummy
    /// `undefined_control_sequence` for a multiletter name the hash table has
    /// never held. §263's `sprint_cs` spells that slot with string number 0,
    /// which §48 built as the printable form of character 0.
    #[test]
    fn raw_delivery_never_enters_a_new_multiletter_name_in_the_hash_table() {
        let mut command = CommandState::default();
        open_generated(&mut command, b"\\brm\\brm");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            let first = processor
                .get_next()
                .expect("raw delivery")
                .expect("input is live");
            assert!(
                first
                    .spelling()
                    .semantic_token()
                    .is_undefined_control_sequence()
            );
            assert_eq!(first.meaning(), Meaning::Undefined);
            // A second raw scan of the same name must still find nothing:
            // the first one entered no hash entry for it to reuse.
            let second = processor
                .get_next()
                .expect("raw delivery")
                .expect("input is live");
            assert!(
                second
                    .spelling()
                    .semantic_token()
                    .is_undefined_control_sequence()
            );
        }
        assert!(universe.symbol("brm").is_none());
        let spellings: Vec<_> = recorder
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Command(delivery) => Some(delivery),
                _ => None,
            })
            .map(|delivery| (delivery.command.clone(), delivery.spelling.clone()))
            .collect();
        assert_eq!(
            spellings,
            vec![
                (
                    "undefined_cs".to_owned(),
                    crate::ObservedToken::ControlSequence("^^@".into())
                ),
                (
                    "undefined_cs".to_owned(),
                    crate::ObservedToken::ControlSequence("^^@".into())
                ),
            ]
        );
    }

    /// The other half of §365: `get_token` is one of the two readers that may
    /// create, so the name it scans becomes a hash entry a later raw
    /// `get_next` then finds and spells.
    #[test]
    fn get_token_enters_a_new_multiletter_name_a_later_raw_scan_then_finds() {
        let mut command = CommandState::default();
        open_generated(&mut command, b"\\brm\\brm");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            let created = processor
                .get_token()
                .expect("raw delivery")
                .expect("input is live");
            assert!(matches!(created.spelling().semantic_token(), Token::Cs(_)));
            let found = processor
                .get_next()
                .expect("raw delivery")
                .expect("input is live");
            assert_eq!(
                found.spelling().semantic_token(),
                created.spelling().semantic_token()
            );
        }
        assert!(universe.symbol("brm").is_some());
    }

    /// §354 resolves a control symbol to `single_base+c` and an escape at
    /// line end to `null_cs`, and §351 gives a blank line's `\par` `par_loc`.
    /// None of those consult the hash table, so `no_new_control_sequence`
    /// cannot turn them into §222's dummy.
    #[test]
    fn permanent_control_sequence_locations_are_never_the_undefined_dummy() {
        let mut command = CommandState::default();
        open_generated(&mut command, b"\\+\\\n\n");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        while let Some(delivered) = processor.get_next().expect("raw delivery") {
            assert!(
                !delivered
                    .spelling()
                    .semantic_token()
                    .is_undefined_control_sequence()
            );
        }
    }

    #[test]
    fn observer_records_raw_expanded_input_and_deterministic_rollback_provenance() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"x".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let snapshot = command.snapshot();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut first = Recorder::default();
        {
            let mut processor =
                processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut first);
            processor
                .get_x_token()
                .expect("expanded delivery")
                .expect("token");
            while processor.get_next().expect("terminal retirement").is_some() {}
        }

        assert!(first.0.iter().any(|record| matches!(
            record,
            CommandObservation::Command(command)
                if command.boundary == CommandDeliveryBoundary::Raw
        )));
        assert!(first.0.iter().any(|record| matches!(
            record,
            CommandObservation::Command(command)
                if command.boundary == CommandDeliveryBoundary::Expanded
        )));
        let source_deliveries = first
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::Command(command) => Some(command),
                _ => None,
            })
            .take(2)
            .collect::<Vec<_>>();
        assert_eq!(source_deliveries.len(), 2);
        assert_ne!(source_deliveries[0].provenance.origin, OriginId::UNKNOWN);
        assert_eq!(
            source_deliveries[0].provenance.source_range,
            Some(crate::SourceRange::new(source, 0, 1))
        );
        assert_eq!(
            source_deliveries[0].provenance.source_location,
            Some(crate::SourceLocation::new(source, 0))
        );
        assert_eq!(
            source_deliveries[0].provenance.source_range,
            source_deliveries[1].provenance.source_range,
            "raw and expanded delivery retain the registered source range"
        );
        assert_eq!(
            source_deliveries[0].provenance.origin, source_deliveries[1].provenance.origin,
            "raw and expanded delivery retain one traced origin identity"
        );
        assert!(first.0.iter().any(|record| matches!(
            record,
            CommandObservation::Input(input) if input.transition == InputTransition::Stop
        )));

        command.rollback(snapshot).expect("rollback succeeds");
        let mut second = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut second);
            processor
                .get_x_token()
                .expect("replayed delivery")
                .expect("token");
            while processor
                .get_next()
                .expect("replayed terminal retirement")
                .is_some()
            {}
        }
        assert_eq!(
            first.0, second.0,
            "rollback must replay observer provenance exactly"
        );
    }

    #[test]
    fn escape_before_synthetic_endline_keeps_the_endline_location() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"\\ \n".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        universe.set_catcode('\r', Catcode::Active);
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let endline = processor
            .get_next()
            .expect("control symbol delivers")
            .expect("source is live");
        assert!(matches!(endline.spelling().semantic_token(), Token::Cs(_)));
        assert_eq!(
            endline.source_location(),
            Some(crate::SourceLocation::new(source, 1)),
            "TeX82 §354 scans the §362 synthetic buffer[limit] after the escape"
        );
    }

    #[test]
    fn ordinary_control_symbol_and_word_keep_their_final_physical_locations() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"\\!\\word".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let symbol = processor
            .get_token()
            .expect("control symbol delivers")
            .expect("source is live");
        assert_eq!(
            symbol.source_location(),
            Some(crate::SourceLocation::new(source, 1))
        );
        let word = processor
            .get_token()
            .expect("control word delivers")
            .expect("source is live");
        assert_eq!(
            word.source_location(),
            Some(crate::SourceLocation::new(source, 6))
        );
    }

    #[test]
    fn observer_projects_hfil_as_hskip_at_raw_and_expanded_boundaries() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(br"\hfil".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        universe.install_primitive_meaning(
            "hfil",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HFil),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            processor
                .get_x_token()
                .expect("hfil delivers")
                .expect("input remains live");
        }

        let deliveries = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::Command(command) => Some(command),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            deliveries.as_slice(),
            [raw, expanded]
                if raw.boundary == CommandDeliveryBoundary::Raw
                    && expanded.boundary == CommandDeliveryBoundary::Expanded
                    && raw.spelling == crate::ObservedToken::ControlSequence("hfil".into())
                    && raw.command == "hskip"
                    && raw.command_operand == Some(0)
                    && expanded.command == raw.command
                    && expanded.command_operand == raw.command_operand
        ));
    }

    #[test]
    fn observer_preserves_hskip_skip_code_at_raw_and_expanded_boundaries() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(br"\hskip".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        universe.install_primitive_meaning(
            "hskip",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            processor
                .get_x_token()
                .expect("hskip delivers")
                .expect("input remains live");
        }

        let deliveries = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::Command(command) => Some(command),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            deliveries.as_slice(),
            [raw, expanded]
                if raw.boundary == CommandDeliveryBoundary::Raw
                    && expanded.boundary == CommandDeliveryBoundary::Expanded
                    && raw.spelling == crate::ObservedToken::ControlSequence("hskip".into())
                    && raw.command == "hskip"
                    && raw.command_operand == Some(4)
                    && expanded.command == raw.command
                    && expanded.command_operand == raw.command_operand
        ));
    }

    #[test]
    fn observer_preserves_sparse_register_type_and_index_at_both_boundaries() {
        let mut command = CommandState::new(crate::CommandProfile::ETEX26);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(br"\alias".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let alias = universe.intern("alias").symbol();
        universe.set_meaning(alias, Meaning::SkipRegister(32_767));
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            processor
                .get_x_token()
                .expect("sparse shorthand delivers")
                .expect("input remains live");
        }

        let deliveries = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::Command(command) => Some(command),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            deliveries.as_slice(),
            [raw, expanded]
                if raw.boundary == CommandDeliveryBoundary::Raw
                    && expanded.boundary == CommandDeliveryBoundary::Expanded
                    && raw.command == "register"
                    && raw.command_operand.is_none()
                    && raw.semantic_operand.as_deref() == Some("skip:32767")
                    && expanded.semantic_operand == raw.semantic_operand
        ));
    }

    #[test]
    fn observer_projects_row_returns_as_car_ret_at_raw_and_expanded_boundaries() {
        for (name, source_bytes, primitive, operand) in [
            ("cr", br"\cr".as_slice(), UnexpandablePrimitive::Cr, 257),
            (
                "crcr",
                br"\crcr".as_slice(),
                UnexpandablePrimitive::CrCr,
                258,
            ),
        ] {
            let mut command = CommandState::default();
            let source = command
                .register_source(SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(source_bytes),
                ))
                .expect("source registers");
            command
                .open_registered_source(source)
                .expect("source opens");
            let mut universe = Universe::new_with_plain_catcodes();
            universe.install_primitive_meaning(name, Meaning::UnexpandablePrimitive(primitive));
            let mut capabilities = CommandHostCapabilities::default();
            let mut recorder = Recorder::default();
            {
                let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                    .with_observer(&mut recorder);
                processor
                    .get_x_token()
                    .expect("row return delivers")
                    .expect("input remains live");
            }

            let deliveries = recorder
                .0
                .iter()
                .filter_map(|record| match record {
                    CommandObservation::Command(command) => Some(command),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(matches!(
                deliveries.as_slice(),
                [raw, expanded]
                    if raw.boundary == CommandDeliveryBoundary::Raw
                        && expanded.boundary == CommandDeliveryBoundary::Expanded
                        && raw.spelling == crate::ObservedToken::ControlSequence(name.into())
                        && raw.command == "car_ret"
                        && raw.command_operand == Some(operand)
                        && expanded.command == raw.command
                        && expanded.command_operand == raw.command_operand
            ));
        }
    }

    #[test]
    fn absent_observer_has_no_delivery_or_snapshot_effect() {
        let mut observed = CommandState::default();
        let mut unobserved = CommandState::default();
        for state in [&mut observed, &mut unobserved] {
            let source = state
                .register_source(SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(b"xy".as_slice()),
                ))
                .expect("source registers");
            state.open_registered_source(source).expect("source opens");
        }
        let mut observed_universe = Universe::new_with_plain_catcodes();
        let mut unobserved_universe = Universe::new_with_plain_catcodes();
        let mut observed_capabilities = CommandHostCapabilities::default();
        let mut unobserved_capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(
                &mut observed,
                &mut observed_universe,
                &mut observed_capabilities,
            )
            .with_observer(&mut recorder);
            while processor.get_next().expect("delivery succeeds").is_some() {}
        }
        {
            let mut processor = processor(
                &mut unobserved,
                &mut unobserved_universe,
                &mut unobserved_capabilities,
            );
            while processor.get_next().expect("delivery succeeds").is_some() {}
        }
        assert!(!recorder.0.is_empty());
        assert_eq!(observed, unobserved);
    }

    #[test]
    fn stored_replay_delivers_once_then_restarts_to_the_underlying_source() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"s".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let stored = universe.finish_traced_token_list(&[TracedTokenWord::pack(
            Token::Char {
                ch: 't',
                cat: Catcode::Other,
            },
            OriginId::UNKNOWN,
        )]);
        command.push_token_level(
            TokenPayload::stored(
                universe.tokens(stored.token_list()).tokens(),
                universe.origin_list(stored.origin_ref()).iter(),
            ),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::MacroReplacement,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        assert_eq!(
            processor
                .get_next()
                .expect("stored token delivers")
                .expect("stored replay is live")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 't',
                cat: Catcode::Other
            }
        );
        assert_eq!(
            processor
                .get_next()
                .expect("underlying source token delivers")
                .expect("underlying source is live")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 's',
                cat: Catcode::Letter
            }
        );
    }

    #[test]
    fn backup_replays_a_literal_brace_once_and_rejects_its_stale_stamp() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"{".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let opening = processor
            .get_token()
            .expect("opening brace delivers")
            .expect("source is live");
        let original_stamp = opening.delivery_stamp();
        let original_range = opening.source_range();
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE + 1
        );
        processor
            .back_input(opening)
            .expect("exact delivery backs up");
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE
        );

        let replayed = processor
            .get_next()
            .expect("backed-up brace delivers")
            .expect("backup is live");
        assert_eq!(replayed.delivery_stamp().position(), 0);
        assert_ne!(replayed.delivery_stamp(), original_stamp);
        assert_eq!(
            replayed.source_range(),
            original_range,
            "backed-up direct-source commands retain their committed range"
        );
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE + 1
        );
        processor
            .back_input(replayed)
            .expect("replayed delivery backs up");
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE
        );
    }

    #[test]
    fn decoded_caret_spelling_keeps_its_raw_span_and_terminal_location_through_backup() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"^^41".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let decoded = processor
            .get_next()
            .expect("caret spelling delivers")
            .expect("source is live");
        assert_eq!(
            decoded.source_range(),
            Some(crate::SourceRange::new(source, 0, 4))
        );
        assert_eq!(
            decoded.source_location(),
            Some(crate::SourceLocation::new(source, 3))
        );
        processor
            .back_input(decoded)
            .expect("decoded token backs up");

        let replayed = processor
            .get_next()
            .expect("backed-up spelling delivers")
            .expect("backup is live");
        assert_eq!(
            replayed.source_range(),
            Some(crate::SourceRange::new(source, 0, 4))
        );
        assert_eq!(
            replayed.source_location(),
            Some(crate::SourceLocation::new(source, 3))
        );
    }

    #[test]
    fn etex_aftergroup_prepend_promotes_inline_backup_and_preserves_order() {
        let mut command = CommandState::new(crate::CommandProfile::ETEX26);
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let words = ['a', 'b', 'c'].map(|ch| {
            tex_state::token::RootedTracedTokenWord::unowned(TracedTokenWord::pack(
                Token::Char {
                    ch,
                    cat: Catcode::Other,
                },
                OriginId::UNKNOWN,
            ))
        });

        processor
            .back_input_aftergroup_tokens(words.clone())
            .expect("aftergroup tokens back up");
        let Some(InputLevel::Tokens(cursor)) = processor.command.input.levels.last() else {
            panic!("aftergroup backup is a token level");
        };
        assert!(matches!(cursor.payload, TokenPayload::Packed(ref chunk) if chunk.is_backed_up()));
        assert_eq!(
            (0..3)
                .map(|index| cursor
                    .payload
                    .backed_up_get(index)
                    .expect("aftergroup uses backed-up storage")
                    .spelling
                    .semantic_token())
                .collect::<Vec<_>>(),
            words.map(|word| word.word().semantic_token())
        );
    }

    #[test]
    fn token_level_backup_creates_an_explicit_replay_level() {
        let mut command = CommandState::default();
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let first = processor
            .get_next()
            .expect("token delivers")
            .expect("token level is live");
        processor.back_input(first).expect("token backs up");
        // TeX82 §325 conserves stack space first: the one-token list is
        // depleted by that delivery, so it retires before the backup is
        // pushed, leaving the backup alone rather than stacked on a dead level.
        assert_eq!(processor.command.input.levels.len(), 1);
        let replayed = processor
            .get_next()
            .expect("backed-up token delivers")
            .expect("token level is live");
        assert_eq!(
            replayed.spelling().semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }
        );
    }

    #[test]
    fn main_control_delivery_surfaces_top_level_delimiters_and_backup_replays_them() {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(17);
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(alignment, templates())
            .expect("cell begins");
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Char {
                    ch: '&',
                    cat: Catcode::AlignmentTab,
                },
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        // §342 runs §789's insertion inside `get_next`, so no scanner ever
        // sees the delimiter. Main control is the one reader that does:
        // §789 stores the delimiter in `extra_info(cur_align)` for §791's
        // `fin_col`, and that identity is executor-owned, so the delimiter
        // is surfaced as a typed event for the executor to hand back.
        let crate::AlignmentDelivery::Event(crate::AlignmentDeliveryEvent::EndTemplate(delimiter)) =
            processor
                .get_x_alignment_delivery(false)
                .expect("delimiter delivers")
                .expect("input is live")
        else {
            panic!("a top-level delimiter is a typed alignment event");
        };
        assert!(matches!(
            delimiter.meaning(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
        ));
        assert_eq!(processor.command.alignment.align_state, 1_000_000);
        processor
            .back_input(delimiter)
            .expect("delimiter backs up exactly");
        assert_eq!(processor.command.alignment.align_state, 0);
        assert!(matches!(
            processor
                .get_x_alignment_delivery(false)
                .expect("delimiter replays")
                .expect("backup is live"),
            crate::AlignmentDelivery::Event(crate::AlignmentDeliveryEvent::EndTemplate(_))
        ));
    }

    #[test]
    fn end_template_alias_expands_without_starting_a_v_template() {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(18);
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(alignment, templates())
            .expect("cell begins");
        let mut universe = Universe::new_with_plain_catcodes();
        let alias = universe.intern("endt").symbol();
        universe.set_meaning(
            alias,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate),
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Cs(alias),
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let crate::AlignmentDelivery::Command(endv) = processor
            .get_x_alignment_delivery(false)
            .expect("alias expands")
            .expect("input is live")
        else {
            panic!("an end-template alias is not an intercepted delimiter");
        };
        assert!(matches!(endv.meaning(), Meaning::EndV));
        assert!(endv.spelling().semantic_token().is_frozen_endv());
        assert_eq!(processor.command.alignment.align_state, CELL_ALIGN_STATE);
        assert!(processor.command.alignment.active_cell.is_some());
    }

    /// TeX82 §342's `@<If an alignment entry has just ended, take appropriate
    /// action@>` is the tail of §341's `get_next`, so §789's ⟨v_j⟩ template
    /// becomes the live input before any raw reader sees the delimiter. The
    /// token `get_next` returns is the template's first token, never the tab
    /// mark, `\span`, or `\cr` that ended the entry (`umber2-johp.258`).
    #[test]
    fn get_next_inserts_the_v_template_for_a_top_level_alignment_delimiter() {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(19);
        let mut universe = Universe::new_with_plain_catcodes();
        let v_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list_ref(&[
                Token::Char {
                    ch: 'v',
                    cat: Catcode::Letter,
                },
            ]));
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(
                alignment,
                crate::AlignmentCellTemplates {
                    u_template: None,
                    v_template,
                },
            )
            .expect("cell begins");
        command
            .install_alignment_cell_template(&universe.command_context(), alignment)
            .expect("cell without a u-template installs");
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Char {
                    ch: '&',
                    cat: Catcode::AlignmentTab,
                },
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        assert_eq!(
            processor
                .get_next()
                .expect("the v-template's own first token delivers")
                .expect("input is live")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'v',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(processor.command.alignment.align_state, 1_000_000);
    }

    #[test]
    fn intercepted_cr_retains_tex82_delimiter_identity_for_observation() {
        let mut command = CommandState::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let alignment = crate::AlignmentIdentity::new(18);
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(alignment, templates())
            .expect("cell begins");
        command
            .install_alignment_cell_template(&universe.command_context(), alignment)
            .expect("cell without a u-template installs");
        let cr = universe.intern("cr").symbol();
        universe.set_meaning(
            cr,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Cs(cr),
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        let delimiter = processor
            .get_next()
            .expect("cr delivers")
            .expect("input is live");
        assert!(matches!(
            delimiter.meaning(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
        ));
        assert!(recorder.0.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Alignment(record)
                    if record.transition == "delimiter"
                        && record.align_state == CELL_ALIGN_STATE
                        && record.delimiter == Some("cr")
            )
        }));
    }

    #[test]
    fn alignment_delimiter_waits_for_literal_brace_depth() {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(29);
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(alignment, templates())
            .expect("cell begins");
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                TracedTokenWord::pack(
                    Token::Char {
                        ch: '{',
                        cat: Catcode::BeginGroup,
                    },
                    OriginId::UNKNOWN,
                ),
                TracedTokenWord::pack(
                    Token::Char {
                        ch: '&',
                        cat: Catcode::AlignmentTab,
                    },
                    OriginId::UNKNOWN,
                ),
                TracedTokenWord::pack(
                    Token::Char {
                        ch: '}',
                        cat: Catcode::EndGroup,
                    },
                    OriginId::UNKNOWN,
                ),
                TracedTokenWord::pack(
                    Token::Char {
                        ch: '&',
                        cat: Catcode::AlignmentTab,
                    },
                    OriginId::UNKNOWN,
                ),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        processor.get_next().expect("opening brace delivers");
        let nested_tab = processor
            .get_next()
            .expect("nested tab delivers")
            .expect("input is live");
        assert!(matches!(
            nested_tab.meaning(),
            Meaning::CharToken {
                cat: Catcode::AlignmentTab,
                ..
            }
        ));
        processor.get_next().expect("closing brace delivers");
        assert!(matches!(
            processor
                .get_x_alignment_delivery(false)
                .expect("top-level tab delivers")
                .expect("input is live"),
            crate::AlignmentDelivery::Event(crate::AlignmentDeliveryEvent::EndTemplate(_))
        ));
    }

    #[test]
    fn fin_col_endv_stack_accepts_exhausted_frames_and_rejects_interwoven_states() {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(71);
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Char {
                    ch: '&',
                    cat: Catcode::AlignmentTab,
                },
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut universe = Universe::new_with_plain_catcodes();
        let u_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list_ref(&[
                Token::Char {
                    ch: 'u',
                    cat: Catcode::Letter,
                },
            ]));
        let v_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list_ref(&[
                Token::Char {
                    ch: 'v',
                    cat: Catcode::Letter,
                },
            ]));
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(
                alignment,
                crate::AlignmentCellTemplates {
                    u_template: Some(u_template),
                    v_template,
                },
            )
            .expect("cell begins");
        command
            .install_alignment_cell_template(&universe.command_context(), alignment)
            .expect("u-template installs after the cell opener lifecycle");
        let snapshot = command.snapshot();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);

            let u = processor
                .get_next()
                .expect("u-template delivers")
                .expect("u-template token");
            assert!(matches!(u.meaning(), Meaning::CharToken { ch: 'u', .. }));
            let end_template = match processor
                .get_x_alignment_delivery(false)
                .expect("delimiter follows exhausted u-template")
                .expect("intercepted delimiter")
            {
                crate::AlignmentDelivery::Event(crate::AlignmentDeliveryEvent::EndTemplate(
                    event,
                )) => crate::AlignmentDeliveryEvent::EndTemplate(event),
                crate::AlignmentDelivery::Command(_) => {
                    panic!("delimiter is delivered as an alignment event")
                }
                crate::AlignmentDelivery::Event(crate::AlignmentDeliveryEvent::ClosingBrace(_)) => {
                    panic!("base-depth delimiter is not an align-group closing brace")
                }
                crate::AlignmentDelivery::Completed(_) => {
                    panic!("no executor-owned replay episode is active in this fixture")
                }
            };
            processor
                .begin_alignment_v_template(alignment, end_template)
                .expect("delimiter is saved for fin_col below v-template input");
            let v = processor
                .get_next()
                .expect("v-template delivers")
                .expect("v-template token");
            assert!(matches!(v.meaning(), Meaning::CharToken { ch: 'v', .. }));
            assert_eq!(
                processor.finish_alignment_cell(alignment),
                Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                    "(interwoven alignment preambles are not allowed)",
                ))),
                "a nonempty v-template has not reached do_endv's loc=null boundary"
            );
            let endv = processor
                .get_x_token()
                .expect("end-template expands to end-v")
                .expect("end-v delivery");
            assert!(matches!(endv.meaning(), Meaning::EndV));
            assert!(endv.spelling().semantic_token().is_frozen_endv());
            processor.command.alignment.align_state = 499_999;
            assert_eq!(
                processor.finish_alignment_cell(alignment),
                Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                    "(interwoven alignment preambles are not allowed)",
                ))),
                "fin_col rejects an interwoven brace sentinel even with the exhausted v-template present"
            );
            processor.command.alignment.align_state = 1_000_000;
            processor.command.push_token_level(
                TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                    Token::Char {
                        ch: '!',
                        cat: Catcode::Other,
                    },
                    OriginId::UNKNOWN,
                )])),
                TokenBehavior::Ordinary,
                RetirementBehavior::Pop,
                ReplayTrace::BackedUp,
            );
            assert_eq!(
                processor.finish_alignment_cell(alignment),
                Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                    "(interwoven alignment preambles are not allowed)",
                ))),
                "a nonempty token-list interwoven above the exhausted v-template is fatal"
            );
            processor.command.input.levels.pop();
            let finished = processor
                .finish_alignment_cell(alignment)
                .expect("do_endv proves the exact retained frame");
            assert_eq!(finished.delimiter, crate::AlignmentCellDelimiter::Tab);
            // tex.web §1131 pops nothing, so the frame is still on the stack
            // here; §357's `end_token_list` retires it in the next fetch.
            assert!(
                processor
                    .get_next()
                    .expect("saved delimiter does not re-enter raw delivery")
                    .is_none()
            );
        }
        let end_template = recorder
            .0
            .iter()
            .rposition(|observation| {
                matches!(
                    observation,
                    CommandObservation::Command(delivery)
                        if delivery.boundary == CommandDeliveryBoundary::Raw
                            && delivery.command == "end_template"
                )
            })
            .expect("frozen end-template is observed as raw delivery");
        let endv = recorder
            .0
            .iter()
            .rposition(|observation| {
                matches!(
                    observation,
                    CommandObservation::Command(delivery)
                        if delivery.boundary == CommandDeliveryBoundary::Expanded
                            && delivery.command == "endv"
                )
            })
            .expect("end-v is observed as expanded delivery");
        assert!(end_template < endv);
        let endv_index = endv;
        let CommandObservation::Command(end_template) = &recorder.0[end_template] else {
            unreachable!("filtered to raw command delivery")
        };
        let CommandObservation::Command(endv) = &recorder.0[endv] else {
            unreachable!("filtered to expanded command delivery")
        };
        assert!(matches!(
            end_template.spelling,
            crate::observation::ObservedToken::ControlSequence(ref name) if name == "endtemplate"
        ));
        assert!(matches!(
            endv.spelling,
            crate::observation::ObservedToken::ControlSequence(ref name) if name == "endtemplate"
        ));
        // tex.web §1131's `do_endv` only inspects the input stack; §357's
        // `end_token_list` pops the depleted v-template on the next fetch,
        // and that is where it is observed -- after `endv`, never before it.
        let v_retirement = recorder
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Input(input)
                        if input.transition == InputTransition::Retire
                            && input.reason == InputReason::AlignmentVTemplate
                )
            })
            .expect("the depleted v-template retires through get_next");
        assert!(endv_index < v_retirement);
        assert!(matches!(
            &recorder.0[v_retirement + 1],
            CommandObservation::Alignment(record) if record.transition == "v_template_retire"
        ));
        // §380's `get_x_token` disposes of `end_template` itself, so nothing
        // is backed up and there is no raw `endv` delivery at all.
        assert!(!recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Input(input)
                if input.transition == InputTransition::Backup
        )));
        assert!(!recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Command(delivery)
                if delivery.boundary == CommandDeliveryBoundary::Raw
                    && delivery.command == "endv"
        )));
        command
            .rollback(snapshot.clone())
            .expect("template input rolls back exactly");
        assert_eq!(command.snapshot(), snapshot);
    }

    /// TeX82 §380's two expanded fetches disagree about `end_template`, and
    /// §1038's `x_token` takes the longer road: §366 `expand` reaches §375,
    /// which backs up a `frozen_endv` token for `x_token`'s own `get_next` to
    /// reread. The sibling test above pins the `get_x_token` form, which
    /// rewrites the live command instead and observes none of this.
    #[test]
    fn x_token_end_template_backs_up_frozen_endv_and_rereads_it() {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(71);
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Char {
                    ch: '&',
                    cat: Catcode::AlignmentTab,
                },
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut universe = Universe::new_with_plain_catcodes();
        let u_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list_ref(&[
                Token::Char {
                    ch: 'u',
                    cat: Catcode::Letter,
                },
            ]));
        let v_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list_ref(&[
                Token::Char {
                    ch: 'v',
                    cat: Catcode::Letter,
                },
            ]));
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(
                alignment,
                crate::AlignmentCellTemplates {
                    u_template: Some(u_template),
                    v_template,
                },
            )
            .expect("cell begins");
        command
            .install_alignment_cell_template(&universe.command_context(), alignment)
            .expect("u-template installs after the cell opener lifecycle");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            processor.get_next().expect("u-template delivers");
            let delimiter = match processor
                .get_x_alignment_delivery(false)
                .expect("delimiter follows exhausted u-template")
                .expect("intercepted delimiter")
            {
                crate::AlignmentDelivery::Event(
                    event @ crate::AlignmentDeliveryEvent::EndTemplate(_),
                ) => event,
                _ => panic!("delimiter is delivered as an alignment event"),
            };
            processor
                .begin_alignment_v_template(alignment, delimiter)
                .expect("delimiter is saved for fin_col below v-template input");
            processor.get_next().expect("v-template delivers");
            // §1038 is parked in the character loop, so this is `x_token`.
            let endv = match processor
                .get_x_alignment_delivery(true)
                .expect("exhausted v-template ends the cell")
                .expect("end-v delivery")
            {
                crate::AlignmentDelivery::Command(command) => command,
                _ => panic!("end-v is an ordinary command delivery"),
            };
            assert!(matches!(endv.meaning(), Meaning::EndV));
            assert!(endv.spelling().semantic_token().is_frozen_endv());
            let finished = processor
                .command
                .finish_alignment_cell(alignment)
                .expect("do_endv retires the backup and the retained frame");
            assert_eq!(finished.delimiter, crate::AlignmentCellDelimiter::Tab);
        }
        let shape: Vec<&str> = recorder
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Command(delivery)
                    if delivery.command == "end_template"
                        && delivery.boundary == CommandDeliveryBoundary::Raw =>
                {
                    Some("raw end_template")
                }
                CommandObservation::Command(delivery) if delivery.command == "endv" => {
                    Some(match delivery.boundary {
                        CommandDeliveryBoundary::Raw => "raw endv",
                        CommandDeliveryBoundary::Expanded => "expanded endv",
                    })
                }
                CommandObservation::Input(input) if input.transition == InputTransition::Backup => {
                    Some("push backup")
                }
                CommandObservation::Recovery(recovery)
                    if recovery.kind == crate::observation::RecoveryKind::Backup
                        && recovery.tokens.len() == 1
                        && matches!(
                            recovery.tokens[0],
                            crate::observation::ObservedToken::FrozenEndV
                        ) =>
                {
                    Some("backup frozen_endv")
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            shape,
            [
                "raw end_template",
                "push backup",
                "backup frozen_endv",
                "raw endv",
                "expanded endv",
            ]
        );
    }

    #[test]
    fn stale_backup_cannot_repeat_literal_brace_rollback() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"{x".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let opening = processor
            .get_next()
            .expect("opening brace delivers")
            .expect("source is live");
        processor
            .get_token()
            .expect("later token delivers")
            .expect("source is live");
        assert_eq!(
            processor.back_input(opening),
            Err(CommandError::StaleDelivery)
        );
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE + 1
        );
    }

    #[test]
    fn noexpand_treatment_and_back_error_are_one_delivery_accounting() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"\\target x".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let target = universe.intern("target").symbol();
        universe.set_meaning(
            target,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let target = processor
            .get_next()
            .expect("target delivers")
            .expect("source is live");
        processor
            .back_input_with_treatment(target, BackupTreatment::SuppressExpandableControlSequence)
            .expect("target backs up for noexpand");
        let suppressed = processor
            .get_next()
            .expect("suppressed target delivers")
            .expect("backup is live");
        assert_eq!(suppressed.meaning(), Meaning::Relax);

        let letter = processor
            .get_next()
            .expect("source resumes")
            .expect("letter delivers");
        processor
            .back_error(letter, 41)
            .expect("back error backs up");
        assert_eq!(processor.command.expansion.pending_diagnostics, vec![41]);
        assert_eq!(
            processor
                .get_next()
                .expect("backed-up recovery input delivers")
                .expect("backup is live")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }
        );
    }

    #[test]
    fn noexpand_treatment_survives_rewindable_token_input() {
        let mut command = CommandState::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let target = universe.intern("target").symbol();
        universe.set_meaning(
            target,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Cs(target),
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::MacroReplacement,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        let target = processor
            .get_next()
            .expect("token-list target delivers")
            .expect("token-list input is live");
        processor
            .back_input_with_treatment(target, BackupTreatment::SuppressExpandableControlSequence)
            .expect("noexpand backs up the exact token-list delivery");
        // §325's stack-conservation loop retires the depleted token list
        // before the `backed_up` level, independently of its treatment.
        assert_eq!(processor.command.input.levels.len(), 1);
        assert_eq!(
            processor
                .get_next()
                .expect("suppressed token-list target delivers")
                .expect("backed-up input is live")
                .meaning(),
            Meaning::Relax
        );
    }

    fn recovery_primitives(universe: &mut Universe) {
        universe.register_primitive_meaning(
            "fi",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        universe.register_primitive_meaning(
            "par",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
        );
        universe.register_primitive_meaning(
            "cr",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
        );
    }

    #[test]
    fn runaway_recovery_inserts_the_status_specific_canonical_tokens() {
        let mut command = CommandState::default();
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let mut capabilities = CommandHostCapabilities::default();
        let warning = ScannerWarning(17);

        let cases = [
            (
                ScannerStatus::Skipping(SkippingContext {
                    condition: ConditionId(1),
                    warning,
                    skip_line: 0,
                    conditional: crate::conditionals::ConditionalKind::IfTrue,
                }),
                vec![universe.primitive_token("fi").expect("fi is registered")],
                None,
            ),
            (
                ScannerStatus::Defining(DefinitionContext {
                    target: None,
                    builder: TokenBuilderId(2),
                    warning,
                }),
                vec![Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                }],
                Some("Runaway definition?"),
            ),
            (
                ScannerStatus::Matching(MatchingContext {
                    macro_name: universe.intern("argument").symbol(),
                    builder: ArgumentBuilderId(3),
                    warning,
                }),
                vec![universe.primitive_token("par").expect("par is registered")],
                Some("Runaway argument?"),
            ),
            (
                ScannerStatus::Aligning(AlignmentScanContext {
                    alignment: AlignmentId(4),
                    builder: TokenBuilderId(5),
                    owner: None,
                    warning,
                }),
                vec![
                    universe.primitive_token("cr").expect("cr is registered"),
                    Token::Char {
                        ch: '}',
                        cat: Catcode::EndGroup,
                    },
                ],
                Some("Runaway preamble?"),
            ),
            (
                ScannerStatus::Absorbing(AbsorbingContext {
                    owner: None,
                    builder: TokenBuilderId(6),
                    warning,
                }),
                vec![Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                }],
                Some("Runaway text?"),
            ),
        ];

        for (status, expected, expected_heading) in cases {
            let actual = command.with_scanner_status(status, |command| {
                let mut processor = processor(command, &mut universe, &mut capabilities);
                let mut delivered = Vec::new();
                for _ in 0..expected.len() {
                    delivered.push(
                        processor
                            .get_next()
                            .expect("recovery delivers")
                            .expect("inserted input is live")
                            .spelling()
                            .semantic_token(),
                    );
                }
                assert!(
                    processor
                        .get_next()
                        .expect("terminal input is legal")
                        .is_none()
                );
                delivered
            });
            assert_eq!(actual, expected);
            let diagnostics = command.take_semantic_diagnostics();
            match expected_heading {
                Some(expected_heading) => {
                    let [
                        crate::CommandSemanticDiagnostic::Recoverable {
                            runaway: Some(runaway),
                            help,
                            ..
                        },
                    ] = diagnostics.as_slice()
                    else {
                        panic!("expected one runaway diagnostic: {diagnostics:?}");
                    };
                    assert_eq!(runaway.heading, expected_heading);
                    assert_eq!(runaway.partial, "");
                    assert_eq!(
                        *help,
                        [
                            "I suspect you have forgotten a `}', causing me",
                            "to read past where you wanted me to stop.",
                            "I'll try to recover; but if the error is serious,",
                            "you'd better type `E' or `X' now and fix your file.",
                        ]
                    );
                }
                None => assert!(matches!(
                    diagnostics.as_slice(),
                    [crate::CommandSemanticDiagnostic::Recoverable { runaway: None, .. }]
                )),
            }
        }
        assert_eq!(command.expansion.pending_diagnostics, vec![17; 5]);
    }

    #[test]
    fn runaway_partial_update_keeps_an_earlier_deferred_report_immutable() {
        // TeX82 §306 pseudoprints the token list owned by the current scanner
        // episode. Deferred executor output may leave a completed §396 report
        // ahead of it, but that older report is not the current scanner's
        // list and must not acquire the later partial.
        let mut command = CommandState::default();
        command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: crate::macro_call::RUNAWAY_ARGUMENT_DIAGNOSTIC,
                runaway: Some(crate::state::RunawayPrelude {
                    heading: "Runaway argument?",
                    partial: String::new(),
                }),
                message: "older argument report".into(),
                help: &[],
                context: String::new(),
            });
        command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: RUNAWAY_SCAN_DIAGNOSTIC,
                runaway: Some(crate::state::RunawayPrelude {
                    heading: "Runaway argument?",
                    partial: String::new(),
                }),
                message: "current scanner report".into(),
                help: &[],
                context: String::new(),
            });
        let mut universe = Universe::new_with_plain_catcodes();
        let caution = universe.intern("caution").symbol();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        processor.set_runaway_partial(
            RUNAWAY_SCAN_DIAGNOSTIC,
            &[
                TracedTokenWord::pack(Token::Cs(caution), OriginId::UNKNOWN),
                TracedTokenWord::pack(
                    Token::Char {
                        ch: 'x',
                        cat: Catcode::Letter,
                    },
                    OriginId::UNKNOWN,
                ),
            ],
        );

        let diagnostics = processor.take_semantic_diagnostics();
        let partials = diagnostics
            .iter()
            .map(|diagnostic| match diagnostic {
                crate::CommandSemanticDiagnostic::Recoverable {
                    runaway: Some(runaway),
                    ..
                } => runaway.partial.as_str(),
                _ => panic!("expected runaway reports"),
            })
            .collect::<Vec<_>>();
        assert_eq!(partials, ["", "\\caution x"]);
    }

    #[test]
    fn aligning_eof_recovery_preserves_frozen_cr_identity_as_an_inserted_control_sequence() {
        let mut command = CommandState::default();
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let frozen_cr = universe.primitive_token("cr").expect("cr is registered");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();

        command.begin_alignment(crate::AlignmentIdentity::new(4));
        command
            .transient
            .builders
            .push(crate::state::LiveTokenBuilder {
                identity: 5,
                tokens: tex_state::token::RootedTracedTokenBuffer::new([
                    tex_state::token::RootedTracedTokenWord::unowned(TracedTokenWord::pack(
                        Token::Char {
                            ch: '{',
                            cat: Catcode::BeginGroup,
                        },
                        OriginId::UNKNOWN,
                    )),
                ]),
            });
        command.with_scanner_status(
            ScannerStatus::Aligning(AlignmentScanContext {
                alignment: AlignmentId(4),
                builder: TokenBuilderId(5),
                owner: None,
                warning: ScannerWarning(17),
            }),
            |command| {
                let mut processor =
                    processor(command, &mut universe, &mut capabilities)
                        .with_observer(&mut recorder);
                assert_eq!(
                    processor
                        .get_next()
                        .expect("EOF recovery succeeds")
                        .expect("frozen cr is inserted")
                        .spelling()
                        .semantic_token(),
                    frozen_cr,
                );
                assert!(
                    matches!(processor.command.scanner.status(), ScannerStatus::Aligning(_)),
                    "TeX82 §23 keeps the aligning episode live until the recovered \\cr completes the preamble"
                );
            },
        );

        assert!(
            recorder.0.iter().any(|record| {
                matches!(record, CommandObservation::Alignment(alignment)
                    if alignment.transition == "outer_validity")
            }),
            "outer-validity alignment recovery is observed: {:?}",
            recorder.0
        );
        let outer_validity = recorder
            .0
            .iter()
            .position(|record| {
                matches!(record, CommandObservation::Alignment(alignment)
                if alignment.transition == "outer_validity")
            })
            .expect("outer-validity recovery is observed");
        let frozen_cr_recovery = recorder
            .0
            .iter()
            .position(|record| {
                matches!(record, CommandObservation::Recovery(recovery)
                if recovery.kind == RecoveryKind::InsertedControlSequence
                    && recovery.tokens
                        == vec![
                            crate::observation::ObservedToken::ControlSequence("cr".into()),
                        ])
            })
            .expect("frozen cr recovery is observed");
        assert!(
            outer_validity < frozen_cr_recovery,
            "TeX82 §23 observes outer-validity recovery before frozen \\cr insertion"
        );
        let diagnostics = command.take_semantic_diagnostics();
        let [
            crate::CommandSemanticDiagnostic::Recoverable {
                runaway: Some(runaway),
                ..
            },
        ] = diagnostics.as_slice()
        else {
            panic!("one alignment runaway diagnostic: {diagnostics:?}");
        };
        assert_eq!(runaway.partial, "{");
    }

    #[test]
    fn skipping_eof_reports_diagnostics_before_frozen_fi_recovery() {
        let mut command = CommandState::default();
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let frozen_fi = universe.primitive_token("fi").expect("fi is registered");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();

        command.with_scanner_status(
            ScannerStatus::Skipping(SkippingContext {
                condition: ConditionId(1),
                warning: ScannerWarning(17),
                skip_line: 0,
                conditional: crate::conditionals::ConditionalKind::IfTrue,
            }),
            |command| {
                let mut processor = processor(command, &mut universe, &mut capabilities)
                    .with_observer(&mut recorder);
                let recovered = processor
                    .get_next()
                    .expect("EOF recovery succeeds")
                    .expect("frozen fi is inserted");
                assert_eq!(recovered.spelling().semantic_token(), frozen_fi);
            },
        );

        let diagnostic_positions = recorder
            .0
            .iter()
            .enumerate()
            .filter_map(|(index, record)| match record {
                CommandObservation::Diagnostic(diagnostic) => Some((index, diagnostic)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostic_positions.len(), 2);
        assert_eq!(diagnostic_positions[0].1.diagnostic, "outer_validity_eof");
        assert_eq!(
            diagnostic_positions[0].1.arguments,
            vec![crate::observation::DiagnosticArgument::Name(
                "skipping".into()
            )]
        );
        assert_eq!(
            diagnostic_positions[1].1.diagnostic,
            "conditional_incomplete"
        );
        let recovery = recorder
            .0
            .iter()
            .position(|record| {
                matches!(record, CommandObservation::Recovery(record)
                    if record.kind == RecoveryKind::InsertedToken)
            })
            .expect("frozen fi recovery is observed");
        assert!(
            diagnostic_positions[1].0 < recovery,
            "§§379/510 diagnose skipped EOF before inserting frozen fi"
        );
    }

    #[test]
    fn scantokens_eof_recovers_scanner_episode_that_predates_the_pseudo_source() {
        // e-TeX 2.6 etex.ch §53a opens `\scantokens` as a real source level,
        // and TeX82 §343 unconditionally runs `check_outer_validity` after
        // `end_file_reading`. The defining episode can therefore predate the
        // pseudo-source open; its unchanged identity does not make EOF legal.
        let mut command = CommandState::new(crate::CommandProfile::ETEX26);
        command.begin_scanner_status(ScannerStatus::Defining(DefinitionContext {
            target: None,
            builder: TokenBuilderId(2),
            warning: ScannerWarning(17),
        }));
        command
            .open_scantokens(
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&b""[..]),
                ),
                None,
                18,
            )
            .expect("empty scantokens pseudo-source opens during definition scan");

        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let recovered = processor(&mut command, &mut universe, &mut capabilities)
            .with_observer(&mut recorder)
            .get_next()
            .expect("pseudo-source EOF recovery succeeds")
            .expect("definition recovery inserts a right brace");
        assert_eq!(
            recovered.spelling().semantic_token(),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }
        );
        assert!(matches!(command.scanner.status(), ScannerStatus::Normal));

        let retirement = recorder
            .0
            .iter()
            .position(|record| {
                matches!(record, CommandObservation::Input(input)
                    if input.transition == InputTransition::Retire
                        && input.source_name == Some(SourceNameClass::Scantokens(18)))
            })
            .expect("scantokens source retirement is observed");
        let diagnostic = recorder
            .0
            .iter()
            .position(|record| {
                matches!(record, CommandObservation::Diagnostic(diagnostic)
                    if diagnostic.diagnostic == "outer_validity_eof")
            })
            .expect("definition EOF outer-validity diagnostic is observed");
        assert!(
            retirement < diagnostic,
            "TeX82 §343 retires the source before checking outer validity"
        );
    }

    #[test]
    fn nested_source_eof_recovers_skipping_before_parent_input_resumes() {
        let mut command = CommandState::default();
        let parent = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"z".as_slice()),
            ))
            .expect("parent source registers");
        let child = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"".as_slice()),
            ))
            .expect("child source registers");
        command
            .open_registered_source(parent)
            .expect("parent source opens");
        command
            .open_registered_source(child)
            .expect("nested source opens above parent");
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let frozen_fi = universe.primitive_token("fi").expect("fi is registered");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();

        command.with_scanner_status(
            ScannerStatus::Skipping(SkippingContext {
                condition: ConditionId(1),
                warning: ScannerWarning(17),
                skip_line: 0,
                conditional: crate::conditionals::ConditionalKind::IfTrue,
            }),
            |command| {
                let mut processor = processor(command, &mut universe, &mut capabilities)
                    .with_observer(&mut recorder);
                let recovered = processor
                    .get_next()
                    .expect("nested EOF recovery succeeds")
                    .expect("frozen fi is inserted above the parent source");
                assert_eq!(recovered.spelling().semantic_token(), frozen_fi);
                let parent_token = processor
                    .get_next()
                    .expect("recovery retires before parent resumes")
                    .expect("parent source remains live after recovery");
                assert!(matches!(
                    parent_token.meaning(),
                    Meaning::CharToken { ch: 'z', .. }
                ));
            },
        );

        let diagnostics = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::Diagnostic(diagnostic) => Some(diagnostic.diagnostic),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            diagnostics,
            vec!["outer_validity_eof", "conditional_incomplete"],
            "TeX82 §§379/510 recover the exhausted nested source before parent input"
        );
    }

    #[test]
    fn outer_macro_is_backed_up_and_current_delivery_becomes_a_space() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"\\outer".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let outer = universe.intern("outer").symbol();
        let frozen_par = universe.primitive_token("par").expect("par is registered");
        let parameters = universe.intern_token_list(&[]);
        let replacement = universe.intern_token_list(&[]);
        let definition = universe.intern_macro(MacroMeaning::new(
            MeaningFlags::OUTER,
            parameters,
            replacement,
        ));
        universe.set_meaning(
            outer,
            Meaning::Macro {
                flags: MeaningFlags::OUTER,
                definition: definition.id(),
            },
        );
        let mut capabilities = CommandHostCapabilities::default();

        command.with_scanner_status(
            ScannerStatus::Matching(MatchingContext {
                macro_name: outer,
                builder: ArgumentBuilderId(1),
                warning: ScannerWarning(23),
            }),
            |command| {
                let mut processor = processor(command, &mut universe, &mut capabilities);
                let recovered = processor
                    .get_next()
                    .expect("outer delivery succeeds")
                    .expect("outer is delivered");
                assert_eq!(
                    recovered.meaning(),
                    Meaning::CharToken {
                        ch: ' ',
                        cat: Catcode::Space
                    }
                );
                assert_eq!(
                    processor
                        .get_next()
                        .expect("recovery par delivers")
                        .expect("recovery is live")
                        .spelling()
                        .semantic_token(),
                    frozen_par,
                );
                assert_eq!(
                    processor
                        .get_next()
                        .expect("outer replay delivers")
                        .expect("backup is live")
                        .meaning(),
                    Meaning::Macro {
                        flags: MeaningFlags::OUTER,
                        definition: definition.id(),
                    },
                );
            },
        );
    }

    #[test]
    fn outer_recovery_is_bounded_and_snapshot_rollback_replays_the_episode() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"\\outer".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let outer = universe.intern("outer").symbol();
        let parameters = universe.intern_token_list(&[]);
        let replacement = universe.intern_token_list(&[]);
        let definition = universe.intern_macro(MacroMeaning::new(
            MeaningFlags::OUTER,
            parameters,
            replacement,
        ));
        universe.set_meaning(
            outer,
            Meaning::Macro {
                flags: MeaningFlags::OUTER,
                definition: definition.id(),
            },
        );
        let mut capabilities = CommandHostCapabilities::default();
        let matching = ScannerStatus::Matching(MatchingContext {
            macro_name: outer,
            builder: ArgumentBuilderId(1),
            warning: ScannerWarning(29),
        });

        let snapshot = command.with_scanner_status(matching.clone(), |command| {
            let snapshot = command.snapshot();
            let mut processor = processor(command, &mut universe, &mut capabilities);
            assert_eq!(
                processor
                    .get_next()
                    .expect("outer recovery succeeds")
                    .expect("recovery substitutes a space")
                    .meaning(),
                Meaning::CharToken {
                    ch: ' ',
                    cat: Catcode::Space,
                }
            );
            assert!(
                processor
                    .get_next()
                    .expect("one recovery token delivers")
                    .is_some()
            );
            assert!(
                processor
                    .get_next()
                    .expect("the backed-up outer token delivers")
                    .is_some()
            );
            assert!(
                processor
                    .get_next()
                    .expect("recovery input is bounded")
                    .is_none()
            );
            snapshot
        });

        command
            .rollback(snapshot)
            .expect("snapshot restores the live scanner episode and input cursor");
        assert_eq!(command.scanner.status(), &matching);
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        assert_eq!(
            processor
                .get_next()
                .expect("rolled-back outer recovery succeeds")
                .expect("recovery substitutes a space again")
                .meaning(),
            Meaning::CharToken {
                ch: ' ',
                cat: Catcode::Space,
            }
        );
    }
}
