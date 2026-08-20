//! Private canonical scalar macro-call state machine.
#![allow(dead_code)] // expansion dispatch is the next ordered integration slice
use std::hash::{Hash, Hasher};

use tex_state::DefinitionId;
use tex_state::env::banks::IntParam;
use tex_state::interner::Symbol;
use tex_state::macro_definition::MacroParameterPattern;
use tex_state::meaning::{Meaning, MeaningFlags, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::attempt::{AttemptArgumentRecordId, AttemptTokenBufferId, AttemptTokenListId};
use crate::processor::status::{
    ArgumentBuilderId, MatchingContext, ScannerStatus, ScannerStatusVisibility, ScannerWarning,
};
use crate::{CommandError, CommandProcessor};

use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, MacroRecord, TokenListRecord,
};

const EXTRA_RIGHT_BRACE_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0395;
pub(crate) const RUNAWAY_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0396;

/// Persistent ownership of live macro-argument activations.
///
/// This is the sole owner of the activation chain. Macro-body input behavior
/// carries a typed activation identity, while parameter payloads retain shared
/// ownership of the one contiguous argument allocation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ParameterState<G> {
    pub(crate) activations: Vec<MacroActivation<G>>,
    pub(crate) next_activation_identity: u64,
}

impl<G> Default for ParameterState<G> {
    fn default() -> Self {
        Self {
            activations: Vec::new(),
            next_activation_identity: 0,
        }
    }
}

/// Typed identity of one live macro activation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct MacroActivationId(pub(crate) u64);

/// One live macro call and the materialized arguments it owns.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroActivation<G> {
    pub(crate) identity: MacroActivationId,
    /// TeX82 §389's `warning_index`: the control sequence being expanded.
    /// §314 prints it as this level's context descriptor.
    pub(crate) name: Symbol,
    pub(crate) definition: DefinitionId<G>,
    pub(crate) arguments: MacroArguments,
    pub(crate) invocation: OriginId,
}

/// One contiguous macro-argument allocation and its at-most-nine ranges.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct MacroArguments(Option<AttemptArgumentRecordId>);

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
    arguments: Vec<AttemptTokenListId>,
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

#[derive(Debug, Eq, Hash, PartialEq)]
struct MacroDelimiter<G> {
    definition: DefinitionId<G>,
    start: usize,
    len: usize,
}

impl<G> Clone for MacroDelimiter<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for MacroDelimiter<G> {}

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

impl MacroArgumentBuilder {
    /// Completes the next argument in canonical definition order.
    pub(crate) fn complete(
        &mut self,
        slot: u8,
        argument: AttemptTokenListId,
    ) -> Result<(), MacroArgumentBuildError> {
        self.validate_slot(slot)?;
        self.arguments.push(argument);
        self.next_slot = slot;
        Ok(())
    }

    /// Completes an argument by transferring the matcher's canonical packed
    /// buffer. This is the production macro-call seam; it avoids rebuilding
    /// per-token owner pairs between matching and activation replay.
    fn complete_buffer(
        &mut self,
        slot: u8,
        argument: AttemptTokenListId,
    ) -> Result<(), MacroArgumentBuildError> {
        self.complete(slot, argument)
    }

    fn validate_slot(&self, slot: u8) -> Result<(), MacroArgumentBuildError> {
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
        Ok(())
    }

    /// Freezes the single shared argument allocation for one activation.
    #[must_use]
    pub(crate) fn finish<G>(
        self,
        attempt: &mut crate::attempt::AttemptArena<G>,
    ) -> Result<MacroArguments, crate::attempt::AttemptError> {
        attempt
            .allocate_arguments(&self.arguments)
            .map(|record| MacroArguments(Some(record)))
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
        arguments: MacroArguments,
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
        arguments: MacroArguments,
        invocation: OriginId,
    ) {
        self.install_activation(identity, name, definition, arguments, invocation);
    }

