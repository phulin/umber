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
use tex_state::interner::{ControlSequenceKind, Symbol};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::processor::alignment::TEMPLATE_ALIGN_STATE;
use crate::processor::expand::is_expandable_command;
use crate::processor::status::{
    AbsorbingContext, DefinitionContext, ScannerStatus, ScannerWarning, TokenBuilderId,
};
use crate::{CommandError, CommandProcessor};
use tex_state::CommandLineSource;

use crate::input::TokenPayload;
use crate::observation::{
    CommandObservation, DiagnosticRecord, InputReason, InputRecord, InputTransition,
    TokenListRecord,
};

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
    /// e-TeX 2.6 etex.ch §53a's recursive `scan_general_text`.
    ///
    /// It has the same absorbing-state recovery semantics as TeX82
    /// `scan_toks(false, false)`, but is a distinct canonical observation
    /// seam: its caller publishes the extension-specific token-list purpose,
    /// and the reference instrumentation does not publish the internal
    /// scanner-status scope.
    GeneralText { purpose: &'static str },
    /// Collect a macro parameter text followed by its replacement text.
    MacroDefinition { expanded: bool },
    /// Production macro definition scan, carrying §479's `warning_index`.
    MacroDefinitionFor { expanded: bool, target: Symbol },
}

/// Frozen output of one `scan_toks` episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScannedToks {
    pub(crate) parameter_text: TracedTokenList,
    pub(crate) replacement_text: TracedTokenList,
    pub(crate) primary: OriginId,
    pub(crate) malformed_parameter: bool,
}

/// TeX82 §403's result after a mandatory left-brace scan.
#[derive(Debug)]
pub(crate) enum ScannedLeftBrace {
    /// A real source or replay token supplied the brace.
    Consumed(crate::CurrentCommand),
    /// §403 inserted the brace after backing up the offending command.
    Inserted,
}

impl ScannedLeftBrace {
    pub(crate) fn origin(&self) -> OriginId {
        match self {
            Self::Consumed(command) => command.origin(),
            Self::Inserted => OriginId::UNKNOWN,
        }
    }
}

