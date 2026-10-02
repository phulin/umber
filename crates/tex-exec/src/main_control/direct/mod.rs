//! Direct main-control calls over the sole persistent interpreter.
use super::*;
#[macro_use]
mod scan_macros;
mod apply_alignment;
mod apply_alignment_recovery;
mod apply_assignments;
mod apply_box_builders;
mod apply_boxes;
mod apply_code_tables;
mod apply_diagnostics;
mod apply_fonts;
mod apply_groups;
mod apply_leaders;
mod apply_math;
mod apply_page_state;
mod apply_paragraph;
mod apply_pdf;
mod apply_recovery;
mod apply_registers;
mod apply_streams;
mod apply_text;
mod characters;
mod command_codes;
mod dispatch;
mod scan_alignment_delivery;
mod scan_alignment_events;
mod scan_alignment_peek;
mod scan_arithmetic;
mod scan_box_dimension;
mod scan_character_recovery;
mod scan_code_tables;
mod scan_fonts;
mod scan_group_recovery;
mod scan_leaders;
mod scan_math;
mod scan_meaning_recovery;
mod scan_pdf_fonts;
mod scan_primitive_recovery;
mod scan_registers;
mod scan_replay;
mod scan_scalars;
mod scan_shapes;
mod scan_streams;
use command_codes as codes;
pub(super) mod paragraph_start;
mod publication;

struct DirectDispatch<'call, 'provider, G> {
    control: &'call mut MainControl<G>,
    stores: &'call mut Universe<G>,
    effects: &'call mut DiagnosticEffects,
    provider: &'call mut Option<&'provider mut dyn ResourceProvider<G>>,
    diagnostics: Vec<PendingDiagnostic<G>>,
    cursor: Option<tex_command::CommandDeliveryCursor>,
    current_origin: Option<OriginId>,
    output_start: OperationOutputStart,
    suppress_left_boundary: bool,
}

impl<G> DirectDispatch<'_, '_, G> {
    fn report_scan(&mut self) -> Result<(), ExecError> {
        self.control
            .capture_first_reported_command_error_context(self.stores);
        if self.diagnostics.is_empty() {
            return Ok(());
        }
        self.control
            .capture_first_causal_context(self.stores, &self.diagnostics);
        report_pending_diagnostics(
            self.stores,
            self.effects,
            std::mem::take(&mut self.diagnostics),
        )
    }
    fn finish_host(
        &mut self,
        applied: Result<ReplayStep, ExecError>,
    ) -> Result<ReplayStep, ExecError> {
        self.control.main_loop_active = false;
        self.control
            .finish_host_owned_step(applied, self.output_start, self.stores, self.effects)
    }
}

fn unimplemented_meaning<G>(
    command: &tex_command::CurrentCommand<G>,
    meaning: Meaning,
    mode: Mode,
) -> ExecError {
    ExecError::UnimplementedMeaning {
        meaning,
        mode,
        origin: command.origin(),
    }
}
impl<G> MainControl<G> {
    #[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
    fn direct_glue_pointer_reassigned(
        context: &CommandContext<'_, G>,
        mu: bool,
        index: u16,
        value: GlueSpec,
        source_identity: Option<GlueId<G>>,
        source_register: Option<(bool, u16)>,
        global: bool,
        skip_pointer_sources: &[GluePointerSource<G>],
        muskip_pointer_sources: &[GluePointerSource<G>],
    ) -> bool {
        if global {
            return false;
        }
        let physical = if mu {
            context.muskip(index)
        } else {
            context.glue_register(index).ok().flatten()
        };
        let Some(physical) = physical else {
            return false;
        };
        if context.glue(physical) == GlueSpec::ZERO && value == GlueSpec::ZERO {
            return true;
        }
        let Some(source_identity) = source_identity else {
            return false;
        };
        if source_register == Some((mu, index)) {
            return true;
        }
        let pointer_sources = if mu {
            muskip_pointer_sources
        } else {
            skip_pointer_sources
        };
        let canonical_source = pointer_sources
            .get(usize::from(index))
            .and_then(|entry| *entry)
            .filter(|(recorded, _)| *recorded == physical)
            .map_or(Some(physical), |(_, source)| source);
        canonical_source == Some(source_identity)
    }
}

