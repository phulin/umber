//! Direct boxes semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_box_dimension_assignment<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    index: u16,
    dimension: tex_state::BoxDimension,
    value: Scaled,
) -> Result<MainControlStep, ExecError> {
    {
        // `Universe::set_box_dimension{,_global}` share one body: TeX82
        // §1055's `alter_box_dimen` mutates the visible box node
        // directly rather than through the save stack, so the assignment
        // prefix does not change which binding level is affected.
        AssignmentCommitter::new(stores, effects)
            .unscoped(|stores| assign_box_dimension(stores, index, dimension, value));
        Ok(MainControlStep::Continue)
    }
}
