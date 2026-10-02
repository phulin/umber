//! Native insert/adjust construction.
use super::super::*;

pub(in crate::main_control) fn execute_begin_insert<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    construction: &mut ScannedInsertConstruction,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1099's `begin_insert_or_adjust`: `scan_eight_bit_int`
        // has already applied its range clamp and queued the canonical
        // "Bad register code" report. The additional `\insert255`
        // rejection ("box 255 is special") runs here. `\vadjust` set
        // `class:=255` directly
        // (`is_vadjust`), without ever calling `scan_eight_bit_int`, so
        // neither diagnostic applies to it -- 255 is its correct,
        // already-valid sentinel class, not a user-typed `\insert255`.
        let mut class = construction.class;
        if !construction.is_vadjust && class == 255 {
            let mut report = stores.print_err("You can't ");
            report
                .print_esc("insert")
                .print_int(255)
                .help(&["I'm changing to \\insert0; box 255 is special."]);
            if let Some(context) = construction.reserved_class_context.take() {
                report.context(context);
            }
            report.error().defer_recovery(command.diagnostic_effects)?;
            class = 0;
        }
        let class = class as u16;
        let migration_segment_start = if boxes.active_boxes.last().is_some_and(|box_state| {
            box_state.kind == ReplayBoxKind::HBox && box_state.box_segment_start.is_some()
        }) && modes.current_mode() == Mode::RestrictedHorizontal
        {
            // Material before the insertion remains in the box body.
            // Finish its pending character run before opening the page
            // interval that hpack will migrate out of this box.
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            Some(stores.begin_page_node_region())
        } else {
            None
        };
        enter_group(
            stores,
            command.state,
            command.diagnostic_effects,
            GroupKind::Insert,
        );
        modes.push_at_line(
            Mode::InternalVertical,
            i32::try_from(command.state.current_file_line_number()).unwrap_or(i32::MAX),
        )?;
        // §1099: `normal_paragraph` resets \parshape/\looseness/\hangindent/
        // \hangafter local to the just-opened insert group, exactly like
        // `begin_box` does for `\vbox`/`\vtop` (§1051-2).
        crate::paragraph_end::normal_paragraph(modes, stores, command.diagnostic_effects);
        boxes.push_box(
            stores,
            ActiveReplayBox {
                target: None,
                shipout_region: None,
                box_segment_start: None,
                migration_segment_start,
                migration_segments: Vec::new(),
                kind: ReplayBoxKind::Insert(class, construction.pre),
                group_kind: GroupKind::Insert,
                packing: PackSpec::Natural,
                leader_kind: None,
                shift: None,
            },
        );
        // Unlike `\hbox`/`\vbox`/`\vtop`, §1099 never begins the
        // `\everyhbox`/`\everyvbox` token list for an insertion body.
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_insert_or_adjust<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
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
