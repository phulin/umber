//! Private canonical scalar macro-call state machine.
use tex_state::env::banks::IntParam;
use tex_state::macro_definition::MacroDelimiter;
use tex_state::meaning::MeaningFlags;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};
use tex_state::{DefinitionRef, ResidentMacroBody};

use crate::command::CurrentCommand;
use crate::execution_scratch::ArgumentRunAdmission;
use crate::input::{ArgumentCursor, MatchedArguments};
use crate::processor::status::{MatchingContext, ScannerStatus};
use crate::{CommandError, CommandProcessor};

use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, MacroRecord, TokenListRecord,
};

const EXTRA_RIGHT_BRACE_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0395;
pub(crate) const RUNAWAY_ARGUMENT_DIAGNOSTIC: u64 = 0x6d61_6372_0000_0396;

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

#[cfg(test)]
fn record_macro_activation(parameterless: bool, exceptional: bool) {
    MACRO_ACTIVATION_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        if exceptional {
            counters.exceptional = counters.exceptional.saturating_add(1);
        } else if parameterless {
            counters.simple = counters.simple.saturating_add(1);
        } else {
            counters.matching = counters.matching.saturating_add(1);
        }
        slot.set(counters);
    });
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
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

    #[inline(always)]
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
        let (body, elide_empty_replacement) = match admitted {
            tex_state::AdmittedMacroDefinition::SimpleMacro { body } => {
                let exceptional = self.macro_activation_needs_context(body.is_none());
                #[cfg(test)]
                record_macro_activation(true, exceptional);
                if !exceptional {
                    return self.activate_simple_macro(macro_name, call_site, body);
                }
                let body = match body {
                    Some(body) => body,
                    None => self
                        .state
                        .admit_macro_body(definition)
                        .ok_or_else(CommandError::input_invariant)?,
                };
                (body, false)
            }
            tex_state::AdmittedMacroDefinition::MatchingMacro { body } => {
                let elide_empty_replacement =
                    body.is_empty() && !self.macro_activation_needs_context(true);
                #[cfg(test)]
                record_macro_activation(
                    false,
                    self.macro_activation_needs_context(body.is_empty()),
                );
                (body, elide_empty_replacement)
            }
        };
        self.match_macro_definition(
            flags,
            definition,
            macro_name,
            call_site,
            elide_empty_replacement,
            body,
            report_prefix_mismatch,
        )
    }

    // Only a real matcher installs scanner status. It saves the actual prior
    // status directly, without an installed-status episode packet.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)] // actual admitted definition and invocation facts
    fn match_macro_definition(
        &mut self,
        flags: MeaningFlags,
        definition: tex_state::DefinitionRef<G>,
        macro_name: tex_state::interner::Symbol,
        call_site: OriginId,
        elide_empty_replacement: bool,
        body: tex_state::ResidentMacroBody<G>,
        report_prefix_mismatch: impl FnOnce(&mut Self),
    ) -> Result<bool, CommandError> {
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
        let prior_status = if needs_matching {
            let status = ScannerStatus::Matching(MatchingContext { macro_name });
            let prior = self.command.begin_scanner_status(status);
            if self.is_observed() {
                self.observe_scanner_status_transition(prior, status);
            }
            Some(prior)
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
            if elide_empty_replacement {
                self.conserve_input_stack()?;
                self.command
                    .record_empty_matching_macro_activation(parameter_count);
                return Ok(true);
            }
            self.conserve_input_stack_for_descendant()?;
            let _level =
                self.command
                    .push_macro_activation(macro_name, body, matching.take(), call_site);
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
                    token_count: match self.command.input.levels.current.as_ref() {
                        Some(
                            row @ crate::input::InputLevel {
                                storage:
                                    crate::input::InputStorage::List(_)
                                    | crate::input::InputStorage::StoredList(_)
                                    | crate::input::InputStorage::MacroBody(_)
                                    | crate::input::InputStorage::MacroArgument { .. },
                                ..
                            },
                        ) => match &row.storage {
                            crate::input::InputStorage::MacroBody(body) => body
                                .argument_values(&self.command.input.levels.arguments)
                                .iter()
                                .map(|span| u64::from(span.len()))
                                .sum(),
                            _ => unreachable!("activation owns a macro body"),
                        },
                        _ => unreachable!("activation owns a resident row"),
                    },
                }),
            );
            Ok(true)
        })();
        // Every Result path settles the one local scan and unfinished owner,
        // including failures conserving depleted parents after a complete match.
        if let Some(prior) = prior_status {
            if self.is_observed() {
                self.restore_scanner_status_with_observation(
                    ScannerStatus::Matching(MatchingContext { macro_name }),
                    prior,
                );
            } else {
                self.command.restore_scanner_status(prior);
            }
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

    #[inline(always)]
    fn macro_activation_needs_context(&self, replacement_is_empty: bool) -> bool {
        self.command.delivery_mode.requires_persistent_settlement()
            || self.state.int_param(IntParam::TRACING_MACROS) > 0
            || (replacement_is_empty && self.empty_macro_needs_completion_descendant())
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
                matches!(level, row @ crate::input::InputLevel { storage: crate::input::InputStorage::List(_) | crate::input::InputStorage::StoredList(_) | crate::input::InputStorage::MacroBody(_) | crate::input::InputStorage::MacroArgument { .. }, .. }
                    if !matches!(row.behavior(), crate::input::TokenBehavior::VTemplate)
                        && level.stored_is_exhausted() == Some(true))
            })
            .any(|level| level.identity() == owner)
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
                .push_macro_activation(macro_name, body, None, call_site);
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
        for index in 0..leading_end {
            let expected = body
                .parameter_word(index)
                .ok_or_else(CommandError::input_invariant)?;
            let matched = self.consume_raw_words::<true, false, _>(&mut |_, word, _, _| {
                Ok(std::ops::ControlFlow::Break(word.token_word() == expected))
            })?;
            if matched != Some(true) {
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
            let mut argument = self.command.input.levels.arguments.begin_argument(matching);
            if let Some(delimiter) = delimiter {
                self.scan_delimited_argument(&mut argument, flags, delimiter, paragraph_token)?;
            } else {
                self.scan_undelimited_argument(&mut argument, flags, paragraph_token)?;
            }
            self.command
                .input
                .levels
                .arguments
                .publish_argument(matching, argument)
                .map_err(|_| CommandError::input_invariant())?;
            if self.state.int_param(IntParam::TRACING_MACROS) > 0 {
                self.trace_macro_argument(body, parameter, matching)?;
            }
            if self.is_observed() {
                let argument_token_count = self
                    .command
                    .input
                    .levels
                    .arguments
                    .matched_words(matching, parameter as u8)
                    .iter()
                    .len();
                self.observe(CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "macro_delimiter_match",
                    tokens: delimiter
                        .map_or(&[][..], MacroDelimiter::words)
                        .iter()
                        .copied()
                        .map(|word| {
                            self.observed_token(TracedTokenWord::from_parts(
                                word,
                                OriginId::UNKNOWN,
                            ))
                        })
                        .collect(),
                }));
                self.observe(CommandObservation::Macro(MacroRecord::Argument {
                    control_sequence: self.state.resolve(macro_name).to_owned(),
                    parameter: (parameter + 1) as u8,
                    token_count: argument_token_count as u64,
                    tokens: self
                        .command
                        .input
                        .levels
                        .arguments
                        .matched_words(matching, parameter as u8)
                        .iter()
                        .map(|token| self.observed_token(token))
                        .collect(),
                }));
            }
        }
        Ok(())
    }
}

fn is_begin_group(token: TokenWord) -> bool {
    matches!(token.literal_catcode(), Some(Catcode::BeginGroup))
}

#[cfg(test)]
mod tests;

mod arguments;
mod diagnostics;
