//! TeX82 §1099: class/error, group/brace, then paragraph reset and body mode.
use super::*;

pub(super) fn begin<G>(
    processor: &mut Processor<'_, '_, G>,
    adjust: bool,
) -> Result<MainControlStep, ExecError> {
    let class = if adjust {
        255
    } else {
        let class = processor
            .scan_restricted_integer(tex_command::RestrictedIntegerClass::EightBit)
            .map_err(command_error)?
            .value;
        report_scan(processor)?;
        if class == 255 {
            processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
                let display = command.output_open_context(context);
                context.publish_diagnostic_effects_before_synchronous_print(effects);
                let mut report = context.print_err("You can't ");
                report
                    .print_esc("insert")
                    .print_int(255)
                    .help(&["I'm changing to \\insert0; box 255 is special."])
                    .context(display);
                report.error().defer_recovery(effects)
            })?;
            recovery::finish_error_dialogue(processor)?;
            0
        } else {
            class as u16
        }
    };
    let pre = adjust
        && processor.profile().capabilities().supports_pdftex()
        && processor.scan_keyword("pre").map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
        command.begin_group(context, effects, GroupKind::Insert);
    });
    processor.scan_left_brace().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let migration_segment_start =
            if execution
                .boxes
                .active_boxes
                .last()
                .is_some_and(|box_state| {
                    box_state.kind == ReplayBoxKind::HBox && box_state.box_segment_start.is_some()
                })
                && execution.modes.current_mode() == Mode::RestrictedHorizontal
            {
                // Material before the insertion remains in the box body.
                // Finish its pending character run before opening the page
                // interval that hpack will migrate out of this box.
                crate::box_runtime::flush_pending_hchars_with_fuel(
                    &mut execution.modes,
                    context,
                    effects,
                    fuel,
                )?;
                Some(context.begin_page_node_region())
            } else {
                None
            };
        crate::paragraph_end::normal_paragraph(&mut execution.modes, context, effects);
        execution.modes.push_at_line(
            Mode::InternalVertical,
            i32::try_from(command.current_file_line_number()).unwrap_or(i32::MAX),
        )?;
        execution.boxes.push_box(
            context,
            ActiveReplayBox {
                target: None,
                shipout_region: None,
                box_segment_start: None,
                migration_segment_start,
                migration_segments: Vec::new(),
                kind: ReplayBoxKind::Insert(class, pre),
                group_kind: GroupKind::Insert,
                packing: PackSpec::Natural,
                leader_kind: None,
                shift: None,
            },
        );
        // Unlike `\hbox`/`\vbox`/`\vtop`, §1099 never begins the
        // `\everyhbox`/`\everyvbox` token list for an insertion body.
        Ok(MainControlStep::Continue)
    })
}
