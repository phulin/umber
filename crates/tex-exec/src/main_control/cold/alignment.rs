//! Cold alignment lifecycle and diagnostic projection.

use super::super::*;
use super::apply::enter_group;
use super::support::*;

pub(in crate::main_control) fn begin_next_replay_alignment_cell<G>(
    alignment: AlignmentIdentity,
    delimiter: AlignmentCellDelimiter,
    delimiter_line: u32,
    command: &mut CommandMachine<'_, G>,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    modes: &mut ModeNest,
    stores: &mut LinearCommandContext<'_, G>,
) -> Result<(), ExecError> {
    let active = active_alignment
        .as_mut()
        .filter(|active| active.identity == alignment)
        .ok_or(ExecError::MissingToken {
            context: "active replay alignment",
        })?;
    // Focused lifecycle tests may construct a command-state cell directly,
    // without replaying a preamble.  There is then no executor template
    // selection to perform after the otherwise complete command transition,
    // and no §774 entry save level to replace either.
    if active.columns.is_empty() {
        return Ok(());
    }
    // TeX82 §1131 runs `end_graf` before §791's `fin_col`, even when the
    // saved delimiter is `span_code` and `fin_col` keeps the current span
    // list open. A valign entry can therefore leave horizontal mode before
    // the following column starts without packaging the spanning cell yet.
    if active.kind == AlignmentKind::VAlign {
        let mut error_context = crate::diagnostics::ExecutionDiagnosticContext::source_free(
            command.state.output_open_context(stores),
        );
        // The live delimiter was intercepted before the v-template started;
        // synthetic `endv` has no source line of its own. TeX82 §§1131/661
        // nevertheless report the delimiter line as this paragraph's end.
        error_context.current_line = i32::try_from(delimiter_line).unwrap_or(i32::MAX);
        let mut geometry = pack_geometry_sink(command.state, command.observations);
        crate::paragraph_end::end_paragraph_with_context(
            modes,
            stores,
            command.diagnostic_effects,
            &mut geometry,
            command.fuel,
            error_context,
        )?;
    }
    if delimiter == AlignmentCellDelimiter::Span {
        active.cell_span = active
            .cell_span
            .checked_add(1)
            .ok_or(ExecError::ArithmeticOverflow)?;
    } else {
        let mut geometry = pack_geometry_sink(command.state, command.observations);
        capture_replay_alignment_cell(
            active,
            modes,
            stores,
            command.diagnostic_effects,
            &mut geometry,
            command.fuel,
        )?;
    }
    let next_column = match delimiter {
        AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => active
            .column
            .checked_add(1)
            .ok_or(ExecError::ArithmeticOverflow)?,
        AlignmentCellDelimiter::Row => 0,
    };
    let extra_tab_recovery = next_column >= active.columns.len()
        && active.repeat_start.is_none()
        && matches!(
            delimiter,
            AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span
        );
    if extra_tab_recovery {
        report_extra_alignment_tab(command.state, stores)?;
    }
    // TeX82 §791's `if extra_info(cur_align)<>span_code then begin unsave;
    // new_save_level(align_group)`: every entry that does not continue through
    // `\span` replaces the §774 entry save level, discarding the cell's local
    // assignments. §792's extra-tab recovery rewrites `extra_info` to
    // `cr_code` *before* that test, so a `\span` whose column does not exist
    // ends the entry -- and its save level -- after all.
    //
    // §791 unsaves before `@<Package an unset box for the current column@>`;
    // here the packaging (`capture_replay_alignment_cell`) runs first because
    // it also flushes the cell's pending characters, which TeX had already
    // appended with the in-cell `cur_font`. `hpack`/`vpackage` at natural size
    // read no restorable parameter, so the two orders agree.
    if delimiter != AlignmentCellDelimiter::Span || extra_tab_recovery {
        replace_alignment_entry_save_level(command, stores)?;
    }
    if extra_tab_recovery {
        let recovered = command
            .state
            .apply_alignment_request(stores, AlignmentRequest::RecoverExtraTab(alignment))
            .map_err(|_| ExecError::MissingToken {
                context: "alignment extra-tab recovery",
            })?;
        debug_assert!(matches!(
            recovered,
            AlignmentRequestResult::ExtraTabRecovered
        ));
        let mut geometry = pack_geometry_sink(command.state, command.observations);
        finish_replay_alignment_row(
            active,
            modes,
            stores,
            command.diagnostic_effects,
            &mut geometry,
            command.fuel,
        )?;
        active.column = 0;
        // §792's extra-tab recovery rewrites `extra_info` to `cr_code`, so
        // §791's `fin_col` returns true and §1131's `do_endv` runs `fin_row`
        // here too -- including its `\everycr` push.
        schedule_everycr(command.state, stores);
        active.align_peek_pending = true;
        return Ok(());
    }
    active.column = if next_column < active.columns.len() {
        next_column
    } else if let Some(repeat_start) = active.repeat_start {
        let repeat_len =
            active
                .columns
                .len()
                .checked_sub(repeat_start)
                .ok_or(ExecError::MissingToken {
                    context: "alignment periodic-preamble boundary",
                })?;
        if repeat_len == 0 {
            return Err(ExecError::MissingToken {
                context: "alignment periodic-preamble columns",
            });
        }
        repeat_start + (next_column - repeat_start) % repeat_len
    } else {
        next_column
    };
    let templates = active
        .columns
        .get(active.column)
        .cloned()
        .ok_or(ExecError::MissingToken {
            context: "next alignment preamble column",
        })?;
    match delimiter {
        AlignmentCellDelimiter::Row => {
            let mut geometry = pack_geometry_sink(command.state, command.observations);
            finish_replay_alignment_row(
                active,
                modes,
                stores,
                command.diagnostic_effects,
                &mut geometry,
                command.fuel,
            )?;
            // TeX82 §799 `fin_row` closes with
            // `if every_cr<>null then begin_token_list(every_cr,every_cr_text);
            // align_peek`, so the hook is installed before the lookahead that
            // starts the next row reads a token.
            schedule_everycr(command.state, stores);
            active.align_peek_pending = true;
        }
        AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => {
            command
                .state
                .begin_prepared_alignment_cell(alignment, templates)
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-cell lifecycle",
                })?;
            if delimiter == AlignmentCellDelimiter::Tab {
                begin_replay_alignment_cell(active, modes, stores, command.diagnostic_effects)?;
            }
            active.next_cell_opening_pending = true;
        }
    }
    Ok(())
}