    fn install_activation(
        &mut self,
        identity: MacroActivationId,
        name: Symbol,
        definition: DefinitionId<G>,
        arguments: MacroArguments,
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

    pub(crate) fn retire_last_activation(&mut self) {
        self.activations.pop();
    }

    pub(crate) fn prepare_argument_build(&mut self) {
        // The operation arena owns all argument allocations. Completed live
        // activations retain their typed records until their body retires.
    }
}

impl MacroArguments {
    pub(crate) const fn record(self) -> Option<AttemptArgumentRecordId> {
        self.0
    }
}

impl<G> CommandProcessor<'_, G> {
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
        crate::processor::expand::append_print_esc_text(&self.state, name, &mut text);
        text.push_str("->");
        let token_count = self.state.token_list(tokens).len();
        for index in 0..token_count {
            let token = self.state.token_list(tokens)[index].semantic_token();
            crate::processor::expand::append_token_list_token_text(&self.state, token, &mut text);
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
    /// Executes a live macro command without cloning its structural origin.
    /// The ordinary expansion loop owns the command for the complete call and
    /// moves it into retry state only when a typed resource barrier is hit.
    pub(crate) fn macro_call(
        &mut self,
        call: &crate::CurrentCommand<G>,
    ) -> Result<MacroCallOutcome, CommandError> {
        let ResolvedMeaning::Macro { flags, definition } = call.meaning() else {
            return Err(CommandError::input_invariant());
        };
        let macro_name = call
            .control_sequence()
            .ok_or(CommandError::input_invariant())?;
        self.command.parameters.prepare_argument_build();
        let definition_view = self.state.definition(definition);
        let pattern = definition_view.parameter_pattern();
        let parameter_len = definition_view.parameter_text().len();
        self.trace_macro_invocation(macro_name, definition);
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
        let arguments = match self.macro_call_scalar(definition, flags, pattern, parameter_len) {
            Ok(arguments) => arguments,
            Err(CommandError::MacroPrefixMismatch) => {
                // TeX82 §391 reports the mismatch through `error` and returns
                // from `macro_call`; the mismatching token stays consumed and
                // no replacement text is installed.
                // TeX82 §391 calls `error` before returning from `macro_call`.
                // Capture §82's context while the mismatching input level is
                // still live; in particular, §336's frozen `\par` retains its
                // `<inserted text>` ownership until this report is complete.
                let context = self.command.output_open_context(&self.state);
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
                return Ok(MacroCallOutcome::PrefixMismatchRecovered);
            }
            Err(error) => {
                if let Some(episode) = episode {
                    self.finish_scanner_episode(episode);
                }
                return Err(error);
            }
        };

        // TeX.web §§391--400 freezes the completed ranges before replacing
        // the input. The activation names one command-arena argument span;
        // its body replays the admitted immutable replacement span and
        // resolves compact `OutParameter` tokens through that coordinate.
        // TeX82 §390's replacement hand-off first drains every depleted token
        // list -- the exhausted macro body or replayed parameter the call
        // token itself came from, any backup or recovery insertion, and any
        // finished stored replay -- before `begin_token_list(..., macro)`.
        // Those retirements must precede this body's input push.
        self.conserve_input_stack()?;
        let _level = self.push_macro_activation(macro_name, definition, call.origin(), arguments);
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
            CommandObservation::Macro(MacroRecord {
                activation: true,
                definition: self.definition_observation_operand(definition),
                control_sequence: Some(self.state.resolve(macro_name).to_owned()),
                argument: Some(pattern.parameter_count() as u8),
                token_count: self.argument_token_count(arguments) as u64,
                tokens: Vec::new(),
            }),
        );
        if let Some(episode) = episode {
            self.finish_scanner_episode(episode);
        }
        Ok(MacroCallOutcome::Activated)
    }

