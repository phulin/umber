//! Private canonical scalar macro-call state machine.
#![allow(dead_code)] // expansion dispatch is the next ordered integration slice
use smallvec::SmallVec;
use tex_state::env::banks::IntParam;
use tex_state::macro_definition::MacroParameterPattern;
use tex_state::meaning::MeaningFlags;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};
use tex_state::{DefinitionRef, ResidentMacroBody};

use crate::command::MacroMatchDelivery;
use crate::execution_scratch::{ArgumentSetId, MacroArgumentWriter, PendingArgumentSet};
use crate::processor::status::{
    ArgumentBuilderId, MatchingContext, ScannerStatus, ScannerStatusVisibility, ScannerWarning,
};
use crate::{CommandError, CommandProcessor};

use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, MacroRecord, TokenListRecord,
};

const EXTRA_RIGHT_BRACE_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0395;
pub(crate) const RUNAWAY_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0396;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct MacroDelimiter {
    start: usize,
    len: usize,
}

/// One admitted immutable macro invocation.
///
/// The definition region/body owner and parameter pattern are acquired once
/// at the opener. Matching then walks the packed parameter span through this
/// resident owner; it never asks the command state to rediscover the
/// definition for each pattern token.
#[derive(Debug)]
struct MacroPlan<G> {
    flags: MeaningFlags,
    macro_name: tex_state::interner::Symbol,
    call_origin: OriginId,
    definition: DefinitionRef<G>,
    pattern: MacroParameterPattern,
    parameter_len: usize,
    body: ResidentMacroBody<G>,
}

impl<G> MacroPlan<G> {
    #[inline(always)]
    fn parameter_word(&self, index: usize) -> Result<TokenWord, CommandError> {
        self.body
            .parameter_word(index)
            .ok_or_else(CommandError::input_invariant)
    }

