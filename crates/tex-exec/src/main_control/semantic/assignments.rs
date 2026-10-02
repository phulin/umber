//! Direct assignments semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_character_definition<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
    target: &mut Symbol,
    value: &mut i32,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let meaning = match primitive {
            UnexpandablePrimitive::CharDef => Meaning::CharGiven(
                char::from_u32(*value as u32)
                    .expect("§434 recovers a character code to a character"),
            ),
            UnexpandablePrimitive::MathCharDef => Meaning::MathCharGiven(*value as u16),
            _ => unreachable!("character-definition step carries only §1224 primitives"),
        };
        let observed = match primitive {
            UnexpandablePrimitive::CharDef => ObservationValue::Character(*value as u32),
            UnexpandablePrimitive::MathCharDef => ObservationValue::Integer(i64::from(*value)),
            _ => unreachable!("character-definition step carries only §1224 primitives"),
        };
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).direct_meaning(
            *target,
            Token::Cs(*target),
            meaning,
            *global,
            |stores| {
                stores
                    .assign_resolved_meaning(
                        *target,
                        tex_state::meaning::ResolvedMeaning::Static(meaning),
                        assignment_scope(*global),
                    )
                    .expect("character-definition target belongs to admitted state")
            },
        );
        if receipt && command.observes_mutations() {
            command.retain_assignment_mutation(MutationRecord {
                target: MutationTarget::Meaning,
                key: ObservationValue::Name(stores.resolve(*target).to_owned()),
                value: observed,
                global: *global,
            });
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_register_definition<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    primitive: &mut UnexpandablePrimitive,
    target: &mut Symbol,
    index: &mut u16,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let meaning = match primitive {
            UnexpandablePrimitive::CountDef => Meaning::CountRegister(*index),
            UnexpandablePrimitive::DimenDef => Meaning::DimenRegister(*index),
            UnexpandablePrimitive::SkipDef => Meaning::SkipRegister(*index),
            UnexpandablePrimitive::MuskipDef => Meaning::MuskipRegister(*index),
            UnexpandablePrimitive::ToksDef => Meaning::ToksRegister(*index),
            _ => unreachable!("register-definition step carries only §1224 primitives"),
        };
        let observed_name =
            if command.state.profile().capabilities().supports_etex() && *index > 255 {
                if *primitive == UnexpandablePrimitive::ToksDef {
                    "toks_register"
                } else {
                    "register"
                }
            } else {
                match primitive {
                    UnexpandablePrimitive::CountDef => "assign_int",
                    UnexpandablePrimitive::DimenDef => "assign_dimen",
                    UnexpandablePrimitive::SkipDef => "assign_glue",
                    UnexpandablePrimitive::MuskipDef => "assign_mu_glue",
                    UnexpandablePrimitive::ToksDef => "assign_toks",
                    _ => unreachable!("register-definition step carries only §1224 primitives"),
                }
            };
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).direct_meaning(
            *target,
            Token::Cs(*target),
            meaning,
            *global,
            |stores| {
                stores
                    .assign_resolved_meaning(
                        *target,
                        tex_state::meaning::ResolvedMeaning::Static(meaning),
                        assignment_scope(*global),
                    )
                    .expect("register-definition target belongs to admitted state")
            },
        );
        if receipt && command.observes_mutations() {
            command.retain_assignment_mutation(MutationRecord {
                target: MutationTarget::Meaning,
                key: ObservationValue::Name(stores.resolve(*target).to_owned()),
                value: ObservationValue::Name(observed_name.into()),
                global: *global,
            });
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_paragraph_shape<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    lines: &mut [ParagraphShapeLine],
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1214's "Adjust for the setting of `\globaldefs`" runs
        // unconditionally before `prefixed_command`'s `case cur_cmd of
        // @<Assignments@> endcases`, so it applies uniformly to all
        // thirty assignment forms §1210 dispatches -- `set_shape`
        // (§1248's `define(par_shape_loc,shape_ref,p)`) among them,
        // since `\parshape` is an ordinary `eqtb` entry that `define`
        // scopes through the save stack. This was the third canonical
        // apply arm (after `\def`/`\edef`/`\gdef`/`\xdef` and
        // `\let`/`\futurelet`) that passed the raw `\global` prefix bit
        // straight through and silently ignored a nonzero
        // `\globaldefs`; it was missed by both earlier sweeps because
        // `set_shape` belongs to neither definition family.
        AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(|stores| {
            stores
                .assign_paragraph_shape(lines, assignment_scope(*global))
                .expect("paragraph shape fits admitted durable storage")
        });
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_penalty_array<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    kind: &mut PenaltyArrayKind,
    values: &mut [i32],
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // e-TeX 2.6 change [49.1248] commits every selector through
        // `define(q, shape_ref, p)`, so this uses the same save-stack and
        // `\globaldefs`-adjusted scope bit as TeX82 §1214/§1248.
        let record = command.observes_mutations().then(|| MutationRecord {
            target: MutationTarget::Register,
            key: ObservationValue::Name(format!(
                "toks:{}",
                match kind {
                    PenaltyArrayKind::InterLine => 256,
                    PenaltyArrayKind::Club => 257,
                    PenaltyArrayKind::Widow => 258,
                    PenaltyArrayKind::DisplayWidow => 259,
                }
            )),
            value: ObservationValue::Tokens(Vec::new()),
            global: *global,
        });
        AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped_with_effects(
            |stores, diagnostic_effects| {
                let old = stores.penalty_array(*kind);
                stores
                    .assign_penalty_array(*kind, values, assignment_scope(*global))
                    .expect("penalty array fits admitted durable storage");
                assignment_tracing::trace_penalty_array(
                    stores,
                    diagnostic_effects,
                    *kind,
                    *global,
                    &old,
                    values,
                );
            },
        );
        if let Some(record) = record {
            command.retain_assignment_mutation(record);
        }
        Ok(ReplayStep::Continue)
    }
}

#[allow(clippy::too_many_arguments)] // final native token owner, scope, and demanded observation
pub(in crate::main_control) fn execute_toks<G>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    state: &CommandState<G>,
    receipts: &mut Option<Vec<MutationRecord>>,
    index: u16,
    tokens: Option<tex_state::TokenListId<G>>,
    global: bool,
    parameter: bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    let mut committer = AssignmentCommitter::new(stores, effects);
    let committed = if parameter {
        committer.token_parameter(index, tokens, global)
    } else {
        committer.toks(index, tokens, global)
    };
    if let Some(receipts) = receipts
        && committed
    {
        let new = if parameter {
            stores.token_parameter(TokParam::new(index))
        } else {
            stores.token_register(index)
        }
        .expect("admitted token assignment");
        let words = new
            .as_ref()
            .map(|tokens| {
                tokens
                    .words()
                    .iter()
                    .map(|word| observed_macro_token(word.semantic_token(), stores))
                    .collect()
            })
            .unwrap_or_default();
        receipts.push(MutationRecord {
            target: if parameter {
                MutationTarget::Parameter
            } else {
                MutationTarget::Register
            },
            key: ObservationValue::Name(if parameter {
                parameter_mutation_key_for_dialect(
                    state.profile().dialect(),
                    ParameterClass::Token,
                    index,
                )
            } else {
                format!("toks:{index}")
            }),
            value: ObservationValue::Tokens(words),
            global,
        });
    }
    Ok(ReplayStep::Continue)
}
