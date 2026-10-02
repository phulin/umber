//! Native scan meaning recovery in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_unclassified_meaning(
        &mut self,
        command: tex_command::CurrentCommand<G>,
        meaning: Meaning,
    ) -> Result<ReplayStep, ExecError> {
        let mode = self.control.execution.modes.current_mode();
        let boxes = &self.control.execution.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

        let mut host_facts = ExecutorHostFacts {
            modes: &self.control.execution.modes,
            pdf_ignore_depth: self.control.execution.pdf_ignore_depth,
            telemetry: &mut self.control.execution.episode_telemetry,
        };
        let mut owned_processor = if let Some(provider) = self.provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
                &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        };
        owned_processor.set_output_routine_active(boxes.output_routine_active);
        if let Some(cursor) = self.cursor {
            owned_processor.resume_delivery_cursor(cursor);
        }
        let processor = &mut owned_processor;
        // Finish this actual processor/context loan before semantic application.
        macro_rules! finish_scan {
            ($call:expr) => {
                direct_finish_scan!(self, owned_processor, admitted_context, host_facts, $call)
            };
        }

        let scan_origin = self.current_origin;

        match meaning {
            // TeX82 §1045's `any_mode(relax): do_nothing`. `\relax` -- and the
            // frozen relax `\noexpand` substitutes for its operand (§358) -- is
            // the one meaning for which "consume nothing and proceed" is the
            // whole specified behavior.
            Meaning::Relax => {
                direct_apply_relax!(self, owned_processor, admitted_context)
            }
            // TeX82 §1048's Forbidden case `any_mode(last_item)`:
            // `report_illegal_case`. `Meaning::InternalInteger` is tex.web's
            // `last_item` command code with an operand other than
            // `\lastpenalty`/`\lastkern`/`\lastskip` (`\badness`,
            // `\inputlineno`, e-TeX's `\currentgrouplevel` family, pdfTeX's
            // `\pdflastxpos` family, ...). Like those three -- which
            // `scan_unclassified_primitive` already routes to the same
            // the direct command arm -- these are legal only as an internal-value operand
            // inside a scan, never as a delivered main-control command.
            Meaning::InternalInteger(_) => {
                let (direct_arg0, direct_arg1, direct_arg2) = (
                    command.spelling().semantic_token(),
                    processor.error_context(),
                    processor.current_diagnostic_site(Some(&command)),
                );
                finish_scan!(self.execute_illegal_last_item(direct_arg0, direct_arg1, direct_arg2))
            }
            Meaning::CharToken { ch, cat } => {
                let (direct_arg0, direct_arg1, direct_arg2) = (command, ch, cat);
                finish_scan!(self.scan_unclassified_char_token(
                    direct_arg0,
                    direct_arg1,
                    direct_arg2
                ))
            }
            // `scan_command`'s outer match ends with an unconditional
            // `Meaning::UnexpandablePrimitive(primitive)` arm delegating to
            // `scan_unclassified_primitive`, so this payload never reaches here.
            Meaning::UnexpandablePrimitive(_) => {
                unreachable!(
                    "unexpandable primitives are classified by scan_unclassified_primitive"
                )
            }
            // TeX82 §1210's `register`, `assign_int`/`assign_dimen`/
            // `assign_glue`/`assign_mu_glue`, `toks_register`/`assign_toks`, and
            // `set_font` assignment forms: `scan_command`'s outer match names
            // every one of them unconditionally.
            Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)
            | Meaning::Font(_) => {
                unreachable!("scan_command names this assignment meaning unconditionally")
            }
            // TeX82 gives a `\chardef`'d character exactly the same three-mode
            // behavior as `\char`: §1090's `vmode+char_given` starts a
            // paragraph, §1034's `main_loop` typesets it in horizontal mode, and
            // §1154's `mmode+char_given: set_math_char(ho(math_code(cur_chr)))`
            // appends a math-char noad. tex.web keeps the two interchangeable
            // right down to §1038's ligature lookahead, which accepts
            // `char_given` and `char_num` at the same label, so this reuses
            // `\char`'s own already-dispatched the direct command arm; the only
            // difference is that the character code is already known and needs
            // no `scan_char_num`.
            Meaning::CharGiven(ch) => {
                let (direct_arg0, direct_arg1, direct_arg2) = (
                    ch as i32,
                    processor
                        .active_macro_origin()
                        .unwrap_or_else(|| command.origin()),
                    false,
                );
                finish_scan!(self.execute_character_code(direct_arg0, direct_arg1, direct_arg2))
            }
            // TeX82 §1046's `non_math(math_given): insert_dollar_sign`, the same
            // recovery the whole math-only vocabulary takes outside math mode.
            // Reaching this arm proves `mode` is not `Math`/`DisplayMath`:
            // §1154's `mmode+math_given` is dispatched by `scan_command`'s
            // `scan_math_request` gate, which consumes the meaning
            // before its outer match runs.
            Meaning::MathCharGiven(_) => {
                (processor
                    .recover_missing_math_shift(command)
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                finish_scan!(self.execute_missing_math_shift())
            }
            // TeX82 §§366/370/380 put `undefined_cs` above `max_command`, so the
            // command-owned expanded-delivery loop reports and drops it before
            // main control can receive a command. Reaching stomach dispatch would
            // prove that a fetch policy has recreated a second recovery owner.
            Meaning::Undefined => unreachable!("undefined_cs escaped expanded delivery"),
            // A macro is expanded by `get_x_token` (§380) and `\noexpand` turns
            // one into a frozen relax (§358), so neither should ever be
            // delivered as an unexpandable command. `\endcsname` is the one
            // deliberately unexpandable `ExpandablePrimitive`; TeX82 §1135's
            // `cs_error` gives it "Extra \endcsname", which is not routed here.
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndCsName) => {
                finish_scan!(self.execute_extra_end_cs_name())
            }
            Meaning::ExpandablePrimitive(_) => {
                Err(super::unimplemented_meaning(&command, meaning, mode))
            }
            // TeX82 §1130's `vmode+endv,hmode+endv: do_endv` (§1131) and §1046's
            // `mmode+endv: insert_dollar_sign`. `scan_alignment_delivery_step`
            // implements the in-alignment half of §1131 before it ever calls
            // `scan_command`; an `endv` reaching main control by any other route
            // ("a devious user might force an `endv` command to occur just about
            // anywhere", §1131) has no dispatch.
            Meaning::EndV => {
                if matches!(mode, Mode::Math | Mode::DisplayMath) {
                    (processor
                        .recover_missing_math_shift(command)
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    finish_scan!(self.execute_missing_math_shift())
                } else {
                    {
                        let (direct_arg0,) = (command,);
                        finish_scan!(self.scan_off_save(direct_arg0))
                    }
                }
            }
            // An opcode `tex-state`'s meaning decoder itself does not recognize.
            Meaning::Unknown(_) => Err(super::unimplemented_meaning(&command, meaning, mode)),
        }
    }
}