/// TeX82 §792's exhausted-preamble diagnostic for a saved tab or span.
pub(in crate::main_control) fn report_extra_alignment_tab<G>(
    command: &CommandState<G>,
    stores: &mut tex_state::CommandContext<'_, G>,
) -> Result<(), ExecError> {
    let context = command.output_open_context(stores);
    crate::error_report::report_error(
        stores,
        "Extra alignment tab has been changed to \\cr",
        &[
            "You have given more \\span or & marks than there were",
            "in the preamble to the \\halign or \\valign now in progress.",
            "So I'll assume that you meant to type \\cr instead.",
        ],
        context,
    )
}

/// Applies TeX82 §791 `fin_col`'s `unsave; new_save_level(align_group)`.
///
/// The pair is what makes an alignment entry a scope: assignments a cell makes
/// -- a font selection such as plain.tex's `\bf`, a `\fam`, any local register
/// -- must not survive the `&` or `\cr` that ends it. §1063's `unsave` also
/// releases the level's `\aftergroup` tokens, so they are backed up here just
/// as every other canonical group exit does.
pub(in crate::main_control) fn replace_alignment_entry_save_level<G>(
    command: &mut CommandMachine<'_, G>,
    stores: &mut LinearCommandContext<'_, G>,
) -> Result<(), ExecError> {
    let aftergroup = leave_alignment_save_level(
        command.state,
        command.diagnostic_effects,
        stores,
        "alignment entry group",
    )?;
    enter_group(
        stores,
        command.state,
        command.diagnostic_effects,
        GroupKind::Align,
    );
    schedule_aftergroup(command, stores, aftergroup)
}

