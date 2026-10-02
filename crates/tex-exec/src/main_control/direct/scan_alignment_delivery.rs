//! Native scan alignment delivery in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_noalign_body(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<ReplayStep, ExecError> {
        let mode = self.control.execution.modes.current_mode();
        let boxes = &self.control.execution.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");
        let innermost_group = admitted_context.innermost_group_kind();

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

        let shown_mode = &mut self.control.execution.shown_mode;

        let scan_origin = self.current_origin;
        prepare_command_trace(processor, mode, *shown_mode);
        let mut destination = None;
        if (processor
            .get_x_token_into(&mut destination)
            .map_err(command_error))
        .map_err(|error| scan_error(error, scan_origin))?
            != tex_command::DeliveryStatus::Command
        {
            return finish_scan!(self.execute_end_of_input());
        }
        let command = destination
            .take()
            .expect("command status initializes destination");
        report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
        match command.meaning() {
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }) if innermost_group == Some(GroupKind::NoAlign) => {
                if partoken_context_replays(processor, mode, 2) {
                    (processor
                        .insert_partoken_before(command)
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    return finish_scan!(self.execute_continue());
                }
                {
                    let (direct_arg0,) = (alignment,);
                    finish_scan!(self.execute_no_align_end_group(direct_arg0))
                }
            }
            // A `\noalign` body is ordinary main control between its braces
            // (TeX82 §785's `no_align_group`), so it dispatches through the same
            // §1030 `reswitch:`/§1211 prefix path as any other step.
            _ => {
                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                    (command, None, true, None);
                finish_scan!(self.run_command(
                    Some(direct_arg0),
                    direct_arg1,
                    direct_arg2,
                    direct_arg3
                ))
            }
        }
    }

    pub(super) fn scan_alignment_delivery_step(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<ReplayStep, ExecError> {
        let mode = self.control.execution.modes.current_mode();
        let boxes = &self.control.execution.boxes;

        let main_loop_active = self.control.execution.main_loop_active;
        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");
        let innermost_group = admitted_context.innermost_group_kind();

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

        let shown_mode = &mut self.control.execution.shown_mode;

        let scan_origin = self.current_origin;
        prepare_command_trace(processor, mode, *shown_mode);
        let mut destination = None;
        let delivery = (processor
            .get_x_alignment_delivery_into(main_loop_active, &mut destination)
            .map_err(command_error))
        .map_err(|error| scan_error(error, scan_origin))?;
        match delivery {
            tex_command::DeliveryStatus::End => {
                finish_scan!(self.execute_end_of_input())
            }
            // An executor-owned replay episode (a math field/group/choice branch
            // or discretionary part) retired mid-cell. This must be reported
            // exactly like ordinary `scan_step`'s `ReplayCompleted` case, rather
            // than falling through to interpret whatever the cascade found next
            // as this cell's own content: that next token can belong to the
            // *enclosing* cell/field context, not the just-retired episode.
            tex_command::DeliveryStatus::ReplayCompleted(episode) => {
                let (direct_arg0,) = (episode,);
                finish_scan!(self.execute_replay_completed(direct_arg0))
            }
            tex_command::DeliveryStatus::Command => {
                let command = destination.expect("command status initializes destination");
                // TeX82 §§1034/1038 keeps an adjacent character fetched by
                // `main_loop_lookahead` inside `main_loop`, even when §789's
                // u-template/body handoff lies between the two characters. The
                // lookahead is a raw delivery owned by alignment control, but it
                // does not create a second §1030 `reswitch` trace boundary.
                let continues_main_loop = main_loop_active
                    && matches!(
                        command.meaning(),
                        ResolvedMeaning::Static(
                            Meaning::CharToken {
                                cat: Catcode::Letter | Catcode::Other,
                                ..
                            } | Meaning::CharGiven(_)
                                | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                        )
                    );
                if !continues_main_loop {
                    report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
                }
                // TeX82 §1132 dispatches every right brace seen with an active
                // `align_group` through the missing-\cr recovery, independent of
                // `align_state`. The command-owned fast path emits a structural
                // ClosingBrace event at the ordinary cell depth, but §1096's
                // `off_save` can insert the brace after `align_state` is already
                // negative. That brace must still back up behind frozen `\cr`;
                // treating it as an ordinary extra brace makes `\par` repeat
                // `off_save` forever while recovery input levels accumulate.
                if innermost_group == Some(GroupKind::Align)
                    && matches!(
                        command.meaning(),
                        ResolvedMeaning::Static(Meaning::CharToken {
                            cat: Catcode::EndGroup,
                            ..
                        })
                    )
                {
                    (processor
                        .recover_alignment_closing_brace(
                            tex_command::AlignmentDeliveryEvent::ClosingBrace(command),
                        )
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    return finish_scan!(self.execute_missing_alignment_cr());
                }
                if matches!(command.meaning(), ResolvedMeaning::Static(Meaning::EndV)) {
                    // TeX82 §§1046-1047 route `mmode+endv` through
                    // `insert_dollar_sign`, just like every other command that
                    // reaches an alignment v-template before its math mode has
                    // closed. The synthesized `$` closes math first; the backed
                    // up `endv` is then redelivered in the cell's h/v mode and
                    // reaches §1131 below.
                    if matches!(mode, Mode::Math | Mode::DisplayMath) {
                        (processor
                            .recover_missing_math_shift(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_missing_math_shift());
                    }
                    if partoken_context_replays(processor, mode, 2) {
                        (processor
                            .insert_partoken_before(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_continue());
                    }
                    // TeX82 §1131 accepts end-v only when `cur_group=align_group`.
                    // The replay driver tracks align-error's inserted `{`
                    // separately because its structural alignment boundary is
                    // executor-owned. Ordinary `\begingroup` is nevertheless a
                    // real `semi_simple_group` save-stack level, and must close
                    // through §§1064--1065 `off_save` before the same end-v is
                    // replayed. Other intervening groups are intercepted by their
                    // owning mode/box delivery paths before reaching this cell
                    // finish boundary.
                    if boxes.recovery_simple_group_open
                        || innermost_group == Some(GroupKind::SemiSimple)
                    {
                        return {
                            let (direct_arg0,) = (command,);
                            finish_scan!(self.scan_off_save(direct_arg0))
                        };
                    }
                    return {
                        let (direct_arg0,) = (alignment,);
                        finish_scan!(self.execute_alignment_cell_finish(direct_arg0))
                    };
                }
                // An alignment cell's body is ordinary main control bounded by
                // §1130's `vmode+endv,hmode+endv: do_endv`, not a dispatcher of
                // its own, so it takes
                // the same §1030 `reswitch:`/§1211 prefix path as any other step.

                {
                    let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                        (command, Some(alignment), true, None);
                    finish_scan!(self.run_command(
                        Some(direct_arg0),
                        direct_arg1,
                        direct_arg2,
                        direct_arg3
                    ))
                }
            }
            tex_command::DeliveryStatus::AlignmentEndTemplate => {
                let event = tex_command::AlignmentDeliveryEvent::EndTemplate(
                    destination.expect("alignment status initializes destination"),
                );
                {
                    let (direct_arg0, direct_arg1) = (alignment, event);
                    finish_scan!(self.scan_alignment_delivery_event(direct_arg0, direct_arg1))
                }
            }
            tex_command::DeliveryStatus::AlignmentClosingBrace => {
                let event = tex_command::AlignmentDeliveryEvent::ClosingBrace(
                    destination.expect("alignment status initializes destination"),
                );
                {
                    let (direct_arg0, direct_arg1) = (alignment, event);
                    finish_scan!(self.scan_alignment_delivery_event(direct_arg0, direct_arg1))
                }
            }
            tex_command::DeliveryStatus::PendingExpanded => {
                unreachable!("alignment delivery commits terminal observations")
            }
            tex_command::DeliveryStatus::CharacterRun
            | tex_command::DeliveryStatus::CharacterRunBoundary => {
                unreachable!("alignment delivery does not admit main-loop character runs")
            }
        }
    }
}
