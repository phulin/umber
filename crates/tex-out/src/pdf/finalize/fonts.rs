//! Private fonts lowering for PDF finalization.

use super::*;

mod objects;
mod unicode;

pub(super) use objects::*;
pub(super) use unicode::*;

#[derive(Clone)]
pub(super) struct PdfFontObjectIds {
    pub(super) font: PdfObjectId,
    pub(super) descriptor: Option<PdfObjectId>,
    pub(super) program: Option<PdfObjectId>,
    pub(super) to_unicode: Option<PdfObjectId>,
    pub(super) encoding: Option<PdfObjectId>,
    pub(super) char_procs: BTreeMap<u8, PdfObjectId>,
}

pub(super) struct PdfFontEncodings {
    pub(super) shared_names: BTreeSet<Vec<u8>>,
    pub(super) entries: BTreeMap<Vec<u8>, PdfFontEncodingEntry>,
}

pub(super) struct PdfFontEncodingEntry {
    object: PdfObjectId,
    encoding: tex_fonts::PdfEncoding,
    used_codes: BTreeSet<u8>,
}

impl PdfFontEncodings {
    pub(super) fn collect(
        input: &PdfFinalizationInput,
        font_usage: &BTreeMap<u32, BTreeSet<u8>>,
    ) -> Result<Self, PdfBuildError> {
        let mut users = BTreeMap::<Vec<u8>, BTreeSet<u32>>::new();
        for resource in input.fonts.values() {
            if !font_usage.contains_key(&resource.object_number) {
                continue;
            }
            let width_resource =
                mapped_width_resource(input, &resource.artifact_resource, resource)?;
            if let Some(name) = Self::encoding_name(width_resource) {
                users
                    .entry(name.to_vec())
                    .or_default()
                    .insert(resource.object_number);
            }
        }
        Ok(Self {
            shared_names: users
                .into_iter()
                .filter_map(|(name, users)| (users.len() > 1).then_some(name))
                .collect(),
            entries: BTreeMap::new(),
        })
    }

    fn encoding_name(input: &PdfFontInput) -> Option<&[u8]> {
        if matches!(
            input.program,
            PdfFontProgramInput::TrueType(_) | PdfFontProgramInput::Pk { .. }
        ) || input.encoding.is_none()
        {
            return None;
        }
        input
            .map_entry
            .as_ref()?
            .encoding_files
            .first()
            .map(Vec::as_slice)
    }

    /// Registers one externally reencoded Type-1 font and returns the shared
    /// encoding object allocated for its logical encoding file.
    ///
    /// pdfTeX keys `fe_entry` by encoding-file name, allocates `fe_objnum` on
    /// its first font dictionary, and accumulates every font's marked slots in
    /// the entry's `tx_tree`; see pdftex.web section 32e,
    /// `writefont.c::create_fontdictionary`, and `writeenc.c`.
    pub(super) fn register(
        &mut self,
        input: &PdfFontInput,
        used_codes: &BTreeSet<u8>,
        next_object: &mut u32,
    ) -> Result<Option<PdfObjectId>, PdfBuildError> {
        let Some(name) = Self::encoding_name(input) else {
            return Ok(None);
        };
        if !self.shared_names.contains(name) {
            return Ok(None);
        }
        let Some(encoding) = input.encoding.as_ref() else {
            unreachable!("shared encoding names require an encoding vector");
        };

        self.register_encoding(name, encoding, used_codes, next_object)
            .map(Some)
    }

    pub(super) fn register_encoding(
        &mut self,
        name: &[u8],
        encoding: &tex_fonts::PdfEncoding,
        used_codes: &BTreeSet<u8>,
        next_object: &mut u32,
    ) -> Result<PdfObjectId, PdfBuildError> {
        if let Some(entry) = self.entries.get_mut(name) {
            if entry.encoding != *encoding {
                return Err(PdfBuildError::ConflictingEncoding(name.to_vec()));
            }
            entry.used_codes.extend(used_codes);
            return Ok(entry.object);
        }

        let object = object_id(*next_object)?;
        *next_object = next_object
            .checked_add(1)
            .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?;
        self.entries.insert(
            name.to_vec(),
            PdfFontEncodingEntry {
                object,
                encoding: encoding.clone(),
                used_codes: used_codes.clone(),
            },
        );
        Ok(object)
    }

