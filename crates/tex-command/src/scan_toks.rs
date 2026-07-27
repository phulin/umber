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
    /// required opening brace. This is TeX82 §1227's token-list assignment
    /// alone: it reads the right-hand side's first token through `get_x_token`
    /// to tell a braced list from a token register or parameter, then backs
    /// that brace up for `scan_toks`. Every other caller enters §473 directly
    /// and must use `General`, whose absorbing transition precedes the brace.
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
    pub(crate) malformed_parameter: bool,
}

struct ScannedParameterText {
    tokens: Vec<TracedTokenWord>,
    highest_parameter: u8,
    hash_brace: Option<TracedTokenWord>,
    primary: OriginId,
    malformed_parameter: bool,
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
        let prior = self.command.begin_scanner_status(status.clone());
        self.observe_scanner_status_transition(
            prior.status().clone(),
            self.command.scanner.status().clone(),
        );
        let result = self.scan_toks_inner(mode);
        self.restore_scanner_status_with_observation(status, prior);
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
        // `macro_parameters` is TeX82 §477's `macro_def` flag carried together
        // with §479's `t`: `Some(highest)` selects the parameter-character
        // rule and bounds a legal parameter number, `None` leaves parameter
        // characters as ordinary text (`\message`, `\write`, `\toks`, ...).
        let (expanded, parameter_text, macro_parameters, hash_brace, primary, malformed_parameter) =
            match mode {
                ScanToksMode::General { expanded } => {
                    // TeX scans the required opening brace through the ordinary
                    // expanded path even when the replacement text itself is
                    // collected unexpanded.
                    let primary = self.scan_left_brace(true)?.origin();
                    (expanded, Vec::new(), None, None, primary, false)
                }
                ScanToksMode::GeneralAfterOpening { expanded, primary } => {
                    let opening = self.get_token()?.ok_or(CommandError::input_invariant())?;
                    if !is_begin_group(opening.spelling().semantic_token()) {
                        return Err(CommandError::input_invariant());
                    }
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe_expanded_delivery(&opening);
                    (expanded, Vec::new(), None, None, primary, false)
                }
                ScanToksMode::MacroDefinition { expanded } => {
                    let parameters = self.scan_parameter_text()?;
                    (
                        expanded,
                        parameters.tokens,
                        Some(parameters.highest_parameter),
                        parameters.hash_brace,
                        parameters.primary,
                        parameters.malformed_parameter,
                    )
                }
            };
        let replacement = self.collect_replacement(expanded, macro_parameters)?;
        let mut replacement = replacement;
        // TeX's `#{` parameter-text special case treats that left brace as a
        // delimiter and appends the same saved brace after the replacement
        // text (TeX.web §476).
        if let Some(brace) = hash_brace {
            replacement.push(brace);
        }
        Ok(ScannedToks {
            parameter_text: self.state.finish_traced_token_list(&parameter_text),
            replacement_text: self.state.finish_traced_token_list(&replacement),
            primary,
            malformed_parameter,
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
            .ok_or(CommandError::input_invariant())?;
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
                    return Err(CommandError::input_invariant());
                }
            }
        }
    }

    /// Scans the prefix before a macro replacement's compulsory opening
    /// brace.  Compact `Token::Param` values are the stored out-parameter
    /// representation; doubled hashes remain literal parameter characters.
    fn scan_parameter_text(&mut self) -> Result<ScannedParameterText, CommandError> {
        let mut output = Vec::new();
        let mut next_parameter = 1_u8;
        let mut primary = OriginId::UNKNOWN;
        let mut malformed_parameter = false;
        loop {
            let command = self.get_token()?.ok_or(CommandError::input_invariant())?;
            if primary == OriginId::UNKNOWN {
                primary = command.origin();
            }
            let token = command.spelling().semantic_token();
            if is_begin_group(token) {
                return Ok(ScannedParameterText {
                    tokens: output,
                    highest_parameter: next_parameter - 1,
                    hash_brace: None,
                    primary,
                    malformed_parameter,
                });
            }
            if !is_parameter(token) {
                output.push(command.spelling());
                continue;
            }
            let follower = self.get_token()?.ok_or(CommandError::input_invariant())?;
            let follower_token = follower.spelling().semantic_token();
            if is_begin_group(follower_token) {
                output.push(follower.spelling());
                return Ok(ScannedParameterText {
                    tokens: output,
                    highest_parameter: next_parameter - 1,
                    hash_brace: Some(follower.spelling()),
                    primary,
                    malformed_parameter,
                });
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
            malformed_parameter = true;
            if next_parameter <= 9 {
                output.push(TracedTokenWord::pack(
                    Token::Param(next_parameter),
                    command.origin(),
                ));
                next_parameter += 1;
            }
        }
    }

    /// TeX82 §477, "Scan and build the body of the token list".
    ///
    /// `macro_parameters` is §477's `macro_def` guard: only a macro
    /// definition's body gives a parameter character its §479 meaning, and
    /// then `Some(highest)` is §479's `t` -- the highest parameter number the
    /// parameter text declared, and so the largest one a `#<digit>` may name.
    /// A body scanned for any other purpose (`\message`, `\write`, `\toks`,
    /// `\mark`, ...) stores parameter characters verbatim.
    fn collect_replacement(
        &mut self,
        expanded: bool,
        macro_parameters: Option<u8>,
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
            .ok_or(CommandError::input_invariant())?;

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
                    // TeX82 §394 recovers a non-`\long` macro argument's
                    // `\par` by backing it up while `matching` is live. The
                    // failed macro expansion is then discarded, but the
                    // enclosing definition scan remains live and consumes
                    // that backed-up paragraph token.
                    match self.expand(command) {
                        Ok(()) | Err(CommandError::ParagraphInMacroArgument) => continue,
                        Err(error) => return Err(error),
                    }
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

            // TeX82 §790's `insert_vj` replaces a delivered `\cr`/`\span`/tab
            // delimiter by inserting the `v_j` template and restarting
            // `get_next` -- the delimiter itself never becomes an ordinary
            // token to any caller, including a balanced-text collector like
            // this one. This matters for a braced group whose matching `}`
            // lives in the `v_j` template (plain.tex's
            // `\eqalign`/`\displaylines` idiom `$\displaystyle{##}$` is the
            // common case): without the check below, this still-open depth
            // would carry the raw scan straight through the delimiter,
            // capturing its spelling as literal replacement text. That
            // captured spelling is later replayed verbatim to build the
            // group's material, so the same `\cr` reaches raw delivery a
            // second time -- by then `align_state`/the active cell have
            // already moved past the point where this delivery's own
            // interception is recognized again, so it falls through to
            // ordinary primitive dispatch instead.
            //
            // `get_x_token_scalar` (this crate's `get_x_token`) already
            // performs exactly this handoff for its own expanded delivery
            // loop: on an intercepted delimiter it calls
            // `begin_scalar_alignment_v_template` and restarts, rather than
            // ever handing the delimiter to its caller as ordinary content.
            // For the `expanded: true` collector above, `is_expandable`
            // already routes a converted `end_template` through
            // `self.expand`, which performs that same handoff before this
            // point is reached. This collector's `expanded: false` mode
            // uses the non-expanding `get_token`, which never reaches
            // either dispatch, so the delimiter's own structural
            // consequence (the `v_j` template becoming the input this loop
            // reads next) must be performed explicitly here. Dropping the
            // delimiter's spelling from the collected text (never storing
            // it, but still advancing the loop) then lets the newly
            // inserted `v_j` tokens continue this same balanced scan
            // exactly as if no alignment boundary had been crossed.
            if matches!(
                command.alignment_adjustment(),
                crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
            ) {
                self.begin_scalar_alignment_v_template(command)?;
                continue;
            }

            let spelling = command.spelling();
            let token = spelling.semantic_token();
            if let Some((hash, highest_parameter)) = pending_parameter.take() {
                // §479: a second parameter character stores that character
                // once -- `##` is one parameter token in the body, not two.
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
            if let Some(highest_parameter) = macro_parameters
                && is_parameter(token)
            {
                pending_parameter = Some((spelling, highest_parameter));
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
        let target = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        let Some(value) = self.scan_the_internal_value(&target)? else {
            self.back_input(target)?;
            return Ok(false);
        };
        let tokens = match value {
            crate::InternalValue::Font(symbol) => {
                vec![TracedTokenWord::pack(Token::Cs(symbol), OriginId::UNKNOWN)]
            }
            crate::InternalValue::Tokens { tokens, .. } => self
                .state
                .tokens(tokens)
                .iter()
                .copied()
                .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
                .collect::<Vec<_>>(),
            value => crate::processor::render_the_value(value)
                .expect("non-token internal values render")
                .chars()
                .map(|ch| {
                    TracedTokenWord::pack(
                        Token::Char {
                            ch,
                            cat: if ch == ' ' {
                                Catcode::Space
                            } else {
                                Catcode::Other
                            },
                        },
                        OriginId::UNKNOWN,
                    )
                })
                .collect(),
        };
        #[cfg(any(test, feature = "instrumentation"))]
        let observed = tokens
            .iter()
            .copied()
            .map(|token| self.observed_token(token))
            .collect();
        output.extend(tokens);
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
        let raw = self.collect_replacement(false, None)?;
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
    use std::sync::Arc;

    use tex_state::Universe;
    use tex_state::macro_store::MacroMeaning;

    use super::*;
    use crate::input::{
        ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
    };
    use crate::{
        CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
        CommandRuntime, CommandState, InputTransition, ObservedToken, RegisteredSourceKind,
        SourceRegistration,
    };

    #[derive(Default)]
    struct Recorder(Vec<CommandObservation>);

    impl CommandObserver for Recorder {
        fn committed(&mut self, observation: CommandObservation) {
            self.0.push(observation);
        }
    }

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

    #[test]
    fn eof_recovery_restores_defining_status_before_macro_replacement_completes() {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"{DEF"[..]),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        processor
            .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
            .expect("EOF recovery closes the replacement text");

        let close = recorder
            .0
            .iter()
            .position(|event| {
                matches!(event, CommandObservation::Command(command)
                if matches!(command.spelling, ObservedToken::Character {
                    character: '}',
                    catcode: Catcode::EndGroup,
                }))
            })
            .expect("inserted right brace is delivered");
        let restored = recorder
            .0
            .iter()
            .position(|event| {
                matches!(event, CommandObservation::ScannerStatus(status)
                if status.from == "defining" && status.to == "normal")
            })
            .expect("defining status restores after the inserted right brace");
        assert!(close < restored);
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
        let mut universe = Universe::new_with_plain_catcodes();
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
    fn direct_the_count_scans_the_eight_bit_index_before_its_terminator_backup() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
        let count = universe.intern("count").symbol();
        universe.set_meaning(
            count,
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
        );
        universe.set_count(21, -83);
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(the),
                Token::Cs(count),
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: ',',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("expanded collection succeeds");
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            &[
                Token::Char {
                    ch: '-',
                    cat: Catcode::Other
                },
                Token::Char {
                    ch: '8',
                    cat: Catcode::Other
                },
                Token::Char {
                    ch: '3',
                    cat: Catcode::Other
                },
                Token::Char {
                    ch: ',',
                    cat: Catcode::Other
                },
            ]
        );
        let two = recorder
            .0
            .iter()
            .position(|observation| matches!(
                observation,
                CommandObservation::Command(record)
                    if matches!(record.spelling, ObservedToken::Character { character: '2', .. })
            ))
            .expect("index digit is delivered");
        let backup = recorder
            .0
            .iter()
            .position(|observation| matches!(
                observation,
                CommandObservation::Input(record) if record.transition == InputTransition::Backup
            ))
            .expect("terminator is backed up");
        assert!(two < backup);
    }

    #[test]
    fn completed_direct_splice_scan_rolls_back_to_the_exact_input_state() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
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
        let mut universe = Universe::new_with_plain_catcodes();
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

    /// TeX82 §477 gates the body's parameter-character rule on `macro_def`
    /// alone, never on whether the parameter text declared a parameter, so a
    /// parameterless definition still collapses `##` to one token. plain.tex's
    /// `\m@ketabbox` (`\ialign\bgroup&\t@bbox##\t@bb@x\crcr`) is the canonical
    /// witness.
    #[test]
    fn parameterless_macro_definition_still_collapses_doubled_hashes() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
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
        assert!(
            processor
                .state
                .tokens(scanned.parameter_text.token_list())
                .is_empty()
        );
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            &[Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            }]
        );
    }

    /// The same rule is `macro_def`-gated: a general text scan (`\message`,
    /// `\toks`, e-TeX `\unexpanded`) stores both parameter characters.
    #[test]
    fn general_text_keeps_both_parameter_characters() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
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
            .scan_toks(ScanToksMode::General { expanded: false })
            .expect("general text scans");
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            &[
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
            ]
        );
    }

    #[test]
    fn macro_definition_hash_brace_reuses_the_left_brace_after_the_body() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
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
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: '[',
                    cat: Catcode::Other,
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
                    ch: ']',
                    cat: Catcode::Other,
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
            &[
                Token::Param(1),
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
            ]
        );
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            &[
                Token::Char {
                    ch: '[',
                    cat: Catcode::Other,
                },
                Token::Param(1),
                Token::Char {
                    ch: ']',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
            ]
        );
    }

    #[test]
    fn expanded_collection_expands_a_macro_one_step_at_a_time() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
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