impl<G> MainControl<G> {
    pub(super) fn execute_control(
        &mut self,
        stores: &mut Universe<G>,
        settled: Option<tex_command::CurrentCommand<G>>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
        effects: &mut DiagnosticEffects,
        tracked: bool,
        provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        self.ensure_primitive_handles(stores);
        let mode = self.modes.current_mode();
        let output_start = OperationOutputStart {
            outer_paragraph_was_active: mode == Mode::Horizontal && self.modes.depth() == 2,
            artifact_count: stores.world().committed_artifacts().len(),
            effect_count: stores.world().effect_records().len(),
            prepared_page_count: self.prepared_dvi_pages.len(),
            tracked_region_is_active: tracked,
        };
        if tracked {
            self.observe_control_projections(stores);
        }
        let mut direct = DirectDispatch {
            control: self,
            stores,
            effects,
            provider,
            diagnostics: Vec::new(),
            cursor,
            current_origin: None,
            output_start,
            suppress_left_boundary: false,
        };
        let result = if let Some(command) = settled {
            direct.run_command(Some(command), None, true, None)
        } else {
            direct.deliver_control_command()
        };
        if result.is_ok() {
            direct
                .control
                .apply_error_stop_transition(direct.stores, direct.effects)?;
        }
        result
    }
    fn observe_control_projections(&mut self, stores: &mut Universe<G>) {
        let mode = self.modes.current_mode();
        let fingerprint = self.modes.semantic_fingerprint(stores);
        let mut context = stores
            .command_context()
            .expect("tracked control projection");
        let mut facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
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
impl<G> DirectDispatch<'_, '_, G> {
    fn deliver_control_command(&mut self) -> Result<ReplayStep, ExecError> {
        if !self.control.main_control_entered {
            let mut context = self
                .stores
                .command_context()
                .expect("main-control prologue");
            if self.control.enter_main_control(&mut context) {
                publish_named_token_list_pushes(
                    &mut self.control.command,
                    &mut context,
                    self.effects,
                    &mut self.control.operation_observations,
                );
            }
        }
        if self.control.modes.current_mode() == Mode::DisplayMath
            && self.control.modes.current_list().has_display_alignment()
        {
            return self.scan_display_alignment_tail();
        }
        let phase = alignment_preamble(self.control.active_alignment.as_mut());
        if phase.is_some() {
            self.scan_replay_step(phase)
        } else {
            self.run_command(None, None, true, None)
        }
    }
}

impl<G> MainControl<G> {
    pub(super) fn update_save_stack_highwater(&mut self, stores: &mut Universe<G>) {
        let words = stores
            .checked_save_stack_words(self.command_profile().capabilities().supports_etex())
            .expect("save-stack highwater")
            .saturating_add(self.boxes.box_spec_words);
        self.max_save_stack = self.max_save_stack.max(words);
    }
}

fn scan_error(error: impl Into<ExecError>, origin: Option<OriginId>) -> ExecError {
    let error = error.into();
    match origin {
        Some(origin) => error.capture_command_origin(origin),
        None => error,
    }
}

impl<G> DirectDispatch<'_, '_, G> {
    fn schedule_assignment_token(&mut self) -> Result<(), ExecError> {
        let mut context = self
            .stores
            .command_context()
            .map_err(|_| ExecError::MissingToken {
                context: "afterassignment admission",
            })?;
        let mut facts = ExecutorHostFacts {
            modes: &self.control.modes,
            pdf_ignore_depth: self.control.pdf_ignore_depth,
            telemetry: &mut self.control.episode_telemetry,
        };
        schedule_afterassignment(
            &mut self.control.command,
            self.control.fuel.fuel_mut(),
            &mut self.control.capabilities,
            &mut facts,
            &mut self.control.operation_observations,
            self.effects,
            &mut context,
        )
    }
    fn record_glue_source(&mut self, mu: bool, index: u16, source: Option<GlueId<G>>) {
        let context = self
            .stores
            .command_context()
            .expect("glue source admission");
        let physical = if mu {
            context.muskip(index)
        } else {
            context.glue_register(index).ok().flatten()
        };
        if let Some(physical) = physical {
            let pointers = if mu {
                &mut self.control.muskip_pointer_sources
            } else {
                &mut self.control.skip_pointer_sources
            };
            if pointers.len() <= usize::from(index) {
                pointers.resize(usize::from(index) + 1, None);
            }
            pointers[usize::from(index)] = Some((physical, source));
        }
    }
}
impl<G> MainControl<G> {
    pub(super) fn resume_character_loop(
        &mut self,
        stores: &mut Universe<G>,
        character: Option<char>,
    ) {
        let context = stores.command_context().expect("character loop admission");
        self.main_loop_active = character.is_some_and(|ch| {
            matches!(
                self.modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) && u8::try_from(u32::from(ch)).ok().is_some_and(|code| {
                context
                    .font_char_metrics(context.current_font(), code)
                    .is_some()
            })
        });
    }
}
