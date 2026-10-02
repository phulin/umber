//! Direct registers semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_count<G>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    receipts: &mut Option<Vec<MutationRecord>>,
    index: u16,
    value: i32,
    global: bool,
) -> Result<ReplayStep, ExecError> {
    let receipt = AssignmentCommitter::new(&mut LinearCommandContext::new(stores), effects)
        .count(index, value, global);
    if let Some(receipts) = receipts
        && receipt
    {
        receipts.push(MutationRecord {
            target: MutationTarget::Register,
            key: ObservationValue::Name(format!("count:{index}")),
            value: ObservationValue::Integer(i64::from(value)),
            global,
        });
    }
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn execute_dimen<G>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    receipts: &mut Option<Vec<MutationRecord>>,
    index: u16,
    value: Scaled,
    global: bool,
) -> Result<ReplayStep, ExecError> {
    let receipt = AssignmentCommitter::new(&mut LinearCommandContext::new(stores), effects)
        .dimension(index, value, global);
    if let Some(receipts) = receipts
        && receipt
    {
        receipts.push(MutationRecord {
            target: MutationTarget::Register,
            key: ObservationValue::Name(format!("dimen:{index}")),
            value: ObservationValue::Scaled(i64::from(value.raw())),
            global,
        });
    }
    Ok(ReplayStep::Continue)
}

#[allow(clippy::too_many_arguments)] // actual glue operand, scope, pointer identity, and observation owner
pub(in crate::main_control) fn execute_skip<G>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    receipts: &mut Option<Vec<MutationRecord>>,
    index: u16,
    value: GlueSpec,
    global: bool,
    mu: bool,
    redundant: bool,
    reassigning: bool,
) -> Result<ReplayStep, ExecError> {
    let committed = AssignmentCommitter::new(&mut LinearCommandContext::new(stores), effects).skip(
        index,
        value,
        global,
        mu,
        redundant,
        reassigning,
    );
    if let Some(receipts) = receipts
        && committed
    {
        receipts.push(MutationRecord {
            target: MutationTarget::Register,
            key: ObservationValue::Name(format!("{}:{index}", if mu { "muskip" } else { "skip" })),
            value: crate::assignments::committer::glue_value(&value),
            global,
        });
    }
    Ok(ReplayStep::Continue)
}

#[allow(clippy::too_many_arguments)] // actual native assignment and demanded observation owners
pub(in crate::main_control) fn execute_int_param<G>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    state: &CommandState<G>,
    receipts: &mut Option<Vec<MutationRecord>>,
    index: u16,
    value: i32,
    global: bool,
) -> Result<ReplayStep, ExecError> {
    let committed = AssignmentCommitter::new(&mut LinearCommandContext::new(stores), effects)
        .int_parameter(index, value, global);
    if let Some(receipts) = receipts
        && committed
    {
        receipts.push(MutationRecord {
            target: MutationTarget::Parameter,
            key: ObservationValue::Name(parameter_mutation_key_for_dialect(
                state.profile().dialect(),
                ParameterClass::Integer,
                index,
            )),
            value: ObservationValue::Integer(i64::from(value)),
            global,
        });
    }
    Ok(ReplayStep::Continue)
}

#[allow(clippy::too_many_arguments)] // actual native assignment and demanded observation owners
pub(in crate::main_control) fn execute_dimen_param<G>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    state: &CommandState<G>,
    receipts: &mut Option<Vec<MutationRecord>>,
    index: u16,
    value: Scaled,
    global: bool,
) -> Result<ReplayStep, ExecError> {
    let committed = AssignmentCommitter::new(&mut LinearCommandContext::new(stores), effects)
        .dimension_parameter(index, value, global);
    if let Some(receipts) = receipts
        && committed
    {
        receipts.push(MutationRecord {
            target: MutationTarget::Parameter,
            key: ObservationValue::Name(parameter_mutation_key_for_dialect(
                state.profile().dialect(),
                ParameterClass::Dimension,
                index,
            )),
            value: ObservationValue::Scaled(i64::from(value.raw())),
            global,
        });
    }
    Ok(ReplayStep::Continue)
}

#[allow(clippy::too_many_arguments)] // actual native assignment and demanded observation owners
pub(in crate::main_control) fn execute_glue_param<G>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    state: &CommandState<G>,
    receipts: &mut Option<Vec<MutationRecord>>,
    index: u16,
    value: GlueSpec,
    global: bool,
) -> Result<ReplayStep, ExecError> {
    let committed = AssignmentCommitter::new(&mut LinearCommandContext::new(stores), effects)
        .glue_parameter(index, value, global);
    if let Some(receipts) = receipts
        && committed
    {
        receipts.push(MutationRecord {
            target: MutationTarget::Parameter,
            key: ObservationValue::Name(parameter_mutation_key_for_dialect(
                state.profile().dialect(),
                ParameterClass::Glue,
                index,
            )),
            value: crate::assignments::committer::glue_value(&value),
            global,
        });
    }
    Ok(ReplayStep::Continue)
}

#[allow(clippy::too_many_arguments)] // concrete arithmetic operands and actual native evidence owners
pub(in crate::main_control) fn execute_arithmetic<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    state: &tex_command::CommandState<G>,
    receipts: &mut Option<Vec<MutationRecord>>,
    primitive: UnexpandablePrimitive,
    target: ArithmeticTarget,
    operand: ArithmeticOperand,
    global: bool,
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
            primitive,
            target,
            operand,
            global,
            state.profile(),
            stores,
            effects,
            receipts.as_mut(),
        ) {
            Err(ExecError::ArithmeticOverflow) => {
                let context = state.output_open_context(&**stores);
                let mut report = stores.print_err("Arithmetic overflow");
                report.help(&[
                    "I can't carry out that multiplication or division,",
                    "since the result is out of range.",
                ]);
                report.context(context);
                report.error().defer_recovery(effects)?;
            }
            Ok(()) => {}
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
