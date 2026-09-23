//! Unicode helpers for PDF font assembly.

use super::*;

pub(in crate::pdf::finalize) fn encoding_differences(
    encoding: &tex_fonts::PdfEncoding,
    used_codes: &BTreeSet<u8>,
    subset: bool,
) -> Vec<PdfValue> {
    if !subset {
        let mut differences = Vec::with_capacity(257);
        differences.push(PdfValue::Integer(0));
        differences.extend(
            encoding
                .glyph_names()
                .iter()
                .map(|name| PdfValue::Name(PdfName::new(name.clone()))),
        );
        return differences;
    }
    let mut differences = Vec::new();
    let mut previous = None;
    for &code in used_codes {
        if previous != Some(code.wrapping_sub(1)) {
            differences.push(PdfValue::Integer(i64::from(code)));
        }
        differences.push(PdfValue::Name(PdfName::new(
            encoding.glyph_names()[usize::from(code)].clone(),
        )));
        previous = Some(code);
    }
    differences
}

pub(in crate::pdf::finalize) fn to_unicode_stream(
    input: &PdfFontInput,
    font: &crate::FontResource,
    encoding: Option<&tex_fonts::PdfEncoding>,
    type1: Option<&tex_fonts::PdfType1Program>,
    id: PdfObjectId,
) -> Result<(PdfObjectId, PdfIndirectObject), PdfBuildError> {
    let mappings = to_unicode_mappings(
        &input.glyph_to_unicode,
        input.infer_builtin_glyph_unicode,
        encoding,
        type1,
    );
    let cmap_name = to_unicode_cmap_name(input, font, encoding.is_some());
    let data = build_to_unicode_cmap(&cmap_name, &mappings);
    Ok((
        id,
        PdfIndirectObject {
            id,
            object: PdfObject::Stream {
                dictionary: PdfDictionary::new(),
                data,
            },
        },
    ))
}

