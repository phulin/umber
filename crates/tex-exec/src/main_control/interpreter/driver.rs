//! Concrete driver observations and native high-water facts.
use super::*;

impl<G> MainControl<G> {
    pub(super) fn observe_control_projections(&mut self, stores: &mut Universe<G>) {
        let mode = self.execution.modes.current_mode();
        let fingerprint = self.execution.modes.semantic_fingerprint(stores);
        let mut context = stores
            .command_context()
            .expect("tracked control projection");
        let mut facts = ExecutorHostFacts {
            modes: &self.execution.modes,
            pdf_ignore_depth: self.execution.pdf_ignore_depth,
            telemetry: &mut self.execution.episode_telemetry,
        };
        let last = tex_command::CommandHostFacts::last_node_type(&mut facts, &context);
        context.observe_changed_command_projection(
            DependencyKey::Engine(DependencyEngineField::Mode),
            DependencyValue::Projection {
                schema: 1,
                fingerprint,
            },
        );
        context.observe_changed_command_projection(
            DependencyKey::Engine(DependencyEngineField::InnerMode),
            DependencyValue::Bool(mode.is_inner()),
        );
        let (level, kind) = context.current_group_values();
        for (key, value) in [
            (DependencyEngineField::GroupLevel, i64::from(level)),
            (DependencyEngineField::GroupType, i64::from(kind)),
            (DependencyEngineField::LastNodeType, i64::from(last)),
            (
                DependencyEngineField::PageInsertions,
                i64::try_from(context.page_insertions().len()).unwrap_or(i64::MAX),
            ),
        ] {
            context.observe_changed_command_projection(
                DependencyKey::Engine(key),
                DependencyValue::Integer(value),
            );
        }
    }
}
impl<G> MainControl<G> {
    pub(in crate::main_control) fn update_save_stack_highwater(
        &mut self,
        stores: &mut Universe<G>,
    ) {
        let words = stores
            .max_save_stack_words(self.command_profile().capabilities().supports_etex())
            .expect("save-stack highwater");
        self.execution.max_save_stack = self.execution.max_save_stack.max(words);
    }
}

impl<G> MainControl<G> {
    pub(in crate::main_control) fn resume_character_loop(
        &mut self,
        stores: &mut Universe<G>,
        character: Option<char>,
    ) {
        let context = stores.command_context().expect("character loop admission");
        self.execution.main_loop_active = character.is_some_and(|ch| {
            matches!(
                self.execution.modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) && u8::try_from(u32::from(ch)).ok().is_some_and(|code| {
                context
                    .font_char_metrics(context.current_font(), code)
                    .is_some()
            })
        });
    }
}

impl<G> MainControl<G> {
    pub(in crate::main_control) fn interpret_chunk(
        &mut self,
        stores: &mut Universe<G>,
        effects: &mut DiagnosticEffects,
        tracked: bool,
        provider: &mut Option<&mut dyn ResourceProvider<G>>,
        remaining_operations: usize,
        completed_operations: &mut usize,
    ) -> Result<MainControlStep, ExecError> {
        self.invoke_interpreter(stores, effects, tracked, provider, |processor, receipts| {
            run(
                processor,
                remaining_operations,
                completed_operations,
                receipts,
            )
        })
    }