    pub(super) fn into_objects(self) -> Result<Vec<PdfIndirectObject>, PdfBuildError> {
        self.entries
            .into_values()
            .map(|entry| {
                let mut dictionary = PdfDictionary::new();
                dictionary.insert("Type", PdfValue::Name("Encoding".into()))?;
                dictionary.insert(
                    "Differences",
                    PdfValue::Array(encoding_differences(
                        &entry.encoding,
                        &entry.used_codes,
                        true,
                    )),
                )?;
                Ok(indirect_dictionary(entry.object, dictionary))
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
pub(super) enum PdfFontDictionaryHeader<'a> {
    Scalable {
        subtype: &'static str,
        base_font: &'a [u8],
    },
    Type3 {
        resource_name: &'a [u8],
    },
}

/// Starts a font dictionary with pdfTeX's subtype-specific identity fields.
///
/// Scalable Type-1 and TrueType dictionaries identify the font only through
/// `/BaseFont`; `/Name` belongs exclusively to Type-3 dictionaries. See
/// pdftex.web §32e, `writefont.c::write_fontdictionary`, and
/// `writet3.c::writet3` in the pinned pdfTeX 1.40.29 source.
pub(super) fn font_dictionary_header(
    header: PdfFontDictionaryHeader<'_>,
) -> Result<PdfDictionary, PdfModelError> {
    let mut dictionary = PdfDictionary::new();
    dictionary.insert("Type", PdfValue::Name("Font".into()))?;
    match header {
        PdfFontDictionaryHeader::Scalable { subtype, base_font } => {
            dictionary.insert("Subtype", PdfValue::Name(subtype.into()))?;
            dictionary.insert("BaseFont", PdfValue::Name(PdfName::new(base_font.to_vec())))?;
        }
        PdfFontDictionaryHeader::Type3 { resource_name } => {
            dictionary.insert("Subtype", PdfValue::Name("Type3".into()))?;
            dictionary.insert("Name", PdfValue::Name(PdfName::new(resource_name)))?;
        }
    }
    Ok(dictionary)
}

#[derive(Clone, Copy)]
pub(super) struct PdfFallbackSpaceFont {
    pub(super) font: PdfObjectId,
}

pub(super) fn allocate_fallback_space_font(
    selected_name: &[u8],
    next_object: &mut u32,
    objects: &mut Vec<PdfIndirectObject>,
) -> Result<PdfFallbackSpaceFont, PdfBuildError> {
    let font = object_id(*next_object)?;
    let char_proc = object_id(
        next_object
            .checked_add(1)
            .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?,
    )?;
    *next_object = next_object
        .checked_add(2)
        .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?;
    objects.push(PdfIndirectObject {
        id: char_proc,
        object: PdfObject::Stream {
            dictionary: PdfDictionary::new(),
            data: crate::pdf::type3_space_glyph_content(333.0),
        },
    });

    let matrix = PdfNumber::new(1, 3)?;
    let mut dictionary = font_dictionary_header(PdfFontDictionaryHeader::Type3 {
        resource_name: selected_name,
    })?;
    dictionary.insert(
        "FontMatrix",
        PdfValue::Array(vec![
            PdfValue::Number(matrix),
            PdfValue::Integer(0),
            PdfValue::Integer(0),
            PdfValue::Number(matrix),
            PdfValue::Integer(0),
            PdfValue::Integer(0),
        ]),
    )?;
    dictionary.insert(
        "FontBBox",
        PdfValue::Array(vec![
            PdfValue::Integer(0),
            PdfValue::Integer(0),
            PdfValue::Integer(0),
            PdfValue::Integer(0),
        ]),
    )?;
    dictionary.insert("Resources", PdfValue::Dictionary(PdfDictionary::new()))?;
    dictionary.insert("FirstChar", PdfValue::Integer(32))?;
    dictionary.insert("LastChar", PdfValue::Integer(32))?;
    dictionary.insert("Widths", PdfValue::Array(vec![PdfValue::Integer(333)]))?;
    let mut encoding = PdfDictionary::new();
    encoding.insert("Type", PdfValue::Name("Encoding".into()))?;
    encoding.insert(
        "Differences",
        PdfValue::Array(vec![PdfValue::Integer(32), PdfValue::Name("space".into())]),
    )?;
    dictionary.insert("Encoding", PdfValue::Dictionary(encoding))?;
    let mut char_procs = PdfDictionary::new();
    char_procs.insert("space", PdfValue::Reference(char_proc))?;
    dictionary.insert("CharProcs", PdfValue::Dictionary(char_procs))?;
    objects.push(indirect_dictionary(font, dictionary));
    Ok(PdfFallbackSpaceFont { font })
}

pub(super) fn ensure_fallback_space_font(
    selected_name: &[u8],
    next_object: &mut u32,
    objects: &mut Vec<PdfIndirectObject>,
    fallback: &mut Option<PdfFallbackSpaceFont>,
) -> Result<PdfFallbackSpaceFont, PdfBuildError> {
    if let Some(fallback) = *fallback {
        return Ok(fallback);
    }
    let allocated = allocate_fallback_space_font(selected_name, next_object, objects)?;
    *fallback = Some(allocated);
    Ok(allocated)
}

pub(super) fn font_has_explicit_space(font: &PdfFontInput) -> bool {
    font.encoding
        .as_ref()
        .is_some_and(|encoding| encoding.glyph_names()[32] == b"space")
}

pub(super) fn mapped_width_resource<'a>(
    input: &'a PdfFinalizationInput,
    font: &crate::FontResource,
    resource: &'a PdfFontInput,
) -> Result<&'a PdfFontInput, PdfBuildError> {
    match &font.construction {
        crate::FontResourceConstruction::Expanded {
            source_identity, ..
        } => input
            .fonts
            .get(source_identity)
            .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone())),
        crate::FontResourceConstruction::Loaded
        | crate::FontResourceConstruction::Copied { .. }
        | crate::FontResourceConstruction::Letterspaced { .. } => Ok(resource),
    }
}