    fn macro_call_scalar(
        &mut self,
        definition: DefinitionId<G>,
        flags: MeaningFlags,
        pattern: MacroParameterPattern,
        parameter_len: usize,
    ) -> Result<MacroArguments, CommandError> {
        for index in 0..pattern.leading_end(parameter_len) {
            let expected = self.macro_parameter_token(definition, index)?;
            let actual = self.get_token()?.ok_or(CommandError::MacroPrefixMismatch)?;
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
            && is_begin_group(
                self.macro_parameter_token(definition, pattern.leading_end(parameter_len) - 1)?,
            )
        {
            self.undo_delimiter_begin_group_delivery();
        }

        let mut arguments = MacroArgumentBuilder::default();
        for parameter in 0..pattern.parameter_count() {
            let (start, end) = pattern.delimiter_bounds(parameter, parameter_len);
            let delimiter = MacroDelimiter {
                definition,
                start,
                len: end - start,
            };
            let argument = if delimiter.len == 0 {
                self.scan_undelimited_argument(flags)?
            } else {
                self.scan_delimited_argument(flags, delimiter)?
            };
            let marker = pattern.marker_index(parameter).map_or(Ok('#'), |index| {
                match self.macro_parameter_token(definition, index)? {
                    Token::Char { ch, .. } => Ok(ch),
                    _ => Err(CommandError::input_invariant()),
                }
            })?;
            self.trace_macro_argument(marker, parameter + 1, argument)?;
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "macro_delimiter_match",
                    tokens: (0..delimiter.len)
                        .filter_map(|index| self.macro_delimiter_token(delimiter, index).ok())
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
                    definition: self.definition_observation_operand(definition),
                    control_sequence: None,
                    argument: Some((parameter + 1) as u8),
                    token_count: self
                        .command
                        .attempt
                        .arena()
                        .token_buffer(argument)
                        .map_err(|_| CommandError::input_invariant())?
                        .len() as u64,
                    tokens: self
                        .command
                        .attempt
                        .arena()
                        .token_buffer(argument)
                        .map_err(|_| CommandError::input_invariant())?
                        .iter()
                        .map(|token| self.observed_token(*token))
                        .collect(),
                }),
            );
            let argument = self
                .command
                .attempt
                .arena_mut()
                .finish_token_buffer(argument)
                .map_err(|_| CommandError::input_invariant())?;
            arguments
                .complete_buffer((parameter + 1) as u8, argument)
                .map_err(|_| CommandError::input_invariant())?;
        }
        arguments
            .finish(self.command.attempt.arena_mut())
            .map_err(|_| CommandError::input_invariant())
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
        delimiter: MacroDelimiter<G>,
        index: usize,
    ) -> Result<Token, CommandError> {
        if index >= delimiter.len {
            return Err(CommandError::input_invariant());
        }
        self.macro_parameter_token(delimiter.definition, delimiter.start + index)
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
        crate::processor::expand::append_print_cs_text(&mut self.state, macro_name, &mut text);
        let definition = self.state.definition(definition);
        for token in definition.parameter_text() {
            crate::processor::expand::append_token_list_token_text(
                &self.state,
                token.semantic_token(),
                &mut text,
            );
        }
        text.push_str("->");
        for word in definition.replacement_text() {
            let token = word.semantic_token();
            crate::processor::expand::append_token_list_token_text(&self.state, token, &mut text);
        }
        self.print_macro_trace(text, true);
    }

    /// TeX82 §400's `#n<-<argument>` trace in completed-argument order.
    fn trace_macro_argument(
        &mut self,
        marker: char,
        parameter: usize,
        argument: AttemptTokenBufferId,
    ) -> Result<(), CommandError> {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 0 {
            return Ok(());
        }
        let mut text = format!("{marker}{parameter}<-");
        for word in self.argument_buffer(argument)? {
            crate::processor::expand::append_token_list_token_text(
                &self.state,
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
        // TeX82 §537 prints an input file's opening before it reads the
        // first line. Expansion can open that file below a token already held
        // by §368's `\expandafter`, so §389's next macro trace can be the
        // first print reached while the new source level is live. Publish the
        // already-committed opening before that trace, just as the ordinary
        // command-trace boundary does. A pending diagnostic took the queued
        // branch above so its earlier report still cannot be overtaken.
        self.command.render_file_framing_events(&mut self.state);
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
            crate::processor::expand::append_token_list_token_text(
                &self.state,
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

    /// TeX82 §395's complete extra-right-brace recovery, shared by the
    /// undelimited and delimited branches of §394's parameter matcher.
    fn recover_extra_right_brace_argument(
        &mut self,
        command: crate::CurrentCommand<G>,
    ) -> Result<AttemptTokenBufferId, CommandError> {
        self.back_input(command)?;
        self.insert_macro_argument_recovery_par()?;
        // §395 ends with `ins_error`, so §82 renders the context with
        // the inserted `\par` level already on the stack.
        self.report_extra_right_brace_argument();
        let par = self
            .get_token()?
            .ok_or(CommandError::ParagraphInMacroArgument)?;
        self.back_input(par)?;
        // §395's `goto continue` immediately reads the inserted `\par`;
        // `long_state := call` makes §396 abort even a `\long` macro.
        self.report_paragraph_ended_before_complete(&[]);
        Err(CommandError::ParagraphInMacroArgument)
    }

    fn scan_undelimited_argument(
        &mut self,
        flags: MeaningFlags,
    ) -> Result<AttemptTokenBufferId, CommandError> {
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
                return self.recover_extra_right_brace_argument(command);
            }
            break command;
        };
        self.check_argument_paragraph(&first, flags, None)?;
        if !matches!(
            first.spelling().semantic_token(),
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            }
        ) {
            let tokens = self.allocate_argument_buffer()?;
            self.push_argument_token(tokens, first.spelling())?;
            return Ok(tokens);
        }

        // TeX82 §394 links the opening left brace into the temporary
        // argument list and removes the matching outer pair only after the
        // argument completes.  Keep that ownership here too: §396's
        // runaway pseudoprint must still see an unmatched opening brace.
        let mut depth = 1_u32;
        let tokens = self.allocate_argument_buffer()?;
        self.push_argument_token(tokens, first.spelling())?;
        loop {
            let command = self
                .get_token()?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            // TeX82 §23's recovered `cur_cmd := spacer` is the return
            // value of the interrupted raw delivery, not a token linked into
            // §394's temporary argument list. The inserted `\par`
            // aborts this match on the next demand; §306's already-owned
            // runaway pseudoprint must therefore end at the last real token.
            if command.is_outer_recovery_space() {
                continue;
            }
            if self.outer_recovered_while_matching && is_paragraph_command(&command) {
                let partial = self.argument_buffer(tokens)?.to_vec();
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
                return Err(CommandError::OuterInMacroArgument);
            }
            self.check_argument_paragraph(&command, flags, Some(tokens))?;
            match command.spelling().semantic_token() {
                Token::Char {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    depth += 1;
                    self.push_argument_token(tokens, command.spelling())?;
                }
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                } => {
                    depth -= 1;
                    if depth == 0 {
                        self.push_argument_token(tokens, command.spelling())?;
                        self.strip_argument_outer_group(tokens)?;
                        return Ok(tokens);
                    }
                    self.push_argument_token(tokens, command.spelling())?;
                }
                _ => self.push_argument_token(tokens, command.spelling())?,
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
        delimiter: MacroDelimiter<G>,
    ) -> Result<AttemptTokenBufferId, CommandError> {
        debug_assert_ne!(delimiter.len, 0);
        let tokens = self.allocate_argument_buffer()?;
        let prefix = self.allocate_argument_buffer()?;
        let mut depth = 0_u32;
        let mut current = None;

        loop {
            let command = match current.take() {
                Some(command) => command,
                None => self
                    .get_token()?
                    .ok_or(CommandError::ParagraphInMacroArgument)?,
            };
            if command.is_outer_recovery_space() {
                continue;
            }
            if self.outer_recovered_while_matching && is_paragraph_command(&command) {
                let mut partial = self.argument_buffer(tokens)?.to_vec();
                partial.extend_from_slice(self.argument_buffer(prefix)?);
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
                return Err(CommandError::OuterInMacroArgument);
            }
            let token = command.spelling().semantic_token();

            if depth == 0
                && token
                    == self.macro_delimiter_token(delimiter, self.argument_buffer(prefix)?.len())?
            {
                self.push_argument_token(prefix, command.spelling())?;
                if self.argument_buffer(prefix)?.len() == delimiter.len {
                    // `#{` consumes the opening brace as parameter text. Raw
                    // delivery has accounted for it, but no replacement-body
                    // replay exists yet to provide the balancing delivery.
                    if is_begin_group(token) {
                        self.undo_delimiter_begin_group_delivery();
                    }
                    self.strip_argument_outer_group(tokens)?;
                    return Ok(tokens);
                }
                continue;
            }

            if !self.argument_buffer(prefix)?.is_empty() {
                let retained =
                    self.overlapping_delimiter_prefix(prefix, command.spelling(), delimiter)?;
                let committed = if retained == 0 {
                    self.argument_buffer(prefix)?.len()
                } else {
                    self.argument_buffer(prefix)?.len() + 1 - retained
                };
                let committed = self
                    .command
                    .attempt
                    .arena_mut()
                    .drain_buffer_prefix(prefix, committed)
                    .map_err(|_| CommandError::input_invariant())?;
                for prefix_token in committed {
                    observe!(
                        self,
                        CommandObservation::TokenList(TokenListRecord {
                            transition: "splice",
                            purpose: "macro_delimiter_recovery",
                            tokens: vec![self.observed_token(prefix_token)],
                        }),
                    );
                    self.push_delimited_argument_token(tokens, &mut depth, prefix_token)?;
                }
                if retained != 0 {
                    self.push_argument_token(prefix, command.spelling())?;
                    continue;
                }

                // TeX82 §394 contributes a failed delimiter prefix first,
                // then applies §395 to the current token. A top-level `}`
                // therefore never becomes delimited argument material.
                if depth == 0 && is_end_group(token) {
                    return self.recover_extra_right_brace_argument(command);
                }

                // The mismatching token cannot continue the delimiter, so it
                // becomes ordinary argument material after the committed
                // prefix. TeX.web §394 permits a recovered `\par` prefix;
                // only this newly ordinary token is subject to the non-long
                // paragraph check.
                self.check_argument_paragraph(&command, flags, Some(tokens))?;
                self.push_delimited_argument_token(tokens, &mut depth, command.spelling())?;
                continue;
            }

            if depth == 0 && is_end_group(token) {
                return self.recover_extra_right_brace_argument(command);
            }

            self.check_argument_paragraph(&command, flags, Some(tokens))?;
            self.push_delimited_argument_token(tokens, &mut depth, command.spelling())?;
        }
    }

    fn overlapping_delimiter_prefix(
        &self,
        prefix: AttemptTokenBufferId,
        current: TracedTokenWord,
        delimiter: MacroDelimiter<G>,
    ) -> Result<usize, CommandError> {
        let prefix = self.argument_buffer(prefix)?;
        let pending_len = prefix.len() + 1;
        for candidate_len in (1..pending_len.min(delimiter.len)).rev() {
            let mut matches = true;
            for index in 0..candidate_len {
                let pending = pending_len - candidate_len + index;
                let token = prefix
                    .get(pending)
                    .copied()
                    .unwrap_or(current)
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
        partial: Option<AttemptTokenBufferId>,
    ) -> Result<(), CommandError> {
        if self.eof_recovered_while_matching && is_paragraph_command(command) {
            // TeX82 §23 calls `check_outer_validity` after source EOF and
            // changes `long_state` to `outer_call`, even for a `\long` macro.
            // Its inserted frozen `\par` terminates the match but is consumed
            // by the failed expansion instead of being replayed by §396.
            let partial = partial
                .map(|buffer| self.argument_buffer(buffer).map(|words| words.to_vec()))
                .transpose()?
                .unwrap_or_default();
            self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        if self.is_par_token(command) && !flags.contains(MeaningFlags::LONG) {
            // TeX82 §394 reports this through `back_error` while the macro
            // matcher is still live.  The caller will then restore its
            // enclosing scanner status, so retain the exact `\par` input
            // ahead of that restoration rather than merely returning an
            // error from the scalar matcher.
            self.back_input(command.copy_for_backup())?;
            // §396 ends with `back_error`, so §82 renders the context with the
            // replayed `\par` already on the stack.
            let partial = partial
                .map(|buffer| self.argument_buffer(buffer).map(|words| words.to_vec()))
                .transpose()?
                .unwrap_or_default();
            self.report_paragraph_ended_before_complete(&partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        Ok(())
    }

    /// TeX82 §394 tests `cur_tok=par_token`, not `cur_cmd=par_end`.
    /// A control sequence aliased to `\par` therefore remains ordinary
    /// argument material, while the `\par` token remains forbidden even if
    /// its mutable meaning cell has subsequently been reassigned.
    fn is_par_token(&self, command: &crate::CurrentCommand<G>) -> bool {
        let Some(par) = self.state.symbol("par") else {
            return false;
        };
        command.spelling().semantic_token() == Token::Cs(par)
    }

    fn allocate_argument_buffer(&mut self) -> Result<AttemptTokenBufferId, CommandError> {
        self.command
            .attempt
            .arena_mut()
            .allocate_token_buffer()
            .map_err(|_| CommandError::input_invariant())
    }

    fn argument_buffer(
        &self,
        buffer: AttemptTokenBufferId,
    ) -> Result<&[TracedTokenWord], CommandError> {
        self.command
            .attempt
            .arena()
            .token_buffer(buffer)
            .map_err(|_| CommandError::input_invariant())
    }

    fn push_argument_token(
        &mut self,
        buffer: AttemptTokenBufferId,
        token: TracedTokenWord,
    ) -> Result<(), CommandError> {
        self.command
            .attempt
            .arena_mut()
            .push_buffer_token(buffer, token)
            .map_err(|_| CommandError::input_invariant())
    }

    fn strip_argument_outer_group(
        &mut self,
        buffer: AttemptTokenBufferId,
    ) -> Result<(), CommandError> {
        if has_one_outer_group(self.argument_buffer(buffer)?) {
            self.command
                .attempt
                .arena_mut()
                .strip_buffer_outer_group(buffer)
                .map_err(|_| CommandError::input_invariant())?;
        }
        Ok(())
    }

    fn push_delimited_argument_token(
        &mut self,
        buffer: AttemptTokenBufferId,
        depth: &mut u32,
        token: TracedTokenWord,
    ) -> Result<(), CommandError> {
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
        self.push_argument_token(buffer, token)
    }

    fn argument_token_count(&self, arguments: MacroArguments) -> usize {
        arguments.record().map_or(0, |record| {
            self.command
                .attempt
                .arena()
                .arguments(record)
                .expect("live macro argument record")
                .iter()
                .map(|argument| {
                    self.command
                        .attempt
                        .arena()
                        .token_words(*argument)
                        .expect("live macro argument list")
                        .len()
                })
                .sum()
        })
    }

    fn definition_observation_operand(&self, definition: DefinitionId<G>) -> u64 {
        let definition = self.state.definition(definition);
        let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
        definition.parameter_text().hash(&mut fingerprint);
        definition.replacement_text().hash(&mut fingerprint);
        fingerprint.finish()
    }
}

/// TeX82 §394 aborts a match on the recovery paragraph that follows its
/// synthetic outer-validity space, not on that space itself.
fn is_paragraph_command<G>(command: &crate::CurrentCommand<G>) -> bool {
    matches!(
        command.meaning(),
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par))
    )
}

/// Returns the number of tokens from `prefix` plus `current` that form the
/// longest proper delimiter prefix after a mismatch. TeX.web §394 compares
/// these scalar token sequences directly while moving the unmatched leading
/// tokens into the completed argument.
fn has_one_outer_group(tokens: &[TracedTokenWord]) -> bool {
    if tokens.len() < 2
        || !is_begin_group(tokens[0].semantic_token())
        || !is_end_group(tokens[tokens.len() - 1].semantic_token())
    {
        return false;
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
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
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