/// One of TeX82 §800 `fin_align`'s `unsave`s, or §791's.
pub(in crate::main_control) fn leave_alignment_save_level<G>(
    command: &mut PersistentInterpreter<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    stores: &mut tex_state::CommandContext<'_, G>,
    context: &'static str,
) -> Result<Vec<tex_state::token::TracedTokenWord>, ExecError> {
    leave_group_payloads(stores, command, diagnostic_effects, GroupKind::Align)
        .map_err(|_| ExecError::MissingToken { context })
}

/// TeX82 §800's internal save-stack checks at the start of `fin_align`.
///
/// Unlike an ordinary group-closing command, reaching either check without
/// the expected `align_group` means the engine's own alignment state is
/// inconsistent. TeX therefore calls `confusion`, with a distinct site for
/// the entry level and the whole-alignment level.
pub(in crate::main_control) fn leave_fin_align_save_level<G>(
    command: &mut PersistentInterpreter<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    stores: &mut tex_state::CommandContext<'_, G>,
    confusion_site: &'static str,
) -> Result<Vec<tex_state::token::TracedTokenWord>, ExecError> {
    leave_group_payloads(stores, command, diagnostic_effects, GroupKind::Align)
        .map_err(|_| ExecError::Fatal(FatalError::confusion(confusion_site)))
}

pub(in crate::main_control) fn replay_alignment_mode(kind: AlignmentKind) -> Mode {
    match kind {
        AlignmentKind::HAlign => Mode::InternalVertical,
        AlignmentKind::VAlign => Mode::RestrictedHorizontal,
    }
}

pub(in crate::main_control) fn replay_alignment_row_mode(kind: AlignmentKind) -> Mode {
    match kind {
        // TeX82 §786 pushes the row level and exchanges the alignment's
        // internal-v/restricted-h mode. The following §787 span level keeps
        // that mode, so the row beneath an h-alignment cell is restricted
        // horizontal, not another internal-vertical level.
        AlignmentKind::HAlign => Mode::RestrictedHorizontal,
        AlignmentKind::VAlign => Mode::InternalVertical,
    }
}

pub(in crate::main_control) fn replay_alignment_cell_mode(kind: AlignmentKind) -> Mode {
    match kind {
        // TeX82 §768: `init_row` changes an \halign from internal vertical
        // to restricted horizontal mode, and §769's `init_span` preserves
        // that mode on the cell's fresh semantic level.
        AlignmentKind::HAlign => Mode::RestrictedHorizontal,
        AlignmentKind::VAlign => Mode::InternalVertical,
    }
}

pub(in crate::main_control) fn begin_replay_alignment_cell<G>(
    active: &mut ActiveReplayAlignment<G>,
    modes: &mut ModeNest,
    stores: &mut tex_state::CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
) -> Result<(), ExecError> {
    if !active.row_open {
        modes.push(replay_alignment_row_mode(active.kind))?;
        // TeX82 §786 clears the otherwise-unused row auxiliary explicitly;
        // §787 then gives the cell/span level its distinct canonical value.
        match active.kind {
            AlignmentKind::HAlign => modes.current_list_mutation().set_space_factor(0),
            AlignmentKind::VAlign => modes
                .current_list_mutation()
                .set_prev_depth(Scaled::from_raw(0)),
        }
        modes.current_list_mutation().push(Node::Glue {
            spec: active
                .tabskips
                .first()
                .cloned()
                .unwrap_or(active.default_tabskip),
            kind: GlueKind::TabSkip,
            leader: None,
        });
        active.captured_rows.push(Vec::new());
        active.row_open = true;
    }
    if active.cell_open {
        return Err(ExecError::MissingToken {
            context: "active replay alignment cell",
        });
    }
    modes.push(replay_alignment_cell_mode(active.kind))?;
    crate::align::init_span_aux(modes, stores, diagnostic_effects);
    active.cell_span = 1;
    active.cell_open = true;
    Ok(())
}

