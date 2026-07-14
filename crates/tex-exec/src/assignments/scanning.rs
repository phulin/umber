use super::*;

pub(super) fn is_assignment_meaning(meaning: Meaning) -> bool {
    match meaning {
        Meaning::UnexpandablePrimitive(primitive) => super::is_assignment_primitive(primitive),
        meaning => is_assignment_target_meaning(meaning),
    }
}

pub(crate) fn is_assignment_target_meaning(meaning: Meaning) -> bool {
    matches!(
        meaning,
        Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)
    )
}

pub(super) fn variable_from_meaning(meaning: Meaning) -> Option<Variable> {
    match meaning {
        Meaning::CountRegister(index) => Some(Variable::IntRegister(index)),
        Meaning::DimenRegister(index) => Some(Variable::DimenRegister(index)),
        Meaning::SkipRegister(index) => Some(Variable::GlueRegister(index)),
        Meaning::MuskipRegister(index) => Some(Variable::MuGlueRegister(index)),
        Meaning::ToksRegister(index) => Some(Variable::ToksRegister(index)),
        Meaning::IntParam(index) => Some(Variable::IntParam(index)),
        Meaning::DimenParam(index) => Some(Variable::DimenParam(index)),
        Meaning::GlueParam(index) => Some(Variable::GlueParam(index)),
        Meaning::MuGlueParam(index) => Some(Variable::MuGlueParam(index)),
        Meaning::TokParam(index) => Some(Variable::TokParam(index)),
        Meaning::PageDimension(dimension) => Some(Variable::PageDimension(dimension)),
        Meaning::PageInteger(integer) => Some(Variable::PageInteger(integer)),
        _ => None,
    }
}

