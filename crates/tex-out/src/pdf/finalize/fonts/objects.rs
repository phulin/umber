//! Objects helpers for PDF font assembly.

use super::*;

pub(in crate::pdf::finalize) fn pdf_font_objects(
    input: &PdfFontInput,
    ids: PdfFontObjectIds,
    font: &crate::FontResource,
    resource_name: &[u8],
    used_codes: &BTreeSet<u8>,
) -> Result<Vec<PdfIndirectObject>, PdfBuildError> {
    let mapped = input.map_entry.as_ref();
    let subset_requested = mapped
        .as_ref()
        .is_some_and(|entry| entry.program == tex_fonts::PdfFontMapProgram::Subset);
    let program_name = mapped.as_ref().and_then(|entry| entry.font_file.as_deref());
    let resident = mapped
        .as_ref()
        .is_some_and(|entry| entry.program == tex_fonts::PdfFontMapProgram::Resident);
    if mapped.is_none() {
        return pdf_pk_font_objects(input, ids, font, resource_name, used_codes);
    }
    if program_name.is_none() && !resident {
        return Err(PdfBuildError::MissingFontProgram(
            font.name.as_bytes().to_vec(),
        ));
    }
    let is_truetype = matches!(input.program, PdfFontProgramInput::TrueType(_));
    let type1 = match &input.program {
        PdfFontProgramInput::Type1(program) => Some(program),
        _ => None,
    };
    // pdfTeX's fd_entry retains the original built-in encoding independently
    // from the subset program written to FontFile. ToUnicode must see that
    // pre-subset table so unused but mapped encoding slots remain available.
    let to_unicode_type1 = type1;
    let truetype = match &input.program {
        PdfFontProgramInput::TrueType(program) => Some(program),
        _ => None,
    };
    if let Some(program_name) = program_name
        && type1.is_none()
        && truetype.is_none()
    {
        return Err(PdfBuildError::MissingFontProgram(program_name.to_vec()));
    }
    let base_font = truetype
        .and_then(tex_fonts::PdfTrueTypeProgram::postscript_name)
        .or_else(|| {
            mapped
                .as_ref()
                .and_then(|entry| entry.postscript_name.as_deref())
        })
        .unwrap_or(font.name.as_bytes())
        .to_vec();
    let encoding = input.encoding.as_ref();
    let glyph_names: BTreeSet<Vec<u8>> = if subset_requested {
        used_codes
            .iter()
            .map(|code| {
                if let Some(encoding) = encoding {
                    Ok(encoding.glyph_names()[usize::from(*code)].clone())
                } else if let Some(program) = type1 {
                    program.builtin_glyph_name(*code).ok_or_else(|| {
                        PdfBuildError::MissingBuiltinGlyphName {
                            font: font.name.clone(),
                            code: *code,
                        }
                    })
                } else {
                    Err(PdfBuildError::TrueTypeSubsetRequiresEncoding(
                        font.name.clone(),
                    ))
                }
            })
            .collect::<Result<_, _>>()?
    } else {
        BTreeSet::new()
    };
    let subset_tag =
        subset_requested.then(|| tex_fonts::pdftex_subset_tag(&glyph_names, &base_font));
    let subset_font_name = subset_tag
        .map(|tag| [tag.as_slice(), b"+", base_font.as_slice()].concat())
        .unwrap_or_else(|| base_font.clone());
    let subset_type1 = if subset_requested {
        type1
            .map(|program| {
                program
                    .subset(&glyph_names, &subset_font_name)
                    .map_err(|error| PdfBuildError::Type1Subset {
                        font: font.name.clone(),
                        error,
                    })
            })
            .transpose()?
    } else {
        None
    };
    let type1 = subset_type1.as_ref().or(type1);
    let subset_truetype = if subset_requested {
        truetype
            .map(|program| program.subset(&glyph_names))
            .transpose()?
    } else {
        None
    };
    let truetype = subset_truetype.as_ref().or(truetype);
    let mut dictionary = font_dictionary_header(PdfFontDictionaryHeader::Scalable {
        subtype: if is_truetype { "TrueType" } else { "Type1" },
        base_font: &subset_font_name,
    })?;
    if let Some(encoding) = encoding {
        if let Some(encoding_object) = ids.encoding {
            dictionary.insert("Encoding", PdfValue::Reference(encoding_object))?;
        } else {
            let differences = encoding_differences(encoding, used_codes, subset_requested);
            let mut encoding_dictionary = PdfDictionary::new();
            encoding_dictionary.insert("Type", PdfValue::Name("Encoding".into()))?;
            encoding_dictionary.insert("Differences", PdfValue::Array(differences))?;
            dictionary.insert("Encoding", PdfValue::Dictionary(encoding_dictionary))?;
        }
    }
    let first_char = if subset_requested {
        i64::from(*used_codes.first().expect("emitted font has used codes"))
    } else {
        0
    };
    let last_char = if subset_requested {
        i64::from(*used_codes.last().expect("emitted font has used codes"))
    } else {
        255
    };
    dictionary.insert("FirstChar", PdfValue::Integer(first_char))?;
    dictionary.insert("LastChar", PdfValue::Integer(last_char))?;
    let widths = (first_char as u8..=last_char as u8)
        .map(|code| {
            let coefficient =
                pdftex_scalable_width_tenths(input.metrics.widths[usize::from(code)], font.at_size)
                    .expect("validated scalable font has positive bounded metrics");
            PdfNumber::new(coefficient, 1).map(PdfValue::Number)
        })
        .collect::<Result<Vec<_>, _>>()?;
    dictionary.insert("Widths", PdfValue::Array(widths))?;
    let to_unicode = ids
        .to_unicode
        .map(|to_unicode_id| {
            to_unicode_stream(input, font, encoding, to_unicode_type1, to_unicode_id)
        })
        .transpose()?;
    if let Some((to_unicode_id, _)) = &to_unicode {
        dictionary.insert("ToUnicode", PdfValue::Reference(*to_unicode_id))?;
    }
    if resident {
        return Ok(vec![indirect_dictionary(ids.font, dictionary)]);
    }
    let descriptor_id = ids
        .descriptor
        .expect("mapped font allocation reserves descriptor");
    let program_id = ids
        .program
        .expect("mapped font allocation reserves program");
    dictionary.insert("FontDescriptor", PdfValue::Reference(descriptor_id))?;

    let mut descriptor = PdfDictionary::new();
    descriptor.insert("Type", PdfValue::Name("FontDescriptor".into()))?;
    descriptor.insert(
        "FontName",
        PdfValue::Name(PdfName::new(subset_font_name.clone())),
    )?;
    let [
        tfm_ascent,
        tfm_descent,
        tfm_cap_height,
        tfm_stem_v,
        tfm_x_height,
    ] = type1_fallback_descriptor_metrics(&input.metrics, font.at_size);
    let (bbox, ascent, descent, cap_height, x_height, italic_angle, stem_v) =
        if let Some(program) = truetype {
            (
                program.bbox(),
                i64::from(program.ascent()),
                i64::from(program.descent()),
                i64::from(program.cap_height()),
                i64::from(program.x_height()),
                i64::from(program.italic_angle()),
                i64::from(program.stem_v()),
            )
        } else {
            let program = type1.expect("program kind checked");
            (
                program.font_bbox().unwrap_or([-500, -500, 1500, 1500]),
                tfm_ascent,
                tfm_descent,
                tfm_cap_height,
                tfm_x_height,
                i64::from(program.italic_angle().unwrap_or(0)),
                type1_descriptor_stem_v(program, tfm_stem_v),
            )
        };
    // pdfTeX writefont.c::write_fontdescriptor uses the embedded-program
    // default directly; it does not infer fixed-pitch or italic bits from the
    // program metrics.
    descriptor.insert("Flags", PdfValue::Integer(4))?;
    descriptor.insert(
        "FontBBox",
        PdfValue::Array(
            bbox.into_iter()
                .map(|value| PdfValue::Integer(i64::from(value)))
                .collect(),
        ),
    )?;
    descriptor.insert("ItalicAngle", PdfValue::Integer(italic_angle))?;
    descriptor.insert("Ascent", PdfValue::Integer(ascent))?;
    descriptor.insert("Descent", PdfValue::Integer(descent))?;
    descriptor.insert("CapHeight", PdfValue::Integer(cap_height))?;
    descriptor.insert("StemV", PdfValue::Integer(stem_v))?;
    descriptor.insert("XHeight", PdfValue::Integer(x_height))?;
    descriptor.insert(
        if is_truetype { "FontFile2" } else { "FontFile" },
        PdfValue::Reference(program_id),
    )?;
    descriptor.set_raw_entries(input.descriptor_entries.clone());
    if subset_requested && !is_truetype && !input.omit_charset {
        let charset = glyph_names
            .iter()
            .filter(|name| name.as_slice() != b".notdef")
            .flat_map(|name| std::iter::once(b'/').chain(name.iter().copied()))
            .collect();
        descriptor.insert("CharSet", PdfValue::String(charset))?;
    }

    let mut stream = PdfDictionary::new();
    let data = if let Some(program) = truetype {
        stream.insert("Length1", PdfValue::Integer(program.bytes().len() as i64))?;
        program.bytes().to_vec()
    } else {
        let program = type1.expect("program kind checked");
        let [length1, length2, length3] = program.lengths();
        stream.insert("Length1", PdfValue::Integer(i64::from(length1)))?;
        stream.insert("Length2", PdfValue::Integer(i64::from(length2)))?;
        stream.insert("Length3", PdfValue::Integer(i64::from(length3)))?;
        program.bytes().to_vec()
    };
    let mut objects = vec![
        indirect_dictionary(ids.font, dictionary),
        indirect_dictionary(descriptor_id, descriptor),
        PdfIndirectObject {
            id: program_id,
            object: PdfObject::Stream {
                dictionary: stream,
                data,
            },
        },
    ];
    if let Some((_, stream)) = to_unicode {
        objects.push(stream);
    }
    Ok(objects)
}

