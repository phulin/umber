use super::*;
use std::path::PathBuf;
use tex_fonts::LoadedFont;
use tex_state::InputOpenState;
use tex_state::ids::FontId;
use tex_state::scaled::FontSizeSpec;

pub(super) fn execute_font_definition(
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let target = scan_definition_target(input, stores, "\\font")?;
    skip_optional_equals_x(input, stores, execution)?;
    let font_name = scan_font_file_name(input, stores, execution)?;
    let size_spec = scan_font_size_spec(input, stores, execution, context)?;
    let opentype_name = font_name.strip_prefix("opentype:");
    let path = if opentype_name.is_some() {
        PathBuf::from(&font_name)
    } else {
        tfm_path(&font_name)
    };
    let lookup = execution
        .open_font(&mut stores.input_open_context(), &path)
        .map_err(|message| ExecError::FontOpen {
            name: font_name.clone(),
            message,
        })?;
    let source = match lookup {
        tex_expand::ResourceLookup::Available(content) => content,
        tex_expand::ResourceLookup::NeedResource(need) => {
            return Err(ExecError::NeedResource(need));
        }
        tex_expand::ResourceLookup::Unavailable => {
            // TeX.web §1257 leaves the newly defined selector at `null_font`
            // after a TFM open failure and continues; §561's
            // `start_font_error_message` names the selector, the file, and
            // the requested size before the reason.
            let selector = stores.resolve(target).to_owned();
            report_font_not_loadable(
                stores,
                &selector,
                &font_name,
                size_spec,
                if opentype_name.is_some() {
                    FontLoadFailure::MissingOpenType
                } else {
                    FontLoadFailure::MissingTfm
                },
            )?;
            let meaning = Meaning::Font(tex_state::font::NULL_FONT);
            if apply_globaldefs(prefixes.global, stores) {
                stores.set_meaning_global(target, meaning);
            } else {
                stores.set_meaning(target, meaning);
            }
            return Ok(());
        }
    };
    macro_rules! parse_tfm {
        ($metrics:expr) => {
            match tex_fonts::TfmFont::parse_with_size($metrics.bytes(), size_spec) {
                Ok(tfm) => tfm,
                Err(_) => {
                    // TeX.web §564 treats every malformed metric file like
                    // an unavailable font and leaves the selector null.
                    let selector = stores.resolve(target).to_owned();
                    report_font_not_loadable(
                        stores,
                        &selector,
                        &font_name,
                        size_spec,
                        FontLoadFailure::MalformedTfm,
                    )?;
                    let meaning = Meaning::Font(tex_state::font::NULL_FONT);
                    if apply_globaldefs(prefixes.global, stores) {
                        stores.set_meaning_global(target, meaning);
                    } else {
                        stores.set_meaning(target, meaning);
                    }
                    return Ok(());
                }
            }
        };
    }
    let loaded = match source {
        crate::FontSource::Tfm { metrics, opentype } => {
            let tfm = parse_tfm!(metrics);
            let parameters = tfm
                .parameters
                .values
                .iter()
                .map(|parameter| parameter.value)
                .collect();
            let mut loaded = LoadedFont::new(
                font_display_name(&font_name),
                metrics.path().to_owned(),
                metrics.hash().bytes(),
                tfm.header.checksum,
                tfm.header.design_size,
                tfm.font_size,
                parameters,
                tfm.font_metrics(),
            );
            if let Some(selection) = opentype {
                loaded = loaded.with_opentype(selection);
            }
            loaded
        }
        crate::FontSource::MappedTfm {
            metrics,
            opentype,
            encoding_map,
        } => {
            let tfm = parse_tfm!(metrics);
            let parameters = tfm
                .parameters
                .values
                .iter()
                .map(|parameter| parameter.value)
                .collect();
            LoadedFont::new(
                font_display_name(&font_name),
                metrics.path().to_owned(),
                metrics.hash().bytes(),
                tfm.header.checksum,
                tfm.header.design_size,
                tfm.font_size,
                parameters,
                tfm.font_metrics(),
            )
            .with_mapped_opentype(opentype, encoding_map)
        }
        crate::FontSource::ClassicTfmFallback { metrics } => {
            let tfm = parse_tfm!(metrics);
            let parameters = tfm
                .parameters
                .values
                .iter()
                .map(|parameter| parameter.value)
                .collect();
            LoadedFont::new(
                font_display_name(&font_name),
                metrics.path().to_owned(),
                metrics.hash().bytes(),
                tfm.header.checksum,
                tfm.header.design_size,
                tfm.font_size,
                parameters,
                tfm.font_metrics(),
            )
            .with_classic_mapping_fallback()
        }
        crate::FontSource::OpenType(selection) => {
            let logical_name = opentype_name.unwrap_or(&font_name);
            let design_size = Scaled::from_raw(10 * Scaled::UNITY);
            let size = tex_state::scaled::tfm_font_size(design_size, size_spec)
                .map_err(|_| ExecError::ArithmeticOverflow)?;
            LoadedFont::new_opentype(logical_name, logical_name, design_size, size, selection)
        }
    };
    let id = match stores.try_intern_font_with_identifier(loaded, target) {
        Ok(id) => id,
        Err(
            tex_state::FontParameterError::TooManyFonts { .. }
            | tex_state::FontParameterError::FontInfoCapacity { .. },
        ) => {
            // TeX.web §567 has already validated the TFM when it discovers
            // that the font table has no room. The destination remains the
            // provisional null font and the failed row is not committed.
            let selector = stores.resolve(target).to_owned();
            report_font_capacity(stores, &selector, &font_name, size_spec)?;
            let meaning = Meaning::Font(tex_state::font::NULL_FONT);
            if apply_globaldefs(prefixes.global, stores) {
                stores.set_meaning_global(target, meaning);
            } else {
                stores.set_meaning(target, meaning);
            }
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let meaning = Meaning::Font(id);
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

pub(super) fn execute_generated_font_definition(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let primitive_name = match primitive {
        UnexpandablePrimitive::LetterspaceFont => "\\letterspacefont",
        UnexpandablePrimitive::PdfCopyFont => "\\pdfcopyfont",
        _ => unreachable!("caller restricts generated-font primitive"),
    };
    let target = scan_definition_target(input, stores, primitive_name)?;
    let global = apply_globaldefs(prefixes.global, stores);

    // pdfTeX defines the destination as nullfont before scanning the source.
    // This is observable for self-copying definitions and on later errors.
    if global {
        stores.set_meaning_global(target, Meaning::Font(tex_state::font::NULL_FONT));
    } else {
        stores.set_meaning(target, Meaning::Font(tex_state::font::NULL_FONT));
    }
    skip_optional_equals_x(input, stores, execution)?;
    let source = scan_font_selector(input, stores, execution)?;

    if primitive == UnexpandablePrimitive::PdfCopyFont
        && matches!(
            stores.font(source).construction(),
            tex_fonts::FontConstruction::Letterspaced { .. }
                | tex_fonts::FontConstruction::Expanded { .. }
        )
    {
        return Err(ExecError::CannotCopyFont(
            match stores.font(source).construction() {
                tex_fonts::FontConstruction::Expanded { .. } => "cannot copy an expanded font",
                _ => "cannot copy a letterspaced font",
            },
        ));
    }

    let id = match primitive {
        UnexpandablePrimitive::PdfCopyFont => {
            stores.try_copy_font_with_identifier(source, target)?
        }
        UnexpandablePrimitive::LetterspaceFont => {
            let amount = scan_i32(input, stores, execution, context)?.clamp(-1000, 1000) as i16;
            let no_ligatures = scan_optional_keyword_x(input, stores, execution, "nolig")?;
            let id = stores.try_letterspace_font_with_identifier(
                source,
                target,
                amount,
                no_ligatures,
            )?;
            if stores.font_parameter(id, 6).raw() == 0 {
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    "\npdfTeX warning (\\letterspacefont): font has zero em size (\\fontdimen6)\n",
                );
            }
            id
        }
        _ => unreachable!("caller restricts generated-font primitive"),
    };
    let meaning = Meaning::Font(id);
    if global {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

pub(super) fn execute_pdf_font_expand(
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_all_prefixes(prefixes)?;
    let font = scan_font_selector(input, stores, execution)?;
    skip_optional_equals_x(input, stores, execution)?;
    let stretch = scan_i32(input, stores, execution, context)?;
    let shrink = scan_i32(input, stores, execution, context)?;
    let step = scan_i32(input, stores, execution, context)?;
    let auto_expand = scan_optional_keyword_x(input, stores, execution, "autoexpand")?;
    let spec = tex_typeset::expansion::FontExpansionSpec::new(stretch, shrink, step, auto_expand)?;
    stores.configure_font_expansion(
        font,
        tex_state::font::FontExpansion {
            stretch: spec.stretch() as u16,
            shrink: spec.shrink() as u16,
            step: spec.step() as u8,
            auto_expand: spec.auto_expand(),
        },
    )?;
    Ok(())
}

pub(super) fn scan_font_variable_target(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Variable, ExecError> {
    match primitive {
        UnexpandablePrimitive::FontDimen => {
            let number = scan_i32(input, stores, execution, context)?;
            if number < 1 {
                return Err(ExecError::RegisterNumberOutOfRange(number));
            }
            let font = scan_font_selector(input, stores, execution)?;
            Ok(Variable::FontDimen(font, number as u32))
        }
        UnexpandablePrimitive::HyphenChar => {
            let font = scan_font_selector(input, stores, execution)?;
            Ok(Variable::FontHyphenChar(font))
        }
        UnexpandablePrimitive::SkewChar => {
            let font = scan_font_selector(input, stores, execution)?;
            Ok(Variable::FontSkewChar(font))
        }
        _ => unreachable!("caller restricts font variable primitive"),
    }
}

pub(super) fn execute_math_family_font_assignment(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let size = math_font_size_for_primitive(primitive);
    let family = scan_math_family(input, stores, execution, context)?;
    skip_optional_equals_x(input, stores, execution)?;
    let font = scan_font_selector(input, stores, execution)?;
    if !stores.font(font).supports_math() {
        return Err(ExecError::OpenTypeMathUnsupported);
    }
    stores.set_math_family_font(
        size,
        family,
        font,
        apply_globaldefs(prefixes.global, stores),
    );
    Ok(())
}

pub(super) fn scan_math_family(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<u8, ExecError> {
    let family = scan_i32(input, stores, execution, context)?;
    if !(0..=15).contains(&family) {
        // TeX.web §435's `scan_four_bit_int` reports the bad value with §91's
        // `int_error` and substitutes family zero so scanning can continue.
        let context = crate::diagnostics::show_context(stores, &input.summary());
        let mut report = stores.print_err("Bad number");
        report
            .help(&[
                "Since I expected to read a number between 0 and 15,",
                "I changed this one to zero.",
            ])
            .context(context);
        report.int_error(family).jump_out()?;
        return Ok(0);
    }
    Ok(family as u8)
}

pub(super) fn scan_font_selector(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<FontId, ExecError> {
    let traced =
        next_non_space_traced_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
            context: "font selector",
        })?;
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        report_missing_font_identifier(input, stores, traced)?;
        return Ok(tex_state::font::NULL_FONT);
    };
    match stores.meaning(symbol) {
        Meaning::Font(id) => Ok(id),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font) => Ok(stores.current_font()),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::TextFont
            | UnexpandablePrimitive::ScriptFont
            | UnexpandablePrimitive::ScriptScriptFont),
        ) => {
            let family = scan_math_family(input, stores, execution, traced)?;
            Ok(stores.math_family_font(math_font_size_for_primitive(primitive), family))
        }
        _ => {
            report_missing_font_identifier(input, stores, traced)?;
            Ok(tex_state::font::NULL_FONT)
        }
    }
}

/// TeX.web §577's `scan_font_ident` failure: `back_error`, then `null_font`.
///
/// Backing the token up is what puts it on §82's `<to be read again>` line and
/// leaves it for main control to reconsider.
fn report_missing_font_identifier(
    input: &mut InputStack,
    stores: &mut Universe,
    traced: TracedTokenWord,
) -> Result<(), ExecError> {
    crate::error_report::back_error(
        input,
        stores,
        traced,
        "Missing font identifier",
        &[
            "I was looking for a control sequence whose",
            "current meaning has been defined by \\font.",
        ],
    )?;
    Ok(())
}

/// TeX.web §561's `<Report that the font won't be loaded>`, opened by §560's
/// `start_font_error_message`.
///
/// The size clause is part of the message, not of the help: `at` prints the
/// scaled dimension and `scaled` the magnification, while a design-size
/// request prints neither. Umber's OpenType lookup has no TeX82 counterpart,
/// so it keeps its own reason and first help line and shares the rest.
#[derive(Clone, Copy)]
pub(crate) enum FontLoadFailure {
    MissingTfm,
    MissingOpenType,
    MalformedTfm,
}

pub(crate) fn report_font_not_loadable(
    stores: &mut Universe,
    selector: &str,
    font_name: &str,
    size_spec: FontSizeSpec,
    failure: FontLoadFailure,
) -> Result<(), ExecError> {
    let context = crate::diagnostics::show_context(stores, stores.input_summary());
    report_font_not_loadable_with_context(stores, selector, font_name, size_spec, failure, context)
}

pub(crate) fn report_font_not_loadable_with_context(
    stores: &mut Universe,
    selector: &str,
    font_name: &str,
    size_spec: FontSizeSpec,
    failure: FontLoadFailure,
    context: String,
) -> Result<(), ExecError> {
    let (reason, detail) = match failure {
        FontLoadFailure::MissingOpenType => (
            " not loadable: OpenType resource not found",
            "I wasn't able to resolve the requested OpenType font,",
        ),
        FontLoadFailure::MissingTfm => (
            " not loadable: Metric (TFM) file not found",
            "I wasn't able to read the size data for this font,",
        ),
        FontLoadFailure::MalformedTfm => (
            " not loadable: Bad metric (TFM) file",
            "I wasn't able to read the size data for this font,",
        ),
    };
    let mut report = stores.print_err("Font ");
    report.print_esc(selector).print("=").print(font_name);
    match size_spec {
        FontSizeSpec::At(size) => {
            report.print(" at ").print_scaled(size).print("pt");
        }
        FontSizeSpec::Scale(scale) => {
            report.print(" scaled ").print_int(scale);
        }
        FontSizeSpec::Design => {}
    }
    report
        .print(reason)
        .help(&[
            detail,
            "so I will ignore the font specification.",
            "[Wizards can fix TFM files using TFtoPL/PLtoTF.]",
            "You might try inserting a different font spec;",
            "e.g., type `I\\font<same font id>=<substitute font name>'.",
        ])
        .context(context);
    report.error().jump_out()?;
    Ok(())
}

/// TeX.web §567's capacity apology after a valid TFM has been read.
pub(crate) fn report_font_capacity(
    stores: &mut Universe,
    selector: &str,
    font_name: &str,
    size_spec: FontSizeSpec,
) -> Result<(), ExecError> {
    let context = crate::diagnostics::show_context(stores, stores.input_summary());
    let mut report = stores.print_err("Font ");
    report.print_esc(selector).print("=").print(font_name);
    match size_spec {
        FontSizeSpec::At(size) => {
            report.print(" at ").print_scaled(size).print("pt");
        }
        FontSizeSpec::Scale(scale) => {
            report.print(" scaled ").print_int(scale);
        }
        FontSizeSpec::Design => {}
    }
    report
        .print(" not loaded: Not enough room left")
        .help(&[
            "I'm afraid I won't be able to make use of this font,",
            "because my memory for character-size data is too small.",
            "If you're really stuck, ask a wizard to enlarge me.",
            "Or maybe try `I\\font<same font id>=<name of loaded font>'.",
        ])
        .context(context);
    report.error().jump_out()?;
    Ok(())
}

fn math_font_size_for_primitive(primitive: UnexpandablePrimitive) -> MathFontSize {
    match primitive {
        UnexpandablePrimitive::TextFont => MathFontSize::Text,
        UnexpandablePrimitive::ScriptFont => MathFontSize::Script,
        UnexpandablePrimitive::ScriptScriptFont => MathFontSize::ScriptScript,
        _ => unreachable!("caller restricts math font primitive"),
    }
}

fn scan_font_size_spec(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<FontSizeSpec, ExecError> {
    if scan_optional_keyword_x(input, stores, execution, "at")? {
        let requested = scan_scaled(input, stores, execution, context)?;
        let size = if requested.raw() > 0 && requested.raw() < 2048 * Scaled::UNITY {
            requested
        } else {
            // TeX.web §1259 folds the rejected dimension into the message
            // itself, between `print_err` and the help lines.
            let context = crate::diagnostics::show_context(stores, &input.summary());
            let mut report = stores.print_err("Improper `at' size (");
            report
                .print_scaled(requested)
                .print("pt), replaced by 10pt")
                .help(&[
                    "I can only handle fonts at positive sizes that are",
                    "less than 2048pt, so I've changed what you said to 10pt.",
                ])
                .context(context);
            report.error().jump_out()?;
            Scaled::from_raw(10 * Scaled::UNITY)
        };
        return Ok(FontSizeSpec::At(size));
    }
    if scan_optional_keyword_x(input, stores, execution, "scaled")? {
        let requested = scan_i32(input, stores, execution, context)?;
        let scale = if (1..=32_768).contains(&requested) {
            requested
        } else {
            // TeX.web §1258 reports the bad magnification with §91's
            // `int_error` and continues at the design-size scale 1000.
            let context = crate::diagnostics::show_context(stores, &input.summary());
            let mut report = stores.print_err("Illegal magnification has been changed to 1000");
            report
                .help(&["The magnification ratio must be between 1 and 32768."])
                .context(context);
            report.int_error(requested).jump_out()?;
            1000
        };
        return Ok(FontSizeSpec::Scale(scale));
    }
    Ok(FontSizeSpec::Design)
}

fn scan_font_file_name(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<String, ExecError> {
    let mut name = String::new();
    let Some(first) = next_non_space_x(input, stores, execution)? else {
        return Err(ExecError::MissingToken { context: "\\font" });
    };
    append_font_name_token(&mut name, first)?;
    while let Some(traced) = get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )? {
        match tex_expand::semantic_token(traced) {
            Token::Char {
                cat: Catcode::Space,
                ..
            } => break,
            token @ Token::Char { .. } => append_font_name_token(&mut name, token)?,
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
                // TeX.web `scan_file_name` backs up the first expanded token
                // that is not a character. It belongs to the following font
                // size scan or main-control command (section 530).
                push_traced_tokens(input, stores, [traced]);
                break;
            }
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
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
            Err(ExecError::MissingToken { context: "\\font" })
        }
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
