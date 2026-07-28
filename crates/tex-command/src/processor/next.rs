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
    OutParameterReplay, ReplayTrace, RetirementBehavior, SharedBackedUpBuffer, SharedTokenBuffer,
    TokenBehavior, TokenCursor, TokenPayload,
};
use crate::profile::{CharacterCode, CharacterMode};
use crate::{
    AlignmentDelivery, AlignmentDeliveryEvent, CommandReplayDelivery, SourceControlSequenceKind,
    SourceProvenance, SourceToken, SourceTokenizationStep,
};

use super::CommandProcessor;
use super::expand::ExpandedFetch;
use super::status::{EofLegality, RecoveryContext, ScannerStatus};

use super::alignment::AlignmentDeliveryState;
#[cfg(any(test, feature = "instrumentation"))]
use super::alignment::CELL_ALIGN_STATE;

#[cfg(any(test, feature = "instrumentation"))]
use crate::input::InputRetirementReason;
#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::{
    AlignmentRecord, CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation,
    CommandProvenance, DiagnosticArgument, DiagnosticRecord, InputReason, InputRecord,
    InputTransition, RecoveryKind, RecoveryRecord, observed_token,
};

impl CommandProcessor<'_> {
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
    pub fn final_cleanup(&mut self) {
        let mut terminal_stopped = false;
        while let Some(retirement) = self.command.pop_input_level_at_end_of_job() {
            let terminal = matches!(retirement.action, InputRetirementAction::TerminalStop);
            terminal_stopped |= terminal;
            #[cfg(any(test, feature = "instrumentation"))]
            self.observe(CommandObservation::Input(InputRecord {
                transition: if terminal {
                    InputTransition::Stop
                } else {
                    InputTransition::Retire
                },
                reason: observed_retirement_reason(retirement.action, retirement.reason),
                level: retirement.identity.0,
                position: 0,
            }));
        }
        #[cfg(any(test, feature = "instrumentation"))]
        if !terminal_stopped {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Stop,
                reason: InputReason::Source,
                level: 0,
                position: 0,
            }));
        }
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = terminal_stopped;
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
            RetirementRestart::Continue => Ok(()),
            RetirementRestart::Stop | RetirementRestart::EndV(_) | RetirementRestart::Completed => {
                Err(CommandError::input_invariant())
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
    /// it can report `ScannedStep::ReplayCompleted` exactly as ordinary
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
        let mut first = true;
        loop {
            let Some(mut command) = (match self.get_next_with_replay_completion()? {
                Some(CommandReplayDelivery::Command(command)) => Some(command),
                Some(CommandReplayDelivery::Completed(episode)) => {
                    return Ok(Some(AlignmentDelivery::Completed(episode)));
                }
                None => None,
            }) else {
                return Ok(None);
            };
            // §1038 short-circuits before every other test in this loop: it
            // reads only `cur_cmd`/`cur_chr` and jumps straight back into the
            // character loop. Neither alignment recovery predicate below can
            // fire for these three commands, and none of them is expandable.
            if std::mem::take(&mut first)
                && main_loop_active
                && crate::processor::expand::is_main_loop_character(command.meaning())
            {
                return Ok(Some(AlignmentDelivery::Command(command)));
            }
            if matches!(
                command.meaning(),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
            ) && !command.spelling().semantic_token().is_frozen_end_template()
            {
                return Ok(Some(AlignmentDelivery::Event(
                    AlignmentDeliveryEvent::EndTemplate(command),
                )));
            }
            if matches!(
                command.meaning(),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
            ) {
                if fetch == ExpandedFetch::XToken {
                    // §366 `expand` has no `end_template` shortcut: it routes
                    // straight to §375, which backs up a `frozen_endv` token
                    // for this loop's own `get_next` to reread.
                    self.insert_frozen_endv()?;
                    continue;
                }
                command.convert_end_template_to_endv(self.state.frozen_endv_token());
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe_expanded_delivery(&command);
                return Ok(Some(AlignmentDelivery::Command(command)));
            }
            if matches!(
                command.meaning(),
                Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_)
            ) {
                self.expand(command)?;
                continue;
            }
            #[cfg(any(test, feature = "instrumentation"))]
            self.observe_expanded_delivery(&command);
            if self
                .command
                .alignment
                .needs_closing_brace_recovery(&command)
            {
                return Ok(Some(AlignmentDelivery::Event(
                    AlignmentDeliveryEvent::ClosingBrace(command),
                )));
            }
            return Ok(Some(AlignmentDelivery::Command(command)));
        }
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
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                recovery,
                OriginId::UNKNOWN,
            )])),
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
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                frozen_cr,
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
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
            .begin_alignment_v_template(alignment, saved_delimiter)
            .map_err(|_| CommandError::input_invariant())?;
        #[cfg(any(test, feature = "instrumentation"))]
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
        self.last_delivery = None;
        loop {
            match self.get_next_with_control_sequence_creation(false)? {
                Some(CommandReplayDelivery::Command(command)) => {
                    match self.insert_alignment_entry_v_template(command)? {
                        Some(command) => return Ok(Some(command)),
                        None => continue,
                    }
                }
                Some(CommandReplayDelivery::Completed(_)) => continue,
                None => return Ok(None),
            }
        }
    }

    /// Delivers one raw command or an executor-owned stored-episode
    /// completion. This is the raw counterpart of
    /// [`Self::get_x_token_with_replay_completion`].
    pub fn get_next_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery>, CommandError> {
        self.last_delivery = None;
        self.get_next_with_control_sequence_creation(false)
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
        self.get_token()
    }

    /// Delivers one raw token for consumers which canonically permit a new
    /// control-sequence spelling. The present interner records a spelling
    /// without assigning it a meaning, so the policy boundary is explicit
    /// even before diagnostic-only interning is separated further.
    pub fn get_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.last_delivery = None;
        loop {
            match self.get_next_with_control_sequence_creation(true)? {
                Some(CommandReplayDelivery::Command(command)) => {
                    match self.insert_alignment_entry_v_template(command)? {
                        Some(command) => return Ok(Some(command)),
                        None => continue,
                    }
                }
                Some(CommandReplayDelivery::Completed(_)) => continue,
                None => return Ok(None),
            }
        }
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
    pub(crate) fn back_list(&mut self, tokens: Vec<BackedUpToken>) {
        debug_assert!(
            !tokens.is_empty(),
            "TeX82 §407 guards back_list with `p<>backup_head`"
        );
        let level = self.command.push_token_level(
            TokenPayload::BackedUp(SharedBackedUpBuffer::new(tokens)),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Backup,
            reason: InputReason::Backup,
            level: level.0,
            position: 0,
        }));
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
        self.conserve_input_stack()?;
        self.command
            .alignment
            .undo_delivery(AlignmentDeliveryState::back_input_adjustment(
                spelling.semantic_token(),
            ));
        let level = self.command.push_token_level(
            TokenPayload::BackedUp(SharedBackedUpBuffer::new(vec![BackedUpToken {
                spelling,
                source_provenance: None,
            }])),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Backup,
                reason: InputReason::Backup,
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
            TokenPayload::Transient(SharedTokenBuffer::new(vec![par])),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        {
            // `head_for_vmode` calls `back_input` after assigning `cur_tok`;
            // the push is therefore observed as backup even though its
            // inserted ownership makes retirement a recovery transition.
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Backup,
                reason: InputReason::Backup,
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
        let dollar = TracedTokenWord::pack(
            Token::Char {
                ch: '$',
                cat: Catcode::MathShift,
            },
            OriginId::UNKNOWN,
        );
        let level = self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![dollar])),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        {
            // `insert_dollar_sign` calls `back_input` before assigning
            // `cur_tok`; the push is therefore observed as backup even
            // though its inserted ownership makes retirement a recovery
            // transition, mirroring `recover_stop_for_vertical_mode` above.
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Backup,
                reason: InputReason::Backup,
                level: level.0,
                position: 0,
            }));
            self.observe(CommandObservation::Recovery(RecoveryRecord {
                kind: RecoveryKind::Backup,
                tokens: vec![self.observed_token(dollar)],
            }));
        }
        Ok(())
    }

    /// Starts TeX82 §1025's already-selected output token list.
    ///
    /// Page selection and `\box255` packing belong to the stomach (§1012's
    /// `fire_up`), but the resulting token-list ownership never leaves
    /// command control.  This is the *only* way `\output` is ever entered:
    /// §1054's `its_all_over` never starts it directly, it only appends the
    /// end-job contribution trio and lets §994's `build_page` decide.
    pub fn begin_selected_output_routine(&mut self) -> Result<(), CommandError> {
        let output = TracedTokenList::synthetic(self.state.tok_param(TokParam::OUTPUT));
        let level = self.command.push_token_level(
            TokenPayload::Stored {
                tokens: output.token_list(),
                origins: output.origin_list(),
            },
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(crate::input::StoredReplayReason::OutputRoutine),
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Push,
            reason: InputReason::OutputRoutine,
            level: level.0,
            position: 0,
        }));
        Ok(())
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
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Diagnostic(
            crate::observation::DiagnosticRecord {
                severity: "error",
                diagnostic: "off_save_replay",
                arguments: vec![DiagnosticArgument::Token(
                    self.observed_command_spelling(&command),
                )],
            },
        ));
        self.back_input(command)?;
        let level = self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(
                closing
                    .iter()
                    .map(|&token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
                    .collect::<Vec<_>>(),
            )),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
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

    /// Whether this delivery has the exact control-sequence spelling `name`.
    ///
    /// Meaning identity is deliberately insufficient here: frozen alignment
    /// aliases can share `end_group` semantics without being ordinary
    /// `\\endgroup` main-control recovery commands.
    #[must_use]
    pub fn has_control_sequence_spelling(&self, command: &CurrentCommand, name: &str) -> bool {
        command
            .control_sequence()
            .is_some_and(|symbol| self.state.resolve(symbol) == name)
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
        #[cfg(any(test, feature = "instrumentation"))]
        let previous_align_state = self.command.alignment.align_state;
        #[cfg(any(test, feature = "instrumentation"))]
        let adjustment = command.alignment_adjustment();
        self.undo_alignment_delivery(&command);

        let level = self.command.push_token_level(
            TokenPayload::BackedUp(SharedBackedUpBuffer::new(vec![BackedUpToken {
                spelling: command.spelling(),
                source_provenance: command.source_provenance(),
            }])),
            TokenBehavior::BackedUp(treatment),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Backup,
                reason: InputReason::Backup,
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

    fn get_next_with_control_sequence_creation(
        &mut self,
        allow_control_sequence_creation: bool,
    ) -> Result<Option<CommandReplayDelivery>, CommandError> {
        loop {
            if let Some(episode) = self.replay_completion.take() {
                return Ok(Some(CommandReplayDelivery::Completed(episode)));
            }
            let Some(delivery) = self.take_input_token(allow_control_sequence_creation)? else {
                if let Some(episode) = self.replay_completion.take() {
                    return Ok(Some(CommandReplayDelivery::Completed(episode)));
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
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe(CommandObservation::Input(InputRecord {
                        transition: InputTransition::Push,
                        reason: InputReason::Parameter,
                        level: _parameter_level.0,
                        position: 0,
                    }));
                    continue;
                }
            }

            let delivery_stamp = DeliveryStamp::new(level.0, position, self.next_delivery_sequence);
            self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
            let mut command = CurrentCommand::resolve(
                spelling,
                delivery_stamp,
                source_provenance,
                direct_source,
                &mut self.state,
            );
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
            #[cfg(any(test, feature = "instrumentation"))]
            let previous_align_state = self.command.alignment.align_state;
            let adjustment = self.command.alignment.classify_delivery(&mut command);
            command.set_alignment_adjustment(adjustment);
            #[cfg(any(test, feature = "instrumentation"))]
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
            #[cfg(any(test, feature = "instrumentation"))]
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
        loop {
            let Some(level) = self.command.input.levels.last().cloned() else {
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Input(InputRecord {
                    transition: InputTransition::Stop,
                    reason: InputReason::Source,
                    level: 0,
                    position: 0,
                }));
                return Ok(None);
            };
            match level {
                InputLevel::Source(source) => {
                    let identity = source.identity;
                    let position = source.cursor.next_physical_offset;
                    self.ensure_source_registration(&source.cursor.backing);
                    match self.next_source_step() {
                        SourceTokenizationStep::Token(token) => {
                            let spelling =
                                self.source_spelling(&token, allow_control_sequence_creation);
                            return Ok(Some(DeliveredToken {
                                spelling,
                                level: identity,
                                position,
                                behavior: TokenBehavior::Ordinary,
                                source_provenance: Some(token.provenance()),
                                direct_source: true,
                            }));
                        }
                        SourceTokenizationStep::InvalidCharacter(_) => continue,
                        SourceTokenizationStep::End => {
                            // TeX82 §343 checks outer validity immediately
                            // after `end_file_reading`, before `get_next`
                            // resumes the caller's input level.  In
                            // particular, a skipped conditional that reaches
                            // EOF in a nested `\\input` must insert frozen
                            // `\\fi` above the parent, rather than allowing
                            // the parent's next token to escape `pass_text`.
                            match self.retire_and_restart(identity)? {
                                RetirementRestart::Stop => return Ok(None),
                                RetirementRestart::Continue => {
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
                InputLevel::Tokens(cursor) => {
                    let identity = cursor.identity;
                    if let Some((spelling, position, behavior, source_provenance)) =
                        self.next_stored_token(&cursor)
                    {
                        let InputLevel::Tokens(cursor) = self
                            .command
                            .input
                            .levels
                            .last_mut()
                            .expect("inspected input level remains live")
                        else {
                            unreachable!("inspected token level remains a token level");
                        };
                        cursor.index += 1;
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
                                    OriginId::UNKNOWN,
                                ),
                                level,
                                position: u64::try_from(cursor.index)
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

    fn retire_and_restart(
        &mut self,
        identity: InputLevelId,
    ) -> Result<RetirementRestart, CommandError> {
        let retirement = self
            .command
            .retire_exhausted_input(identity)
            .map_err(|_| CommandError::input_invariant())?;
        let action = retirement.action;
        #[cfg(any(test, feature = "instrumentation"))]
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
                level: identity.0,
                position: 0,
            }));
        }
        // The pinned observer names a retiring alignment template from the
        // token list the level holds, exactly as tex.web's `end_token_list`
        // distinguishes `start=omit_template` from a column's ⟨v_j⟩ part.
        // A retained v-template has not left the stack yet, so it is not
        // named here.
        #[cfg(any(test, feature = "instrumentation"))]
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
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            }));
        }
        match action {
            InputRetirementAction::TerminalStop => Ok(RetirementRestart::Stop),
            InputRetirementAction::VTemplateRetained => {
                // The exhausted frame remains live while `get_next` delivers
                // frozen end-template. Its expanded `endv` is then handled by
                // typed `do_endv`, which retires this exact frame.
                Ok(RetirementRestart::EndV(identity))
            }
            InputRetirementAction::SourcePopped
            | InputRetirementAction::TokenListPopped
            | InputRetirementAction::ScantokensClosed
            | InputRetirementAction::VTemplatePopped => {
                #[cfg(any(test, feature = "instrumentation"))]
                let previous_align_state = self.command.alignment.align_state;
                if self.command.alignment.finish_u_template(identity) {
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe(CommandObservation::Alignment(AlignmentRecord {
                        transition: "state_change",
                        alignment: self
                            .command
                            .alignment
                            .active_alignment
                            .map(|alignment| alignment.raw()),
                        align_state: self.command.alignment.align_state,
                        delimiter: None,
                        previous_align_state: Some(previous_align_state),
                    }));
                }
                if let Some(episode) = self.command.take_replay_completion(identity) {
                    self.replay_completion = Some(episode);
                    Ok(RetirementRestart::Completed)
                } else {
                    Ok(RetirementRestart::Continue)
                }
            }
        }
    }

    fn next_source_step(&mut self) -> SourceTokenizationStep {
        let profile = self.command.profile();
        // TeX82's `firm_up_the_line` captures `end_line_char` when it loads
        // each physical line.  The cursor keeps that captured value through
        // the line, so assignments affect the next refill but cannot rewrite
        // a partially consumed line.
        let endlinechar = self.state.int_param(IntParam::END_LINE_CHAR);
        let catcode = |code: CharacterCode| self.state.catcode(character_from_code(code));
        match profile.character_mode() {
            CharacterMode::EightBitExact => {
                self.command.next_exact_source_step(endlinechar, catcode)
            }
            CharacterMode::UnicodeExtended => {
                self.command.next_unicode_source_step(endlinechar, catcode)
            }
        }
    }

    fn ensure_source_registration(&mut self, source: &crate::input::RegisteredSource) {
        let _ = self
            .state
            .register_source(source.id, source.source_descriptor());
    }

    /// Resolves one scanned source token into its semantic spelling.
    ///
    /// `allow_control_sequence_creation` is TeX82's `no_new_control_sequence`
    /// inverted. §257 sets that flag, §365 clears it only around `get_token`,
    /// and §374 clears it only around `\csname`'s `id_lookup`, so a raw
    /// `get_next` may not enter a new name into the hash table: §259's
    /// `id_lookup` hands it §222's dummy `undefined_control_sequence`
    /// instead. Only §356's multiletter branch (`k>loc+1`) consults the hash
    /// at all -- §354 resolves a control symbol to `single_base+c` and an
    /// escape at line end to `null_cs`, and §351 gives a blank line's `\par`
    /// `par_loc`, all permanent eqtb locations that exist before any scan.
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
                    let name: String = name.iter().copied().map(character_from_code).collect();
                    if hashed && !allow_control_sequence_creation {
                        self.state
                            .known_control_sequence(&name)
                            .map_or_else(Token::undefined_control_sequence, Token::Cs)
                    } else {
                        Token::Cs(self.state.intern_control_sequence(&name))
                    }
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
        &self,
        cursor: &TokenCursor,
    ) -> Option<(
        TracedTokenWord,
        u64,
        TokenBehavior,
        Option<SourceProvenance>,
    )> {
        let position = u64::try_from(cursor.index).ok()?;
        let spelling = match &cursor.payload {
            TokenPayload::Transient(buffer) => {
                buffer.get(cursor.index).map(|spelling| (spelling, None))
            }
            TokenPayload::BackedUp(buffer) => buffer
                .get(cursor.index)
                .map(|token| (token.spelling, token.source_provenance)),
            TokenPayload::ArgumentRange { buffer, range } => (cursor.index
                < range.end().saturating_sub(range.start()))
            .then(|| buffer.get(range.start() + cursor.index))
            .flatten()
            .map(|spelling| (spelling, None)),
            TokenPayload::Stored { tokens, origins } => {
                let token = *self.state.tokens(*tokens).get(cursor.index)?;
                let origin = self
                    .state
                    .origin_list(*origins)
                    .get(cursor.index)
                    .copied()
                    .unwrap_or(OriginId::UNKNOWN);
                Some((TracedTokenWord::pack(token, origin), None))
            }
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
        let mut completed = false;
        loop {
            let depleted = match self.command.input.levels.last() {
                Some(InputLevel::Tokens(cursor))
                    if drains_for_stack_conservation(&cursor.behavior)
                        && self.next_stored_token(cursor).is_none() =>
                {
                    Some(cursor.identity)
                }
                Some(InputLevel::Tokens(_)) | Some(InputLevel::Source(_)) | None => None,
            };
            let Some(identity) = depleted else {
                return Ok(());
            };
            match self.retire_and_restart(identity)? {
                RetirementRestart::Continue => {}
                // A finished stored replay episode is recorded on the
                // processor and reported to the caller by the next `get_next`;
                // draining continues so the whole depleted run is cleaned off.
                // Two completions in one drain would overwrite that one slot,
                // so refuse loudly rather than dropping an episode silently.
                RetirementRestart::Completed if !completed => completed = true,
                RetirementRestart::Stop
                | RetirementRestart::EndV(_)
                | RetirementRestart::Completed => {
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
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe_outer_validity_diagnostic(&recovery.status, false);
        if matches!(recovery.status, ScannerStatus::Matching(_)) {
            self.outer_recovered_while_matching = true;
        }
        self.back_input(command.copy_for_backup())?;
        self.install_outer_recovery(recovery)?;
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
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe_outer_validity_diagnostic(&recovery.status, true);
        self.install_outer_recovery(recovery)?;
        Ok(true)
    }

    /// TeX.web's `check_outer_validity` recovery table. Primitive insertions
    /// are frozen tokens, retaining their original meanings if user code has
    /// reassigned their visible spellings.
    fn install_outer_recovery(&mut self, recovery: RecoveryContext) -> Result<(), CommandError> {
        let RecoveryContext { status, warning } = recovery;
        #[cfg(any(test, feature = "instrumentation"))]
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
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            }));
        }
        #[cfg(any(test, feature = "instrumentation"))]
        let (frozen_recovery_name, recovery_kind) = match &status {
            ScannerStatus::Skipping(_) => (Some("fi"), RecoveryKind::InsertedToken),
            ScannerStatus::Matching(_) => (Some("par"), RecoveryKind::InsertedControlSequence),
            // TeX82's `check_outer_validity` inserts frozen `\\cr` before
            // its required follow-up right brace. The recovery event denotes
            // the inaccessible control sequence alone; raw delivery still
            // owns the whole inserted token list.
            ScannerStatus::Aligning(_) => (Some("cr"), RecoveryKind::InsertedControlSequence),
            ScannerStatus::Normal | ScannerStatus::Defining(_) | ScannerStatus::Absorbing(_) => {
                (None, RecoveryKind::InsertedToken)
            }
        };
        let tokens = match status {
            ScannerStatus::Normal => return Ok(()),
            ScannerStatus::Skipping(_) => vec![self.frozen_primitive("fi")?],
            ScannerStatus::Defining(_) | ScannerStatus::Absorbing(_) => vec![right_brace()],
            ScannerStatus::Matching(_) => vec![self.frozen_primitive("par")?],
            ScannerStatus::Aligning(_) => vec![self.frozen_primitive("cr")?, right_brace()],
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
        #[cfg(any(test, feature = "instrumentation"))]
        let observed_tokens = tokens
            .iter()
            .copied()
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
            TokenPayload::Transient(SharedTokenBuffer::new(tokens)),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Recovery,
            reason: InputReason::Recovery,
            level: level.0,
            position: 0,
        }));
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Recovery(RecoveryRecord {
            kind: recovery_kind,
            tokens: observed_tokens,
        }));
        Ok(())
    }

    #[cfg(any(test, feature = "instrumentation"))]
    fn observe_outer_validity_diagnostic(&mut self, status: &ScannerStatus, at_eof: bool) {
        let status_name = match status {
            ScannerStatus::Normal => "normal",
            ScannerStatus::Skipping(_) => "skipping",
            ScannerStatus::Defining(_) => "defining",
            ScannerStatus::Matching(_) => "matching",
            ScannerStatus::Aligning(_) => "aligning",
            ScannerStatus::Absorbing(_) => "absorbing",
        };
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic: if at_eof {
                "outer_validity_eof"
            } else {
                "outer_validity_control_sequence"
            },
            arguments: vec![DiagnosticArgument::Name(status_name.into())],
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

    #[cfg(any(test, feature = "instrumentation"))]
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

    #[cfg(any(test, feature = "instrumentation"))]
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

    #[cfg(any(test, feature = "instrumentation"))]
    fn observe_raw_delivery(&mut self, command: &CurrentCommand) {
        let (command_name, command_operand) =
            crate::observation::canonical_current_command_identity(command);
        let spelling = self.observed_command_spelling(command);
        self.observe(CommandObservation::Command(CommandDeliveryRecord {
            boundary: CommandDeliveryBoundary::Raw,
            spelling,
            command: command_name,
            command_operand,
            provenance: CommandProvenance::from_stamp(
                command.delivery_stamp(),
                command.origin(),
                command.direct_source_provenance(),
            ),
        }));
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

#[cfg(any(test, feature = "instrumentation"))]
fn observed_retirement_reason(
    action: InputRetirementAction,
    reason: InputRetirementReason,
) -> InputReason {
    match (action, reason) {
        (InputRetirementAction::SourcePopped | InputRetirementAction::TerminalStop, _) => {
            InputReason::Source
        }
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
#[cfg(any(test, feature = "instrumentation"))]
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
        Stored::Mark => InputReason::Mark,
        Stored::Write => InputReason::Write,
        Stored::Discretionary => InputReason::UmberReplay(UmberReplayKind::Discretionary),
        Stored::AfterAssignment => InputReason::UmberReplay(UmberReplayKind::AfterAssignment),
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

fn character_from_code(code: CharacterCode) -> char {
    match code.to_byte() {
        Ok(byte) => char::from(byte),
        Err(_) => code
            .to_char()
            .expect("registered Unicode source supplies valid scalars"),
    }
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
    use crate::input::{ReplayTrace, RetirementBehavior};
    use crate::observation::{
        CommandDeliveryBoundary, CommandObservation, CommandObserver, InputTransition,
    };
    use crate::processor::{
        AbsorbingContext, AlignmentId, AlignmentScanContext, ArgumentBuilderId, ConditionId,
        DefinitionContext, MatchingContext, ScannerWarning, SkippingContext, TokenBuilderId,
    };
    use crate::{
        CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState,
        RegisteredSourceKind, SourceRegistration,
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
        runtime: &'a mut CommandRuntime,
        universe: &'a mut Universe,
        capabilities: &'a mut CommandHostCapabilities,
    ) -> CommandProcessor<'a> {
        CommandProcessor::new(
            command,
            runtime,
            universe.command_context(),
            CommandHostContext::new(capabilities),
        )
    }

    fn templates() -> crate::AlignmentCellTemplates {
        crate::AlignmentCellTemplates {
            u_template: None,
            v_template: tex_state::input::TracedTokenList::synthetic(
                tex_state::ids::TokenListId::EMPTY,
            ),
        }
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
        let alignment = crate::AlignmentIdentity::new(17);
        command.begin_alignment(alignment);
        command
            .apply_alignment_request(crate::AlignmentRequest::BeginCell {
                alignment,
                templates: templates(),
            })
            .expect("cell begins");
        command
            .apply_alignment_request(crate::AlignmentRequest::InstallCellTemplate(alignment))
            .expect("empty template installs");
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

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
        command.alignment.align_state = crate::processor::TOP_LEVEL_ALIGN_STATE + 1;
        let alignment = crate::AlignmentIdentity::new(23);
        command.begin_alignment(alignment);
        assert_eq!(
            command.alignment.align_state,
            crate::processor::alignment::PREAMBLE_ALIGN_STATE
        );
        command
            .apply_alignment_request(crate::AlignmentRequest::Finish(alignment))
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
        let alignment = crate::AlignmentIdentity::new(18);
        command.begin_alignment(alignment);
        command
            .apply_alignment_request(crate::AlignmentRequest::BeginCell {
                alignment,
                templates: templates(),
            })
            .expect("empty-template cell begins");
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        universe.set_catcode('~', Catcode::Active);
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut first = Recorder::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
                    .with_observer(&mut first);
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
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        universe.install_primitive_meaning(
            "hfil",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HFil),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        universe.install_primitive_meaning(
            "hskip",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
            let mut runtime = CommandRuntime::default();
            let mut universe = Universe::new_with_plain_catcodes();
            universe.install_primitive_meaning(name, Meaning::UnexpandablePrimitive(primitive));
            let mut capabilities = CommandHostCapabilities::default();
            let mut recorder = Recorder::default();
            {
                let mut processor =
                    processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
        let mut observed_runtime = CommandRuntime::default();
        let mut unobserved_runtime = CommandRuntime::default();
        let mut observed_universe = Universe::new_with_plain_catcodes();
        let mut unobserved_universe = Universe::new_with_plain_catcodes();
        let mut observed_capabilities = CommandHostCapabilities::default();
        let mut unobserved_capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(
                &mut observed,
                &mut observed_runtime,
                &mut observed_universe,
                &mut observed_capabilities,
            )
            .with_observer(&mut recorder);
            while processor.get_next().expect("delivery succeeds").is_some() {}
        }
        {
            let mut processor = processor(
                &mut unobserved,
                &mut unobserved_runtime,
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
            TokenPayload::Stored {
                tokens: stored.token_list(),
                origins: stored.origin_list(),
            },
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::MacroReplacement,
        );
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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

    /// TeX82 §342's `@<If an alignment entry has just ended, take appropriate
    /// action@>` is the tail of §341's `get_next`, so §789's ⟨v_j⟩ template
    /// becomes the live input before any raw reader sees the delimiter. The
    /// token `get_next` returns is the template's first token, never the tab
    /// mark, `\span`, or `\cr` that ended the entry (`umber2-johp.258`).
    #[test]
    fn get_next_inserts_the_v_template_for_a_top_level_alignment_delimiter() {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(19);
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let v_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list(&[
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
            .install_alignment_cell_template(alignment)
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let alignment = crate::AlignmentIdentity::new(18);
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(alignment, templates())
            .expect("cell begins");
        command
            .install_alignment_cell_template(alignment)
            .expect("cell without a u-template installs");
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
    fn retained_v_template_returns_saved_delimiter_structurally_after_endv() {
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let u_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list(&[
                Token::Char {
                    ch: 'u',
                    cat: Catcode::Letter,
                },
            ]));
        let v_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list(&[
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
            .install_alignment_cell_template(alignment)
            .expect("u-template installs after the cell opener lifecycle");
        let snapshot = command.snapshot();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
            let endv = processor
                .get_x_token()
                .expect("end-template expands to end-v")
                .expect("end-v delivery");
            assert!(matches!(endv.meaning(), Meaning::EndV));
            assert!(endv.spelling().semantic_token().is_frozen_endv());
            let finished = processor
                .command
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let u_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list(&[
                Token::Char {
                    ch: 'u',
                    cat: Catcode::Letter,
                },
            ]));
        let v_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list(&[
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
            .install_alignment_cell_template(alignment)
            .expect("u-template installs after the cell opener lifecycle");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let target = universe.intern("target").symbol();
        universe.set_meaning(
            target,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let mut runtime = CommandRuntime::default();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let mut capabilities = CommandHostCapabilities::default();
        let warning = ScannerWarning(17);

        let cases = [
            (
                ScannerStatus::Skipping(SkippingContext {
                    condition: ConditionId(1),
                    warning,
                }),
                vec![universe.primitive_token("fi").expect("fi is registered")],
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
            ),
            (
                ScannerStatus::Matching(MatchingContext {
                    macro_name: universe.intern("argument").symbol(),
                    builder: ArgumentBuilderId(3),
                    warning,
                }),
                vec![universe.primitive_token("par").expect("par is registered")],
            ),
            (
                ScannerStatus::Aligning(AlignmentScanContext {
                    alignment: AlignmentId(4),
                    builder: TokenBuilderId(5),
                    warning,
                }),
                vec![
                    universe.primitive_token("cr").expect("cr is registered"),
                    Token::Char {
                        ch: '}',
                        cat: Catcode::EndGroup,
                    },
                ],
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
            ),
        ];

        for (status, expected) in cases {
            let actual = command.with_scanner_status(status, |command| {
                let mut processor =
                    processor(command, &mut runtime, &mut universe, &mut capabilities);
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
        }
        assert_eq!(command.expansion.pending_diagnostics, vec![17; 5]);
    }

    #[test]
    fn aligning_eof_recovery_preserves_frozen_cr_identity_as_an_inserted_control_sequence() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let frozen_cr = universe.primitive_token("cr").expect("cr is registered");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();

        command.begin_alignment(crate::AlignmentIdentity::new(4));
        command.with_scanner_status(
            ScannerStatus::Aligning(AlignmentScanContext {
                alignment: AlignmentId(4),
                builder: TokenBuilderId(5),
                warning: ScannerWarning(17),
            }),
            |command| {
                let mut processor =
                    processor(command, &mut runtime, &mut universe, &mut capabilities)
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
    }

    #[test]
    fn skipping_eof_reports_diagnostics_before_frozen_fi_recovery() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let frozen_fi = universe.primitive_token("fi").expect("fi is registered");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();

        command.with_scanner_status(
            ScannerStatus::Skipping(SkippingContext {
                condition: ConditionId(1),
                warning: ScannerWarning(17),
            }),
            |command| {
                let mut processor =
                    processor(command, &mut runtime, &mut universe, &mut capabilities)
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        recovery_primitives(&mut universe);
        let frozen_fi = universe.primitive_token("fi").expect("fi is registered");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();

        command.with_scanner_status(
            ScannerStatus::Skipping(SkippingContext {
                condition: ConditionId(1),
                warning: ScannerWarning(17),
            }),
            |command| {
                let mut processor =
                    processor(command, &mut runtime, &mut universe, &mut capabilities)
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
        let mut runtime = CommandRuntime::default();
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
                definition,
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
                let mut processor =
                    processor(command, &mut runtime, &mut universe, &mut capabilities);
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
                        definition
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
        let mut runtime = CommandRuntime::default();
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
                definition,
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
            let mut processor = processor(command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