pub(in crate::main_control) fn capture_replay_alignment_cell<G>(
    active: &mut ActiveReplayAlignment<G>,
    modes: &mut ModeNest,
    stores: &mut tex_state::CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if !active.cell_open {
        return Ok(());
    }

    // Alignment packaging still defers that paragraph's lowering,
    // but §815's negative pretolerance makes its immediate transition into
    // the hyphenating pass certain. Publish §919's one-way trie lifecycle at
    // this boundary, before `align_peek` fetches what follows.
    if matches!(
        modes.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) && stores.int_param(IntParam::PRETOLERANCE) < 0
    {
        stores.close_hyphenation_patterns();
    }
    let mut cell =
        crate::box_runtime::commit_current_list(modes, stores, diagnostic_effects, fuel)?;
    let material = if active.kind == AlignmentKind::HAlign {
        // TeX82 §796 packs an `\halign` column with `adjust_tail:=cur_tail`,
        // so §651/§655 remove its insertions, marks, and `\vadjust` contents
        // from the column and hold them on the row's migration list; §799
        // appends them after the packaged row. A `\valign` column is
        // `vpackage`d with `adjust_tail` null and migrates nothing.
        let material = crate::math::finish_math_lists_owned(
            stores,
            diagnostic_effects,
            geometry,
            cell.list_mutation().take_nodes(),
            false,
        );
        let (retained, mut pre_migrated, migrated) =
            crate::box_runtime::split_hpack_migrations(stores, material);
        pre_migrated.extend(migrated);
        active.row_migrations.extend(pre_migrated);
        retained
    } else {
        cell.list_mutation().take_nodes()
    };
    let material = stores.publish_page_nodes(material);
    active
        .captured_rows
        .last_mut()
        .ok_or(ExecError::MissingToken {
            context: "active replay alignment row",
        })?
        .push(material);
    let cell = crate::align::packaging::make_unset_node(
        stores,
        diagnostic_effects,
        geometry,
        &crate::diagnostics::ExecutionDiagnosticContext::source_free("alignment cell"),
        material,
        crate::align::packaging::cell_unset_kind(active.kind),
        active.cell_span,
        crate::align::packaging::UnsetPackContext::Cell,
    )?;
    modes.current_list_mutation().push(cell);
    modes.current_list_mutation().push(Node::Glue {
        spec: active
            .tabskips
            .get(active.column.saturating_add(1))
            .cloned()
            .unwrap_or(active.default_tabskip),
        kind: GlueKind::TabSkip,
        leader: None,
    });
    active.cell_open = false;
    Ok(())
}

