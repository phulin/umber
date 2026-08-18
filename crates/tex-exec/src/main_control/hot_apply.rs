//! Allocation-free semantic application for the measured common commands.
//!
//! These handlers are the single semantic owner for TeX82 §§1211--1234's
//! definition, let, prefix-result, group, and catcode families. Scanning still
//! produces a typed operand during the staged migration, but application
//! borrows it in place: it neither clones the universal step nor formats
//! detached evidence unless an observer is attached.

use super::*;

/// Applies one measured common step, returning `None` for the cold dispatcher.
pub(super) fn apply(
    scanned: &ScannedStep,
    stores: &mut Universe,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_>,
) -> Option<Result<ReplayStep, ExecError>> {
    let result = match scanned {
        ScannedStep::MacroDefinition {
            target,
            flags,
            global,
            parameter_text,
            replacement_text,
            definition_origin,
        } => apply_macro_definition(
            *target,
            *flags,
            *global,
            parameter_text,
            replacement_text,
            definition_origin,
            stores,
            command,
        ),
        ScannedStep::Let {
            target,
            source,
            meaning,
            macro_root,
            global,
        } => {
            // Keep the source spelling and macro definition strongly rooted
            // through the dense-cell replacement. They are semantically cold
            // but prevent the copied meaning from outliving its owner.
            let _strong_roots = (source, macro_root);
            apply_let(*target, *meaning, *global, stores, command)
        }
        ScannedStep::CodeTable {
            primitive: UnexpandablePrimitive::CatCode,
            character,
            value,
            global,
        } => apply_catcode(*character, *value, *global, stores, command),
        ScannedStep::BeginOrdinaryGroup => flush_group_boundary(modes, stores, command).map(|()| {
            enter_group(stores, command.state, GroupKind::Simple);
            ReplayStep::Continue
        }),
        ScannedStep::BeginSemiSimpleGroup => {
            flush_group_boundary(modes, stores, command).map(|()| {
                enter_group(stores, command.state, GroupKind::SemiSimple);
                ReplayStep::Continue
            })
        }
        ScannedStep::EndOrdinaryGroup => leave_group(
            GroupKind::Simple,
            "ordinary simple group",
            modes,
            stores,
            command,
        ),
        ScannedStep::EndSemiSimpleGroup => leave_group(
            GroupKind::SemiSimple,
            "semi simple group",
            modes,
            stores,
            command,
        ),
        _ => return None,
    };
    Some(result)
}

