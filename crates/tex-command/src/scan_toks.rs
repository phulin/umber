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

    /// TeX82 §403's `scan_left_brace`, the one routine every mandatory
    /// opening brace goes through.  On a non-brace it backs the rejected
    /// command up, as §403's `back_error` does, and reports the failure so
    /// the caller can apply §403's "behave as though a `{` had been read"
    /// recovery where that recovery is observable.
    ///
    /// §403 opens with §404's "Get the next non-blank non-relax non-call
    /// token" -- `repeat get_x_token until (cur_cmd<>spacer)and
    /// (cur_cmd<>relax)` -- because, in §403's own words, "\TeX\ allows
    /// \.{\\relax} to appear before the |left_brace|". Skipping only spaces
    /// rejected the brace in `\message\relax{...}` and every plain-TeX idiom
    /// that parks a `\relax` in front of a mandatory group
    /// (`umber2-johp.209`).
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
                }
                | Meaning::Relax => continue,
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

            // TeX82 §342 has already replaced a delivered `\cr`/`\span`/tab
            // delimiter by §789's ⟨v_j⟩ template inside `get_next`, so this
            // balanced-text collector never sees one. That matters for a
            // braced group whose matching `}` lives in the ⟨v_j⟩ template
            // (plain.tex's `\eqalign`/`\displaylines` idiom
            // `$\displaystyle{##}$` is the common case): the still-open
            // `depth` continues over the boundary exactly as if no alignment
            // entry had ended.
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
mod tests;