pub(super) struct MappedTextContext<'a> {
    pub(super) resource: &'a PdfFontInput,
    pub(super) font: &'a crate::FontResource,
    pub(super) font_name: &'a [u8],
    pub(super) h_origin: Scaled,
    pub(super) decimal_digits: i32,
    pub(super) baseline: Scaled,
    pub(super) font_size: f32,
    pub(super) positioning_font_size: f64,
    pub(super) horizontal_scale: f32,
}

pub(super) fn mapped_text_segment(
    context: &MappedTextContext<'_>,
    bytes: &mut Vec<u8>,
    positions: &mut Vec<Scaled>,
    segment_x: &mut Option<Scaled>,
) -> Result<PdfContentOperation, PdfBuildError> {
    let anchor = segment_x
        .take()
        .expect("nonempty segment has an anchor")
        .checked_add(context.h_origin)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let advance = scalable_text_advance(
        context.resource,
        context.font,
        bytes,
        context.positioning_font_size,
        context.horizontal_scale,
    );
    let glyphs = scalable_glyph_rasters(
        context.resource,
        context.font,
        bytes,
        positions,
        context.h_origin,
        context.positioning_font_size,
        context.horizontal_scale,
    );
    Ok(PdfContentOperation::Text(PdfContentTextRun {
        x: scaled_to_bp_f32(anchor, context.decimal_digits),
        exact_position: exact_text_position(anchor, context.baseline, context.decimal_digits),
        raster: Some(PdfContentTextRaster {
            serialized_x: scaled_to_bp_f64(anchor, context.decimal_digits),
            position_x: scaled_to_bp_unrounded_f64(anchor),
            font_size: context.positioning_font_size,
            exact: exact_text_raster(context.font),
            glyphs: glyphs.unwrap_or_default(),
        }),
        baseline: scaled_to_bp_f32(context.baseline, context.decimal_digits),
        font_name: context.font_name.to_vec(),
        font_size: context.font_size,
        horizontal_scale: context.horizontal_scale,
        bytes: std::mem::take(bytes),
        advance,
    }))
}