    #[inline(always)]
    fn delimiter_word(
        &self,
        delimiter: MacroDelimiter,
        index: usize,
    ) -> Result<TokenWord, CommandError> {
        if index >= delimiter.len {
            return Err(CommandError::input_invariant());
        }
        self.parameter_word(delimiter.start + index)
    }
}

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
    /// The ordinary expansion loop owns the command for the complete call and
    /// moves it into retry state only when a typed resource barrier is hit.
    pub(crate) fn macro_call(
        &mut self,
        call: &mut crate::CurrentCommand<G>,
    ) -> Result<bool, CommandError> {
        let mut hot = crate::command::HotCommand::from_current_ref(call);
        self.macro_call_hot(&mut hot)
    }

    pub(crate) fn macro_call_hot(
        &mut self,
        call: &mut crate::command::HotCommand<G>,
    ) -> Result<bool, CommandError> {
        let Some((flags, definition)) = call.macro_parts() else {
            return Err(CommandError::input_invariant());
        };
        let macro_name = call
            .control_sequence()
            .ok_or(CommandError::input_invariant())?;
        let call_site = call.origin();
        let admitted = self
            .state
            .admit_macro_definition(definition)
            .ok_or_else(CommandError::input_invariant)?;
        let (activation, pattern, body) = match admitted {
            tex_state::AdmittedMacroDefinition::SimpleMacro { pattern, body } => {
                let activation = self.classify_macro_activation(true, body.is_none());
                if activation == MacroActivationClass::Simple {
                    record_macro_activation_class(activation);
                    return self.activate_simple_macro(macro_name, call_site, body);
                }
                let body = match body {
                    Some(body) => body,
                    None => {
                        self.state
                            .admit_macro_body(definition)
                            .ok_or_else(CommandError::input_invariant)?
                            .2
                    }
                };
                (activation, pattern, body)
            }
            tex_state::AdmittedMacroDefinition::MatchingMacro {
                pattern,
                parameter_len: _,
                body,
            } => (
                self.classify_macro_activation(false, body.is_empty()),
                pattern,
                body,
            ),
        };
        let plan = MacroPlan {
            flags,
            macro_name,
            call_origin: call_site,
            definition,
            parameter_len: body.parameter_len(),
            pattern,
            body,
        };
        record_macro_activation_class(activation);
        self.trace_macro_invocation(plan.macro_name, &plan.definition);
        // TeX82 §389 calls the §391 parameter matcher only when the macro's
        // parameter text does not begin with `end_match`. A parameterless
        // macro therefore feeds its replacement directly, without a transient
        // `matching` scanner episode. Literal leading tokens still need the
        // matcher even when there are no numbered parameters.
        let needs_matching = plan.pattern.leading_end(plan.parameter_len) != 0
            || plan.pattern.parameter_count() != 0;
        // Only numbered parameters need the reusable argument lane. A macro
        // whose parameter text is solely a compulsory literal prefix matches
        // directly from its immutable definition metadata and creates no
        // MacroMatch, ArgumentSet, writer, or captured-word block.
        let matching = (plan.pattern.parameter_count() != 0)
            .then(|| self.command.scratch.begin_macro_match())
            .transpose()
            .map_err(|_| CommandError::input_invariant())?;
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
        let scanned_arguments = if needs_matching {
            self.macro_call_scalar(matching.as_ref(), &plan)
        } else {
            Ok(())
        };
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
                        macro_name: plan.macro_name,
                        context,
                    },
                );
                let observed_call = call.materialize();
                self.observe_command_diagnostic("macro_prefix_mismatch", &observed_call);
                if let Some(episode) = episode {
                    self.finish_scanner_episode(episode);
                }
                if let Some(matching) = matching {
                    self.command
                        .scratch
                        .discard_macro_match(matching)
                        .map_err(|_| CommandError::input_invariant())?;
                }
                return Ok(false);
            }
            Err(error) => {
                if let Some(episode) = episode {
                    self.finish_scanner_episode(episode);
                }
                if let Some(matching) = matching {
                    self.command
                        .scratch
                        .discard_macro_match(matching)
                        .map_err(|_| CommandError::input_invariant())?;
                }
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
        self.conserve_input_stack_for_descendant()?;
        let arguments = if plan.pattern.parameter_count() == 0 {
            if let Some(matching) = matching {
                self.command
                    .scratch
                    .discard_macro_match(matching)
                    .map_err(|_| CommandError::input_invariant())?;
            }
            None
        } else {
            let frame = self
                .command
                .scratch
                .commit_macro_match(matching.ok_or_else(CommandError::input_invariant)?)
                .map_err(|_| CommandError::input_invariant())?;
            Some(frame)
        };
        let macro_name = plan.macro_name;
        let call_origin = plan.call_origin;
        let parameter_count = plan.pattern.parameter_count();
        let body = plan.body;
        let _level = self.push_macro_activation(macro_name, body, call_origin, arguments);
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
                token_count: arguments.map_or(0, |arguments| {
                    self.argument_token_count(arguments) as u64
                }),
            }),
        );
        if let Some(episode) = episode {
            self.finish_scanner_episode(episode);
        }
        Ok(true)
    }

    #[inline(always)]
    fn classify_macro_activation(
        &self,
        parameterless: bool,
        replacement_is_empty: bool,
    ) -> MacroActivationClass {
        let exceptional = self.command.delivery_mode.requires_slow_settlement()
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
            let _ = self.push_macro_activation(macro_name, body, call_site, None);
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
        matching: Option<&PendingArgumentSet<G>>,
        plan: &MacroPlan<G>,
    ) -> Result<(), CommandError> {
        let paragraph_token = self.state.symbol("par").map(Token::Cs).map(TokenWord::pack);
        for index in 0..plan.pattern.leading_end(plan.parameter_len) {
            let expected = plan.parameter_word(index)?;
            let actual = self
                .get_macro_match_token(paragraph_token)?
                .ok_or(CommandError::MacroPrefixMismatch)?;
            if actual.word() != expected {
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
        if plan.pattern.parameter_count() == 0
            && plan.pattern.leading_end(plan.parameter_len) != 0
            && is_begin_group(
                plan.parameter_word(plan.pattern.leading_end(plan.parameter_len) - 1)?,
            )
        {
            self.undo_delimiter_begin_group_delivery();
        }

        for parameter in 0..plan.pattern.parameter_count() {
            let matching = matching.ok_or_else(CommandError::input_invariant)?;
            let (start, end) = plan.pattern.delimiter_bounds(parameter, plan.parameter_len);
            let delimiter = MacroDelimiter {
                start,
                len: end - start,
            };
            let argument = if delimiter.len == 0 {
                self.scan_undelimited_argument(matching, plan.flags, paragraph_token)?
            } else {
                self.scan_delimited_argument(
                    matching,
                    plan.flags,
                    plan,
                    delimiter,
                    paragraph_token,
                )?
            };
            let marker =
                plan.pattern
                    .marker_index(parameter)
                    .map_or(Ok('#'), |index| {
                        match plan.parameter_word(index)?.semantic_token() {
                            Token::Char { ch, .. } => Ok(ch),
                            _ => Err(CommandError::input_invariant()),
                        }
                    })?;
            self.trace_macro_argument(marker, parameter + 1, &argument)?;
            let argument_token_count = argument
                .visible_len()
                .map_err(|_| CommandError::input_invariant())?;
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "macro_delimiter_match",
                    tokens: (0..delimiter.len)
                        .filter_map(|index| plan.delimiter_word(delimiter, index).ok())
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
                    control_sequence: self.state.resolve(plan.macro_name).to_owned(),
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
                .publish_argument(argument)
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
        drop(definition);
        self.print_macro_trace(text, true);
    }

    /// TeX82 §400's `#n<-<argument>` trace in completed-argument order.
    fn trace_macro_argument(
        &mut self,
        marker: char,
        parameter: usize,
        argument: &MacroArgumentWriter<G>,
    ) -> Result<(), CommandError> {
        if self.state.int_param(IntParam::TRACING_MACROS) <= 0 {
            return Ok(());
        }
        let mut text = format!("{marker}{parameter}<-");
        for word in self
            .command
            .scratch
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
        delivery: MacroMatchDelivery<G>,
    ) -> Result<MacroArgumentWriter<G>, CommandError> {
        self.back_input_hot(delivery.into_hot())?;
        self.insert_macro_argument_recovery_par()?;
        // §395 ends with `ins_error`, so §82 renders the context with
        // the inserted `\par` level already on the stack.
        self.report_extra_right_brace_argument();
        let par = self
            .get_macro_match_token(None)?
            .ok_or(CommandError::ParagraphInMacroArgument)?;
        self.back_input_hot(par.into_hot())?;
        // §395's `goto continue` immediately reads the inserted `\par`;
        // `long_state := call` makes §396 abort even a `\long` macro.
        self.report_paragraph_ended_before_complete(&[]);
        Err(CommandError::ParagraphInMacroArgument)
    }

    fn scan_undelimited_argument(
        &mut self,
        matching: &PendingArgumentSet<G>,
        flags: MeaningFlags,
        paragraph_token: Option<TokenWord>,
    ) -> Result<MacroArgumentWriter<G>, CommandError> {
        let first = loop {
            let delivery = self
                .get_macro_match_token(paragraph_token)?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            if self.outer_recovered_while_matching && delivery.effective_paragraph() {
                return Err(CommandError::OuterInMacroArgument);
            }
            if delivery.word().literal_catcode() == Some(Catcode::Space) {
                continue;
            }
            if delivery.word().literal_catcode() == Some(Catcode::EndGroup) {
                return self.recover_extra_right_brace_argument(delivery);
            }
            break delivery;
        };
        self.check_argument_paragraph(&first, flags, None)?;
        if first.word().literal_catcode() != Some(Catcode::BeginGroup) {
            let mut tokens = self
                .command
                .scratch
                .begin_argument_writer(matching)
                .map_err(|_| CommandError::input_invariant())?;
            self.command
                .scratch
                .append_match_delivery(&mut tokens, &first, true)
                .map_err(|_| CommandError::input_invariant())?;
            return Ok(tokens);
        }

        // TeX82 §394 links the opening left brace into the temporary
        // argument list and removes the matching outer pair only after the
        // argument completes.  Keep that ownership here too: §396's
        // runaway pseudoprint must still see an unmatched opening brace.
        let mut tokens = self
            .command
            .scratch
            .begin_argument_writer(matching)
            .map_err(|_| CommandError::input_invariant())?;
        let first_depth = self
            .command
            .scratch
            .append_match_delivery(&mut tokens, &first, true)
            .map_err(|_| CommandError::input_invariant())?;
        debug_assert_eq!(first_depth, 1);
        loop {
            if !self.is_observed()
                && self
                    .command
                    .consume_plain_macro_body_argument_run(&mut tokens, self.fuel)?
                    != 0
            {
                continue;
            }
            let delivery = self
                .get_macro_match_token(paragraph_token)?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            // TeX82 §23's recovered `cur_cmd := spacer` is the return
            // value of the interrupted raw delivery, not a token linked into
            // §394's temporary argument list. The inserted `\par`
            // aborts this match on the next demand; §306's already-owned
            // runaway pseudoprint must therefore end at the last real token.
            if delivery.is_outer_recovery_space() {
                continue;
            }
            if self.outer_recovered_while_matching && delivery.effective_paragraph() {
                let partial = self
                    .command
                    .scratch
                    .match_words(&tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    .collect::<Vec<_>>();
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
                return Err(CommandError::OuterInMacroArgument);
            }
            self.check_argument_paragraph(&delivery, flags, Some(&tokens))?;
            let closes_outer_group = delivery.word().literal_catcode() == Some(Catcode::EndGroup);
            let depth = self
                .command
                .scratch
                .append_match_delivery(&mut tokens, &delivery, true)
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
    /// of the delimiter. This is intentionally not a compiled string matcher:
    /// token catcodes, brace depth, and the recovery splice are semantic here.
    fn scan_delimited_argument(
        &mut self,
        matching: &PendingArgumentSet<G>,
        flags: MeaningFlags,
        plan: &MacroPlan<G>,
        delimiter: MacroDelimiter,
        paragraph_token: Option<TokenWord>,
    ) -> Result<MacroArgumentWriter<G>, CommandError> {
        debug_assert_ne!(delimiter.len, 0);
        // Build immutable KMP failure links once for this delimiter.  The
        // matcher retains only the current state in its writer; overlap
        // fallback therefore stays linear in the input stream.
        let mut failure = SmallVec::<[u32; 32]>::new();
        self.build_delimiter_failure(plan, delimiter, &mut failure)?;
        let mut tokens = self
            .command
            .scratch
            .begin_argument_writer(matching)
            .map_err(|_| CommandError::input_invariant())?;

        loop {
            // Delimiters are inactive inside a balanced group. Consume the
            // ordinary literal prefix of an admitted macro body in place and
            // return to scalar delivery only at a semantic boundary.
            if tokens.brace_depth() != 0
                && !self.is_observed()
                && self
                    .command
                    .consume_plain_macro_body_argument_run(&mut tokens, self.fuel)?
                    != 0
            {
                continue;
            }
            let delivery = self
                .get_macro_match_token(paragraph_token)?
                .ok_or(CommandError::ParagraphInMacroArgument)?;
            if delivery.is_outer_recovery_space() {
                continue;
            }
            if self.outer_recovered_while_matching && delivery.effective_paragraph() {
                let mut partial = self
                    .command
                    .scratch
                    .match_words(&tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    .collect::<Vec<_>>();
                partial.extend(
                    self.command
                        .scratch
                        .delimiter_prefix_words(&tokens)
                        .map_err(|_| CommandError::input_invariant())?,
                );
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
                return Err(CommandError::OuterInMacroArgument);
            }

            let spelling = delivery.word();
            let prefix_len = self
                .command
                .scratch
                .delimiter_prefix_len(&tokens)
                .map_err(|_| CommandError::input_invariant())?;
            if tokens.brace_depth() == 0
                && prefix_len < delimiter.len
                && spelling == plan.delimiter_word(delimiter, prefix_len)?
            {
                self.command
                    .scratch
                    .append_delimiter_word(&mut tokens, delivery.spelling())
                    .map_err(|_| CommandError::input_invariant())?;
                if self
                    .command
                    .scratch
                    .delimiter_prefix_len(&tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    == delimiter.len
                {
                    // `#{` consumes the opening brace as parameter text. Raw
                    // delivery has accounted for it, but no replacement-body
                    // replay exists yet to provide the balancing delivery.
                    if delivery.literal_catcode() == Some(Catcode::BeginGroup) {
                        self.undo_delimiter_begin_group_delivery();
                    }
                    self.command
                        .scratch
                        .truncate_delimiter_suffix(&mut tokens)
                        .map_err(|_| CommandError::input_invariant())?;
                    tokens = self.strip_argument_outer_group(tokens)?;
                    return Ok(tokens);
                }
                continue;
            }

            if prefix_len != 0 {
                let retained =
                    self.delimiter_overlap(plan, delimiter, &failure, prefix_len, spelling)?;
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
                            .scratch
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
                    .scratch
                    .reveal_delimiter_words(&mut tokens, committed as u32)
                    .map_err(|_| CommandError::input_invariant())?;
                if retained != 0 {
                    // The current spelling is the final word of the retained
                    // suffix. It is unpublished until the next mismatch or
                    // until the complete delimiter truncates the holdback.
                    self.command
                        .scratch
                        .append_delimiter_word(&mut tokens, delivery.spelling())
                        .map_err(|_| CommandError::input_invariant())?;
                    continue;
                }
            }

            // The failed delimiter prefix is revealed before §395 examines
            // the current token. A held opening brace can consequently make a
            // closing brace ordinary argument material.
            if tokens.brace_depth() == 0 && delivery.literal_catcode() == Some(Catcode::EndGroup) {
                return self.recover_extra_right_brace_argument(delivery);
            }
            self.check_argument_paragraph(&delivery, flags, Some(&tokens))?;
            self.command
                .scratch
                .append_match_delivery(&mut tokens, &delivery, true)
                .map_err(|_| CommandError::input_invariant())?;
        }
    }

    /// Builds KMP failure links over one immutable parameter-text delimiter.
    /// The packed parameter reader is the sole pattern source; no semantic
    /// token or delimiter copy is created.
    fn build_delimiter_failure(
        &self,
        plan: &MacroPlan<G>,
        delimiter: MacroDelimiter,
        failure: &mut SmallVec<[u32; 32]>,
    ) -> Result<(), CommandError> {
        failure.clear();
        failure
            .try_reserve(delimiter.len)
            .map_err(|_| CommandError::input_invariant())?;
        if delimiter.len == 0 {
            return Ok(());
        }
        failure.push(0);
        for index in 1..delimiter.len {
            let mut matched = failure[index - 1] as usize;
            let current = plan.delimiter_word(delimiter, index)?;
            while matched != 0 && current != plan.delimiter_word(delimiter, matched)? {
                matched = failure[matched - 1] as usize;
            }
            if current == plan.delimiter_word(delimiter, matched)? {
                matched += 1;
            }
            failure.push(u32::try_from(matched).map_err(|_| CommandError::input_invariant())?);
        }
        Ok(())
    }

    /// Returns the next KMP state for a token which did not extend the held
    /// delimiter prefix. Failure links make the fallback linear in the input.
    fn delimiter_overlap(
        &self,
        plan: &MacroPlan<G>,
        delimiter: MacroDelimiter,
        failure: &[u32],
        prefix_len: usize,
        current: TokenWord,
    ) -> Result<usize, CommandError> {
        let mut matched = prefix_len;
        while matched != 0 && current != plan.delimiter_word(delimiter, matched)? {
            matched = failure
                .get(matched - 1)
                .copied()
                .map(|value| value as usize)
                .ok_or_else(CommandError::input_invariant)?;
        }
        if current == plan.delimiter_word(delimiter, matched)? {
            matched += 1;
        }
        Ok(matched.min(delimiter.len - 1))
    }

    fn check_argument_paragraph(
        &mut self,
        delivery: &MacroMatchDelivery<G>,
        flags: MeaningFlags,
        partial: Option<&MacroArgumentWriter<G>>,
    ) -> Result<(), CommandError> {
        if self.eof_recovered_while_matching && delivery.effective_paragraph() {
            // TeX82 §23 calls `check_outer_validity` after source EOF and
            // changes `long_state` to `outer_call`, even for a `\long` macro.
            // Its inserted frozen `\par` terminates the match but is consumed
            // by the failed expansion instead of being replayed by §396.
            let partial = partial
                .map(|buffer| {
                    self.command
                        .scratch
                        .match_words(buffer)
                        .map(|words| words.collect::<Vec<_>>())
                        .map_err(|_| CommandError::input_invariant())
                })
                .transpose()?
                .unwrap_or_default();
            self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
            return Err(CommandError::ParagraphInMacroArgument);
        }
        if delivery.paragraph_spelling() && !flags.contains(MeaningFlags::LONG) {
            // TeX82 §394 reports this through `back_error` while the macro
            // matcher is still live.  The caller will then restore its
            // enclosing scanner status, so retain the exact `\par` input
            // ahead of that restoration rather than merely returning an
            // error from the scalar matcher.
            self.back_input_hot((*delivery).into_hot())?;
            // §396 ends with `back_error`, so §82 renders the context with the
            // replayed `\par` already on the stack.
            let partial = partial
                .map(|buffer| {
                    self.command
                        .scratch
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
        mut buffer: MacroArgumentWriter<G>,
    ) -> Result<MacroArgumentWriter<G>, CommandError> {
        if buffer.facts().removable_outer_group() {
            buffer
                .strip_outer_group()
                .map_err(|_| CommandError::input_invariant())?;
        }
        Ok(buffer)
    }

    fn argument_token_count(&self, arguments: ArgumentSetId<G>) -> usize {
        (1..=9)
            .filter_map(|slot| {
                self.command
                    .scratch
                    .argument_range(arguments, slot)
                    .ok()
                    .flatten()
            })
            .map(|range| range.len() as usize)
            .sum()
    }
}

fn is_begin_group(token: TokenWord) -> bool {
    matches!(token.literal_catcode(), Some(Catcode::BeginGroup))
}

#[cfg(test)]
mod tests;
