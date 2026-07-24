//! Canonical raw command delivery.
//!
//! This is the sole scalar path from input levels to `CurrentCommand`, after
//! TeX.web §343 (`get_next`).  Later scanner and alignment milestones extend
//! the two explicit entry points below; they do not add another lexical path.

use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::command::{CurrentCommand, DeliveryStamp};
use crate::error::CommandError;
use crate::input::{
    BackupTreatment, InputLevel, InputLevelId, InputRetirementAction, OutParameterReplay,
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenCursor, TokenPayload,
};
use crate::profile::{CharacterCode, CharacterMode};
use crate::{
    AlignmentDelivery, AlignmentDeliveryEvent, SourceControlSequenceKind, SourceToken,
    SourceTokenizationStep,
};

use super::CommandProcessor;
use super::status::{EofLegality, RecoveryContext, ScannerStatus};

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::{
    AlignmentRecord, CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation,
    CommandProvenance, InputRecord, InputTransition, RecoveryRecord, observed_token,
};

const DEFAULT_END_LINE_CHAR: i32 = 13;

impl CommandProcessor<'_> {
    /// Delivers one expanded command, separating an intercepted alignment
    /// delimiter from ordinary main-control delivery.
    ///
    /// No executor-side classifier is involved: `get_next` has already made
    /// the canonical `align_state` decision before this method observes the
    /// frozen `end_template` meaning.
    pub fn get_x_alignment_delivery(&mut self) -> Result<Option<AlignmentDelivery>, CommandError> {
        loop {
            let Some(command) = self.get_next()? else {
                return Ok(None);
            };
            if matches!(
                command.meaning(),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
            ) {
                return Ok(Some(AlignmentDelivery::Event(
                    AlignmentDeliveryEvent::EndTemplate(command),
                )));
            }
            if matches!(
                command.meaning(),
                Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_)
            ) {
                self.expand(command)?;
                continue;
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
        let AlignmentDeliveryEvent::EndTemplate(delimiter) = event;
        if self.last_delivery != Some(delimiter.delivery_stamp())
            || !matches!(
                delimiter.meaning(),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
            )
        {
            return Err(CommandError::StaleDelivery);
        }
        // Validate the lifecycle before changing the input stack, then make
        // the original delimiter the first token beneath the v-template.
        self.command
            .alignment
            .v_template(alignment)
            .map_err(|_| CommandError::InputInvariant)?;
        self.last_delivery = None;
        self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![delimiter.spelling()])),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        self.command
            .begin_alignment_v_template(alignment)
            .map_err(|_| CommandError::InputInvariant)?;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Alignment(AlignmentRecord {
            transition: "begin_v_template",
            alignment: Some(alignment.raw()),
            align_state: self.command.alignment.align_state,
        }));
        Ok(())
    }

    /// Delivers one unexpanded raw command through canonical `get_next`.
    pub fn get_next(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.last_delivery = None;
        self.get_next_with_control_sequence_creation(false)
    }

    /// Delivers one raw token for consumers which canonically permit a new
    /// control-sequence spelling. The present interner records a spelling
    /// without assigning it a meaning, so the policy boundary is explicit
    /// even before diagnostic-only interning is separated further.
    pub fn get_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.last_delivery = None;
        self.get_next_with_control_sequence_creation(true)
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

    /// Restores a command and records the diagnostic selected by `back_error`.
    ///
    /// Scanner-status recovery supplies the canonical diagnostic identity in a
    /// later milestone; keeping its accounting here ensures recovery input
    /// remains ordinary input after the one backup transition.
    #[allow(dead_code)] // invoked by scanner-status recovery in the next milestone
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
        self.last_delivery = None;
        self.undo_alignment_delivery(&command);

        // Ordinary `back_input` may restore a live token cursor in place.
        // `\\noexpand`, however, associates a one-delivery treatment with the
        // replayed level, so it must retain an explicit backed-up level even
        // when the original cursor could otherwise be rewound.
        if matches!(treatment, BackupTreatment::Ordinary) && self.rewind_current_token_cursor(stamp)
        {
            #[cfg(any(test, feature = "instrumentation"))]
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Backup,
                level: stamp.input_level(),
                position: stamp.position(),
            }));
            return Ok(());
        }

        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Recovery(RecoveryRecord {
            backup: true,
            tokens: vec![self.observed_token(command.spelling())],
        }));
        self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![command.spelling()])),
            TokenBehavior::BackedUp(treatment),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        Ok(())
    }

    fn get_next_with_control_sequence_creation(
        &mut self,
        _allow_control_sequence_creation: bool,
    ) -> Result<Option<CurrentCommand>, CommandError> {
        loop {
            let Some(delivery) = self.take_input_token()? else {
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
            } = delivery;

            if let Token::Param(slot) = spelling.semantic_token()
                && matches!(
                    self.command
                        .replay_out_parameter(level, slot)
                        .map_err(|_| CommandError::InputInvariant)?,
                    OutParameterReplay::Pushed(_)
                )
            {
                continue;
            }

            let delivery_stamp = DeliveryStamp::new(level.0, position, self.next_delivery_sequence);
            self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
            let mut command = CurrentCommand::resolve(spelling, delivery_stamp, &mut self.state);
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
            let adjustment = self.command.alignment.classify_delivery(&mut command);
            command.set_alignment_adjustment(adjustment);
            #[cfg(any(test, feature = "instrumentation"))]
            if !matches!(
                adjustment,
                crate::processor::AlignmentDeliveryAdjustment::None
            ) {
                self.observe(CommandObservation::Alignment(AlignmentRecord {
                    transition: match adjustment {
                        crate::processor::AlignmentDeliveryAdjustment::BeginGroup => "begin_group",
                        crate::processor::AlignmentDeliveryAdjustment::EndGroup => "end_group",
                        crate::processor::AlignmentDeliveryAdjustment::Delimiter => "delimiter",
                        crate::processor::AlignmentDeliveryAdjustment::None => unreachable!(),
                    },
                    alignment: self
                        .command
                        .alignment
                        .active_alignment
                        .map(|identity| identity.raw()),
                    align_state: self.command.alignment.align_state,
                }));
            }
            #[cfg(any(test, feature = "instrumentation"))]
            self.observe_raw_delivery(&command);
            return Ok(Some(command));
        }
    }

    fn take_input_token(&mut self) -> Result<Option<DeliveredToken>, CommandError> {
        loop {
            let Some(level) = self.command.input.levels.last().cloned() else {
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Input(InputRecord {
                    transition: InputTransition::Stop,
                    level: 0,
                    position: 0,
                }));
                return Ok(None);
            };
            match level {
                InputLevel::Source(source) => {
                    let identity = source.identity;
                    let position = source.cursor.next_physical_offset;
                    match self.next_source_step() {
                        SourceTokenizationStep::Token(token) => {
                            let spelling = self.source_spelling(token);
                            return Ok(Some(DeliveredToken {
                                spelling,
                                level: identity,
                                position,
                                behavior: TokenBehavior::Ordinary,
                            }));
                        }
                        SourceTokenizationStep::InvalidCharacter(_) => continue,
                        SourceTokenizationStep::End => match self.retire_and_restart(identity)? {
                            RetirementRestart::Stop => return Ok(None),
                            RetirementRestart::Continue => {}
                            RetirementRestart::EndV(_) => return Err(CommandError::InputInvariant),
                        },
                    }
                }
                InputLevel::Tokens(cursor) => {
                    let identity = cursor.identity;
                    if let Some((spelling, position, behavior)) = self.next_stored_token(&cursor) {
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
                        }));
                    }
                    match self.retire_and_restart(identity)? {
                        RetirementRestart::Stop => return Ok(None),
                        RetirementRestart::Continue => {}
                        RetirementRestart::EndV(level) => {
                            return Ok(Some(DeliveredToken {
                                spelling: TracedTokenWord::pack(
                                    self.state.frozen_endv_token(),
                                    OriginId::UNKNOWN,
                                ),
                                level,
                                position: u64::try_from(cursor.index)
                                    .map_err(|_| CommandError::InputInvariant)?,
                                behavior: TokenBehavior::VTemplate,
                            }));
                        }
                    }
                }
            }
        }
    }

    fn retire_and_restart(
        &mut self,
        identity: InputLevelId,
    ) -> Result<RetirementRestart, CommandError> {
        let action = self
            .command
            .retire_exhausted_input(identity)
            .map_err(|_| CommandError::InputInvariant)?
            .action;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Input(InputRecord {
            transition: if matches!(action, InputRetirementAction::TerminalStop) {
                InputTransition::Stop
            } else {
                InputTransition::Retire
            },
            level: identity.0,
            position: 0,
        }));
        match action {
            InputRetirementAction::TerminalStop => Ok(RetirementRestart::Stop),
            InputRetirementAction::VTemplateRetained => {
                // End-v is synthesized only after the exact v-template frame
                // becomes retained. `do_endv` later pops that same frame.
                Ok(RetirementRestart::EndV(identity))
            }
            InputRetirementAction::SourcePopped
            | InputRetirementAction::TokenListPopped
            | InputRetirementAction::ScantokensClosed
            | InputRetirementAction::VTemplatePopped => {
                self.command.alignment.finish_u_template(identity);
                Ok(RetirementRestart::Continue)
            }
        }
    }

    fn next_source_step(&mut self) -> SourceTokenizationStep {
        let profile = self.command.profile();
        let catcode = |code: CharacterCode| self.state.catcode(character_from_code(code));
        match profile.character_mode() {
            CharacterMode::EightBitExact => self
                .command
                .next_exact_source_step(DEFAULT_END_LINE_CHAR, catcode),
            CharacterMode::UnicodeExtended => self
                .command
                .next_unicode_source_step(DEFAULT_END_LINE_CHAR, catcode),
        }
    }

    fn source_spelling(&mut self, token: SourceToken) -> TracedTokenWord {
        let token = match token {
            SourceToken::Character { code, catcode, .. } => Token::Char {
                ch: character_from_code(code),
                cat: catcode,
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
                    let name: String = name.into_iter().map(character_from_code).collect();
                    Token::Cs(self.state.intern_control_sequence(&name))
                }
            },
        };
        // Source-range provenance is installed with the source-map integration
        // milestone; production delivery still carries the mandatory unknown
        // origin rather than admitting an untraced token representation.
        TracedTokenWord::pack(token, OriginId::UNKNOWN)
    }

    fn next_stored_token(
        &self,
        cursor: &TokenCursor,
    ) -> Option<(TracedTokenWord, u64, TokenBehavior)> {
        let position = u64::try_from(cursor.index).ok()?;
        let spelling = match &cursor.payload {
            TokenPayload::Transient(buffer) => buffer.get(cursor.index),
            TokenPayload::ArgumentRange { buffer, range } => {
                buffer.get(range.start() + cursor.index)
            }
            TokenPayload::Stored { tokens, origins } => {
                let token = *self.state.tokens(*tokens).get(cursor.index)?;
                let origin = self
                    .state
                    .origin_list(*origins)
                    .get(cursor.index)
                    .copied()
                    .unwrap_or(OriginId::UNKNOWN);
                Some(TracedTokenWord::pack(token, origin))
            }
        }?;
        Some((spelling, position, cursor.behavior.clone()))
    }

    fn check_outer_validity_entry(
        &mut self,
        command: &mut CurrentCommand,
    ) -> Result<(), CommandError> {
        if !command.is_outer() || matches!(self.command.scanner.status(), ScannerStatus::Normal) {
            return Ok(());
        }

        let recovery = self.command.scanner.recovery_context();
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
        self.install_outer_recovery(recovery)?;
        Ok(true)
    }

    /// TeX.web's `check_outer_validity` recovery table. Primitive insertions
    /// are frozen tokens, retaining their original meanings if user code has
    /// reassigned their visible spellings.
    fn install_outer_recovery(&mut self, recovery: RecoveryContext) -> Result<(), CommandError> {
        let RecoveryContext { status, warning } = recovery;
        let tokens = match status {
            ScannerStatus::Normal => return Ok(()),
            ScannerStatus::Skipping(_) => vec![self.frozen_primitive("fi")?],
            ScannerStatus::Defining(_) | ScannerStatus::Absorbing(_) => vec![right_brace()],
            ScannerStatus::Matching(_) => vec![self.frozen_primitive("par")?],
            ScannerStatus::Aligning(_) => vec![self.frozen_primitive("cr")?, right_brace()],
        };
        self.command.scanner.clear_for_recovery();
        if let Some(warning) = warning {
            self.command.expansion.pending_diagnostics.push(warning.0);
        }
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Recovery(RecoveryRecord {
            backup: false,
            tokens: tokens
                .iter()
                .copied()
                .map(|token| self.observed_token(token))
                .collect(),
        }));
        self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(tokens)),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Transient(crate::input::TransientReplayReason::Inserted),
        );
        Ok(())
    }

    fn frozen_primitive(&self, name: &str) -> Result<TracedTokenWord, CommandError> {
        let token = self
            .state
            .primitive_token(name)
            .ok_or(CommandError::InputInvariant)?;
        Ok(TracedTokenWord::pack(token, OriginId::UNKNOWN))
    }

    #[cfg(any(test, feature = "instrumentation"))]
    pub(crate) fn observed_token(
        &self,
        token: TracedTokenWord,
    ) -> crate::observation::ObservedToken {
        observed_token(token, |symbol| self.state.resolve(symbol).to_owned())
    }

    #[cfg(any(test, feature = "instrumentation"))]
    fn observe_raw_delivery(&mut self, command: &CurrentCommand) {
        let command_name = match command.meaning() {
            Meaning::CharToken { .. } => "character".to_owned(),
            Meaning::Macro { .. } => "macro".to_owned(),
            Meaning::ExpandablePrimitive(_) => "expandable".to_owned(),
            Meaning::UnexpandablePrimitive(_) => "unexpandable".to_owned(),
            _ => "internal".to_owned(),
        };
        let spelling = self.observed_token(command.spelling());
        self.observe(CommandObservation::Command(CommandDeliveryRecord {
            boundary: CommandDeliveryBoundary::Raw,
            spelling,
            command: command_name,
            provenance: CommandProvenance::from_command(command),
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

    fn rewind_current_token_cursor(&mut self, stamp: DeliveryStamp) -> bool {
        let Some(InputLevel::Tokens(cursor)) = self.command.input.levels.last_mut() else {
            return false;
        };
        if cursor.identity.0 != stamp.input_level()
            || u64::try_from(cursor.index).ok() != Some(stamp.position().saturating_add(1))
        {
            return false;
        }
        cursor.index -= 1;
        true
    }
}

struct DeliveredToken {
    spelling: TracedTokenWord,
    level: InputLevelId,
    position: u64,
    behavior: TokenBehavior,
}

enum RetirementRestart {
    Stop,
    Continue,
    EndV(InputLevelId),
}

fn character_from_code(code: CharacterCode) -> char {
    match code.to_byte() {
        Ok(byte) => char::from(byte),
        Err(_) => code
            .to_char()
            .expect("registered Unicode source supplies valid scalars"),
    }
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
        let mut universe = Universe::new();
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
        assert_eq!(processor.command.alignment.align_state, 1);
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
        assert_eq!(processor.command.alignment.align_state, 0);
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
        let mut universe = Universe::new();
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
        let mut observed_universe = Universe::new();
        let mut unobserved_universe = Universe::new();
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
        let mut universe = Universe::new();
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
        let mut universe = Universe::new();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let opening = processor
            .get_token()
            .expect("opening brace delivers")
            .expect("source is live");
        let original_stamp = opening.delivery_stamp();
        assert_eq!(processor.command.alignment.align_state, 1);
        processor
            .back_input(opening)
            .expect("exact delivery backs up");
        assert_eq!(processor.command.alignment.align_state, 0);

        let replayed = processor
            .get_next()
            .expect("backed-up brace delivers")
            .expect("backup is live");
        assert_eq!(replayed.delivery_stamp().position(), 0);
        assert_ne!(replayed.delivery_stamp(), original_stamp);
        assert_eq!(processor.command.alignment.align_state, 1);
        processor
            .back_input(replayed)
            .expect("replayed delivery backs up");
        assert_eq!(processor.command.alignment.align_state, 0);
    }

    #[test]
    fn token_level_backup_rewinds_without_an_extra_input_level() {
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
        let mut universe = Universe::new();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let first = processor
            .get_next()
            .expect("token delivers")
            .expect("token level is live");
        processor.back_input(first).expect("token cursor rewinds");
        assert_eq!(processor.command.input.levels.len(), 1);
        let replayed = processor
            .get_next()
            .expect("rewound token delivers")
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
    fn get_next_alone_intercepts_top_level_alignment_delimiters_and_backup_replays_them() {
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
        let mut universe = Universe::new();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let delimiter = processor
            .get_next()
            .expect("delimiter delivers")
            .expect("input is live");
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
                .get_next()
                .expect("delimiter replays")
                .expect("backup is live")
                .meaning(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
        ));
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
        let mut universe = Universe::new();
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
                .get_next()
                .expect("top-level tab delivers")
                .expect("input is live")
                .meaning(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
        ));
    }

    #[test]
    fn alignment_templates_deliver_through_input_and_retire_before_delimiter_replay() {
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
        let mut universe = Universe::new();
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
        let snapshot = command.snapshot();
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

            let u = processor
                .get_next()
                .expect("u-template delivers")
                .expect("u-template token");
            assert!(matches!(u.meaning(), Meaning::CharToken { ch: 'u', .. }));
            let end_template = match processor
                .get_x_alignment_delivery()
                .expect("delimiter follows exhausted u-template")
                .expect("intercepted delimiter")
            {
                crate::AlignmentDelivery::Event(crate::AlignmentDeliveryEvent::EndTemplate(
                    event,
                )) => crate::AlignmentDeliveryEvent::EndTemplate(event),
                crate::AlignmentDelivery::Command(_) => {
                    panic!("delimiter is delivered as an alignment event")
                }
            };
            processor
                .begin_alignment_v_template(alignment, end_template)
                .expect("delimiter is backed up below v-template input");
            let v = processor
                .get_next()
                .expect("v-template delivers")
                .expect("v-template token");
            assert!(matches!(v.meaning(), Meaning::CharToken { ch: 'v', .. }));
            let endv = processor
                .get_next()
                .expect("retained v-template emits end-v")
                .expect("frozen end-v");
            assert!(endv.spelling().semantic_token().is_frozen_endv());
            processor
                .command
                .finish_alignment_cell(alignment)
                .expect("do_endv retires the exact retained frame once");
            let delimiter = processor
                .get_next()
                .expect("backed-up delimiter replays")
                .expect("delimiter is live");
            assert!(matches!(
                delimiter.meaning(),
                Meaning::CharToken {
                    cat: Catcode::AlignmentTab,
                    ..
                }
            ));
        }
        command
            .rollback(snapshot.clone())
            .expect("template input rolls back exactly");
        assert_eq!(command.snapshot(), snapshot);
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
        let mut universe = Universe::new();
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
        assert_eq!(processor.command.alignment.align_state, 1);
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
        let mut universe = Universe::new();
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
        let mut universe = Universe::new();
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
        assert_eq!(processor.command.input.levels.len(), 2);
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
        let mut universe = Universe::new();
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
        let mut universe = Universe::new();
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
        let mut universe = Universe::new();
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
