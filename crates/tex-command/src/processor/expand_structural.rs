//! Structural TeX expansion primitives.

use tex_state::meaning::{ExpandablePrimitive, Meaning, ResolvedMeaning};
use tex_state::token::{Token, TracedTokenWord};

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
            if let Err(error) = self.expand_into(&mut second, None, true) {
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

    /// TeX.web's `\\csname`: collect ordinary expanded character commands
    /// until the inaccessible `\\endcsname` boundary, then inject the one
    /// named control-sequence token through normal input delivery.
    pub(super) fn expand_csname(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let name = match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => String::new(),
            crate::state::PendingExpansionResume::CsName { name } => name,
            _ => return Err(CommandError::input_invariant()),
        };
        let mut suspended_name = None;
        let name = match self.scan_csname_characters(name, &mut suspended_name) {
            Ok(name) => name,
            Err(error) => {
                if error.is_resource_suspension() {
                    *suspended = suspended_name
                        .map(|name| crate::state::PendingExpansionResume::CsName { name });
                }
                return Err(error);
            }
        };
        let symbol = self.state.intern_relaxed_control_sequence(&name);
        self.back_input_token(TracedTokenWord::pack(Token::Cs(symbol), opener.origin()))
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
                let status = match self.get_x_token_into(&mut destination) {
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
