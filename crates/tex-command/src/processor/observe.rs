//! Observation entry points whose callers must not know they are optional.
//!
//! `crate::observation` compiles only under `cfg(any(test, feature =
//! "instrumentation"))`. A transition whose sole consumer is an observer would
//! therefore have to assemble its values inside a `cfg` block at the call
//! site, which leaves every binding that feeds it unused in the resolution a
//! shipped binary is actually built in -- the one `cargo build -p umber`
//! selects (`umber2-johp.200`).
//!
//! Each entry point below is defined twice against one signature: once
//! building the record, and once with an empty body. Call sites stay
//! unconditional, so the values they compute are consumed in every feature
//! resolution, while the shipping build still compiles the observation, its
//! record allocation, and its spelling resolution away to nothing.

use tex_state::token::Token;

use crate::command::CurrentCommand;
use crate::input::InputLevelId;

use super::CommandProcessor;

#[cfg(any(test, feature = "instrumentation"))]
use tex_state::token::{OriginId, TracedTokenWord};

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::{
    AlignmentRecord, CommandObservation, DiagnosticArgument, DiagnosticRecord, InputReason,
    InputRecord, InputTransition, RecoveryKind, RecoveryRecord,
};

#[cfg(any(test, feature = "instrumentation"))]
impl CommandProcessor<'_> {
    /// Records a recovery level that replays one inserted token.
    ///
    /// TeX's trace names the input push and the recovery insertion separately,
    /// so both records are emitted here rather than folded into one.
    pub(crate) fn observe_inserted_token_recovery(&mut self, level: InputLevelId, token: Token) {
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Recovery,
            reason: InputReason::Recovery,
            source_name: None,
            level: level.0,
            position: 0,
        }));
        self.observe(CommandObservation::Recovery(RecoveryRecord {
            kind: RecoveryKind::InsertedToken,
            tokens: vec![self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN))],
        }));
    }

    /// Records TeX82 §1132's correction of an unbalanced alignment delimiter.
    ///
    /// `transition` names the brace the correction inserted; the pair of
    /// records is the insertion itself followed by the `align_state` it moved.
    pub(crate) fn observe_unbalanced_delimiter_correction(
        &mut self,
        transition: &'static str,
        previous: i32,
    ) {
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition,
            alignment: self.command.alignment.active_alignment.map(|id| id.raw()),
            align_state: previous,
            delimiter: None,
            previous_align_state: None,
        }));
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition: "state_change",
            alignment: self.command.alignment.active_alignment.map(|id| id.raw()),
            align_state: self.command.alignment.align_state,
            delimiter: None,
            previous_align_state: Some(previous),
        }));
    }

    /// Records TeX82 §1132's correction of an inserted brace's backup,
    /// carrying the `align_state` the correction started from.
    pub(crate) fn observe_alignment_backup_correction(&mut self, previous: i32) {
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition: "backup_correction",
            alignment: self.command.alignment.active_alignment.map(|id| id.raw()),
            align_state: self.command.alignment.align_state,
            delimiter: None,
            previous_align_state: Some(previous),
        }));
    }

    /// Registers TeX82 §53's write-list replay level and records its push.
    ///
    /// §53 names this artificial replay as a write input lifetime. Keeping the
    /// classification at the scanner/control seam is what lets the raw
    /// delivery loop stay free of the level's trace.
    pub(crate) fn observe_write_list_push(&mut self, level: InputLevelId) {
        self.observe_immediate_write_retirement(level);
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Push,
            reason: InputReason::Write,
            source_name: None,
            level: level.0,
            position: 0,
        }));
    }

    /// Records TeX82 §37's `align_peek` sentinel assignment.
    ///
    /// TeX82 §785 executes the assignment at the `restart` label on every
    /// pass, including after an ignored `\crcr`, so an idempotent assignment
    /// remains a canonical transition.
    pub(crate) fn observe_alignment_peek_sentinel(&mut self, announce: bool) {
        if !announce {
            return;
        }
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition: "state_change",
            alignment: self.command.alignment.active_alignment.map(|id| id.raw()),
            align_state: self.command.alignment.align_state,
            delimiter: None,
            previous_align_state: None,
        }));
    }

    /// Records an error diagnostic whose only argument is the spelling of the
    /// command that provoked it.
    pub(crate) fn observe_command_diagnostic(
        &mut self,
        diagnostic: &'static str,
        command: &CurrentCommand,
    ) {
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic,
            arguments: vec![DiagnosticArgument::Token(
                self.observed_command_spelling(command),
            )],
        }));
    }
}

/// The shipping definitions: the same signatures, and no observation.
#[cfg(not(any(test, feature = "instrumentation")))]
impl CommandProcessor<'_> {
    pub(crate) fn observe_inserted_token_recovery(&mut self, _level: InputLevelId, _token: Token) {}

    pub(crate) fn observe_unbalanced_delimiter_correction(
        &mut self,
        _transition: &'static str,
        _previous: i32,
    ) {
    }

    pub(crate) fn observe_alignment_backup_correction(&mut self, _previous: i32) {}

    pub(crate) fn observe_write_list_push(&mut self, _level: InputLevelId) {}

    pub(crate) fn observe_alignment_peek_sentinel(&mut self, _announce: bool) {}

    pub(crate) fn observe_command_diagnostic(
        &mut self,
        _diagnostic: &'static str,
        _command: &CurrentCommand,
    ) {
    }
}
