use super::*;

pub(super) fn execute_aftergroup(
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let token = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\aftergroup",
    })?;
    stores.push_aftergroup(token);
    Ok(())
}

pub(super) fn execute_afterassignment(
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let token = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\afterassignment",
    })?;
    stores.set_afterassignment(token);
    Ok(())
}

pub(super) fn fire_afterassignment(input: &mut InputStack, stores: &mut Universe) -> bool {
    if let Some(token) = stores.take_afterassignment() {
        push_tokens(input, stores, [token]);
        true
    } else {
        false
    }
}

pub(super) fn execute_def(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let target = scan_traced_definition_target(input, stores, "macro definition")?;
    let expanded = matches!(
        primitive,
        UnexpandablePrimitive::Edef | UnexpandablePrimitive::Xdef
    );
    let global = prefixes.global
        || matches!(
            primitive,
            UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
        );
    let scanned = if expanded {
        scan_toks_expanded_with_driver(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            prefixes.flags,
            target.traced,
            execution,
        )?
    } else {
        scan_toks(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            prefixes.flags,
            target.traced,
        )?
    }
    .with_definition_origin(target.origin);
    for diagnostic in scanned.diagnostics() {
        let context = match diagnostic {
            MacroScanDiagnostic::UndefinedControlSequence { context, .. }
            | MacroScanDiagnostic::IllegalParameterNumber { context } => *context,
        };
        execution.record_macro_scan_error(context)?;
        match diagnostic {
            // TeX.web §370. The offending name is not part of the message:
            // §82's context display is what shows which control sequence it
            // was, so the report has to go through the funnel to name it.
            MacroScanDiagnostic::UndefinedControlSequence { .. } => {
                crate::error_report::report_input_error(
                    input,
                    stores,
                    "Undefined control sequence",
                    &[
                        "The control sequence at the end of the top line",
                        "of your error message was never \\def'ed. If you have",
                        "misspelled it (e.g., `\\hobx'), type `I' and the correct",
                        "spelling (e.g., `I\\hbox'). Otherwise just continue,",
                        "and I'll forget about whatever was undefined.",
                    ],
                )?;
            }
            // TeX.web §479 names `warning_index`, which for a macro
            // definition is the control sequence being defined.
            MacroScanDiagnostic::IllegalParameterNumber { .. } => {
                let name = definition_target_text(stores, target.symbol);
                crate::error_report::report_input_error(
                    input,
                    stores,
                    &format!("Illegal parameter number in definition of {name}"),
                    &[
                        "You meant to type ## instead of #, right?",
                        "Or maybe a } was forgotten somewhere earlier, and things",
                        "are all screwed up? I'm going to assume that you meant ##.",
                    ],
                )?;
            }
        }
    }
    let global = apply_globaldefs(global, stores);
    execution.mark_paragraph_local_meaning(stores, target.symbol, global);
    tracing::trace_meaning_write(
        stores,
        Token::Cs(target.symbol),
        // TeX82 §1224/§1230's `\def` family always allocates a fresh
        // definition, so real TeX never reports "reassigning" here even
        // when the new body is byte-identical to the old one -- only
        // `\let`/`\futurelet` can copy an already-installed equivalent.
        true,
        global,
        |stores| {
            if global {
                stores.set_macro_meaning_global_with_provenance(
                    target.symbol,
                    scanned.meaning(),
                    scanned.provenance(),
                );
            } else {
                stores.set_macro_meaning_with_provenance(
                    target.symbol,
                    scanned.meaning(),
                    scanned.provenance(),
                );
            }
        },
    );
    Ok(())
}

/// TeX.web §263's `sprint_cs`: an active character prints bare, and every
/// other control sequence prints escaped and without `print_cs`'s trailing
/// space, so a following period lands directly against the name.
fn definition_target_text(stores: &Universe, target: Symbol) -> String {
    let name = stores.resolve(target).to_owned();
    match stores.control_sequence_kind(target) {
        tex_state::interner::ControlSequenceKind::ActiveCharacter => name,
        tex_state::interner::ControlSequenceKind::Null
        | tex_state::interner::ControlSequenceKind::SingleCharacter
        | tex_state::interner::ControlSequenceKind::Named => format!("\\{name}"),
    }
}

pub(super) fn execute_let(
    prefixes: Prefixes,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let target = scan_definition_target(input, stores, "\\let")?;
    let rhs = scan_optional_equals_one_space(input, stores)?;
    let meaning = token_meaning_for_let(rhs, stores, execution)?;
    let global = apply_globaldefs(prefixes.global, stores);
    execution.mark_paragraph_local_meaning(stores, target, global);
    let changed = stores.meaning(target) != meaning;
    tracing::trace_meaning_write(stores, Token::Cs(target), changed, global, move |stores| {
        if global {
            stores.set_meaning_global(target, meaning);
        } else {
            stores.set_meaning(target, meaning);
        }
    });
    Ok(())
}

pub(super) fn execute_futurelet(
    prefixes: Prefixes,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let target = scan_definition_target(input, stores, "\\futurelet")?;
    // TeX.web future_let uses get_token for both lookahead tokens. This is
    // observable inside alignments: fetching the second token can intercept a
    // cell delimiter and expose the v-template's frozen end marker instead.
    let first = tex_expand::get_token(input, &mut tex_state::ExpansionContext::new(stores))?
        .ok_or(ExecError::MissingToken {
            context: "\\futurelet lookahead",
        })?;
    let second = tex_expand::get_token(input, &mut tex_state::ExpansionContext::new(stores))?
        .ok_or(ExecError::MissingToken {
            context: "\\futurelet lookahead",
        })?;
    let meaning = token_meaning_for_let(second, stores, execution)?;
    let global = apply_globaldefs(prefixes.global, stores);
    execution.mark_paragraph_local_meaning(stores, target, global);
    let changed = stores.meaning(target) != meaning;
    tracing::trace_meaning_write(stores, Token::Cs(target), changed, global, move |stores| {
        if global {
            stores.set_meaning_global(target, meaning);
        } else {
            stores.set_meaning(target, meaning);
        }
    });
    input.back_input([first, second]);
    Ok(())
}

pub(super) fn execute_globaldefs(
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    skip_optional_equals_x(input, stores, execution)?;
    let value = scan_int::scan_int_with_mode_and_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        &mut DriverExpansionMode,
        context,
    )
    .map_err(ExpandError::from)?
    .value();
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_int_param_global(IntParam::GLOBAL_DEFS, value);
    } else {
        stores.set_int_param(IntParam::GLOBAL_DEFS, value);
    }
    Ok(())
}
