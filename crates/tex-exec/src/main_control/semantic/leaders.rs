//! Direct leaders semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_h_rule_here_except_leaders<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let context = command.state.output_open_context(&**stores);
        report_escaped_error(
            stores,
            command.diagnostic_effects,
            "You can't use `",
            "hrule",
            "' here except with leaders",
            &[
                "To put a horizontal rule in an hbox or an alignment,",
                "you should use \\leaders or \\hrulefill (see The TeXbook).",
            ],
            context,
        )?;
        Ok(ReplayStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
pub(in crate::main_control) fn execute_leaders<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    kind: &mut GlueKind,
    payload: &mut LeaderPayload,
    glue: &mut GlueSpec,
    shared_zero_glue: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        boxes.pending_leader = None;
        crate::box_runtime::append_leader_contribution(
            modes,
            stores,
            command.diagnostic_effects,
            *kind,
            *payload,
            *glue,
            if *shared_zero_glue {
                tex_state::node::GlueSpecOrigin::SharedZero
            } else {
                tex_state::node::GlueSpecOrigin::Owned
            },
            command.fuel,
        )?;
        crate::vertical::build_page_if_outer_vertical(
            modes,
            stores,
            command.diagnostic_effects,
            command.state.state(),
        )?;
        Ok(ReplayStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
pub(in crate::main_control) fn execute_leader_register<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    kind: &mut GlueKind,
    index: &mut u16,
    copy: &mut bool,
    glue: &mut GlueSpec,
    shared_zero_glue: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if let Some(payload) = crate::box_runtime::take_register_payload(stores, *index, *copy) {
            crate::box_runtime::append_leader_contribution(
                modes,
                stores,
                command.diagnostic_effects,
                *kind,
                payload,
                *glue,
                if *shared_zero_glue {
                    tex_state::node::GlueSpecOrigin::SharedZero
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
                command.fuel,
            )?;
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

pub(in crate::main_control) fn execute_missing_leader_payload<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // A leader payload is scanned by §1084's `scan_box` like any
        // other box context, so a non-box command there gets §1084's own
        // report, not a leader-specific one.
        report_missing_box(command.state, command.diagnostic_effects, stores)?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_leaders_not_followed_by_glue<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        boxes.pending_leader = None;
        // TeX82 §1078's `back_error`; `scan_leader_glue_command` has
        // already put the command that was not glue back.
        let context = command.state.output_open_context(&**stores);
        crate::error_report::report_error(
            stores,
            command.diagnostic_effects,
            "Leaders not followed by proper glue",
            &[
                "You should say `\\leaders <box or rule><hskip or vskip>'.",
                "I found the <box or rule>, but there's no suitable",
                "<hskip or vskip>, so I'm ignoring these leaders.",
            ],
            context,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_begin_leader_box<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    construction: &mut ScannedBoxConstruction,
    leader_kind: &mut GlueKind,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let kind = ReplayBoxKind::from_scanned(construction.kind);
        let packing = match construction.packing {
            ScannedPackingSpec::Natural => PackSpec::Natural,
            ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
            ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
        };
        enter_group(
            stores,
            command.state,
            command.diagnostic_effects,
            kind.group_kind(),
        );
        modes.push_at_line(
            if kind.horizontal() {
                Mode::RestrictedHorizontal
            } else {
                Mode::InternalVertical
            },
            i32::try_from(command.state.current_file_line_number()).unwrap_or(i32::MAX),
        )?;
        if !kind.horizontal() {
            commit_box_normal_paragraph(modes, stores, command);
        }
        boxes.push_box(ActiveReplayBox {
            target: None,
            shipout_region: None,
            box_segment_start: None,
            migration_segment_start: None,
            migration_segments: Vec::new(),
            kind,
            group_kind: kind.group_kind(),
            packing,
            leader_kind: Some(*leader_kind),
            shift: None,
        });
        schedule_everybox(command.state, stores, kind.horizontal());
        Ok(ReplayStep::Continue)
    }
}
