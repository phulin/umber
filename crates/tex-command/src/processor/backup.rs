//! Canonical command backup and replay insertion transitions.

use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, TracedTokenWord};

use crate::command::CurrentCommand;
use crate::error::CommandError;
use crate::input::{
    BackedUpToken, BackupTreatment, InputLevel, PackedTokenSpanHandle, ReplayTrace,
    RetirementBehavior, TokenBehavior,
};
use crate::observation::{
    AlignmentRecord, CommandObservation, InputReason, InputRecord, InputTransition, RecoveryKind,
    RecoveryRecord,
};

use super::CommandProcessor;
use super::alignment::AlignmentDeliveryState;

impl<G> CommandProcessor<'_, '_, G> {
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
        self.conserve_input_stack_for_descendant()?;
        self.command
            .alignment
            .undo_delivery(AlignmentDeliveryState::<G>::back_input_adjustment(
                spelling.semantic_token(),
            ));
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::backed_up([BackedUpToken { spelling }]),
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
            let Some(InputLevel::ReplayTokens(row)) = self.command.input.levels.last() else {
                unreachable!("back_input above installed a token-list level");
            };
            assert_eq!(
                row.header.position(),
                0,
                "no delivery occurs while e-TeX links aftergroup tokens"
            );
            let replay = row.replay;
            let admitted = self
                .command
                .input
                .replay
                .prepend_backed_up(
                    replay,
                    tokens
                        .into_iter()
                        .map(|spelling| BackedUpToken { spelling }),
                )
                .map_err(|_| CommandError::input_invariant())?;
            let Ok(prepended) = u32::try_from(prepended) else {
                return Err(CommandError::input_invariant());
            };
            debug_assert_eq!(admitted, prepended);
            let replay_cursor = self
                .command
                .input
                .replay
                .resident_cursor(replay)
                .ok_or_else(CommandError::input_invariant)?;
            let extended = self
                .command
                .input
                .levels
                .extend_top_token_limit(prepended, replay_cursor)
                .expect("back_input above installed a token-list level");
            if !extended {
                return Err(CommandError::input_invariant());
            }
        } else {
            for spelling in tokens.into_iter().rev() {
                self.back_input_token(spelling)?;
            }
        }
        Ok(())
    }
    /// Canonical backing operation used by `\\noexpand` for one replayed
    /// command. The treatment belongs to the backed-up level, not the token
    /// or the returned command.
    pub(crate) fn back_input_with_treatment(
        &mut self,
        command: CurrentCommand<G>,
        treatment: BackupTreatment,
    ) -> Result<(), CommandError> {
        if !self.delivery_is_fresh(&command) {
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
        self.invalidate_delivery_freshness();
        // §325 runs the stack-conservation loop before it touches
        // `align_state` and before it pushes the `backed_up` list, so every
        // depleted level retires ahead of the backup.
        self.conserve_input_stack_for_descendant()?;
        let previous_align_state = self.command.alignment.align_state;
        let adjustment = command.alignment_adjustment();
        self.undo_alignment_delivery(&command);

        let level = self.command.push_token_level(
            PackedTokenSpanHandle::backed_up([BackedUpToken {
                spelling: command.spelling(),
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
