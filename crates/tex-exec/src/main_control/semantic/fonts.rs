//! Direct fonts semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_font_dimen<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    font: FontId,
    number: i32,
    value: Scaled,
    recovery_context: Option<String>,
) -> Result<ReplayStep, ExecError> {
    // tex.web §578's `find_font_dimen` resolves an unusable parameter
    // number to the scratch location `fmem_ptr`; §579 then reports
    // "Font x has only n fontdimen parameters" and §1253 still runs
    // `scan_optional_equals; scan_normal_dimen; font_info[k].sc:=
    // cur_val` into that scratch cell, so the font is unchanged and
    // the job continues. Only §580's grow path, which
    // `set_font_dimen` implements, can add a parameter.
    //
    // The scan already made §578's decision and captured §579's
    // context there, so this only writes or reports.
    match recovery_context {
        Some(context) => report_font_parameter_recovery(stores, effects, font, context, None)?,
        None => {
            let number =
                u32::try_from(number).expect("a writable parameter number is a positive u32");
            match AssignmentCommitter::new(stores, effects)
                .try_unscoped(|stores| stores.set_font_dimen(font, number, value))
            {
                Ok(_) => {}
                Err(capacity) => {
                    return Err(ExecError::Fatal(tex_command::FatalError::overflow(
                        "font memory",
                        i32::try_from(capacity).expect("font capacity fits TeX integer"),
                    )));
                }
            }
        }
    }
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn execute_font_integer<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    font: FontId,
    skew: bool,
    value: i32,
) -> Result<ReplayStep, ExecError> {
    AssignmentCommitter::new(stores, effects).unscoped(|stores| {
        if skew {
            stores.set_font_skew_char(font, value)
        } else {
            stores.set_font_hyphen_char(font, value)
        }
    });
    Ok(ReplayStep::Continue)
}

