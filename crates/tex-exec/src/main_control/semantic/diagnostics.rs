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