pub(super) fn scalable_glyph_rasters(
    input: &PdfFontInput,
    font: &crate::FontResource,
    bytes: &[u8],
    positions: &mut Vec<Scaled>,
    h_origin: Scaled,
    font_size: f64,
    horizontal_scale: f32,
) -> Option<Vec<PdfContentGlyphRaster>> {
    let positions = std::mem::take(positions);
    input.map_entry.as_ref()?;
    if positions.len() != bytes.len() {
        return None;
    }
    positions
        .into_iter()
        .zip(bytes)
        .map(|(position, &code)| {
            let position = position.checked_add(h_origin)?;
            let width_tenths = pdftex_scalable_width_tenths(
                input.metrics.widths[usize::from(code)],
                font.at_size,
            )?;
            Some(PdfContentGlyphRaster {
                position_x: scaled_to_bp_unrounded_f64(position),
                advance: width_tenths as f64 * font_size * f64::from(horizontal_scale) / 10_000.0,
                position_raw: i64::from(position.raw()),
                width_raw: positioned_char_width_raw(
                    input.metrics.widths[usize::from(code)],
                    &font.construction,
                )?,
            })
        })
        .collect()
}

pub(super) fn exact_text_raster(font: &crate::FontResource) -> Option<PdfContentTextExactRaster> {
    const ONE_HUNDRED_BP: i64 = 6_578_176;
    let (_, font_size) =
        pdftex_divide_scaled_positive(i64::from(font.at_size.raw()), ONE_HUNDRED_BP, 6)?;
    let expansion_ratio = match font.construction {
        crate::FontResourceConstruction::Expanded { ratio, .. } => ratio,
        crate::FontResourceConstruction::Loaded
        | crate::FontResourceConstruction::Copied { .. }
        | crate::FontResourceConstruction::Letterspaced { .. } => 0,
    };
    Some(PdfContentTextExactRaster {
        font_size,
        expansion_ratio,
    })
}

pub(super) fn exact_text_position(
    h: Scaled,
    v: Scaled,
    decimal_digits: i32,
) -> Option<PdfContentTextPosition> {
    Some(PdfContentTextPosition {
        h: i64::from(h.raw()),
        v: i64::from(v.raw()),
        decimal_digits: u8::try_from(decimal_digits).ok()?,
    })
}

pub(super) fn positioned_char_width_raw(
    width: Scaled,
    construction: &crate::FontResourceConstruction,
) -> Option<i64> {
    match construction {
        crate::FontResourceConstruction::Expanded { ratio, .. } => {
            let numerator = i64::from(width.raw()).checked_mul(1000 + i64::from(*ratio))?;
            Some(if numerator >= 0 {
                (numerator + 500) / 1000
            } else {
                -((-numerator + 500) / 1000)
            })
        }
        crate::FontResourceConstruction::Loaded
        | crate::FontResourceConstruction::Copied { .. }
        | crate::FontResourceConstruction::Letterspaced { .. } => Some(i64::from(width.raw())),
    }
}

pub(in crate::pdf) fn positioned_char_end(
    position: Scaled,
    width: Scaled,
    construction: &crate::FontResourceConstruction,
) -> Result<Scaled, PdfBuildError> {
    let width = Scaled::from_raw(
        i32::try_from(
            positioned_char_width_raw(width, construction)
                .ok_or(PdfBuildError::PageGeometryOverflow)?,
        )
        .map_err(|_| PdfBuildError::PageGeometryOverflow)?,
    );
    position
        .checked_add(width)
        .ok_or(PdfBuildError::PageGeometryOverflow)
}

pub(in crate::pdf) fn font_horizontal_scale(construction: &crate::FontResourceConstruction) -> f32 {
    match construction {
        crate::FontResourceConstruction::Expanded { ratio, .. } => {
            (1000.0 + f32::from(*ratio)) / 1000.0
        }
        crate::FontResourceConstruction::Loaded
        | crate::FontResourceConstruction::Copied { .. }
        | crate::FontResourceConstruction::Letterspaced { .. } => 1.0,
    }
}

pub(super) fn scalable_text_advance(
    input: &PdfFontInput,
    font: &crate::FontResource,
    bytes: &[u8],
    font_size: f64,
    horizontal_scale: f32,
) -> Option<f64> {
    input.map_entry.as_ref()?;
    let width_tenths = bytes.iter().try_fold(0_i64, |total, &code| {
        total.checked_add(pdftex_scalable_width_tenths(
            input.metrics.widths[usize::from(code)],
            font.at_size,
        )?)
    })?;
    Some(width_tenths as f64 * font_size * f64::from(horizontal_scale) / 10_000.0)
}
