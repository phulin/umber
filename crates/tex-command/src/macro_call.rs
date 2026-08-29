//! Private canonical scalar macro-call state machine.
#![allow(dead_code)] // expansion dispatch is the next ordered integration slice
use tex_state::DefinitionId;
use tex_state::env::banks::IntParam;
use tex_state::interner::Symbol;
use tex_state::macro_definition::MacroParameterPattern;
use tex_state::meaning::{Meaning, MeaningFlags, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::execution_scratch::{
    MacroArgumentTokenFacts, MacroFrameId, MacroMatch, MacroMatchBuffer, MacroWords,
};
use crate::processor::status::{
    ArgumentBuilderId, MatchingContext, ScannerStatus, ScannerStatusVisibility, ScannerWarning,
};
use crate::{CommandError, CommandProcessor};

use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, MacroRecord, TokenListRecord,
};

const EXTRA_RIGHT_BRACE_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0395;
pub(crate) const RUNAWAY_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0396;

/// Semantic ownership of live macro activations.
///
/// Macro-body input behavior carries a typed activation identity. Each
/// activation holds only a private generation-branded descriptor for its
/// stable execution-scratch slot.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct ParameterState<G> {
    pub(crate) activations: crate::timeline::LogicalStack<MacroActivation<G>>,
    pub(crate) next_activation_identity: u64,
}

impl<G> Default for ParameterState<G> {
    fn default() -> Self {
        Self {
            activations: crate::timeline::LogicalStack::default(),
            next_activation_identity: 0,
        }
    }
}

/// Typed identity of one live macro activation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct MacroActivationId(pub(crate) u64);

/// One live macro call and its stable scratch-slot descriptor.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroActivation<G> {
    pub(crate) identity: MacroActivationId,
    /// TeX82 §389's `warning_index`: the control sequence being expanded.
    /// §314 prints it as this level's context descriptor.
    pub(crate) name: Symbol,
    pub(crate) definition: DefinitionId<G>,
    pub(crate) arguments: MacroArguments<G>,
    pub(crate) invocation: OriginId,
}

impl<G> crate::timeline::LogicalStackElement for MacroActivation<G> {
    type InlineState = ();
    type StoredState = ();

    fn capture_state(
        &self,
    ) -> crate::timeline::CapturedStackState<Self::InlineState, Self::StoredState> {
        crate::timeline::CapturedStackState::Inline(())
    }

    fn swap_inline_state(&mut self, (): &mut Self::InlineState) {}

    fn swap_stored_state(&mut self, (): &mut Self::StoredState) {}
}

/// Private descriptor for one sealed at-most-nine-argument scratch slot.
#[derive(Debug)]
pub(crate) struct MacroArguments<G> {
    frame: MacroFrameId<G>,
}

impl<G> Copy for MacroArguments<G> {}

impl<G> Clone for MacroArguments<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> PartialEq for MacroArguments<G> {
    fn eq(&self, other: &Self) -> bool {
        self.frame == other.frame
    }
}

impl<G> Eq for MacroArguments<G> {}

impl<G> core::hash::Hash for MacroArguments<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.frame.hash(state);
    }
}

/// Exhaustive result of TeX82's `macro_call`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MacroCallOutcome {
    Activated,
    PrefixMismatchRecovered,
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

#[derive(Debug, Eq, Hash, PartialEq)]
struct MacroDelimiter<G> {
    definition: DefinitionId<G>,
    start: usize,
    len: usize,
}

impl<G> Clone for MacroDelimiter<G> {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            start: self.start,
            len: self.len,
        }
    }
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

impl<G> ParameterState<G> {
    /// Installs the sole owner of a macro activation before its body level is
    /// pushed. The caller immediately associates the returned identity with
    /// `TokenBehavior::MacroBody`, making retirement an atomic ownership pair.
    pub(crate) fn push_activation(
        &mut self,
        name: Symbol,
        definition: DefinitionId<G>,
        arguments: MacroArguments<G>,
        invocation: OriginId,
    ) -> MacroActivationId {
        let identity = MacroActivationId(self.next_activation_identity);
        self.next_activation_identity = self.next_activation_identity.wrapping_add(1);
        self.install_activation(identity, name, definition, arguments, invocation);
        identity
    }

