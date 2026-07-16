//! pdfTeX font-map and per-font output actions.

use tex_expand::append_token_string_text;
use tex_expand::scan::scan_general_text_expanded_with_driver;
use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::token::TracedTokenWord;

use crate::ExecError;

use super::scan_font_selector;

pub(super) fn execute_pdf_font_output_action(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    if primitive == UnexpandablePrimitive::PdfNoBuiltinToUnicode {
        let font = scan_font_selector(input, stores, execution)?;
        stores.disable_pdf_builtin_to_unicode(font);
        return Ok(());
    }
    if primitive == UnexpandablePrimitive::PdfGlyphToUnicode {
        let glyph_tokens = scan_general_text_expanded_with_driver(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
            context,
        )?;
        let glyph = token_list_bytes(stores, glyph_tokens);
        let unicode_tokens = scan_general_text_expanded_with_driver(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
            context,
        )?;
        let unicode = token_list_bytes(stores, unicode_tokens);
        stores.set_pdf_glyph_to_unicode(parse_glyph_to_unicode(&glyph, &unicode)?);
        return Ok(());
    }
    let font = match primitive {
        UnexpandablePrimitive::PdfFontAttr | UnexpandablePrimitive::PdfIncludeChars => {
            Some(scan_font_selector(input, stores, execution)?)
        }
        UnexpandablePrimitive::PdfMapFile | UnexpandablePrimitive::PdfMapLine => None,
        _ => unreachable!("caller restricts PDF font output actions"),
    };
    let tokens = scan_general_text_expanded_with_driver(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    )?;
    let bytes = token_list_bytes(stores, tokens);

    match primitive {
        UnexpandablePrimitive::PdfFontAttr => {
            stores.set_pdf_font_attribute(font.expect("font action scanned a font"), bytes);
        }
        UnexpandablePrimitive::PdfIncludeChars => {
            stores.include_pdf_font_chars(font.expect("font action scanned a font"), bytes);
        }
        UnexpandablePrimitive::PdfMapFile => {
            let file = tex_fonts::PdfFontMapFile::parse(&bytes)?;
            stores.push_pdf_font_map(tex_state::PdfFontMapOperation::File(file));
        }
        UnexpandablePrimitive::PdfMapLine => {
            let line = tex_fonts::PdfFontMapEntry::parse(&bytes)?;
            let duplicate_count = stores.pdf_font_map_duplicate_names().len();
            stores.push_pdf_font_map(tex_state::PdfFontMapOperation::Line(line));
            let duplicates = stores.pdf_font_map_duplicate_names();
            if duplicates.len() > duplicate_count
                && stores.int_param(IntParam::PDF_SUPPRESS_WARNING_DUP_MAP) <= 0
            {
                let name = String::from_utf8_lossy(
                    duplicates
                        .last()
                        .expect("a newly recorded duplicate has a name"),
                );
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    &format!(
                        "\npdfTeX warning: pdftex: fontmap entry for `{name}' already exists, duplicates ignored\n"
                    ),
                );
            }
        }
        _ => unreachable!("caller restricts PDF font output actions"),
    }
    Ok(())
}

fn parse_glyph_to_unicode(
    glyph: &[u8],
    unicode: &[u8],
) -> Result<tex_state::PdfGlyphToUnicode, ExecError> {
    if glyph.is_empty() || glyph.iter().any(u8::is_ascii_whitespace) {
        return Err(ExecError::PdfGlyphToUnicode(
            "glyph name must be one nonempty PostScript name".into(),
        ));
    }
    let (tfm_name, glyph_name) = if let Some(scoped) = glyph.strip_prefix(b"tfm:") {
        let Some(slash) = scoped.iter().position(|byte| *byte == b'/') else {
            return Err(ExecError::PdfGlyphToUnicode(
                "tfm-scoped glyph name must have the form tfm:name/glyph".into(),
            ));
        };
        let (tfm, glyph) = scoped.split_at(slash);
        if tfm.is_empty() || glyph.len() == 1 {
            return Err(ExecError::PdfGlyphToUnicode(
                "tfm-scoped glyph name must have the form tfm:name/glyph".into(),
            ));
        }
        (Some(tfm.to_vec()), glyph[1..].to_vec())
    } else {
        (None, glyph.to_vec())
    };
    let values = unicode
        .split(u8::is_ascii_whitespace)
        .filter(|part| !part.is_empty())
        .map(|value| {
            if !(4..=6).contains(&value.len()) || !value.iter().all(u8::is_ascii_hexdigit) {
                return Err(ExecError::PdfGlyphToUnicode(
                    "Unicode values must be 4-6 hexadecimal digits separated by spaces".into(),
                ));
            }
            std::str::from_utf8(value)
                .ok()
                .and_then(|text| u32::from_str_radix(text, 16).ok())
                .ok_or_else(|| {
                    ExecError::PdfGlyphToUnicode("Unicode value is not a scalar value".into())
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut scalars = Vec::with_capacity(values.len());
    let mut index = 0;
    while let Some(&value) = values.get(index) {
        let scalar = if (0xD800..=0xDBFF).contains(&value) {
            let Some(&low) = values.get(index + 1) else {
                return Err(ExecError::PdfGlyphToUnicode(
                    "Unicode value is not a scalar value".into(),
                ));
            };
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err(ExecError::PdfGlyphToUnicode(
                    "Unicode value is not a scalar value".into(),
                ));
            }
            index += 2;
            0x1_0000 + ((value - 0xD800) << 10) + (low - 0xDC00)
        } else {
            index += 1;
            value
        };
        if char::from_u32(scalar).is_none() {
            return Err(ExecError::PdfGlyphToUnicode(
                "Unicode value is not a scalar value".into(),
            ));
        }
        scalars.push(scalar);
    }
    if scalars.is_empty() {
        return Err(ExecError::PdfGlyphToUnicode(
            "at least one Unicode value is required".into(),
        ));
    }
    Ok(tex_state::PdfGlyphToUnicode {
        tfm_name,
        glyph_name,
        unicode: scalars,
    })
}

fn token_list_bytes(stores: &Universe, tokens: tex_state::ids::TokenListId) -> Vec<u8> {
    let mut text = String::new();
    for &token in stores.tokens(tokens) {
        append_token_string_text(stores, token, &mut text);
    }
    text.into_bytes()
}