pub(super) fn scan_variable_target(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Variable, ExecError> {
    let traced =
        next_non_space_traced_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
            context: "arithmetic target",
        })?;
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        return Err(ExecError::ExpectedControlSequence {
            context: "arithmetic target",
            token,
            origin: traced.origin(),
        });
    };
    let meaning = stores.meaning(symbol);
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => Ok(Variable::IntRegister(
            scan_register_index_with_context(input, stores, execution, traced)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            Ok(Variable::DimenRegister(scan_register_index_with_context(
                input, stores, execution, traced,
            )?))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => Ok(Variable::GlueRegister(
            scan_register_index_with_context(input, stores, execution, traced)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            Ok(Variable::MuGlueRegister(scan_register_index_with_context(
                input, stores, execution, traced,
            )?))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen) => {
            scan_font_variable_target(
                UnexpandablePrimitive::FontDimen,
                traced,
                input,
                stores,
                execution,
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HyphenChar) => {
            scan_font_variable_target(
                UnexpandablePrimitive::HyphenChar,
                traced,
                input,
                stores,
                execution,
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SkewChar) => {
            scan_font_variable_target(
                UnexpandablePrimitive::SkewChar,
                traced,
                input,
                stores,
                execution,
            )
        }
        meaning => variable_from_meaning(meaning).ok_or(ExecError::UnsupportedAssignmentTarget),
    }
}

pub(super) fn scan_register_index(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<u16, ExecError> {
    scan_register_index_with_context(input, stores, execution, context)
}

pub(super) fn scan_register_index_with_context(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<u16, ExecError> {
    let scanned = scan_int::scan_int_with_mode_and_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        &mut DriverExpansionMode,
        context,
    )
    .map_err(ExpandError::from)?;
    if let Some(diagnostic) = scanned.diagnostic() {
        diagnostics::report_integer_diagnostic(stores, diagnostic);
    }
    let value = scanned.value();
    let maximum: u16 = if stores.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 {
        32_767
    } else {
        255
    };
    if !(0..=i32::from(maximum)).contains(&value) {
        stores.report_bad_register_code(value, maximum);
        return Ok(0);
    }
    Ok(value as u16)
}

pub(crate) fn scan_i32(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<i32, ExecError> {
    let scanned = scan_int::scan_int_with_mode_and_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        &mut DriverExpansionMode,
        context,
    )
    .map_err(ExpandError::from)?;
    if let Some(diagnostic) = scanned.diagnostic() {
        diagnostics::report_integer_diagnostic(stores, diagnostic);
    }
    Ok(scanned.value())
}

pub(super) fn scan_nonzero_i32(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<i32, ExecError> {
    let value = scan_i32(input, stores, execution, context)?;
    if value == 0 {
        Err(ExecError::ArithmeticOverflow)
    } else {
        Ok(value)
    }
}

pub(crate) fn scan_scaled(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<Scaled, ExecError> {
    let scanned = scan_dimen::scan_dimen_with_mode_and_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        &mut DriverExpansionMode,
        scan_dimen::ScanDimenOptions::STANDARD,
        context,
    )
    .map_err(ExpandError::from)?;
    diagnostics::report_dimension_diagnostics(stores, scanned.diagnostics());
    Ok(scanned.value())
}

pub(crate) fn scan_glue_id(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    mu: bool,
    context: TracedTokenWord,
) -> Result<GlueId, ExecError> {
    let scanned = scan_glue::scan_glue_with_mode_and_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        &mut DriverExpansionMode,
        mu,
        context,
    )
    .map_err(ExecError::ScanGlue)?;
    diagnostics::report_dimension_diagnostics(stores, scanned.diagnostics());
    Ok(scanned.id())
}

pub(super) fn scan_token_list_assignment(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<tex_state::ids::TokenListId, ExecError> {
    let traced = next_non_space_traced_x(input, stores, execution)?
        .ok_or(ExecError::MissingTracedToken { context })?;
    let token = tex_expand::semantic_token(traced);
    if let Token::Cs(symbol) = token {
        match stores.meaning(symbol) {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                let index = scan_register_index(input, stores, execution, traced)?;
                return Ok(stores.toks(index));
            }
            Meaning::ToksRegister(index) => return Ok(stores.toks(index)),
            Meaning::TokParam(index) => return Ok(stores.tok_param(TokParam::new(index))),
            _ => {}
        }
    }
    if !is_begin_group(token) {
        return Err(ExecError::MissingToken {
            context: "token-list assignment group",
        });
    }
    scan_balanced_text_after_open_group(input, stores)
}

fn scan_balanced_text_after_open_group(
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<tex_state::ids::TokenListId, ExecError> {
    let mut depth = 1usize;
    let mut builder = stores.token_list_builder();
    while let Some(traced) =
        tex_expand::next_semantic_raw_token(input, &mut tex_state::ExpansionContext::new(stores))?
    {
        let token = tex_expand::semantic_token(traced);
        let meaning = match token {
            Token::Cs(symbol) => stores.meaning(symbol),
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = active_character_symbol(stores, ch);
                stores.meaning(symbol)
            }
            _ => Meaning::Undefined,
        };
        if matches!(
            meaning,
            Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::OUTER)
        ) {
            // TeX.web §336's absorbing scanner inserts a right brace and
            // backs up the forbidden outer token for ordinary expansion.
            crate::insert_traced_tokens(input, stores, [traced]);
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Forbidden control sequence found while scanning text.\nI've inserted a closing brace and will continue.\n",
            );
            return Ok(stores.finish_token_list(&mut builder));
        }
        match token {
            token if is_begin_group(token) => {
                depth += 1;
                builder.push(token);
            }
            token if is_end_group(token) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(stores.finish_token_list(&mut builder));
                }
                builder.push(token);
            }
            token => builder.push(token),
        }
    }
    Err(ExecError::MissingToken {
        context: "token-list closing brace",
    })
}

pub(crate) fn next_non_space_x(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<Token>, ExecError> {
    loop {
        let Some(token) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        .map(tex_expand::semantic_token) else {
            return Ok(None);
        };
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
}

pub(crate) fn next_non_space_traced_x(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExecError> {
    loop {
        let Some(token) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Ok(None);
        };
        if !is_space(tex_expand::semantic_token(token)) {
            return Ok(Some(token));
        }
    }
}

pub(crate) fn scan_optional_keyword_x(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    keyword: &str,
) -> Result<bool, ExecError> {
    Ok(scan_optional_keyword_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        keyword,
    )?)
}
