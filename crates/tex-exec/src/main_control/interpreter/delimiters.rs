//! Left/middle/right use the live math tail and native group stack.
use super::*;

pub(super) fn left<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    let delimiter = processor.scan_delimiter(false).map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        command.begin_group(context, effects, GroupKind::MathLeft);
        execution.modes.push_at_line(
            Mode::Math,
            command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        execution.active_math_left_boundaries.push(false);
        execution.modes.current_list_mutation().push(
            context,
            Node::MathNoad(MathNoad::new(
                NoadKind::LeftDelimiter {
                    delimiter: delimiter.code,
                },
                MathField::Empty,
            )),
        );
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn close<G>(
    processor: &mut Processor<'_, '_, G>,
    middle: bool,
) -> Result<MainControlStep, ExecError> {
    let delimiter = processor.scan_delimiter(false).map_err(command_error)?;
    report_scan(processor)?;
    let open = processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        if context.innermost_group_kind() != Some(GroupKind::MathLeft) {
            return false;
        }
        let left_node = |node: Option<tex_state::node_view::NodeView<'_>>| {
            matches!(
                node,
                Some(tex_state::node_view::NodeView::MathNoad(MathNoad {
                    kind: NoadKind::LeftDelimiter { .. },
                    ..
                }))
            )
        };
        left_node(execution.modes.current_list().nodes(context).first())
            || execution
                .modes
                .current_list()
                .incomplete_fraction()
                .is_some_and(|fraction| {
                    context
                        .page_node_span(fraction.numerator)
                        .ok()
                        .and_then(|list| list.nodes().first())
                        .is_some_and(|node| left_node(Some(node)))
                })
    });
    if !open {
        processor.with_execution_owners(
            |context, command, effects, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                let display = command.output_open_context(context);
                report_escaped_error(
                    context,
                    effects,
                    "Extra ",
                    if middle { "middle" } else { "right" },
                    "",
                    &[if middle {
                        "I'm ignoring a \\middle that had no matching \\left."
                    } else {
                        "I'm ignoring a \\right that had no matching \\left."
                    }],
                    display,
                )
            },
        )?;
        recovery::finish_error_dialogue(processor)?;
        return Ok(MainControlStep::Continue);
    }
    let (content, mut closed) =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            execution.main_loop_active = false;
            let (nodes, fraction) = {
                let mut list = execution.modes.current_list_mutation();
                (list.take_nodes(), list.take_incomplete_fraction())
            };
            let content = finish_math_list(nodes, fraction, context)?;
            let closed = crate::box_runtime::commit_current_list(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            Ok::<_, ExecError>((content, closed))
        })?;
    groups::leave(processor, GroupKind::MathLeft, "math left group")?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution
            .active_math_left_boundaries
            .pop()
            .expect("live left group");
        if middle {
            command.begin_group(context, effects, GroupKind::MathLeft);
            execution.modes.push_at_line(
                Mode::Math,
                command
                    .current_file_line_number()
                    .try_into()
                    .unwrap_or(i32::MAX),
            )?;
            execution.active_math_left_boundaries.push(true);
            let content = context.reclaim_unique_page_list(content);
            let mut list = execution.modes.current_list_mutation();
            list.append_unique_list(context, content);
            list.push(
                context,
                Node::MathNoad(MathNoad::new(
                    NoadKind::MiddleDelimiter {
                        delimiter: delimiter.code,
                    },
                    MathField::Empty,
                )),
            );
        } else {
            // The actual closed math tail owns the final right boundary. No
            // one-node publication and immediately consumed composition root.
            let content = context.reclaim_unique_page_list(content);
            let content = {
                let mut list = closed.list_mutation();
                list.append_unique_list(context, content);
                list.push(
                    context,
                    Node::MathNoad(MathNoad::new(
                        NoadKind::RightDelimiter {
                            delimiter: delimiter.code,
                        },
                        MathField::Empty,
                    )),
                );
                list.take_nodes()
            };
            execution.modes.current_list_mutation().push(
                context,
                Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Inner),
                    MathField::SubMlist(content),
                )),
            );
        }
        Ok(MainControlStep::Continue)
    })
}
