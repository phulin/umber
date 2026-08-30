//! Fused scanning and allocation-free application for common commands.
//!
//! These handlers are the single semantic owner for TeX82 §§1211--1234's
//! definition, let, prefix-result, group, and catcode families. They scan into
//! a family-sized typed operand and apply it immediately after the command
//! processor releases its borrow. `ColdOperation` is not materialized on this
//! path. The caller-owned [`OperationFrame`] stores the hot result in its one
//! mutually exclusive operation-payload field, alongside its reusable scalar
//! destination; preparation and application borrow that resident value.

use super::*;

/// The complete operand of one measured common operation.
///
/// This enum is intentionally private to the fused scan/apply seam. It is not
/// a second executor or a durable continuation: it exists only long enough to
/// release `CommandProcessor`'s borrow of `Universe` before stomach mutation.
/// Keeping the scanned definition inline preserves the allocation-free common
/// command path despite the deliberate variant size difference.
pub(super) enum HotOperation<G> {
    MacroDefinition {
        target: Symbol,
        definition: HotDefinitionRoot<G>,
        flags: MeaningFlags,
        global: bool,
    },
    Let {
        target: Symbol,
        meaning: Option<tex_state::meaning::ResolvedMeaning<G>>,
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

pub(super) enum HotDefinitionRoot<G> {
    Attempt(tex_command::AttemptDefinitionId),
    Prepared(Option<tex_state::DefinitionId<G>>),
}

impl<G> HotDefinitionRoot<G> {
    fn prepare(
        &mut self,
        command: &tex_command::CommandState<G>,
        stores: &mut Universe<G>,
    ) -> Result<(), tex_command::AttemptError> {
        let Self::Attempt(attempt) = self else {
            panic!("hot definition root is prepared exactly once")
        };
        *self = Self::Prepared(Some(command.promote_attempt_definition(stores, *attempt)?));
        Ok(())
    }

    fn take_prepared(&mut self) -> tex_state::DefinitionId<G> {
        match self {
            Self::Prepared(definition) => definition
                .take()
                .expect("prepared hot definition root is consumed exactly once"),
            Self::Attempt(_) => panic!("hot definition root must be prepared before application"),
        }
    }
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
}

/// Promotes every declared hot-operation root before command-state admission.
pub(super) fn prepare<G>(
    operation: &mut HotOperation<G>,
    command: &tex_command::CommandState<G>,
    stores: &mut Universe<G>,
) -> Result<(), tex_command::AttemptError> {
    if let HotOperation::MacroDefinition { definition, .. } = operation {
        definition.prepare(command, stores)?;
    }
    Ok(())
}

/// Scans a ranked common command after §1211's prefix loop and all contextual
/// mode/group cases have selected the ordinary assignment arm.
pub(super) fn scan<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &mut OperationFrame<G>,
    global: bool,
    flags: MeaningFlags,
    innermost_group: Option<GroupKind>,
    suspended_operation_scan: &mut Option<PendingOperationScanPhase>,
) -> Result<bool, ExecError> {
    let operation = match command.meaning() {
        tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        }) => HotOperation::begin_ordinary_group(),
        tex_state::meaning::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::BeginGroup,
        )) => HotOperation::EnterGroup(GroupKind::SemiSimple),
        tex_state::meaning::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::EndGroup,
        )) if innermost_group == Some(GroupKind::SemiSimple) => HotOperation::LeaveGroup {
            kind: GroupKind::SemiSimple,
            context: "semi simple group",
        },
        tex_state::meaning::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::CatCode,
        )) => scan_catcode_assignment(
            processor,
            &mut command.scalar,
            global,
            CatCodeScanPhase::Character,
            suspended_operation_scan,
        )?,
        tex_state::meaning::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef),
        )) => {
            let definition = processor
                .scan_macro_definition(matches!(
                    primitive,
                    UnexpandablePrimitive::Edef | UnexpandablePrimitive::Xdef
                ))
                .map_err(command_error)?;
            HotOperation::MacroDefinition {
                target: definition.target,
                definition: HotDefinitionRoot::Attempt(definition.definition),
                flags,
                global,
            }
        }
        tex_state::meaning::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Let | UnexpandablePrimitive::FutureLet),
        )) => {
            let assignment = processor
                .scan_let_assignment(primitive == UnexpandablePrimitive::FutureLet)
                .map_err(command_error)?;
            HotOperation::Let {
                target: assignment.target,
                meaning: Some(assignment.meaning),
                global,
            }
        }
        _ => return Ok(false),
    };
    command.write_hot(operation);
    Ok(true)
}

