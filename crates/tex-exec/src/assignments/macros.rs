use super::*;

pub(super) fn execute_aftergroup<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let token = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\aftergroup",
    })?;
    stores.push_aftergroup(token);
    Ok(())
}

pub(super) fn execute_afterassignment<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let token = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\afterassignment",
    })?;
    stores.set_afterassignment(token);
    Ok(())
}

pub(super) fn fire_afterassignment<S>(input: &mut InputStack<S>, stores: &mut Universe)
where
    S: InputSource,
{
    if let Some(token) = stores.take_afterassignment() {
        push_tokens(input, stores, [token]);
    }
}

pub(super) fn execute_def<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
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
        scan_toks_expanded_with_driver(input, stores, prefixes.flags, target.traced, hooks)?
    } else {
        scan_toks(input, stores, prefixes.flags, target.traced)?
    }
    .with_definition_origin(target.origin);
    if apply_globaldefs(global, stores) {
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

pub(super) fn execute_let<S>(
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    reject_macro_prefixes(prefixes)?;
    let target = scan_definition_target(input, stores, "\\let")?;
    let rhs = scan_optional_equals_one_space(input, stores)?;
    let meaning = token_meaning_for_let(rhs, stores)?;
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

pub(super) fn execute_futurelet<S>(
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    reject_macro_prefixes(prefixes)?;
    let target = scan_definition_target(input, stores, "\\futurelet")?;
    let first = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\futurelet lookahead",
    })?;
    let second = input
        .next_traced_token(stores)?
        .ok_or(ExecError::MissingToken {
            context: "\\futurelet lookahead",
        })?;
    let meaning = token_meaning_for_let(second, stores)?;
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    push_tokens(input, stores, [first, tex_expand::semantic_token(second)]);
    Ok(())
}

pub(super) fn execute_globaldefs<S, H>(
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    reject_macro_prefixes(prefixes)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let mut recorder = NoopRecorder;
    let value = scan_int::scan_int_with_expander_and_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        &mut DriverExpandNext,
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
