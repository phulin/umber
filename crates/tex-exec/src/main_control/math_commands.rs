//! Concrete math semantics selected by the one packed mode/command switch.
use super::*;

impl<G> MainControl<G> {
    pub(super) fn apply_math_text_field(
        &mut self,
        kind: NoadKind,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        // TeX82 §1151's caller has already allocated the noad and
        // passes `nucleus(tail)` to `scan_math`.  This is observable
        // while a braced field is being scanned: `\showlists` in the
        // nested math level must still display the enclosing noad in
        // its parent level, with an empty subsidiary field.  Reserve
        // that exact parent-list position before entering the live
        // group, then fill it after the scan completes.
        let simplify_accent = kind == NoadKind::Normal(NoadClass::Ord);
        let mut context = stores.command_context().expect("math text-field admission");
        let node_index = self.execution.modes.current_list().nodes(&context).len();
        self.execution.modes.current_list_mutation().push(
            &mut context,
            Node::MathNoad(MathNoad::new(kind, MathField::Empty)),
        );
        drop(context);
        let episode =
            self.command_scan_math_field(stores, diagnostic_effects, resource_provider)?;
        self.accept_math_field(
            episode,
            ActiveMathFieldTarget::Nucleus {
                node_index,
                simplify_accent,
            },
            stores,
            diagnostic_effects,
        )?;
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_math_script(
        &mut self,
        script: MathScriptKind,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        let target = reserve_script_target(
            self.execution.modes.current_list_mutation(),
            stores,
            diagnostic_effects,
            script,
        )?;
        let episode =
            self.command_scan_math_field(stores, diagnostic_effects, resource_provider)?;
        self.accept_math_field(
            episode,
            ActiveMathFieldTarget::Script(target),
            stores,
            diagnostic_effects,
        )?;
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_math_limits(
        &mut self,
        kind: LimitType,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        if !apply_limits(
            self.execution.modes.current_list_mutation(),
            &mut stores.command_context().expect("math limits admission"),
            kind,
        ) {
            // §1159 falls through to the error only when the tail is
            // not an `op_noad`; the switch is dropped and the job
            // continues.
            let context = self
                .command
                .output_open_context(&stores.command_context().expect("live generation"));
            let mut report = stores.print_err("Limit controls must follow a math operator");
            report.help(&["I'm ignoring this misplaced \\limits or \\nolimits command."]);
            report.context(context);
            report.error().defer_recovery(diagnostic_effects)?;
        }
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_math_fraction(
        &mut self,
        fraction: tex_command::ScannedMathFraction,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        if !start_fraction(
            self.execution.modes.current_list_mutation(),
            stores,
            fraction,
        ) {
            let context = self
                .command
                .output_open_context(&stores.command_context().expect("live generation"));
            let mut report = stores.print_err("Ambiguous; you need another { and }");
            report.help(&[
                "I'm ignoring this fraction specification, since I don't",
                "know whether a construction like `x \\over y \\over z'",
                "means `{x \\over y} \\over z' or `x \\over {y \\over z}'.",
            ]);
            report.context(context);
            report.error().defer_recovery(diagnostic_effects)?;
        }
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_math_choice(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        tracked_region_is_active: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        // TeX82 §1172's `append_choices` opens the first branch with
        // `push_math(math_choice_group); scan_left_brace`, and
        // §1174's `build_choices` repeats exactly that after storing
        // each finished mlist. All four branches are therefore live
        // `math_choice_group` bodies read by ordinary main control,
        // never token lists absorbed ahead of construction: absorbing
        // them backs the opening brace up a second time (an extra
        // `backed_up` input level TeX never pushes) and reorders
        // every input level the branch body itself opens.
        self.execution.active_math_choices.push(0);
        let branches = (|| {
            let display = self.execute_math_choice_branch(
                stores,
                diagnostic_effects,
                tracked_region_is_active,
                resource_provider,
            )?;
            *self
                .execution
                .active_math_choices
                .last_mut()
                .expect("live math choice") = 1;
            let text = self.execute_math_choice_branch(
                stores,
                diagnostic_effects,
                tracked_region_is_active,
                resource_provider,
            )?;
            *self
                .execution
                .active_math_choices
                .last_mut()
                .expect("live math choice") = 2;
            let script = self.execute_math_choice_branch(
                stores,
                diagnostic_effects,
                tracked_region_is_active,
                resource_provider,
            )?;
            *self
                .execution
                .active_math_choices
                .last_mut()
                .expect("live math choice") = 3;
            let script_script = self.execute_math_choice_branch(
                stores,
                diagnostic_effects,
                tracked_region_is_active,
                resource_provider,
            )?;
            Ok::<_, ExecError>((display, text, script, script_script))
        })();
        self.execution.active_math_choices.pop();
        let (display, text, script, script_script) = branches?;
        self.execution.modes.current_list_mutation().push(
            &mut stores.command_context().expect("math choice admission"),
            Node::MathChoice(MathChoice {
                display,
                text,
                script,
                script_script,
            }),
        );
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_math_radical(
        &mut self,
        delimiter: tex_command::ScannedMathDelimiter,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        let mut context = stores.command_context().expect("math radical admission");
        let node_index = self.execution.modes.current_list().nodes(&context).len();
        self.execution.modes.current_list_mutation().push(
            &mut context,
            Node::MathNoad(MathNoad::new(
                NoadKind::Radical {
                    delimiter: delimiter.code,
                },
                MathField::Empty,
            )),
        );
        drop(context);
        let episode =
            self.command_scan_math_field(stores, diagnostic_effects, resource_provider)?;
        self.accept_math_field(
            episode,
            ActiveMathFieldTarget::Nucleus {
                node_index,
                simplify_accent: false,
            },
            stores,
            diagnostic_effects,
        )?;
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_math_accent(
        &mut self,
        accent: tex_command::ScannedMathCharacter,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        let accent = math_char(
            &stores.command_context().expect("live generation"),
            u32::from(accent.code),
            accent.provenance.primary,
        )?
        .1;
        let mut context = stores.command_context().expect("math accent admission");
        let node_index = self.execution.modes.current_list().nodes(&context).len();
        self.execution.modes.current_list_mutation().push(
            &mut context,
            Node::MathNoad(MathNoad::new(NoadKind::Accent { accent }, MathField::Empty)),
        );
        drop(context);
        let episode =
            self.command_scan_math_field(stores, diagnostic_effects, resource_provider)?;
        self.accept_math_field(
            episode,
            ActiveMathFieldTarget::Nucleus {
                node_index,
                simplify_accent: false,
            },
            stores,
            diagnostic_effects,
        )?;
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_text_accent_in_math(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        // TeX82 §1110 reports before `math_ac` advances to
        // §436's `scan_fifteen_bit_int`. In particular, §82's
        // context must still own an exhausted token-list level
        // whose last command was the text `\accent`.
        let context = self
            .command
            .output_open_context(&stores.command_context().expect("live generation"));
        let mut report = stores.print_err("Please use \\mathaccent for accents in math mode");
        report.help(&[
            "I'm changing \\accent to \\mathaccent here; wish me luck.",
            "(Accents are not the same in formulas as they are in text.)",
        ]);
        report.context(context);
        report.error().defer_recovery(diagnostic_effects)?;
        self.apply_error_stop_transition(stores, diagnostic_effects)?;
        let accent =
            self.command_scan_math_character(stores, diagnostic_effects, resource_provider)?;
        self.apply_math_accent(accent, stores, diagnostic_effects, resource_provider)
    }

    pub(super) fn apply_math_mu_glue(
        &mut self,
        spec: GlueSpec,
        shared_zero: bool,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        self.execution.modes.current_list_mutation().push(
            &mut stores.command_context().expect("math glue admission"),
            Node::Glue {
                origin: if shared_zero {
                    tex_state::node::GlueSpecOrigin::SharedZero
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
                spec,
                kind: GlueKind::MuSkip,
                leader: None,
            },
        );
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_math_mu_kern(
        &mut self,
        amount: Scaled,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        self.execution.modes.current_list_mutation().push(
            &mut stores.command_context().expect("math kern admission"),
            Node::Kern {
                amount,
                kind: KernKind::Mu,
            },
        );
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_math_equation_number(
        &mut self,
        side: crate::mode::EqNoSide,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        if self.execution.modes.current_mode() != Mode::DisplayMath {
            // §1140's `mmode+eq_no` is guarded by `privileged`, and
            // §1049 lists `eq_no` under `non_math`; both failures end
            // in §1050's `report_illegal_case`, which names the mode
            // the command was actually used in.
            let primitive = match side {
                crate::mode::EqNoSide::Left => "leqno",
                crate::mode::EqNoSide::Right => "eqno",
            };
            let mode = self.execution.modes.current_mode();
            let mut command_context = stores.command_context().expect("live generation");
            let token = Token::Cs(
                command_context
                    .known_control_sequence(primitive)
                    .expect("equation-number primitives are installed"),
            );
            let context = self.command.output_open_context(&command_context);
            crate::diagnostics::report_illegal_case_with_context(
                &mut command_context,
                diagnostic_effects,
                token,
                mode,
                Some(context),
            )?;
        } else {
            let display = take_finished_math_list(&mut self.execution.modes, stores)?;
            let mut context = stores.command_context().expect("live generation");
            enter_group(
                &mut context,
                &mut self.command,
                diagnostic_effects,
                GroupKind::MathShift,
            );
            context
                .assign_int_param(IntParam::FAM, -1, tex_state::AssignmentScope::Local)
                .expect("family parameter is admitted");
            self.execution.modes.push_at_line(
                Mode::Math,
                self.command
                    .current_file_line_number()
                    .try_into()
                    .unwrap_or(i32::MAX),
            )?;

            let shift = MathShiftContext::EqNo(side);
            self.execution.active_math_shifts.push(shift);
            let display = context
                .admit_page_node_span(display)
                .expect("equation-number display belongs to the live page owner");
            self.execution
                .modes
                .current_list_mutation()
                .set_display_eq_no(&context, crate::mode::DisplayEqNo { side, display });
        }
        Ok(ReplayStep::Continue)
    }
}
