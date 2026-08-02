//! Private canonical scalar macro-call state machine.
#![allow(dead_code)] // expansion dispatch is the next ordered integration slice
use std::sync::Arc;

use tex_state::env::banks::IntParam;
use tex_state::ids::MacroDefinitionId;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::input::SharedTokenBuffer;
use crate::processor::status::{ArgumentBuilderId, MatchingContext, ScannerStatus, ScannerWarning};
use crate::{CommandError, CommandProcessor};

use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, MacroRecord, TokenListRecord,
};

const EXTRA_RIGHT_BRACE_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0395;
const RUNAWAY_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0396;

/// Persistent ownership of live macro-argument activations.
///
/// This is the sole owner of the activation chain. Macro-body input behavior
/// carries a typed activation identity, while parameter payloads retain shared
/// ownership of the one contiguous argument allocation.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ParameterState {
    pub(crate) activations: Vec<MacroActivation>,
    pub(crate) next_activation_identity: u64,
}

/// Typed identity of one live macro activation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct MacroActivationId(pub(crate) u64);

/// One live macro call and the materialized arguments it owns.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroActivation {
    pub(crate) identity: MacroActivationId,
    /// TeX82 §389's `warning_index`: the control sequence being expanded.
    /// §314 prints it as this level's context descriptor.
    pub(crate) name: Symbol,
    pub(crate) definition: MacroDefinitionId,
    pub(crate) arguments: MacroArguments,
    pub(crate) invocation: OriginId,
}

/// One contiguous macro-argument allocation and its at-most-nine ranges.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct MacroArguments {
    pub(crate) buffer: SharedTokenBuffer,
    pub(crate) ranges: [Option<MacroArgumentRange>; 9],
}

/// Exhaustive result of TeX82's `macro_call`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MacroCallOutcome {
    Activated,
    PrefixMismatchRecovered,
}

/// Incremental construction of one canonical macro activation.
///
/// The scalar matcher completes arguments in definition order. Each completed
/// argument is appended once to this one buffer; its slot records only a
/// half-open range, so parameter replay never duplicates argument tokens.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct MacroArgumentBuilder {
    tokens: Vec<TracedTokenWord>,
    ranges: [Option<MacroArgumentRange>; 9],
    next_slot: u8,
}

/// A malformed attempt to finish one macro argument.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MacroArgumentBuildError {
    InvalidSlot(u8),
    OutOfOrderSlot { expected: u8, actual: u8 },
}

/// Compact interpretation of a parameter-related replacement token.
///
/// `Token::Param` is the canonical compact out-parameter representation.
/// A literal parameter character reaches a replacement list only through
/// TeX's `##` escape, and remains an ordinary character during replay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MacroParameterEscape {
    OutParameter(u8),
    EscapedParameter,
}

impl MacroParameterEscape {
    pub(crate) const fn classify(token: Token) -> Option<Self> {
        match token {
            Token::Param(slot) => Some(Self::OutParameter(slot)),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            } => Some(Self::EscapedParameter),
            _ => None,
        }
    }
}

/// A half-open range within a macro activation's shared argument buffer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroArgumentRange {
    start: usize,
    end: usize,
}

impl MacroArgumentRange {
    pub(crate) const fn new(start: usize, end: usize) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    pub(crate) const fn start(self) -> usize {
        self.start
    }

    pub(crate) const fn end(self) -> usize {
        self.end
    }
}

impl MacroArgumentBuilder {
    /// Completes the next argument in canonical definition order.
    pub(crate) fn complete(
        &mut self,
        slot: u8,
        argument: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<(), MacroArgumentBuildError> {
        if !(1..=9).contains(&slot) {
            return Err(MacroArgumentBuildError::InvalidSlot(slot));
        }
        let expected = self.next_slot + 1;
        if slot != expected {
            return Err(MacroArgumentBuildError::OutOfOrderSlot {
                expected,
                actual: slot,
            });
        }
        let start = self.tokens.len();
        self.tokens.extend(argument);
        let end = self.tokens.len();
        self.ranges[usize::from(slot - 1)] = MacroArgumentRange::new(start, end);
        self.next_slot = slot;
        Ok(())
    }

    /// Freezes the single shared argument allocation for one activation.
    #[must_use]
    pub(crate) fn finish(self) -> MacroArguments {
        MacroArguments {
            buffer: SharedTokenBuffer::new(Arc::from(self.tokens)),
            ranges: self.ranges,
        }
    }
}

impl ParameterState {
    /// Installs the sole owner of a macro activation before its body level is
    /// pushed. The caller immediately associates the returned identity with
    /// `TokenBehavior::MacroBody`, making retirement an atomic ownership pair.
    pub(crate) fn push_activation(
        &mut self,
        name: Symbol,
        definition: MacroDefinitionId,
        arguments: MacroArguments,
        invocation: OriginId,
    ) -> MacroActivationId {
        let identity = MacroActivationId(self.next_activation_identity);
        self.next_activation_identity = self.next_activation_identity.wrapping_add(1);
        self.activations.push(MacroActivation {
            identity,
            name,
            definition,
            arguments,
            invocation,
        });
        identity
    }