/// pdfTeX's Type-1 descriptor fallbacks use named TFM characters, not the
/// extrema of the complete character table. In particular, `/StemV` starts as
/// one third of period's width and is replaced only when the font program has
/// `/StdVW`. The metrics are divided by the six-place PDF font-size raster,
/// rather than the original TeX font size. See pdftex.web §690,
/// `writefont.c::preset_fontmetrics`, and `writet1.c::t1_scan_keys` in the
/// pinned 1.40.29 source.
pub(in crate::pdf::finalize) fn type1_fallback_descriptor_metrics(
    metrics: &PdfFontMetricsInput,
    at_size: Scaled,
) -> [i64; 5] {
    let denominator = pdftex_font_size_raster(at_size)
        .expect("validated scalable font has a positive bounded size");
    let scale_metric = |value: Scaled| {
        pdftex_divide_scaled_positive(i64::from(value.raw()), denominator, 3)
            .expect("validated TFM descriptor metric is nonnegative and bounded")
            .0
    };
    [
        scale_metric(metrics.heights[usize::from(b'h')]),
        -scale_metric(metrics.depths[usize::from(b'y')]),
        scale_metric(metrics.heights[usize::from(b'H')]),
        scale_metric(Scaled::from_raw(
            metrics.widths[usize::from(b'.')].raw() / 3,
        )),
        scale_metric(metrics.x_height),
    ]
}

