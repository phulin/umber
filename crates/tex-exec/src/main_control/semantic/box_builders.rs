//! Native box construction and group closure.
use super::super::*;

pub(in crate::main_control) fn execute_illegal_box_shift<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let context = command.state.output_open_context(&**stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_box_end_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    ships_out: &mut bool,
    current_line: &mut i32,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // `active_boxes` is the move-only counterpart of TeX82 §1086's
        // saved box context. Keep it resident until every fallible part of
        // `package` has committed: resource suspension rolls back the
        // universe and mode nest, but this replay sidecar intentionally
        // lives outside those transactional roots. Popping it before a
        // pending character run requests its first format-restored font
        // would make the retained `BoxEndGroup` retry consume the next
        // enclosing box and eventually classify the original `}` as an
        // extra right brace.
        let (box_kind, group_kind, packing, leader_kind, shift, has_target) = boxes
            .active_boxes
            .last()
            .map(|box_state| {
                (
                    box_state.kind,
                    box_state.group_kind,
                    box_state.packing,
                    box_state.leader_kind,
                    box_state.shift,
                    box_state.target.is_some(),
                )
            })
            .ok_or(ExecError::MissingToken {
                context: "box group",
            })?;
        if let ReplayBoxKind::Insert(class, pre) = box_kind {
            let step = finish_insert_or_adjust_group(class, pre, modes, stores, command)?;
            let inserted = boxes
                .pop_box()
                .expect("successful insert completion retains its active owner");
            if let Some(start) = inserted.migration_segment_start {
                let segment = stores.close_page_box_segment(start);
                if !segment.is_empty() {
                    boxes
                        .active_boxes
                        .last_mut()
                        .expect("isolated insertion has an enclosing hbox")
                        .migration_segments
                        .push(segment);
                }
            }
            return Ok(step);
        }
        // TeX82 §1085's `handle_right_brace` runs `end_graf` (§1096) for
        // `vbox_group` and `vtop_group` -- and only for those two -- before
        // `package`: `hbox_group` and `adjusted_hbox_group` package
        // immediately. A vertical box whose body still has a paragraph open
        // when its closing brace arrives must therefore line-break that
        // paragraph into the box's own vertical list first. Without this,
        // `modes.pop()` below took the still-open *horizontal* level for the
        // box body and packaged its hlist material directly, so
        // `\vbox{\noindent A}` produced `\vbox(0.0+0.0)x0.0` holding a bare
        // char node -- and left the box's real internal-vertical level open
        // on the mode nest (`umber2-johp.232`).
        if !box_kind.horizontal() {
            let mut diagnostic_context =
                command_diagnostic_context(command.state, command.output_routine_active);
            // The closing brace can exhaust and retire its source before
            // this cold operation is applied. TeX82 §661 nevertheless
            // uses the brace's live `line` as the ending line of every
            // paragraph pack report, so retain that delivered scalar in
            // `BoxEndGroup` instead of re-reading a now-empty input stack.
            diagnostic_context.current_line = *current_line;
            let mut geometry = pack_geometry_sink(command.state, command.observations);
            crate::paragraph_end::end_paragraph_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                &mut geometry,
                diagnostic_context,
                command.fuel,
            )?;
        }
        // TeX82's main-control loop appends every character (and its
        // resolved ligature/kern chain) to the current list synchronously
        // as it is scanned, so by the time `handle_right_brace` reaches
        // `package` (§1086) to `hpack`/`vpack` a finished box, the list
        // is already complete. Umber batches a run of pending horizontal
        // characters (for ligature/kerning/shaping) in
        // `ModeList::pending_hchars` rather than materializing nodes
        // immediately, so any box-body list a `}` is about to freeze must
        // first flush that batch -- exactly like every other site that
        // treats a list as finished (`execute_discretionary_part`,
        // `capture_replay_alignment_cell`, `finish_replay_alignment_row`).
        // Without this, a box whose body ends in a bare character run
        // with no trailing glue/kern/space to force an earlier flush
        // (e.g. `\hbox{c}`, or plain.tex's `\setbox\z@\hbox{#1}` inside
        // `\c`) silently packages an empty list: the pending characters
        // are dropped along with the popped mode level instead of ever
        // becoming node.

        let mut level = crate::box_runtime::commit_current_list(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        let children = level.list_mutation().take_nodes();
        // TeX82 §1086 snapshots `d:=box_max_depth` before `unsave`.
        // The box body may assign a local, signed `\boxmaxdepth`; that
        // value governs this package operation even though the assignment
        // is restored before `vpackage` runs.
        let box_max_depth = stores.dimen_param(DimenParam::BOX_MAX_DEPTH);
        // e-TeX 2.6 [23.328]'s `group_warning` runs immediately before
        // every `unsave`, including §1086's hbox/vbox packaging path.
        // Keeping the hook here preserves save-stack order when one
        // nested source closes both a box group and a conditional.
        warn_cross_file_group_close(stores, command);
        unsave_group(stores, command, group_kind).map_err(|_| ExecError::MissingToken {
            context: "box group",
        })?;
        // TeX82 §1086 restores the box group before it calls `hpack` or
        // `vpack`. Besides putting §283's tracing-restores lines ahead of
        // §660/§674 diagnostics, this makes the enclosing h/v badness,
        // fuzz, and overfull-rule parameters authoritative. Max depth is
        // the exception: `package` saved it above before `unsave`.
        let node = if box_kind.horizontal() {
            let diagnostic_context = command_pack_context(command);
            let mut geometry = pack_geometry_sink(command.state, command.observations);
            Node::HList(crate::box_runtime::hpack_with_overfull_rule(
                stores,
                command.diagnostic_effects,
                &mut geometry,
                &diagnostic_context,
                children,
                packing,
            ))
        } else {
            Node::VList(match box_kind {
                ReplayBoxKind::VBox | ReplayBoxKind::VCenter => {
                    let diagnostic_context = command_pack_context(command);
                    let mut params = crate::packing_params::vpack_params(stores);
                    params.box_max_depth = box_max_depth;
                    let mut geometry = pack_geometry_sink(command.state, command.observations);
                    crate::packing_params::vpack(
                        stores,
                        command.diagnostic_effects,
                        &mut geometry,
                        &diagnostic_context,
                        children,
                        packing,
                        params,
                    )
                    .node
                }
                ReplayBoxKind::VTop => {
                    let diagnostic_context = command_pack_context(command);
                    let mut params = crate::packing_params::vpack_params(stores);
                    params.box_max_depth = box_max_depth;
                    let mut geometry = pack_geometry_sink(command.state, command.observations);
                    crate::packing_params::vtop(
                        stores,
                        command.diagnostic_effects,
                        &mut geometry,
                        &diagnostic_context,
                        children,
                        packing,
                        params,
                    )
                    .node
                }
                ReplayBoxKind::HBox => unreachable!("horizontal box was handled above"),
                ReplayBoxKind::Insert(_, _) => unreachable!(
                    "insert/adjust bodies return through finish_insert_or_adjust_group above"
                ),
            })
        };
        // TeX82 §1168's `vcenter_group` case of `handle_right_brace`:
        //
        //     vcenter_group: begin end_graf; unsave; save_ptr:=save_ptr-2;
        //       p:=vpack(link(head),saved(1),saved(0)); pop_nest;
        //       tail_append(new_noad); type(tail):=vcenter_noad;
        //       math_type(nucleus(tail)):=sub_box; info(nucleus(tail)):=p;
        //       end;
        //
        // The packaged box becomes a `vcenter_noad` nucleus on the
        // enclosing mlist. It never reaches §1075's `box_end`: §1073's
        // `scan_box` admits only `cur_cmd=make_box`, so a `\vcenter` can
        // be neither a `\setbox` target, a `\shipout` operand, a leader
        // payload, nor a `\raise`/`\lower` operand, and the whole box
        // context every other branch below classifies is inapplicable.
        if box_kind == ReplayBoxKind::VCenter {
            let boxed = stores.publish_page_nodes(vec![node]);
            modes.current_list_mutation().push(
                stores,
                Node::MathNoad(MathNoad::new(NoadKind::VCenter, MathField::SubBox(boxed))),
            );
            boxes
                .pop_box()
                .expect("successful vcenter completion retains its active owner");
            return Ok(ReplayStep::Continue);
        }
        if let Some(kind) = leader_kind {
            let payload =
                crate::box_runtime::payload_from_node(node).ok_or(ExecError::MissingToken {
                    context: "leader box payload",
                })?;
            boxes.pending_leader = Some((kind, payload));
            boxes
                .pop_box()
                .expect("successful leader completion retains its active owner");
        } else if *ships_out {
            let box_state = boxes
                .pop_box()
                .expect("successful shipout packaging retains its active owner");
            let region = box_state
                .shipout_region
                .expect("constructed shipout retains its page region");
            debug_assert!(command.prepared_shipout.is_none());
            *command.prepared_shipout = Some(PreparedShipout {
                source: PreparedShipoutSource::Page(node),
                region: Some(region),
            });
        } else if has_target {
            let target = boxes
                .pop_box()
                .expect("successful setbox packaging retains its active owner")
                .target
                .expect("setbox completion retains its target");
            let boxed = stores.publish_page_nodes(vec![node]);
            commit_set_box_target(target, Some(boxed), stores, command);
        } else {
            // TeX82 §1076's `box_end` branch for an ordinary
            // (non-register, non-shipout, non-leader) box appends the
            // freshly built box to whatever list is currently open,
            // exactly like `\box<n>` (`box_end`'s `BoxContext::Append`
            // above): baseline-skip insertion, migration extraction, and
            // (in outer vertical mode) page-builder contribution all
            // apply. A bare `modes.current_list_mutation().push(node)` here
            // bypassed all of that, silently dropping every standalone
            // `\hbox`/`\vbox`/`\vtop` (and macros built on them, such as
            // plain.tex's `\centerline`) appended directly in vertical
            // mode: the node landed in the mode-nest list rather than the
            // page contribution list the page builder actually drains.
            //
            // TeX82 §1073's box-shift prefixes (`\raise`/`\lower`/
            // `\moveleft`/`\moveright`) reach exactly this branch: their
            // wrapped `\hbox`/`\vbox`/`\vtop` can never itself be a
            // `\setbox` target, a `\shipout` operand, or a leader payload
            // (`scan_box`'s `cur_cmd=make_box` requirement excludes
            // `vmove`/`hmove`), so `box_state.shift` is only ever set
            // here.
            let mut node = node;
            if let Some(shift) = shift {
                crate::box_runtime::apply_box_shift_delta(&mut node, shift.delta)?;
            }
            let box_state = boxes
                .pop_box()
                .expect("successful box append retains its active owner");
            if let Some(start) = box_state.box_segment_start {
                crate::box_runtime::append_box_node_with_segment(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                    node,
                    start,
                    box_state.migration_segments,
                )?;
            } else {
                crate::box_runtime::append_box_node_to_current_list(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    node,
                    command.fuel,
                )?;
            }
            crate::vertical::build_page_if_outer_vertical(
                modes,
                stores,
                command.diagnostic_effects,
                command.state.state(),
            )?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_begin_shipout<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    boxes: &mut ReplayBoxes<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        boxes.pending_shipout = Some(stores.begin_page_node_region());
        Ok(ReplayStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
pub(in crate::main_control) fn execute_set_box<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    active_discretionaries: &[ActiveDiscretionary],
    active_math_choices: &[usize],
    active_math_fields: &mut [ActiveMathFieldTarget],
    prepared_dvi_pages: &mut PreparedDviPages,
    target: &mut SetBoxTarget,
    path: &mut ScannedSetBoxPath,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // §1214's `<Adjust for the setting of \globaldefs>` runs inside
        // `prefixed_command`, before §1241 scans the box, so `global` in
        // §1241's `if global then n:=256+cur_val` is the *effective*
        // scope. Resolving it at `box_end` instead would read
        // `\globaldefs` as the box body left it.
        // TeX82 §§1074 and 1077 remove `\lastbox` from the current
        // list before storing that box in the destination register. Keep
        // the same ownership boundary: the source-list rewrite must
        // settle before the destination construction suffix opens, or
        // sealing that suffix would transfer the live source descriptor
        // into durable box ownership along with the selected box.
        if let ScannedSetBoxPath::Payload(ScannedBoxShiftPayload::LastBox { error_context }) = path
        {
            let state = &*command.state;
            let removed: Option<crate::box_runtime::RemovedLastBox> =
                crate::box_runtime::take_last_box_with_segment(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                    |stores| render_diagnostic_context(state, stores, **error_context),
                )?;
            if let Some(removed) = &removed
                && let (Some(root), Some(metadata)) =
                    (removed.root, removed.migration_metadata.as_ref())
                && stores.can_transfer_interleaved_page_box(root, metadata, removed.shift_reset)
            {
                commit_interleaved_set_box_target(
                    PendingSetBox {
                        target: *target,
                        region: stores.begin_page_node_region(),
                        origin: tex_state::page_node_arena::BuiltBoxOrigin::LastBox,
                    },
                    root,
                    metadata.clone(),
                    removed.shift_reset,
                    stores,
                    command,
                );
                return Ok(ReplayStep::Continue);
            }
            let node = removed.map(|removed| removed.node);
            boxes.pending_setbox = Some(PendingSetBox {
                target: *target,
                region: stores.begin_page_node_region(),
                origin: tex_state::page_node_arena::BuiltBoxOrigin::LastBox,
            });
            let context = boxes.take_box_context(false);
            box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
            return Ok(ReplayStep::Continue);
        }

        boxes.pending_setbox = Some(PendingSetBox {
            target: *target,
            region: stores.begin_page_node_region(),
            origin: tex_state::page_node_arena::BuiltBoxOrigin::SetBoxConstruction,
        });
        match path {
            ScannedSetBoxPath::Forbidden { error_context } => {
                if let BoxContext::SetBox(pending) = boxes.take_box_context(false) {
                    stores
                        .release_page_node_region(pending.region)
                        .expect("rejected setbox releases its empty page region");
                }
                report_improper_setbox(
                    std::mem::take(error_context),
                    stores,
                    command.diagnostic_effects,
                )?;
            }
            ScannedSetBoxPath::Payload(payload) => match payload {
                ScannedBoxShiftPayload::Missing => {
                    if let BoxContext::SetBox(pending) = boxes.take_box_context(false) {
                        stores
                            .release_page_node_region(pending.region)
                            .expect("missing setbox payload releases its page region");
                    }
                    let context = command.state.output_open_context(&**stores);
                    report_improper_setbox(context, stores, command.diagnostic_effects)?;
                }
                ScannedBoxShiftPayload::BoxRegister { index, copy } => {
                    let moves = if *copy {
                        let target = &boxes
                            .pending_setbox
                            .as_ref()
                            .expect("setbox target remains pending")
                            .target;
                        target.index == *index
                            && stores.self_copy_moves_box_register(
                                *index,
                                assignment_scope(target.global),
                            )
                    } else {
                        stores.can_take_unique_box_source(*index)
                    };
                    if moves {
                        let pending = boxes
                            .pending_setbox
                            .take()
                            .expect("setbox target remains pending");
                        commit_unique_register_take_target(pending, *index, stores, command);
                        return Ok(ReplayStep::Continue);
                    }
                    if !*copy
                        && stores.can_assign_output_carrier_take(*index)
                        && boxes.active_boxes.is_empty()
                        && active_alignment.is_none()
                        && active_discretionaries.is_empty()
                        && active_math_choices.is_empty()
                        && active_math_fields.is_empty()
                        && let Some(modes) = modes.preflight_page_region_succession(stores)
                    {
                        let pending = boxes
                            .pending_setbox
                            .take()
                            .expect("setbox target remains pending");
                        match commit_output_carrier_take_target(
                                pending, modes, stores, command,
                            ) {
                                Ok(()) => return Ok(ReplayStep::Continue),
                                Err(declined) => match *declined {
                                    (pending, tex_state::page_node_arena::OutputCarrierTakeError::UnsupportedGeometry) => {
                                        boxes.pending_setbox = Some(pending);
                                    }
                                    (_pending, tex_state::page_node_arena::OutputCarrierTakeError::Promotion(_error)) => {
                                        return Err(ExecError::Fatal(FatalError::confusion(
                                            "output carrier transfer",
                                        )));
                                    }
                                },
                            }
                    }
                    boxes
                        .pending_setbox
                        .as_mut()
                        .expect("setbox target remains pending")
                        .origin = if *copy {
                        tex_state::page_node_arena::BuiltBoxOrigin::SetBoxRegisterCopy
                    } else {
                        #[cfg(feature = "profiling")]
                        {
                            let mode_roots_clear =
                                modes.preflight_page_region_succession(stores).is_some();
                            stores.profile_box_register_take_decline(
                                *index,
                                mode_roots_clear,
                                boxes.active_boxes.is_empty(),
                            )
                        }
                        #[cfg(not(feature = "profiling"))]
                        {
                            tex_state::page_node_arena::BuiltBoxOrigin::SetBoxRegisterTake
                        }
                    };
                    let id = read_box_register(*index, *copy, stores, command);
                    let node = crate::box_runtime::first_box_node(stores, id);
                    let context = boxes.take_box_context(false);
                    box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
                }
                ScannedBoxShiftPayload::LastBox { .. } => {
                    unreachable!("setbox lastbox is ordered before its destination suffix")
                }
                ScannedBoxShiftPayload::VSplit(split) => {
                    boxes
                        .pending_setbox
                        .as_mut()
                        .expect("setbox target remains pending")
                        .origin = tex_state::page_node_arena::BuiltBoxOrigin::SetBoxVSplit;
                    if let Some(context) = &split.missing_to_context {
                        report_missing_vsplit_to(context, command.diagnostic_effects, stores)?;
                    }
                    let diagnostic_context =
                        command_diagnostic_context(command.state, command.output_routine_active);
                    let mut geometry = pack_geometry_sink(command.state, command.observations);
                    let node = crate::box_runtime::split_vbox_register(
                        stores,
                        command.diagnostic_effects,
                        &mut geometry,
                        &diagnostic_context,
                        split.index,
                        split.height,
                        &split.split_context,
                    )?;
                    let context = boxes.take_box_context(false);
                    box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
                }
                ScannedBoxShiftPayload::Construction(construction) => begin_replay_box(
                    *construction,
                    boxes.pending_setbox.take(),
                    None,
                    modes,
                    stores,
                    boxes,
                    command,
                )?,
            },
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_begin_box<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    construction: &mut ScannedBoxConstruction,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let target = boxes.pending_setbox.take();
        let shipout_region = boxes.pending_shipout.take();
        begin_replay_box(
            *construction,
            target,
            shipout_region,
            modes,
            stores,
            boxes,
            command,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_box_shift<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    shift: &mut ScannedBoxShift,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    apply_box_shift(shift, command, modes, stores, boxes)
}