    pub(crate) fn restore_activation(
        &mut self,
        identity: MacroActivationId,
        name: Symbol,
        definition: DefinitionId<G>,
        arguments: MacroArguments<G>,
        invocation: OriginId,
    ) {
        self.install_activation(identity, name, definition, arguments, invocation);
    }

    fn install_activation(
        &mut self,
        identity: MacroActivationId,
        name: Symbol,
        definition: DefinitionId<G>,
        arguments: MacroArguments<G>,
        invocation: OriginId,
    ) {
        self.activations.push(MacroActivation {
            identity,
            name,
            definition,
            arguments,
            invocation,
        });
    }

    pub(crate) fn parent_invocation(&self) -> OriginId {
        self.activations
            .last()
            .map_or(OriginId::UNKNOWN, |activation| activation.invocation)
    }

    pub(crate) fn active_invocation_origin(&self) -> Option<OriginId> {
        self.activations
            .last()
            .map(|activation| activation.invocation)
    }

    pub(crate) fn retire_last_activation(&mut self) -> Option<MacroArguments<G>> {
        self.activations
            .pop_project(|activation| activation.arguments)
    }
}

impl<G> MacroArguments<G> {
    pub(crate) const fn new(frame: MacroFrameId<G>) -> Self {
        Self { frame }
    }

