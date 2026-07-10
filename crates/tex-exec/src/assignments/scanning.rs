use super::*;

pub(super) fn is_assignment_meaning(meaning: Meaning) -> bool {
    match meaning {
        Meaning::UnexpandablePrimitive(primitive) => matches!(
            primitive,
            UnexpandablePrimitive::Def
                | UnexpandablePrimitive::Edef
                | UnexpandablePrimitive::Gdef
                | UnexpandablePrimitive::Xdef
                | UnexpandablePrimitive::Let
                | UnexpandablePrimitive::FutureLet
                | UnexpandablePrimitive::GlobalDefs
                | UnexpandablePrimitive::Global
                | UnexpandablePrimitive::BeginGroup
                | UnexpandablePrimitive::EndGroup
                | UnexpandablePrimitive::AfterGroup
                | UnexpandablePrimitive::AfterAssignment
                | UnexpandablePrimitive::Long
                | UnexpandablePrimitive::Outer
                | UnexpandablePrimitive::Protected
                | UnexpandablePrimitive::Count
                | UnexpandablePrimitive::Dimen
                | UnexpandablePrimitive::Skip
                | UnexpandablePrimitive::Muskip
                | UnexpandablePrimitive::Toks
                | UnexpandablePrimitive::CountDef
                | UnexpandablePrimitive::DimenDef
                | UnexpandablePrimitive::SkipDef
                | UnexpandablePrimitive::MuskipDef
                | UnexpandablePrimitive::ToksDef
                | UnexpandablePrimitive::CharDef
                | UnexpandablePrimitive::MathCharDef
                | UnexpandablePrimitive::Advance
                | UnexpandablePrimitive::Multiply
                | UnexpandablePrimitive::Divide
                | UnexpandablePrimitive::CatCode
                | UnexpandablePrimitive::LcCode
                | UnexpandablePrimitive::UcCode
                | UnexpandablePrimitive::SfCode
                | UnexpandablePrimitive::MathCode
                | UnexpandablePrimitive::DelCode
                | UnexpandablePrimitive::Font
                | UnexpandablePrimitive::TextFont
                | UnexpandablePrimitive::ScriptFont
                | UnexpandablePrimitive::ScriptScriptFont
                | UnexpandablePrimitive::FontDimen
                | UnexpandablePrimitive::HyphenChar
                | UnexpandablePrimitive::SkewChar
                | UnexpandablePrimitive::Patterns
                | UnexpandablePrimitive::Hyphenation
                | UnexpandablePrimitive::SpaceFactor
                | UnexpandablePrimitive::PrevDepth
                | UnexpandablePrimitive::PrevGraf
                | UnexpandablePrimitive::SetBox
                | UnexpandablePrimitive::Wd
                | UnexpandablePrimitive::Ht
                | UnexpandablePrimitive::Dp
                | UnexpandablePrimitive::OpenIn
                | UnexpandablePrimitive::CloseIn
                | UnexpandablePrimitive::OpenOut
                | UnexpandablePrimitive::CloseOut
                | UnexpandablePrimitive::Immediate
                | UnexpandablePrimitive::Write
                | UnexpandablePrimitive::Read
                | UnexpandablePrimitive::BatchMode
                | UnexpandablePrimitive::NonstopMode
                | UnexpandablePrimitive::ScrollMode
                | UnexpandablePrimitive::ErrorStopMode
        ),
        meaning => is_assignment_target_meaning(meaning),
    }
}