pub(in crate::main_control) fn finish_replay_alignment_row<G>(
    active: &mut ActiveReplayAlignment<G>,
    modes: &mut ModeNest,
    stores: &mut tex_state::CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    capture_replay_alignment_cell(active, modes, stores, diagnostic_effects, geometry, fuel)?;
    if !active.row_open {
        return Ok(());
    }

    let mut row = crate::box_runtime::commit_current_list(modes, stores, diagnostic_effects, fuel)?;
    let children = stores.publish_page_nodes(row.list_mutation().take_nodes());
    let row = crate::align::packaging::make_unset_node(
        stores,
        diagnostic_effects,
        geometry,
        &crate::diagnostics::ExecutionDiagnosticContext::source_free("alignment row"),
        children,
        crate::align::packaging::row_unset_kind(active.kind),
        1,
        crate::align::packaging::UnsetPackContext::Row,
    )?;
    // TeX82 §799's `fin_row`: `p:=hpack(link(head),natural,...); pop_nest;
    // append_to_vlist(p)`. The completed (still unset) row joins the
    // alignment's own vertical list through §679 `append_to_vlist`, so the
    // interline glue between two rows is the ordinary `\baselineskip`/
    // `\lineskip` decision against the running `prev_depth` -- not a bare
    // splice. §807's unset-to-set conversion changes only widths and glue
    // set, never a row's height or depth, so computing the glue here is
    // exactly what tex.web computes. A bare push produced rows stacked with
    // no interline glue at all, which is why plain's `\pmatrix`/`\matrix`/
    // `\cases`/`\eqalign`/`\halign` bodies came out short by one
    // `\baselineskip` per row (`umber2-johp.260`).
    match active.kind {
        AlignmentKind::HAlign => {
            crate::vertical::append_node_to_vertical_list(modes, stores, row)?;
        }
        AlignmentKind::VAlign => {
            // TeX82 §799's other branch is a plain horizontal splice:
            // `link(tail):=p; tail:=p; space_factor:=1000`. A valign row
            // must not pass through §679's vertical baseline calculation;
            // doing so inserts baselineskip between rows, and a surrounding
            // hpack then counts that vertical glue as horizontal cell width.
            modes.current_list_mutation().push(row);
            modes.current_list_mutation().set_space_factor(1000);
        }
    }
    // §799 continues `if cur_head<>cur_tail then begin link(tail):=link(cur_head);
    // tail:=cur_tail end`: the migrated material is spliced immediately after the
    // row, as a plain list splice with no interline glue of its own.
    for node in std::mem::take(&mut active.row_migrations) {
        crate::vertical::append_vertical_contribution(modes, stores, node);
    }
    active.row_open = false;
    Ok(())
}

/// Carries TeX82 §645's `spec_code`/`cur_val` pair from the command-owned
/// `scan_spec` to the alignment state §805 packs the prototype box with.
pub(in crate::main_control) fn alignment_pack_spec(
    packing: ScannedPackingSpec,
) -> AlignmentPackSpec {
    match packing {
        ScannedPackingSpec::Natural => AlignmentPackSpec::Natural,
        ScannedPackingSpec::Exactly(size) => AlignmentPackSpec::Exactly(size),
        ScannedPackingSpec::Spread(size) => AlignmentPackSpec::Spread(size),
    }
}

pub(in crate::main_control) struct AlignmentFinishSite<'a> {
    error_context: &'a str,
    current_line: u32,
}

impl<'a> AlignmentFinishSite<'a> {
    pub(in crate::main_control) const fn new(error_context: &'a str, current_line: u32) -> Self {
        Self {
            error_context,
            current_line,
        }
    }
}

pub(in crate::main_control) fn finish_replay_alignment<G>(
    active: &mut ActiveReplayAlignment<G>,
    modes: &mut ModeNest,
    stores: &mut tex_state::CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    fuel: &mut tex_command::CommandFuel,
    site: AlignmentFinishSite<'_>,
) -> Result<(), ExecError> {
    finish_replay_alignment_row(active, modes, stores, diagnostic_effects, geometry, fuel)?;
    let mut alignment =
        crate::box_runtime::commit_current_list(modes, stores, diagnostic_effects, fuel)?;
    // TeX82 §800 makes §661's box-diagnostic origin negative for the whole
    // `fin_align` setting pass. The magnitude is the alignment level's
    // `mode_line`, captured by §774's `push_nest`, and §812 restores the
    // enclosing diagnostic state after the finished alignment is appended.
    let diagnostic_context = crate::diagnostics::ExecutionDiagnosticContext::new(
        i32::try_from(site.current_line).unwrap_or(i32::MAX),
        -alignment.entry_line(),
        false,
        site.error_context,
    );
    finish_replay_alignment_with_origin(
        active,
        modes,
        stores,
        diagnostic_effects,
        geometry,
        &mut alignment,
        &diagnostic_context,
    )
}

