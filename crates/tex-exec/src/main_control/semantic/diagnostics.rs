//! Direct diagnostics semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_set_interaction_mode<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
) -> Result<ReplayStep, ExecError> {
    {
        let mode = match primitive {
            UnexpandablePrimitive::BatchMode => tex_state::InteractionMode::Batch,
            UnexpandablePrimitive::NonstopMode => tex_state::InteractionMode::Nonstop,
            UnexpandablePrimitive::ScrollMode => tex_state::InteractionMode::Scroll,
            UnexpandablePrimitive::ErrorStopMode => tex_state::InteractionMode::ErrorStop,
            _ => unreachable!("only the four interaction-mode primitives are scanned"),
        };
        // TeX82 §1264's `new_interaction`: `print_ln` under the *old*
        // interaction mode's selector, unconditionally, before
        // `interaction:=cur_chr` takes effect. Skipping it left whichever
        // channel's column tracking stale until something else happened
        // to force a newline later -- invisible while every diagnostic
        // wrote both channels in lockstep, but a real divergence once
        // `\tracingonline<=0` redirects one channel alone (`umber2-
        // alfh.9`): the terminal's stale column then forces an extra,
        // unwanted newline into the log too, the first time anything
        // prints through the restored `term_and_log` selector.
        AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(|stores| {
            stores.printer().print_ln();
            stores.set_interaction_mode(mode);
        });
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_set_interaction_mode_value<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    value: &mut i32,
    context: &mut tex_command::DiagnosticContextCoordinate,
) -> Result<ReplayStep, ExecError> {
    {
        let mode = match value {
            0 => tex_state::InteractionMode::Batch,
            1 => tex_state::InteractionMode::Nonstop,
            2 => tex_state::InteractionMode::Scroll,
            3 => tex_state::InteractionMode::ErrorStop,
            value => {
                let context = render_diagnostic_coordinate(command, stores, *context)?;
                crate::diagnostics::report_bad_interaction_mode_with_context(
                    stores,
                    command.diagnostic_effects,
                    *value,
                    context,
                )?;
                return Ok(ReplayStep::Continue);
            }
        };
        // See the sibling `SetInteractionMode` arm's comment.
        AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(|stores| {
            stores.printer().print_ln();
            stores.set_interaction_mode(mode);
        });
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_message<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    tokens: &mut tex_state::TokenListId<G>,
    error: &mut bool,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1279's `issue_message` renders the scanned list through
        // `token_show` into one string and then hands it to §1280 or
        // §1283; neither branch formats or routes its own output.
        let text = message_tokens_text(stores, tokens.clone());
        if *error {
            let context = command.state.output_open_context(&*stores);
            issue_error_message(stores, command.diagnostic_effects, &text, context)?;
        } else {
            issue_terminal_message(stores, &text);
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_display_diagnostic<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    diagnostic: &mut ScannedDisplayDiagnostic,
) -> Result<ReplayStep, ExecError> {
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
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_show_lists<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
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
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_show_tokens<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    tokens: &mut tex_state::TokenListId<G>,
) -> Result<ReplayStep, ExecError> {
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
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_show_ifs<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    conditions: &mut [tex_command::ActiveCondition],
) -> Result<ReplayStep, ExecError> {
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
        Ok(ReplayStep::Continue)
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
) -> Result<ReplayStep, ExecError> {
    match diagnostic {
        Some(diagnostic) => {
            let context = command.state.output_open_context(&*stores);
            crate::diagnostics::execute_showgroups(stores, command.diagnostic_effects, diagnostic);
            command.defer_show_completion(true, context);
            Ok(ReplayStep::Continue)
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
            Ok(ReplayStep::Continue)
        }
    }
}
