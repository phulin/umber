//! Native group entry and unsave on the live save stack.
use super::super::*;

pub(in crate::main_control) fn enter_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut tex_command::CommandState<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    kind: GroupKind,
) {
    let entered_line = command.current_file_line_number();
    stores
        .begin_group(kind, entered_line)
        .expect("native TeX group boundary opens");
    let level = u32::try_from(stores.execution_group_depth()).unwrap_or(u32::MAX);
    stores.trace_group_enter(diagnostic_effects, kind, level, entered_line);
}
