//! TeX82 discretionary command application on the sole main-control owner.

use super::*;

impl<G> MainControl<G> {
    /// Enters TeX82 §1117's live `disc_group` after the command processor has
    /// consumed only its opening brace.
    pub(super) fn begin_discretionary(
        &mut self,
        _opening: ScannedDiscretionaryOpening,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            let mut context = stores.command_context().expect("live generation");
            start_paragraph(
                &mut self.command,
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                true,
            )?;
        }
        {
            let mut context = stores.command_context().expect("live generation");
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                self.fuel.fuel_mut(),
            )?;
        }
        self.open_discretionary_part(stores, diagnostic_effects)?;
        self.active_discretionaries.push(ActiveDiscretionary {
            parts: Vec::new(),
            rejected: false,
        });
        Ok(ReplayStep::Continue)
    }

    pub(super) fn open_discretionary_part(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<(), ExecError> {
        // TeX82 §216 checks nest capacity before saving the current semantic
        // level. Fatal overflow is committed by main control, so
        // this fallible operation must precede both halves of the live
        // discretionary lifecycle: no rejected opener may leave a disc_group
        // without its restricted-horizontal mode.
        self.modes.push_at_line(
            Mode::RestrictedHorizontal,
            self.command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        let mut context = stores.command_context().expect("live generation");
        enter_group(
            &mut context,
            &mut self.command,
            diagnostic_effects,
            GroupKind::Disc,
        );
        Ok(())
    }

    /// Implements §1120's `build_discretionary`: finish the current live
    /// restricted-horizontal list, `unsave`, and either scan the next opening
    /// brace or append the completed three-part node.
    pub(super) fn finish_discretionary_part(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        let mut level = {
            let mut context = stores.command_context().expect("live generation");
            crate::box_runtime::commit_current_list(
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                self.fuel.fuel_mut(),
            )?
        };
        // TeX82 §1121 advances `q` across the admissible prefix and, on the
        // first forbidden node `p`, severs `link(q)`. Thus the prefix remains
        // this discretionary part while `show_box(p)` reports and flushes the
        // entire suffix beginning at the offending node.
        let (nodes, deleted, prefix_end) = {
            let mut stores = stores.command_context().expect("live generation");
            let part_nodes = level.list().nodes(&stores);
            let part_len = part_nodes.len();
            let first_forbidden = match part_nodes.try_for_each_range(0..part_len, |index, node| {
                if matches!(
                    node,
                    tex_state::NodeView::Char { .. }
                        | tex_state::NodeView::Lig { .. }
                        | tex_state::NodeView::Kern { .. }
                        | tex_state::NodeView::Rule { .. }
                        | tex_state::NodeView::HList(_)
                        | tex_state::NodeView::VList(_)
                ) {
                    core::ops::ControlFlow::Continue(())
                } else {
                    core::ops::ControlFlow::Break(index)
                }
            }) {
                core::ops::ControlFlow::Break(index) => Some(index),
                core::ops::ControlFlow::Continue(()) => None,
            };
            let prefix_end = first_forbidden.unwrap_or(part_len);
            let part = level.list_mutation().take_span();
            let nodes = stores.slice_page_node_span(part, 0..prefix_end);
            let deleted = first_forbidden
                .map(|index| stores.slice_page_node_span(part, index..part_len).list());
            let aftergroup = leave_group_payloads(
                &mut stores,
                &mut self.command,
                diagnostic_effects,
                GroupKind::Disc,
            )
            .map_err(|_| ExecError::MissingToken {
                context: "discretionary group",
            })?;
            schedule_aftergroup(
                &mut self.command_machine(diagnostic_effects),
                &mut stores,
                aftergroup,
            )?;
            (nodes, deleted, prefix_end)
        };

        let (part_count, replacement_too_long) = {
            let active = self
                .active_discretionaries
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "active discretionary",
                })?;
            active.parts.push(nodes);
            let part_count = active.parts.len();
            let replacement_too_long = part_count == 3 && prefix_end > 127;
            active.rejected |= replacement_too_long;
            (part_count, replacement_too_long)
        };
        if let Some(deleted) = deleted {
            let mut stores = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&stores);
            report_improper_discretionary(&mut stores, diagnostic_effects, deleted, context)?;
        }
        if replacement_too_long {
            let mut stores = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&stores);
            crate::error_report::report_ordered_error(
                &mut stores,
                diagnostic_effects,
                "Discretionary list is too long",
                &["Wow---I never thought anybody would tweak me here."],
                context,
            )?;
        }
        if part_count < 3 {
            let mut diagnostics = Vec::new();
            {
                let mut context = stores.command_context().expect("live generation");
                let mut host_facts = ExecutorHostFacts {
                    modes: &self.modes,
                    pdf_ignore_depth: self.pdf_ignore_depth,
                    telemetry: &mut self.episode_telemetry,
                };
                let mut processor = if let Some(provider) = resource_provider.as_deref_mut() {
                    command_processor_with_resource_provider(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        provider,
                        &mut self.declined_resource_attempt,
                        &mut self.operation_observations,
                        diagnostic_effects,
                        &mut context,
                    )
                } else {
                    command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        &mut self.operation_observations,
                        diagnostic_effects,
                        &mut context,
                    )
                };
                let _ = processor
                    .scan_discretionary_opening()
                    .map_err(command_error)?;
                diagnostics.extend(
                    processor
                        .take_semantic_diagnostics()
                        .into_iter()
                        .map(PendingDiagnostic::Command),
                );
            }
            self.capture_first_causal_context(stores, &diagnostics);
            report_pending_diagnostics(stores, diagnostic_effects, diagnostics)?;
            self.open_discretionary_part(stores, diagnostic_effects)?;
            return Ok(ReplayStep::Continue);
        }
        let active = self
            .active_discretionaries
            .pop()
            .expect("three parts require an active discretionary");
        if active.rejected {
            return Ok(ReplayStep::Continue);
        }
        let [pre, post, mut replace]: [tex_state::page_node_arena::PageListSpan; 3] = active
            .parts
            .try_into()
            .expect("discretionary completes after exactly three parts");
        if matches!(self.modes.current_mode(), Mode::Math | Mode::DisplayMath)
            && !replace.is_empty()
        {
            // TeX82 §1120 diagnoses and deletes only a nonempty third part
            // in math mode; the discretionary and its first two parts survive.
            let mut command_context = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&command_context);
            // Section 1120 calls `unsave` before diagnosing a forbidden
            // nonempty replacement in math mode. Publish that completed
            // restoration program before the synchronous error dialogue.
            command_context.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
            report_escaped_error(
                &mut command_context,
                diagnostic_effects,
                "Illegal math ",
                "discretionary",
                "",
                &[
                    "Sorry: The third part of a discretionary break must be",
                    "empty, in math formulas. I had to delete your third part.",
                ],
                context,
            )?;
            replace = tex_state::page_node_arena::PageListSpan::empty();
        }
        let physical_replace_count = stores
            .command_context()
            .expect("live generation")
            .page_node_span(replace)
            .expect("discretionary replacement is a live page list")
            .len()
            .try_into()
            .expect("TeX discretionary replacement count fits a quarterword");
        self.modes.current_list_mutation().push(
            &mut stores.command_context().expect("live generation"),
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre: pre.list(),
                post: post.list(),
                replace: replace.list(),
                physical_replace_count,
            },
        );
        Ok(ReplayStep::Continue)
    }

    /// Executes TeX82 §1113's `append_discretionary` shorthand for `\-`.
    pub(super) fn apply_discretionary_hyphen(
        &mut self,
        origin: OriginId,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            let mut context = stores.command_context().expect("live generation");
            start_paragraph(
                &mut self.command,
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                true,
            )?;
        }
        let pre = {
            let mut stores = stores.command_context().expect("live generation");
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut self.modes,
                &mut stores,
                diagnostic_effects,
                self.fuel.fuel_mut(),
            )?;
            let font = stores.current_font();
            match u8::try_from(stores.font_hyphen_char(font)) {
                Ok(hyphen) if stores.font_char_metrics(font, hyphen).is_some() => stores
                    .publish_page_nodes(vec![Node::Char {
                        font,
                        ch: char::from(hyphen),
                        origin,
                    }]),
                Ok(hyphen) => {
                    // TeX82 §1113 delegates the in-range hyphen to §581's
                    // `new_character`: an absent glyph warns and leaves the
                    // pre-break list empty.
                    crate::diagnostics::report_missing_character_warning(
                        &mut stores,
                        diagnostic_effects,
                        font,
                        char::from(hyphen),
                        self.command_profile() == CommandProfile::ETEX26,
                    );
                    stores.publish_page_nodes(Vec::new())
                }
                Err(_) => stores.publish_page_nodes(Vec::new()),
            }
        };
        let empty = tex_state::page_node_arena::PageListId::empty();
        self.modes.current_list_mutation().push(
            &mut stores.command_context().expect("live generation"),
            Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre,
                post: empty,
                replace: empty,
                physical_replace_count: 0,
            },
        );
        Ok(ReplayStep::Continue)
    }
}
