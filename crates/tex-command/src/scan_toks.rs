//! Private canonical token-list scanner.
//!
//! This is deliberately a small, separate state machine rather than a
//! second `get_x_token` interpreter.  TeX.web's `scan_toks` has one crucial
//! exception to ordinary expansion: token-list results from `\the` (and the
//! e-TeX `\unexpanded` family) join the result directly.  In particular, the
//! contents of such a list neither consume the caller's input nor contribute
//! to the brace depth of this collection.
#![allow(dead_code)] // executor scanner callers arrive in the following slice

use tex_state::TracedTokenList;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::processor::status::{
    AbsorbingContext, DefinitionContext, ScannerStatus, ScannerWarning, TokenBuilderId,
};
use crate::{CommandError, CommandProcessor};

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::{CommandObservation, TokenListRecord};

/// The two canonical `scan_toks` collection forms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScanToksMode {
    /// Collect balanced general text; parameter characters are ordinary text.
    General { expanded: bool },
    /// Collect general text after the caller has validated and backed up the
    /// required opening brace. This preserves TeX82's scanner ordering for
    /// `\\write`: `scan_int`'s terminator is first validated through the
    /// expanded path, then the absorbing collector replays that exact backup.
    GeneralAfterOpening { expanded: bool, primary: OriginId },
    /// Collect a macro parameter text followed by its replacement text.
    MacroDefinition { expanded: bool },
}

/// Frozen output of one `scan_toks` episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScannedToks {
    pub(crate) parameter_text: TracedTokenList,
    pub(crate) replacement_text: TracedTokenList,
    pub(crate) primary: OriginId,
}

