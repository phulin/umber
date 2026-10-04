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
    ) -> Result<ReplayStep, ExecError> {
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
    ) -> Result<ReplayStep, ExecError> {
        self.invoke_interpreter(stores, effects, tracked, provider, |processor, receipts| {
            if let Some(cursor) = cursor {
                processor.resume_delivery_cursor(cursor);
            }
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
        ) -> Result<ReplayStep, ExecError>,
    ) -> Result<ReplayStep, ExecError> {
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
        let result = interpret(&mut processor, &mut receipts);
        if result.is_ok() {
            report_scan(&mut processor)?;
        }
        drop(processor);
        drop(context);
        if result.is_ok() {
            if let Some(output_start) = output_start {
                self.publish_semantic_observations(stores, output_start, None, receipts.take());
            }
            self.apply_error_stop_transition(stores, effects)?;
        }
        result
    }
}
