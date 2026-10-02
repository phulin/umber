//! Apply scanned diagnostics values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_set_interaction_mode(
        &mut self,
        mut operand0: UnexpandablePrimitive,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;
        {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
        }

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_set_interaction_mode(stores, command, &mut operand0)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_set_interaction_mode_value(
        &mut self,
        mut value: i32,
        mut context: tex_command::DiagnosticContextCoordinate,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;
        {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
        }

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_set_interaction_mode_value(
                    stores,
                    command,
                    &mut value,
                    &mut context,
                )
            },
        )
    }

    pub(super) fn execute_message(
        &mut self,
        tokens: tex_command::AttemptTokenListId,
        mut error: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut tokens = publication::publish_tokens(
            self.control.command.state_mut(),
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            tokens,
        )?;
        let context = self.stores.command_context().expect("effect observation");
        let effect = if self.control.operation_observations.is_some() {
            Some(EffectRecord {
                kind: ObservationEffectKind::Message,
                channel: "terminal".into(),
                value: ObservationValue::Bytes(
                    message_tokens_text(&context, tokens.clone()).into_bytes(),
                ),
                source: None,
            })
        } else {
            None
        };
        drop(context);
        self.control.main_loop_active = false;
        {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
        }

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            effect,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_message(stores, command, &mut tokens, &mut error)
            },
        )
    }

    pub(super) fn execute_display_diagnostic(
        &mut self,
        mut operand0: ScannedDisplayDiagnostic,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_display_diagnostic(stores, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_show_lists(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_show_lists(stores, modes, command)
            },
        )
    }

    pub(super) fn execute_show_tokens(
        &mut self,
        tokens: tex_command::AttemptTokenListId,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut tokens = publication::publish_tokens(
            self.control.command.state_mut(),
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            tokens,
        )?;
        let context = self.stores.command_context().expect("effect observation");
        let effect = if self.control.operation_observations.is_some() {
            Some(EffectRecord {
                kind: ObservationEffectKind::ShowTokens,
                channel: "showtokens".into(),
                value: ObservationValue::Tokens(
                    context
                        .token_list(tokens.clone())
                        .iter()
                        .map(|word| observed_macro_token(word.semantic_token(), &context))
                        .collect(),
                ),
                source: None,
            })
        } else {
            None
        };
        drop(context);
        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            effect,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_show_tokens(stores, command, &mut tokens)
            },
        )
    }

    pub(super) fn execute_show_ifs(
        &mut self,
        mut conditions: Vec<tex_command::ActiveCondition>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let context = self.stores.command_context().expect("effect observation");
        let effect = if self.control.operation_observations.is_some() {
            Some(EffectRecord {
                kind: ObservationEffectKind::ShowIfs,
                channel: "showifs".into(),
                value: ObservationValue::Name(render_showifs(&conditions)),
                source: None,
            })
        } else {
            None
        };
        drop(context);
        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            effect,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_show_ifs(stores, command, &mut conditions)
            },
        )
    }

    pub(super) fn execute_show_groups(
        &mut self,
        mut diagnostic: Option<crate::diagnostics::ShowGroupsDiagnostic>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        if diagnostic.is_none() {
            let context = self.stores.command_context().expect("group diagnostic");
            diagnostic = Some(detached_showgroups(
                &context,
                &self.control.active_alignment,
                &self.control.boxes,
                &self.control.active_discretionaries,
                &self.control.active_math_choices,
                &self.control.active_math_left_boundaries,
                &self.control.active_math_shifts,
            ));
        }
        let context = self.stores.command_context().expect("effect observation");
        let effect = if self.control.operation_observations.is_some() {
            diagnostic.as_ref().map(|diagnostic| EffectRecord {
                kind: ObservationEffectKind::ShowGroups,
                channel: "showgroups".into(),
                value: ObservationValue::Name(crate::diagnostics::render_showgroups(diagnostic)),
                source: None,
            })
        } else {
            None
        };
        drop(context);
        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            effect,
            |stores,
             _,
             _,
             active_alignment,
             command,
             boxes,
             active_discretionaries,
             active_math_choices,
             _,
             active_math_left_boundaries,
             active_math_shifts,
             _| {
                semantic::execute_show_groups(
                    stores,
                    active_alignment,
                    command,
                    boxes,
                    active_discretionaries,
                    active_math_choices,
                    active_math_left_boundaries,
                    active_math_shifts,
                    &mut diagnostic,
                )
            },
        )
    }
}
