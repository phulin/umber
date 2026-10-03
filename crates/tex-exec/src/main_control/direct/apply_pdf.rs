//! Apply scanned pdf values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_pdf_font_code(
        &mut self,
        mut table: tex_state::PdfFontCode,
        mut font: FontId,
        mut character: u8,
        mut value: i32,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_font_code(
                    stores,
                    command,
                    &mut table,
                    &mut font,
                    &mut character,
                    &mut value,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_pdf_no_ligatures(
        &mut self,
        mut font: FontId,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_no_ligatures(stores, command, &mut font)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_pdf_x_image(
        &mut self,
        mut request: tex_command::PdfImageRequest<tex_command::AttemptTokenListId>,
        mut resource: PdfImageResource,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        self.control.resolve_pdf_image_request(
            &mut request,
            &mut resource,
            self.stores,
            self.provider,
        )?;
        let mut request = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            request,
        )?;
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, _, _, _, _, _, _, _, _| {
                semantic::execute_pdf_x_image(stores, &mut request, &mut resource)
            },
        )
    }

    pub(super) fn execute_pdf_ref_x_image(
        &mut self,
        mut object: i32,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_ref_x_image(stores, modes, command, &mut object)
            },
        )
    }

    pub(super) fn execute_pdf_set_random_seed(
        &mut self,
        seed: i32,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.stores.world_mut().set_pdf_random_seed(seed);
            Ok(ReplayStep::Continue)
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_pdf_reset_timer(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.stores.world_mut().reset_pdf_timer();
            Ok(ReplayStep::Continue)
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_pdf_interword_space(
        &mut self,
        mut operand0: tex_state::node::PdfAccessibilityControl,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_interword_space(stores, modes, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_running_link(
        &mut self,
        mut operand0: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_running_link(stores, modes, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_space_font(
        &mut self,
        operand0: tex_command::AttemptTokenListId,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut operand0 = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            operand0,
        )?;
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, _, _, _, _, _, _, _, _| {
                semantic::execute_pdf_space_font(stores, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_graphics(
        &mut self,
        operand0: tex_command::PdfGraphicsRequest<tex_command::AttemptTokenListId>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut operand0 = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            operand0,
        )?;
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_graphics(stores, modes, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_object(
        &mut self,
        operand0: tex_command::PdfObjectRequest<tex_command::AttemptTokenListId>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut operand0 = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            operand0,
        )?;
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, _, _, _, _, _, _, _, _| {
                semantic::execute_pdf_object(stores, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_reference_object(
        &mut self,
        mut operand0: PdfReferenceObjectRequest,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_reference_object(stores, modes, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_form(
        &mut self,
        operand0: tex_command::PdfFormRequest<tex_command::AttemptTokenListId>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut operand0 = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            operand0,
        )?;
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_form(stores, modes, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_document_fragment(
        &mut self,
        operand0: tex_command::PdfDocumentFragmentRequest<tex_command::AttemptTokenListId>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut operand0 = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            operand0,
        )?;
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, _, _, _, _, _, _, _, _| {
                semantic::execute_pdf_document_fragment(stores, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_navigation(
        &mut self,
        operand0: tex_command::PdfNavigationRequest<tex_command::AttemptTokenListId>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut operand0 = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            operand0,
        )?;
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_pdf_navigation(stores, modes, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_pdf_font_expand(
        &mut self,
        mut font: FontId,
        mut spec: tex_typeset::expansion::FontExpansionSpec,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, _, _, _, _, _, _, _, _| {
                semantic::execute_pdf_font_expand(stores, &mut font, &mut spec)
            },
        )
    }

    pub(super) fn execute_pdf_font_action(
        &mut self,
        mut primitive: UnexpandablePrimitive,
        mut font: Option<FontId>,
        first: Option<tex_command::AttemptTokenListId>,
        second: Option<tex_command::AttemptTokenListId>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let (mut first, mut second) = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            (first, second),
        )?;
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, _, _, _, _, _, _, _, _| {
                semantic::execute_pdf_font_action(
                    stores,
                    &mut primitive,
                    &mut font,
                    &mut first,
                    &mut second,
                )
            },
        )
    }

    pub(super) fn execute_immediate_extension(
        &mut self,
        operand0: tex_command::ImmediateExtension<tex_command::AttemptTokenListId>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut operand0 = publication::publish_tokens(
            &mut self.control.command,
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            operand0,
        )?;
        if let tex_command::ImmediateExtension::PdfForm(request) = &mut operand0 {
            return self.execute_immediate_pdf_form(request);
        }
        let context = self
            .stores
            .command_context()
            .expect("immediate write observation");
        let effect = if self.control.operation_observations.is_some() {
            match &operand0 {
                tex_command::ImmediateExtension::Write { stream, tokens } => Some(EffectRecord {
                    kind: ObservationEffectKind::Write,
                    channel: format!("stream:{}", stream.normalized_number()),
                    value: ObservationValue::Tokens(
                        context
                            .token_list(tokens.clone())
                            .iter()
                            .map(|word| observed_macro_token(word.semantic_token(), &context))
                            .collect(),
                    ),
                    source: None,
                }),
                _ => None,
            }
        } else {
            None
        };
        drop(context);
        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            effect,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_immediate_extension(stores, modes, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_immediate_pdf_form(
        &mut self,
        request: &mut tex_command::PdfFormRequest<tex_state::TokenListId<G>>,
    ) -> Result<ReplayStep, ExecError> {
        let demand = self.stores.provenance_demand();
        let budget = self
            .stores
            .provenance_budgets()
            .detached_artifact_recipe_bytes;
        self.control.execution.main_loop_active = false;
        let mut command = CommandMachine {
            state: &mut self.control.command,
            fuel: self.control.fuel.fuel_mut(),
            capabilities: &mut self.control.capabilities,
            resource_provider: self.provider.take(),
            declined_resource_attempt: Some(&mut self.control.declined_resource_attempt),
            host_facts: CommandMachineHostFacts::Detached {
                pdf_ignore_depth: self.control.execution.pdf_ignore_depth,
                telemetry: &mut self.control.execution.episode_telemetry,
            },
            observations: self
                .control
                .operation_observations
                .as_mut()
                .map(|buffer| buffer as &mut dyn CommandObserver),
            assignment_receipts: None,
            diagnostic_effects: self.effects,
            shown_mode: &mut self.control.execution.shown_mode,
            initex: self.control.execution.initex,
            emit_dvi_override: self.control.execution.emit_dvi_override,
            immediate_prints: &mut self.control.execution.immediate_prints,
            prepared_shipout: &mut self.control.execution.prepared_shipout,
            pending_show_completion: None,
            pending_outer_page_build_context: None,
            output_routine_active: self.control.execution.boxes.output_routine_active,
        };
        let result = (|| -> Result<ReplayStep, ExecError> {
            let (form, resolver) = {
                let mut context = self
                    .stores
                    .command_context()
                    .expect("immediate form admission");
                let form = apply_pdf_form_request(
                    request,
                    &mut context,
                    &mut self.control.execution.modes,
                    &mut command,
                    true,
                )?
                .expect("immediate form creates a publication record");
                let page = context
                    .copy_pdf_form_to_page(form.object())
                    .ok_or(ExecError::PdfXFormVoidBox)?;
                let resolver =
                    DetachedArtifactSourceResolver::capture_page_list(page, &context, demand);
                (form, resolver)
            };
            let mut geometry = DetachedShipoutGeometry::default();
            publish_immediate_pdf_form(
                form,
                &mut command,
                &self.control.execution.modes,
                self.stores,
                &resolver,
                demand,
                budget,
                &mut geometry,
            )?;
            if let Some(geometry) = geometry.0 {
                crate::shipout::ShipoutGeometrySink::committed_shipout_geometry(
                    &mut command.shipout_geometry_sink(),
                    geometry,
                );
            }
            Ok(ReplayStep::Continue)
        })();
        *self.provider = command.resource_provider.take();
        drop(command);
        self.finish_host(result)
    }
}