struct ScannedParameterText {
    tokens: Vec<TracedTokenWord>,
    highest_parameter: u8,
    hash_brace: Option<TracedTokenWord>,
    primary: OriginId,
    malformed_parameter: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MacroParameterDiagnostic {
    NonconsecutiveNumber,
    TooManyParameters,
    IllegalReplacementNumber { target: Option<Symbol> },
}

const NONCONSECUTIVE_PARAMETER_DIAGNOSTIC: u64 = 0x6465_6600_0000_0476;
const ILLEGAL_REPLACEMENT_PARAMETER_DIAGNOSTIC: u64 = 0x6465_6600_0000_0479;

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
            ScanToksMode::General { .. }
            | ScanToksMode::GeneralAfterOpening { .. }
            | ScanToksMode::GeneralText { .. } => ScannerStatus::Absorbing(AbsorbingContext {
                owner: None,
                builder,
                warning,
            }),
            ScanToksMode::MacroDefinition { .. } | ScanToksMode::MacroDefinitionFor { .. } => {
                ScannerStatus::Defining(DefinitionContext {
                    target: match mode {
                        ScanToksMode::MacroDefinitionFor { target, .. } => Some(target),
                        _ => None,
                    },
                    builder,
                    warning,
                })
            }
        };
        let prior = self.command.begin_scanner_status(status.clone());
        let observe_status = !matches!(mode, ScanToksMode::GeneralText { .. });
        if observe_status {
            self.observe_scanner_status_transition(
                prior.status().clone(),
                self.command.scanner.status().clone(),
            );
        }
        let result = self.scan_toks_inner(mode);
        if observe_status {
            self.restore_scanner_status_with_observation(status, prior);
        } else {
            self.command.restore_scanner_status(prior);
        }
        let result = result?;
        let completed_tokens = if !self.is_observed() {
            Vec::new()
        } else if matches!(
            mode,
            ScanToksMode::GeneralText {
                purpose: "detokenize"
            }
        ) {
            crate::processor::expand::token_list_string_text(
                &mut self.state,
                result.replacement_text.token_list(),
            )
            .chars()
            .map(|ch| {
                self.observed_token(TracedTokenWord::pack(
                    Token::Char {
                        ch,
                        cat: if ch == ' ' {
                            Catcode::Space
                        } else {
                            Catcode::Other
                        },
                    },
                    OriginId::UNKNOWN,
                ))
            })
            .collect()
        } else {
            self.state
                .tokens(result.replacement_text.token_list())
                .iter()
                .copied()
                .map(|token| self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN)))
                .collect()
        };
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: match mode {
                    ScanToksMode::General { expanded: true }
                    | ScanToksMode::GeneralAfterOpening { expanded: true, .. } =>
                        "expanded_scan_toks",
                    ScanToksMode::General { expanded: false }
                    | ScanToksMode::GeneralAfterOpening {
                        expanded: false, ..
                    } => "scan_toks",
                    ScanToksMode::GeneralText { purpose } => purpose,
                    ScanToksMode::MacroDefinition { .. }
                    | ScanToksMode::MacroDefinitionFor { .. } => {
                        "macro_replacement"
                    }
                },
                tokens: completed_tokens,
            }),
        );
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
                    self.observe_expanded_delivery(&opening);
                    (expanded, Vec::new(), None, None, primary, false)
                }
                ScanToksMode::GeneralText { .. } => {
                    let primary = self.scan_left_brace(true)?.origin();
                    (false, Vec::new(), None, None, primary, false)
                }
                ScanToksMode::MacroDefinition { expanded } => {
                    let parameters = self.scan_parameter_text()?;
                    (
                        expanded,
                        parameters.tokens,
                        Some((parameters.highest_parameter, None)),
                        parameters.hash_brace,
                        parameters.primary,
                        parameters.malformed_parameter,
                    )
                }
                ScanToksMode::MacroDefinitionFor { expanded, target } => {
                    let parameters = self.scan_parameter_text()?;
                    (
                        expanded,
                        parameters.tokens,
                        Some((parameters.highest_parameter, Some(target))),
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
    /// opening brace goes through. On a non-brace it reports the exact §403
    /// error, backs the rejected command up, installs the synthetic brace's
    /// `align_state` contribution, and returns normally just as TeX does.
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
    ) -> Result<ScannedLeftBrace, CommandError> {
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
                } => return Ok(ScannedLeftBrace::Consumed(command)),
                _ => {
                    let deferred = {
                        let mut report = self.state.print_err("Missing { inserted");
                        report.help(&[
                            "A left brace was mandatory here, so I've put one in.",
                            "You might want to delete and/or insert some corrections",
                            "so that I will find a matching right brace soon.",
                            "(If you're confused by all this, try typing `I}' now.)",
                        ]);
                        report.defer()
                    };
                    // §403's `back_error` is `back_input; error`: the message
                    // and help are prepared above, then the rejected command
                    // is restored before §82 appends the period and help.
                    // Rendering §310's display only after the backup is what
                    // makes §314 name the rejected token on its own
                    // `<to be read again>␣` line.
                    self.back_input(command)?;
                    let context = self.command.output_open_context(&self.state);
                    let mut report = self.state.resume_error_report(deferred);
                    report.context(context);
                    report.error();
                    // §403 assigns `cur_cmd=left_brace` and increments
                    // `align_state` exactly as raw delivery of that synthetic
                    // brace would have done. The token itself is not pushed:
                    // the caller continues after it while the rejected command
                    // remains first on the backed-up input level.
                    self.command.alignment.align_state += 1;
                    return Ok(ScannedLeftBrace::Inserted);
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
            if next_parameter > 9 {
                // TeX82 §476 has already consumed both the parameter
                // character and its follower when `t=#9`; it diagnoses and
                // returns to `continue` without backing either token up.
                self.report_macro_parameter_diagnostic(MacroParameterDiagnostic::TooManyParameters);
                malformed_parameter = true;
                continue;
            }
            // Canonical recovery keeps the rejected follower available and
            // supplies the expected parameter number.  The pending outer
            // validity operation remains responsible for all inaccessible
            // token recovery.
            // §476's text is already rendered by
            // `report_macro_parameter_diagnostic` below; this records only
            // the recovery identity.
            self.back_error(follower, NONCONSECUTIVE_PARAMETER_DIAGNOSTIC)?;
            self.report_macro_parameter_diagnostic(MacroParameterDiagnostic::NonconsecutiveNumber);
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
        macro_parameters: Option<(u8, Option<Symbol>)>,
    ) -> Result<Vec<TracedTokenWord>, CommandError> {
        let mut output = Vec::new();
        let mut depth = 1_u32;
        let mut pending_parameter: Option<(_, u8, Option<Symbol>)> = None;
        let collector_status = self.command.scanner.status().clone();
        loop {
            let delivered;
            let spelling = {
                let mut command = if expanded {
                    self.get_next()?
                } else {
                    self.get_token()?
                }
                .ok_or(CommandError::input_invariant())?;
                if expanded && is_expandable_command(&command) {
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
                    if matches!(
                        command.meaning(),
                        Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize)
                    ) {
                        self.append_detokenize(&mut output)?;
                        continue;
                    }
                    if matches!(command.meaning(), Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED))
                    {
                        // e-TeX 2.6 change section [27.465] represents a
                        // protected macro as `relax/no_expand_flag` for this
                        // collector iteration. The spelling is retained, and
                        // the reference instrumentation observes that exact
                        // one-token suppression splice before the terminal
                        // expanded delivery.
                        observe!(
                            self,
                            CommandObservation::TokenList(TokenListRecord {
                                transition: "splice",
                                purpose: "protected_expansion_suppression",
                                tokens: vec![self.observed_token(command.spelling())],
                            }),
                        );
                        command.suppress_expandable();
                    } else {
                        // TeX82 §394 returns from a failed macro call after
                        // either an ordinary non-`\long` `\par` or §23's
                        // outer-validity recovery. Both return to §380's
                        // get_x_token loop; this inlined expanded collector is
                        // that loop's owner while scan_toks is active.
                        match self.expand(command) {
                            Ok(()) | Err(CommandError::ParagraphInMacroArgument) => continue,
                            Err(CommandError::OuterInMacroArgument) => {
                                self.restore_collector_status_after_outer_abort(&collector_status);
                                continue;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }

                // The expanded collector has completed a get_x-style delivery
                // for each retained unexpandable token. Emit that boundary before
                // storing the spelling, while expandable commands above remain
                // represented by their own expansion transitions.
                if expanded {
                    self.observe_expanded_delivery(&command);
                }
                let spelling = command.spelling();
                delivered = command;
                spelling
            };

            // TeX82 §342 has already replaced a delivered `\cr`/`\span`/tab
            // delimiter by §789's ⟨v_j⟩ template inside `get_next`, so this
            // balanced-text collector never sees one. That matters for a
            // braced group whose matching `}` lives in the ⟨v_j⟩ template
            // (plain.tex's `\eqalign`/`\displaylines` idiom
            // `$\displaystyle{##}$` is the common case): the still-open
            // `depth` continues over the boundary exactly as if no alignment
            // entry had ended.
            let token = spelling.semantic_token();
            if let Some((hash, highest_parameter, target)) = pending_parameter.take() {
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
                    observe!(
                        self,
                        CommandObservation::TokenList(TokenListRecord {
                            transition: "splice",
                            purpose: "parameter_conversion",
                            tokens: vec![self.observed_token(converted)],
                        }),
                    );
                    continue;
                }
                // §479's text is already rendered by
                // `report_macro_parameter_diagnostic` below.
                self.back_error(delivered, ILLEGAL_REPLACEMENT_PARAMETER_DIAGNOSTIC)?;
                self.report_macro_parameter_diagnostic(
                    MacroParameterDiagnostic::IllegalReplacementNumber { target },
                );
                output.push(hash);
                continue;
            }
            if let Some((highest_parameter, target)) = macro_parameters
                && is_parameter(token)
            {
                pending_parameter = Some((spelling, highest_parameter, target));
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

    /// Reasserts §400's saved caller status after a nested §394 abort.
    fn restore_collector_status_after_outer_abort(&mut self, collector_status: &ScannerStatus) {
        // TeX82 §400 restores `scan_toks`'s saved absorbing/defining status
        // when §394 aborts a macro call after §23 outer-token recovery. A
        // nested expansion can unwind through more than one macro call, so
        // the collector reasserts its own saved status before it rereads the
        // backed outer token.
        if matches!(self.command.scanner.status(), ScannerStatus::Normal) {
            let prior = self.command.begin_scanner_status(collector_status.clone());
            self.observe_scanner_status_transition(
                prior.status().clone(),
                self.command.scanner.status().clone(),
            );
        }
    }

    fn report_macro_parameter_diagnostic(&mut self, diagnostic: MacroParameterDiagnostic) {
        observe!(
            self,
            CommandObservation::Diagnostic(DiagnosticRecord {
                severity: "error",
                diagnostic: match diagnostic {
                    MacroParameterDiagnostic::NonconsecutiveNumber =>
                        "nonconsecutive_macro_parameter",
                    MacroParameterDiagnostic::TooManyParameters => "too_many_macro_parameters",
                    MacroParameterDiagnostic::IllegalReplacementNumber { .. } => {
                        "illegal_replacement_parameter"
                    }
                },
                arguments: Vec::new(),
            }),
        );
        // §§476/479 reach `error` only after their own `back_error` has
        // restored the rejected token, which is why §310's display is
        // rendered here rather than at the scanner's decision point.
        let context = self.command.output_open_context(&self.state);
        match diagnostic {
            MacroParameterDiagnostic::NonconsecutiveNumber => {
                let mut report = self
                    .state
                    .print_err("Parameters must be numbered consecutively");
                report
                    .help(&[
                        "I've inserted the digit you should have used after the #.",
                        "Type `1' to delete what you did use.",
                    ])
                    .context(context);
                report.error();
            }
            MacroParameterDiagnostic::TooManyParameters => {
                let mut report = self.state.print_err("You already have nine parameters");
                report
                    .help(&[
                        "I'm going to ignore the # sign you just used,",
                        "as well as the token that followed it.",
                    ])
                    .context(context);
                report.error();
            }
            MacroParameterDiagnostic::IllegalReplacementNumber { target } => {
                let rendered_target = target.map(|target| {
                    (
                        self.state.resolve(target).to_owned(),
                        self.state.control_sequence_kind(target),
                    )
                });
                let mut report = self
                    .state
                    .print_err("Illegal parameter number in definition of ");
                if let Some((name, kind)) = rendered_target {
                    if kind == ControlSequenceKind::Named {
                        report.print_esc(&name);
                    } else {
                        report.print(&name);
                    }
                }
                report
                    .help(&[
                        "You meant to type ## instead of #, right?",
                        "Or maybe a } was forgotten somewhere earlier, and things",
                        "are all screwed up? I'm going to assume that you meant ##.",
                    ])
                    .context(context);
                report.error();
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
        // Built only for an observed episode: an unobserved one leaves this
        // empty, which `Vec::new` does without allocating.
        let observed: Vec<_> = if self.is_observed() {
            tokens
                .iter()
                .copied()
                .map(|token| self.observed_token(token))
                .collect()
        } else {
            Vec::new()
        };
        output.extend(tokens);
        self.command.expansion.cumulative_expansions = self
            .command
            .expansion
            .cumulative_expansions
            .saturating_add(1);
        // TeX82 §478 attaches `the_toks` only when `link(temp_head)<>null`.
        // Keep the observation on that same semantic boundary: an empty
        // internal token list contributes no splice transition at all.
        if !observed.is_empty() {
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "the_toks",
                    tokens: observed,
                }),
            );
        }
        Ok(true)
    }

    /// e-TeX `\unexpanded` uses the same direct-splice rule.  Its balanced
    /// text is scanned raw and attached without parameter conversion or
    /// recursive expansion.
    fn append_unexpanded(&mut self, output: &mut Vec<TracedTokenWord>) -> Result<(), CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "unexpanded",
        })?;
        let raw = self.traced_words(scanned.replacement_text);
        let observed = raw
            .iter()
            .copied()
            .map(|token| self.observed_token(token))
            .collect::<Vec<_>>();
        output.extend(raw);
        self.command.expansion.cumulative_expansions = self
            .command
            .expansion
            .cumulative_expansions
            .saturating_add(1);
        // TeX82 §478 attaches `the_toks` only when `link(temp_head)<>null`.
        if !observed.is_empty() {
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "the_toks",
                    tokens: observed,
                }),
            );
        }
        Ok(())
    }

    /// e-TeX 2.6 etex.ch §53a returns `\detokenize` through `the_toks`.
    ///
    /// Inside §477's expanded collector the converted list is therefore
    /// attached directly, just like §478's ordinary `\the` result. It must
    /// not become a §470 `conv_toks` inserted input level whose characters
    /// are fetched again one by one.
    fn append_detokenize(&mut self, output: &mut Vec<TracedTokenWord>) -> Result<(), CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let text = crate::processor::expand::token_list_string_text(
            &mut self.state,
            scanned.replacement_text.token_list(),
        );
        let tokens = text
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
            .collect::<Vec<_>>();
        let observed = tokens
            .iter()
            .copied()
            .map(|token| self.observed_token(token))
            .collect::<Vec<_>>();
        output.extend(tokens);
        self.command.expansion.cumulative_expansions = self
            .command
            .expansion
            .cumulative_expansions
            .saturating_add(1);
        if !observed.is_empty() {
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "the_toks",
                    tokens: observed,
                }),
            );
        }
        Ok(())
    }

    pub(crate) fn expand_unexpanded(&mut self) -> Result<(), CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "unexpanded",
        })?;
        let first = self
            .state
            .tokens(scanned.replacement_text.token_list())
            .first()
            .copied();
        self.insert_expansion_list(
            TokenPayload::Stored {
                tokens: scanned.replacement_text.token_list(),
                origins: scanned.replacement_text.origin_list(),
            },
            first,
        );
        Ok(())
    }

    fn traced_words(&self, list: TracedTokenList) -> Vec<TracedTokenWord> {
        self.state
            .tokens(list.token_list())
            .iter()
            .copied()
            .zip(self.state.origin_list(list.origin_list()).iter().copied())
            .map(|(token, origin)| TracedTokenWord::pack(token, origin))
            .collect()
    }
}

