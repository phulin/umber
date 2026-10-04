//! Ordered evidence publication after one direct typed semantic call.
use super::*;

impl<G> MainControl<G> {
    #[allow(clippy::too_many_arguments)] // one borrowed application crosses all actual semantic owners
    pub(super) fn apply_direct_leaf(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
        output_start: OperationOutputStart,
        effect: Option<EffectRecord>,
        apply_leaf: impl FnOnce(
            &mut CommandContext<'_, G>,
            &mut ModeNest,
            &mut u64,
            &mut Option<ActiveReplayAlignment<G>>,
            &mut CommandMachine<'_, '_, G>,
            &mut ReplayBoxes<G>,
            &[ActiveDiscretionary],
            &[usize],
            &mut Vec<ActiveMathFieldTarget>,
            &[bool],
            &[MathShiftContext],
            &mut PreparedDviPages,
        ) -> Result<MainControlStep, ExecError>,
    ) -> Result<MainControlStep, ExecError> {
        let mut operation_resource_provider = resource_provider.take();
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::SemanticApply,
        );
        #[cfg(feature = "profiling")]
        let _semantic_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        let observing = self.operation_observations.is_some();
        let mut assignment_receipts = observing.then(Vec::new);
        let output_routine_was_active = self.execution.boxes.output_routine_active;
        let mut command = CommandMachine {
            state: &mut self.command,
            fuel: self.fuel.fuel_mut(),
            capabilities: &mut self.capabilities,
            resource_provider: operation_resource_provider.take(),
            declined_resource_attempt: Some(&mut self.declined_resource_attempt),
            host_facts: CommandMachineHostFacts::Detached {
                pdf_ignore_depth: self.execution.pdf_ignore_depth,
                telemetry: &mut self.execution.episode_telemetry,
            },
            observations: self
                .operation_observations
                .as_mut()
                .map(|buffer| buffer as &mut dyn CommandObserver),
            assignment_receipts: assignment_receipts.as_mut(),
            diagnostic_effects,
            shown_mode: &mut self.execution.shown_mode,
            initex: self.execution.initex,
            emit_dvi_override: self.execution.emit_dvi_override,
            immediate_prints: &mut self.execution.immediate_prints,
            prepared_shipout: &mut self.execution.prepared_shipout,
            pending_show_completion: None,
            pending_outer_page_build_context: None,
            output_routine_active: self.execution.boxes.output_routine_active,
        };
        let (mut result, mut pending_page_output) = {
            let mut admitted_context = stores.command_context().expect("live semantic generation");
            let context = &mut admitted_context;
            let result = apply_leaf(
                context,
                &mut self.execution.modes,
                &mut self.execution.next_alignment_identity,
                &mut self.execution.active_alignment,
                &mut command,
                &mut self.execution.boxes,
                &self.execution.active_discretionaries,
                &self.execution.active_math_choices,
                &mut self.execution.active_math_fields,
                &self.execution.active_math_left_boundaries,
                &self.execution.active_math_shifts,
                &mut self.execution.prepared_dvi_pages,
            );
            if result.is_ok() {
                command.publish_named_token_list_pushes(context);
            }
            let pending_page_output = PendingPageOutputFacts::capture(context);
            (result, pending_page_output)
        };
        if result.is_ok()
            && let Some(completion) = command.pending_show_completion.take()
        {
            // TeX82 §§1293/1298 completes a long `\show` only after
            // `end_diagnostic` has restored the selector and made the whole
            // dump visible. The dump is an operation-local detached program,
            // whereas `error` still owns the live World dialogue, so release
            // command admission, atomically publish the dump, and only then
            // enter the synchronous `! OK.` completion. This also preserves
            // the dump when ErrorStop exits from the dialogue.
            stores
                .world_mut()
                .publish_diagnostic_effects_preserving(command.diagnostic_effects);
            let completion_result = {
                let mut context = stores
                    .command_context()
                    .expect("show completion has a live generation");
                crate::diagnostics::complete_show(
                    &mut context,
                    command.diagnostic_effects,
                    completion.long,
                    Some(completion.context),
                )
            };
            if let Err(error) = completion_result {
                result = Err(error);
            }
        }
        if result.is_ok()
            && let Some(context) = command.pending_outer_page_build_context.take()
        {
            // TeX82 §1099 runs `build_page` after §1100 has closed the
            // insertion group. Group-command and restoration traces are
            // detached, while §993/§1009 recovery still uses the live error
            // dialogue. Publish the completed group-close program at this
            // outer admission boundary before page building can report.
            stores
                .world_mut()
                .publish_diagnostic_effects_preserving(command.diagnostic_effects);
            let mut stores = stores.command_context().expect("live generation");
            match crate::vertical::build_page_if_outer_vertical_with_error_context(
                &self.execution.modes,
                &mut stores,
                command.diagnostic_effects,
                &context,
            ) {
                Ok(()) => pending_page_output = PendingPageOutputFacts::capture(&stores),
                Err(error) => result = Err(error),
            }
        }
        if result.is_ok() {
            if !command.immediate_prints.is_empty() {
                // TeX82 §1375 performs an immediate write only after its
                // token list has been expanded. Expansion/command traces are
                // detached during that scan, while the resulting write is an
                // outer World publication; commit the former first so the
                // expanded text cannot overtake its own trace.
                stores
                    .world_mut()
                    .publish_diagnostic_effects_preserving(command.diagnostic_effects);
            }
            for print in command.immediate_prints.drain(..) {
                match print.encoded {
                    Some(bytes) if print.ensure_line_start => {
                        stores.world_mut().publish_print_nl_encoded_bytes(
                            print.sink,
                            &bytes,
                            print.max_print_line,
                        );
                    }
                    Some(bytes) => {
                        stores.world_mut().publish_print_encoded_bytes(
                            print.sink,
                            &bytes,
                            print.max_print_line,
                        );
                    }
                    None if print.ensure_line_start => {
                        stores.world_mut().publish_print_nl_text(
                            print.sink,
                            &print.text,
                            print.max_print_line,
                        );
                    }
                    None => {
                        stores.world_mut().publish_print_text(
                            print.sink,
                            &print.text,
                            print.max_print_line,
                        );
                    }
                }
            }
            if let Some(shipout) = command.prepared_shipout.take() {
                // TeX82 §§367/1006 print the delivered command and page
                // cost before §638 opens the shipout marker. Both reports
                // are detached while their respective admitted scanners run,
                // so move their complete ordered program into World before
                // the outer shipout transaction can materialize `[`. Any
                // diagnostics produced by deferred whatsit replay join that
                // transaction separately at its own pre-commit boundary.
                stores
                    .world_mut()
                    .publish_diagnostic_effects_preserving(command.diagnostic_effects);
                let staged =
                    shipout_replay_box(shipout, stores, &mut command, &self.execution.modes);
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
        } else {
            command.immediate_prints.clear();
            stores.release_shipout_source();
            if let Some(shipout) = command.prepared_shipout.take()
                && let Some(region) = shipout.region
            {
                stores
                    .release_page_node_region(region)
                    .expect("aborted shipout command releases its nested page region");
            }
        }
        let completed_output_routine =
            output_routine_was_active && !self.execution.boxes.output_routine_active;
        operation_resource_provider = command.resource_provider.take();
        drop(command);
        *resource_provider = operation_resource_provider;
        if result.is_ok() && completed_output_routine {
            // §1026 has consumed the output mode list and established the
            // complete held-over contribution closure. Rotate only now, after
            // any user `\shipout` has detached its output and after box255 has
            // either been consumed or diagnosed and cleared.
            self.execution.page_region_succession_pending = true;
            self.finish_pending_page_region_succession(stores);
        }
        if matches!(result, Ok(MainControlStep::EndOfInput))
            && self.execution.root_completion == RootCompletionPolicy::RequireTeXEnd
        {
            result = Ok(self.handle_root_end_of_input(stores));
        }
        if result.is_ok() {
            self.fire_pending_page_output(stores, diagnostic_effects, pending_page_output)?;
            if observing {
                self.publish_semantic_observations(
                    stores,
                    output_start,
                    effect,
                    assignment_receipts,
                );
            }
            self.finish_paragraph_boundary(output_start.outer_paragraph_was_active, stores);
        }
        result
    }

