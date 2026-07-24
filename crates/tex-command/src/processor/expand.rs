//! Ordinary expanded-command delivery.

use tex_state::env::banks::{IntParam, TokParam};
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::page::PageMark;
use tex_state::provenance::SynthesizedOriginKind;
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::input::{
    BackupTreatment, InputLevelId, ReplayTrace, RetirementBehavior, SharedTokenBuffer,
    TokenBehavior, TokenPayload,
};
use crate::macro_call::MacroArguments;
use crate::profile::CommandProfile;
use crate::{CommandError, CurrentCommand};

use super::CommandProcessor;

/// Stable pending-diagnostic identity for TeX.web's `Missing \\endcsname
/// inserted` recovery. Rendering belongs to the diagnostic milestone.
const MISSING_ENDCSNAME_DIAGNOSTIC: u64 = 0x6373_6e61_6d65_0001;

impl CommandProcessor<'_> {
    /// Delivers one ordinary expanded command through TeX.web's `get_x_token`.
    ///
    /// This is the sole production expanded loop. Expansion mutates the
    /// canonical command state and restarts here; it never returns a
    /// push-bearing dispatch result or enters a second interpreter.
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.command.transient.active_expansion_depth += 1;
        let result = self.get_x_token_scalar();
        self.command.transient.active_expansion_depth -= 1;
        result
    }

    fn get_x_token_scalar(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        loop {
            let Some(command) = self.get_next()? else {
                return Ok(None);
            };
            if !is_expandable(command.meaning()) {
                return Ok(Some(command));
            }
            self.expand(command)?;
        }
    }

    /// TeX.web's scalar `expand`: each case changes the active input/state
    /// directly, then returns to [`Self::get_x_token_scalar`].
    fn expand(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
        self.command.expansion.cumulative_expansions =
            self.command.expansion.cumulative_expansions.wrapping_add(1);
        match command.meaning() {
            Meaning::Macro { .. } => {
                self.macro_call(command)?;
                Ok(())
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => self.expand_noexpand(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
                self.expand_expandafter()
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName) => {
                self.expand_csname(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::String) => {
                self.expand_string(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning) => {
                self.expand_meaning(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number) => {
                self.expand_number(command, false)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::RomanNumeral) => {
                self.expand_number(command, true)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The) => self.expand_the(command),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName) => {
                self.expand_fontname(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input) => self.expand_input(command),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput) => self.expand_endinput(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::JobName) => {
                let job_name = self.host.job_name().to_owned();
                self.push_rendered_text(&job_name, command.origin());
                Ok(())
            }
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::TopMark
                | ExpandablePrimitive::FirstMark
                | ExpandablePrimitive::BotMark
                | ExpandablePrimitive::SplitFirstMark
                | ExpandablePrimitive::SplitBotMark),
            ) => self.expand_mark(primitive),
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::TopMarks
                | ExpandablePrimitive::FirstMarks
                | ExpandablePrimitive::BotMarks
                | ExpandablePrimitive::SplitFirstMarks
                | ExpandablePrimitive::SplitBotMarks),
            ) => self.expand_mark_class(primitive),
            Meaning::ExpandablePrimitive(primitive) => {
                Err(CommandError::UnsupportedExpandablePrimitive(primitive))
            }
            _ => Err(CommandError::InputInvariant),
        }
    }

    /// TeX.web's `\noexpand`: read normally, then replay exactly one target
    /// from a backed-up level carrying the non-sticky suppression treatment.
    fn expand_noexpand(&mut self) -> Result<(), CommandError> {
        let target = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        self.back_input_with_treatment(target, BackupTreatment::SuppressExpandableControlSequence)
    }

    /// TeX.web's `\expandafter`: preserve the first token, expand (or back
    /// up) the second token, then put the first token above the resulting
    /// input. The first delivery is intentionally replayed through an
    /// explicit backed-up level because it is no longer the latest delivery.
    fn expand_expandafter(&mut self) -> Result<(), CommandError> {
        let first = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        let second = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        if is_expandable(second.meaning()) {
            self.expand(second)?;
        } else {
            self.back_input(second)?;
        }
        self.replay_expandafter_first(first);
        Ok(())
    }

    /// TeX.web's `\\csname`: collect ordinary expanded character commands
    /// until the inaccessible `\\endcsname` boundary, then inject the one
    /// named control-sequence token through normal input delivery.
    fn expand_csname(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let mut name = String::new();
        loop {
            let Some(command) = self.get_x_token()? else {
                return Err(CommandError::InputInvariant);
            };
            match command.meaning() {
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => break,
                Meaning::CharToken { ch, .. } => name.push(ch),
                _ => {
                    self.back_error(command, MISSING_ENDCSNAME_DIAGNOSTIC)?;
                    break;
                }
            }
        }

        let symbol = self.state.intern_relaxed_control_sequence(&name);
        let origin = self
            .state
            .synthesized_origin(SynthesizedOriginKind::Expansion, opener.origin());
        self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                Token::Cs(symbol),
                origin,
            )])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Transient(crate::input::TransientReplayReason::Inserted),
        );
        Ok(())
    }

    /// `\\string` observes spelling, never an effective control-sequence meaning.
    fn expand_string(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let target = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        self.push_rendered_text(
            &string_text(&self.state, target.spelling().semantic_token()),
            opener.origin(),
        );
        Ok(())
    }

    fn expand_meaning(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let target = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        self.push_rendered_text(&meaning_text(&self.state, &target), opener.origin());
        Ok(())
    }

    fn expand_number(&mut self, opener: CurrentCommand, roman: bool) -> Result<(), CommandError> {
        let value = self.scan_decimal_integer()?;
        let text = if roman {
            roman_numeral(value)
        } else {
            value.to_string()
        };
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    /// Direct token-list `\\the` splicing has no recursive expansion and only
    /// consumes its target command; scanner collection owns any later expansion.
    fn expand_the(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let target = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        match target.meaning() {
            Meaning::ToksRegister(index) => {
                self.push_the_tokens(self.state.toks(index), index, false)
            }
            Meaning::TokParam(index) => {
                self.push_the_tokens(self.state.tok_param(TokParam::new(index)), index, true)
            }
            Meaning::CountRegister(index) => {
                self.push_rendered_text(&self.state.count(index).to_string(), opener.origin());
                Ok(())
            }
            Meaning::IntParam(index) => {
                self.push_rendered_text(
                    &self.state.int_param(IntParam::new(index)).to_string(),
                    opener.origin(),
                );
                Ok(())
            }
            _ => {
                self.push_rendered_text("0", opener.origin());
                Ok(())
            }
        }
    }

    fn expand_fontname(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let target = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        let Meaning::Font(font) = target.meaning() else {
            return Err(CommandError::InputInvariant);
        };
        self.push_rendered_text(&self.state.font_name(font), opener.origin());
        Ok(())
    }

    fn expand_input(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let name = self.scan_input_name()?;
        let source = self.host.input(&name).ok_or(CommandError::MissingInput)?;
        let id = self
            .command
            .register_source(source)
            .map_err(|_| CommandError::InputInvariant)?;
        self.command
            .open_registered_source(id)
            .map_err(|_| CommandError::InputInvariant)?;
        let _ = opener;
        Ok(())
    }

    fn expand_endinput(&mut self) -> Result<(), CommandError> {
        self.command
            .end_current_source_after_current_line()
            .then_some(())
            .ok_or(CommandError::InputInvariant)
    }

    fn expand_mark(&mut self, primitive: ExpandablePrimitive) -> Result<(), CommandError> {
        self.push_the_tokens(self.state.page_mark(page_mark(primitive)), 0, false)
    }

    fn expand_mark_class(&mut self, primitive: ExpandablePrimitive) -> Result<(), CommandError> {
        let class = self.scan_decimal_integer()?;
        let class = u16::try_from(class).unwrap_or(0);
        self.push_the_tokens(
            self.state.page_mark_class(page_mark(primitive), class),
            class,
            false,
        )
    }

    /// TeX's `scan_file_name`, restricted to the command processor so the
    /// helper cannot reach input-opening authority.
    fn scan_input_name(&mut self) -> Result<String, CommandError> {
        let first = loop {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    ch: ' ',
                    cat: tex_state::token::Catcode::Space
                }
            ) {
                break command;
            }
        };
        let grouped = matches!(
            first.meaning(),
            Meaning::CharToken {
                cat: tex_state::token::Catcode::BeginGroup,
                ..
            }
        );
        let mut name = String::new();
        let mut quoted = false;
        let mut next = if grouped { None } else { Some(first) };
        loop {
            let command = match next.take() {
                Some(command) => command,
                None => self.get_x_token()?.ok_or(CommandError::InputInvariant)?,
            };
            match command.meaning() {
                Meaning::CharToken { ch: '"', .. } => quoted = !quoted,
                Meaning::CharToken {
                    cat: tex_state::token::Catcode::EndGroup,
                    ..
                } if grouped && !quoted => break,
                Meaning::CharToken {
                    ch: ' ',
                    cat: tex_state::token::Catcode::Space,
                } if !grouped && !quoted => break,
                Meaning::CharToken { ch, .. } => name.push(ch),
                _ if !grouped => {
                    self.back_input(command)?;
                    break;
                }
                _ => return Err(CommandError::InputInvariant),
            }
        }
        if name.is_empty() {
            Err(CommandError::InputInvariant)
        } else {
            Ok(name)
        }
    }

    fn push_the_tokens(
        &mut self,
        tokens: TokenListId,
        index: u16,
        parameter: bool,
    ) -> Result<(), CommandError> {
        self.command.push_token_level(
            TokenPayload::Stored {
                tokens,
                origins: OriginListId::EMPTY,
            },
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(if parameter {
                crate::input::StoredReplayReason::TokenParameter(index)
            } else {
                crate::input::StoredReplayReason::TokenRegister(index)
            }),
        );
        Ok(())
    }

    fn scan_decimal_integer(&mut self) -> Result<i32, CommandError> {
        let mut sign = 1_i32;
        let mut value = 0_i32;
        let mut saw_digit = false;
        loop {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            match command.meaning() {
                Meaning::CharToken {
                    ch: ' ',
                    cat: tex_state::token::Catcode::Space,
                } if !saw_digit => {}
                Meaning::CharToken { ch: '+', .. } if !saw_digit => {}
                Meaning::CharToken { ch: '-', .. } if !saw_digit => sign = -sign,
                Meaning::CharToken { ch, .. } if ch.is_ascii_digit() => {
                    saw_digit = true;
                    value = value
                        .saturating_mul(10)
                        .saturating_add(i32::from(ch as u8 - b'0'));
                }
                _ => {
                    self.back_input(command)?;
                    break;
                }
            }
        }
        Ok(sign.saturating_mul(value))
    }

    fn push_rendered_text(&mut self, text: &str, parent: OriginId) {
        let origin = self
            .state
            .synthesized_origin(SynthesizedOriginKind::ValueRendering, parent);
        let tokens = text
            .chars()
            .map(|ch| {
                TracedTokenWord::pack(
                    Token::Char {
                        ch,
                        cat: if ch == ' ' {
                            tex_state::token::Catcode::Space
                        } else {
                            tex_state::token::Catcode::Other
                        },
                    },
                    origin,
                )
            })
            .collect::<Vec<_>>();
        self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(tokens)),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Transient(crate::input::TransientReplayReason::Inserted),
        );
    }

    fn replay_expandafter_first(&mut self, command: CurrentCommand) {
        self.undo_alignment_delivery(&command);
        self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![command.spelling()])),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
    }

    /// Creates one invocation provenance node and atomically exposes its
    /// activation/body ownership pair to the input stack.
    ///
    /// The scalar macro matcher owns argument matching and calls this only
    /// after it has completed every range. Nested invocations use the live
    /// activation chain, not a replay trace, as their provenance parent.
    #[allow(dead_code)] // consumed by the ordered scalar macro matcher issue
    pub(crate) fn push_macro_activation(
        &mut self,
        definition: MacroDefinitionId,
        call_site: OriginId,
        arguments: MacroArguments,
        replacement_tokens: TokenListId,
        replacement_origins: OriginListId,
    ) -> InputLevelId {
        let definition_origin = self
            .state
            .macro_definition_provenance(definition)
            .definition_origin();
        let parent = self.command.parameters.parent_invocation();
        let invocation =
            self.state
                .macro_invocation_origin(definition, call_site, definition_origin, parent);
        self.command.push_macro_activation(
            definition,
            arguments,
            invocation,
            replacement_tokens,
            replacement_origins,
        )
    }
}