    pub(crate) fn parent_invocation(&self) -> OriginId {
        self.activations
            .last()
            .map(|activation| activation.invocation)
            .unwrap_or(OriginId::UNKNOWN)
    }
}

impl CommandProcessor<'_> {
    /// TeX82 §323's diagnostic for a named token-list parameter installed by
    /// `begin_token_list`. Unlike ordinary macro calls, these lists trace only
    /// when `\tracingmacros>1`.
    pub(crate) fn report_named_token_list(
        &mut self,
        name: &str,
        tokens: tex_state::ids::TokenListId,
    ) {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 1 {
            return;
        }
        let mut text = format!("\\{name}->");
        for token in self.state.tokens(tokens).to_vec() {
            text.push_str(&crate::processor::expand::token_list_token_text(
                &self.state,
                token,
            ));
        }
        // §323 uses `print_nl`, unlike §389's unconditional `print_ln` for
        // an ordinary macro invocation. At an existing line boundary this
        // must not introduce a blank line before the named list.
        let mut output = self.state.begin_diagnostic();
        output.print_nl(&text);
        output.end(false);
    }

    /// TeX.web's scalar `macro_call` path for compulsory parameter text,
    /// literal argument matching, and replacement activation.
    pub(crate) fn macro_call(
        &mut self,
        call: crate::CurrentCommand,
    ) -> Result<MacroCallOutcome, CommandError> {
        let Meaning::Macro { definition, .. } = call.meaning() else {
            return Err(CommandError::input_invariant());
        };
        let macro_name = call
            .control_sequence()
            .ok_or(CommandError::input_invariant())?;
        let meaning = self.state.macro_definition(definition);
        let pattern = self.state.macro_definition_parameter_pattern(definition);
        self.trace_macro_invocation(
            macro_name,
            meaning.parameter_text(),
            meaning.replacement_text(),
        );
        // TeX82 §389 calls the §391 parameter matcher only when the macro's
        // parameter text does not begin with `end_match`. A parameterless
        // macro therefore feeds its replacement directly, without a transient
        // `matching` scanner episode. Literal leading tokens still need the
        // matcher even when there are no numbered parameters.
        let needs_matching = !pattern.leading().is_empty() || pattern.parameter_count() != 0;
        let prior = if needs_matching {
            let builder = ArgumentBuilderId(self.command.transient.next_builder_identity);
            self.command.transient.next_builder_identity =
                self.command.transient.next_builder_identity.wrapping_add(1);
            let status = ScannerStatus::Matching(MatchingContext {
                macro_name,
                builder,
                // The diagnostic/provenance bridge assigns stable warning ids in
                // its own ordered slice; matching nevertheless owns a typed live
                // warning slot now so outer recovery has one canonical path.
                warning: ScannerWarning(0),
            });
            let prior = self.command.begin_scanner_status(status.clone());
            self.observe_scanner_status_transition(
                prior.status().clone(),
                self.command.scanner.status().clone(),
            );
            Some((prior, status))
        } else {
            None
        };
        self.outer_recovered_while_matching = false;
        self.eof_recovered_while_matching = false;
        let arguments = match self.macro_call_scalar(definition, meaning.flags(), &pattern) {
            Ok(arguments) => arguments,
            Err(CommandError::MacroPrefixMismatch) => {
                // TeX82 §391 reports the mismatch through `error` and returns
                // from `macro_call`; the mismatching token stays consumed and
                // no replacement text is installed.
                self.command.semantic_diagnostics.push(
                    crate::CommandSemanticDiagnostic::MacroPrefixMismatch(macro_name),
                );
                self.observe_command_diagnostic("macro_prefix_mismatch", &call);
                if let Some((prior, status)) = prior {
                    self.restore_scanner_status_with_observation(status, prior);
                }
                return Ok(MacroCallOutcome::PrefixMismatchRecovered);
            }
            Err(error) => {
                if let Some((prior, status)) = prior {
                    self.restore_scanner_status_with_observation(status, prior);
                }
                return Err(error);
            }
        };

        // TeX.web §§391--400 freezes the completed ranges before replacing
        // the input. The activation owns that one shared buffer; its body
        // replays the canonical immutable replacement list and resolves
        // compact `OutParameter` tokens through that owner.
        // TeX82 §390's replacement hand-off first drains every depleted token
        // list -- the exhausted macro body or replayed parameter the call
        // token itself came from, any backup or recovery insertion, and any
        // finished stored replay -- before `begin_token_list(..., macro)`.
        // Those retirements must precede this body's input push.
        self.conserve_input_stack()?;
        let provenance = self.state.macro_definition_provenance(definition);
        let _level = self.push_macro_activation(
            macro_name,
            definition,
            call.spelling().origin(),
            arguments.clone(),
            meaning.replacement_text(),
            provenance.replacement_origins(),
        );
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::Macro,
                source_name: None,
                level: _level.0,
                position: 0,
            }),
        );
        observe!(
            self,
            CommandObservation::Macro(MacroRecord {
                activation: true,
                definition: u64::from(definition.raw()),
                control_sequence: Some(self.state.resolve(macro_name).to_owned()),
                argument: Some(pattern.parameter_count() as u8),
                token_count: arguments.buffer.len() as u64,
                tokens: Vec::new(),
            }),
        );
        if let Some((prior, status)) = prior {
            self.restore_scanner_status_with_observation(status, prior);
        }
        Ok(MacroCallOutcome::Activated)
    }

    fn macro_call_scalar(
        &mut self,
        _definition: MacroDefinitionId,
        flags: MeaningFlags,
        pattern: &tex_state::macro_store::MacroParameterPattern,
    ) -> Result<MacroArguments, CommandError> {
        for expected in pattern.leading() {
            let actual = self.get_token()?.ok_or(CommandError::MacroPrefixMismatch)?;
            if (self.outer_recovered_while_matching && is_paragraph_command(&actual))
                || actual.spelling().semantic_token() != *expected
            {
                return Err(if self.outer_recovered_while_matching {
                    CommandError::OuterInMacroArgument
                } else {
                    CommandError::MacroPrefixMismatch
                });
            }
        }

        let mut arguments = MacroArgumentBuilder::default();
        for parameter in 0..pattern.parameter_count() {
            let delimiter = pattern.delimiter(parameter);
            let argument = if delimiter.is_empty() {
                self.scan_undelimited_argument(flags)?
            } else {
                self.scan_delimited_argument(flags, delimiter)?
            };
            self.trace_macro_argument(parameter + 1, &argument);
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "macro_delimiter_match",
                    tokens: delimiter
                        .iter()
                        .copied()
                        .map(|token| {
                            self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN))
                        })
                        .collect(),
                }),
            );
            observe!(
                self,
                CommandObservation::Macro(MacroRecord {
                    activation: false,
                    definition: u64::from(_definition.raw()),
                    control_sequence: None,
                    argument: Some((parameter + 1) as u8),
                    token_count: argument.len() as u64,
                    tokens: argument
                        .iter()
                        .copied()
                        .map(|token| self.observed_token(token))
                        .collect(),
                }),
            );
            arguments
                .complete((parameter + 1) as u8, argument)
                .map_err(|_| CommandError::input_invariant())?;
        }
        Ok(arguments.finish())
    }

    /// TeX82 §389's invocation trace, including `print_ln` before the macro
    /// name and §262's control-word separator before `->`.
    fn trace_macro_invocation(
        &mut self,
        macro_name: tex_state::interner::Symbol,
        parameters: tex_state::ids::TokenListId,
        replacement: tex_state::ids::TokenListId,
    ) {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 0 {
            return;
        }
        let mut text = crate::processor::expand::print_cs_text(&mut self.state, macro_name);
        for token in self.state.tokens(parameters).to_vec() {
            text.push_str(&crate::processor::expand::token_list_token_text(
                &self.state,
                token,
            ));
        }
        text.push_str("->");
        for token in self.state.tokens(replacement).to_vec() {
            text.push_str(&crate::processor::expand::token_list_token_text(
                &self.state,
                token,
            ));
        }
        self.print_macro_trace(text, true);
    }

    /// TeX82 §400's `#n<-<argument>` trace in completed-argument order.
    fn trace_macro_argument(&mut self, parameter: usize, argument: &[TracedTokenWord]) {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 0 {
            return;
        }
        let mut text = format!("#{parameter}<-");
        for word in argument {
            text.push_str(&crate::processor::expand::token_list_token_text(
                &self.state,
                word.semantic_token(),
            ));
        }
        self.print_macro_trace(text, false);
    }

    /// Prints a TeX82 §389/§400 macro diagnostic at the point `macro_call`
    /// reaches it. Only deferred-write expansion postpones printing until its
    /// owning executor episode can restore the live diagnostic selector.
    fn print_macro_trace(&mut self, text: String, force_newline: bool) {
        if self.command.expanding_deferred_write() {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Trace {
                    text,
                    force_newline,
                });
            return;
        }
        let mut output = self.state.begin_diagnostic();
        if force_newline {
            output.print_ln().print(&text);
        } else {
            output.print_nl(&text);
        }
        output.end(false);
    }

    /// TeX82 §389's `warning_index`, spelled as §395/§396 print it.
    ///
    /// Both report the macro whose argument was being matched with
    /// `sprint_cs(warning_index)`, and that is exactly what the live
    /// `matching` scanner status carries, so the name is read back from there
    /// rather than threaded through every argument scanner.
    fn matching_macro_name(&self) -> String {
        match self.command.scanner.status() {
            ScannerStatus::Matching(context) => {
                let spelling = self.state.resolve(context.macro_name).to_owned();
                crate::processor::expand::print_esc_text(&self.state, &spelling)
            }
            _ => String::new(),
        }
    }

    /// TeX82 §396's `<Report a runaway argument and abort>`.
    ///
    /// §396 issues this only for `long_state=call` -- a `\long` macro accepts
    /// the paragraph instead -- which is the same test the caller has already
    /// made before reaching here.
    fn report_paragraph_ended_before_complete(&mut self, partial: &[TracedTokenWord]) {
        let name = self.matching_macro_name();
        let context = self.command.output_open_context(&self.state);
        let mut display = String::new();
        for token in partial {
            display.push_str(&crate::processor::expand::token_list_token_text(
                &self.state,
                token.semantic_token(),
            ));
        }
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: RUNAWAY_ARGUMENT_DIAGNOSTIC,
                runaway: Some(crate::state::RunawayPrelude {
                    heading: "Runaway argument?",
                    partial: display,
                }),
                message: format!("Paragraph ended before {name} was complete"),
                help: &[
                    "I suspect you've forgotten a `}', causing me to apply this",
                    "control sequence to too much text. How can we recover?",
                    "My plan is to forget the whole thing and hope for the best.",
                ],
                context,
            });
    }

    /// TeX82 §395's `<Report an extra right brace and goto continue>`.
    fn report_extra_right_brace_argument(&mut self) {
        let name = self.matching_macro_name();
        let context = self.command.output_open_context(&self.state);
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: EXTRA_RIGHT_BRACE_ARGUMENT_DIAGNOSTIC,
                runaway: None,
                message: format!("Argument of {name} has an extra }}"),
                help: &[
                    "I've run across a `}' that doesn't seem to match anything.",
                    "For example, `\\def\\a#1{...}' and `\\a}' would produce",
                    "this error. If you simply proceed now, the `\\par' that",
                    "I've just inserted will cause me to report a runaway",
                    "argument that might be the root of the problem. But if",
                    "your `}' was spurious, just type `2' and it will go away.",
                ],
                context,
            });
    }

    fn scan_undelimited_argument(
        &mut self,
        flags: MeaningFlags,
    ) -> Result<Vec<TracedTokenWord>, CommandError> {
        let first = loop {
            let command = self
                .get_token()?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            if self.outer_recovered_while_matching && is_paragraph_command(&command) {
                return Err(CommandError::OuterInMacroArgument);
            }
            if matches!(
                command.spelling().semantic_token(),
                Token::Char {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                continue;
            }
            if matches!(
                command.spelling().semantic_token(),
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                }
            ) {
                // TeX82 §395 backs up a bare extra `}`, inserts frozen
                // `\par`, and sets `long_state := call`. The inserted
                // paragraph therefore takes §394's ordinary back_error path
                // even when this macro was originally declared `\long`.
                self.back_input(command)?;
                self.insert_macro_argument_recovery_par()?;
                // §395 ends with `ins_error`, so §82 renders the context with
                // the inserted `\par` level already on the stack.
                self.report_extra_right_brace_argument();
                let par = self
                    .get_token()?
                    .ok_or(CommandError::ParagraphInMacroArgument)?;
                self.back_input(par)?;
                // §395's `goto continue` returns to the matching loop, which
                // immediately reads the `\par` it just inserted and takes
                // §394's abort. `long_state:=call` above is there precisely so
                // that §396 reports even for a `\long` macro.
                self.report_paragraph_ended_before_complete(&[]);
                return Err(CommandError::ParagraphInMacroArgument);
            }
            break command;
        };
        self.check_argument_paragraph(&first, flags, &[])?;
        if !matches!(
            first.spelling().semantic_token(),
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            }
        ) {
            return Ok(vec![first.spelling()]);
        }

        let mut depth = 1_u32;
        let mut tokens = Vec::new();
        loop {
            let command = self
                .get_token()?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            if self.outer_recovered_while_matching && is_paragraph_command(&command) {
                self.set_runaway_partial(&tokens);
                return Err(CommandError::OuterInMacroArgument);
            }
            self.check_argument_paragraph(&command, flags, &tokens)?;
            match command.spelling().semantic_token() {
                Token::Char {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    depth += 1;
                    tokens.push(command.spelling());
                }
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                } => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(tokens);
                    }
                    tokens.push(command.spelling());
                }
                _ => tokens.push(command.spelling()),
            }
        }
    }

    /// TeX.web §394's literal, scalar delimiter matcher. A failed delimiter
    /// prefix commits only the part that cannot be reused as an overlapping
    /// prefix, then retains the maximal suffix that still matches the start
    /// of the delimiter. This is intentionally not a compiled string matcher:
    /// token catcodes, brace depth, and the recovery splice are semantic here.
    fn scan_delimited_argument(
        &mut self,
        flags: MeaningFlags,
        delimiter: &[Token],
    ) -> Result<Vec<TracedTokenWord>, CommandError> {
        debug_assert!(!delimiter.is_empty());
        let mut tokens = Vec::new();
        let mut prefix = Vec::with_capacity(delimiter.len());
        let mut depth = 0_u32;
        let mut current = None;

        loop {
            let command = match current.take() {
                Some(command) => command,
                None => self
                    .get_token()?
                    .ok_or(CommandError::ParagraphInMacroArgument)?,
            };
            if self.outer_recovered_while_matching && is_paragraph_command(&command) {
                let mut partial = tokens.clone();
                partial.extend(prefix.iter().copied());
                self.set_runaway_partial(&partial);
                return Err(CommandError::OuterInMacroArgument);
            }
            let token = command.spelling().semantic_token();

            if depth == 0 && token == delimiter[prefix.len()] {
                prefix.push(command.spelling());
                if prefix.len() == delimiter.len() {
                    // `#{` consumes the opening brace as parameter text. Raw
                    // delivery has accounted for it, but no replacement-body
                    // replay exists yet to provide the balancing delivery.
                    if is_begin_group(token) {
                        self.undo_delimiter_begin_group_delivery();
                    }
                    return Ok(strip_one_outer_group(tokens));
                }
                continue;
            }

            if !prefix.is_empty() {
                let retained = overlapping_delimiter_prefix(&prefix, command.spelling(), delimiter);
                let committed = if retained == 0 {
                    prefix.len()
                } else {
                    prefix.len() + 1 - retained
                };
                for prefix_token in prefix.drain(..committed) {
                    observe!(
                        self,
                        CommandObservation::TokenList(TokenListRecord {
                            transition: "splice",
                            purpose: "macro_delimiter_recovery",
                            tokens: vec![self.observed_token(prefix_token)],
                        }),
                    );
                    push_delimited_argument_token(&mut tokens, &mut depth, prefix_token);
                }
                if retained != 0 {
                    prefix.push(command.spelling());
                    continue;
                }

                // The mismatching token cannot continue the delimiter, so it
                // becomes ordinary argument material after the committed
                // prefix. TeX.web §394 permits a recovered `\par` prefix;
                // only this newly ordinary token is subject to the non-long
                // paragraph check.
                self.check_argument_paragraph(&command, flags, &tokens)?;
                push_delimited_argument_token(&mut tokens, &mut depth, command.spelling());
                continue;
            }

            self.check_argument_paragraph(&command, flags, &tokens)?;
            push_delimited_argument_token(&mut tokens, &mut depth, command.spelling());
        }
    }

    fn check_argument_paragraph(
        &mut self,
        command: &crate::CurrentCommand,
        flags: MeaningFlags,
        partial: &[TracedTokenWord],
    ) -> Result<(), CommandError> {
        if matches!(
            command.meaning(),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)
        ) && !flags.contains(MeaningFlags::LONG)
        {
            if self.eof_recovered_while_matching {
                // TeX82 §23 calls `check_outer_validity` after source EOF;
                // §394 then aborts this match on its inserted frozen `\par`.
                // That terminator is consumed by the failed expansion, unlike
                // a user-supplied paragraph which `back_error` must replay.
                self.set_runaway_partial(partial);
                return Err(CommandError::ParagraphInMacroArgument);
            }
            // TeX82 §394 reports this through `back_error` while the macro
            // matcher is still live.  The caller will then restore its
            // enclosing scanner status, so retain the exact `\par` input
            // ahead of that restoration rather than merely returning an
            // error from the scalar matcher.
            self.back_input(command.copy_for_backup())?;
            // §396 ends with `back_error`, so §82 renders the context with the
            // replayed `\par` already on the stack.
            self.report_paragraph_ended_before_complete(partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        Ok(())
    }
}