impl CommandProcessor<'_> {
    /// TeX.web's special token-list collector (parts 26--27).
    ///
    /// `expanded` means one `get_next`/`expand` step per iteration, never a
    /// call to the ordinary expanded delivery loop.  That distinction keeps
    /// the collector's closing brace inaccessible to expansion that happens
    /// to retire an inserted replay level.
    pub(crate) fn scan_toks(&mut self, mode: ScanToksMode) -> Result<ScannedToks, CommandError> {
        let builder = TokenBuilderId(self.command.transient.next_builder_identity);
        self.command.transient.next_builder_identity =
            self.command.transient.next_builder_identity.wrapping_add(1);
        let warning = ScannerWarning(builder.0);
        let status = match mode {
            ScanToksMode::General { .. } | ScanToksMode::GeneralAfterOpening { .. } => {
                ScannerStatus::Absorbing(AbsorbingContext {
                    owner: None,
                    builder,
                    warning,
                })
            }
            ScanToksMode::MacroDefinition { .. } => ScannerStatus::Defining(DefinitionContext {
                target: None,
                builder,
                warning,
            }),
        };
        let prior = self.command.begin_scanner_status(status);
        self.observe_scanner_status(true);
        let result = self.scan_toks_inner(mode);
        self.observe_scanner_status(false);
        self.command.restore_scanner_status(prior);
        let result = result?;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::TokenList(TokenListRecord {
            transition: "complete",
            purpose: match mode {
                ScanToksMode::General { expanded: true }
                | ScanToksMode::GeneralAfterOpening { expanded: true, .. } => "expanded_scan_toks",
                ScanToksMode::General { expanded: false }
                | ScanToksMode::GeneralAfterOpening {
                    expanded: false, ..
                } => "scan_toks",
                ScanToksMode::MacroDefinition { .. } => "macro_replacement",
            },
            tokens: self
                .state
                .tokens(result.replacement_text.token_list())
                .iter()
                .copied()
                .map(|token| self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN)))
                .collect(),
        }));
        Ok(result)
    }

    fn scan_toks_inner(&mut self, mode: ScanToksMode) -> Result<ScannedToks, CommandError> {
        let (expanded, parameter_text, highest_parameter, hash_brace, primary) = match mode {
            ScanToksMode::General { expanded } => {
                // TeX scans the required opening brace through the ordinary
                // expanded path even when the replacement text itself is
                // collected unexpanded.
                let primary = self.scan_left_brace(true)?.origin();
                (expanded, Vec::new(), 0, None, primary)
            }
            ScanToksMode::GeneralAfterOpening { expanded, primary } => {
                let opening = self.get_token()?.ok_or(CommandError::InputInvariant)?;
                if !is_begin_group(opening.spelling().semantic_token()) {
                    return Err(CommandError::InputInvariant);
                }
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe_expanded_delivery(&opening);
                (expanded, Vec::new(), 0, None, primary)
            }
            ScanToksMode::MacroDefinition { expanded } => {
                let (parameters, highest, hash_brace, primary) = self.scan_parameter_text()?;
                (expanded, parameters, highest, hash_brace, primary)
            }
        };
        let replacement = self.collect_replacement(expanded, highest_parameter)?;
        let mut replacement = replacement;
        // TeX's `#{` parameter-text special case treats the brace as a
        // delimiter and places the saved parameter character after the
        // replacement text (TeX.web §476).
        if let Some(hash) = hash_brace {
            replacement.push(hash);
        }
        Ok(ScannedToks {
            parameter_text: self.state.finish_traced_token_list(&parameter_text),
            replacement_text: self.state.finish_traced_token_list(&replacement),
            primary,
        })
    }

    pub(crate) fn scan_left_brace(
        &mut self,
        expanded: bool,
    ) -> Result<crate::CurrentCommand, CommandError> {
        loop {
            let command = if expanded {
                self.get_x_token()?
            } else {
                self.get_token()?
            }
            .ok_or(CommandError::InputInvariant)?;
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } => continue,
                Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                } => return Ok(command),
                _ => {
                    self.back_input(command)?;
                    return Err(CommandError::InputInvariant);
                }
            }
        }
    }

    /// Scans the prefix before a macro replacement's compulsory opening
    /// brace.  Compact `Token::Param` values are the stored out-parameter
    /// representation; doubled hashes remain literal parameter characters.
    fn scan_parameter_text(
        &mut self,
    ) -> Result<(Vec<TracedTokenWord>, u8, Option<TracedTokenWord>, OriginId), CommandError> {
        let mut output = Vec::new();
        let mut next_parameter = 1_u8;
        let mut primary = OriginId::UNKNOWN;
        loop {
            let command = self.get_token()?.ok_or(CommandError::InputInvariant)?;
            if primary == OriginId::UNKNOWN {
                primary = command.origin();
            }
            let token = command.spelling().semantic_token();
            if is_begin_group(token) {
                return Ok((output, next_parameter - 1, None, primary));
            }
            if !is_parameter(token) {
                output.push(command.spelling());
                continue;
            }
            let follower = self.get_token()?.ok_or(CommandError::InputInvariant)?;
            let follower_token = follower.spelling().semantic_token();
            if is_begin_group(follower_token) {
                output.push(follower.spelling());
                return Ok((
                    output,
                    next_parameter - 1,
                    Some(command.spelling()),
                    primary,
                ));
            }
            if let Some(number) = parameter_number(follower_token)
                && number == next_parameter
                && number <= 9
            {
                output.push(TracedTokenWord::pack(
                    Token::Param(number),
                    follower.origin(),
                ));
                next_parameter += 1;
                continue;
            }
            // Canonical recovery keeps the rejected follower available and
            // supplies the expected parameter number.  The pending outer
            // validity operation remains responsible for all inaccessible
            // token recovery.
            self.back_input(follower)?;
            if next_parameter <= 9 {
                output.push(TracedTokenWord::pack(
                    Token::Param(next_parameter),
                    command.origin(),
                ));
                next_parameter += 1;
            }
        }
    }

    fn collect_replacement(
        &mut self,
        expanded: bool,
        highest_parameter: u8,
    ) -> Result<Vec<TracedTokenWord>, CommandError> {
        let mut output = Vec::new();
        let mut depth = 1_u32;
        let mut pending_parameter = None;
        loop {
            let command = if expanded {
                self.get_next()?
            } else {
                self.get_token()?
            }
            .ok_or(CommandError::InputInvariant)?;

            if expanded && is_expandable(command.meaning()) {
                if matches!(
                    command.meaning(),
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::The)
                ) && self.append_direct_the_toks(&mut output)?
                {
                    continue;
                }
                if matches!(
                    command.meaning(),
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded)
                ) {
                    self.append_unexpanded(&mut output)?;
                    continue;
                }
                if matches!(command.meaning(), Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED))
                {
                    // Protected macros are terminal tokens in an e-TeX
                    // expanded token-list scan.
                } else {
                    self.expand(command)?;
                    continue;
                }
            }

            // The expanded collector has completed a get_x-style delivery
            // for each retained unexpandable token. Emit that boundary before
            // storing the spelling, while expandable commands above remain
            // represented by their own expansion transitions.
            #[cfg(any(test, feature = "instrumentation"))]
            if expanded {
                self.observe_expanded_delivery(&command);
            }

            let spelling = command.spelling();
            let token = spelling.semantic_token();
            if let Some(hash) = pending_parameter.take() {
                if is_parameter(token) {
                    output.push(spelling);
                    continue;
                }
                if let Some(number) = parameter_number(token)
                    && number <= highest_parameter
                {
                    let converted = TracedTokenWord::pack(Token::Param(number), spelling.origin());
                    output.push(converted);
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe(CommandObservation::TokenList(TokenListRecord {
                        transition: "splice",
                        purpose: "parameter_conversion",
                        tokens: vec![self.observed_token(converted)],
                    }));
                    continue;
                }
                self.back_input(command)?;
                output.push(hash);
                continue;
            }
            if is_parameter(token) && highest_parameter != 0 {
                pending_parameter = Some(spelling);
                continue;
            }
            if is_begin_group(token) {
                depth = depth.saturating_add(1);
                output.push(spelling);
            } else if is_end_group(token) {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(output);
                }
                output.push(spelling);
            } else {
                output.push(spelling);
            }
        }
    }

    /// Splices a token-list result of `\the` into the builder directly.
    /// The target alone is read; no input from after that target is examined.
    fn append_direct_the_toks(
        &mut self,
        output: &mut Vec<TracedTokenWord>,
    ) -> Result<bool, CommandError> {
        let target = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
        let tokens = match target.meaning() {
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Toks) => {
                let index = u16::try_from(self.scan_integer()?.value).unwrap_or(0);
                let tokens = self.state.toks(index);
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(crate::ScannerRecord {
                    kind: "internal",
                    value: "tokens".into(),
                    tokens: Some(
                        self.state
                            .tokens(tokens)
                            .iter()
                            .copied()
                            .map(|token| {
                                self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN))
                            })
                            .collect(),
                    ),
                }));
                Some(tokens)
            }
            Meaning::ToksRegister(index) => Some(self.state.toks(index)),
            Meaning::TokParam(index) => Some(
                self.state
                    .tok_param(tex_state::env::banks::TokParam::new(index)),
            ),
            _ => None,
        };
        let Some(tokens) = tokens else {
            self.back_input(target)?;
            return Ok(false);
        };
        #[cfg(any(test, feature = "instrumentation"))]
        let observed = self
            .state
            .tokens(tokens)
            .iter()
            .copied()
            .map(|token| self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN)))
            .collect();
        output.extend(
            self.state
                .tokens(tokens)
                .iter()
                .copied()
                .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN)),
        );
        self.command.expansion.cumulative_expansions =
            self.command.expansion.cumulative_expansions.wrapping_add(1);
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::TokenList(TokenListRecord {
            transition: "splice",
            purpose: "the_toks",
            tokens: observed,
        }));
        Ok(true)
    }

    /// e-TeX `\unexpanded` uses the same direct-splice rule.  Its balanced
    /// text is scanned raw and attached without parameter conversion or
    /// recursive expansion.
    fn append_unexpanded(&mut self, output: &mut Vec<TracedTokenWord>) -> Result<(), CommandError> {
        let _ = self.scan_left_brace(false)?;
        let raw = self.collect_replacement(false, 0)?;
        output.extend(raw);
        self.command.expansion.cumulative_expansions =
            self.command.expansion.cumulative_expansions.wrapping_add(1);
        Ok(())
    }
}