pub(super) fn scan_catcode_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    global: bool,
    phase: CatCodeScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<HotOperation<G>, ExecError> {
    let phase = if matches!(phase, CatCodeScanPhase::Character) {
        let status =
            processor.scan_restricted_integer_into(RestrictedIntegerClass::CharacterCode, scalar);
        let character = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::CatCode {
                global,
                phase: CatCodeScanPhase::Character,
            },
            suspended,
            take_restricted
        )
        .value;
        CatCodeScanPhase::OptionalEquals {
            character: char::from_u32(character as u32)
                .expect("scan_char_num returns a valid character"),
        }
    } else {
        phase
    };
    let phase = match phase {
        CatCodeScanPhase::OptionalEquals { character } => {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::CatCode {
                    global,
                    phase: CatCodeScanPhase::OptionalEquals { character },
                },
                suspended,
                take_boolean
            );
            CatCodeScanPhase::Value { character }
        }
        phase => phase,
    };
    let CatCodeScanPhase::Value { character } = phase else {
        unreachable!()
    };
    let status = processor.scan_integer_into(scalar);
    let value = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::CatCode {
            global,
            phase: CatCodeScanPhase::Value { character },
        },
        suspended,
        take_integer
    )
    .value;
    Ok(HotOperation::CatCode {
        character,
        value,
        global,
    })
}

/// Applies one measured common operation to canonical state and journals.
pub(super) fn apply<G>(
    operation: &mut HotOperation<G>,
    stores: tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, G>,
) -> Result<ReplayStep, ExecError> {
    let mut stores = LinearCommandContext::new(stores);
    let stores = &mut stores;
    match operation {
        HotOperation::MacroDefinition {
            target,
            definition,
            flags,
            global,
        } => apply_macro_definition(
            *target,
            definition.take_prepared(),
            *flags,
            *global,
            stores,
            command,
        ),
        HotOperation::Let {
            target,
            meaning,
            global,
        } => apply_let(
            *target,
            meaning
                .take()
                .expect("prepared hot let meaning is consumed exactly once"),
            *global,
            stores,
            command,
        ),
        HotOperation::CatCode {
            character,
            value,
            global,
        } => apply_catcode(*character, *value, *global, stores, command),
        HotOperation::EnterGroup(kind) => flush_group_boundary(modes, stores, command).map(|()| {
            enter_group(stores, command.state, command.diagnostic_effects, *kind);
            ReplayStep::Continue
        }),
        HotOperation::LeaveGroup { kind, context } => {
            leave_group(*kind, context, modes, stores, command)
        }
    }
}

