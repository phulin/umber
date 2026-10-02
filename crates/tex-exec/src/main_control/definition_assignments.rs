//! Concrete definition, let, catcode, and group mutation helpers.
//!
//! TeX82 §§1211--1234 scan through the sole direct engine and pass completed
//! native operands here after ending the processor loan. Published definitions
//! enter the actual destination meaning cell; no operation carrier is created.

use super::*;

pub(super) fn apply_macro_definition<G>(
    target: Symbol,
    definition: tex_state::PublishedDefinition<G>,
    flags: MeaningFlags,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let observed_definition = command
        .observes_mutations()
        .then_some(definition.reference());
    commit_macro_definition(
        target,
        definition,
        flags,
        global,
        stores,
        command.diagnostic_effects,
    );

    // TeX82 §1211's trace seam reports the stored body. Walking that body is
    // cold evidence publication, never part of an unobserved definition.
    if let Some(definition) = observed_definition {
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

/// Commits §1218's macro definition and its §1211 assignment trace.
pub(super) fn commit_macro_definition<G>(
    target: Symbol,
    definition: tex_state::PublishedDefinition<G>,
    flags: MeaningFlags,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
) {
    assignment_tracing::trace_meaning_write(
        stores,
        diagnostic_effects,
        Token::Cs(target),
        true,
        global,
        |stores| {
            stores
                .assign_published_definition(target, definition, flags, assignment_scope(global))
                .expect("macro target belongs to the admitted generation");
        },
    );
}

pub(super) fn apply_let<G>(
    target: Symbol,
    meaning: tex_state::meaning::ResolvedMeaning<G>,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let committed = commit_let(target, meaning, global, stores, command.diagnostic_effects)?;
    if let Some(meaning) = committed
        && command.observes_mutations()
    {
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

/// Commits §1221's `\let` through §277's `eq_define`, returning the
/// meaning it wrote, or `None` when e-TeX suppressed a redundant local
/// definition.
pub(super) fn commit_let<G>(
    target: Symbol,
    meaning: tex_state::meaning::ResolvedMeaning<G>,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
) -> Result<Option<tex_state::meaning::ResolvedMeaning<G>>, ExecError> {
    // TeX82 §§277/1221 always route `\let` through `eq_define`. e-TeX change
    // [19.277] (retained by pdftex.web §277) alone suppresses an identical
    // local definition while extended mode is active.
    let redundant = if !global && stores.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 {
        let current = stores.meaning(target);
        crate::assignments::committer::redundant_local_assignment(true, &current, &meaning, false)
    } else {
        false
    };
    let committed = !redundant;
    assignment_tracing::trace_meaning_write(
        stores,
        diagnostic_effects,
        Token::Cs(target),
        committed,
        global,
        |stores| {
            if committed {
                stores
                    .assign_resolved_meaning(target, meaning, assignment_scope(global))
                    .expect("let target belongs to the admitted generation");
            }
        },
    );
    Ok(committed.then_some(meaning))
}

pub(super) fn apply_catcode<G>(
    character: char,
    raw_value: i32,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let value =
        recover_code_table_value(UnexpandablePrimitive::CatCode, raw_value, stores, command)?;
    let catcode = catcode_from_value(value)?;
    let committed = commit_catcode(
        character,
        catcode,
        global,
        stores,
        command.diagnostic_effects,
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

/// Commits §1232's validated category code through §277's `eq_define`,
/// returning whether the code table changed.
pub(super) fn commit_catcode<G>(
    character: char,
    catcode: Catcode,
    global: bool,
    stores: &mut tex_state::CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
) -> bool {
    let old = stores.catcode(character);
    AssignmentCommitter::new(stores, diagnostic_effects).direct_scoped_word(
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
    )
}

pub(super) fn catcode_from_value(value: i32) -> Result<Catcode, ExecError> {
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
