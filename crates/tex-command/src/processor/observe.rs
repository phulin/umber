//! Observation entry points for canonical multi-record transitions.
//!
//! Observation vocabulary and construction compile unconditionally. At
//! runtime, [`CommandProcessor::is_observed`] enables construction when an
//! external observer is attached. [`CommandProcessor::observe`] delivers each
//! constructed record to that observer.
//!
//! These helpers keep transitions that publish multiple related records in
//! canonical order. They have one definition in every build; there is no
//! compile-time observation feature or alternate empty implementation.

use tex_state::token::Token;

use crate::command::CurrentCommand;
use crate::input::InputLevelId;

use super::CommandProcessor;

use tex_state::token::{OriginId, TracedTokenWord};

use crate::observation::{
    AlignmentRecord, CommandObservation, DiagnosticArgument, DiagnosticClass,
    DiagnosticLifecycleRecord, DiagnosticRecord, InputReason, InputRecord, InputTransition,
    RecoveryKind, RecoveryRecord,
};

impl<G> CommandProcessor<'_, '_, G> {
    /// Records a recovery level that replays one inserted token.
    ///
    /// TeX's trace names the input push and the recovery insertion separately,
    /// so both records are emitted here rather than folded into one.
    pub(crate) fn observe_inserted_token_recovery(&mut self, level: InputLevelId, token: Token) {
        if !self.is_observed() {
            return;
        }
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
            tokens: vec![self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN))],
        }));
    }

    /// Records TeX82 §1127's correction of an unbalanced alignment delimiter.
    ///
    /// `transition` names the brace the correction inserted; the pair of
    /// records is the insertion itself followed by the `align_state` it moved.
    pub(crate) fn observe_unbalanced_delimiter_correction(
        &mut self,
        transition: &'static str,
        previous: i32,
    ) {
        if !self.is_observed() {
            return;
        }
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition,
            alignment: self.command.alignment.active_alignment.map(|id| id.raw()),
            nesting: self.command.alignment_observation_nesting(),
            align_state: previous,
            delimiter: None,
            previous_align_state: None,
        }));
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition: "state_change",
            alignment: self.command.alignment.active_alignment.map(|id| id.raw()),
            nesting: self.command.alignment_observation_nesting(),
            align_state: self.command.alignment.align_state,
            delimiter: None,
            previous_align_state: Some(previous),
        }));
    }

    /// Records TeX82 §1127's correction of an inserted brace's backup,
    /// carrying the `align_state` the correction started from.
    pub(crate) fn observe_alignment_backup_correction(&mut self, previous: i32) {
        if !self.is_observed() {
            return;
        }
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition: "backup_correction",
            alignment: self.command.alignment.active_alignment.map(|id| id.raw()),
            nesting: self.command.alignment_observation_nesting(),
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
        if !self.is_observed() {
            return;
        }
        self.observe_immediate_write_retirement(level);
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Push,
            reason: InputReason::Write,
            source_name: None,
            source: None,
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
        if !announce || !self.is_observed() {
            return;
        }
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition: "state_change",
            alignment: self.command.alignment.active_alignment.map(|id| id.raw()),
            nesting: self.command.alignment_observation_nesting(),
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
        command: &CurrentCommand<G>,
    ) {
        if !self.is_observed() {
            return;
        }
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic,
            arguments: vec![DiagnosticArgument::Token(
                self.observed_command_spelling(command),
            )],
        }));
    }

    /// Compact counterpart used by recoverable expansion diagnostics whose
    /// only command argument is its delivered spelling.
    pub(crate) fn observe_hot_command_diagnostic(
        &mut self,
        diagnostic: &'static str,
        command: &crate::command::HotCommand<G>,
    ) {
        if !self.is_observed() {
            return;
        }
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic,
            arguments: vec![DiagnosticArgument::Token(
                self.observed_hot_command_spelling(command),
            )],
        }));
    }

    /// Publishes one source-located schema-v4 diagnostic report.
    pub(crate) fn observe_diagnostic_lifecycle(
        &mut self,
        class: DiagnosticClass,
        severity: &'static str,
        diagnostic: &'static str,
        arguments: Vec<DiagnosticArgument>,
    ) {
        if !self.is_observed() {
            return;
        }
        let Some(location) = self.command.last_diagnostic_location() else {
            return;
        };
        self.observe(CommandObservation::DiagnosticLifecycle(
            DiagnosticLifecycleRecord::Report {
                class,
                severity,
                diagnostic,
                arguments,
                location,
            },
        ));
    }
}
