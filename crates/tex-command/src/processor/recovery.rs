//! Canonical command-input recovery and backup transitions.

use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::AlignmentDeliveryEvent;
use crate::command::CurrentCommand;
use crate::error::CommandError;
use crate::input::{PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, TokenBehavior};
use crate::observation::{
    CommandObservation, DiagnosticArgument, InputReason, InputRecord, InputTransition,
    RecoveryKind, RecoveryRecord,
};

use super::CommandProcessor;

impl<G> CommandProcessor<'_, '_, G> {
    /// Captures a complete command-neutral site for an error that is being
    /// queued by the executor while the delivered command is still live.
    /// Scanner callers use the two lower-level steps separately when TeX
    /// backs the command up between capture and report completion.
    pub fn current_diagnostic_site(
        &mut self,
        command: Option<&crate::CurrentCommand<G>>,
    ) -> tex_state::diagnostic::DiagnosticSite {
        self.complete_diagnostic_site(self.capture_diagnostic_site(command))
    }

    /// Captures only command-owned diagnostic facts before a scanner can move
    /// the offending command into a backed-up input level. No live command or
    /// context borrow crosses the report queue.
    pub fn capture_diagnostic_site(
        &self,
        command: Option<&crate::CurrentCommand<G>>,
    ) -> tex_state::diagnostic::DiagnosticSite {
        let (command_name, command_operand, observed_token, origin) =
            command.map_or((None, None, None, None), |command| {
                let (name, operand) =
                    crate::observation::canonical_current_command_identity_for_profile(
                        self.command.profile(),
                        command,
                    );
                (
                    Some(name),
                    operand,
                    Some(neutral_diagnostic_token(
                        self.observed_command_spelling(command),
                    )),
                    (command.origin() != tex_state::token::OriginId::UNKNOWN)
                        .then_some(command.origin()),
                )
            });
        tex_state::diagnostic::DiagnosticSite {
            origin,
            observed_token,
            command: command_name,
            command_operand,
            context: None,
            mode: None,
            scanner_status: "normal",
            interaction: Some(self.diagnostic_interaction()),
        }
    }

    /// Captures command-owned diagnostic facts from the compact hot command
    /// before a recovery path materializes or backs it up.
    pub(crate) fn capture_hot_diagnostic_site(
        &self,
        command: &crate::command::HotCommand<G>,
    ) -> tex_state::diagnostic::DiagnosticSite {
        let (name, operand) = crate::observation::canonical_delivery_identity_for_profile(
            self.command.profile(),
            command.identity(),
            command.resolved_meaning(),
        );
        tex_state::diagnostic::DiagnosticSite {
            origin: (command.origin() != tex_state::token::OriginId::UNKNOWN)
                .then_some(command.origin()),
            observed_token: Some(neutral_diagnostic_token(
                self.observed_hot_command_spelling(command),
            )),
            command: Some(name),
            command_operand: operand,
            context: None,
            mode: None,
            scanner_status: "normal",
            interaction: Some(self.diagnostic_interaction()),
        }
    }

    fn diagnostic_interaction(&self) -> tex_state::InteractionMode {
        match self.state.interaction_mode_value() {
            0 => tex_state::InteractionMode::Batch,
            1 => tex_state::InteractionMode::Nonstop,
            2 => tex_state::InteractionMode::Scroll,
            3 => tex_state::InteractionMode::ErrorStop,
            _ => tex_state::InteractionMode::ErrorStop,
        }
    }

    /// Freezes the report-time context, mode, and scanner state onto a
    /// command-neutral site. This is called after any required `back_input`,
    /// so the context describes the same backed-up token that §82 displays.
    pub fn complete_diagnostic_site(
        &mut self,
        mut site: tex_state::diagnostic::DiagnosticSite,
    ) -> tex_state::diagnostic::DiagnosticSite {
        let (input_frame_count, input_frame_tail) = self.command.diagnostic_input_context(8);
        let group_tail = self
            .state
            .group_frames()
            .iter()
            .rev()
            .take(8)
            .map(|frame| tex_state::diagnostic::DiagnosticGroup {
                kind: frame.kind().diagnostic_name(),
                entered_line: frame.entered_line(),
            })
            .collect();
        site.context = Some(tex_state::diagnostic::DiagnosticContext {
            input_frame_count,
            input_frame_tail,
            group_depth: u32::try_from(self.state.group_frames().len()).unwrap_or(u32::MAX),
            group_tail,
        });
        site.mode = Some(self.host.diagnostic_mode_name());
        site.scanner_status =
            crate::observation::canonical_names::scanner_status_name(self.command.scanner.status());
        site
    }

