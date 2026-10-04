//! Direct groups semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_after_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandState<G>,
    token: &mut tex_state::token::TracedTokenWord,
) -> Result<MainControlStep, ExecError> {
    {
        match command.save_aftergroup(stores, *token) {
            // TeX82 §280's `save_for_after` consumes the operand at every
            // level, but creates an `insert_token` save word only when
            // `cur_level > level_one`. `NoOpenGroup` is therefore the
            // canonical outer-level no-op; every synchronization failure
            // remains an invariant violation.
            Ok(()) | Err(tex_command::CommandGroupError::NoOpenGroup) => {}
            Err(error) => {
                panic!("aftergroup is admitted only for the synchronized group stack: {error:?}")
            }
        }
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_after_assignment<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandState<G>,
    token: &mut tex_state::token::TracedTokenWord,
) -> Result<MainControlStep, ExecError> {
    {
        command
            .set_afterassignment(&*stores, *token)
            .expect("afterassignment uses the synchronized command generation");
        Ok(MainControlStep::Continue)
    }
}