fn is_expandable(meaning: Meaning) -> bool {
    matches!(meaning, Meaning::Macro { .. })
        || matches!(
            meaning,
            Meaning::ExpandablePrimitive(primitive)
                if primitive != ExpandablePrimitive::EndCsName
        )
}

fn page_mark(primitive: ExpandablePrimitive) -> PageMark {
    match primitive {
        ExpandablePrimitive::TopMark | ExpandablePrimitive::TopMarks => PageMark::Top,
        ExpandablePrimitive::FirstMark | ExpandablePrimitive::FirstMarks => PageMark::First,
        ExpandablePrimitive::BotMark | ExpandablePrimitive::BotMarks => PageMark::Bot,
        ExpandablePrimitive::SplitFirstMark | ExpandablePrimitive::SplitFirstMarks => {
            PageMark::SplitFirst
        }
        ExpandablePrimitive::SplitBotMark | ExpandablePrimitive::SplitBotMarks => {
            PageMark::SplitBot
        }
        _ => unreachable!("only mark primitives reach page_mark"),
    }
}

fn string_text(state: &tex_state::CommandContext<'_>, token: Token) -> String {
    match token {
        Token::Cs(symbol) => {
            let mut text = String::new();
            let escape = state.int_param(IntParam::ESCAPE_CHAR);
            if let Some(ch) = char::from_u32(u32::try_from(escape).unwrap_or(u32::MAX)) {
                text.push(ch);
            }
            text.push_str(state.resolve(symbol));
            text
        }
        Token::Char { ch, .. } => ch.to_string(),
        Token::Param(slot) => format!("#{slot}"),
        Token::Frozen(_) => "\\relax".to_owned(),
    }
}

