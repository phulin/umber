//! Direct alignment semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_align_peek_restart<G>(
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    alignment: &mut AlignmentIdentity,
) -> Result<ReplayStep, ExecError> {
    {
        let active = active_alignment
            .as_mut()
            .filter(|active| active.identity == *alignment)
            .ok_or(ExecError::MissingToken {
                context: "alignment restart lookahead",
            })?;
        active.align_peek_pending = true;
        active.align_peek_after_noalign = true;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_alignment_template_entered<G>(
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §§1034--1038's character lookahead calls `get_next`,
        // whose alignment interception inserts the v-template and then
        // resumes that same lookahead. Characters at the end of a cell
        // body can therefore ligate or kern with the v-template prefix.
        // The run ends at §1131's `endv` below, before `fin_col` advances
        // across a tab or span delimiter.
        let _active = active_alignment.as_mut().ok_or(ExecError::MissingToken {
            context: "active replay alignment",
        })?;
        Ok(ReplayStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
pub(in crate::main_control) fn execute_begin_alignment<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    next_alignment_identity: &mut u64,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    vertical: &mut bool,
    owner: &mut Option<tex_state::interner::Symbol>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §774's display-math entry accepts an alignment only when
        // the current formula is empty. `flush_math` owns both the
        // material and an incomplete fraction before `push_nest` opens
        // the alignment list; retaining either here makes §812's display
        // alignment handoff collide with pre-alignment math material.
        if modes.current_mode() == Mode::DisplayMath {
            let has_formula = !modes.current_list().nodes(stores).is_empty()
                || modes.current_list().incomplete_fraction().is_some();
            if has_formula {
                let primitive = if *vertical { "\\valign" } else { "\\halign" };
                let context = command.state.output_open_context(&**stores);
                let mut report = stores.print_err(&format!("Improper {primitive} inside $$'s"));
                report.help(&[
                    "Displays can use special alignments (like \\eqalignno)",
                    "only if nothing but the alignment itself is between $$'s.",
                    "So I've deleted the formulas that preceded this alignment.",
                ]);
                report.context(context);
                report.error().defer_recovery(command.diagnostic_effects)?;
                let mut list = modes.current_list_mutation();
                list.take_nodes();
                list.take_incomplete_fraction();
            }
        }
        if let Some(outer) = active_alignment.take() {
            command
                .state
                .apply_alignment_request(&**stores, AlignmentRequest::Suspend(outer.identity))
                .map_err(|_| ExecError::MissingToken {
                    context: "nested alignment suspension",
                })?;
            boxes.suspended_alignments.push(outer);
        }
        let identity = AlignmentIdentity::new(*next_alignment_identity);
        *next_alignment_identity = next_alignment_identity.wrapping_add(1);
        command
            .state
            .apply_alignment_request(&**stores, AlignmentRequest::Begin(identity))
            .map_err(|_| ExecError::MissingToken {
                context: "alignment lifecycle",
            })?;
        let tabskip_id = stores.glue_param(GlueParam::TAB_SKIP);
        let default_tabskip = tex_state::node::GlueValue {
            spec: tabskip_id.map_or(GlueSpec::ZERO, |id| stores.glue(id)),
            origin: if tabskip_id.is_none() {
                tex_state::node::GlueSpecOrigin::SharedZero
            } else {
                tex_state::node::GlueSpecOrigin::Owned
            },
        };
        *active_alignment = Some(ActiveReplayAlignment {
            identity,
            kind: if *vertical {
                AlignmentKind::VAlign
            } else {
                AlignmentKind::HAlign
            },
            owner: *owner,
            packing: AlignmentPackSpec::Natural,
            columns: Vec::new(),
            repeat_start: None,
            column: 0,
            preamble_opening_pending: true,
            preamble_start_pending: false,
            cell_opening_pending: false,
            next_cell_opening_pending: false,
            align_peek_pending: false,
            align_peek_after_noalign: false,
            noalign_open: false,
            captured_row_count: 0,
            captured_cell_count: 0,
            tabskips: vec![default_tabskip],
            default_tabskip,
            row_migrations: tex_state::page_node_arena::PageListSpan::empty(),
            cell_span: 1,
            row_open: false,
            cell_open: false,
        });
        // TeX82 §774's `init_align` runs `push_nest` and then only
        // *negates* an ordinary vertical mode, so the alignment's own
        // list inherits that list's `aux` (`prev_depth`). Display math is
        // the deliberate exception: its `aux` is `incompleat_noad`, so
        // §774 reaches through it to `nest[nest_ptr-2].aux_field.sc`, the
        // enclosing vertical list's `prev_depth`. Umber's independent
        // mode levels do not copy either value on push, so select the
        // canonical source explicitly before opening the alignment.
        let enclosing_prev_depth = if modes.current_mode() == Mode::DisplayMath {
            modes.enclosing_vertical_prev_depth()
        } else {
            modes.current_list().prev_depth()
        };
        modes.push_at_line(
            replay_alignment_mode(if *vertical {
                AlignmentKind::VAlign
            } else {
                AlignmentKind::HAlign
            }),
            i32::try_from(command.state.current_file_line_number()).unwrap_or(i32::MAX),
        )?;
        if let Some(prev_depth) = enclosing_prev_depth {
            modes.current_list_mutation().set_prev_depth(prev_depth);
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_alignment_preamble_opening<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    alignment: &mut AlignmentIdentity,
    packing: &mut ScannedPackingSpec,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        command
            .state
            .apply_alignment_request(&**stores, AlignmentRequest::Preamble(*alignment))
            .map_err(|_| ExecError::MissingToken {
                context: "alignment preamble lifecycle",
            })?;
        if let Some(active) = active_alignment.as_mut()
            && active.identity == *alignment
        {
            active.packing = alignment_pack_spec(*packing);
            active.preamble_opening_pending = false;
            active.preamble_start_pending = true;
        }
        // TeX82 §774's `init_align` reaches the preamble through §645's
        // `scan_spec(align_group,false)`, whose `new_save_level(c)` opens
        // the save level that brackets the alignment as a whole. §800's
        // `fin_align` removes it with the second of its two `unsave`s.
        enter_group(
            stores,
            command.state,
            command.diagnostic_effects,
            GroupKind::Align,
        );
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_alignment_preamble_start<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    alignment: &mut AlignmentIdentity,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // `init_row` reaches `align_peek` before `init_col` selects the
        // first cell. Keep the first pair validated here, but defer
        // `BeginCell` until that lookahead has classified the next token.
        // In particular, a recovered preamble may be followed directly
        // by `}`, which `align_peek` passes to `fin_align`.
        debug_assert!(
            active_alignment
                .as_ref()
                .is_some_and(|active| active.identity == *alignment && !active.columns.is_empty())
        );
        // TeX82 §774 closes `init_align` with a second
        // `new_save_level(align_group)`, the level that brackets one
        // alignment *entry*. §791's `fin_col` replaces it at every `&`,
        // `\\span`-free column end, and `\\cr`, so an assignment made in a
        // cell -- `\\bf`, `\\tt`, a `\\fam`, any local register -- is
        // restored before the next entry begins. §800's first `unsave`
        // removes the last one.
        enter_group(
            stores,
            command.state,
            command.diagnostic_effects,
            GroupKind::Align,
        );
        // §774 then runs
        // `if every_cr<>null then begin_token_list(every_cr,every_cr_text)`
        // before its own `align_peek`, exactly as §799 does at every later
        // row boundary. The push follows the entry save level, so the hook
        // is scoped to the entry it opens.
        schedule_everycr(command.state, stores);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_alignment_cell_opening<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    alignment: &mut AlignmentIdentity,
    opening: &mut AlignmentCellOpening,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        command
            .state
            .apply_alignment_request(
                &**stores,
                match opening {
                    AlignmentCellOpening::Template => {
                        AlignmentRequest::InstallCellTemplate(*alignment)
                    }
                    AlignmentCellOpening::Omit => {
                        AlignmentRequest::InstallOmitCellTemplate(*alignment)
                    }
                },
            )
            .map_err(|_| ExecError::MissingToken {
                context: "alignment cell-template lifecycle",
            })?;
        if let Some(active) = active_alignment.as_mut()
            && active.identity == *alignment
        {
            active.cell_opening_pending = false;
            active.next_cell_opening_pending = false;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_alignment_cell_finish<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    alignment: &mut AlignmentIdentity,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if matches!(
            modes.current_mode(),
            Mode::Horizontal | Mode::RestrictedHorizontal
        ) {
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
        }
        let mut processor = command.processor(stores);
        let finished = processor
            .finish_alignment_cell(*alignment)
            .map_err(command_error);
        drop(processor);
        let finished = finished?;
        begin_next_replay_alignment_cell(
            *alignment,
            finished.delimiter,
            finished.delimiter_line,
            command,
            active_alignment,
            modes,
            stores,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_alignment_finish<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    alignment: &mut AlignmentIdentity,
    current_line: &mut u32,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if active_alignment.as_ref().map(|active| active.identity) != Some(*alignment) {
            return Err(ExecError::MissingToken {
                context: "active replay alignment",
            });
        }
        // TeX82 §800's `fin_align` opens with two `unsave`s -- "that
        // |align_group| was for individual entries", then "that
        // |align_group| was for the whole alignment" -- before it
        // determines the column widths and packages the prototype box.
        leave_fin_align_save_level(command, stores, "align1")?;
        leave_fin_align_save_level(command, stores, "align0")?;
        let active = active_alignment
            .as_mut()
            .expect("active replay alignment was checked");
        let error_context = command.state.output_open_context(&**stores);
        let mut geometry = pack_geometry_sink(command.state, command.observations);
        finish_replay_alignment(
            active,
            modes,
            stores,
            command.diagnostic_effects,
            &mut geometry,
            command.fuel,
            AlignmentFinishSite::new(&error_context, *current_line),
        )?;
        command
            .state
            .apply_alignment_request(&**stores, AlignmentRequest::Finish(*alignment))
            .map_err(|_| ExecError::MissingToken {
                context: "alignment finish lifecycle",
            })?;
        *active_alignment = None;
        if let Some(outer) = boxes.suspended_alignments.pop() {
            command
                .state
                .apply_alignment_request(&**stores, AlignmentRequest::Resume(outer.identity))
                .map_err(|_| ExecError::MissingToken {
                    context: "nested alignment resumption",
                })?;
            *active_alignment = Some(outer);
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_begin_no_align<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    alignment: &mut AlignmentIdentity,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let active = active_alignment
            .as_mut()
            .filter(|active| active.identity == *alignment)
            .ok_or(ExecError::MissingToken {
                context: "active replay alignment",
            })?;
        active.align_peek_pending = false;
        active.noalign_open = true;
        enter_group(
            stores,
            command.state,
            command.diagnostic_effects,
            GroupKind::NoAlign,
        );
        // TeX82 §785 leaves the alignment's own mode level in place when
        // `\noalign` opens. It calls `normal_paragraph` only for an
        // h-alignment's internal-vertical mode; a v-alignment is already
        // in restricted horizontal mode, but that level is the alignment
        // list itself, not a paragraph to pop.
        if modes.current_mode() == Mode::InternalVertical {
            crate::paragraph_end::normal_paragraph(modes, stores, command.diagnostic_effects);
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_alignment_peek_cell<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    alignment: &mut AlignmentIdentity,
    omit: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let active = active_alignment
            .as_mut()
            .filter(|active| active.identity == *alignment)
            .ok_or(ExecError::MissingToken {
                context: "active replay alignment",
            })?;
        let templates =
            active
                .columns
                .get(active.column)
                .cloned()
                .ok_or(ExecError::MissingToken {
                    context: "next alignment preamble column",
                })?;
        command
            .state
            .begin_prepared_alignment_cell(*alignment, templates)
            .map_err(|_| ExecError::MissingToken {
                context: "alignment next-row lifecycle",
            })?;
        begin_replay_alignment_cell(active, modes, stores, command.diagnostic_effects)?;
        active.align_peek_pending = false;
        if *omit {
            command
                .state
                .apply_alignment_request(
                    &**stores,
                    AlignmentRequest::PrepareCellLookahead(*alignment),
                )
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment omit lookahead lifecycle",
                })?;
            command
                .state
                .apply_alignment_request(
                    &**stores,
                    AlignmentRequest::InstallOmitCellTemplate(*alignment),
                )
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment omit-cell lifecycle",
                })?;
        } else {
            // TeX82 §37 now calls `init_col`, which immediately pushes
            // the selected u-template above the command backed up by
            // `align_peek`. A second lookahead would re-deliver that
            // command before the template is installed.
            command
                .state
                .apply_alignment_request(
                    &**stores,
                    AlignmentRequest::InstallCellTemplate(*alignment),
                )
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-row cell-template lifecycle",
                })?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_no_align_end_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    alignment: &mut AlignmentIdentity,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let active = active_alignment
            .as_mut()
            .filter(|active| active.identity == *alignment)
            .ok_or(ExecError::MissingToken {
                context: "active replay alignment",
            })?;
        if !active.noalign_open {
            return Err(ExecError::MissingToken {
                context: "noalign group",
            });
        }
        active.noalign_open = false;
        active.align_peek_pending = true;
        active.align_peek_after_noalign = true;
        // TeX82 §1133's whole `no_align_group` case of `handle_right_brace`
        // is `end_graf; unsave; align_peek`. A `\noalign` body is ordinary
        // internal vertical material, so anything horizontal in it (a
        // character, an `\hskip`, an `\indent`) starts a paragraph through
        // §1090 exactly as it would anywhere else in vertical mode, and the
        // closing brace is what line-breaks it back onto the alignment's own
        // vertical list. Without `end_graf` the paragraph stayed open across
        // the brace, so the following rows were built on the horizontal
        // level and `fin_align` popped that level instead of the alignment
        // (`umber2-usol`).
        let diagnostic_context =
            command_diagnostic_context(command.state, command.output_routine_active);
        let mut geometry = pack_geometry_sink(command.state, command.observations);
        crate::paragraph_end::end_paragraph_with_fuel(
            modes,
            stores,
            command.diagnostic_effects,
            &mut geometry,
            diagnostic_context,
            command.fuel,
        )?;
        unsave_group(stores, command, GroupKind::NoAlign).map_err(|_| ExecError::MissingToken {
            context: "noalign group",
        })?;
        Ok(ReplayStep::Continue)
    }
}
