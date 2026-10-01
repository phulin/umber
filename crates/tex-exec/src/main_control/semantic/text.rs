//! Direct text semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_fixed_horizontal_glue<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_paragraph(
                command.state,
                modes,
                stores,
                command.diagnostic_effects,
                true,
            )?;
        }
        crate::box_runtime::flush_pending_hchars_with_fuel(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        modes.current_list_mutation().push(
            stores,
            Node::Glue {
                origin: tex_state::node::GlueSpecOrigin::Owned,
                spec: crate::box_runtime::fixed_infinite_glue(*primitive),
                kind: GlueKind::Normal,
                leader: None,
            },
        );
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_fixed_vertical_glue<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // See the vertical-skip handler: same §1054/§1057
        // `append_glue`, no paragraph start, no page build.
        let spec = crate::box_runtime::fixed_infinite_glue(*primitive);
        crate::box_runtime::append_node_to_current_list(
            modes,
            stores,
            command.diagnostic_effects,
            Node::Glue {
                origin: tex_state::node::GlueSpecOrigin::Owned,
                spec,
                kind: GlueKind::Normal,
                leader: None,
            },
            command.fuel,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_rule<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    width: &mut Option<Scaled>,
    height: &mut Option<Scaled>,
    depth: &mut Option<Scaled>,
    horizontal: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    apply_scanned_rule(command, modes, stores, *width, *height, *depth, *horizontal)
}

pub(in crate::main_control) fn execute_character<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    ch: &mut char,
    cat: &mut Catcode,
    origin: &mut tex_state::token::OriginId,
    suppress_left_boundary: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if matches!(modes.current_mode(), Mode::Math | Mode::DisplayMath) {
            if !matches!(cat, Catcode::Space) {
                // TeX82 §1154's `mmode+letter,mmode+other_char:
                // set_math_char(ho(math_code(cur_chr)))`.
                set_math_char(*ch, *origin, stores, modes, command)?;
            }
            return Ok(ReplayStep::Continue);
        }
        match cat {
            // TeX82 §1045's `any_mode(relax),vmode+spacer,mmode+spacer,
            // mmode+no_boundary:do_nothing` leaves vertical mode
            // untouched by an ordinary space; only `start_par`, a
            // letter/other/char_num/char_given, or an explicit
            // box/rule/etc. triggers `new_graf` via §1090's
            // `back_input; new_graf(true)`. A space therefore never
            // itself opens a paragraph here.
            Catcode::Space => {
                if matches!(
                    modes.current_mode(),
                    Mode::Horizontal | Mode::RestrictedHorizontal
                ) {
                    crate::box_runtime::append_space_with_fuel(
                        modes,
                        stores,
                        command.diagnostic_effects,
                        command.fuel,
                    )?;
                }
            }
            Catcode::Letter | Catcode::Other => {
                if matches!(
                    modes.current_mode(),
                    Mode::Vertical | Mode::InternalVertical
                ) {
                    start_paragraph(
                        command.state,
                        modes,
                        stores,
                        command.diagnostic_effects,
                        true,
                    )?;
                }
                modes
                    .current_list_mutation()
                    .set_no_boundary(*suppress_left_boundary);
                crate::box_runtime::append_character_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    *ch,
                    *origin,
                    command.state.profile() == CommandProfile::ETEX26,
                    command.fuel,
                )?;
            }
            _ => unreachable!("canonical character scan restricts catcodes"),
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_text_direction<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    direction: &mut tex_state::node::Direction,
    enabled: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if *enabled {
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            modes
                .current_list_mutation()
                .construct(stores, |destination| {
                    destination.direction(*direction);
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
            let context = command.state.output_open_context(&**stores);
            report_escaped_error(
                stores,
                command.diagnostic_effects,
                "Improper ",
                name,
                "",
                &["Sorry, this optional e-TeX feature has been disabled."],
                context,
            )?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_horizontal_skip<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    value: &mut GlueSpec,
    shared_zero_glue: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_paragraph(
                command.state,
                modes,
                stores,
                command.diagnostic_effects,
                true,
            )?;
        }
        crate::box_runtime::flush_pending_hchars_with_fuel(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        modes.current_list_mutation().push(
            stores,
            Node::Glue {
                spec: *value,
                kind: GlueKind::Normal,
                origin: if *shared_zero_glue {
                    tex_state::node::GlueSpecOrigin::SharedZero
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
                leader: None,
            },
        );
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_vertical_skip<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    value: &mut GlueSpec,
    shared_zero_glue: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
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
            command.diagnostic_effects,
            Node::Glue {
                spec: *value,
                kind: GlueKind::Normal,
                origin: if *shared_zero_glue {
                    tex_state::node::GlueSpecOrigin::SharedZero
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
                leader: None,
            },
            command.fuel,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_kern<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    amount: &mut Scaled,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
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
        crate::box_runtime::flush_pending_hchars_with_fuel(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        crate::vertical::append_vertical_contribution(
            modes,
            stores,
            Node::Kern {
                amount: *amount,
                kind: KernKind::Explicit,
            },
        );
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_penalty<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    amount: &mut i32,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1103's `append_penalty`: `tail_append(new_penalty(cur_val))`
        // in whichever list is current, then `if mode=vmode then
        // build_page` -- i.e. only in *outer* vertical mode, not internal
        // vertical mode, matching `append_vertical_contribution`'s own
        // `is_outer_vertical` gate and (unlike `\vskip`'s `append_glue`,
        // §1057) always followed by a page-builder call in that case.
        crate::box_runtime::flush_pending_hchars_with_fuel(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        crate::vertical::append_vertical_contribution(modes, stores, Node::Penalty(*amount));
        crate::vertical::build_page_if_outer_vertical(
            modes,
            stores,
            command.diagnostic_effects,
            command.state.state(),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_character_code<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    value: &mut i32,
    origin: &mut tex_state::token::OriginId,
    suppress_left_boundary: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let ch =
            u32::try_from(*value)
                .ok()
                .and_then(char::from_u32)
                .ok_or(ExecError::InvalidCode {
                    context: "\\char",
                    value: *value,
                })?;
        if matches!(modes.current_mode(), Mode::Math | Mode::DisplayMath) {
            // TeX82 `main_control`'s `mmode+char_num` (§1154) scans the
            // character number and then calls `set_math_char` (§1155)
            // with its `math_code`, exactly like the sibling
            // `mmode+letter`/`mmode+other_char`/`mmode+char_given` cases:
            // it appends a math-char noad and never begins or continues
            // a horizontal list from math mode.
            set_math_char(ch, *origin, stores, modes, command)?;
            return Ok(ReplayStep::Continue);
        }
        if matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_paragraph(
                command.state,
                modes,
                stores,
                command.diagnostic_effects,
                true,
            )?;
        }
        modes
            .current_list_mutation()
            .set_no_boundary(*suppress_left_boundary);
        crate::box_runtime::append_character_with_fuel(
            modes,
            stores,
            command.diagnostic_effects,
            ch,
            *origin,
            command.state.profile() == CommandProfile::ETEX26,
            command.fuel,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_delete_last<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
    context: &mut tex_command::DiagnosticContextCoordinate,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
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
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_italic_correction<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        match modes.current_mode() {
            Mode::Horizontal | Mode::RestrictedHorizontal => {
                crate::box_runtime::append_italic_correction_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
            }
            Mode::Math | Mode::DisplayMath => {
                // TeX82 §1112: `mmode+ital_corr: tail_append(new_kern(0));`
                // -- `new_kern`'s default subtype (`normal`) is never
                // overridden here (unlike hmode's italic-correction kern,
                // or an explicit `\kern`), so it must not become a legal
                // kern-then-glue line-break point.
                modes.current_list_mutation().push(
                    stores,
                    Node::Kern {
                        amount: Scaled::from_raw(0),
                        kind: KernKind::Font,
                    },
                );
            }
            Mode::Vertical | Mode::InternalVertical => {
                unreachable!("vertical \\/ is scanned as IllegalItalicCorrection")
            }
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_no_boundary<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    suppress_right: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if *suppress_right {
            crate::box_runtime::flush_pending_hchars_without_right_boundary(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_non_script<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1171: a zero glue with the `cond_math_glue` subtype.
        modes.current_list_mutation().push(
            stores,
            Node::Glue {
                origin: tex_state::node::GlueSpecOrigin::SharedZero,
                spec: GlueSpec::ZERO,
                kind: GlueKind::NonScript,
                leader: None,
            },
        );
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_control_space<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        match modes.current_mode() {
            Mode::Math | Mode::DisplayMath => {
                // TeX82 §1030's `mmode+ex_space: goto append_normal_space`
                // (§1041) appends real interword glue in math mode, unlike
                // an ordinary `mmode+spacer`, which §1045 makes a no-op.
                let spec = crate::box_runtime::control_space_glue_spec(stores);
                modes.current_list_mutation().push(
                    stores,
                    Node::Glue {
                        origin: tex_state::node::GlueSpecOrigin::Owned,
                        spec,
                        kind: GlueKind::Normal,
                        leader: None,
                    },
                );
            }
            Mode::Vertical | Mode::InternalVertical => {
                start_paragraph(
                    command.state,
                    modes,
                    stores,
                    command.diagnostic_effects,
                    true,
                )?;
                crate::box_runtime::append_control_space_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
            }
            _ => {
                crate::box_runtime::append_control_space_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
            }
        }
        Ok(ReplayStep::Continue)
    }
}
