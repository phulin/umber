//! Private canonical scalar macro-call state machine.
use tex_state::env::banks::IntParam;
use tex_state::macro_definition::MacroDelimiter;
use tex_state::meaning::MeaningFlags;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};
use tex_state::{DefinitionRef, ResidentMacroBody};

use crate::command::CurrentCommand;
use crate::execution_scratch::ArgumentRunAdmission;
use crate::input::{ArgumentCursor, MatchedArguments};
use crate::processor::status::{
    MatchingContext, ScannerStatus, ScannerStatusVisibility, ScannerWarning,
};
use crate::{CommandError, CommandProcessor};

use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, MacroRecord, TokenListRecord,
};

const EXTRA_RIGHT_BRACE_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0395;
pub(crate) const RUNAWAY_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0396;

/// Activation shape of one already-admitted immutable macro definition.
///
/// The exceptional variant is selected by live observation, tracing, scanner,
/// recovery, alignment, or replay-completion state. It deliberately retains
/// the canonical scalar boundary instead of teaching the hot path those
/// semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MacroActivationClass {
    Simple,
    Matching,
    Exceptional,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MacroActivationCounters {
    simple: u64,
    matching: u64,
    exceptional: u64,
    empty_rows_elided: u64,
}

#[cfg(test)]
thread_local! {
    static MACRO_ACTIVATION_COUNTERS: core::cell::Cell<MacroActivationCounters> =
        const { core::cell::Cell::new(MacroActivationCounters {
            simple: 0,
            matching: 0,
            exceptional: 0,
            empty_rows_elided: 0,
        }) };
}

#[cfg(test)]
fn macro_activation_counters() -> MacroActivationCounters {
    MACRO_ACTIVATION_COUNTERS.with(core::cell::Cell::get)
}