pub(in crate::main_control) fn finish_replay_alignment_with_origin<G>(
    active: &ActiveReplayAlignment<G>,
    modes: &mut ModeNest,
    stores: &mut tex_state::CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    alignment: &mut crate::mode::ModeLevelSummary,
    diagnostic_context: &crate::diagnostics::ExecutionDiagnosticContext,
) -> Result<(), ExecError> {
    let rows = alignment.list_mutation().take_nodes();
    let columns = active
        .columns
        .iter()
        .map(|templates| AlignColumn {
            u_template: tex_state::node::NodeTokenList::new(
                stores
                    .token_list(
                        templates
                            .u_template
                            .expect("alignment columns retain u templates"),
                    )
                    .to_vec(),
            ),
            v_template: tex_state::node::NodeTokenList::new(
                stores.token_list(templates.v_template).to_vec(),
            ),
        })
        .collect();
    let state = AlignState::new(
        active.kind,
        active.packing,
        columns,
        active.tabskips.clone(),
        active.default_tabskip,
        active.repeat_start,
    );
    // TeX82 §800: `if nest[nest_ptr-1].mode_field=mmode then o:=display_indent
    // else o:=0`. The alignment level has just been popped, so the current mode
    // is the enclosing one §800 inspects.
    let offset = if modes.current_mode() == Mode::DisplayMath {
        stores.dimen_param(DimenParam::DISPLAY_INDENT)
    } else {
        Scaled::from_raw(0)
    };
    let finished = crate::align::widths::finish_alignment(
        &state,
        &rows,
        offset,
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
    )?;
    let aux_prev_depth = alignment.list().prev_depth();
    let aux_space_factor = matches!(
        alignment.mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    )
    .then(|| alignment.list().space_factor());
    if modes.current_mode() == Mode::DisplayMath {
        // Preserve §812's `(p,q,aux_save)` handoff until the closing `$$`
        // has run §§1206–1207's assignment and delimiter scan.
        modes
            .current_list_mutation()
            .set_display_alignment(finished, aux_prev_depth);
    } else {
        crate::align::append_finished_alignment(
            modes,
            stores,
            crate::align::FinishedAlignment {
                nodes: finished,
                aux_prev_depth,
                aux_space_factor,
            },
        );
    }
    crate::vertical::build_page_if_outer_vertical_with_error_context(
        modes,
        stores,
        diagnostic_effects,
        &diagnostic_context.output_context,
    )?;
    Ok(())
}

