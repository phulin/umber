//! Structural TeX expansion primitives.

use tex_state::meaning::{ExpandablePrimitive, Meaning, ResolvedMeaning};
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::command::HotCommand;
use crate::input::{
    BackedUpToken, BackupTreatment, PackedTokenSpanHandle, ReplayTrace, RetirementBehavior,
    TokenBehavior,
};
use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, RecoveryKind, RecoveryRecord,
};
use crate::processor::status::{ScannerStatus, ScannerStatusVisibility};
use crate::{CommandError, CurrentCommand};

use super::expand::is_expandable_command;
use super::expand_render::print_esc_text;
use super::{CommandProcessor, DeliveryStatus};

/// Operand state held by TeX82 §368 while `\expandafter` expands its second
/// command across an immutable host suspension.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingExpandAfter<G> {
    first: CurrentCommand<G>,
    child: Option<crate::execution_scratch::ChildContinuation<G, PendingExpandAfterDestination>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
enum PendingExpandAfterDestination {
    ExpandingSecond,
}

impl<G> PendingExpandAfter<G> {
    pub(crate) fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        self.child.take().map(|child| child.restore().0)
    }
}

/// Stable pending-diagnostic identity for TeX.web's `Missing \\endcsname
/// inserted` recovery. Rendering belongs to the diagnostic milestone.
pub(crate) const MISSING_ENDCSNAME_DIAGNOSTIC: u64 = 0x6373_6e61_6d65_0001;

impl<G> CommandProcessor<'_, '_, G> {
    /// Starts the hot `\csname` continuation. The name spelling is appended
    /// directly to the generation-owned fixed-chunk lane while the control
    /// retains only its opener origin and dynamic `ifincsname` bit.
    pub(super) fn begin_csname_continuation_with_parent(
        &mut self,
        opener: OriginId,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        let previous = self.is_in_csname;
        self.command
            .scratch
            .push_csname_control_with_parent(opener, previous, parent)
            .map_err(crate::scan_toks::scratch_command_error)?;
        self.is_in_csname = true;
        Ok(())
    }

    /// Appends one compact character command to the active name lane. UTF-8
    /// encoding is performed in a stack-local array; the lane itself grows in
    /// fixed chunks and never allocates per character after warmup.
    pub(super) fn append_csname_character(&mut self, character: char) -> Result<(), CommandError> {
        let mut bytes = [0_u8; 4];
        for byte in character.encode_utf8(&mut bytes).as_bytes() {
            self.command
                .scratch
                .push_name_byte(*byte)
                .map_err(crate::scan_toks::scratch_command_error)?;
        }
        Ok(())
    }

    /// Completes a hot `\expandafter` after its second command has settled on
    /// an unexpandable token. Both operands remain compact until this true
    /// backup/replay boundary, where the existing semantic input machinery
    /// receives the materialized commands.
    pub(super) fn complete_expandafter_continuation(
        &mut self,
        second: HotCommand<G>,
    ) -> Result<(), CommandError> {
        let control = self
            .command
            .scratch
            .pop_expandafter_control()
            .map_err(crate::scan_toks::scratch_command_error)?;
        let first = control
            .saved_first
            .ok_or_else(CommandError::input_invariant)?;
        self.back_input(second.materialize())?;
        self.replay_expandafter_first(first.materialize())
    }

    /// Completes a hot `\expandafter` whose second expandable command
    /// consumed itself without producing a token (an undefined command, an
    /// empty macro, or a conditional). The saved first token is replayed
    /// immediately; the next input token is not accidentally consumed as the
    /// second operand.
    pub(super) fn complete_expandafter_without_second(&mut self) -> Result<(), CommandError> {
        let control = self
            .command
            .scratch
            .pop_expandafter_control()
            .map_err(crate::scan_toks::scratch_command_error)?;
        let first = control
            .saved_first
            .ok_or_else(CommandError::input_invariant)?;
        self.replay_expandafter_first(first.materialize())
    }