fn apply_macro_definition<G>(
    target: Symbol,
    definition: tex_state::DefinitionId<G>,
    flags: MeaningFlags,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, G>,
) -> Result<ReplayStep, ExecError> {
    assignment_tracing::trace_meaning_write(
        stores,
        command.diagnostic_effects,
        Token::Cs(target),
        true,
        global,
        |stores| {
            stores
                .assign_resolved_meaning(
                    target,
                    tex_state::meaning::ResolvedMeaning::Macro {
                        flags,
                        definition: definition.clone(),
                    },
                    assignment_scope(global),
                )
                .expect("macro target belongs to the admitted generation");
        },
    );

    // TeX82 §1211's trace seam reports the stored body. Walking that body is
    // cold evidence publication, never part of an unobserved definition.
    if command.observes_mutations() {
        let stored = stores.definition(definition);
        if flags.contains(MeaningFlags::PROTECTED) {
            // e-TeX change section [27.465] installs the protected marker as
            // a distinct token-list link after §477 has completed the macro
            // replacement. The canonical observer therefore sees this
            // transition before the definition becomes the target meaning.
            command.retain_hot_observation(CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: "protected_macro",
                tokens: observed_macro_body(
                    stored.parameter_text(),
                    stored.replacement_text(),
                    stores,
                ),
            }));
        }
        let record = MutationRecord {
            target: MutationTarget::Meaning,
            key: ObservationValue::Name(stores.resolve(target).to_owned()),
            value: ObservationValue::Tokens(observed_stored_macro_body(
                flags,
                stored.parameter_text(),
                stored.replacement_text(),
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

fn apply_let<G>(
    target: Symbol,
    meaning: tex_state::meaning::ResolvedMeaning<G>,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, G>,
) -> Result<ReplayStep, ExecError> {
    // TeX82 §§277/1221 always route `\let` through `eq_define`. e-TeX change
    // [19.277] (retained by pdftex.web §277) alone suppresses an identical
    // local definition while extended mode is active.
    let current = stores.meaning(target);
    let redundant = crate::assignments::committer::redundant_local_assignment(
        stores.int_param(IntParam::ETEX_EXTENDED_MODE) > 0,
        &current,
        &meaning,
        global,
    );
    let committed = !redundant;
    assignment_tracing::trace_meaning_write(
        stores,
        command.diagnostic_effects,
        Token::Cs(target),
        committed,
        global,
        |stores| {
            if committed {
                stores
                    .assign_resolved_meaning(target, meaning.clone(), assignment_scope(global))
                    .expect("let target belongs to the admitted generation");
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

fn apply_catcode<G>(
    character: char,
    raw_value: i32,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, G>,
) -> Result<ReplayStep, ExecError> {
    let mut value = raw_value;
    if !(0..=15).contains(&value) {
        let context = command.state.output_open_context(stores);
        let mut report = stores.print_err("Invalid code (");
        report
            .print_int(value)
            .print("), should be in the range 0..")
            .print_int(15)
            .help(&["I changed this one to zero."])
            .context(context);
        report.error().defer_recovery(command.diagnostic_effects)?;
        value = 0;
    }
    let catcode = catcode_from_value(value)?;
    let old = stores.catcode(character);
    let committed = AssignmentCommitter::new(stores, command.diagnostic_effects)
        .direct_scoped_word(
            old,
            catcode,
            global,
            |stores, global| {
                stores
                    .assign_code(
                        tex_state::env::CodeTableKind::Catcode,
                        character,
                        i64::from(catcode as u8),
                        assignment_scope(global),
                    )
                    .expect("catcode target belongs to the admitted generation");
            },
            |stores, diagnostic_effects, _| {
                assignment_tracing::trace_code(
                    stores,
                    diagnostic_effects,
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

fn flush_group_boundary<G>(
    modes: &mut ModeNest,
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, G>,
) -> Result<(), ExecError> {
    crate::box_runtime::flush_pending_hchars_with_fuel(
        modes,
        stores,
        command.diagnostic_effects,
        command.fuel,
    )
}

fn leave_group<G>(
    kind: GroupKind,
    context: &'static str,
    modes: &mut ModeNest,
    stores: &mut LinearCommandContext<'_, G>,
    command: &mut CommandMachine<'_, G>,
) -> Result<ReplayStep, ExecError> {
    flush_group_boundary(modes, stores, command)?;
    warn_cross_file_group_close(stores, command);
    let aftergroup = leave_group_payloads(stores, command.state, command.diagnostic_effects, kind)
        .map_err(|_| ExecError::MissingToken { context })?;
    schedule_aftergroup(command, stores, aftergroup)?;
    Ok(ReplayStep::Continue)
}
