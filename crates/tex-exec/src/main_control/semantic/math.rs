//! Direct math semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_illegal_eq_no<G>(
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

pub(in crate::main_control) fn execute_missing_math_shift<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1047's `insert_dollar_sign` diagnostic; the matching
        // input recovery (backing up the offending command behind an
        // inserted `$`) already ran in `recover_missing_math_shift`.
        let context = command.state.output_open_context(&**stores);
        crate::error_report::report_error(
            stores,
            command.diagnostic_effects,
            "Missing $ inserted",
            &[
                "I've inserted a begin-math/end-math symbol since I think",
                "you left one out. Proceed, with fingers crossed.",
            ],
            context,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_math_family<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    family: &mut tex_command::ScannedMathFamily,
    font: &mut FontId,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        AssignmentCommitter::new(stores, command.diagnostic_effects).try_unscoped(
            None,
            |stores| {
                assign_math_family_font(
                    stores,
                    MathFontSize::from(family.size),
                    family.family,
                    *font,
                    *global,
                )
            },
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_end_math_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    active_math_fields: &mut Vec<ActiveMathFieldTarget>,
    kind: &mut GroupKind,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1186 and §1174's `build_choices` both open with
        // `unsave`. Everything after it -- popping `saved`, `fin_mlist`,
        // and storing the result in the field or branch -- belongs to the
        // scanner that opened the group, so `execute_live_math_group`
        // performs it once its level is gone.
        unsave_group(stores, command, *kind).map_err(|_| ExecError::MissingToken {
            context: "math group",
        })?;
        if *kind == GroupKind::Math {
            // TeX82 §1186 runs `fin_mlist` in the right-brace command
            // itself, then writes the result through §1153's saved field
            // pointer. This is the ordinary main-control closer for the
            // live group, not a return into a nested executor loop.
            let target = *active_math_fields.last().ok_or(ExecError::MissingToken {
                context: "active math field",
            })?;
            let mut level = crate::box_runtime::commit_current_list(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            let (nodes, incomplete) = {
                let mut list = level.list_mutation();
                (list.take_nodes(), list.take_incomplete_fraction())
            };
            let list = finish_math_list(nodes, incomplete, stores)?;
            let field = collapse_singleton_math_group(stores, list);
            fill_math_field_target(modes, stores, target, field);
            active_math_fields.pop();
        }
        Ok(ReplayStep::Continue)
    }
}