// Borrow the actual owners and pass concrete operands without a command packet.
#[allow(clippy::too_many_arguments)]
pub(in crate::main_control) fn execute_hyphenation_data<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    initex: bool,
    words: Vec<Vec<char>>,
    pattern_specs: Vec<tex_state::hyphenation::PatternSpec>,
    patterns: bool,
    rejection_context: String,
    trie_built: bool,
) -> Result<ReplayStep, ExecError> {
    // TeX82 §1252 rejects `\patterns` for two different reasons, and
    // the two do not share a message. The `init`/`tini` split comes
    // first: a production binary has no `new_patterns` to call, so it
    // reports "Patterns can be loaded only by INITEX" with `help0`
    // and flushes the braced group. Only INITEX reaches §960, whose
    // own `trie_not_ready=false` guard is the "Too late" one, and it
    // does carry help. `\hyphenation` is legal in both binaries.
    if patterns && !initex {
        let mut report = stores.print_err("Patterns can be loaded only by INITEX");
        report.context(rejection_context);
        report.error().defer_recovery(effects)?;
        return Ok(ReplayStep::Continue);
    }
    if trie_built {
        let mut report = stores.print_err("Too late for \\patterns");
        report.help(&["All patterns must be given before typesetting begins."]);
        report.context(rejection_context);
        report.error().defer_recovery(effects)?;
        return Ok(ReplayStep::Continue);
    }
    // Both halves of §§935/963's diagnostics were already reported by
    // the live scan, where §82 could still show the character that
    // caused them; installing is all that is left here.
    if patterns {
        AssignmentCommitter::new(stores, effects).try_unscoped(|stores| {
            crate::paragraph_end::apply_scanned_patterns(stores, pattern_specs)
        })?;
    } else {
        AssignmentCommitter::new(stores, effects).unscoped(|stores| {
            crate::paragraph_end::apply_scanned_hyphenation_exceptions(stores, words);
        });
    }
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn execute_font_select<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    font: FontId,
    global: bool,
) -> Result<ReplayStep, ExecError> {
    AssignmentCommitter::new(stores, effects).unscoped(|stores| {
        stores
            .assign_current_font(font, assignment_scope(global))
            .expect("current font belongs to admitted state")
    });
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn execute_font_definition<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    font_resource: Option<&FontResource>,
    request: &mut FontLoadRequest,
    global: &mut bool,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1257 records `font_id_text(f):=t` at `common_ending`,
        // including the `f=null_font` recovery path. Active targets use
        // the synthesized string `FONT<char>` rather than their bare
        // control-sequence text.
        let identifier = font_identifier_for_definition(stores, request.target);
        let observe_font_definition =
            command.observes_mutations() && command.state.profile().capabilities().supports_etex();
        // TeX82 §1258/§1259 report an illegal `at`/`scaled` size and
        // continue with the replaced value; §1257 then loads the font
        // normally. The replacement is the scanner's, the report this
        // seam's.
        if let Some(recovery) = &request.size_recovery {
            report_font_size_recovery(stores, command.diagnostic_effects, recovery)?;
        }
        let bind_null_font =
            |stores: &mut tex_state::CommandContext<'_, G>,
             diagnostic_effects: &mut DiagnosticEffects| {
                let record = font_definition_mutation(
                    stores,
                    request.target,
                    *global,
                    observe_font_definition,
                );
                AssignmentCommitter::new(stores, diagnostic_effects).unscoped(|stores| {
                    stores
                        .assign_meaning(
                            request.target,
                            tex_state::MeaningWord::from_static(Meaning::Font(
                                tex_state::font::NULL_FONT,
                            )),
                            assignment_scope(*global),
                        )
                        .expect("font selector belongs to admitted state");
                    stores.set_font_identifier_symbol(tex_state::font::NULL_FONT, identifier);
                });
                record
            };
        let resource = font_resource.expect("font resource is resolved before application");
        if matches!(resource, FontResource::Unavailable) {
            // TeX.web §§1257/561 diagnose the failed TFM open before
            // continuing with the selector bound to `null_font`. The
            // scanner has already backed up the delimiter token, so the
            // live input context also shows the command that follows the
            // font specification.
            let selector = stores.resolve(request.target).to_owned();
            let selector_kind = stores.control_sequence_kind(request.target);
            report_font_not_loadable_with_context(
                stores,
                command.diagnostic_effects,
                selector_kind,
                &selector,
                &request.name,
                request.size,
                if request.name.starts_with("opentype:") {
                    FontLoadFailure::MissingOpenType
                } else {
                    FontLoadFailure::MissingTfm
                },
                request.error_context.clone(),
            )?;
            let receipt = bind_null_font(stores, command.diagnostic_effects);
            if let Some(record) = receipt {
                command.retain_assignment_mutation(record);
            }
            return Ok(ReplayStep::Continue);
        }
        let loaded = match load_font(request, resource) {
            Ok(loaded) => loaded,
            Err(ExecError::FontParse(_)) => {
                // TeX.web §564 treats malformed metrics exactly like the
                // recoverable not-loadable path. The fulfilled resource
                // remains retained by the host; only this definition is
                // replaced by nullfont with its requested assignment scope.
                let selector = stores.resolve(request.target).to_owned();
                let selector_kind = stores.control_sequence_kind(request.target);
                report_font_not_loadable_with_context(
                    stores,
                    command.diagnostic_effects,
                    selector_kind,
                    &selector,
                    &request.name,
                    request.size,
                    FontLoadFailure::MalformedTfm,
                    request.error_context.clone(),
                )?;
                let receipt = bind_null_font(stores, command.diagnostic_effects);
                if let Some(record) = receipt {
                    command.retain_assignment_mutation(record);
                }
                return Ok(ReplayStep::Continue);
            }
            Err(error) => return Err(error),
        };
        let mut loaded = Some(loaded);
        let id = match stores.try_intern_font_with_identifier(&mut loaded, identifier) {
            Ok(id) => id,
            Err(_) => {
                let selector = stores.resolve(request.target).to_owned();
                let selector_kind = stores.control_sequence_kind(request.target);
                report_font_capacity(
                    stores,
                    command.diagnostic_effects,
                    selector_kind,
                    &selector,
                    &request.name,
                    request.size,
                    request.error_context.clone(),
                )?;
                let receipt = bind_null_font(stores, command.diagnostic_effects);
                if let Some(record) = receipt {
                    command.retain_assignment_mutation(record);
                }
                return Ok(ReplayStep::Continue);
            }
        };
        // Web2C tex.ch [49.1260] removes TeX82 §1254's `flush_string`:
        // [29.517]'s `slow_make_string` may have returned an older pool
        // string, so flushing it would retire an unrelated allocation.
        let record =
            font_definition_mutation(stores, request.target, *global, observe_font_definition);
        AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(|stores| {
            stores
                .assign_meaning(
                    request.target,
                    tex_state::MeaningWord::from_static(Meaning::Font(id)),
                    assignment_scope(*global),
                )
                .expect("font selector belongs to admitted state")
        });
        if let Some(record) = record {
            command.retain_assignment_mutation(record);
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_generated_font_definition<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    receipts: &mut Option<Vec<MutationRecord>>,
    definition: ScannedGeneratedFontDefinition,
    global: bool,
) -> Result<ReplayStep, ExecError> {
    if definition.kind == GeneratedFontKind::Copy
        && matches!(
            stores.font_construction(definition.source),
            tex_fonts::FontConstruction::Letterspaced { .. }
                | tex_fonts::FontConstruction::Expanded { .. }
        )
    {
        let reason = match stores.font_construction(definition.source) {
            tex_fonts::FontConstruction::Expanded { .. } => "cannot copy an expanded font",
            _ => "cannot copy a letterspaced font",
        };
        return Err(ExecError::CannotCopyFont(reason));
    }
    let id = match definition.kind {
        GeneratedFontKind::Copy => {
            stores.try_copy_font_with_identifier(definition.source, definition.target)?
        }
        GeneratedFontKind::Letterspace => {
            let zero_em = stores.font_parameter(definition.source, 6).raw() == 0;
            let id = stores.try_letterspace_font_with_identifier(
                definition.source,
                definition.target,
                definition.amount,
                definition.no_ligatures,
            )?;
            if zero_em {
                stores.printer().print_rendered(
                    "\npdfTeX warning (\\letterspacefont): font has zero em size (\\fontdimen6)\n",
                );
            }
            id
        }
    };
    let record = font_definition_mutation(stores, definition.target, global, receipts.is_some());
    AssignmentCommitter::new(stores, effects).unscoped(|stores| {
        stores
            .assign_meaning(
                definition.target,
                tex_state::MeaningWord::from_static(Meaning::Font(id)),
                assignment_scope(global),
            )
            .expect("generated-font selector belongs to admitted state")
    });
    if let Some(record) = record {
        receipts
            .as_mut()
            .expect("requested mutation receipt")
            .push(record);
    }
    Ok(ReplayStep::Continue)
}
