//! Direct diagnostics semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_set_interaction_mode<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &CommandState<G>,
    effects: &mut DiagnosticEffects,
    value: i32,
    coordinate: Option<tex_command::DiagnosticContextCoordinate>,
) -> Result<MainControlStep, ExecError> {
    let mode = match value {
        0 => tex_state::InteractionMode::Batch,
        1 => tex_state::InteractionMode::Nonstop,
        2 => tex_state::InteractionMode::Scroll,
        3 => tex_state::InteractionMode::ErrorStop,
        _ => {
            let context = command
                .render_diagnostic_context(
                    coordinate.expect("invalid interaction mode is a scanned operand"),
                    stores,
                )
                .map_err(|_| {
                    ExecError::Command(tex_command::CommandError::StaleDiagnosticContext)
                })?;
            crate::diagnostics::report_bad_interaction_mode_with_context(
                stores, effects, value, context,
            )?;
            return Ok(MainControlStep::Continue);
        }
    };
    // §1264 prints under the old selector before changing interaction.
    stores.publish_diagnostic_effects_before_synchronous_print(effects);
    AssignmentCommitter::new(stores, effects).unscoped(|stores| {
        stores.printer().print_ln();
        stores.set_interaction_mode(mode);
    });
    Ok(MainControlStep::Continue)
}

pub(in crate::main_control) fn execute_display_diagnostic<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    diagnostic: &mut ScannedDisplayDiagnostic,
) -> Result<MainControlStep, ExecError> {
    {
        // TeX82 §§62/1294/1297 begin the display with `print_nl(">␣")`,
        // which closes a partial selected line but does not add a blank
        // line when both selected sinks are already at column zero.
        // The scanned value carries exactly that line's content; replay
        // owns the selector-sensitive transition and decodes no textual
        // envelope.
        let context = command.state.output_open_context(&*stores);
        print_display_content(stores, command.diagnostic_effects, &diagnostic.content);
        command.defer_show_completion(false, context);
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_show_lists<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    {
        // TeX82 §218 observes the synchronous linked list built by
        // main_control. Materialize Umber's batched character tail before
        // traversing its diagnostic physical projection.
        crate::box_runtime::flush_pending_hchars(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::execute_showlists(
            stores,
            command.diagnostic_effects,
            modes,
            command.state.profile(),
        )?;
        command.defer_show_completion(true, context);
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_show_tokens<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    tokens: &mut tex_state::TokenListId<G>,
) -> Result<MainControlStep, ExecError> {
    {
        // e-TeX's odd xray modifier reaches `the_toks`, then TeX82
        // §1297 prints `token_show(temp_head)` and takes the common
        // `\show` completion path.
        let context = command.state.output_open_context(&*stores);
        let text = show_tokens_tokens_text(stores, tokens.clone());
        // §1297 opens with §62's `print_nl(">␣")`, whose break is
        // conditional on a selected sink already having an open column.
        // An unconditional newline here left a blank line above the
        // display whenever the file's own `(` had just closed one.
        let mut diagnostic = stores.begin_online_diagnostic(command.diagnostic_effects);
        diagnostic.print_nl("> ").print_rendered(&text);
        diagnostic.end_open();
        command.defer_show_completion(false, context);
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_show_ifs<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    conditions: &mut [tex_command::ActiveCondition],
) -> Result<MainControlStep, ExecError> {
    {
        // etex.ch [17.3720]'s `show_ifs` is a `begin_diagnostic` form
        // like `\showbox`/`\showlists`/`\showgroups`, not a direct
        // print: see `tex-exec::diagnostics`'s module doc for why the
        // dump must be routed through §245's redirection rather than
        // written straight to both channels.
        let context = command.state.output_open_context(&*stores);
        let mut diagnostic = stores.begin_diagnostic(command.diagnostic_effects);
        diagnostic.print_nl("").print_ln();
        diagnostic.print_rendered(&render_showifs(conditions));
        diagnostic.end(true);
        command.defer_show_completion(true, context);
        Ok(MainControlStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
pub(in crate::main_control) fn execute_show_groups<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    active_discretionaries: &[ActiveDiscretionary],
    active_math_choices: &[usize],
    active_math_left_boundaries: &[bool],
    active_math_shifts: &[MathShiftContext],
    diagnostic: &mut Option<crate::diagnostics::ShowGroupsDiagnostic>,
) -> Result<MainControlStep, ExecError> {
    match diagnostic {
        Some(diagnostic) => {
            let context = command.state.output_open_context(&*stores);
            crate::diagnostics::execute_showgroups(stores, command.diagnostic_effects, diagnostic);
            command.defer_show_completion(true, context);
            Ok(MainControlStep::Continue)
        }
        None => {
            let diagnostic = detached_showgroups(
                stores,
                active_alignment,
                boxes,
                active_discretionaries,
                active_math_choices,
                active_math_left_boundaries,
                active_math_shifts,
            );
            let context = command.state.output_open_context(&*stores);
            crate::diagnostics::execute_showgroups(stores, command.diagnostic_effects, &diagnostic);
            command.defer_show_completion(true, context);
            Ok(MainControlStep::Continue)
        }
    }
}