    /// Publishes evidence only for the active observation owner.
    pub(super) fn publish_semantic_observations(
        &mut self,
        stores: &mut Universe<G>,
        output_start: OperationOutputStart,
        effect: Option<EffectRecord>,
        assignment_receipts: Option<Vec<tex_command::MutationRecord>>,
    ) {
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::EvidencePublication,
        );
        #[cfg(feature = "profiling")]
        let _evidence_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
        );
        {
            // These records are produced by applying the step, after the
            // command-processor episode's own borrow has ended. Named token
            // pushes were already sent directly to this operation's optional
            // commit sink while their live command context was admitted.
            let mut records: Vec<CommandObservation> = Vec::new();
            let effects = committed_stream_effect_observations(
                output_start.effect_count,
                output_start.prepared_page_count,
                stores,
                &self.execution.prepared_dvi_pages,
            );
            records.extend(
                assignment_receipts
                    .into_iter()
                    .flatten()
                    .map(CommandObservation::Mutation),
            );
            if let Some(effect) = effect {
                records.push(CommandObservation::Effect(effect));
            }
            records.extend(effects.into_iter().map(CommandObservation::Effect));
            for shipout in committed_shipout_observations(output_start.artifact_count, stores) {
                records.push(CommandObservation::Effect(shipout));
            }
            if let Some(mut observations) = self.execution.page_output_observations.take() {
                observations.append_to(&mut records);
            }
            self.observe_committed(records);
        }
        self.execution.page_output_observations = None;
    }
}
