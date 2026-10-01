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
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).meaning(
            *target,
            Token::Cs(*target),
            meaning,
            observed,
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
        command.retain_assignment_receipt(receipt);
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
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).meaning(
            *target,
            Token::Cs(*target),
            meaning,
            ObservationValue::Name(observed_name.into()),
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
        command.retain_assignment_receipt(receipt);
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
        AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |stores| {
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
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
            .unscoped_with_effects(record, |stores, diagnostic_effects| {
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
            });
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_toks<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    tokens: &mut Option<tex_state::TokenListId<G>>,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let new = tokens.as_ref();
        let observed = ObservationValue::Tokens(
            new.map(|new| {
                stores
                    .token_list(new.clone())
                    .iter()
                    .map(|word| observed_macro_token(word.semantic_token(), stores))
                    .collect()
            })
            .unwrap_or_default(),
        );
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).toks(
            *index,
            tokens.take(),
            observed,
            *global,
        );
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_tok_param<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    index: &mut u16,
    tokens: &mut Option<tex_state::TokenListId<G>>,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let new = tokens.as_ref();
        let observed = ObservationValue::Tokens(
            new.map(|tokens| {
                stores
                    .token_list(tokens.clone())
                    .iter()
                    .map(|word| observed_macro_token(word.semantic_token(), stores))
                    .collect()
            })
            .unwrap_or_default(),
        );
        let key = parameter_mutation_key_for_dialect(
            command.state.profile().dialect(),
            ParameterClass::Token,
            *index,
        );
        let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).token_parameter(
            *index,
            tokens.take(),
            observed,
            key,
            *global,
        );
        command.retain_assignment_receipt(receipt);
        Ok(ReplayStep::Continue)
    }
}