    /// Diagnostic preflight has genuinely already consumed this spelling.
    /// Its retained meaning must survive any subsequent operand scan.
    pub(in crate::main_control) fn interpret_fetched(
        &mut self,
        stores: &mut Universe<G>,
        command: tex_command::CurrentCommand<G>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
        effects: &mut DiagnosticEffects,
        tracked: bool,
        provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<MainControlStep, ExecError> {
        self.invoke_interpreter(stores, effects, tracked, provider, |processor, receipts| {
            if let Some(cursor) = cursor {
                processor.resume_delivery_cursor(cursor);
            }
            trace::prepare(processor);
            dispatch::execute(
                processor,
                command.spelling(),
                command.native_meaning(),
                command.control_sequence(),
                Some(&command),
                receipts,
                0,
            )
        })
    }

    fn invoke_interpreter(
        &mut self,
        stores: &mut Universe<G>,
        effects: &mut DiagnosticEffects,
        tracked: bool,
        provider: &mut Option<&mut dyn ResourceProvider<G>>,
        interpret: impl FnOnce(
            &mut Processor<'_, '_, G>,
            &mut Option<Vec<MutationRecord>>,
        ) -> Result<MainControlStep, ExecError>,
    ) -> Result<MainControlStep, ExecError> {
        self.ensure_primitive_handles(stores);
        if tracked {
            self.observe_control_projections(stores);
        }
        let observing = self.operation_observations.is_some();
        let output_start = observing.then(|| OperationOutputStart {
            outer_paragraph_was_active: self.execution.modes.current_mode() == Mode::Horizontal
                && self.execution.modes.depth() == 2,
            artifact_count: stores.world().committed_artifacts().len(),
            effect_count: stores.world().effect_records().len(),
            prepared_page_count: self.execution.prepared_dvi_pages.len(),
            tracked_region_is_active: tracked,
        });
        let mut receipts = observing.then(Vec::new);
        let mut context = stores
            .command_context()
            .expect("interpreter generation admission");
        if self.enter_main_control(&mut context) {
            publish_named_token_list_pushes(
                &mut self.command,
                &mut context,
                effects,
                &mut self.operation_observations,
            );
        }
        let output_routine_active = self.execution.boxes.output_routine_active;
        let mut processor = if let Some(provider) = provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.execution,
                provider,
                &mut self.declined_resource_attempt,
                &mut self.operation_observations,
                effects,
                &mut context,
            )
        } else {
            command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.execution,
                &mut self.operation_observations,
                effects,
                &mut context,
            )
        };
        processor.set_output_routine_active(output_routine_active);
        let mut result = interpret(&mut processor, &mut receipts);
        drop(processor);
        drop(context);
        if let Some((dump, incomplete)) = self.execution.pending_job_end.take() {
            if self.execution.initex && dump {
                let mut context = stores.command_context().expect("format dump accounting");
                let receipt = crate::job::FormatDumpReceipt::new(
                    self.capabilities.job_name().to_owned(),
                    context.int_param(IntParam::YEAR),
                    context.int_param(IntParam::MONTH),
                    context.int_param(IntParam::DAY),
                );
                context.make_string_pool_string(&receipt.pool_string());
                self.execution.dumped_format = Some(receipt);
            }
            stores
                .world_mut()
                .publish_diagnostic_effects_preserving(effects);
            self.end_of_job_final_cleanup(stores, dump, incomplete);
            if let Some(observations) = self.operation_observations.as_mut() {
                observations.committed(CommandObservation::Effect(engine_termination_effect()));
            }
        }
        if result.is_ok()
            && let Some(form) = self.execution.prepared_pdf_form.take()
        {
            stores
                .world_mut()
                .publish_diagnostic_effects_preserving(effects);
            let demand = stores.provenance_demand();
            let budget = stores.provenance_budgets().detached_artifact_recipe_bytes;
            let staged = (|| {
                let resolver = {
                    let mut context = stores
                        .command_context()
                        .expect("immediate form source admission");
                    let page = context
                        .copy_pdf_form_to_page(form.object())
                        .ok_or(ExecError::PdfXFormVoidBox)?;
                    DetachedArtifactSourceResolver::capture_page_list(page, &context, demand)
                };
                let mut command =
                    self.command_machine_with_resource_provider(effects, provider.take());
                let modes = match &command.host_facts {
                    CommandMachineHostFacts::Live(facts) => facts.modes,
                    _ => unreachable!("actual form traverses the live mode owner"),
                };
                let mut geometry = DetachedShipoutGeometry::default();
                let staged = publish_immediate_pdf_form(
                    form,
                    &mut command,
                    modes,
                    stores,
                    &resolver,
                    demand,
                    budget,
                    &mut geometry,
                );
                *provider = command.resource_provider.take();
                if let Some(geometry) = geometry.0 {
                    crate::shipout::ShipoutGeometrySink::committed_shipout_geometry(
                        &mut command.shipout_geometry_sink(),
                        geometry,
                    );
                }
                staged
            })();
            if let Err(error) = staged {
                result = Err(error);
            }
        }
        if result.is_ok() {
            if let Some(shipout) = self.execution.prepared_shipout.take() {
                stores
                    .world_mut()
                    .publish_diagnostic_effects_preserving(effects);
                let staged = {
                    let mut command =
                        self.command_machine_with_resource_provider(effects, provider.take());
                    let modes = match &command.host_facts {
                        CommandMachineHostFacts::Live(facts) => facts.modes,
                        _ => unreachable!("actual shipout borrows the live mode owner"),
                    };
                    let staged = shipout_replay_box(shipout, stores, &mut command, modes);
                    *provider = command.resource_provider.take();
                    staged
                };
                stores.release_shipout_source();
                match staged {
                    Ok(publication) => {
                        if let Some(receipt) = publication.and_then(|publication| publication.dvi) {
                            push_prepared_dvi_page(&mut self.execution.prepared_dvi_pages, receipt);
                        }
                    }
                    Err(error) => result = Err(error),
                }
            }
            if output_routine_active && !self.execution.boxes.output_routine_active {
                self.execution.page_region_succession_pending = true;
                self.finish_pending_page_region_succession(stores);
            }
            if matches!(result, Ok(MainControlStep::EndOfInput))
                && self.execution.root_completion == RootCompletionPolicy::RequireTeXEnd
            {
                result = Ok(self.handle_root_end_of_input(stores));
            }
        }
        if result.is_ok() {
            if let Some(output_start) = output_start {
                self.publish_semantic_observations(stores, output_start, None, receipts.take());
            }
        }
        result
    }
}
