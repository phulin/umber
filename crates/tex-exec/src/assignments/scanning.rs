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
                | UnexpandablePrimitive::Write
                | UnexpandablePrimitive::Read
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
    let token = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "arithmetic target",
    })?;
    let Token::Cs(symbol) = token else {
        return Err(ExecError::ExpectedControlSequence {
            context: "arithmetic target",
            token,
        });
    };
    let meaning = stores.meaning(symbol);
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => Ok(Variable::IntRegister(
            scan_register_index_with_recorder(input, stores, &mut recorder, hooks)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            Ok(Variable::DimenRegister(scan_register_index_with_recorder(
                input,
                stores,
                &mut recorder,
                hooks,
            )?))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => Ok(Variable::GlueRegister(
            scan_register_index_with_recorder(input, stores, &mut recorder, hooks)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            Ok(Variable::MuGlueRegister(scan_register_index_with_recorder(
                input,
                stores,
                &mut recorder,
                hooks,
            )?))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen) => {
            scan_font_variable_target(UnexpandablePrimitive::FontDimen, input, stores, hooks)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HyphenChar) => {
            scan_font_variable_target(UnexpandablePrimitive::HyphenChar, input, stores, hooks)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SkewChar) => {
            scan_font_variable_target(UnexpandablePrimitive::SkewChar, input, stores, hooks)
        }
        meaning => variable_from_meaning(meaning).ok_or(ExecError::UnsupportedAssignmentTarget),
    }
}

pub(super) fn scan_register_index<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<u16, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    scan_register_index_with_recorder(input, stores, &mut NoopRecorder, hooks)
}

pub(super) fn scan_register_index_with_recorder<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<u16, ExecError>
where
    S: InputSource,
    R: tex_expand::ReadRecorder,
    H: ExpansionHooks<S>,
{
    let value = scan_int::scan_int_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut DriverExpandNext,
    )
    .map_err(ExpandError::from)?
    .value();
    if !(0..=32_767).contains(&value) {
        return Err(ExecError::RegisterNumberOutOfRange(value));
    }
    Ok(value as u16)
}

pub(crate) fn scan_i32<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<i32, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    Ok(scan_int::scan_int_with_expander_and_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        &mut DriverExpandNext,
    )
    .map_err(ExpandError::from)?
    .value())
}

pub(super) fn scan_nonzero_i32<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<i32, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let value = scan_i32(input, stores, hooks)?;
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
    )
    .map_err(ExpandError::from)?;
    if let Some(diagnostic) = scanned.diagnostic() {
        diagnostics::report_dimension_diagnostic(stores, diagnostic);
    }
    Ok(scanned.value())
}

pub(crate) fn scan_glue_id<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    mu: bool,
) -> Result<GlueId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    Ok(scan_glue::scan_glue_with_expander_and_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        &mut DriverExpandNext,
        mu,
    )
    .map_err(ExecError::ScanGlue)?
    .id())
}

pub(super) fn scan_token_list_assignment<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<tex_state::ids::TokenListId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let token = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "token-list assignment",
    })?;
    if let Token::Cs(symbol) = token {
        match stores.meaning(symbol) {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                let index = scan_register_index(input, stores, hooks)?;
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
        else {
            return Ok(None);
        };
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
}

pub(super) fn scan_optional_keyword_x<S, H>(
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
