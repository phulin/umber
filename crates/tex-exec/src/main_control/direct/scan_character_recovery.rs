//! Native scan character recovery in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_unclassified_char_token(
        &mut self,
        command: tex_command::CurrentCommand<G>,
        ch: char,
        cat: Catcode,
    ) -> Result<ReplayStep, ExecError> {
        let mode = self.control.modes.current_mode();
        let boxes = &self.control.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

        let mut host_facts = ExecutorHostFacts {
            modes: &self.control.modes,
            pdf_ignore_depth: self.control.pdf_ignore_depth,
            telemetry: &mut self.control.episode_telemetry,
        };
        let mut owned_processor = if let Some(provider) = self.provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                provider,
                &mut self.control.declined_resource_attempt,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        } else {
            command_processor(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        };
        owned_processor.set_output_routine_active(boxes.output_routine_active);
        if let Some(cursor) = self.cursor {
            owned_processor.resume_delivery_cursor(cursor);
        }
        let processor: &mut CommandProcessor<'_, '_, G> = &mut owned_processor;
        // Finish this actual processor/context loan before semantic application.
        macro_rules! finish_scan {
            ($call:expr) => {
                direct_finish_scan!(self, owned_processor, admitted_context, host_facts, $call)
            };
        }

        let scan_origin = self.current_origin;

        match cat {
            // TeX82 §1046's `non_math(sup_mark)`/`non_math(sub_mark)`:
            // §1047's `insert_dollar_sign` backs the command up behind a
            // synthesized `$`. Reaching this arm proves `mode` is not
            // `Math`/`DisplayMath`, since `scan_command`'s superscript/subscript
            // gates consume both categories before its outer match in those two
            // modes.
            Catcode::Superscript | Catcode::Subscript => {
                (processor
                    .recover_missing_math_shift(command)
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                finish_scan!(self.execute_missing_math_shift())
            }
            // TeX82 §1045's `any_mode(mac_param): report_illegal_case`. A bare
            // parameter token has no operand of its own; the command is consumed
            // after the diagnostic and main control continues.
            Catcode::Parameter => {
                let (direct_arg0,) = (command.spelling().semantic_token(),);
                finish_scan!(self.execute_illegal_macro_parameter(direct_arg0))
            }
            // TeX82 §1126's `any_mode(tab_mark)` (a category-4 character token)
            // and `any_mode(car_ret)` (a category-5 one, which `get_next`'s
            // §344 end-of-line handling normally consumes before delivery).
            // Both command codes take §1127's `align_error`.
            Catcode::AlignmentTab | Catcode::EndLine => {
                let (direct_arg0,) = (command,);
                finish_scan!(self.scan_align_error(direct_arg0))
            }
            // Category codes that never become a delivered command: `get_next`
            // (§341-§356) consumes escape characters into control-sequence
            // spellings, resolves active characters to their own meanings, drops
            // ignored and comment characters, and reports invalid characters at
            // the lexer boundary.
            Catcode::Escape
            | Catcode::Active
            | Catcode::Ignored
            | Catcode::Comment
            | Catcode::Invalid => Err(super::unimplemented_meaning(
                &command,
                Meaning::CharToken { ch, cat },
                mode,
            )),
            // `scan_command`'s outer match names all five of these
            // unconditionally.
            Catcode::BeginGroup
            | Catcode::EndGroup
            | Catcode::MathShift
            | Catcode::Space
            | Catcode::Letter
            | Catcode::Other => {
                unreachable!("scan_command names this character category unconditionally")
            }
        }
    }

    pub(super) fn scan_align_error(
        &mut self,
        command: tex_command::CurrentCommand<G>,
    ) -> Result<ReplayStep, ExecError> {
        let boxes = &self.control.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

        let mut host_facts = ExecutorHostFacts {
            modes: &self.control.modes,
            pdf_ignore_depth: self.control.pdf_ignore_depth,
            telemetry: &mut self.control.episode_telemetry,
        };
        let mut owned_processor = if let Some(provider) = self.provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                provider,
                &mut self.control.declined_resource_attempt,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        } else {
            command_processor(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        };
        owned_processor.set_output_routine_active(boxes.output_routine_active);
        if let Some(cursor) = self.cursor {
            owned_processor.resume_delivery_cursor(cursor);
        }
        let processor: &mut CommandProcessor<'_, '_, G> = &mut owned_processor;
        // Finish this actual processor/context loan before semantic application.
        macro_rules! finish_scan {
            ($call:expr) => {
                direct_finish_scan!(self, owned_processor, admitted_context, host_facts, $call)
            };
        }

        let scan_origin = self.current_origin;

        let token = command.spelling().semantic_token();
        match (processor
            .recover_align_error(command)
            .map_err(command_error))
        .map_err(|error| scan_error(error, scan_origin))?
        {
            // TeX82 §1128 calls §82's `error` synchronously, while the delimiter's
            // input level is still current. Split scan/apply must therefore carry
            // that exact context rather than reconstructing it after a retained
            // alignment v-template has advanced or retired.
            None => {
                let (direct_arg0, direct_arg1) = (token, processor.diagnostic_context_coordinate());
                finish_scan!(self.execute_misplaced_alignment_delimiter(direct_arg0, direct_arg1))
            }
            Some(tex_state::token::Token::Char {
                cat: brace @ (Catcode::BeginGroup | Catcode::EndGroup),
                ..
            }) => {
                let (direct_arg0,) = (brace,);
                finish_scan!(self.execute_alignment_recovery(direct_arg0))
            }
            Some(_) => Err(ExecError::MissingToken {
                context: "align_error balancing brace",
            }),
        }
    }
}
