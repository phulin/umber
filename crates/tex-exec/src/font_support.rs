use tex_state::CommandContext;
use tex_state::interner::ControlSequenceKind;
use tex_state::scaled::FontSizeSpec;

use crate::ExecError;

#[derive(Clone, Copy)]
pub(crate) enum FontLoadFailure {
    MissingTfm,
    MissingOpenType,
    MalformedTfm,
}

pub(crate) fn report_font_not_loadable_with_context<G>(
    stores: &mut CommandContext<'_, G>,
    selector_kind: ControlSequenceKind,
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
    report
        .sprint_cs(selector_kind, selector)
        .print("=")
        .print(font_name);
    print_size(&mut report, size_spec);
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

pub(crate) fn report_font_capacity<G>(
    stores: &mut CommandContext<'_, G>,
    selector_kind: ControlSequenceKind,
    selector: &str,
    font_name: &str,
    size_spec: FontSizeSpec,
    context: String,
) -> Result<(), ExecError> {
    let mut report = stores.print_err("Font ");
    report
        .sprint_cs(selector_kind, selector)
        .print("=")
        .print(font_name);
    print_size(&mut report, size_spec);
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

fn print_size<G>(report: &mut tex_state::print::ErrorReport<'_, G>, size_spec: FontSizeSpec) {
    match size_spec {
        FontSizeSpec::At(size) => {
            report.print(" at ").print_scaled(size).print("pt");
        }
        FontSizeSpec::Scale(scale) => {
            report.print(" scaled ").print_int(scale);
        }
        FontSizeSpec::Design => {}
    }
}

pub(crate) fn warn_pdf_destination_duplicate<G>(
    stores: &CommandContext<'_, G>,
    identity: &tex_state::PdfDestinationIdentity,
) -> Option<(tex_state::PrintSink, String)> {
    if stores.int_param(tex_state::env::banks::IntParam::PDF_SUPPRESS_WARNING_DUP_DEST) > 0 {
        return None;
    }
    let identity = match identity {
        tex_state::PdfDestinationIdentity::Name(name) => {
            format!("name{{{}}}", String::from_utf8_lossy(name))
        }
        tex_state::PdfDestinationIdentity::Number(number) => format!("num{number}"),
    };
    Some((
        tex_state::PrintSink::TerminalAndLog,
        format!(
            "\npdfTeX warning (ext4): destination with the same identifier ({identity}) has been already used, duplicate ignored\n"
        ),
    ))
}

pub(crate) enum GlyphToUnicodeParse {
    Mapping(tex_state::PdfGlyphToUnicode),
    Warning(String),
}

pub(crate) fn parse_glyph_to_unicode(glyph: &[u8], unicode: &[u8]) -> GlyphToUnicodeParse {
    let unicode = trim_spaces(unicode);
    if glyph.is_empty()
        || glyph == b".notdef"
        || unicode.is_empty()
        || unicode
            .iter()
            .any(|byte| *byte != b' ' && !byte.is_ascii_hexdigit())
    {
        return invalid(glyph, unicode);
    }
    let (tfm_name, glyph_name) = match glyph.strip_prefix(b"tfm:") {
        Some(scoped) => match scoped.iter().position(|byte| *byte == b'/') {
            Some(slash) if slash > 0 && slash + 1 < scoped.len() => {
                (Some(scoped[..slash].to_vec()), scoped[slash + 1..].to_vec())
            }
            _ => return invalid(glyph, unicode),
        },
        None => (None, glyph.to_vec()),
    };
    let compact: Vec<_> = unicode
        .iter()
        .copied()
        .filter(|byte| *byte != b' ')
        .collect();
    let units = if unicode.contains(&b' ') {
        if compact.len() % 4 != 0 {
            return invalid(glyph, unicode);
        }
        compact
            .chunks_exact(4)
            .filter_map(parse_hex)
            .collect::<Vec<_>>()
    } else {
        vec![match parse_hex(&compact) {
            Some(value) => value,
            None => return invalid(glyph, unicode),
        }]
    };
    let mut scalars = Vec::new();
    let mut index = 0;
    while index < units.len() {
        let high = units[index];
        let scalar = if (0xD800..=0xDBFF).contains(&high) {
            let Some(&low) = units.get(index + 1) else {
                return invalid(glyph, unicode);
            };
            if !(0xDC00..=0xDFFF).contains(&low) {
                return invalid(glyph, unicode);
            }
            index += 2;
            0x1_0000 + ((high - 0xD800) << 10) + low - 0xDC00
        } else {
            index += 1;
            high
        };
        if char::from_u32(scalar).is_none() {
            return GlyphToUnicodeParse::Warning(format!(
                "value out of range [0,10FFFF]: {scalar:X}"
            ));
        }
        scalars.push(scalar);
    }
    GlyphToUnicodeParse::Mapping(tex_state::PdfGlyphToUnicode {
        tfm_name,
        glyph_name,
        unicode: scalars,
    })
}

fn trim_spaces(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| *byte != b' ')
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(start, |end| end + 1);
    &bytes[start..end]
}

fn parse_hex(value: &[u8]) -> Option<u32> {
    std::str::from_utf8(value)
        .ok()
        .and_then(|text| u32::from_str_radix(text, 16).ok())
}

fn invalid(glyph: &[u8], unicode: &[u8]) -> GlyphToUnicodeParse {
    GlyphToUnicodeParse::Warning(format!(
        "invalid parameter(s): `{}` => `{}`",
        String::from_utf8_lossy(glyph),
        String::from_utf8_lossy(unicode)
    ))
}
