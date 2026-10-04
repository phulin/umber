//! Direct text semantics on native owners.
use super::super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::main_control) fn execute_text_direction<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &CommandState<G>,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    direction: tex_state::node::Direction,
    enabled: bool,
) -> Result<MainControlStep, ExecError> {
    {
        if enabled {
            crate::box_runtime::flush_pending_hchars_with_fuel(modes, stores, effects, fuel)?;
            modes
                .current_list_mutation()
                .construct(stores, |destination| {
                    destination.direction(direction);
                });
        } else {
            let name = match direction {
                tex_state::node::Direction::BeginM => "beginM",
                tex_state::node::Direction::EndM => "endM",
                tex_state::node::Direction::BeginL => "beginL",
                tex_state::node::Direction::EndL => "endL",
                tex_state::node::Direction::BeginR => "beginR",
                tex_state::node::Direction::EndR => "endR",
            };
            // etex.ch's `eTeX_enabled`: one report for every optional
            // feature, so the help names the disabled feature generally
            // rather than the primitive the message already named.
            let context = command.output_open_context(&*stores);
            report_escaped_error(
                stores,
                effects,
                "Improper ",
                name,
                "",
                &["Sorry, this optional e-TeX feature has been disabled."],
                context,
            )?;
        }
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_horizontal_skip<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    value: GlueSpec,
    shared_zero_glue: bool,
) -> Result<MainControlStep, ExecError> {
    {
        crate::box_runtime::flush_pending_hchars_with_fuel(modes, stores, effects, fuel)?;
        modes.current_list_mutation().push(
            stores,
            Node::Glue {
                spec: value,
                kind: GlueKind::Normal,
                origin: if shared_zero_glue {
                    tex_state::node::GlueSpecOrigin::SharedZero
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
                leader: None,
            },
        );
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_vertical_skip<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    value: GlueSpec,
    shared_zero_glue: bool,
) -> Result<MainControlStep, ExecError> {
    {
        // TeX82 §1054's `vmode+vskip: append_glue` (§1057): unlike
        // `\hskip` in vertical mode, `\vskip` never starts a paragraph --
        // the scan side (`scan_command`) only produces this step when the
        // mode is already `Vertical` or `InternalVertical`. §1057 also
        // notes `append_glue` deliberately never calls `build_page`
        // itself ("it is used in at least one place where that would be
        // a mistake"), unlike `append_penalty` (§1103); no page build
        // follows here.
        crate::box_runtime::append_node_to_current_list(
            modes,
            stores,
            effects,
            Node::Glue {
                spec: value,
                kind: GlueKind::Normal,
                origin: if shared_zero_glue {
                    tex_state::node::GlueSpecOrigin::SharedZero
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
                leader: None,
            },
            fuel,
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_kern<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    amount: Scaled,
) -> Result<MainControlStep, ExecError> {
    {
        // TeX82 §1057's `any_mode(kern),mmode+mkern: append_kern`
        // (§1061: `tail_append(new_kern(cur_val)); subtype(tail):=s`).
        // Unlike `\hskip` (§1090's `head_for_vmode`, which is genuinely
        // `vmode+hskip`-listed), `\kern` has no mode-specific dispatch
        // entry at all -- it is legal in every mode and appends directly,
        // with no paragraph start and no page-builder call. The outer
        // vertical list is represented by the page contribution queue,
        // so it still uses the shared contribution splice (contrast
        // `\penalty`, §1103, which also calls `build_page` there).
        crate::box_runtime::flush_pending_hchars_with_fuel(modes, stores, effects, fuel)?;
        crate::vertical::append_vertical_contribution(
            modes,
            stores,
            Node::Kern {
                amount,
                kind: KernKind::Explicit,
            },
        );
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_delete_last<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
    context: &mut tex_command::DiagnosticContextCoordinate,
) -> Result<MainControlStep, ExecError> {
    {
        let state = &*command.state;
        crate::box_runtime::execute_delete_last(
            *primitive,
            |stores| {
                state
                    .render_diagnostic_context(*context, stores)
                    .map_err(|_| {
                        ExecError::Command(tex_command::CommandError::StaleDiagnosticContext)
                    })
            },
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        Ok(MainControlStep::Continue)
    }
}
