//! Native scan leaders in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_leaders_step(
        &mut self,
        primitive: UnexpandablePrimitive,
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
                direct_finish_scan!(self, owned_processor, admitted_context, $call)
            };
        }

        let scan_origin = self.current_origin;
        let kind = crate::box_runtime::leader_glue_kind(primitive);
        let payload = (processor.scan_leader_payload().map_err(command_error))
            .map_err(|error| scan_error(error, scan_origin))?;
        let result = match payload {
            ScannedLeaderPayload::Missing => {
                return finish_scan!(self.execute_missing_leader_payload());
            }
            ScannedLeaderPayload::Construction(construction) => {
                return finish_scan!(self.execute_begin_leader_box(construction, kind));
            }
            ScannedLeaderPayload::Rule(rule) => LeaderGlueResult::Payload {
                kind,
                payload: LeaderPayload::Rule {
                    width: rule.width,
                    height: rule.height,
                    depth: rule.depth,
                },
            },
            ScannedLeaderPayload::BoxRegister { index, copy } => {
                LeaderGlueResult::Register { kind, index, copy }
            }
        };
        let mut destination = None;
        if (next_non_blank_non_relax_x_token_into(processor, &mut destination)
            .map_err(command_error))
        .map_err(|error| scan_error(error, scan_origin))?
            != tex_command::DeliveryStatus::Command
        {
            return Err(scan_error(
                ExecError::MissingToken {
                    context: "leader glue",
                },
                scan_origin,
            ));
        }
        let glue = (scan_leader_glue_operand(processor, &mut destination, mode))
            .map_err(|error| scan_error(error, scan_origin))?;
        finish_scan!(self.complete_leader_glue(result, glue))
    }

    pub(super) fn scan_leader_glue_command(
        &mut self,
        mut command: Option<tex_command::CurrentCommand<G>>,
        result: LeaderGlueResult,
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
                direct_finish_scan!(self, owned_processor, admitted_context, $call)
            };
        }

        let scan_origin = self.current_origin;
        let glue = (scan_leader_glue_operand(processor, &mut command, mode))
            .map_err(|error| scan_error(error, scan_origin))?;
        finish_scan!(self.complete_leader_glue(result, glue))
    }

    pub(super) fn complete_leader_glue(
        &mut self,
        result: LeaderGlueResult,
        glue: Option<(GlueSpec, bool)>,
    ) -> Result<ReplayStep, ExecError> {
        let Some((glue, shared_zero_glue)) = glue else {
            return self.execute_leaders_not_followed_by_glue();
        };
        match result {
            LeaderGlueResult::Payload { kind, payload } => {
                self.execute_leaders(kind, payload, glue, shared_zero_glue)
            }
            LeaderGlueResult::Register { kind, index, copy } => {
                self.execute_leader_register(kind, index, copy, glue, shared_zero_glue)
            }
        }
    }
}

pub(super) fn scan_leader_glue_operand<G, Facts: tex_command::CommandHostFacts<G>>(
    processor: &mut CommandProcessor<'_, '_, G, Facts>,
    command: &mut Option<tex_command::CurrentCommand<G>>,
    mode: Mode,
) -> Result<Option<(GlueSpec, bool)>, ExecError> {
    let horizontal = matches!(
        mode,
        Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath
    );
    let primitive = match command
        .as_ref()
        .expect("leader glue scanner owns its current command")
        .meaning()
    {
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) => primitive,
        _ => {
            processor
                .back_input(
                    command
                        .take()
                        .expect("leader glue scanner owns its current command"),
                )
                .map_err(command_error)?;
            return Ok(None);
        }
    };
    if (horizontal && primitive == UnexpandablePrimitive::HSkip)
        || (!horizontal && primitive == UnexpandablePrimitive::VSkip)
    {
        let glue = match processor.scan_glue_retained(false) {
            tex_command::RetainedScalarScan::Complete(value) => value,
            tex_command::RetainedScalarScan::Failed(error) => return Err(command_error(error)),
        };
        return Ok(Some((glue, processor.scanned_glue_shared_zero())));
    }
    let infinite = match (horizontal, primitive) {
        (true, UnexpandablePrimitive::HFil) | (false, UnexpandablePrimitive::VFil) => {
            Some((Order::Fil, false, false))
        }
        (true, UnexpandablePrimitive::HFill) | (false, UnexpandablePrimitive::VFill) => {
            Some((Order::Fill, false, false))
        }
        (true, UnexpandablePrimitive::HSs) | (false, UnexpandablePrimitive::VSs) => {
            Some((Order::Fil, false, true))
        }
        (true, UnexpandablePrimitive::HFilNeg) | (false, UnexpandablePrimitive::VFilNeg) => {
            Some((Order::Fil, true, false))
        }
        _ => None,
    };
    let Some((order, negative, shrink)) = infinite else {
        processor
            .back_input(
                command
                    .take()
                    .expect("leader glue scanner owns its current command"),
            )
            .map_err(command_error)?;
        return Ok(None);
    };
    let unit = Scaled::from_raw(if negative {
        -Scaled::UNITY
    } else {
        Scaled::UNITY
    });
    let zero = Scaled::from_raw(0);
    let glue = if shrink {
        GlueSpec {
            width: zero,
            stretch: zero,
            stretch_order: Order::Normal,
            shrink: unit,
            shrink_order: order,
        }
    } else {
        GlueSpec {
            width: zero,
            stretch: unit,
            stretch_order: order,
            shrink: zero,
            shrink_order: Order::Normal,
        }
    };
    Ok(Some((glue, false)))
}
