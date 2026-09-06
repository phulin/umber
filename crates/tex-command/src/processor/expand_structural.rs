//! Structural TeX expansion primitives.

use tex_state::meaning::ExpandablePrimitive;
use tex_state::token::{OriginId, Token, TracedTokenWord};

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
        if second.as_ref().is_none_or(is_expandable_command) {
            self.request_expansion_into(&mut second, true)?;
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
    ) -> Result<String, CommandError> {
        // pdfTeX section 57 saves and restores the prior flag so nested name
        // scans remain true to ifincsname and unwind to their caller.
        let previous = std::mem::replace(&mut self.is_in_csname, true);
        let result = (|| {
            let mut destination = None;
            loop {
                let status = self.request_expanded_hot_token(&mut destination)?;
                match status {
                    DeliveryStatus::End => return Err(CommandError::input_invariant()),
                    DeliveryStatus::Command => {}
                    _ => unreachable!("ordinary expanded delivery returns only commands"),
                }
                let command = destination
                    .as_ref()
                    .expect("command status initializes destination");
                if command.command_word().expandable_primitive()
                    == Some(ExpandablePrimitive::EndCsName)
                {
                    break;
                }
                if let Some(ch) = command.character_token() {
                    name.push(ch);
                    destination.take();
                    continue;
                }
                let rendered = print_esc_text(self.state, "endcsname");
                let command = destination
                    .take()
                    .expect("csname recovery consumes the delivered command")
                    .materialize();
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
            Ok(name)
        })();
        if let Ok(name) = &result {
            self.command
                .record_csname_buffer_usage(name.chars().count());
        }
        self.is_in_csname = previous;
        result
    }

    /// TeX82 section 372's complete `\csname` expansion.  The character
    /// collector owns its local name and the ordinary expanded-token scanner
    /// owns every nested expansion; no caller or operand phase is retained in
    /// command scratch.
    pub(super) fn expand_csname(&mut self, opener: OriginId) -> Result<(), CommandError> {
        let name = self.scan_csname_characters(String::new())?;
        let symbol = self.state.intern_relaxed_control_sequence(&name);
        self.back_input_token(TracedTokenWord::pack(Token::Cs(symbol), opener))
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
