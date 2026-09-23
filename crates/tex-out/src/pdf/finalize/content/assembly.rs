//! Ordered page and form content object assembly.

use super::*;

/// Borrows the finalizer's one object collection and allocation cursor while
/// lowering page and form content. No document state is published here.
pub(in crate::pdf::finalize) struct ContentInputs<'a> {
    pub input: &'a PdfFinalizationInput,
    pub positioned_pages: &'a [PositionedPage],
    pub positioned_forms: &'a BTreeMap<u32, PositionedPage>,
    pub mapped_font_names: &'a BTreeSet<Vec<u8>>,
    pub font_usage: &'a BTreeMap<u32, BTreeSet<u8>>,
    pub pdf_image_objects: &'a BTreeMap<u32, PdfObjectId>,
    pub pdf_image_groups: &'a BTreeMap<u32, Option<PdfObjectId>>,
    pub transparent_raster_group: Option<PdfObjectId>,
    pub parameters: FinalizationParameters,
    pub pages_id: PdfObjectId,
    pub page_annotations: &'a [Vec<ShippedAnnotation>],
    pub thread_output: &'a ThreadOutput,
    pub objects: &'a mut Vec<PdfIndirectObject>,
    pub next_object: &'a mut u32,
    pub font_encodings: &'a mut PdfFontEncodings,
}

pub(in crate::pdf::finalize) struct ContentOutput {
    pub kids: Vec<PdfValue>,
    pub diagnostics: Vec<String>,
    pub font_embed_ns: u128,
}