fn meaning_text(state: &tex_state::CommandContext<'_>, command: &CurrentCommand) -> String {
    match command.meaning() {
        Meaning::Undefined => "undefined".to_owned(),
        Meaning::Relax => "\\relax".to_owned(),
        Meaning::CharToken { ch, cat } => format!("the character {ch} (catcode {})", cat as u8),
        Meaning::CharGiven(ch) => format!("the character {ch}"),
        Meaning::CountRegister(index) => format!("\\count{index}"),
        Meaning::ToksRegister(index) => format!("\\toks{index}"),
        Meaning::IntParam(index) => format!("integer parameter {index}"),
        Meaning::TokParam(index) => format!("token parameter {index}"),
        Meaning::Font(font) => format!("select font {}", state.font_name(font)),
        Meaning::Macro { .. } => "macro:".to_owned(),
        Meaning::ExpandablePrimitive(_) | Meaning::UnexpandablePrimitive(_) => command
            .control_sequence()
            .map(|symbol| format!("\\{}", state.resolve(symbol)))
            .unwrap_or_else(|| "primitive".to_owned()),
        other => format!("{other:?}"),
    }
}

fn roman_numeral(value: i32) -> String {
    if value <= 0 {
        return String::new();
    }
    let mut remaining = value;
    let mut output = String::new();
    for (amount, glyph) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while remaining >= amount {
            output.push_str(glyph);
            remaining -= amount;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tex_state::Universe;
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
    use tex_state::page::PageMark;
    use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

    use super::*;
    use crate::input::{ReplayTrace, RetirementBehavior};
    use crate::{
        CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState,
        RegisteredSourceKind, SourceRegistration,
    };

    fn traced(token: Token) -> TracedTokenWord {
        TracedTokenWord::pack(token, OriginId::UNKNOWN)
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

    fn install_macro(
        universe: &mut Universe,
        name: &str,
        replacement: Token,
    ) -> tex_state::interner::Symbol {
        let name = universe.intern(name).symbol();
        let empty = universe.intern_token_list(&[]);
        let replacement = universe.intern_token_list(&[replacement]);
        let definition =
            universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
        universe.set_meaning(
            name,
            Meaning::Macro {
                flags: MeaningFlags::EMPTY,
                definition,
            },
        );
        name
    }

    fn install_expandable(
        universe: &mut Universe,
        name: &str,
        primitive: ExpandablePrimitive,
    ) -> tex_state::interner::Symbol {
        let symbol = universe.intern(name).symbol();
        universe.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
        symbol
    }

    fn rendered(processor: &mut CommandProcessor<'_>) -> String {
        let mut text = String::new();
        while let Some(command) = processor.get_x_token().expect("conversion expands") {
            let Token::Char { ch, .. } = command.spelling().semantic_token() else {
                panic!("expected rendered character")
            };
            text.push(ch);
        }
        text
    }

    fn chars(processor: &mut CommandProcessor<'_>) -> String {
        let mut text = String::new();
        while let Some(command) = processor.get_x_token().expect("input expands") {
            if let Token::Char { ch, .. } = command.spelling().semantic_token() {
                text.push(ch);
            }
        }
        text
    }

    #[test]
    fn input_uses_only_capability_registered_backing_and_returns_to_parent() {
        let mut command = CommandState::default();
        let parent = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
            ))
            .expect("parent registers");
        command
            .open_registered_source(parent)
            .expect("parent opens");
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.register_input(
            "inc",
            SourceRegistration::new(
                RegisteredSourceKind::World,
                Arc::<[u8]>::from(b"ab".as_slice()),
            ),
        );
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        assert_eq!(chars(&mut processor), "ab z ");
    }

    #[test]
    fn endinput_keeps_its_line_but_retires_nested_source_before_the_next_line() {
        let mut command = CommandState::default();
        let parent = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
            ))
            .expect("parent registers");
        command
            .open_registered_source(parent)
            .expect("parent opens");
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
        install_expandable(&mut universe, "endinput", ExpandablePrimitive::EndInput);
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.register_input(
            "inc",
            SourceRegistration::new(
                RegisteredSourceKind::World,
                Arc::<[u8]>::from(b"a\\endinput b\nc".as_slice()),
            ),
        );
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        assert_eq!(chars(&mut processor), "ab z ");
    }

    #[test]
    fn jobname_and_mark_retrieval_replay_deterministic_state_values() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let jobname = install_expandable(&mut universe, "jobname", ExpandablePrimitive::JobName);
        let topmark = install_expandable(&mut universe, "topmark", ExpandablePrimitive::TopMark);
        let mark = universe.intern_token_list(&[Token::Char {
            ch: 'M',
            cat: Catcode::Letter,
        }]);
        universe.set_page_mark(PageMark::Top, mark);
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(jobname)),
                traced(Token::Cs(topmark)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.set_job_name("paper");
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        assert_eq!(rendered(&mut processor), "paperM");
    }

    #[test]
    fn scalar_conversions_render_immutable_other_character_tokens() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let number = install_expandable(&mut universe, "number", ExpandablePrimitive::Number);
        let roman = install_expandable(
            &mut universe,
            "romannumeral",
            ExpandablePrimitive::RomanNumeral,
        );
        let string = install_expandable(&mut universe, "string", ExpandablePrimitive::String);
        let target = universe.intern("target").symbol();
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(number)),
                traced(Token::Char {
                    ch: '-',
                    cat: Catcode::Other,
                }),
                traced(Token::Char {
                    ch: '4',
                    cat: Catcode::Other,
                }),
                traced(Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                }),
                traced(Token::Cs(roman)),
                traced(Token::Char {
                    ch: '9',
                    cat: Catcode::Other,
                }),
                traced(Token::Cs(string)),
                traced(Token::Cs(target)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        assert_eq!(rendered(&mut processor), "-42ix\\target");
    }

    #[test]
    fn the_toks_pushes_immutable_stored_input_without_reading_beyond_target() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
        let register = universe.intern("stored").symbol();
        universe.set_meaning(register, Meaning::ToksRegister(7));
        let stored = universe.intern_token_list(&[Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]);
        universe.set_toks(7, stored);
        let trailing = Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        };
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(the)),
                traced(Token::Cs(register)),
                traced(trailing),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let opener = processor.get_next().expect("raw the").expect("the command");
        processor.expand(opener).expect("the inserts stored list");
        assert!(
            matches!(processor.command.input.levels.last(), Some(crate::input::InputLevel::Tokens(cursor))
            if matches!(cursor.payload, TokenPayload::Stored { tokens, .. } if tokens == stored))
        );
        assert_eq!(
            processor
                .get_x_token()
                .expect("stored token")
                .expect("x")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert_eq!(
            processor
                .get_x_token()
                .expect("trailing token")
                .expect("z")
                .spelling()
                .semantic_token(),
            trailing
        );
    }

    #[test]
    fn ordinary_loop_expands_macro_body_on_the_canonical_raw_path() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let macro_name = install_macro(
            &mut universe,
            "m",
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(macro_name))])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let delivered = processor
            .get_x_token()
            .expect("macro expands")
            .expect("body token");
        assert_eq!(
            delivered.spelling().semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert_eq!(processor.command.expansion.cumulative_expansions, 1);
        assert_eq!(processor.command.transient.active_expansion_depth, 0);
    }

    #[test]
    fn noexpand_suppresses_one_macro_delivery_without_changing_its_spelling() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let noexpand = universe.intern("noexpand").symbol();
        universe.set_meaning(
            noexpand,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
        );
        let macro_name = install_macro(
            &mut universe,
            "m",
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(noexpand)),
                traced(Token::Cs(macro_name)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let delivered = processor
            .get_x_token()
            .expect("noexpand completes")
            .expect("target");
        assert_eq!(delivered.spelling().semantic_token(), Token::Cs(macro_name));
        assert_eq!(delivered.meaning(), Meaning::Relax);
        assert_eq!(processor.command.expansion.cumulative_expansions, 1);
    }

    #[test]
    fn expandafter_expands_second_token_before_replaying_first() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let expandafter = universe.intern("expandafter").symbol();
        universe.set_meaning(
            expandafter,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let macro_name = install_macro(
            &mut universe,
            "m",
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(expandafter)),
                traced(Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                }),
                traced(Token::Cs(macro_name)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let first = processor
            .get_x_token()
            .expect("expandafter completes")
            .expect("first token");
        let second = processor
            .get_x_token()
            .expect("macro body follows")
            .expect("body token");
        assert_eq!(
            first.spelling().semantic_token(),
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter
            }
        );
        assert_eq!(
            second.spelling().semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert_eq!(processor.command.expansion.cumulative_expansions, 2);
    }

    #[test]
    fn csname_expands_characters_then_injects_a_relaxed_named_control_sequence() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
        let endcsname =
            install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
        let macro_name = install_macro(
            &mut universe,
            "letter",
            Token::Char {
                ch: 'a',
                cat: Catcode::Other,
            },
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(csname)),
                traced(Token::Cs(macro_name)),
                traced(Token::Char {
                    ch: 'b',
                    cat: Catcode::Letter,
                }),
                traced(Token::Cs(endcsname)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let delivered = {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
            processor
                .get_x_token()
                .expect("csname expands")
                .expect("constructed control sequence")
        };

        let Token::Cs(symbol) = delivered.spelling().semantic_token() else {
            panic!("csname must inject a control sequence");
        };
        assert_eq!(universe.meaning(symbol), Meaning::Relax);
        assert_eq!(
            universe.control_sequence_kind(symbol),
            tex_state::interner::ControlSequenceKind::Named
        );
        assert!(matches!(
            universe.origin(delivered.origin()),
            tex_state::provenance::OriginRecord::Synthesized(origin)
                if origin.kind() == SynthesizedOriginKind::Expansion
        ));
        assert_eq!(command.expansion.cumulative_expansions, 2);
    }

    #[test]
    fn csname_recovers_by_backing_up_a_non_character_before_constructing_the_name() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
        let endcsname =
            install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
        let relax = universe.intern("r").symbol();
        universe.set_meaning(relax, Meaning::Relax);
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(csname)),
                traced(Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                }),
                traced(Token::Cs(relax)),
                traced(Token::Cs(endcsname)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let constructed = processor
            .get_x_token()
            .expect("csname recovery")
            .expect("constructed name");
        let replayed = processor
            .get_x_token()
            .expect("backed up token")
            .expect("relax");
        assert_eq!(constructed.meaning(), Meaning::Relax);
        assert_eq!(replayed.spelling().semantic_token(), Token::Cs(relax));
        assert_eq!(
            processor.command.expansion.pending_diagnostics,
            vec![MISSING_ENDCSNAME_DIAGNOSTIC]
        );
    }

    #[test]
    fn endcsname_is_an_ordinary_loop_boundary_not_an_expandable_dispatch_error() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let endcsname =
            install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(endcsname))])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let boundary = processor
            .get_x_token()
            .expect("boundary delivery")
            .expect("endcsname");
        assert_eq!(
            boundary.meaning(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)
        );
    }

    #[test]
    fn macro_activations_allocate_nested_invocation_provenance() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let empty = universe.intern_token_list(&[]);
        let definition =
            universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
        let mut capabilities = CommandHostCapabilities::default();
        let outer_invocation;
        let inner_invocation;
        {
            let mut processor = CommandProcessor::new(
                &mut command,
                &mut runtime,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
            );
            processor.push_macro_activation(
                definition,
                OriginId::UNKNOWN,
                MacroArguments::default(),
                empty,
                tex_state::ids::OriginListId::EMPTY,
            );
            outer_invocation = processor
                .command
                .parameters
                .activations
                .last()
                .expect("outer activation")
                .invocation;
            processor.push_macro_activation(
                definition,
                OriginId::UNKNOWN,
                MacroArguments::default(),
                empty,
                tex_state::ids::OriginListId::EMPTY,
            );
            inner_invocation = processor
                .command
                .parameters
                .activations
                .last()
                .expect("inner activation")
                .invocation;
        }

        assert_ne!(outer_invocation, inner_invocation);
        assert_eq!(command.parameters.activations.len(), 2);
        assert_eq!(
            universe.macro_invocation_provenance_stats().invocations(),
            2
        );
    }
}

/// Future-relevant expansion facts.
///
/// Per-request fuel is deliberately absent: it is call-local and recreated
/// when an executor step is retried. Caches and profiling likewise belong to
/// [`crate::CommandRuntime`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ExpansionState {
    pub(crate) cumulative_expansions: u64,
    pub(crate) next_resource_resolution: u64,
    pub(crate) pending_diagnostics: Vec<u64>,
    pub(crate) observed_dependencies: Vec<u64>,
    pub(crate) semantic_barriers: Vec<u64>,
    pub(crate) profile: CommandProfile,
}