/// TeX82 §394 aborts a match on the recovery paragraph that follows its
/// synthetic outer-validity space, not on that space itself.
fn is_paragraph_command(command: &crate::CurrentCommand) -> bool {
    matches!(
        command.meaning(),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)
    )
}

/// Returns the number of tokens from `prefix` plus `current` that form the
/// longest proper delimiter prefix after a mismatch. TeX.web §394 compares
/// these scalar token sequences directly while moving the unmatched leading
/// tokens into the completed argument.
fn overlapping_delimiter_prefix(
    prefix: &[TracedTokenWord],
    current: TracedTokenWord,
    delimiter: &[Token],
) -> usize {
    let pending_len = prefix.len() + 1;
    (1..pending_len.min(delimiter.len()))
        .rev()
        .find(|&candidate_len| {
            (0..candidate_len).all(|index| {
                let pending = pending_len - candidate_len + index;
                let token = prefix
                    .get(pending)
                    .copied()
                    .unwrap_or(current)
                    .semantic_token();
                token == delimiter[index]
            })
        })
        .unwrap_or(0)
}

fn push_delimited_argument_token(
    tokens: &mut Vec<TracedTokenWord>,
    depth: &mut u32,
    token: TracedTokenWord,
) {
    match token.semantic_token() {
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => *depth = depth.saturating_add(1),
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } if *depth > 0 => *depth -= 1,
        _ => {}
    }
    tokens.push(token);
}

fn strip_one_outer_group(mut tokens: Vec<TracedTokenWord>) -> Vec<TracedTokenWord> {
    if tokens.len() < 2
        || !is_begin_group(tokens[0].semantic_token())
        || !is_end_group(tokens[tokens.len() - 1].semantic_token())
    {
        return tokens;
    }

    let mut depth = 0_u32;
    for (index, token) in tokens.iter().enumerate() {
        match token.semantic_token() {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => depth = depth.saturating_add(1),
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } if depth > 0 => {
                depth -= 1;
                if depth == 0 && index + 1 != tokens.len() {
                    return tokens;
                }
            }
            _ => {}
        }
    }
    if depth == 0 {
        tokens.pop();
        tokens.remove(0);
    }
    tokens
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