fn is_parameter(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Parameter,
            ..
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

/// TeX82 §482's `read_toks`, the `\read`/`\readline` collector.
///
/// `read_toks` is deliberately not a `scan_toks` mode: it collects whole
/// _lines_ rather than a brace-balanced group, disables alignment delimiters
/// for its whole duration, and continues across a brace imbalance instead of
/// ending at a closing brace. It shares only the frozen-list result.
impl CommandProcessor<'_> {
    /// Collects TeX82 §482's `read_toks` list.
    ///
    /// `begin scanner_status:=defining; warning_index:=r; def_ref:=get_avail;
    /// token_ref_count(def_ref):=null; p:=def_ref; store_new_token(
    /// end_match_token); if (n<0)or(n>15) then m:=16 else m:=n; s:=align_state;
    /// align_state:=1000000; repeat <Input and store tokens from the next line
    /// of the file>; until align_state=1000000; cur_val:=def_ref;
    /// scanner_status:=normal; align_state:=s; end`.
    ///
    /// The result is a parameterless macro body: §482 stores `end_match_token`
    /// as the whole parameter text, which is Umber's empty parameter list.
    pub(crate) fn read_toks(
        &mut self,
        stream: i32,
        target: tex_state::interner::Symbol,
        raw_catcodes: bool,
    ) -> Result<TracedTokenList, CommandError> {
        let builder = TokenBuilderId(self.command.transient.next_builder_identity);
        self.command.transient.next_builder_identity =
            self.command.transient.next_builder_identity.wrapping_add(1);
        // §482: `scanner_status:=defining; warning_index:=r`.
        let status = ScannerStatus::Defining(DefinitionContext {
            target: Some(target),
            builder,
            warning: ScannerWarning(builder.0),
        });
        let prior = self.command.begin_scanner_status(status.clone());
        self.observe_scanner_status_transition(
            prior.status().clone(),
            self.command.scanner.status().clone(),
        );
        // §482: `s:=align_state; align_state:=1000000` disables tab marks and
        // `\cr` for the whole collection, and is restored whatever happens.
        let saved_align_state = self.command.alignment.align_state;
        self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
        let result = self.read_toks_lines(stream, target, raw_catcodes);
        self.command.alignment.align_state = saved_align_state;
        self.restore_scanner_status_with_observation(status, prior);
        let tokens = result?;
        // §482 leaves the collected list in `cur_val`; §1225 immediately
        // installs it with `define(p,call,cur_val)`. Unlike §473's
        // `scan_toks`, this is not an independently observable completed
        // token-list assignment. The committed observation is §1225's
        // meaning mutation, whose macro body includes §482's leading
        // `end_match_token`.
        Ok(self.state.finish_traced_token_list(&tokens))
    }

    /// §482's `repeat <Input and store tokens from the next line> until
    /// align_state=1000000`.
    fn read_toks_lines(
        &mut self,
        stream: i32,
        target: tex_state::interner::Symbol,
        raw_catcodes: bool,
    ) -> Result<Vec<TracedTokenWord>, CommandError> {
        // §482: `if (n<0)or(n>15) then m:=16 else m:=n`. Stream 16 is never
        // open, so §483 always takes §484's terminal branch for it.
        let slot = u8::try_from(stream)
            .ok()
            .filter(|slot| *slot < tex_state::world::STREAM_SLOT_COUNT as u8)
            .map(tex_state::world::StreamSlot::new);
        let mut tokens = Vec::new();
        // §484: "The value of `n` is set negative so that additional prompts
        // will not be given in the case of multi-line input."
        let mut prompted = false;
        loop {
            self.read_toks_line(slot, target, raw_catcodes, &mut prompted, &mut tokens)?;
            if self.command.alignment.align_state == TEMPLATE_ALIGN_STATE {
                return Ok(tokens);
            }
        }
    }

    /// §483's ⟨Input and store tokens from the next line of the file⟩.
    fn read_toks_line(
        &mut self,
        slot: Option<tex_state::world::StreamSlot>,
        target: tex_state::interner::Symbol,
        raw_catcodes: bool,
        prompted: &mut bool,
        tokens: &mut Vec<TracedTokenWord>,
    ) -> Result<(), CommandError> {
        // §483 calls `begin_file_reading` before §484-§486 acquire the line.
        // §328 establishes that new level with `name:=0`; the selected
        // stream classification is installed only after acquisition chooses
        // the stream rather than §484's terminal fallback.
        let level = self
            .command
            .begin_read_line()
            .map_err(|_| CommandError::input_invariant())?;
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::Source,
                source_name: Some(crate::input::SourceNameClass::Terminal),
                level: level.0,
                position: 0,
            }),
        );
        let (line, file_ended, name_class) = self.acquire_read_line(slot, target, prompted)?;
        self.command
            .finish_read_line(level, name_class, line.into_bytes())
            .map_err(|_| CommandError::input_invariant())?;
        if raw_catcodes {
            self.collect_read_line_verbatim(level, tokens)?;
            if file_ended {
                self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
            }
            return Ok(());
        }
        // §483: `loop get_token; if cur_tok=0 then goto done; if
        // align_state<1000000 then {unmatched `}' aborts the line} begin
        // repeat get_token until cur_tok=0; align_state:=1000000; goto done;
        // end; store_new_token(cur_tok); end`.
        while let Some(command) = self.get_token()? {
            if self.command.alignment.align_state < TEMPLATE_ALIGN_STATE {
                while self.get_token()?.is_some() {}
                self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
                return Ok(());
            }
            tokens.push(command.spelling());
        }
        if file_ended {
            self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
        }
        Ok(())
    }

    /// Collects one e-TeX `\readline` line.
    ///
    /// `\readline` reads the line with every character carrying category 12,
    /// or 10 for a space, whatever the current table says, so no control
    /// sequence, brace, or tab mark can form and §483's alignment and brace
    /// rules have nothing to act on. §483's line still ends at its
    /// `\endlinechar`, which is why the line is loaded and read through the
    /// same cursor rather than out of the acquired string.
    fn collect_read_line_verbatim(
        &mut self,
        level: crate::input::InputLevelId,
        tokens: &mut Vec<TracedTokenWord>,
    ) -> Result<(), CommandError> {
        let endlinechar = self
            .state
            .int_param(tex_state::env::banks::IntParam::END_LINE_CHAR);
        self.command.load_next_source_line(endlinechar);
        while let Some(character) = self.command.next_source_character() {
            let ch = crate::profile::token_character(character.code());
            let origin = self.state.source_token_origin(
                character.range().source(),
                character.range().start(),
                character.range().end(),
            );
            let cat = if ch == ' ' {
                Catcode::Space
            } else {
                Catcode::Other
            };
            tokens.push(TracedTokenWord::pack(Token::Char { ch, cat }, origin));
        }
        self.retire_read_line_level(level)?;
        Ok(())
    }

    /// §483's line acquisition: §484's terminal, or §485/§486's stream.
    ///
    /// The flag is §486's `input_ln` returning false: the stream has just
    /// closed, and "if align_state<>1000000 then begin runaway; print_err(
    /// "File ended within \read"); ... align_state:=1000000; limit:=0;
    /// error; end". The line itself is still read and tokenized -- it is
    /// §486's one appended empty line -- so only the brace count is reset.
    fn acquire_read_line(
        &mut self,
        slot: Option<tex_state::world::StreamSlot>,
        target: tex_state::interner::Symbol,
        prompted: &mut bool,
    ) -> Result<(String, bool, crate::input::SourceNameClass), CommandError> {
        // §483: `if read_open[m]=closed then <terminal> else <the file>`.
        if let Some(slot) = slot
            && !self.state.read_stream_at_eof(slot)
            && let Some(line) = self.state.input_ln(CommandLineSource::Stream(slot))
        {
            let ended = self.state.read_stream_at_eof(slot);
            return Ok((
                line,
                ended,
                crate::input::SourceNameClass::ReadStream(slot.raw()),
            ));
        }
        // §484: `if interaction>nonstop_mode then if n<0 then
        // prompt_input("") else begin wake_up_terminal; print_ln; sprint_cs(r);
        // prompt_input("="); n:=-1; end else fatal_error(...)`.
        if !self.state.interaction_permits_terminal_input() {
            return Err(CommandError::input_invariant());
        }
        let prompt = if *prompted {
            String::new()
        } else {
            *prompted = true;
            format!("\n\\{}=", self.state.resolve(target))
        };
        let line = self
            .state
            .input_ln(CommandLineSource::Terminal { prompt: &prompt })
            .ok_or_else(CommandError::input_invariant)?;
        Ok((line, false, crate::input::SourceNameClass::Terminal))
    }
}

#[cfg(test)]
mod tests;