    fn complete_first_recoverable_site(
        &mut self,
        site: Option<tex_state::diagnostic::DiagnosticSite>,
    ) {
        if !self.diagnostic_effects.first_recoverable_site_missing() {
            return;
        }
        let mut site = site.unwrap_or_else(|| self.capture_diagnostic_site(None));
        if site.command.is_none() && site.observed_token.is_none() {
            // Reports which did not retain a command (startup/format and
            // legacy outer seams) still get structural context, but their
            // interaction is already fixed by ErrorReport::error_inner. Do
            // not let the post-dialog mode overwrite that response.
            site.interaction = None;
        }
        let site = self.complete_diagnostic_site(site);
        self.diagnostic_effects.complete_first_recoverable(site);
    }

    /// Reports TeX82 §1096's `hmode+par_end` recovery predicate.
    ///
    /// `align_state` belongs to raw command delivery, so the stomach asks
    /// this command-owned question instead of mirroring the brace counter.
    #[must_use]
    pub fn paragraph_end_needs_alignment_recovery(&self) -> bool {
        self.command.alignment.align_state < 0
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
                    self.invalidate_delivery_freshness();
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
        self.complete_first_recoverable_site(None);
        match outcome {
            tex_state::print::ErrorOutcome::Continue => Ok(()),
            tex_state::print::ErrorOutcome::Recovery(request) => {
                self.apply_error_stop_recovery(request)
            }
            tex_state::print::ErrorOutcome::JumpOut(jump) => Err(jump.into()),
        }
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
        self.invalidate_delivery_freshness();
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient([TracedTokenWord::pack(token, OriginId::UNKNOWN)]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        self.observe_inserted_token_recovery(level, token);
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
    /// Restores a command and composes the report §82 will render.
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
        let site = self.capture_diagnostic_site(Some(&command));
        self.back_input(command)?;
        let context = self.command.output_open_context(self.state);
        let site = Some(self.complete_diagnostic_site(site));
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: diagnostic,
                runaway: None,
                message,
                help,
                context,
                integer_error: None,
                site,
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
        let context = self.command.output_open_context(self.state);
        let site = Some(self.current_diagnostic_site(None));
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: diagnostic,
                runaway: None,
                message,
                help,
                context,
                integer_error: None,
                site,
            });
    }
}

fn neutral_diagnostic_token(
    token: crate::observation::ObservedToken,
) -> tex_state::diagnostic::DiagnosticToken {
    match token {
        crate::observation::ObservedToken::Character { character, catcode } => {
            tex_state::diagnostic::DiagnosticToken::Character { character, catcode }
        }
        crate::observation::ObservedToken::ControlSequence(value) => {
            tex_state::diagnostic::DiagnosticToken::ControlSequence(value)
        }
        crate::observation::ObservedToken::MacroMatch => {
            tex_state::diagnostic::DiagnosticToken::MacroMatch
        }
        crate::observation::ObservedToken::MacroEndMatch => {
            tex_state::diagnostic::DiagnosticToken::MacroEndMatch
        }
        crate::observation::ObservedToken::Parameter(value) => {
            tex_state::diagnostic::DiagnosticToken::Parameter(value)
        }
        crate::observation::ObservedToken::FrozenEndTemplate => {
            tex_state::diagnostic::DiagnosticToken::FrozenEndTemplate
        }
        crate::observation::ObservedToken::FrozenEndV => {
            tex_state::diagnostic::DiagnosticToken::FrozenEndV
        }
        crate::observation::ObservedToken::FrozenPrimitive(value) => {
            tex_state::diagnostic::DiagnosticToken::FrozenPrimitive(value)
        }
        crate::observation::ObservedToken::FrozenOther => {
            tex_state::diagnostic::DiagnosticToken::FrozenOther
        }
    }
}