    /// Returns whether the top hot control is waiting for the settled second
    /// operand. This keeps the expanded loop's post-dispatch decision typed;
    /// it never peeks into a rich pending command.
    pub(super) fn expandafter_second_pending(&self) -> Result<bool, CommandError> {
        self.command
            .scratch
            .top_expandafter_control()
            .map(|control| {
                control.is_some_and(|control| {
                    control.phase
                        == crate::expansion_work::control::SynchronousExpandAfterPhase::NeedSecond
                })
            })
            .map_err(crate::scan_toks::scratch_command_error)
    }

    /// Completes the active hot `\csname`, or performs TeX82's missing
    /// `\endcsname` recovery for the already-delivered offending command.
    /// Name materialization is a semantic boundary and therefore occurs only
    /// once, after all expanded character delivery has settled.
    pub(super) fn complete_csname_continuation(
        &mut self,
        offending: Option<CurrentCommand<G>>,
    ) -> Result<(), CommandError> {
        let control = self
            .command
            .scratch
            .top_csname_control()
            .map_err(crate::scan_toks::scratch_command_error)?
            .ok_or_else(CommandError::input_invariant)?;
        let bytes = self
            .command
            .scratch
            .expansion_name_bytes(control.name)
            .map_err(crate::scan_toks::scratch_command_error)?
            .collect::<Vec<_>>();
        let name = String::from_utf8(bytes).map_err(|_| CommandError::input_invariant())?;
        let control = self
            .command
            .scratch
            .pop_csname_control()
            .map_err(crate::scan_toks::scratch_command_error)?;
        self.is_in_csname = control.previous_in_csname;
        if let Some(command) = offending {
            let rendered = print_esc_text(self.state, "endcsname");
            self.back_error_reporting(
                command,
                MISSING_ENDCSNAME_DIAGNOSTIC,
                format!("Missing {rendered} inserted"),
                &[
                    "The control sequence marked <to be read again> should",
                    "not appear between \\csname and \\endcsname.",
                ],
            )?;
        }
        self.command
            .record_csname_buffer_usage(name.chars().count());
        let symbol = self.state.intern_relaxed_control_sequence(&name);
        self.back_input_token(TracedTokenWord::pack(Token::Cs(symbol), control.opener))
    }

    /// TeX.web's `\noexpand`: read normally, then replay exactly one target
    /// from a backed-up level carrying the non-sticky suppression treatment.
    pub(super) fn expand_noexpand(&mut self) -> Result<(), CommandError> {
        let mut destination = None;
        match self.get_token_with_normal_scanner_status_into(&mut destination)? {
            DeliveryStatus::End => return Err(CommandError::input_invariant()),
            DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let target = destination
            .take()
            .expect("command status initializes destination");
        self.back_input_with_treatment(target, BackupTreatment::SuppressExpandableControlSequence)
    }

    /// Reads one token with TeX82's temporary `scanner_status := normal`
    /// scope, restoring the complete prior scanner state before returning.
    ///
    /// Both `\noexpand` (§25) and `conv_toks`'s `\string`/`\meaning` cases
    /// (§27) need this scope: their operand is delivered normally even while
    /// an enclosing `\edef` is collecting replacement text.
    pub(super) fn get_token_with_normal_scanner_status_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        if matches!(self.command.scanner.status(), ScannerStatus::Normal) {
            return self.get_token_into(destination);
        }

        let episode =
            self.begin_scanner_episode(ScannerStatus::Normal, ScannerStatusVisibility::Observed);
        let delivery = self.get_token_into(destination);
        self.finish_scanner_episode(episode);
        delivery
    }

