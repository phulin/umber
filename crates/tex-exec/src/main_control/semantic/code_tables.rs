//! Direct code tables semantics on native owners.
use super::super::*;

// Borrow the actual owners and pass concrete operands without a command packet.
#[allow(clippy::too_many_arguments)]
pub(in crate::main_control) fn execute_code_table<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    state: &CommandState<G>,
    effects: &mut DiagnosticEffects,
    receipts: &mut Option<Vec<MutationRecord>>,
    primitive: UnexpandablePrimitive,
    character: char,
    value: i32,
    global: bool,
) -> Result<MainControlStep, ExecError> {
    // TeX82 §§1230--1234's `def_code` reports an invalid table
    // value after scanning it, substitutes zero, and commits the
    // assignment. In particular, this is not an operation failure:
    // rolling the command snapshot back would reread the operand and
    // lose ownership of the following input.
    let value = recover_code_table_value(primitive, value, stores, state, effects)?;
    match primitive {
        UnexpandablePrimitive::CatCode => {
            unreachable!("measured catcode assignments are owned by definition_assignments")
        }
        UnexpandablePrimitive::LcCode => {
            let value = u32::try_from(value).map_err(|_| ExecError::InvalidCode {
                context: "\\lccode",
                value,
            })? as LcCode;
            let old = stores.lccode(character);

            let receipt = AssignmentCommitter::new(stores, effects).direct_scoped_word(
                old,
                value,
                global,
                |stores, global| {
                    stores
                        .assign_code(
                            tex_state::env::CodeTableKind::Lccode,
                            character,
                            i64::from(value),
                            assignment_scope(global),
                        )
                        .expect("lccode target belongs to admitted state")
                },
                |stores, diagnostic_effects, _| {
                    assignment_tracing::trace_code(
                        stores,
                        diagnostic_effects,
                        "lccode",
                        character,
                        global,
                        old as i32,
                        value as i32,
                    )
                },
            );
            if receipt && let Some(receipts) = receipts.as_mut() {
                receipts.push(code_table_mutation(
                    "lccode",
                    character,
                    value as i64,
                    global,
                ));
            }
        }
        UnexpandablePrimitive::UcCode => {
            let value = checked_character_code(value, "\\uccode")? as UcCode;
            let old = stores.uccode(character);

            let receipt = AssignmentCommitter::new(stores, effects).direct_scoped_word(
                old,
                value,
                global,
                |stores, global| {
                    stores
                        .assign_code(
                            tex_state::env::CodeTableKind::Uccode,
                            character,
                            i64::from(value),
                            assignment_scope(global),
                        )
                        .expect("uccode target belongs to admitted state")
                },
                |stores, diagnostic_effects, _| {
                    assignment_tracing::trace_code(
                        stores,
                        diagnostic_effects,
                        "uccode",
                        character,
                        global,
                        old as i32,
                        value as i32,
                    )
                },
            );
            if receipt && let Some(receipts) = receipts.as_mut() {
                receipts.push(code_table_mutation(
                    "uccode",
                    character,
                    value as i64,
                    global,
                ));
            }
        }
        UnexpandablePrimitive::SfCode => {
            let value = u16::try_from(value)
                .ok()
                .filter(|value| *value <= 32_767)
                .ok_or(ExecError::InvalidCode {
                    context: "\\sfcode",
                    value,
                })? as SfCode;
            let old = stores.sfcode(character);

            let receipt = AssignmentCommitter::new(stores, effects).direct_scoped_word(
                old,
                value,
                global,
                |stores, global| {
                    stores
                        .assign_code(
                            tex_state::env::CodeTableKind::Sfcode,
                            character,
                            i64::from(value),
                            assignment_scope(global),
                        )
                        .expect("sfcode target belongs to admitted state")
                },
                |stores, diagnostic_effects, _| {
                    assignment_tracing::trace_code(
                        stores,
                        diagnostic_effects,
                        "sfcode",
                        character,
                        global,
                        i32::from(old),
                        i32::from(value),
                    )
                },
            );
            if receipt && let Some(receipts) = receipts.as_mut() {
                receipts.push(code_table_mutation(
                    "sfcode",
                    character,
                    i64::from(value),
                    global,
                ));
            }
        }
        UnexpandablePrimitive::MathCode => {
            let value = u32::try_from(value)
                .ok()
                .filter(|value| *value <= 32_768)
                .ok_or(ExecError::InvalidCode {
                    context: "\\mathcode",
                    value,
                })? as MathCode;
            let old = stores.mathcode(character);

            let receipt = AssignmentCommitter::new(stores, effects).direct_scoped_word(
                old,
                value,
                global,
                |stores, global| {
                    stores
                        .assign_code(
                            tex_state::env::CodeTableKind::Mathcode,
                            character,
                            i64::from(value),
                            assignment_scope(global),
                        )
                        .expect("mathcode target belongs to admitted state")
                },
                |stores, diagnostic_effects, _| {
                    assignment_tracing::trace_code(
                        stores,
                        diagnostic_effects,
                        "mathcode",
                        character,
                        global,
                        old as i32,
                        value as i32,
                    )
                },
            );
            if receipt && let Some(receipts) = receipts.as_mut() {
                receipts.push(code_table_mutation(
                    "mathcode",
                    character,
                    value as i64,
                    global,
                ));
            }
        }
        UnexpandablePrimitive::DelCode => {
            let value = (-1..=0xFF_FFFF)
                .contains(&value)
                .then_some(value as DelCode)
                .ok_or(ExecError::InvalidCode {
                    context: "\\delcode",
                    value,
                })?;
            let old = stores.delcode(character);

            let receipt = AssignmentCommitter::new(stores, effects).direct_scoped_word(
                old,
                value,
                global,
                |stores, global| {
                    stores
                        .assign_code(
                            tex_state::env::CodeTableKind::Delcode,
                            character,
                            i64::from(value),
                            assignment_scope(global),
                        )
                        .expect("delcode target belongs to admitted state")
                },
                |stores, diagnostic_effects, _| {
                    assignment_tracing::trace_code(
                        stores,
                        diagnostic_effects,
                        "delcode",
                        character,
                        global,
                        old,
                        value,
                    )
                },
            );
            if receipt && let Some(receipts) = receipts.as_mut() {
                receipts.push(code_table_mutation(
                    "delcode",
                    character,
                    i64::from(value),
                    global,
                ));
            }
        }
        _ => unreachable!("only code-table primitives are scanned"),
    }
    Ok(MainControlStep::Continue)
}