/// Resolves e-TeX 2.6 [49.1292]'s save-stack traversal and its retained live
/// construction contexts into an immutable diagnostic value without changing
/// any engine stack.
pub(in crate::main_control) fn detached_showgroups<G>(
    stores: &tex_state::CommandContext<'_, G>,
    active_alignment: &Option<ActiveReplayAlignment<G>>,
    boxes: &ReplayBoxes<G>,
    active_discretionaries: &[ActiveDiscretionary],
    active_math_choices: &[usize],
    active_math_left_boundaries: &[bool],
    active_math_shifts: &[MathShiftContext],
) -> crate::diagnostics::ShowGroupsDiagnostic {
    use crate::diagnostics::{ShowGroupFrame, ShowGroupsDiagnostic};

    let frames = stores.group_frames().to_vec();
    let mut alignment_contexts = alignment_group_contexts(&frames, active_alignment, boxes);
    let mut box_index = 0usize;
    let mut discretionary_index = 0usize;
    let mut math_choice_index = 0usize;
    let mut math_left_index = 0usize;
    let mut math_shift_index = 0usize;
    let mut rendered = Vec::with_capacity(frames.len());
    for (index, frame) in frames.into_iter().enumerate() {
        let kind = frame.kind();
        let context = match kind {
            GroupKind::Simple | GroupKind::Math => "{".to_owned(),
            GroupKind::SemiSimple => "\\begingroup".to_owned(),
            GroupKind::HBox
            | GroupKind::AdjustedHBox
            | GroupKind::VBox
            | GroupKind::VTop
            | GroupKind::VCenter
            | GroupKind::Insert => {
                let context = boxes
                    .active_boxes
                    .get(box_index)
                    .map_or_else(|| fallback_group_context(kind).to_owned(), show_box_context);
                box_index = box_index.saturating_add(1);
                context
            }
            // e-TeX [49.1292]'s `output_group` arm jumps directly to
            // `found`, whose closing parenthesis terminates the context.
            // Unlike braced source groups, the synthetic output group does
            // not pass through `found2` and therefore prints no `{`.
            GroupKind::Output => "\\output".to_owned(),
            GroupKind::Disc => {
                // e-TeX 2.6 [49.1292] prints one `{}` for each part already
                // completed, followed by `{` for the currently live part.
                let completed = active_discretionaries
                    .get(discretionary_index)
                    .map_or(0, |active| active.parts.len());
                discretionary_index = discretionary_index.saturating_add(1);
                format!("\\discretionary{}{{", "{}".repeat(completed))
            }
            GroupKind::MathChoice => {
                // The other half of e-TeX [49.1292]'s shared
                // `disc_group,math_choice_group` case uses §1174's saved
                // count in exactly the same way as a discretionary.
                let completed = active_math_choices
                    .get(math_choice_index)
                    .copied()
                    .unwrap_or(0);
                math_choice_index = math_choice_index.saturating_add(1);
                format!("\\mathchoice{}{{", "{}".repeat(completed))
            }
            GroupKind::MathShift => {
                // TeX82 §1176's mode test and §1177's saved equation-number
                // side are the semantic opener identity e-TeX [49.1292]
                // reconstructs while traversing the live groups.
                let context = match active_math_shifts.get(math_shift_index) {
                    Some(MathShiftContext::Display) => "$$",
                    Some(MathShiftContext::EqNo(crate::mode::EqNoSide::Left)) => "\\leqno",
                    Some(MathShiftContext::EqNo(crate::mode::EqNoSide::Right)) => "\\eqno",
                    Some(MathShiftContext::Inline) | None => "$",
                };
                math_shift_index = math_shift_index.saturating_add(1);
                context.to_owned()
            }
            GroupKind::MathLeft => {
                // e-TeX 2.6 [49.1292] distinguishes the consecutive
                // `math_left_group` segments opened by `\left` and `\middle`
                // using the delimiter identity [48.1191] retains in the mode
                // level's `eTeX_aux_field`.
                let opened_by_middle = active_math_left_boundaries
                    .get(math_left_index)
                    .copied()
                    .unwrap_or(false);
                math_left_index = math_left_index.saturating_add(1);
                if opened_by_middle {
                    "\\middle".to_owned()
                } else {
                    "\\left".to_owned()
                }
            }
            GroupKind::NoAlign => "\\noalign{".to_owned(),
            GroupKind::Align => alignment_contexts[index]
                .take()
                .unwrap_or_else(|| "align entry".to_owned()),
        };
        rendered.push(ShowGroupFrame {
            kind,
            level: index + 1,
            entered_line: frame.entered_line(),
            context,
        });
    }
    ShowGroupsDiagnostic { frames: rendered }
}