fn record_macro_activation_class(activation: MacroActivationClass) {
    #[cfg(not(test))]
    let _ = activation;
    #[cfg(test)]
    MACRO_ACTIVATION_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        match activation {
            MacroActivationClass::Simple => {
                counters.simple = counters.simple.saturating_add(1);
            }
            MacroActivationClass::Matching => {
                counters.matching = counters.matching.saturating_add(1);
            }
            MacroActivationClass::Exceptional => {
                counters.exceptional = counters.exceptional.saturating_add(1);
            }
        }
        slot.set(counters);
    });
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
        crate::processor::expand_render::append_print_esc_text(self.state, name, &mut text);
        text.push_str("->");
        for word in self.state.token_list(tokens) {
            let token = word.semantic_token();
            crate::processor::expand_render::append_token_list_token_text(
                self.state, token, &mut text,
            );
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
    /// Explicit command callers retain their opener across synchronous matching.
    /// Ordinary expansion enters with invocation facts; resource failure unwinds
    /// either caller before host-owned checkpoint replay.
    #[cfg(test)] // Direct scalar-entry tests; production calls macro_call_hot.
    pub(crate) fn macro_call(
        &mut self,
        call: &mut crate::CurrentCommand<G>,
    ) -> Result<bool, CommandError> {
        let mut hot = *call;
        self.macro_call_hot(&mut hot)
    }

    pub(crate) fn macro_call_hot(
        &mut self,
        call: &mut crate::command::CurrentCommand<G>,
    ) -> Result<bool, CommandError> {
        let Some((flags, definition)) = call.macro_parts() else {
            return Err(CommandError::input_invariant());
        };
        let macro_name = call
            .control_sequence()
            .ok_or_else(|| CommandError::input_invariant())?;
        let call_site = call.origin();
        self.macro_call_parts(flags, definition, macro_name, call_site, |processor| {
            processor.report_macro_prefix_mismatch(call);
        })
    }

    pub(crate) fn macro_call_parts(
        &mut self,
        flags: MeaningFlags,
        definition: tex_state::DefinitionRef<G>,
        macro_name: tex_state::interner::Symbol,
        call_site: OriginId,
        report_prefix_mismatch: impl FnOnce(&mut Self),
    ) -> Result<bool, CommandError> {
        let admitted = self
            .state
            .admit_macro_definition(definition)
            .ok_or_else(CommandError::input_invariant)?;
        let (activation, body) = match admitted {
            tex_state::AdmittedMacroDefinition::SimpleMacro { body } => {
                let activation = self.classify_macro_activation(true, body.is_none());
                if activation == MacroActivationClass::Simple {
                    record_macro_activation_class(activation);
                    return self.activate_simple_macro(macro_name, call_site, body);
                }
                let body = match body {
                    Some(body) => body,
                    None => self
                        .state
                        .admit_macro_body(definition)
                        .ok_or_else(CommandError::input_invariant)?,
                };
                (activation, body)
            }
            tex_state::AdmittedMacroDefinition::MatchingMacro { body } => {
                (self.classify_macro_activation(false, body.is_empty()), body)
            }
        };
        record_macro_activation_class(activation);
        self.trace_macro_invocation(macro_name, &definition);
        // TeX82 §389 calls the §391 parameter matcher only when the macro's
        // parameter text does not begin with `end_match`. A parameterless
        // macro therefore feeds its replacement directly, without a transient
        // `matching` scanner episode. Literal leading tokens still need the
        // matcher even when there are no numbered parameters.
        let parameter_count = body.parameter_pattern().parameter_count();
        let leading_end = body.parameter_pattern().leading_end(body.parameter_len());
        let needs_matching = leading_end != 0 || parameter_count != 0;
        // Only numbered parameters need the reusable argument lane. A macro
        // whose parameter text is solely a compulsory literal prefix matches
        // directly from its immutable definition metadata and creates no
        // MacroMatch, ArgumentSet, writer, or captured-word block.
        let mut matching = (parameter_count != 0)
            .then(|| self.command.input.levels.arguments.begin_match())
            .transpose()
            .map_err(|_| CommandError::input_invariant())?;
        let episode = if needs_matching {
            let status = ScannerStatus::Matching(MatchingContext {
                macro_name,
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
        let result = (|| {
            let scanned_arguments = if needs_matching {
                self.macro_call_scalar(matching.as_mut(), &body, flags, macro_name)
            } else {
                Ok(())
            };
            match scanned_arguments {
                Ok(()) => {}
                Err(CommandError::MacroPrefixMismatch) => {
                    // §391 reports the mismatch while its input context is live.
                    report_prefix_mismatch(self);
                    return Ok(false);
                }
                Err(error) => return Err(error),
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
            if activation == MacroActivationClass::Matching && body.is_empty() {
                self.conserve_input_stack()?;
                self.command
                    .record_empty_matching_macro_activation(parameter_count);
                return Ok(true);
            }
            self.conserve_input_stack_for_descendant()?;
            let parameter_base = self.command.input.levels.parameters.len();
            let _level = self
                .command
                .push_macro_activation(macro_name, body, matching.take(), call_site)
                .map_err(|_| CommandError::input_invariant())?;
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
                    argument_count: parameter_count as u8,
                    token_count: self.command.input.levels.parameters[parameter_base..]
                        .iter()
                        .map(|span| u64::from(span.len))
                        .sum(),
                }),
            );
            Ok(true)
        })();
        // Every Result path settles the one local scan and unfinished owner,
        // including failures conserving depleted parents after a complete match.
        if let Some(episode) = episode {
            self.finish_scanner_episode(episode);
        }
        self.command.input.levels.arguments.abort_capture();
        result
    }

    #[inline(always)]
    #[cold]
    pub(crate) fn report_macro_prefix_mismatch(
        &mut self,
        call: &crate::command::CurrentCommand<G>,
    ) {
        let site = self.capture_hot_diagnostic_site(call);
        self.queue_macro_prefix_mismatch(call.control_sequence().expect("macro name"), site);
        if self.is_observed() {
            let observed_call = call;
            self.observe_command_diagnostic("macro_prefix_mismatch", observed_call);
        }
    }

    #[cold]
    pub(crate) fn report_unobserved_macro_prefix_mismatch(
        &mut self,
        spelling: TracedTokenWord,
        flags: MeaningFlags,
        definition: DefinitionRef<G>,
        name: tex_state::interner::Symbol,
    ) {
        let site = self.capture_spelled_diagnostic_site(
            crate::command::CommandIdentity::Ordinary,
            spelling,
            Some(name),
            tex_state::meaning::ResolvedMeaning::Macro { flags, definition },
        );
        self.queue_macro_prefix_mismatch(name, site);
    }

    fn queue_macro_prefix_mismatch(
        &mut self,
        macro_name: tex_state::interner::Symbol,
        site: tex_state::diagnostic::DiagnosticSite,
    ) {
        let context = self.command.output_open_context(self.state);
        let site = Some(self.complete_diagnostic_site(site));
        self.command.semantic_diagnostics.push(
            crate::CommandSemanticDiagnostic::MacroPrefixMismatch {
                macro_name,
                context,
                site,
            },
        );
    }

    fn classify_macro_activation(
        &self,
        parameterless: bool,
        replacement_is_empty: bool,
    ) -> MacroActivationClass {
        // Settlement has already consumed token-local suppression and outer
        // facts: suppression leaves a Relax command, while scanner+outer
        // recovery leaves a space. Neither can be a valid macro dispatch, so
        // this aggregate check is intentionally persistent-only.
        let persistent_settlement = self.command.delivery_mode.requires_persistent_settlement();
        let exceptional = persistent_settlement
            || self.state.int_param(IntParam::TRACING_MACROS) > 0
            || (replacement_is_empty && self.empty_macro_needs_completion_descendant());
        if exceptional {
            MacroActivationClass::Exceptional
        } else if parameterless {
            MacroActivationClass::Simple
        } else {
            MacroActivationClass::Matching
        }
    }

    /// True when §390 will retire the owner of a replay-completion fence and
    /// therefore needs the normally installed macro row as its descendant.
    fn empty_macro_needs_completion_descendant(&self) -> bool {
        let Some(owner) = self.command.youngest_replay_completion_owner() else {
            return false;
        };
        self.command
            .input
            .levels
            .iter()
            .rev()
            .take_while(|level| {
                matches!(level, crate::input::InputLevel::Resident(row)
                    if !matches!(row.header.behavior(), crate::input::TokenBehavior::VTemplate)
                        && level.stored_is_exhausted() == Some(true))
            })
            .any(|level| crate::input::input_level_identity(level) == owner)
    }

    /// TeX82 §392's `end_match` branch for an ordinary unobserved macro.
    ///
    /// No argument owner or rich command is constructed. The replacement
    /// owner moves directly into its input row after §390 conservation; an
    /// empty replacement records the same logical push maximum but exposes no
    /// immediately exhausted row.
    #[inline(always)]
    fn activate_simple_macro(
        &mut self,
        macro_name: tex_state::interner::Symbol,
        call_site: OriginId,
        body: Option<tex_state::ResidentMacroBody<G>>,
    ) -> Result<bool, CommandError> {
        if let Some(body) = body {
            self.conserve_input_stack_for_descendant()?;
            self.command
                .push_macro_activation(macro_name, body, None, call_site)
                .map_err(|_| CommandError::input_invariant())?;
        } else {
            self.conserve_input_stack()?;
            self.command.record_empty_macro_activation();
            #[cfg(test)]
            MACRO_ACTIVATION_COUNTERS.with(|slot| {
                let mut counters = slot.get();
                counters.empty_rows_elided = counters.empty_rows_elided.saturating_add(1);
                slot.set(counters);
            });
        }
        Ok(true)
    }

    fn macro_call_scalar(
        &mut self,
        matching: Option<&mut MatchedArguments>,
        body: &ResidentMacroBody<G>,
        flags: MeaningFlags,
        macro_name: tex_state::interner::Symbol,
    ) -> Result<(), CommandError> {
        let paragraph_token = self.paragraph_word();
        let parameter_count = body.parameter_pattern().parameter_count();
        let leading_end = body.parameter_pattern().leading_end(body.parameter_len());
        let mut delivery = None;
        for index in 0..leading_end {
            let expected = body
                .parameter_word(index)
                .ok_or_else(CommandError::input_invariant)?;
            if self.get_macro_match_token(&mut delivery)? != crate::DeliveryStatus::Command {
                return Err(CommandError::MacroPrefixMismatch);
            }
            let actual = delivery.take().ok_or_else(CommandError::input_invariant)?;
            if actual.spelling_word() != expected {
                // TeX82 §391 tests every compulsory parameter-text token
                // after raw delivery has completed §336 recovery. An outer
                // control sequence therefore contributes the inserted
                // frozen `\par` to this same mismatch test; only §394's
                // argument writer turns that recovery into an aborted
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
        if parameter_count == 0
            && leading_end != 0
            && is_begin_group(
                body.parameter_word(leading_end - 1)
                    .ok_or_else(CommandError::input_invariant)?,
            )
        {
            self.undo_delimiter_begin_group_delivery();
        }

        let mut matching = matching;
        for parameter in 0..parameter_count {
            let matching = matching
                .as_deref_mut()
                .ok_or_else(CommandError::input_invariant)?;
            let delimiter = body
                .parameter_pattern()
                .delimiter(parameter, body.parameter_words());
            let argument = if delimiter.is_empty() {
                self.scan_undelimited_argument(matching, flags, paragraph_token)?
            } else {
                self.scan_delimited_argument(matching, flags, delimiter, paragraph_token)?
            };
            let marker = body.parameter_pattern().marker_index(parameter).map_or(
                Ok('#'),
                |index| match body
                    .parameter_word(index)
                    .ok_or_else(CommandError::input_invariant)?
                    .semantic_token()
                {
                    Token::Char { ch, .. } => Ok(ch),
                    _ => Err(CommandError::input_invariant()),
                },
            )?;
            self.trace_macro_argument(marker, parameter + 1, &argument)?;
            let argument_token_count = argument
                .visible_len()
                .map_err(|_| CommandError::input_invariant())?;
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "macro_delimiter_match",
                    tokens: delimiter
                        .words()
                        .iter()
                        .copied()
                        .map(|word| {
                            self.observed_token(TracedTokenWord::from_parts(
                                word,
                                OriginId::UNKNOWN,
                            ))
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
                        .input
                        .levels
                        .arguments
                        .match_words(&argument)
                        .expect("matched macro argument remains live until sealing")
                        .map(|token| self.observed_token(token))
                        .collect(),
                }),
            );
            self.command
                .input
                .levels
                .arguments
                .publish_argument(matching, argument)
                .map_err(|_| CommandError::input_invariant())?;
        }
        Ok(())
    }

    /// TeX82 §389's invocation trace, including `print_ln` before the macro
    /// name and §262's control-word separator before `->`.
    fn trace_macro_invocation(
        &mut self,
        macro_name: tex_state::interner::Symbol,
        definition: &DefinitionRef<G>,
    ) {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 0 {
            return;
        }
        let mut text = String::new();
        crate::processor::expand_render::append_print_cs_text(self.state, macro_name, &mut text);
        // TeX82 §§389 uses `token_show` on the stored definition. A
        // non-`#` parameter marker is stored beside its compact out-parameter
        // slot and must render as one pair (`U3`), not as the literal marker
        // followed by the generic `#3` spelling.
        let definition = self.state.definition(*definition);
        crate::processor::expand_render::append_meaning_token_words(
            self.state,
            definition.parameter_text().iter(),
            false,
            &mut text,
        );
        text.push_str("->");
        crate::processor::expand_render::append_meaning_token_words(
            self.state,
            definition.replacement_text().iter(),
            false,
            &mut text,
        );
        self.print_macro_trace(text, true);
    }

    /// TeX82 §400's `#n<-<argument>` trace in completed-argument order.
    fn trace_macro_argument(
        &mut self,
        marker: char,
        parameter: usize,
        argument: &ArgumentCursor,
    ) -> Result<(), CommandError> {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 0 {
            return Ok(());
        }
        let mut text = format!("{marker}{parameter}<-");
        for word in self
            .command
            .input
            .levels
            .arguments
            .match_words(argument)
            .map_err(|_| CommandError::input_invariant())?
        {
            crate::processor::expand_render::append_token_list_token_text(
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
                crate::processor::expand_render::print_esc_text(self.state, &spelling)
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
        let site = Some(self.current_diagnostic_site(None));
        let mut display = String::new();
        for token in partial {
            crate::processor::expand_render::append_token_list_token_text(
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
                site,
            });
    }

    /// TeX82 §395's `<Report an extra right brace and goto continue>`.
    fn report_extra_right_brace_argument(&mut self) {
        let name = self.matching_macro_name();
        let context = self.command.output_open_context(self.state);
        let site = Some(self.current_diagnostic_site(None));
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
                site,
            });
    }

    /// TeX82 §395's complete extra-right-brace recovery, shared by the
    /// undelimited and delimited branches of §394's parameter matcher.
    fn recover_extra_right_brace_argument(
        &mut self,
        delivery: CurrentCommand<G>,
    ) -> Result<ArgumentCursor, CommandError> {
        self.back_input_hot(delivery)?;
        self.insert_macro_argument_recovery_par()?;
        // §395 ends with `ins_error`, so §82 renders the context with
        // the inserted `\par` level already on the stack.
        self.report_extra_right_brace_argument();
        let mut par = None;
        if self.get_macro_match_token(&mut par)? != crate::DeliveryStatus::Command {
            return Err(CommandError::ParagraphInMacroArgument);
        }
        let par = par.take().ok_or_else(CommandError::input_invariant)?;
        self.back_input_hot(par)?;
        // §395's `goto continue` immediately reads the inserted `\par`;
        // `long_state := call` makes §396 abort even a `\long` macro.
        self.report_paragraph_ended_before_complete(&[]);
        Err(CommandError::ParagraphInMacroArgument)
    }

    fn scan_undelimited_argument(
        &mut self,
        matching: &MatchedArguments,
        flags: MeaningFlags,
        paragraph_token: Option<TokenWord>,
    ) -> Result<ArgumentCursor, CommandError> {
        // TeX82 §394 links the opening left brace into the temporary
        // argument list and removes the matching outer pair only after the
        // argument completes.  Keep that ownership here too: §396's
        // runaway pseudoprint must still see an unmatched opening brace.
        let mut tokens = self.command.input.levels.arguments.begin_argument(matching);
        let mut delivery = None;
        loop {
            // A resident run admits a sole ordinary token or a whole balanced
            // group, completing the argument without scalar delivery. It
            // stops before a leading space, a stray closing brace, and every
            // word needing ordinary settlement.
            if !self.is_observed() {
                let run = self.consume_argument_run(&mut tokens, paragraph_token, None)?;
                if run.consumed != 0 {
                    if tokens.brace_depth() == 0 {
                        return self.strip_argument_outer_group(tokens);
                    }
                    break;
                }
                if run.at_transition && self.settle_resident_run_boundary()? {
                    continue;
                }
            }
            if self.get_macro_match_token(&mut delivery)? != crate::DeliveryStatus::Command {
                return Err(CommandError::ParagraphInMacroArgument);
            }
            let current = delivery
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            if self.outer_recovered_while_matching && current.is_effective_paragraph() {
                return Err(CommandError::OuterInMacroArgument);
            }
            if current.literal_catcode() == Some(Catcode::Space) {
                continue;
            }
            if current.literal_catcode() == Some(Catcode::EndGroup) {
                return self.recover_extra_right_brace_argument(
                    delivery.take().ok_or_else(CommandError::input_invariant)?,
                );
            }
            self.check_argument_paragraph(current, paragraph_token, flags, None)?;
            let depth = self
                .command
                .input
                .levels
                .arguments
                .append_hot_delivery(&mut tokens, current, true, paragraph_token)
                .map_err(|_| CommandError::input_invariant())?;
            if depth == 0 {
                return Ok(tokens);
            }
            break;
        }
        loop {
            if !self.is_observed() {
                let run = self.consume_argument_run(&mut tokens, paragraph_token, None)?;
                if run.consumed != 0 && tokens.brace_depth() == 0 {
                    return self.strip_argument_outer_group(tokens);
                }
                if run.at_transition && self.settle_resident_run_boundary()? {
                    continue;
                }
            }
            if self.get_macro_match_token(&mut delivery)? != crate::DeliveryStatus::Command {
                return Err(CommandError::ParagraphInMacroArgument);
            }
            let current = delivery
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            // TeX82 §23's recovered `cur_cmd := spacer` is the return
            // value of the interrupted raw delivery, not a token linked into
            // §394's temporary argument list. The inserted `\par`
            // aborts this match on the next demand; §306's already-owned
            // runaway pseudoprint must therefore end at the last real token.
            if current.is_outer_recovery_space() {
                continue;
            }
            if self.outer_recovered_while_matching && current.is_effective_paragraph() {
                let partial = self
                    .command
                    .input
                    .levels
                    .arguments
                    .match_words(&tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    .collect::<Vec<_>>();
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
                return Err(CommandError::OuterInMacroArgument);
            }
            self.check_argument_paragraph(current, paragraph_token, flags, Some(&tokens))?;
            let closes_outer_group = current.literal_catcode() == Some(Catcode::EndGroup);
            let depth = self
                .command
                .input
                .levels
                .arguments
                .append_hot_delivery(&mut tokens, current, true, paragraph_token)
                .map_err(|_| CommandError::input_invariant())?;
            if closes_outer_group && depth == 0 {
                tokens = self.strip_argument_outer_group(tokens)?;
                return Ok(tokens);
            }
        }
    }

    /// TeX.web §394's literal, scalar delimiter matcher. A failed delimiter
    /// prefix commits only the part that cannot be reused as an overlapping
    /// prefix, then retains the maximal suffix that still matches the start
    /// of the delimiter using its sealed failure links. Token catcodes,
    /// brace depth, and the recovery splice remain semantic here.
    fn scan_delimited_argument(
        &mut self,
        matching: &MatchedArguments,
        flags: MeaningFlags,
        delimiter: MacroDelimiter<'_>,
        paragraph_token: Option<TokenWord>,
    ) -> Result<ArgumentCursor, CommandError> {
        debug_assert!(!delimiter.is_empty());
        let mut tokens = self.command.input.levels.arguments.begin_argument(matching);
        let mut delivery = None;

        loop {
            // Delimiters are inactive inside a balanced group, and at depth
            // zero with no pending delimiter prefix a word other than the
            // delimiter's first token cannot start a match. Consume such
            // ordinary words in place and return to scalar delivery only at a
            // semantic boundary. A held prefix exists only at depth zero.
            if !self.is_observed()
                && self
                    .command
                    .input
                    .levels
                    .arguments
                    .delimiter_prefix_len(&tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    == 0
            {
                let stop_word = delimiter.words()[0];
                let run =
                    self.consume_argument_run(&mut tokens, paragraph_token, Some(stop_word))?;
                if run.at_transition && self.settle_resident_run_boundary()? {
                    continue;
                }
            }
            if self.get_macro_match_token(&mut delivery)? != crate::DeliveryStatus::Command {
                return Err(CommandError::ParagraphInMacroArgument);
            }
            let current = delivery
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            if current.is_outer_recovery_space() {
                continue;
            }
            if self.outer_recovered_while_matching && current.is_effective_paragraph() {
                let mut partial = self
                    .command
                    .input
                    .levels
                    .arguments
                    .match_words(&tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    .collect::<Vec<_>>();
                partial.extend(
                    self.command
                        .input
                        .levels
                        .arguments
                        .delimiter_prefix_words(&tokens)
                        .map_err(|_| CommandError::input_invariant())?,
                );
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
                return Err(CommandError::OuterInMacroArgument);
            }

            let spelling = current.spelling_word();
            let prefix_len = self
                .command
                .input
                .levels
                .arguments
                .delimiter_prefix_len(&tokens)
                .map_err(|_| CommandError::input_invariant())?;
            if tokens.brace_depth() == 0
                && prefix_len < delimiter.len()
                && spelling == delimiter.words()[prefix_len]
            {
                self.command
                    .input
                    .levels
                    .arguments
                    .append_delimiter_word(&mut tokens, current.spelling())
                    .map_err(|_| CommandError::input_invariant())?;
                if self
                    .command
                    .input
                    .levels
                    .arguments
                    .delimiter_prefix_len(&tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    == delimiter.len()
                {
                    // `#{` consumes the opening brace as parameter text. Raw
                    // delivery has accounted for it, but no replacement-body
                    // replay exists yet to provide the balancing delivery.
                    if current.literal_catcode() == Some(Catcode::BeginGroup) {
                        self.undo_delimiter_begin_group_delivery();
                    }
                    self.command
                        .input
                        .levels
                        .arguments
                        .truncate_delimiter_suffix(&mut tokens)
                        .map_err(|_| CommandError::input_invariant())?;
                    tokens = self.strip_argument_outer_group(tokens)?;
                    return Ok(tokens);
                }
                continue;
            }

            if prefix_len != 0 {
                let retained = delimiter.overlap(prefix_len, spelling);
                // If the current token is retained, every already-held word
                // before that suffix is now committed.  If no suffix remains,
                // all held words are committed and the current token is
                // handled as ordinary argument material below.
                let committed = if retained == 0 {
                    prefix_len
                } else {
                    prefix_len + 1 - retained
                };
                if self.is_observed() {
                    for index in 0..committed {
                        let prefix_token = self
                            .command
                            .input
                            .levels
                            .arguments
                            .delimiter_prefix_word(&tokens, index)
                            .map_err(|_| CommandError::input_invariant())?;
                        observe!(
                            self,
                            CommandObservation::TokenList(TokenListRecord {
                                transition: "splice",
                                purpose: "macro_delimiter_recovery",
                                tokens: vec![self.observed_token(prefix_token)],
                            }),
                        );
                    }
                }
                self.command
                    .input
                    .levels
                    .arguments
                    .reveal_delimiter_words(&mut tokens, committed as u32)
                    .map_err(|_| CommandError::input_invariant())?;
                if retained != 0 {
                    // The current spelling is the final word of the retained
                    // suffix. It is unpublished until the next mismatch or
                    // until the complete delimiter truncates the holdback.
                    self.command
                        .input
                        .levels
                        .arguments
                        .append_delimiter_word(&mut tokens, current.spelling())
                        .map_err(|_| CommandError::input_invariant())?;
                    continue;
                }
            }

            // The failed delimiter prefix is revealed before §395 examines
            // the current token. A held opening brace can consequently make a
            // closing brace ordinary argument material.
            if tokens.brace_depth() == 0 && current.literal_catcode() == Some(Catcode::EndGroup) {
                return self.recover_extra_right_brace_argument(
                    delivery.take().ok_or_else(CommandError::input_invariant)?,
                );
            }
            self.check_argument_paragraph(current, paragraph_token, flags, Some(&tokens))?;
            self.command
                .input
                .levels
                .arguments
                .append_hot_delivery(&mut tokens, current, true, paragraph_token)
                .map_err(|_| CommandError::input_invariant())?;
        }
    }

    /// Consumes one batched run of ordinary argument words. Control
    /// sequences are classified only while no alignment interception or outer
    /// recovery can apply to them.
    fn consume_argument_run(
        &mut self,
        tokens: &mut ArgumentCursor,
        paragraph_token: Option<TokenWord>,
        stop_word: Option<TokenWord>,
    ) -> Result<crate::input::ResidentRun, CommandError> {
        // An undelimited argument (no delimiter stop word) ends at its own
        // closing brace, which its run admits last; a delimited argument may
        // close any group it opened.
        let close_floor = u32::from(stop_word.is_none());
        let admission = ArgumentRunAdmission::with_commands(
            self.state,
            paragraph_token,
            stop_word,
            tokens.brace_depth(),
            close_floor,
        );
        let admission = if stop_word.is_none() {
            admission.completing_undelimited()
        } else {
            admission
        };
        let admission = if self.command.delivery_mode.alignment_active()
            || self.outer_recovered_while_matching
        {
            admission.characters_only()
        } else {
            admission
        };
        self.command
            .consume_plain_resident_run(tokens, self.fuel, admission)
    }

    fn check_argument_paragraph(
        &mut self,
        delivery: &CurrentCommand<G>,
        paragraph_token: Option<TokenWord>,
        flags: MeaningFlags,
        partial: Option<&ArgumentCursor>,
    ) -> Result<(), CommandError> {
        if self.eof_recovered_while_matching && delivery.is_effective_paragraph() {
            // TeX82 §23 calls `check_outer_validity` after source EOF and
            // changes `long_state` to `outer_call`, even for a `\long` macro.
            // Its inserted frozen `\par` terminates the match but is consumed
            // by the failed expansion instead of being replayed by §396.
            let partial = partial
                .map(|buffer| {
                    self.command
                        .input
                        .levels
                        .arguments
                        .match_words(buffer)
                        .map(|words| words.collect::<Vec<_>>())
                        .map_err(|_| CommandError::input_invariant())
                })
                .transpose()?
                .unwrap_or_default();
            self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        if delivery.is_paragraph_spelling(paragraph_token) && !flags.contains(MeaningFlags::LONG) {
            // TeX82 §394 reports this through `back_error` while the macro
            // matcher is still live.  The caller will then restore its
            // enclosing scanner status, so retain the exact `\par` input
            // ahead of that restoration rather than merely returning an
            // error from the scalar matcher.
            self.back_input_hot(*delivery)?;
            // §396 ends with `back_error`, so §82 renders the context with the
            // replayed `\par` already on the stack.
            let partial = partial
                .map(|buffer| {
                    self.command
                        .input
                        .levels
                        .arguments
                        .match_words(buffer)
                        .map(|words| words.collect::<Vec<_>>())
                        .map_err(|_| CommandError::input_invariant())
                })
                .transpose()?
                .unwrap_or_default();
            self.report_paragraph_ended_before_complete(&partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        Ok(())
    }

    fn strip_argument_outer_group(
        &mut self,
        mut buffer: ArgumentCursor,
    ) -> Result<ArgumentCursor, CommandError> {
        if buffer.facts().removable_outer_group() {
            buffer
                .strip_outer_group()
                .map_err(|_| CommandError::input_invariant())?;
        }
        Ok(buffer)
    }
}

fn is_begin_group(token: TokenWord) -> bool {
    matches!(token.literal_catcode(), Some(Catcode::BeginGroup))
}

#[cfg(test)]
mod tests;