fn is_expandable(meaning: Meaning) -> bool {
    matches!(
        meaning,
        Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_)
    ) && !matches!(
        meaning,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)
    )
}

fn is_parameter(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            ch: '#',
            cat: Catcode::Parameter
        }
    )
}

fn parameter_number(token: Token) -> Option<u8> {
    match token {
        Token::Char {
            ch: '1'..='9',
            cat: Catcode::Other,
        } => Some((token_char(token)? as u8) - b'0'),
        _ => None,
    }
}

fn token_char(token: Token) -> Option<char> {
    match token {
        Token::Char { ch, .. } => Some(ch),
        _ => None,
    }
}

fn is_begin_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    )
}

fn is_end_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use tex_state::Universe;
    use tex_state::macro_store::MacroMeaning;

    use super::*;
    use crate::input::{
        ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
    };
    use crate::{CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState};

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

    fn push(command: &mut CommandState, tokens: Vec<Token>) {
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(
                tokens.into_iter().map(traced).collect::<Vec<_>>(),
            )),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
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

    #[test]
    fn direct_the_toks_splice_is_unexpanded_and_does_not_balance_the_collector() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
        let macro_symbol = universe.intern("storedmacro").symbol();
        let register = universe.intern("stored").symbol();
        universe.set_meaning(register, Meaning::ToksRegister(3));
        let stored = universe.intern_token_list(&[
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(macro_symbol),
        ]);
        universe.set_toks(3, stored);
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(the),
                Token::Cs(register),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'z',
                    cat: Catcode::Letter,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("scan succeeds");
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            &[
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(macro_symbol)
            ]
        );
        assert_eq!(
            processor
                .get_next()
                .expect("trailing token delivers")
                .expect("trailing token exists")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'z',
                cat: Catcode::Letter,
            }
        );
    }

    #[test]
    fn completed_direct_splice_scan_rolls_back_to_the_exact_input_state() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
        let register = universe.intern("stored").symbol();
        universe.set_meaning(register, Meaning::ToksRegister(3));
        let stored = universe.intern_token_list(&[
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ]);
        universe.set_toks(3, stored);
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(the),
                Token::Cs(register),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let expected = command.clone();
        let snapshot = command.snapshot();
        let mut capabilities = CommandHostCapabilities::default();

        let first = {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
            let scanned = processor
                .scan_toks(ScanToksMode::General { expanded: true })
                .expect("direct splice scan succeeds");
            processor
                .state
                .tokens(scanned.replacement_text.token_list())
                .to_vec()
        };
        command.rollback(snapshot).expect("rollback succeeds");
        assert_eq!(command, expected);

        let replayed = {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
            let scanned = processor
                .scan_toks(ScanToksMode::General { expanded: true })
                .expect("rolled-back direct splice scan succeeds");
            processor
                .state
                .tokens(scanned.replacement_text.token_list())
                .to_vec()
        };
        assert_eq!(replayed, first);
    }

    #[test]
    fn macro_definition_converts_parameters_and_preserves_doubled_hashes() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let scanned = processor
            .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
            .expect("definition scans");
        assert_eq!(
            processor.state.tokens(scanned.parameter_text.token_list()),
            &[Token::Param(1)]
        );
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            &[
                Token::Param(1),
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
            ]
        );
    }

    #[test]
    fn expanded_collection_expands_a_macro_one_step_at_a_time() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let macro_symbol = universe.intern("m").symbol();
        let empty = universe.intern_token_list(&[]);
        let replacement = universe.intern_token_list(&[Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]);
        let definition =
            universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
        universe.set_meaning(
            macro_symbol,
            Meaning::Macro {
                flags: MeaningFlags::EMPTY,
                definition,
            },
        );
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(macro_symbol),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("expanded scan succeeds");
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            &[Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }]
        );
    }
}