#[allow(clippy::too_many_arguments)]
fn apply_macro_definition(
    target: Symbol,
    flags: MeaningFlags,
    global: bool,
    parameter_text: &TracedTokenList,
    replacement_text: &TracedTokenList,
    definition_origin: &tex_state::provenance::OriginRef,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<ReplayStep, ExecError> {
    let meaning = MacroMeaning::new(
        flags,
        parameter_text.token_list(),
        replacement_text.token_list(),
    );
    let provenance = MacroDefinitionProvenance::new(
        definition_origin.clone(),
        parameter_text.origin_ref().clone(),
        replacement_text.origin_ref().clone(),
    );
    assignment_tracing::trace_meaning_write(stores, Token::Cs(target), true, global, |stores| {
        if global {
            stores.set_macro_meaning_global_with_provenance(target, meaning, provenance)
        } else {
            stores.set_macro_meaning_with_provenance(target, meaning, provenance)
        }
    });

    // TeX82 §1211's trace seam reports the stored body. Walking that body is
    // cold evidence publication, never part of an unobserved definition.
    if command.observes_mutations() {
        let record = MutationRecord {
            target: MutationTarget::Meaning,
            key: ObservationValue::Name(stores.resolve(target).to_owned()),
            value: ObservationValue::Tokens(observed_stored_macro_body(
                flags,
                parameter_text.token_list(),
                replacement_text.token_list(),
                stores,
            )),
            global,
        };
        command.retain_assignment_receipt(
            crate::assignments::committer::MutationReceipt::observed(record),
        );
    }
    Ok(ReplayStep::Continue)
}

fn apply_let(
    target: Symbol,
    meaning: Meaning,
    global: bool,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<ReplayStep, ExecError> {
    let committed = AssignmentCommitter::new(stores).direct_meaning(
        target,
        Token::Cs(target),
        meaning,
        global,
        |stores| {
            if global {
                stores.set_meaning_global(target, meaning)
            } else {
                stores.set_meaning(target, meaning)
            }
        },
    );
    if committed && command.observes_mutations() {
        let record = MutationRecord {
            target: MutationTarget::Meaning,
            key: ObservationValue::Name(stores.resolve(target).to_owned()),
            value: meaning_mutation_value(meaning, stores),
            global,
        };
        command.retain_assignment_receipt(
            crate::assignments::committer::MutationReceipt::observed(record),
        );
    }
    Ok(ReplayStep::Continue)
}

fn apply_catcode(
    character: char,
    raw_value: i32,
    global: bool,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<ReplayStep, ExecError> {
    let mut value = raw_value;
    if !(0..=15).contains(&value) {
        let context = command.state.output_open_context(&stores.command_context());
        let mut report = stores.print_err("Invalid code (");
        report
            .print_int(value)
            .print("), should be in the range 0..")
            .print_int(15)
            .help(&["I changed this one to zero."])
            .context(context);
        report.error().jump_out()?;
        value = 0;
    }
    let catcode = catcode_from_value(value)?;
    let old = stores.catcode(character);
    let committed = AssignmentCommitter::new(stores).direct_scoped_word(
        old,
        catcode,
        global,
        |stores, global| {
            if global {
                stores.set_catcode_global(character, catcode)
            } else {
                stores.set_catcode(character, catcode)
            }
        },
        |stores, _| {
            assignment_tracing::trace_code(
                stores,
                "catcode",
                character,
                global,
                old as i32,
                catcode as i32,
            )
        },
    );
    if committed && command.observes_mutations() {
        let record = MutationRecord {
            target: MutationTarget::Catcode,
            key: ObservationValue::Character(u32::from(character)),
            value: ObservationValue::Name(
                tex_command::canonical_names::catcode_assignment_name(i64::from(value))
                    .expect("validated category code")
                    .into(),
            ),
            global,
        };
        command.retain_assignment_receipt(
            crate::assignments::committer::MutationReceipt::observed(record),
        );
    }
    Ok(ReplayStep::Continue)
}

fn catcode_from_value(value: i32) -> Result<Catcode, ExecError> {
    match value {
        0 => Ok(Catcode::Escape),
        1 => Ok(Catcode::BeginGroup),
        2 => Ok(Catcode::EndGroup),
        3 => Ok(Catcode::MathShift),
        4 => Ok(Catcode::AlignmentTab),
        5 => Ok(Catcode::EndLine),
        6 => Ok(Catcode::Parameter),
        7 => Ok(Catcode::Superscript),
        8 => Ok(Catcode::Subscript),
        9 => Ok(Catcode::Ignored),
        10 => Ok(Catcode::Space),
        11 => Ok(Catcode::Letter),
        12 => Ok(Catcode::Other),
        13 => Ok(Catcode::Active),
        14 => Ok(Catcode::Comment),
        15 => Ok(Catcode::Invalid),
        _ => Err(ExecError::InvalidCode {
            context: "\\catcode",
            value,
        }),
    }
}

fn flush_group_boundary(
    modes: &mut ModeNest,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<(), ExecError> {
    crate::box_runtime::flush_pending_hchars_with_fuel(modes, stores, command.fuel)
}

fn leave_group(
    kind: GroupKind,
    context: &'static str,
    modes: &mut ModeNest,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<ReplayStep, ExecError> {
    flush_group_boundary(modes, stores, command)?;
    warn_cross_file_group_close(stores, command);
    let aftergroup = stores
        .leave_group_with_kind(kind)
        .map_err(|_| ExecError::MissingToken { context })?;
    schedule_aftergroup(command, stores, aftergroup)?;
    Ok(ReplayStep::Continue)
}
