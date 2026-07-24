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
    TokenBehavior, TokenCursor, TokenPayload,
};
use crate::profile::{CharacterCode, CharacterMode};
use crate::{SourceControlSequenceKind, SourceToken, SourceTokenizationStep};

use super::CommandProcessor;

const DEFAULT_END_LINE_CHAR: i32 = 13;

impl CommandProcessor<'_> {
    /// Delivers one unexpanded raw command through canonical `get_next`.
    pub fn get_next(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.get_next_with_control_sequence_creation(false)
    }

    /// Delivers one raw token for consumers which canonically permit a new
    /// control-sequence spelling. The present interner records a spelling
    /// without assigning it a meaning, so the policy boundary is explicit
    /// even before diagnostic-only interning is separated further.
    pub fn get_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.get_next_with_control_sequence_creation(true)
    }

    fn get_next_with_control_sequence_creation(
        &mut self,
        _allow_control_sequence_creation: bool,
    ) -> Result<Option<CurrentCommand>, CommandError> {
        loop {
            let Some(delivery) = self.take_input_token()? else {
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

            let mut command = CurrentCommand::resolve(
                spelling,
                DeliveryStamp::new(level.0, position),
                &mut self.state,
            );
            if matches!(
                behavior,
                TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence)
            ) {
                command.suppress_expandable();
            }
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
        _command: &mut CurrentCommand,
    ) -> Result<(), CommandError> {
        // The status-specific recovery sequence is installed by the scanner
        // milestone. Keeping this call at raw delivery prevents a second path
        // when that state machine is added.
        let _status = &self.command.scanner.status;
        Ok(())
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tex_state::Universe;
    use tex_state::token::{OriginId, Token, TracedTokenWord};

    use super::*;
    use crate::input::{ReplayTrace, RetirementBehavior};
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
}
