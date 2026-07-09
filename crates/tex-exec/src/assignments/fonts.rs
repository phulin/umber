use super::*;
use std::path::PathBuf;
use tex_fonts::LoadedFont;
use tex_state::ids::FontId;
use tex_state::scaled::FontSizeSpec;

pub(super) fn execute_font_definition<S, H>(
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
    let target = scan_definition_target(input, stores, "\\font")?;
    skip_optional_equals_x(input, stores, hooks)?;
    let font_name = scan_font_file_name(input, stores, hooks)?;
    let size_spec = scan_font_size_spec(input, stores, hooks)?;
    let path = tfm_path(&font_name);
    let content = hooks
        .open_font(&mut stores.input_open_context(), &path)
        .map_err(|message| ExecError::FontOpen {
            name: font_name.clone(),
            message,
        })?;
    let tfm = tex_fonts::TfmFont::parse_with_size(content.bytes(), size_spec)?;
    let parameters = tfm
        .parameters
        .values
        .iter()
        .map(|parameter| parameter.value)
        .collect();
    let loaded = LoadedFont::new(
        font_display_name(&font_name),
        content.path().to_owned(),
        content.hash().bytes(),
        tfm.header.checksum,
        tfm.header.design_size,
        tfm.font_size,
        parameters,
        tfm.font_metrics(),
    );
    let id = stores.intern_font_with_identifier(loaded, target);
    let meaning = Meaning::Font(id);
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

pub(super) fn scan_font_variable_target<S, H>(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Variable, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::FontDimen => {
            let number = scan_i32(input, stores, hooks)?;
            if !(1..=32_767).contains(&number) {
                return Err(ExecError::RegisterNumberOutOfRange(number));
            }
            let font = scan_font_selector(input, stores, hooks)?;
            Ok(Variable::FontDimen(font, number as u16))
        }
        UnexpandablePrimitive::HyphenChar => {
            let font = scan_font_selector(input, stores, hooks)?;
            Ok(Variable::FontHyphenChar(font))
        }
        UnexpandablePrimitive::SkewChar => {
            let font = scan_font_selector(input, stores, hooks)?;
            Ok(Variable::FontSkewChar(font))
        }
        _ => unreachable!("caller restricts font variable primitive"),
    }
}

pub(super) fn execute_math_family_font_assignment<S, H>(
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
    reject_macro_prefixes(prefixes)?;
    let size = math_font_size_for_primitive(primitive);
    let family = scan_math_family(input, stores, hooks)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let font = scan_font_selector(input, stores, hooks)?;
    stores.set_math_family_font(
        size,
        family,
        font,
        apply_globaldefs(prefixes.global, stores),
    );
    Ok(())
}

pub(super) fn scan_math_family<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<u8, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let family = scan_i32(input, stores, hooks)?;
    if !(0..=15).contains(&family) {
        return Err(ExecError::InvalidCode {
            context: "math family",
            value: family,
        });
    }
    Ok(family as u8)
}

pub(super) fn scan_font_selector<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<FontId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let traced = next_non_space_traced_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "font selector",
    })?;
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        return Err(ExecError::ExpectedControlSequence {
            context: "font selector",
            token,
            origin: traced.origin(),
        });
    };
    match stores.meaning(symbol) {
        Meaning::Font(id) => Ok(id),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::TextFont
            | UnexpandablePrimitive::ScriptFont
            | UnexpandablePrimitive::ScriptScriptFont),
        ) => {
            let family = scan_math_family(input, stores, hooks)?;
            Ok(stores.math_family_font(math_font_size_for_primitive(primitive), family))
        }
        _ => Err(ExecError::ExpectedControlSequence {
            context: "font selector",
            token,
            origin: traced.origin(),
        }),
    }
}

fn math_font_size_for_primitive(primitive: UnexpandablePrimitive) -> MathFontSize {
    match primitive {
        UnexpandablePrimitive::TextFont => MathFontSize::Text,
        UnexpandablePrimitive::ScriptFont => MathFontSize::Script,
        UnexpandablePrimitive::ScriptScriptFont => MathFontSize::ScriptScript,
        _ => unreachable!("caller restricts math font primitive"),
    }
}

fn scan_font_size_spec<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<FontSizeSpec, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    if scan_optional_keyword_x(input, stores, hooks, "at")? {
        return Ok(FontSizeSpec::At(scan_scaled(input, stores, hooks)?));
    }
    if scan_optional_keyword_x(input, stores, hooks, "scaled")? {
        return Ok(FontSizeSpec::Scale(scan_i32(input, stores, hooks)?));
    }
    Ok(FontSizeSpec::Design)
}

fn scan_font_file_name<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<String, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut name = String::new();
    let Some(first) = next_non_space_x(input, stores, hooks)? else {
        return Err(ExecError::MissingToken { context: "\\font" });
    };
    append_font_name_token(&mut name, first)?;
    let mut recorder = NoopRecorder;
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
            .map(tex_expand::semantic_token)
    {
        match token {
            Token::Char {
                cat: Catcode::Space,
                ..
            } => break,
            token => append_font_name_token(&mut name, token)?,
        }
    }
    Ok(name)
}

fn append_font_name_token(name: &mut String, token: Token) -> Result<(), ExecError> {
    match token {
        Token::Char { ch, .. } => {
            name.push(ch);
            Ok(())
        }
        Token::Cs(_) | Token::Param(_) => Err(ExecError::MissingToken { context: "\\font" }),
    }
}

fn tfm_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension("tfm");
    }
    path
}

fn font_display_name(name: &str) -> String {
    name.strip_suffix(".tfm").unwrap_or(name).to_owned()
}