#[allow(clippy::disallowed_methods)] // Existing font timing is optional process telemetry.
pub(in crate::pdf::finalize) fn append_content_objects(
    inputs: ContentInputs<'_>,
) -> Result<ContentOutput, PdfBuildError> {
    let ContentInputs {
        input,
        positioned_pages,
        positioned_forms,
        mapped_font_names,
        font_usage,
        pdf_image_objects,
        pdf_image_groups,
        transparent_raster_group,
        parameters,
        pages_id,
        page_annotations,
        thread_output,
        objects,
        next_object: next_object_ref,
        font_encodings,
    } = inputs;
    let page_records = &input.pages;
    let mut next_object = *next_object_ref;
    let mut kids = Vec::with_capacity(page_records.len());
    let mut emitted_fonts = BTreeSet::new();
    let mut interword_space_enabled = false;
    let mut fallback_space_font = None;
    let mut diagnostics = Vec::new();
    let mut referenced_forms = input
        .forms
        .values()
        .filter(|form| form.immediate)
        .map(|form| form.object)
        .collect::<BTreeSet<_>>();
    let mut font_embed_ns = 0_u128;
    for (page_index, record) in page_records.iter().enumerate() {
        let artifact = PageArtifact::from_bytes(&record.artifact_bytes)?;
        let positioned = positioned_pages[page_index].clone();
        let (page_width, page_height) = pdf_page_extents(&artifact, record)?;
        let mut content_operations = Vec::new();
        let mut page_forms = BTreeMap::<u32, PdfObjectId>::new();
        let mut page_images = BTreeMap::<Vec<u8>, PdfObjectId>::new();
        let mut page_group_selected = false;
        let mut page_group = None;
        let mut procset = PdfProcSetUsage::default();
        let mut page_fonts = std::collections::BTreeMap::new();
        let mut fallback_space_on_page = false;
        for event in positioned.events {
            match event {
                PositionedEvent::Rule(rule) => {
                    content_operations.push(PdfContentOperation::Rule(PdfContentRule {
                        x: rule
                            .x
                            .checked_add(record.h_origin())
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        y: page_height
                            .checked_sub(rule.y)
                            .and_then(|value| value.checked_sub(record.v_origin()))
                            .and_then(|value| value.checked_sub(rule.height))
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        width: rule.width,
                        height: rule.height,
                        decimal_digits: parameters.decimal_digits as u8,
                    }))
                }
                PositionedEvent::TextRun(run) if !run.units.is_empty() => {
                    let font = positioned
                        .fonts
                        .iter()
                        .find(|font| font.font_id == run.font_id)
                        .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
                    let resource = input
                        .fonts
                        .get(&font.semantic_identity)
                        .ok_or(PdfBuildError::MissingFontResource(font.name.clone()))?;
                    let width_resource = mapped_width_resource(input, font, resource)?;
                    let resource_name = format!("F{}", resource.resource_number).into_bytes();
                    let font_id = match page_fonts.get(&resource.resource_number).copied() {
                        Some(id) => id,
                        None => {
                            let id = object_id(resource.object_number)?;
                            page_fonts.insert(resource.resource_number, id);
                            if emitted_fonts.insert(resource.object_number) {
                                let used_codes =
                                    font_usage.get(&resource.object_number).ok_or_else(|| {
                                        PdfBuildError::MissingFontUsage(font.name.clone())
                                    })?;
                                let mapped = mapped_font_names.contains(font.name.as_bytes());
                                let ids = if mapped {
                                    let encoding = font_encodings.register(
                                        width_resource,
                                        used_codes,
                                        &mut next_object,
                                    )?;
                                    let descriptor = object_id(next_object)?;
                                    let program = object_id(
                                        next_object
                                            .checked_add(1)
                                            .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?,
                                    )?;
                                    let wants_to_unicode = resource.generate_to_unicode
                                        && !resource.disable_builtin_to_unicode;
                                    let to_unicode = wants_to_unicode
                                        .then(|| object_id(next_object.saturating_add(2)))
                                        .transpose()?;
                                    next_object = next_object
                                        .checked_add(if wants_to_unicode { 3 } else { 2 })
                                        .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?;
                                    PdfFontObjectIds {
                                        font: id,
                                        descriptor: Some(descriptor),
                                        program: Some(program),
                                        to_unicode,
                                        encoding,
                                        char_procs: BTreeMap::new(),
                                    }
                                } else {
                                    let mut char_procs = BTreeMap::new();
                                    for &code in used_codes {
                                        char_procs.insert(code, object_id(next_object)?);
                                        next_object = next_object
                                            .checked_add(1)
                                            .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?;
                                    }
                                    PdfFontObjectIds {
                                        font: id,
                                        descriptor: None,
                                        program: None,
                                        to_unicode: None,
                                        encoding: None,
                                        char_procs,
                                    }
                                };
                                let font_started = std::time::Instant::now();
                                objects.extend(pdf_font_objects(
                                    width_resource,
                                    ids,
                                    font,
                                    &resource_name,
                                    used_codes,
                                )?);
                                font_embed_ns += font_started.elapsed().as_nanos();
                            }
                            id
                        }
                    };
                    debug_assert_eq!(page_fonts.get(&resource.resource_number), Some(&font_id));
                    debug_assert_eq!(run.units.len(), run.positions.len());
                    debug_assert_eq!(run.units.len(), run.physical_codes.len());
                    let baseline = page_height
                        .checked_sub(run.baseline)
                        .and_then(|value| value.checked_sub(record.v_origin()))
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    let font_size = pdftex_font_size(font.at_size);
                    let positioning_font_size = pdftex_font_size_f64(font.at_size);
                    let horizontal_scale = font_horizontal_scale(&font.construction);
                    let explicit_space = font_has_explicit_space(width_resource);
                    let text_context = MappedTextContext {
                        resource: width_resource,
                        font,
                        font_name: &resource_name,
                        h_origin: record.h_origin(),
                        decimal_digits: parameters.decimal_digits,
                        baseline,
                        font_size,
                        positioning_font_size,
                        horizontal_scale,
                    };
                    let mut segment = Vec::new();
                    let mut segment_positions = Vec::new();
                    let mut segment_x = None;
                    let mut segment_end = None;
                    for ((unit, position), physical_code) in run
                        .units
                        .iter()
                        .zip(&run.positions)
                        .zip(&run.physical_codes)
                    {
                        match unit {
                            crate::positioned::TextUnit::Code(_) => {
                                if let Some(code) = physical_code {
                                    if !segment.is_empty() && segment_end != Some(*position) {
                                        content_operations.push(mapped_text_segment(
                                            &text_context,
                                            &mut segment,
                                            &mut segment_positions,
                                            &mut segment_x,
                                        )?);
                                    }
                                    segment_x.get_or_insert(*position);
                                    segment.push(*code);
                                    segment_positions.push(*position);
                                    segment_end = Some(positioned_char_end(
                                        *position,
                                        width_resource.metrics.widths[usize::from(*code)],
                                        &font.construction,
                                    )?);
                                }
                            }
                            crate::positioned::TextUnit::Space => {
                                if !segment.is_empty() {
                                    content_operations.push(mapped_text_segment(
                                        &text_context,
                                        &mut segment,
                                        &mut segment_positions,
                                        &mut segment_x,
                                    )?);
                                }
                                segment_end = None;
                                if interword_space_enabled {
                                    let (font_name, space_size, space_horizontal_scale) =
                                        if explicit_space {
                                            (resource_name.clone(), font_size, horizontal_scale)
                                        } else {
                                            ensure_fallback_space_font(
                                                &record.space_font_name,
                                                &mut next_object,
                                                objects,
                                                &mut fallback_space_font,
                                            )?;
                                            fallback_space_on_page = true;
                                            (b"UmberSpace".to_vec(), 10.0, 1.0)
                                        };
                                    content_operations.push(PdfContentOperation::Text(
                                        PdfContentTextRun {
                                            x: scaled_to_bp_f32(
                                                position
                                                    .checked_add(record.h_origin())
                                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                                parameters.decimal_digits,
                                            ),
                                            exact_position: exact_text_position(
                                                position
                                                    .checked_add(record.h_origin())
                                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                                baseline,
                                                parameters.decimal_digits,
                                            ),
                                            raster: None,
                                            baseline: scaled_to_bp_f32(
                                                baseline,
                                                parameters.decimal_digits,
                                            ),
                                            font_name,
                                            font_size: space_size,
                                            horizontal_scale: space_horizontal_scale,
                                            bytes: vec![b' '],
                                            advance: None,
                                        },
                                    ));
                                }
                            }
                        }
                    }
                    if !segment.is_empty() {
                        content_operations.push(mapped_text_segment(
                            &text_context,
                            &mut segment,
                            &mut segment_positions,
                            &mut segment_x,
                        )?);
                    }
                }
                PositionedEvent::PdfAccessibility(control) => match control.control {
                    crate::PdfAccessibilityEffect::InterwordSpaceOn => {
                        interword_space_enabled = true;
                    }
                    crate::PdfAccessibilityEffect::InterwordSpaceOff => {
                        interword_space_enabled = false;
                    }
                    crate::PdfAccessibilityEffect::FakeSpace => {
                        ensure_fallback_space_font(
                            &record.space_font_name,
                            &mut next_object,
                            objects,
                            &mut fallback_space_font,
                        )?;
                        fallback_space_on_page = true;
                        content_operations.push(PdfContentOperation::Text(PdfContentTextRun {
                            x: scaled_to_bp_f32(
                                control
                                    .x
                                    .checked_add(record.h_origin())
                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                parameters.decimal_digits,
                            ),
                            exact_position: exact_text_position(
                                control
                                    .x
                                    .checked_add(record.h_origin())
                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                page_height
                                    .checked_sub(control.y)
                                    .and_then(|value| value.checked_sub(record.v_origin()))
                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                parameters.decimal_digits,
                            ),
                            raster: None,
                            baseline: scaled_to_bp_f32(
                                page_height
                                    .checked_sub(control.y)
                                    .and_then(|value| value.checked_sub(record.v_origin()))
                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                parameters.decimal_digits,
                            ),
                            font_name: b"UmberSpace".to_vec(),
                            font_size: 10.0,
                            horizontal_scale: 1.0,
                            bytes: vec![b' '],
                            advance: None,
                        }));
                    }
                },
                PositionedEvent::PdfAnnotation(_) => {}
                PositionedEvent::Special(special) if special.class == "dvi" => {}
                PositionedEvent::Special(special) => {
                    return Err(PdfBuildError::UnsupportedSpecial(special.class));
                }
                PositionedEvent::PdfGraphics(graphics) => {
                    let raw_x = graphics
                        .x
                        .checked_add(record.h_origin())
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    let raw_y = page_height
                        .checked_sub(graphics.y)
                        .and_then(|value| value.checked_sub(record.v_origin()))
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    // Keep placement conversion lazy: imported PDF pages use the
                    // checked fixed-point path below and must not cross through f32.
                    let x = || scaled_to_bp_f32(raw_x, parameters.decimal_digits);
                    let y = || scaled_to_bp_f32(raw_y, parameters.decimal_digits);
                    let exact_position =
                        exact_text_position(raw_x, raw_y, parameters.decimal_digits);
                    let operation = match graphics.effect {
                        crate::PageEffect::PdfLiteral { mode, payload } => {
                            PdfContentOperation::Literal {
                                mode,
                                x: x(),
                                y: y(),
                                exact_position,
                                bytes: payload,
                            }
                        }
                        crate::PageEffect::PdfSetMatrix { payload } => {
                            PdfContentOperation::SetMatrix {
                                x: x(),
                                y: y(),
                                exact_position,
                                matrix: parse_pdf_matrix(&payload)?,
                            }
                        }
                        crate::PageEffect::PdfSave => PdfContentOperation::Save {
                            x: x(),
                            y: y(),
                            exact_position,
                        },
                        crate::PageEffect::PdfRestore => PdfContentOperation::Restore {
                            x: x(),
                            y: y(),
                            exact_position,
                        },
                        crate::PageEffect::PdfColorStack { mode, payload, .. } => {
                            PdfContentOperation::ColorStack {
                                mode,
                                x: x(),
                                y: y(),
                                exact_position,
                                bytes: payload,
                            }
                        }
                        crate::PageEffect::PdfRefXForm { object, .. } => {
                            let form = input
                                .forms
                                .get(&object)
                                .ok_or(PdfBuildError::ReferencedFormNotFound(object))?;
                            let form_y = page_height
                                .checked_sub(graphics.y)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_sub(form.depth()))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let form_id = object_id(form.object())?;
                            referenced_forms.insert(form.object());
                            page_forms.insert(form.resource(), form_id);
                            PdfContentOperation::FormXObject {
                                x: x(),
                                y: scaled_to_bp_f32(form_y, parameters.decimal_digits),
                                name: format!("Fm{}", form.resource()).into_bytes(),
                            }
                        }
                        crate::PageEffect::PdfRefXImage {
                            object,
                            width,
                            height,
                            depth,
                        } => {
                            let image = input
                                .images
                                .get(&object)
                                .ok_or(PdfBuildError::MissingRasterImage(object))?;
                            procset.include_image(image.metadata);
                            if raster_needs_transparency_page_group(
                                image.metadata,
                                input.document.version,
                            ) && page_group.is_none()
                            {
                                page_group = transparent_raster_group;
                                page_group_selected = page_group.is_some();
                            }
                            if matches!(image.metadata, PdfImageMetadataInput::PdfPage { .. }) {
                                let group = pdf_image_groups.get(&object).copied().flatten();
                                if group.is_some() {
                                    if !page_group_selected {
                                        page_group_selected = true;
                                        page_group = group;
                                    } else if !input.document.suppress_page_group_warning {
                                        diagnostics.push("PDF inclusion: multiple pdfs with page group included in a single page".to_owned());
                                    }
                                }
                            }
                            let name = image_resource_name(image, parameters);
                            let image_object = pdf_image_objects
                                .get(&object)
                                .copied()
                                .ok_or(PdfBuildError::MissingRasterImage(object))?;
                            page_images.insert(name.clone(), image_object);
                            let total_height = height
                                .checked_add(depth)
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let image_y = page_height
                                .checked_sub(graphics.y)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_sub(depth))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            match image.metadata {
                                PdfImageMetadataInput::PdfPage {
                                    page_box, rotation, ..
                                } => PdfContentOperation::ImportedPdfPage {
                                    matrix: imported_pdf_page_matrix(
                                        raw_x,
                                        image_y,
                                        width,
                                        total_height,
                                        page_box,
                                        rotation,
                                        parameters.decimal_digits,
                                    )?,
                                    name,
                                },
                                PdfImageMetadataInput::Raster { .. } => {
                                    PdfContentOperation::ImageXObject {
                                        x: x(),
                                        y: scaled_to_bp_f32(image_y, parameters.decimal_digits),
                                        width: scaled_to_bp_f32(width, parameters.decimal_digits),
                                        height: scaled_to_bp_f32(
                                            total_height,
                                            parameters.decimal_digits,
                                        ),
                                        name,
                                    }
                                }
                            }
                        }
                        _ => unreachable!("positioned PDF graphics event contains PDF effect"),
                    };
                    content_operations.push(operation);
                }
                PositionedEvent::Box(_)
                | PositionedEvent::BoxEnd(_)
                | PositionedEvent::PdfDestination(_)
                | PositionedEvent::PdfThread(_)
                | PositionedEvent::PdfEndThread { .. }
                | PositionedEvent::TextRun(_) => {}
            }
        }

        let resources_id = object_id(record.resources_object())?;
        let contents_id = object_id(record.contents_object())?;
        let page_id = object_id(record.page_object())?;
        kids.push(PdfValue::Reference(page_id));
        let mut resources = PdfDictionary::new();
        if record.omit_procset() < 0 || (record.omit_procset() == 0 && parameters.major_version < 2)
        {
            procset.include_text(!page_fonts.is_empty() || fallback_space_on_page);
            resources.insert("ProcSet", procset.into_pdf_array())?;
        }
        if !page_fonts.is_empty() || fallback_space_on_page {
            let mut fonts = PdfDictionary::new();
            for (resource_number, object) in page_fonts {
                fonts.insert(
                    format!("F{resource_number}").as_str(),
                    PdfValue::Reference(object),
                )?;
            }
            if fallback_space_on_page {
                let fallback = fallback_space_font.expect("page fallback use allocated its font");
                fonts.insert("UmberSpace", PdfValue::Reference(fallback.font))?;
            }
            resources.insert("Font", PdfValue::Dictionary(fonts))?;
        }
        if !page_forms.is_empty() || !page_images.is_empty() {
            let mut xobjects = PdfDictionary::new();
            for (resource, object) in page_forms {
                xobjects.insert(
                    format!("Fm{resource}").as_str(),
                    PdfValue::Reference(object),
                )?;
            }
            for (name, object) in page_images {
                xobjects.insert(
                    std::str::from_utf8(&name).expect("generated image resource name is ASCII"),
                    PdfValue::Reference(object),
                )?;
            }
            resources.insert("XObject", PdfValue::Dictionary(xobjects))?;
        }
        resources.set_raw_entries(record.resource_entries.clone());
        objects.push(indirect_dictionary(resources_id, resources));
        objects.push(PdfIndirectObject {
            id: contents_id,
            object: PdfObject::Stream {
                dictionary: PdfDictionary::new(),
                // pdftex.web §§729, 731--734 walk the shipped list in node
                // order. Each rule calls `pdf_set_rule`, which ends the active
                // text object (§691), before traversal resumes at the next node.
                data: ordered_page_content(&content_operations),
            },
        });

        let mut page = PdfDictionary::new();
        page.insert("Type", PdfValue::Name("Page".into()))?;
        page.insert("Parent", PdfValue::Reference(pages_id))?;
        let page_attr = record.page_entries.clone();
        if !page_attr
            .windows(b"/MediaBox".len())
            .any(|window| window == b"/MediaBox")
        {
            page.insert(
                "MediaBox",
                PdfValue::Array(vec![
                    PdfValue::Integer(0),
                    PdfValue::Integer(0),
                    PdfValue::Number(scaled_to_bp_number(page_width, parameters.decimal_digits)?),
                    PdfValue::Number(scaled_to_bp_number(page_height, parameters.decimal_digits)?),
                ]),
            )?;
        }
        page.insert("Resources", PdfValue::Reference(resources_id))?;
        page.insert("Contents", PdfValue::Reference(contents_id))?;
        if let Some(group) = page_group {
            page.insert("Group", PdfValue::Reference(group))?;
        }
        let shipped_annotations = &page_annotations[page_index];
        if !shipped_annotations.is_empty() {
            page.insert(
                "Annots",
                PdfValue::Array(
                    shipped_annotations
                        .iter()
                        .map(|annotation| object_id(annotation.object).map(PdfValue::Reference))
                        .collect::<Result<_, _>>()?,
                ),
            )?;
        }
        if let Some(beads) = thread_output.page_beads.get(page_index)
            && !beads.is_empty()
        {
            page.insert(
                "B",
                PdfValue::Array(beads.iter().copied().map(PdfValue::Reference).collect()),
            )?;
        }
        page.set_raw_entries(page_attr);
        for annotation in shipped_annotations {
            objects.push(annotation_object(
                input,
                *annotation,
                record,
                page_height,
                page_records,
                parameters.decimal_digits,
            )?);
        }
        objects.push(indirect_dictionary(page_id, page));
    }

    let mut pending_forms = referenced_forms.into_iter().collect::<VecDeque<_>>();
    let mut emitted_form_objects = BTreeSet::new();
    while let Some(object) = pending_forms.pop_front() {
        if !emitted_form_objects.insert(object) {
            continue;
        }
        let form = input
            .forms
            .get(&object)
            .ok_or(PdfBuildError::ReferencedFormNotFound(object))?;
        let positioned = positioned_forms
            .get(&object)
            .cloned()
            .ok_or(PdfBuildError::MissingFormArtifact(object))?;
        let total_height = form
            .height()
            .checked_add(form.depth())
            .ok_or(PdfBuildError::PageGeometryOverflow)?;
        let mut operations = Vec::new();
        let mut nested_forms = BTreeMap::<u32, PdfObjectId>::new();
        let mut form_images = BTreeMap::<Vec<u8>, PdfObjectId>::new();
        let mut form_fonts = BTreeMap::<u32, PdfObjectId>::new();
        let mut form_procset = PdfProcSetUsage::default();
        for event in positioned.events {
            match event {
                PositionedEvent::Rule(rule) => {
                    operations.push(PdfContentOperation::Rule(PdfContentRule {
                        x: rule.x,
                        y: total_height
                            .checked_sub(rule.y)
                            .and_then(|value| value.checked_sub(rule.height))
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        width: rule.width,
                        height: rule.height,
                        decimal_digits: parameters.decimal_digits as u8,
                    }))
                }
                PositionedEvent::PdfGraphics(graphics) => {
                    let raw_x = graphics.x;
                    let raw_y = total_height
                        .checked_sub(graphics.y)
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    // Keep placement conversion lazy: imported PDF pages use the
                    // checked fixed-point path below and must not cross through f32.
                    let x = || scaled_to_bp_f32(raw_x, parameters.decimal_digits);
                    let y = || scaled_to_bp_f32(raw_y, parameters.decimal_digits);
                    let exact_position =
                        exact_text_position(raw_x, raw_y, parameters.decimal_digits);
                    let operation = match graphics.effect {
                        crate::PageEffect::PdfLiteral { mode, payload } => {
                            PdfContentOperation::Literal {
                                mode,
                                x: x(),
                                y: y(),
                                exact_position,
                                bytes: payload,
                            }
                        }
                        crate::PageEffect::PdfSetMatrix { payload } => {
                            PdfContentOperation::SetMatrix {
                                x: x(),
                                y: y(),
                                exact_position,
                                matrix: parse_pdf_matrix(&payload)?,
                            }
                        }
                        crate::PageEffect::PdfSave => PdfContentOperation::Save {
                            x: x(),
                            y: y(),
                            exact_position,
                        },
                        crate::PageEffect::PdfRestore => PdfContentOperation::Restore {
                            x: x(),
                            y: y(),
                            exact_position,
                        },
                        crate::PageEffect::PdfColorStack { mode, payload, .. } => {
                            PdfContentOperation::ColorStack {
                                mode,
                                x: x(),
                                y: y(),
                                exact_position,
                                bytes: payload,
                            }
                        }
                        crate::PageEffect::PdfRefXForm { object, .. } => {
                            let nested = input
                                .forms
                                .get(&object)
                                .ok_or(PdfBuildError::ReferencedFormNotFound(object))?;
                            if object == form.object() {
                                return Err(PdfBuildError::RecursiveForm(object));
                            }
                            nested_forms.insert(nested.resource(), object_id(object)?);
                            pending_forms.push_back(object);
                            let form_y = total_height
                                .checked_sub(graphics.y)
                                .and_then(|value| value.checked_sub(nested.depth()))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            PdfContentOperation::FormXObject {
                                x: x(),
                                y: scaled_to_bp_f32(form_y, parameters.decimal_digits),
                                name: format!("Fm{}", nested.resource()).into_bytes(),
                            }
                        }
                        crate::PageEffect::PdfRefXImage {
                            object,
                            width,
                            height,
                            depth,
                        } => {
                            let image = input
                                .images
                                .get(&object)
                                .ok_or(PdfBuildError::MissingRasterImage(object))?;
                            form_procset.include_image(image.metadata);
                            let name = image_resource_name(image, parameters);
                            let image_object = pdf_image_objects
                                .get(&object)
                                .copied()
                                .ok_or(PdfBuildError::MissingRasterImage(object))?;
                            form_images.insert(name.clone(), image_object);
                            let total_image_height = height
                                .checked_add(depth)
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let image_y = total_height
                                .checked_sub(graphics.y)
                                .and_then(|value| value.checked_sub(depth))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            match image.metadata {
                                PdfImageMetadataInput::PdfPage {
                                    page_box, rotation, ..
                                } => PdfContentOperation::ImportedPdfPage {
                                    matrix: imported_pdf_page_matrix(
                                        raw_x,
                                        image_y,
                                        width,
                                        total_image_height,
                                        page_box,
                                        rotation,
                                        parameters.decimal_digits,
                                    )?,
                                    name,
                                },
                                PdfImageMetadataInput::Raster { .. } => {
                                    PdfContentOperation::ImageXObject {
                                        x: x(),
                                        y: scaled_to_bp_f32(image_y, parameters.decimal_digits),
                                        width: scaled_to_bp_f32(width, parameters.decimal_digits),
                                        height: scaled_to_bp_f32(
                                            total_image_height,
                                            parameters.decimal_digits,
                                        ),
                                        name,
                                    }
                                }
                            }
                        }
                        _ => continue,
                    };
                    operations.push(operation);
                }
                PositionedEvent::TextRun(run) if !run.units.is_empty() => {
                    let font = positioned
                        .fonts
                        .iter()
                        .find(|font| font.font_id == run.font_id)
                        .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
                    let resource = input
                        .fonts
                        .get(&font.semantic_identity)
                        .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
                    let width_resource = mapped_width_resource(input, font, resource)?;
                    let resource_name = format!("F{}", resource.resource_number).into_bytes();
                    let font_id = object_id(resource.object_number)?;
                    form_fonts.insert(resource.resource_number, font_id);
                    if emitted_fonts.insert(resource.object_number) {
                        let used_codes = font_usage
                            .get(&resource.object_number)
                            .ok_or_else(|| PdfBuildError::MissingFontUsage(font.name.clone()))?;
                        let mapped = mapped_font_names.contains(font.name.as_bytes());
                        let ids = if mapped {
                            let encoding = font_encodings.register(
                                width_resource,
                                used_codes,
                                &mut next_object,
                            )?;
                            let descriptor = object_id(next_object)?;
                            let program = object_id(
                                next_object
                                    .checked_add(1)
                                    .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?,
                            )?;
                            let wants_to_unicode = resource.generate_to_unicode
                                && !resource.disable_builtin_to_unicode;
                            let to_unicode = wants_to_unicode
                                .then(|| object_id(next_object.saturating_add(2)))
                                .transpose()?;
                            next_object = next_object
                                .checked_add(if wants_to_unicode { 3 } else { 2 })
                                .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?;
                            PdfFontObjectIds {
                                font: font_id,
                                descriptor: Some(descriptor),
                                program: Some(program),
                                to_unicode,
                                encoding,
                                char_procs: BTreeMap::new(),
                            }
                        } else {
                            let mut char_procs = BTreeMap::new();
                            for &code in used_codes {
                                char_procs.insert(code, object_id(next_object)?);
                                next_object = next_object
                                    .checked_add(1)
                                    .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?;
                            }
                            PdfFontObjectIds {
                                font: font_id,
                                descriptor: None,
                                program: None,
                                to_unicode: None,
                                encoding: None,
                                char_procs,
                            }
                        };
                        let font_started = std::time::Instant::now();
                        objects.extend(pdf_font_objects(
                            width_resource,
                            ids,
                            font,
                            &resource_name,
                            used_codes,
                        )?);
                        font_embed_ns += font_started.elapsed().as_nanos();
                    }
                    let bytes =
                        run.units
                            .iter()
                            .map(|unit| match unit {
                                crate::positioned::TextUnit::Code(code) => u8::try_from(*code)
                                    .map_err(|_| PdfBuildError::PositionedCharacterOutOfRange {
                                        font: font.name.clone(),
                                        code: *code,
                                    }),
                                crate::positioned::TextUnit::Space => Ok(b' '),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                    operations.push(PdfContentOperation::Text(PdfContentTextRun {
                        x: scaled_to_bp_f32(run.x, parameters.decimal_digits),
                        exact_position: exact_text_position(
                            run.x,
                            total_height
                                .checked_sub(run.baseline)
                                .ok_or(PdfBuildError::PageGeometryOverflow)?,
                            parameters.decimal_digits,
                        ),
                        raster: None,
                        baseline: scaled_to_bp_f32(
                            total_height
                                .checked_sub(run.baseline)
                                .ok_or(PdfBuildError::PageGeometryOverflow)?,
                            parameters.decimal_digits,
                        ),
                        font_name: resource_name,
                        font_size: pdftex_font_size(font.at_size),
                        horizontal_scale: font_horizontal_scale(&font.construction),
                        advance: None,
                        bytes,
                    }));
                }
                PositionedEvent::Special(special) if special.class == "dvi" => {}
                PositionedEvent::Special(special) => {
                    return Err(PdfBuildError::UnsupportedSpecial(special.class));
                }
                PositionedEvent::Box(_)
                | PositionedEvent::BoxEnd(_)
                | PositionedEvent::PdfAccessibility(_)
                | PositionedEvent::PdfAnnotation(_)
                | PositionedEvent::PdfDestination(_)
                | PositionedEvent::PdfThread(_)
                | PositionedEvent::PdfEndThread { .. }
                | PositionedEvent::TextRun(_) => {}
            }
        }
        let mut dictionary = PdfDictionary::new();
        dictionary.insert("FormType", PdfValue::Integer(1))?;
        let mut resources = PdfDictionary::new();
        resources.set_raw_entries(form.resource_entries.clone());
        let omit_procset = input.document.form_omit_procset;
        if omit_procset < 0 || (omit_procset == 0 && parameters.major_version < 2) {
            form_procset.include_text(!form_fonts.is_empty());
            resources.insert("ProcSet", form_procset.into_pdf_array())?;
        }
        if !nested_forms.is_empty() || !form_images.is_empty() {
            let mut xobjects = PdfDictionary::new();
            for (resource, object) in nested_forms {
                xobjects.insert(
                    format!("Fm{resource}").as_str(),
                    PdfValue::Reference(object),
                )?;
            }
            for (name, object) in form_images {
                xobjects.insert(
                    std::str::from_utf8(&name).expect("generated image resource name is ASCII"),
                    PdfValue::Reference(object),
                )?;
            }
            resources.insert("XObject", PdfValue::Dictionary(xobjects))?;
        }
        if !form_fonts.is_empty() {
            let mut fonts = PdfDictionary::new();
            for (resource, object) in form_fonts {
                fonts.insert(format!("F{resource}").as_str(), PdfValue::Reference(object))?;
            }
            resources.insert("Font", PdfValue::Dictionary(fonts))?;
        }
        dictionary.insert("Resources", PdfValue::Dictionary(resources))?;
        dictionary.set_raw_entries(form.entries.clone());
        let zero = PdfNumber::new(0, 0)?;
        let one = PdfNumber::new(1, 0)?;
        objects.push(PdfIndirectObject {
            id: object_id(form.object())?,
            object: PdfObject::FormXObject {
                dictionary,
                data: ordered_page_content(&operations),
                bbox: [
                    zero,
                    zero,
                    scaled_to_bp_number(form.width(), parameters.decimal_digits)?,
                    scaled_to_bp_number(total_height, parameters.decimal_digits)?,
                ],
                matrix: Some([one, zero, zero, one, zero, zero]),
            },
        });
    }
    *next_object_ref = next_object;
    Ok(ContentOutput {
        kids,
        diagnostics,
        font_embed_ns,
    })
}