pub(in crate::pdf::finalize) fn type1_descriptor_stem_v(
    program: &tex_fonts::PdfType1Program,
    fallback: i64,
) -> i64 {
    program.stem_v().map_or(fallback, i64::from)
}

pub(in crate::pdf::finalize) fn pdf_pk_font_objects(
    input: &PdfFontInput,
    ids: PdfFontObjectIds,
    font: &crate::FontResource,
    resource_name: &[u8],
    used_codes: &BTreeSet<u8>,
) -> Result<Vec<PdfIndirectObject>, PdfBuildError> {
    let PdfFontProgramInput::Pk { request, font: pk } = &input.program else {
        return Err(PdfBuildError::MissingFontProgram(
            font.name.as_bytes().to_vec(),
        ));
    };
    let first_char = *used_codes
        .first()
        .ok_or_else(|| PdfBuildError::MissingFontUsage(font.name.clone()))?;
    let last_char = *used_codes.last().expect("nonempty usage checked");
    let matrix = rounded_pk_matrix(font.at_size, request.dpi())?;
    let mut font_bbox = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
    let mut char_procs = PdfDictionary::new();
    let mut encoding_differences = Vec::new();
    let mut widths = Vec::new();
    let mut objects = Vec::with_capacity(1 + used_codes.len());

    for code in first_char..=last_char {
        widths.push(PdfValue::Number(PdfNumber::new(
            pk_advance_hundredths(input.metrics.widths[usize::from(code)], request.dpi()),
            2,
        )?));
        if !used_codes.contains(&code) {
            continue;
        }
        let glyph = pk
            .glyph(u32::from(code))
            .ok_or_else(|| PdfBuildError::MissingPkGlyph {
                font: font.name.clone(),
                code,
            })?;
        let bbox = [
            -glyph.x_offset,
            glyph.y_offset - i32::try_from(glyph.height).expect("bounded PK height") + 1,
            -glyph.x_offset + i32::try_from(glyph.width).expect("bounded PK width") + 1,
            glyph.y_offset + 1,
        ];
        for index in 0..2 {
            font_bbox[index] = font_bbox[index].min(bbox[index]);
            font_bbox[index + 2] = font_bbox[index + 2].max(bbox[index + 2]);
        }
        let name = format!("a{code}").into_bytes();
        let id = ids.char_procs[&code];
        char_procs.insert(
            String::from_utf8_lossy(&name).as_ref(),
            PdfValue::Reference(id),
        )?;
        encoding_differences.push(PdfValue::Integer(i64::from(code)));
        encoding_differences.push(PdfValue::Name(PdfName::new(name)));
        let advance = pk_advance_hundredths(input.metrics.widths[usize::from(code)], request.dpi())
            as f32
            / 100.0;
        let data = crate::pdf::type3_bitmap_glyph_content(&crate::pdf::PdfType3BitmapGlyph {
            advance,
            bbox,
            width: glyph.width,
            height: glyph.height,
            x: -glyph.x_offset,
            y: bbox[1],
            bitmap: &glyph.bitmap,
        });
        objects.push(PdfIndirectObject {
            id,
            object: PdfObject::Stream {
                dictionary: PdfDictionary::new(),
                data,
            },
        });
    }

    let mut dictionary = font_dictionary_header(PdfFontDictionaryHeader::Type3 { resource_name })?;
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
        PdfValue::Array(
            font_bbox
                .into_iter()
                .map(|value| PdfValue::Integer(i64::from(value)))
                .collect(),
        ),
    )?;
    let mut resources = PdfDictionary::new();
    resources.insert(
        "ProcSet",
        PdfValue::Array(vec![
            PdfValue::Name("PDF".into()),
            PdfValue::Name("ImageB".into()),
        ]),
    )?;
    dictionary.insert("Resources", PdfValue::Dictionary(resources))?;
    dictionary.insert("FirstChar", PdfValue::Integer(i64::from(first_char)))?;
    dictionary.insert("LastChar", PdfValue::Integer(i64::from(last_char)))?;
    dictionary.insert("Widths", PdfValue::Array(widths))?;
    let mut encoding = PdfDictionary::new();
    encoding.insert("Type", PdfValue::Name("Encoding".into()))?;
    encoding.insert("Differences", PdfValue::Array(encoding_differences))?;
    dictionary.insert("Encoding", PdfValue::Dictionary(encoding))?;
    dictionary.insert("CharProcs", PdfValue::Dictionary(char_procs))?;
    objects.push(indirect_dictionary(ids.font, dictionary));
    Ok(objects)
}

pub(in crate::pdf::finalize) fn rounded_pk_matrix(
    at_size: Scaled,
    dpi: u32,
) -> Result<PdfNumber, PdfBuildError> {
    let denominator = i64::from(at_size.raw())
        .checked_mul(i64::from(dpi))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    if denominator <= 0 {
        return Err(PdfBuildError::PageGeometryOverflow);
    }
    let numerator = 7_227_i64 * 65_536 * 1_000;
    PdfNumber::new((numerator + denominator / 2) / denominator, 5).map_err(Into::into)
}

pub(in crate::pdf::finalize) fn pk_advance_hundredths(width: Scaled, dpi: u32) -> i64 {
    let numerator = i64::from(width.raw()) * i64::from(dpi) * 10_000;
    let denominator = 65_536_i64 * 7_227;
    (numerator + denominator / 2) / denominator
}
