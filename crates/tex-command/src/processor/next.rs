//! Canonical raw command delivery.
//!
//! This is the sole scalar path from input levels to `CurrentCommand`, after
//! TeX.web §343 (`get_next`).  Later scanner and alignment milestones extend
//! the two explicit entry points below; they do not add another lexical path.

use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::command::{CurrentCommand, DeliveryStamp};
use crate::error::CommandError;
use crate::input::{
    BackupTreatment, InputLevel, InputLevelId, InputRetirementAction, OutParameterReplay,
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenCursor, TokenPayload,
};
use crate::profile::{CharacterCode, CharacterMode};
use crate::{SourceControlSequenceKind, SourceToken, SourceTokenizationStep};

use super::CommandProcessor;
use super::status::{RecoveryContext, ScannerStatus};

const DEFAULT_END_LINE_CHAR: i32 = 13;

impl CommandProcessor<'_> {
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

        if self.rewind_current_token_cursor(stamp) {
            return Ok(());
        }

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
            self.alignment_delivery_entry(&command)?;
            return Ok(Some(command));
        }
    }

    fn take_input_token(&mut self) -> Result<Option<DeliveredToken>, CommandError> {
        loop {
            let Some(level) = self.command.input.levels.last().cloned() else {
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
                        SourceTokenizationStep::End => {
                            if self.retire_and_restart(identity)? {
                                return Ok(None);
                            }
                        }
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
                    if self.retire_and_restart(identity)? {
                        return Ok(None);
                    }
                }
            }
        }
    }

    fn retire_and_restart(&mut self, identity: InputLevelId) -> Result<bool, CommandError> {
        match self
            .command
            .retire_exhausted_input(identity)
            .map_err(|_| CommandError::InputInvariant)?
            .action
        {
            InputRetirementAction::TerminalStop => Ok(true),
            InputRetirementAction::VTemplateRetained => {
                // A retained v-template is intentionally left live for `do_endv`.
                // No later level may be read past it.
                Err(CommandError::InputInvariant)
            }
            InputRetirementAction::SourcePopped
            | InputRetirementAction::TokenListPopped
            | InputRetirementAction::ScantokensClosed
            | InputRetirementAction::VTemplatePopped => Ok(false),
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
        self.back_input(command.copy_for_backup())?;
        self.install_outer_recovery(recovery)?;
        command.recover_as_space();
        Ok(())
    }

    /// Recovers terminal input only while a scanner episode is live. The
    /// inserted tokens are then delivered through this same raw loop.
    fn recover_runaway_eof(&mut self) -> Result<bool, CommandError> {
        let recovery = self.command.scanner.recovery_context();
        if matches!(recovery.status, ScannerStatus::Normal) {
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

    fn alignment_delivery_entry(&mut self, command: &CurrentCommand) -> Result<(), CommandError> {
        match command.spelling().semantic_token() {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                self.command.alignment.align_state =
                    self.command.alignment.align_state.saturating_add(1);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                self.command.alignment.align_state =
                    self.command.alignment.align_state.saturating_sub(1);
            }
            _ => {}
        }
        Ok(())
    }

    fn undo_alignment_delivery(&mut self, command: &CurrentCommand) {
        match command.spelling().semantic_token() {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                self.command.alignment.align_state =
                    self.command.alignment.align_state.saturating_sub(1);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                self.command.alignment.align_state =
                    self.command.alignment.align_state.saturating_add(1);
            }
            _ => {}
        }
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
    use crate::processor::{
        AbsorbingContext, AlignmentId, AlignmentScanContext, ArgumentBuilderId, ConditionId,
        DefinitionContext, MatchingContext, ScannerWarning, SkippingContext, TokenBuilderId,
    };
    use crate::{
        CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState,
        RegisteredSourceKind, SourceRegistration,
    };

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
}