    /// TeX.web's `\expandafter`: preserve the first token, expand (or back
    /// up) the second token, then put the first token above the resulting
    /// input. The first delivery is intentionally replayed through an
    /// explicit backed-up level because it is no longer the latest delivery.
    #[allow(dead_code)]
    pub(super) fn expand_expandafter(&mut self) -> Result<(), CommandError> {
        let pending = if self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_expandafter)
        {
            let key = self
                .scanner_resume
                .take()
                .expect("matched expandafter frame");
            Some(
                self.command
                    .scratch
                    .take_expandafter_frame(key)
                    .map_err(crate::scan_toks::scratch_command_error)?,
            )
        } else {
            None
        };
        let (first, mut second) = if let Some(mut pending) = pending {
            if let Some(child) = pending.child.take() {
                let (key, destination) = child.restore();
                if destination != PendingExpandAfterDestination::ExpandingSecond {
                    return Err(CommandError::input_invariant());
                }
                self.install_scanner_resume(Some(key));
            }
            (pending.first, None)
        } else {
            let mut first = None;
            match self.get_token_into(&mut first)? {
                DeliveryStatus::End => return Err(CommandError::input_invariant()),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary token delivery returns only commands"),
            }
            let first = first
                .take()
                .expect("command status initializes destination");
            let mut second = None;
            match self.get_token_into(&mut second)? {
                DeliveryStatus::End => return Err(CommandError::input_invariant()),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary token delivery returns only commands"),
            }
            (first, second)
        };
        if second.as_ref().is_none_or(is_expandable_command) {
            if let Err(error) = self.request_expansion_into(&mut second, true) {
                if error.is_resource_suspension() {
                    let key = self
                        .command
                        .scratch
                        .store_expandafter_frame(PendingExpandAfter {
                            first,
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                PendingExpandAfterDestination::ExpandingSecond,
                            ),
                        })
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    self.scanner_resume = Some(key);
                }
                return Err(error);
            }
            if self.scanner_resume.is_some() {
                return Err(CommandError::input_invariant());
            }
            self.replay_expandafter_first(first)?;
        } else {
            self.back_input(
                second
                    .take()
                    .expect("unexpandable second command remains in its destination"),
            )?;
            self.replay_expandafter_first(first)?;
        }
        Ok(())
    }

    /// Collects TeX82 §372's expanded character list through `\\endcsname`.
    ///
    /// e-TeX 2.6 etex.ch [17.4765--4779] deliberately reuses this exact
    /// name-building scan for `\\ifcsname`; only the subsequent hash-table
    /// operation differs.
    pub(crate) fn scan_csname_characters(
        &mut self,
        mut name: String,
        suspended: &mut Option<String>,
    ) -> Result<String, CommandError> {
        // pdfTeX section 57 saves and restores the prior flag so nested name
        // scans remain true to ifincsname and unwind to their caller.
        let previous = std::mem::replace(&mut self.is_in_csname, true);
        let result = (|| {
            let mut destination = None;
            loop {
                let status = match self.request_expanded_token(&mut destination) {
                    Ok(status) => status,
                    Err(error) => {
                        if error.is_resource_suspension() {
                            *suspended = Some(name);
                        }
                        return Err(error);
                    }
                };
                match status {
                    DeliveryStatus::End => return Err(CommandError::input_invariant()),
                    DeliveryStatus::Command => {}
                    _ => unreachable!("ordinary expanded delivery returns only commands"),
                }
                let command = destination
                    .as_ref()
                    .expect("command status initializes destination");
                match command.meaning_ref() {
                    ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                        ExpandablePrimitive::EndCsName,
                    )) => break,
                    ResolvedMeaning::Static(Meaning::CharToken { ch, .. }) => {
                        name.push(*ch);
                        destination = None;
                    }
                    _ => {
                        let rendered = print_esc_text(self.state, "endcsname");
                        let command = destination
                            .take()
                            .expect("csname recovery consumes the delivered command");
                        self.back_error_reporting(
                            command,
                            MISSING_ENDCSNAME_DIAGNOSTIC,
                            format!("Missing {rendered} inserted"),
                            &[
                                "The control sequence marked <to be read again> should",
                                "not appear between \\csname and \\endcsname.",
                            ],
                        )?;
                        break;
                    }
                }
            }
            Ok(name)
        })();
        if let Ok(name) = &result {
            self.command
                .record_csname_buffer_usage(name.chars().count());
        }
        self.is_in_csname = previous;
        result
    }

    fn replay_expandafter_first(&mut self, command: CurrentCommand<G>) -> Result<(), CommandError> {
        self.conserve_input_stack_for_descendant()?;
        self.undo_alignment_delivery(&command);
        self.invalidate_delivery_freshness();
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::backed_up([BackedUpToken {
                spelling: command.spelling(),
            }]),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        if self.is_observed() {
            // TeX82 §25's `back_input` is part of the expandafter lifecycle:
            // after expanding its second token, the saved first token must be
            // a visible ordinary backup before raw delivery resumes.
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
        }
        Ok(())
    }
}
