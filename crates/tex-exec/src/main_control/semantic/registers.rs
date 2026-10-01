//! Direct registers semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_count<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    value: &mut i32,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
            .count(*index, *value, *global);
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_dimen<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    value: &mut Scaled,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
            .dimension(*index, *value, *global);
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_skip<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    value: &mut GlueSpec,
    redundant: &mut bool,
    reassigning: &mut bool,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).skip(
            *index,
            *value,
            *global,
            false,
            *redundant,
            *reassigning,
        );
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_muskip<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    value: &mut GlueSpec,
    redundant: &mut bool,
    reassigning: &mut bool,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).skip(
            *index,
            *value,
            *global,
            true,
            *redundant,
            *reassigning,
        );
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_int_param<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    value: &mut i32,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let key = parameter_mutation_key_for_dialect(
            command.state.profile().dialect(),
            ParameterClass::Integer,
            *index,
        );
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
            .int_parameter(*index, *value, key, *global);
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_dimen_param<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    value: &mut Scaled,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let key = parameter_mutation_key_for_dialect(
            command.state.profile().dialect(),
            ParameterClass::Dimension,
            *index,
        );
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
            .dimension_parameter(*index, *value, key, *global);
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_glue_param<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    value: &mut GlueSpec,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let key = parameter_mutation_key_for_dialect(
            command.state.profile().dialect(),
            ParameterClass::Glue,
            *index,
        );
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
            .glue_parameter(*index, *value, key, *global);
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_arithmetic<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
    target: &mut ArithmeticTarget,
    operand: &mut ArithmeticOperand,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1236 sets `arith_error` and, when it is set, reports
        // "Arithmetic overflow" and `return`s *before* `word_define`, so
        // the target keeps its old value and the job continues. Every
        // arm of `apply_arithmetic` computes its value before writing it,
        // so the target is provably unwritten on this path.
        match apply_arithmetic(
            *primitive,
            *target,
            *operand,
            *global,
            command.state.profile(),
            stores,
            command.diagnostic_effects,
        ) {
            Err(ExecError::ArithmeticOverflow) => {
                let context = command.state.output_open_context(&**stores);
                let mut report = stores.print_err("Arithmetic overflow");
                report.help(&[
                    "I can't carry out that multiplication or division,",
                    "since the result is out of range.",
                ]);
                report.context(context);
                report.error().defer_recovery(command.diagnostic_effects)?;
            }
            Ok(receipt) => {
                command.retain_assignment_receipt(receipt);
            }
            Err(error) => return Err(error),
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_invalid_arithmetic_target<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
    target: &mut tex_command::PrintCommand<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1236 prints this error and returns from
        // `do_register_command`; §1269's common `done` path still gets
        // to replay a pending `\afterassignment` token.
        let target = tex_command::print_cmd_chr_text(&**stores, target.clone());
        let primitive = stores
            .primitive_name(Meaning::UnexpandablePrimitive(*primitive))
            .expect("installed arithmetic primitive has a canonical name")
            .to_owned();
        let context = command.state.output_open_context(&**stores);
        let mut report = stores.print_err("You can't use `");
        report
            .print(&target)
            .print("' after ")
            .print_esc(&primitive);
        report.help(&["I'm forgetting what you said and not changing anything."]);
        report.context(context);
        report.error().defer_recovery(command.diagnostic_effects)?;
        Ok(ReplayStep::Continue)
    }
}