/// Replays e-TeX [49.1292]'s inward-to-outward alignment state (`a`) while
/// retaining the outermost-first frame order used by the detached diagnostic.
pub(in crate::main_control) fn alignment_group_contexts<G>(
    frames: &[tex_state::GroupFrame],
    active_alignment: &Option<ActiveReplayAlignment<G>>,
    boxes: &ReplayBoxes<G>,
) -> Vec<Option<String>> {
    let mut kinds = boxes
        .suspended_alignments
        .iter()
        .map(|alignment| alignment.kind)
        .chain(active_alignment.iter().map(|alignment| alignment.kind))
        .collect::<Vec<_>>();
    let mut alignment_state = 1i8;
    let mut contexts = vec![None; frames.len()];
    for (index, frame) in frames.iter().enumerate().rev() {
        match frame.kind() {
            GroupKind::NoAlign => alignment_state = -1,
            GroupKind::Align if alignment_state == 0 => {
                let kind = kinds.pop().unwrap_or(AlignmentKind::HAlign);
                contexts[index] = Some(match kind {
                    AlignmentKind::HAlign => "\\halign{".to_owned(),
                    AlignmentKind::VAlign => "\\valign{".to_owned(),
                });
                alignment_state = 1;
            }
            GroupKind::Align => {
                contexts[index] = Some(if alignment_state == 1 {
                    "align entry".to_owned()
                } else {
                    "\\cr".to_owned()
                });
                alignment_state = 0;
            }
            _ => {}
        }
    }
    contexts
}

pub(in crate::main_control) fn fallback_group_context(kind: GroupKind) -> &'static str {
    match kind {
        GroupKind::HBox | GroupKind::AdjustedHBox => "\\hbox{",
        GroupKind::VBox => "\\vbox{",
        GroupKind::VTop => "\\vtop{",
        GroupKind::VCenter => "\\vcenter{",
        GroupKind::Insert => "\\insert{",
        _ => "{",
    }
}

pub(in crate::main_control) fn show_box_context(active: &ActiveReplayBox) -> String {
    let mut context = String::new();
    if let Some(shift) = active.shift
        && shift.delta.raw() != 0
    {
        context.push_str(match (shift.axis, shift.delta.raw().is_negative()) {
            (BoxShiftAxis::Horizontal, true) => "\\moveleft",
            (BoxShiftAxis::Horizontal, false) => "\\moveright",
            (BoxShiftAxis::Vertical, true) => "\\raise",
            (BoxShiftAxis::Vertical, false) => "\\lower",
        });
        let magnitude = if shift.delta.raw().is_negative() {
            -shift.delta
        } else {
            shift.delta
        };
        context.push_str(&crate::node_dump::format_scaled_for_diagnostics(magnitude));
        context.push_str("pt");
    } else if let Some(target) = active.target {
        if target.global {
            context.push_str("\\global");
        }
        context.push_str("\\setbox");
        context.push_str(&target.index.to_string());
        context.push('=');
    } else if active.ships_out {
        context.push_str("\\shipout");
    } else if let Some(kind) = active.leader_kind {
        context.push_str(match kind {
            GlueKind::Leaders => "\\leaders",
            GlueKind::Cleaders => "\\cleaders",
            GlueKind::Xleaders => "\\xleaders",
            _ => "\\leaders",
        });
    }
    match active.kind {
        ReplayBoxKind::HBox => context.push_str("\\hbox"),
        ReplayBoxKind::VBox => context.push_str("\\vbox"),
        ReplayBoxKind::VTop => context.push_str("\\vtop"),
        ReplayBoxKind::VCenter => context.push_str("\\vcenter"),
        ReplayBoxKind::Insert(255, pre) => {
            context.push_str(if pre { "\\vadjust pre" } else { "\\vadjust" });
        }
        ReplayBoxKind::Insert(class, _) => {
            context.push_str("\\insert");
            context.push_str(&class.to_string());
        }
    }
    match active.packing {
        PackSpec::Natural => {}
        PackSpec::Exactly(size) => {
            context.push_str(" to");
            context.push_str(&crate::node_dump::format_scaled_for_diagnostics(size));
            context.push_str("pt");
        }
        PackSpec::Spread(size) => {
            context.push_str(" spread");
            context.push_str(&crate::node_dump::format_scaled_for_diagnostics(size));
            context.push_str("pt");
        }
    }
    context.push('{');
    context
}