pub(super) fn is_assignment_target_meaning(meaning: Meaning) -> bool {
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

pub(super) fn scan_variable_target<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Variable, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let traced = next_non_space_traced_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
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
            scan_register_index_with_recorder(input, stores, &mut recorder, hooks, traced)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            Ok(Variable::DimenRegister(scan_register_index_with_recorder(
                input,
                stores,
                &mut recorder,
                hooks,
                traced,
            )?))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => Ok(Variable::GlueRegister(
            scan_register_index_with_recorder(input, stores, &mut recorder, hooks, traced)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            Ok(Variable::MuGlueRegister(scan_register_index_with_recorder(
                input,
                stores,
                &mut recorder,
                hooks,
                traced,
            )?))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen) => {
            scan_font_variable_target(
                UnexpandablePrimitive::FontDimen,
                traced,
                input,
                stores,
                hooks,
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HyphenChar) => {
            scan_font_variable_target(
                UnexpandablePrimitive::HyphenChar,
                traced,
                input,
                stores,
                hooks,
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SkewChar) => {
            scan_font_variable_target(
                UnexpandablePrimitive::SkewChar,
                traced,
                input,
                stores,
                hooks,
            )
        }
        meaning => variable_from_meaning(meaning).ok_or(ExecError::UnsupportedAssignmentTarget),
    }
}

pub(super) fn scan_register_index<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<u16, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    scan_register_index_with_recorder(input, stores, &mut NoopRecorder, hooks, context)
}

pub(super) fn scan_register_index_with_recorder<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<u16, ExecError>
where
    S: InputSource,
    R: tex_expand::ReadRecorder,
    H: ExpansionHooks<S>,
{
    let scanned = scan_int::scan_int_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut DriverExpandNext,
        context,
    )
    .map_err(ExpandError::from)?;
    if let Some(diagnostic) = scanned.diagnostic() {
        diagnostics::report_integer_diagnostic(stores, diagnostic);
    }
    let value = scanned.value();
    if !(0..=32_767).contains(&value) {
        return Err(
            ExpandError::from(scan_int::ScanIntError::RegisterNumberOutOfRange {
                value,
                context: scanned.context(),
            })
            .into(),
        );
    }
    Ok(value as u16)
}

pub(crate) fn scan_i32<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<i32, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let scanned = scan_int::scan_int_with_expander_and_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        &mut DriverExpandNext,
        context,
    )
    .map_err(ExpandError::from)?;
    if let Some(diagnostic) = scanned.diagnostic() {
        diagnostics::report_integer_diagnostic(stores, diagnostic);
    }
    Ok(scanned.value())
}

pub(super) fn scan_nonzero_i32<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<i32, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let value = scan_i32(input, stores, hooks, context)?;
    if value == 0 {
        Err(ExecError::ArithmeticOverflow)
    } else {
        Ok(value)
    }
}

pub(crate) fn scan_scaled<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<Scaled, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let scanned = scan_dimen::scan_dimen_with_expander_and_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        &mut DriverExpandNext,
        scan_dimen::ScanDimenOptions::STANDARD,
        context,
    )
    .map_err(ExpandError::from)?;
    diagnostics::report_dimension_diagnostics(stores, scanned.diagnostics());
    Ok(scanned.value())
}

pub(crate) fn scan_glue_id<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    mu: bool,
    context: TracedTokenWord,
) -> Result<GlueId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let scanned = scan_glue::scan_glue_with_expander_and_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        &mut DriverExpandNext,
        mu,
        context,
    )
    .map_err(ExecError::ScanGlue)?;
    diagnostics::report_dimension_diagnostics(stores, scanned.diagnostics());
    Ok(scanned.id())
}

pub(super) fn scan_token_list_assignment<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<tex_state::ids::TokenListId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let traced = next_non_space_traced_x(input, stores, hooks)?
        .ok_or(ExecError::MissingTracedToken { context })?;
    let token = tex_expand::semantic_token(traced);
    if let Token::Cs(symbol) = token {
        match stores.meaning(symbol) {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                let index = scan_register_index(input, stores, hooks, traced)?;
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

fn scan_balanced_text_after_open_group<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<tex_state::ids::TokenListId, ExecError>
where
    S: InputSource,
{
    let mut depth = 1usize;
    let mut builder = stores.token_list_builder();
    while let Some(token) = input.next_token(stores)? {
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

pub(crate) fn next_non_space_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Option<Token>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
            .map(tex_expand::semantic_token)
        else {
            return Ok(None);
        };
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
}

pub(crate) fn next_non_space_traced_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Option<TracedTokenWord>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
        else {
            return Ok(None);
        };
        if !is_space(tex_expand::semantic_token(token)) {
            return Ok(Some(token));
        }
    }
}

pub(crate) fn scan_optional_keyword_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    keyword: &str,
) -> Result<bool, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    Ok(scan_optional_keyword_with_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        keyword,
    )?)
}
