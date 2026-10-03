//! Macro invocation tracing and argument recovery diagnostics.

use super::*;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// TeX82 §389's invocation trace, including `print_ln` before the macro
    /// name and §262's control-word separator before `->`.
    pub(super) fn trace_macro_invocation(
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
    #[cold]
    #[inline(never)]
    pub(super) fn trace_macro_argument(
        &mut self,
        body: &ResidentMacroBody<G>,
        parameter: usize,
        argument: &ArgumentCursor,
    ) -> Result<(), CommandError> {
        let marker = body
            .parameter_pattern()
            .marker_index(parameter)
            .map_or(Ok('#'), |index| {
                match body
                    .parameter_word(index)
                    .ok_or_else(CommandError::input_invariant)?
                    .semantic_token()
                {
                    Token::Char { ch, .. } => Ok(ch),
                    _ => Err(CommandError::input_invariant()),
                }
            })?;
        let mut text = format!("{marker}{}<-", parameter + 1);
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
        match self.command.scanner {
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
    pub(super) fn report_paragraph_ended_before_complete(&mut self, partial: &[TracedTokenWord]) {
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
    pub(super) fn report_extra_right_brace_argument(&mut self) {
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
}
