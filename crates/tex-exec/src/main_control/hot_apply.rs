//! Fused scanning and allocation-free application for common commands.
//!
//! These handlers are the single semantic owner for TeX82 §§1211--1234's
//! definition, let, prefix-result, group, and catcode families. They scan into
//! a family-sized typed operand and apply it immediately after the command
//! processor releases its borrow. `ColdOperation` and
//! `PreparedColdOperation` never exist on this path.

use super::*;

/// The complete operand of one measured common operation.
///
/// This enum is intentionally private to the fused scan/apply seam. It is not
/// a second executor or a durable continuation: it exists only long enough to
/// release `CommandProcessor`'s borrow of `Universe` before stomach mutation.
/// Keeping the scanned definition inline preserves the allocation-free common
/// command path despite the deliberate variant size difference.
#[allow(clippy::large_enum_variant)]
pub(super) enum HotOperation<G> {
    MacroDefinition {
        definition: tex_command::ScannedMacroDefinition,
        flags: MeaningFlags,
        global: bool,
    },
    Let {
        assignment: tex_command::ScannedLetAssignment<G>,
        global: bool,
    },
    CatCode {
        character: char,
        value: i32,
        global: bool,
    },
    EnterGroup(GroupKind),
    LeaveGroup {
        kind: GroupKind,
        context: &'static str,
    },
}

impl<G> HotOperation<G> {
    pub(super) const fn begin_ordinary_group() -> Self {
        Self::EnterGroup(GroupKind::Simple)
    }

    pub(super) const fn end_ordinary_group() -> Self {
        Self::LeaveGroup {
            kind: GroupKind::Simple,
            context: "ordinary simple group",
        }
    }

    pub(super) const fn fires_afterassignment(&self) -> bool {
        matches!(
            self,
            Self::MacroDefinition { .. } | Self::Let { .. } | Self::CatCode { .. }
        )
    }

    pub(super) fn protected_definition_observation(
        &self,
        stores: &Universe<G>,
    ) -> Option<TokenListRecord> {
        let Self::MacroDefinition {
            definition, flags, ..
        } = self
        else {
            return None;
        };
        flags
            .contains(MeaningFlags::PROTECTED)
            .then(|| TokenListRecord {
                transition: "complete",
                purpose: "protected_macro",
                tokens: {
                    let mut tokens = definition
                        .parameter_text
                        .words()
                        .iter()
                        .map(|word| match word.semantic_token() {
                            Token::Param(_) => ObservedToken::MacroMatch,
                            token => observed_macro_token(token, stores),
                        })
                        .collect::<Vec<_>>();
                    tokens.push(ObservedToken::MacroEndMatch);
                    tokens.extend(
                        definition
                            .replacement_text
                            .words()
                            .iter()
                            .map(|word| observed_macro_token(word.semantic_token(), stores)),
                    );
                    tokens
                },
            })
    }
}

/// Scans a ranked common command after §1211's prefix loop and all contextual
/// mode/group cases have selected the ordinary assignment arm.
pub(super) fn scan<G>(
    processor: &mut CommandProcessor<'_, G>,
    command: &tex_command::CurrentCommand<G>,
    global: bool,
    flags: MeaningFlags,
    innermost_group: Option<GroupKind>,
) -> Result<Option<HotOperation<G>>, ExecError> {
    let operation = match command.meaning() {
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        } => HotOperation::begin_ordinary_group(),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginGroup) => {
            HotOperation::EnterGroup(GroupKind::SemiSimple)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup)
            if innermost_group == Some(GroupKind::SemiSimple) =>
        {
            HotOperation::LeaveGroup {
                kind: GroupKind::SemiSimple,
                context: "semi simple group",
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CatCode) => {
            let character = processor
                .scan_restricted_integer(RestrictedIntegerClass::CharacterCode)
                .map_err(command_error)?
                .value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            HotOperation::CatCode {
                character: char::from_u32(character as u32)
                    .expect("scan_char_num returns a valid character"),
                value,
                global,
            }
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef),
        ) => HotOperation::MacroDefinition {
            definition: processor
                .scan_macro_definition(matches!(
                    primitive,
                    UnexpandablePrimitive::Edef | UnexpandablePrimitive::Xdef
                ))
                .map_err(command_error)?,
            flags,
            global,
        },
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Let | UnexpandablePrimitive::FutureLet),
        ) => HotOperation::Let {
            assignment: processor
                .scan_let_assignment(primitive == UnexpandablePrimitive::FutureLet)
                .map_err(command_error)?,
            global,
        },
        _ => return Ok(None),
    };
    Ok(Some(operation))
}

/// Applies one measured common operation to canonical state and journals.
pub(super) fn apply<G>(
    operation: &HotOperation<G>,
    stores: &mut Universe<G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, G>,
) -> Result<ReplayStep, ExecError> {
    match operation {
        HotOperation::MacroDefinition {
            definition,
            flags,
            global,
        } => apply_macro_definition(
            definition.target,
            *flags,
            *global,
            &definition.parameter_text,
            &definition.replacement_text,
            &definition.definition_origin,
            stores,
            command,
        ),
        HotOperation::Let { assignment, global } => {
            // Keep the source spelling and macro definition strongly rooted
            // through the dense-cell replacement. They are semantically cold
            // but prevent the copied meaning from outliving its owner.
            let _strong_roots = (&assignment.source, &assignment.macro_root);
            apply_let(
                assignment.target,
                assignment.meaning,
                *global,
                stores,
                command,
            )
        }
        HotOperation::CatCode {
            character,
            value,
            global,
        } => apply_catcode(*character, *value, *global, stores, command),
        HotOperation::EnterGroup(kind) => flush_group_boundary(modes, stores, command).map(|()| {
            enter_group(stores, command.state, *kind);
            ReplayStep::Continue
        }),
        HotOperation::LeaveGroup { kind, context } => {
            leave_group(*kind, context, modes, stores, command)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_macro_definition(
    target: Symbol,
    flags: MeaningFlags,
    global: bool,
    parameter_text: &tex_state::token::RootedTracedTokenBuffer,
    replacement_text: &tex_state::token::RootedTracedTokenBuffer,
    definition_origin: &tex_state::provenance::OriginRef,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<ReplayStep, ExecError> {
    assignment_tracing::trace_meaning_write(stores, Token::Cs(target), true, global, |stores| {
        stores.set_macro_meaning_from_buffers(
            target,
            flags,
            parameter_text,
            replacement_text,
            definition_origin.clone(),
            global,
        )
    });

    // TeX82 §1211's trace seam reports the stored body. Walking that body is
    // cold evidence publication, never part of an unobserved definition.
    if command.observes_mutations() {
        let meaning = stores
            .macro_meaning(target)
            .expect("new macro definition is installed");
        let record = MutationRecord {
            target: MutationTarget::Meaning,
            key: ObservationValue::Name(stores.resolve(target).to_owned()),
            value: ObservationValue::Tokens(observed_stored_macro_body(
                flags,
                meaning.parameter_text(),
                meaning.replacement_text(),
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

fn leave_group<G>(
    kind: GroupKind,
    context: &'static str,
    modes: &mut ModeNest,
    stores: &mut Universe<G>,
    command: &mut CommandMachine<'_, G>,
) -> Result<ReplayStep, ExecError> {
    flush_group_boundary(modes, stores, command)?;
    warn_cross_file_group_close(stores, command);
    let aftergroup = leave_group_payloads(stores, command.state, kind)
        .map_err(|_| ExecError::MissingToken { context })?;
    schedule_aftergroup(command, stores, aftergroup)?;
    Ok(ReplayStep::Continue)
}
