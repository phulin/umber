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
            MacroScanDiagnostic::UndefinedControlSequence { name, .. } => {
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    &format!("\n! Undefined control sequence \\{name}.\n"),
                );
            }
            MacroScanDiagnostic::IllegalParameterNumber { .. } => {
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    "\n! Illegal parameter number in definition.\n",
                );
            }
        }
    }
    let global = apply_globaldefs(global, stores);
    execution.mark_paragraph_local_meaning(stores, target.symbol, global);
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
    Ok(())
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
    if global {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
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
    if global {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    tex_expand::back_input(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        [first, second],
    );
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
