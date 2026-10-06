//! Three live discretionary parts, closed by the ordinary interpreter.
use super::*;

pub(super) fn begin<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::flush_pending_hchars_with_fuel(
            &mut execution.modes,
            context,
            effects,
            fuel,
        )?;
        Ok::<(), ExecError>(())
    })?;
    open_part(processor)?;
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        execution.active_discretionaries.push(ActiveDiscretionary {
            parts: [tex_state::page_node_arena::PageListSpan::empty(); 3],
            completed: 0,
            rejected: false,
        });
    });
    Ok(MainControlStep::Continue)
}

fn open_part<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    processor.scan_left_brace().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.modes.push_at_line(
            Mode::RestrictedHorizontal,
            command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        command.begin_group(context, effects, GroupKind::Disc);
        execution.main_loop_active = false;
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn close_part<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let (nodes, deleted, prefix_end) =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            let mut level = crate::box_runtime::commit_current_list(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            let part_nodes = level.list().nodes(context);
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
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(index)
                }
            }) {
                ControlFlow::Break(index) => Some(index),
                ControlFlow::Continue(()) => None,
            };
            let prefix_end = first_forbidden.unwrap_or(part_len);
            let part = level.list_mutation().take_span();
            let nodes = context.slice_page_node_span(part, 0..prefix_end);
            let deleted = first_forbidden
                .map(|index| context.slice_page_node_span(part, index..part_len).list());
            Ok::<_, ExecError>((nodes, deleted, prefix_end))
        })?;
    groups::leave(processor, GroupKind::Disc, "discretionary group")?;
    let complete = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            let active =
                execution
                    .active_discretionaries
                    .last_mut()
                    .ok_or(ExecError::MissingToken {
                        context: "active discretionary",
                    })?;
            active.parts[active.completed] = nodes;
            active.completed += 1;
            let complete = active.completed == 3;
            let too_long = complete && prefix_end > 127;
            active.rejected |= too_long;
            if let Some(deleted) = deleted {
                let display = command.output_open_context(context);
                report_improper_discretionary(context, effects, deleted, display)?;
            }
            if too_long {
                let display = command.output_open_context(context);
                crate::error_report::report_ordered_error(
                    context,
                    effects,
                    "Discretionary list is too long",
                    &["Wow---I never thought anybody would tweak me here."],
                    display,
                )?;
            }
            execution.main_loop_active = false;
            Ok::<_, ExecError>(complete)
        },
    )?;
    recovery::finish_error_dialogue(processor)?;
    if !complete {
        return open_part(processor);
    }
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        let active = execution
            .active_discretionaries
            .pop()
            .expect("completed discretionary");
        if active.rejected {
            return Ok(MainControlStep::Continue);
        }
        let [pre, post, mut replace] = active.parts;
        if matches!(
            execution.modes.current_mode(),
            Mode::Math | Mode::DisplayMath
        ) && !replace.is_empty()
        {
            let display = command.output_open_context(context);
            context.publish_diagnostic_effects_before_synchronous_print(effects);
            report_escaped_error(
                context,
                effects,
                "Illegal math ",
                "discretionary",
                "",
                &[
                    "Sorry: The third part of a discretionary break must be",
                    "empty, in math formulas. I had to delete your third part.",
                ],
                display,
            )?;
            replace = tex_state::page_node_arena::PageListSpan::empty();
        }
        let physical_replace_count = context
            .page_node_span(replace)
            .expect("live replacement list")
            .len()
            .try_into()
            .expect("quarterword replacement count");
        execution.modes.current_list_mutation().push(
            context,
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre: pre.list(),
                post: post.list(),
                replace: replace.list(),
                physical_replace_count,
            },
        );
        Ok::<_, ExecError>(MainControlStep::Continue)
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

pub(super) fn hyphen<G>(
    processor: &mut Processor<'_, '_, G>,
    origin: OriginId,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    let extended = processor.profile() == CommandProfile::ETEX26;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::flush_pending_hchars_with_fuel(
            &mut execution.modes,
            context,
            effects,
            fuel,
        )?;
        let font = context.current_font();
        let empty = tex_state::page_node_arena::PageListId::empty();
        let pre = match u8::try_from(context.font_hyphen_char(font)) {
            Ok(hyphen) if context.font_char_metrics(font, hyphen).is_some() => context
                .publish_page_nodes([Node::Char {
                    font,
                    ch: char::from(hyphen),
                    origin,
                }]),
            Ok(hyphen) => {
                crate::diagnostics::report_missing_character_warning(
                    context,
                    effects,
                    font,
                    char::from(hyphen),
                    extended,
                );
                empty
            }
            Err(_) => empty,
        };
        execution.modes.current_list_mutation().push(
            context,
            Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre,
                post: empty,
                replace: empty,
                physical_replace_count: 0,
            },
        );
        Ok(MainControlStep::Continue)
    })
}