pub(in crate::pdf::finalize) fn to_unicode_mappings(
    glyph_to_unicode: &BTreeMap<Vec<u8>, Vec<u32>>,
    infer_builtin: bool,
    encoding: Option<&tex_fonts::PdfEncoding>,
    type1: Option<&tex_fonts::PdfType1Program>,
) -> Vec<ToUnicodeMapping> {
    let mut mappings = Vec::new();
    for code in 0..=u8::MAX {
        let owned_glyph;
        let glyph = if let Some(encoding) = encoding {
            encoding.glyph_names()[usize::from(code)].as_slice()
        } else if let Some(type1) = type1 {
            let Some(name) = type1.builtin_glyph_name(code) else {
                continue;
            };
            owned_glyph = name;
            owned_glyph.as_slice()
        } else {
            continue;
        };
        if let Some(mapping) = resolve_glyph_unicode(glyph_to_unicode, infer_builtin, glyph) {
            mappings.push(ToUnicodeMapping { code, mapping });
        }
    }
    mappings
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::pdf::finalize) enum ResolvedGlyphUnicode {
    Numeric(u32),
    String(Vec<u32>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::pdf::finalize) struct ToUnicodeMapping {
    pub(in crate::pdf::finalize) code: u8,
    pub(in crate::pdf::finalize) mapping: ResolvedGlyphUnicode,
}

pub(in crate::pdf::finalize) fn resolve_glyph_unicode(
    glyph_to_unicode: &BTreeMap<Vec<u8>, Vec<u32>>,
    infer_builtin: bool,
    name: &[u8],
) -> Option<ResolvedGlyphUnicode> {
    let name = name.split(|byte| *byte == b'.').next()?;
    if name.is_empty() || name == b".notdef" {
        return None;
    }
    if name.contains(&b'_') {
        let mut sequence = Vec::new();
        for component in name.split(|byte| *byte == b'_') {
            match resolve_simple_glyph_unicode(glyph_to_unicode, infer_builtin, component) {
                Some(ResolvedGlyphUnicode::Numeric(scalar)) => sequence.push(scalar),
                Some(ResolvedGlyphUnicode::String(mut scalars)) => {
                    sequence.append(&mut scalars);
                }
                None => {}
            }
        }
        return Some(ResolvedGlyphUnicode::String(sequence));
    }
    resolve_simple_glyph_unicode(glyph_to_unicode, infer_builtin, name)
}

pub(in crate::pdf::finalize) fn resolve_simple_glyph_unicode(
    glyph_to_unicode: &BTreeMap<Vec<u8>, Vec<u32>>,
    infer_builtin: bool,
    name: &[u8],
) -> Option<ResolvedGlyphUnicode> {
    if let Some(unicode) = glyph_to_unicode.get(name) {
        return match unicode.as_slice() {
            [scalar] => Some(ResolvedGlyphUnicode::Numeric(*scalar)),
            _ => Some(ResolvedGlyphUnicode::String(unicode.clone())),
        };
    }
    infer_builtin
        .then(|| inferred_glyph_unicode(name))
        .flatten()
}

pub(in crate::pdf::finalize) fn inferred_glyph_unicode(
    name: &[u8],
) -> Option<ResolvedGlyphUnicode> {
    if let Some(hex) = name.strip_prefix(b"uni")
        && !hex.is_empty()
        && hex.len() % 4 == 0
        && hex.iter().all(u8::is_ascii_hexdigit)
    {
        let scalars = hex
            .chunks(4)
            .map(|chunk| {
                std::str::from_utf8(chunk)
                    .ok()
                    .and_then(|text| u32::from_str_radix(text, 16).ok())
                    .filter(|value| char::from_u32(*value).is_some())
            })
            .collect::<Option<Vec<_>>>()?;
        return match scalars.as_slice() {
            [scalar] => Some(ResolvedGlyphUnicode::Numeric(*scalar)),
            _ => Some(ResolvedGlyphUnicode::String(scalars)),
        };
    }
    if let Some(hex) = name.strip_prefix(b"u")
        && (4..=6).contains(&hex.len())
        && hex.iter().all(u8::is_ascii_hexdigit)
    {
        return std::str::from_utf8(hex)
            .ok()
            .and_then(|text| u32::from_str_radix(text, 16).ok())
            .filter(|value| char::from_u32(*value).is_some())
            .map(ResolvedGlyphUnicode::Numeric);
    }
    None
}

pub(in crate::pdf::finalize) fn to_unicode_cmap_name(
    input: &PdfFontInput,
    font: &crate::FontResource,
    has_encoding: bool,
) -> Vec<u8> {
    let mut name = font.name.as_bytes().to_vec();
    name.push(b'-');
    if has_encoding {
        let encoding_name = input
            .map_entry
            .as_ref()
            .and_then(|entry| entry.encoding_files.first())
            .map_or_else(
                || {
                    input
                        .encoding
                        .as_ref()
                        .map_or(b"encoding".as_slice(), tex_fonts::PdfEncoding::name)
                },
                Vec::as_slice,
            );
        name.extend_from_slice(encoding_name.strip_suffix(b".enc").unwrap_or(encoding_name));
    } else {
        name.extend_from_slice(b"builtin");
    }
    name
}

pub(in crate::pdf::finalize) fn build_to_unicode_cmap(
    cmap_name: &[u8],
    mappings: &[ToUnicodeMapping],
) -> Vec<u8> {
    let cmap_name = String::from_utf8_lossy(cmap_name);
    let mut cmap = format!(
        "%!PS-Adobe-3.0 Resource-CMap\n%%DocumentNeededResources: ProcSet (CIDInit)\n%%IncludeResource: ProcSet (CIDInit)\n%%BeginResource: CMap (TeX-{cmap_name}-0)\n%%Title: (TeX-{cmap_name}-0 TeX {cmap_name} 0)\n%%Version: 1.000\n%%EndComments\n/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo\n<< /Registry (TeX)\n/Ordering ({cmap_name})\n/Supplement 0\n>> def\n/CMapName /TeX-{cmap_name}-0 def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n"
    )
    .into_bytes();

    let mut ranges = Vec::new();
    let mut characters = Vec::new();
    let mut index = 0;
    while index < mappings.len() {
        let mapping = &mappings[index];
        let ResolvedGlyphUnicode::Numeric(first_scalar) = mapping.mapping else {
            characters.push(mapping);
            index += 1;
            continue;
        };
        let mut end = index;
        while let Some(next) = mappings.get(end + 1) {
            let ResolvedGlyphUnicode::Numeric(current_scalar) = mappings[end].mapping else {
                break;
            };
            let ResolvedGlyphUnicode::Numeric(next_scalar) = next.mapping else {
                break;
            };
            if next.code != mappings[end].code.wrapping_add(1)
                || current_scalar.checked_add(1) != Some(next_scalar)
                || !is_last_byte_valid(mapping.code, mappings[end].code, current_scalar)
            {
                break;
            }
            end += 1;
        }
        if end == index {
            characters.push(mapping);
        } else {
            ranges.push((mapping.code, mappings[end].code, first_scalar));
        }
        index = end + 1;
    }

    for chunk in ranges.chunks(100) {
        cmap.extend_from_slice(format!("{} beginbfrange\n", chunk.len()).as_bytes());
        for (first, last, scalar) in chunk {
            cmap.extend_from_slice(format!("<{first:02X}> <{last:02X}> <").as_bytes());
            append_utf16be_hex(&mut cmap, &[*scalar]);
            cmap.extend_from_slice(b">\n");
        }
        cmap.extend_from_slice(b"endbfrange\n");
    }
    if ranges.is_empty() {
        cmap.extend_from_slice(b"0 beginbfrange\nendbfrange\n");
    }

    for chunk in characters.chunks(100) {
        cmap.extend_from_slice(format!("{} beginbfchar\n", chunk.len()).as_bytes());
        for mapping in chunk {
            cmap.extend_from_slice(format!("<{0:02X}> <", mapping.code).as_bytes());
            match &mapping.mapping {
                ResolvedGlyphUnicode::Numeric(scalar) => append_utf16be_hex(&mut cmap, &[*scalar]),
                ResolvedGlyphUnicode::String(scalars) => append_utf16be_hex(&mut cmap, scalars),
            }
            cmap.extend_from_slice(b">\n");
        }
        cmap.extend_from_slice(b"endbfchar\n");
    }
    if characters.is_empty() {
        cmap.extend_from_slice(b"0 beginbfchar\nendbfchar\n");
    }
    cmap.extend_from_slice(
        b"endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n%%EndResource\n%%EOF\n",
    );
    cmap
}

pub(in crate::pdf::finalize) fn append_utf16be_hex(output: &mut Vec<u8>, scalars: &[u32]) {
    for scalar in scalars {
        let mut encoded = [0; 2];
        for unit in char::from_u32(*scalar)
            .expect("validated Unicode scalar")
            .encode_utf16(&mut encoded)
        {
            output.extend_from_slice(format!("{unit:04X}").as_bytes());
        }
    }
}

pub(in crate::pdf::finalize) fn is_last_byte_valid(
    first_code: u8,
    current_code: u8,
    scalar: u32,
) -> bool {
    let mut encoded = [0; 2];
    let units = char::from_u32(scalar)
        .expect("validated Unicode scalar")
        .encode_utf16(&mut encoded);
    let last_byte = units.last().expect("one UTF-16 unit").to_be_bytes()[1];
    last_byte < u8::MAX - current_code.wrapping_sub(first_code)
}