    pub(crate) const fn frame(self) -> MacroFrameId<G> {
        self.frame
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    /// TeX82 §323's diagnostic for a named token-list parameter installed by
    /// `begin_token_list`. Unlike ordinary macro calls, these lists trace only
    /// when `\tracingmacros>1`.
    pub(crate) fn report_named_token_list(
        &mut self,
        name: &str,
        tokens: tex_state::TokenListId<G>,
    ) {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 1 {
            return;
        }
        let mut text = String::new();
        crate::processor::expand::append_print_esc_text(self.state, name, &mut text);
        text.push_str("->");
        for word in self.state.token_list(tokens) {
            let token = word.semantic_token();
            crate::processor::expand::append_token_list_token_text(self.state, token, &mut text);
        }
        // §323 uses `print_nl`, unlike §389's unconditional `print_ln` for
        // an ordinary macro invocation. At an existing line boundary this
        // must not introduce a blank line before the named list.
        let mut output = self.begin_diagnostic();
        output.print_nl(&text);
        output.end(false);
    }

    /// TeX.web's scalar `macro_call` path for compulsory parameter text,
    /// literal argument matching, and replacement activation.
    /// Executes a live macro command without cloning its structural origin.
    /// The ordinary expansion loop owns the command for the complete call and
    /// moves it into retry state only when a typed resource barrier is hit.
    pub(crate) fn macro_call(
        &mut self,
        call: &crate::CurrentCommand<G>,
    ) -> Result<MacroCallOutcome, CommandError> {
        let ResolvedMeaning::Macro { flags, definition } = call.meaning_ref() else {
            return Err(CommandError::input_invariant());
        };
        let macro_name = call
            .control_sequence()
            .ok_or(CommandError::input_invariant())?;
        let matching = self
            .command
            .scratch
            .begin_macro_match()
            .map_err(|_| CommandError::input_invariant())?;
        let definition_view = self.state.definition(definition.clone());
        let pattern = definition_view.parameter_pattern();
        let parameter_len = definition_view.parameter_text().len();
        self.trace_macro_invocation(macro_name, definition.clone());
        // TeX82 §389 calls the §391 parameter matcher only when the macro's
        // parameter text does not begin with `end_match`. A parameterless
        // macro therefore feeds its replacement directly, without a transient
        // `matching` scanner episode. Literal leading tokens still need the
        // matcher even when there are no numbered parameters.
        let needs_matching =
            pattern.leading_end(parameter_len) != 0 || pattern.parameter_count() != 0;
        let episode = if needs_matching {
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
            Some(self.begin_scanner_episode(status, ScannerStatusVisibility::Observed))
        } else {
            None
        };
        self.outer_recovered_while_matching = false;
        self.eof_recovered_while_matching = false;
        let scanned_arguments = self.macro_call_scalar(
            &matching,
            macro_name,
            definition.clone(),
            *flags,
            pattern,
            parameter_len,
        );
        match scanned_arguments {
            Ok(()) => {}
            Err(CommandError::MacroPrefixMismatch) => {
                // TeX82 §391 reports the mismatch through `error` and returns
                // from `macro_call`; the mismatching token stays consumed and
                // no replacement text is installed.
                // TeX82 §391 calls `error` before returning from `macro_call`.
                // Capture §82's context while the mismatching input level is
                // still live; in particular, §336's frozen `\par` retains its
                // `<inserted text>` ownership until this report is complete.
                let context = self.command.output_open_context(self.state);
                self.command.semantic_diagnostics.push(
                    crate::CommandSemanticDiagnostic::MacroPrefixMismatch {
                        macro_name,
                        context,
                    },
                );
                self.observe_command_diagnostic("macro_prefix_mismatch", call);
                if let Some(episode) = episode {
                    self.finish_scanner_episode(episode);
                }
                self.command
                    .scratch
                    .discard_macro_match(matching)
                    .map_err(|_| CommandError::input_invariant())?;
                return Ok(MacroCallOutcome::PrefixMismatchRecovered);
            }
            Err(error) => {
                if let Some(episode) = episode {
                    self.finish_scanner_episode(episode);
                }
                self.command
                    .scratch
                    .discard_macro_match(matching)
                    .map_err(|_| CommandError::input_invariant())?;
                return Err(error);
            }
        }

        // TeX.web §§391--400 freezes the completed ranges before replacing
        // the input. The activation names one command-arena argument span;
        // its body replays the admitted immutable replacement span and
        // resolves compact `OutParameter` tokens through that coordinate.
        // TeX82 §390's replacement hand-off first drains every depleted token
        // list -- the exhausted macro body or replayed parameter the call
        // token itself came from, any backup or recovery insertion, and any
        // finished stored replay -- before `begin_token_list(..., macro)`.
        // Those retirements must precede this body's input push. The pending
        // frame stays canonical if an older active frame retires beneath it.
        self.conserve_input_stack()?;
        let frame = self
            .command
            .scratch
            .commit_macro_match(matching)
            .map_err(|_| CommandError::input_invariant())?;
        let arguments = MacroArguments::new(frame);
        let _level =
            self.push_macro_activation(macro_name, definition.clone(), call.origin(), arguments);
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::Macro,
                source_name: None,
                source: None,
                level: _level.0,
                position: 0,
            }),
        );
        observe!(
            self,
            CommandObservation::Macro(MacroRecord::Activation {
                control_sequence: self.state.resolve(macro_name).to_owned(),
                argument_count: pattern.parameter_count() as u8,
                token_count: self.argument_token_count(arguments) as u64,
            }),
        );
        if let Some(episode) = episode {
            self.finish_scanner_episode(episode);
        }
        Ok(MacroCallOutcome::Activated)
    }

    fn macro_call_scalar(
        &mut self,
        matching: &MacroMatch<G>,
        macro_name: tex_state::interner::Symbol,
        definition: DefinitionId<G>,
        flags: MeaningFlags,
        pattern: MacroParameterPattern,
        parameter_len: usize,
    ) -> Result<(), CommandError> {
        let paragraph_token = self.state.symbol("par").map(Token::Cs);
        let mut delivered = None;
        for index in 0..pattern.leading_end(parameter_len) {
            let expected = self.macro_parameter_token(definition.clone(), index)?;
            if self.get_token_into(&mut delivered)? != crate::DeliveryStatus::Command {
                return Err(CommandError::MacroPrefixMismatch);
            }
            let actual = delivered
                .take()
                .expect("command status initializes destination");
            if actual.spelling().semantic_token() != expected {
                // TeX82 §391 tests every compulsory parameter-text token
                // after raw delivery has completed §336 recovery. An outer
                // control sequence therefore contributes the inserted
                // frozen `\par` to this same mismatch test; only §394's
                // argument collector turns that recovery into an aborted
                // argument scan.
                return Err(CommandError::MacroPrefixMismatch);
            }
        }
        // TeX82 §394 corrects the final matched left brace in a `#{`
        // parameter delimiter when the following pattern token is
        // `end_match`: definition scanning saves that same brace at the end
        // of the replacement, so only the replayed copy may contribute to
        // `align_state`. With no numbered parameters the brace lives in the
        // compulsory leading pattern rather than an argument delimiter.
        if pattern.parameter_count() == 0
            && pattern.leading_end(parameter_len) != 0
            && is_begin_group(self.macro_parameter_token(
                definition.clone(),
                pattern.leading_end(parameter_len) - 1,
            )?)
        {
            self.undo_delimiter_begin_group_delivery();
        }

        for parameter in 0..pattern.parameter_count() {
            let (start, end) = pattern.delimiter_bounds(parameter, parameter_len);
            let delimiter = MacroDelimiter {
                definition: definition.clone(),
                start,
                len: end - start,
            };
            let argument = if delimiter.len == 0 {
                self.scan_undelimited_argument(matching, flags, paragraph_token)?
            } else {
                self.scan_delimited_argument(matching, flags, &delimiter, paragraph_token)?
            };
            let marker = pattern.marker_index(parameter).map_or(Ok('#'), |index| {
                match self.macro_parameter_token(definition.clone(), index)? {
                    Token::Char { ch, .. } => Ok(ch),
                    _ => Err(CommandError::input_invariant()),
                }
            })?;
            self.trace_macro_argument(matching, marker, parameter + 1, &argument)?;
            let argument_token_count = self
                .command
                .scratch
                .match_words(&argument)
                .map_err(|_| CommandError::input_invariant())?
                .len();
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "macro_delimiter_match",
                    tokens: (0..delimiter.len)
                        .filter_map(|index| self.macro_delimiter_token(&delimiter, index).ok())
                        .map(|token| {
                            self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN))
                        })
                        .collect(),
                }),
            );
            observe!(
                self,
                CommandObservation::Macro(MacroRecord::Argument {
                    control_sequence: self.state.resolve(macro_name).to_owned(),
                    parameter: (parameter + 1) as u8,
                    token_count: argument_token_count as u64,
                    tokens: self
                        .command
                        .scratch
                        .match_words(&argument)
                        .expect("matched macro argument remains live until sealing")
                        .map(|token| self.observed_token(token))
                        .collect(),
                }),
            );
            self.command
                .scratch
                .finish_match_buffer(argument)
                .map_err(|_| CommandError::input_invariant())?;
        }
        Ok(())
    }

    fn macro_parameter_token(
        &self,
        definition: DefinitionId<G>,
        index: usize,
    ) -> Result<Token, CommandError> {
        self.state
            .definition(definition)
            .parameter_text()
            .get(index)
            .map(|word| word.semantic_token())
            .ok_or(CommandError::input_invariant())
    }

    fn macro_delimiter_token(
        &self,
        delimiter: &MacroDelimiter<G>,
        index: usize,
    ) -> Result<Token, CommandError> {
        if index >= delimiter.len {
            return Err(CommandError::input_invariant());
        }
        self.macro_parameter_token(delimiter.definition.clone(), delimiter.start + index)
    }

    /// TeX82 §389's invocation trace, including `print_ln` before the macro
    /// name and §262's control-word separator before `->`.
    fn trace_macro_invocation(
        &mut self,
        macro_name: tex_state::interner::Symbol,
        definition: DefinitionId<G>,
    ) {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 0 {
            return;
        }
        let mut text = String::new();
        crate::processor::expand::append_print_cs_text(self.state, macro_name, &mut text);
        let definition = self.state.definition(definition);
        // TeX82 §§389 uses `token_show` on the stored definition. A
        // non-`#` parameter marker is stored beside its compact out-parameter
        // slot and must render as one pair (`U3`), not as the literal marker
        // followed by the generic `#3` spelling.
        crate::processor::expand::append_meaning_token_words(
            self.state,
            definition.parameter_text(),
            false,
            &mut text,
        );
        text.push_str("->");
        crate::processor::expand::append_meaning_token_words(
            self.state,
            definition.replacement_text(),
            false,
            &mut text,
        );
        self.print_macro_trace(text, true);
    }

    /// TeX82 §400's `#n<-<argument>` trace in completed-argument order.
    fn trace_macro_argument(
        &mut self,
        matching: &MacroMatch<G>,
        marker: char,
        parameter: usize,
        argument: &MacroMatchBuffer<G>,
    ) -> Result<(), CommandError> {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 0 {
            return Ok(());
        }
        let mut text = format!("{marker}{parameter}<-");
        for word in self.argument_buffer(matching, argument)? {
            crate::processor::expand::append_token_list_token_text(
                self.state,
                word.semantic_token(),
                &mut text,
            );
        }
        self.print_macro_trace(text, false);
        Ok(())
    }

    /// Prints a TeX82 §389/§400 macro diagnostic at the point `macro_call`
    /// reaches it. Deferred-write expansion postpones printing until its
    /// owning executor episode can restore the live diagnostic selector, and
    /// a pending synchronous error keeps later traces in that same queue.
    fn print_macro_trace(&mut self, text: String, force_newline: bool) {
        // TeX82 §82 completes a recoverable error synchronously before
        // execution resumes far enough for §389 or §400 to print a later
        // macro trace. The command boundary queues those reports for its
        // executor owner, so a trace reached while one is pending must join
        // the same ordered queue instead of overtaking it through the live
        // selector.
        if self.command.expanding_deferred_write() || !self.command.semantic_diagnostics.is_empty()
        {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Trace {
                    text,
                    force_newline,
                });
            return;
        }
        let mut output = self.begin_diagnostic();
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
                crate::processor::expand::print_esc_text(self.state, &spelling)
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
        let context = self.command.output_open_context(self.state);
        let mut display = String::new();
        for token in partial {
            crate::processor::expand::append_token_list_token_text(
                self.state,
                token.semantic_token(),
                &mut display,
            );
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
                integer_error: None,
            });
    }

    /// TeX82 §395's `<Report an extra right brace and goto continue>`.
    fn report_extra_right_brace_argument(&mut self) {
        let name = self.matching_macro_name();
        let context = self.command.output_open_context(self.state);
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
                integer_error: None,
            });
    }

    /// TeX82 §395's complete extra-right-brace recovery, shared by the
    /// undelimited and delimited branches of §394's parameter matcher.
    fn recover_extra_right_brace_argument(
        &mut self,
        command: crate::CurrentCommand<G>,
    ) -> Result<MacroMatchBuffer<G>, CommandError> {
        self.back_input(command)?;
        self.insert_macro_argument_recovery_par()?;
        // §395 ends with `ins_error`, so §82 renders the context with
        // the inserted `\par` level already on the stack.
        self.report_extra_right_brace_argument();
        let mut delivered = None;
        if self.get_token_into(&mut delivered)? != crate::DeliveryStatus::Command {
            return Err(CommandError::ParagraphInMacroArgument);
        }
        let par = delivered
            .take()
            .expect("command status initializes destination");
        self.back_input(par)?;
        // §395's `goto continue` immediately reads the inserted `\par`;
        // `long_state := call` makes §396 abort even a `\long` macro.
        self.report_paragraph_ended_before_complete(&[]);
        Err(CommandError::ParagraphInMacroArgument)
    }

    fn scan_undelimited_argument(
        &mut self,
        matching: &MacroMatch<G>,
        flags: MeaningFlags,
        paragraph_token: Option<Token>,
    ) -> Result<MacroMatchBuffer<G>, CommandError> {
        let mut delivered = None;
        let first = loop {
            if self.get_token_into(&mut delivered)? != crate::DeliveryStatus::Command {
                return Err(CommandError::ParagraphInMacroArgument);
            }
            let command = delivered
                .take()
                .expect("command status initializes destination");
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
                return self.recover_extra_right_brace_argument(command);
            }
            break command;
        };
        let first_facts = argument_token_facts(&first, paragraph_token, true);
        self.check_argument_paragraph(&first, flags, first_facts, None)?;
        if !matches!(
            first.spelling().semantic_token(),
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            }
        ) {
            let mut tokens = self.allocate_argument_buffer(matching)?;
            self.push_argument_token(matching, &mut tokens, first.spelling(), first_facts)?;
            return Ok(tokens);
        }

        // TeX82 §394 links the opening left brace into the temporary
        // argument list and removes the matching outer pair only after the
        // argument completes.  Keep that ownership here too: §396's
        // runaway pseudoprint must still see an unmatched opening brace.
        let mut depth = 1_u32;
        let mut tokens = self.allocate_argument_buffer(matching)?;
        self.push_argument_token(matching, &mut tokens, first.spelling(), first_facts)?;
        loop {
            if self.get_token_into(&mut delivered)? != crate::DeliveryStatus::Command {
                return Err(CommandError::ParagraphInMacroArgument);
            }
            let command = delivered
                .as_ref()
                .expect("command status initializes destination");
            // TeX82 §23's recovered `cur_cmd := spacer` is the return
            // value of the interrupted raw delivery, not a token linked into
            // §394's temporary argument list. The inserted `\par`
            // aborts this match on the next demand; §306's already-owned
            // runaway pseudoprint must therefore end at the last real token.
            if command.is_outer_recovery_space() {
                delivered = None;
                continue;
            }
            if self.outer_recovered_while_matching && is_paragraph_command(command) {
                let partial = self.argument_buffer(matching, &tokens)?.collect::<Vec<_>>();
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
                return Err(CommandError::OuterInMacroArgument);
            }
            let facts = argument_token_facts(command, paragraph_token, true);
            self.check_argument_paragraph(command, flags, facts, Some((matching, &tokens)))?;
            match command.spelling().semantic_token() {
                Token::Char {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    depth += 1;
                    self.push_argument_token(matching, &mut tokens, command.spelling(), facts)?;
                }
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                } => {
                    depth -= 1;
                    if depth == 0 {
                        self.push_argument_token(matching, &mut tokens, command.spelling(), facts)?;
                        tokens = self.strip_argument_outer_group(matching, tokens)?;
                        return Ok(tokens);
                    }
                    self.push_argument_token(matching, &mut tokens, command.spelling(), facts)?;
                }
                _ => self.push_argument_token(matching, &mut tokens, command.spelling(), facts)?,
            }
            delivered = None;
        }
    }

    /// TeX.web §394's literal, scalar delimiter matcher. A failed delimiter
    /// prefix commits only the part that cannot be reused as an overlapping
    /// prefix, then retains the maximal suffix that still matches the start
    /// of the delimiter. This is intentionally not a compiled string matcher:
    /// token catcodes, brace depth, and the recovery splice are semantic here.
    fn scan_delimited_argument(
        &mut self,
        matching: &MacroMatch<G>,
        flags: MeaningFlags,
        delimiter: &MacroDelimiter<G>,
        paragraph_token: Option<Token>,
    ) -> Result<MacroMatchBuffer<G>, CommandError> {
        debug_assert_ne!(delimiter.len, 0);
        let mut tokens = self.allocate_argument_buffer(matching)?;
        self.command.scratch.clear_delimiter_prefix();
        let mut depth = 0_u32;
        let mut delivered = None;

        loop {
            if self.get_token_into(&mut delivered)? != crate::DeliveryStatus::Command {
                return Err(CommandError::ParagraphInMacroArgument);
            }
            let command = delivered
                .as_ref()
                .expect("command status initializes destination");
            if command.is_outer_recovery_space() {
                delivered = None;
                continue;
            }
            if self.outer_recovered_while_matching && is_paragraph_command(command) {
                let mut partial = self.argument_buffer(matching, &tokens)?.collect::<Vec<_>>();
                partial.extend(self.command.scratch.delimiter_prefix_words());
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
                return Err(CommandError::OuterInMacroArgument);
            }
            let token = command.spelling().semantic_token();

            if depth == 0
                && token
                    == self.macro_delimiter_token(
                        delimiter,
                        self.command.scratch.delimiter_prefix_len(),
                    )?
            {
                self.command
                    .scratch
                    .push_delimiter_prefix(command.spelling())
                    .map_err(|_| CommandError::input_invariant())?;
                if self.command.scratch.delimiter_prefix_len() == delimiter.len {
                    // `#{` consumes the opening brace as parameter text. Raw
                    // delivery has accounted for it, but no replacement-body
                    // replay exists yet to provide the balancing delivery.
                    if is_begin_group(token) {
                        self.undo_delimiter_begin_group_delivery();
                    }
                    self.command.scratch.clear_delimiter_prefix();
                    tokens = self.strip_argument_outer_group(matching, tokens)?;
                    return Ok(tokens);
                }
                delivered = None;
                continue;
            }

            if !self.command.scratch.delimiter_prefix_is_empty() {
                let retained = self.overlapping_delimiter_prefix(command.spelling(), delimiter)?;
                let committed = if retained == 0 {
                    self.command.scratch.delimiter_prefix_len()
                } else {
                    self.command.scratch.delimiter_prefix_len() + 1 - retained
                };
                for _ in 0..committed {
                    let prefix_token = self
                        .command
                        .scratch
                        .pop_delimiter_prefix_word()
                        .map_err(|_| CommandError::input_invariant())?;
                    observe!(
                        self,
                        CommandObservation::TokenList(TokenListRecord {
                            transition: "splice",
                            purpose: "macro_delimiter_recovery",
                            tokens: vec![self.observed_token(prefix_token)],
                        }),
                    );
                    let facts = argument_word_facts(prefix_token, paragraph_token, false);
                    self.push_delimited_argument_token(
                        matching,
                        &mut tokens,
                        &mut depth,
                        prefix_token,
                        facts,
                    )?;
                }
                if retained != 0 {
                    self.command
                        .scratch
                        .push_delimiter_prefix(command.spelling())
                        .map_err(|_| CommandError::input_invariant())?;
                    delivered = None;
                    continue;
                }

                // TeX82 §394 contributes a failed delimiter prefix first,
                // then applies §395 to the current token. A top-level `}`
                // therefore never becomes delimited argument material.
                if depth == 0 && is_end_group(token) {
                    let command = delivered
                        .take()
                        .expect("command destination remains initialized");
                    return self.recover_extra_right_brace_argument(command);
                }

                // The mismatching token cannot continue the delimiter, so it
                // becomes ordinary argument material after the committed
                // prefix. TeX.web §394 permits a recovered `\par` prefix;
                // only this newly ordinary token is subject to the non-long
                // paragraph check.
                let facts = argument_token_facts(command, paragraph_token, true);
                self.check_argument_paragraph(command, flags, facts, Some((matching, &tokens)))?;
                self.push_delimited_argument_token(
                    matching,
                    &mut tokens,
                    &mut depth,
                    command.spelling(),
                    facts,
                )?;
                delivered = None;
                continue;
            }

            if depth == 0 && is_end_group(token) {
                let command = delivered
                    .take()
                    .expect("command destination remains initialized");
                return self.recover_extra_right_brace_argument(command);
            }

            let facts = argument_token_facts(command, paragraph_token, true);
            self.check_argument_paragraph(command, flags, facts, Some((matching, &tokens)))?;
            self.push_delimited_argument_token(
                matching,
                &mut tokens,
                &mut depth,
                command.spelling(),
                facts,
            )?;
            delivered = None;
        }
    }

    fn overlapping_delimiter_prefix(
        &self,
        current: TracedTokenWord,
        delimiter: &MacroDelimiter<G>,
    ) -> Result<usize, CommandError> {
        let pending_len = self.command.scratch.delimiter_prefix_len() + 1;
        for candidate_len in (1..pending_len.min(delimiter.len)).rev() {
            let mut matches = true;
            for index in 0..candidate_len {
                let pending = pending_len - candidate_len + index;
                let token = if pending == self.command.scratch.delimiter_prefix_len() {
                    current
                } else {
                    self.command
                        .scratch
                        .delimiter_prefix_word(pending)
                        .map_err(|_| CommandError::input_invariant())?
                }
                .semantic_token();
                if token != self.macro_delimiter_token(delimiter, index)? {
                    matches = false;
                    break;
                }
            }
            if matches {
                return Ok(candidate_len);
            }
        }
        Ok(0)
    }

    fn check_argument_paragraph(
        &mut self,
        command: &crate::CurrentCommand<G>,
        flags: MeaningFlags,
        facts: MacroArgumentTokenFacts,
        partial: Option<(&MacroMatch<G>, &MacroMatchBuffer<G>)>,
    ) -> Result<(), CommandError> {
        if self.eof_recovered_while_matching && is_paragraph_command(command) {
            // TeX82 §23 calls `check_outer_validity` after source EOF and
            // changes `long_state` to `outer_call`, even for a `\long` macro.
            // Its inserted frozen `\par` terminates the match but is consumed
            // by the failed expansion instead of being replayed by §396.
            let partial = partial
                .map(|(matching, buffer)| {
                    self.argument_buffer(matching, buffer)
                        .map(|words| words.collect::<Vec<_>>())
                })
                .transpose()?
                .unwrap_or_default();
            self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        if facts.rejects_non_long_paragraph && !flags.contains(MeaningFlags::LONG) {
            // TeX82 §394 reports this through `back_error` while the macro
            // matcher is still live.  The caller will then restore its
            // enclosing scanner status, so retain the exact `\par` input
            // ahead of that restoration rather than merely returning an
            // error from the scalar matcher.
            self.back_input(command.copy_for_backup())?;
            // §396 ends with `back_error`, so §82 renders the context with the
            // replayed `\par` already on the stack.
            let partial = partial
                .map(|(matching, buffer)| {
                    self.argument_buffer(matching, buffer)
                        .map(|words| words.collect::<Vec<_>>())
                })
                .transpose()?
                .unwrap_or_default();
            self.report_paragraph_ended_before_complete(&partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        Ok(())
    }

    fn allocate_argument_buffer(
        &mut self,
        matching: &MacroMatch<G>,
    ) -> Result<MacroMatchBuffer<G>, CommandError> {
        self.command
            .scratch
            .begin_match_buffer(matching)
            .map_err(|_| CommandError::input_invariant())
    }

    fn argument_buffer(
        &self,
        _matching: &MacroMatch<G>,
        buffer: &MacroMatchBuffer<G>,
    ) -> Result<MacroWords<'_, G>, CommandError> {
        self.command
            .scratch
            .match_words(buffer)
            .map_err(|_| CommandError::input_invariant())
    }

    fn push_argument_token(
        &mut self,
        _matching: &MacroMatch<G>,
        buffer: &mut MacroMatchBuffer<G>,
        token: TracedTokenWord,
        facts: MacroArgumentTokenFacts,
    ) -> Result<(), CommandError> {
        self.command
            .scratch
            .push_match_word(buffer, token, facts)
            .map_err(|_| CommandError::input_invariant())
    }

    fn strip_argument_outer_group(
        &mut self,
        _matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<MacroMatchBuffer<G>, CommandError> {
        if self
            .command
            .scratch
            .match_argument_facts(&buffer)
            .map_err(|_| CommandError::input_invariant())?
            .removable_outer_group()
        {
            self.command
                .scratch
                .strip_match_outer_group(&buffer)
                .map_err(|_| CommandError::input_invariant())?;
        }
        Ok(buffer)
    }

    fn push_delimited_argument_token(
        &mut self,
        matching: &MacroMatch<G>,
        buffer: &mut MacroMatchBuffer<G>,
        depth: &mut u32,
        token: TracedTokenWord,
        facts: MacroArgumentTokenFacts,
    ) -> Result<(), CommandError> {
        if facts.begin_group {
            *depth = depth.saturating_add(1);
        } else if facts.end_group && *depth > 0 {
            *depth -= 1;
        }
        self.push_argument_token(matching, buffer, token, facts)
    }

    fn argument_token_count(&self, arguments: MacroArguments<G>) -> usize {
        (1..=9)
            .filter_map(|slot| {
                self.command
                    .scratch
                    .argument_range(arguments.frame(), slot)
                    .ok()
                    .flatten()
            })
            .map(|range| range.len() as usize)
            .sum()
    }
}

/// TeX82 §394 aborts a match on the recovery paragraph that follows its
/// synthetic outer-validity space, not on that space itself.
fn is_paragraph_command<G>(command: &crate::CurrentCommand<G>) -> bool {
    matches!(
        command.meaning_ref(),
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par))
    )
}

/// Classifies the only token facts §394 needs while the raw delivery is
/// already in hand. The `paragraph_checked` bit is semantic: a delimiter
/// prefix committed after a mismatch becomes argument material without
/// passing through the ordinary `cur_tok=par_token` branch.
fn argument_token_facts<G>(
    command: &crate::CurrentCommand<G>,
    paragraph_token: Option<Token>,
    paragraph_checked: bool,
) -> MacroArgumentTokenFacts {
    argument_word_facts(command.spelling(), paragraph_token, paragraph_checked)
}

fn argument_word_facts(
    word: TracedTokenWord,
    paragraph_token: Option<Token>,
    paragraph_checked: bool,
) -> MacroArgumentTokenFacts {
    let token = word.semantic_token();
    MacroArgumentTokenFacts {
        // TeX82 §394 tests `cur_tok=par_token`, not `cur_cmd=par_end`.
        // Aliases of `\par` therefore remain ordinary, while the original
        // token stays forbidden after its mutable meaning is reassigned.
        rejects_non_long_paragraph: paragraph_checked && Some(token) == paragraph_token,
        begin_group: is_begin_group(token),
        end_group: is_end_group(token),
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
