//! Detached PDF assembly from checkpointed shipout receipts.

use md5::{Digest, Md5};
use tex_arith::Scaled;
use tex_expand::append_token_string_text;
use tex_out::PageNode;
use tex_out::pdf::{
    PdfAnnotationAction, PdfAnnotationObject, PdfAnnotationType, PdfBeadObject,
    PdfContentOperation, PdfContentRectangle, PdfContentTextRun, PdfDestinationAction,
    PdfDestinationActionKind, PdfDestinationNameTree, PdfDestinationNameTreeChildren,
    PdfDestinationPage, PdfDestinationStructure, PdfDestinationTarget, PdfDestinationView,
    PdfDictionary, PdfExplicitDestination, PdfImageColorSpace, PdfImageFilter, PdfImageXObject,
    PdfIndirectObject, PdfModelError, PdfName, PdfNamesObject, PdfNumber, PdfObject,
    PdfObjectCompression, PdfObjectId, PdfOutlineItemObject, PdfOutlineObject,
    PdfSerializationOptions, PdfSerializeError, PdfStreamCompression, PdfThreadObject, PdfTrailer,
    PdfValue, PdfVersion, UnvalidatedPdfDocument, ordered_page_content, page_content,
};
use tex_out::positioned::{
    BoxKind, PositionedBox, PositionedError, PositionedEvent, PositionedPage,
};
use tex_state::env::banks::{IntParam, TokParam};
use tex_state::ids::FontId;
use tex_state::ids::TokenListId;
use tex_state::{
    CommittedArtifact, ContentHash, PdfActionIdentifier, PdfActionSpec, PdfActionTarget,
    PdfActionWindow, PdfAnnotationDimensions, PdfDocumentFragmentKind, PdfExternalImageMetadata,
    PdfLinkRecord, PdfOutputParameters, PdfRasterColorSpace, PdfRasterFormat, Universe, WorldError,
};

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::{Read, Write};

pub(crate) const DEFAULT_PDF_PK_RESOLUTION: i32 = 600;

fn parse_pdf_matrix(payload: &[u8]) -> Result<[f32; 4], PdfBuildError> {
    let text =
        std::str::from_utf8(payload).map_err(|_| PdfBuildError::InvalidMatrix(payload.to_vec()))?;
    let mut values = text.split_ascii_whitespace();
    let mut matrix = [0.0; 4];
    for value in &mut matrix {
        *value = values
            .next()
            .and_then(|word| word.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .ok_or_else(|| PdfBuildError::InvalidMatrix(payload.to_vec()))?;
    }
    if values.next().is_some() {
        return Err(PdfBuildError::InvalidMatrix(payload.to_vec()));
    }
    Ok(matrix)
}

pub(crate) fn pk_font_request(
    stores: &Universe,
    font_id: FontId,
    driver_dpi: i32,
) -> Result<tex_fonts::PdfPkFontRequest, String> {
    let font = stores.font(font_id);
    let parameters = output_parameters(stores);
    let base_dpi = if parameters.pk_resolution == 0 {
        driver_dpi.clamp(72, 8_000)
    } else {
        parameters.pk_resolution
    };
    let design_size = i64::from(font.design_size().raw());
    if design_size <= 0 {
        return Err(format!("font {} has invalid PK design size", font.name()));
    }
    let scaled_dpi = i64::from(base_dpi)
        .checked_mul(i64::from(font.size().raw()))
        .and_then(|value| value.checked_add(design_size / 2))
        .map(|value| value / design_size)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("font {} PK resolution overflows", font.name()))?;
    let mode = stores
        .fixed_pdf_pk_mode()
        .unwrap_or_else(|| stores.tok_param(TokParam::PDF_PK_MODE));
    Ok(tex_fonts::PdfPkFontRequest::new(
        font.name().as_bytes().to_vec(),
        scaled_dpi,
        token_list_bytes(stores, mode),
    ))
}

/// Builds one deterministic PDF from the current checkpointed page ledger.
pub fn pdf_from_committed_artifacts(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
) -> Result<Vec<u8>, PdfBuildError> {
    pdf_from_committed_artifacts_with_virtual_fonts(
        stores,
        artifacts,
        &crate::PdfVirtualFontResources::default(),
    )
}

/// Builds one deterministic PDF with the accepted virtual-font resource closure.
pub fn pdf_from_committed_artifacts_with_virtual_fonts(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    virtual_fonts: &crate::PdfVirtualFontResources,
) -> Result<Vec<u8>, PdfBuildError> {
    pdf_from_committed_artifacts_at_dpi_with_virtual_fonts(
        stores,
        artifacts,
        DEFAULT_PDF_PK_RESOLUTION,
        virtual_fonts,
    )
}

/// Builds a PDF using an explicit host bitmap-device DPI when
/// `\pdfpkresolution` retains its zero sentinel.
pub fn pdf_from_committed_artifacts_at_dpi(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    driver_dpi: i32,
) -> Result<Vec<u8>, PdfBuildError> {
    pdf_from_committed_artifacts_at_dpi_with_virtual_fonts(
        stores,
        artifacts,
        driver_dpi,
        &crate::PdfVirtualFontResources::default(),
    )
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn pdf_from_committed_artifacts_at_dpi_with_virtual_fonts(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    driver_dpi: i32,
    virtual_fonts: &crate::PdfVirtualFontResources,
) -> Result<Vec<u8>, PdfBuildError> {
    let total_started = std::time::Instant::now();
    let parameters = output_parameters(stores);
    if parameters.output <= 0 {
        return Err(PdfBuildError::PdfOutputDisabled);
    }
    let version = pdf_version(parameters)?;
    let options = serialization_options(parameters)?;
    let page_records = stores.pdf_pages().to_vec();
    let map_started = std::time::Instant::now();
    let resolved_font_map = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .map(|entry| (entry.tex_name.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let map_resolve_ns = map_started.elapsed().as_nanos();
    let positioning_started = std::time::Instant::now();
    let mut positioned_pages = positioned_pages(stores, artifacts, &page_records)?;
    let page_count = positioned_pages.len();
    positioned_pages.extend(positioned_forms(stores)?);
    let positioning_ns = positioning_started.elapsed().as_nanos();
    let vf_started = std::time::Instant::now();
    crate::pdf_vf::lower_pages(
        stores,
        &mut positioned_pages,
        virtual_fonts,
        crate::pdf_vf::PdfVfLimits::default(),
    )?;
    let vf_ns = vf_started.elapsed().as_nanos();
    let positioned_forms = positioned_pages.split_off(page_count);
    let positioned_forms = stores
        .pdf_forms()
        .map(|form| form.object())
        .zip(positioned_forms)
        .collect::<BTreeMap<_, _>>();
    let font_usage_started = std::time::Instant::now();
    let font_usage = collect_font_usage(
        stores,
        &positioned_pages,
        &positioned_forms,
        &resolved_font_map,
    )?;
    let font_usage_ns = font_usage_started.elapsed().as_nanos();
    let destinations_started = std::time::Instant::now();
    let shipped_destinations = lower_page_destinations(
        stores,
        artifacts,
        &page_records,
        &positioned_pages,
        parameters.decimal_digits,
    )?;
    let destinations_ns = destinations_started.elapsed().as_nanos();
    let page_link_margins = page_records
        .iter()
        .map(|record| record.link_margin())
        .collect::<Vec<_>>();
    let annotations_started = std::time::Instant::now();
    let mut page_annotations =
        lower_page_annotations(stores, &positioned_pages, &page_link_margins)?;
    assign_annotation_objects(stores, &mut page_annotations)?;
    let annotations_ns = annotations_started.elapsed().as_nanos();
    let include_info = stores.int_param(IntParam::PDF_OMIT_INFO_DICT) == 0;
    let document_ids = stores
        .finalize_pdf_document_objects(include_info)
        .map_err(|_| PdfBuildError::ObjectCapacity)?;
    let catalog_id = object_id(
        document_ids
            .catalog()
            .expect("PDF finalization allocates the catalog"),
    )?;
    let pages_id = object_id(
        document_ids
            .pages()
            .expect("PDF finalization allocates the page tree"),
    )?;
    let mut next_object = stores.pdf_next_object_id();
    let outline_output = outline_objects(stores, &page_records, &mut next_object)?;
    let destination_output = destination_objects(
        stores,
        &page_records,
        shipped_destinations,
        &mut next_object,
    )?;
    let thread_output = thread_objects(
        stores.pdf_threads(),
        &positioned_pages,
        &page_records,
        parameters.decimal_digits,
        &mut next_object,
    )?;
    let mut objects =
        Vec::with_capacity(2 + page_records.len() * 3 + stores.pdf_raw_objects().len() + 2);
    let mut kids = Vec::with_capacity(page_records.len());
    let mut emitted_fonts = std::collections::BTreeSet::new();
    let mut interword_space_enabled = false;
    let mut fallback_space_font = None;
    let mut referenced_forms = BTreeSet::<u32>::new();
    let object_started = std::time::Instant::now();
    let mut font_embed_ns = 0_u128;
    referenced_forms.extend(
        stores
            .pdf_forms()
            .filter(|form| form.immediate())
            .map(|form| form.object()),
    );

    let mut catalog = PdfDictionary::new();
    catalog.insert("Type", PdfValue::Name("Catalog".into()))?;
    catalog.insert("Pages", PdfValue::Reference(pages_id))?;
    if let Some(names) = document_ids.names() {
        catalog.insert("Names", PdfValue::Reference(object_id(names)?))?;
    }
    if let Some(outlines) = outline_output.root {
        catalog.insert("Outlines", PdfValue::Reference(outlines))?;
    }
    if let Some(threads) = thread_output.list {
        catalog.insert("Threads", PdfValue::Reference(threads))?;
    }
    let open_action = stores.pdf_catalog_open_action();
    if let Some(action) = open_action {
        catalog.insert("OpenAction", PdfValue::Reference(object_id(action.id())?))?;
    }
    catalog.set_raw_entries(document_fragment_bytes(
        stores,
        PdfDocumentFragmentKind::Catalog,
    ));
    objects.push(indirect_dictionary(catalog_id, catalog));

    if let Some(action) = open_action {
        objects.push(PdfIndirectObject {
            id: object_id(action.id())?,
            object: PdfObject::Action(detached_link_action(stores, action.spec(), &page_records)?),
        });
    }

    if let Some(names) = document_ids.names() {
        objects.push(PdfIndirectObject {
            id: object_id(names)?,
            object: PdfObject::Names(PdfNamesObject {
                destinations: destination_output.name_tree_root,
                raw_entries: document_fragment_bytes(stores, PdfDocumentFragmentKind::Names),
            }),
        });
    }
    objects.extend(outline_output.objects);
    objects.extend(destination_output.destinations);
    objects.extend(destination_output.name_tree);
    objects.extend(thread_output.objects.clone());

    if let Some(info) = document_ids.info() {
        let mut dictionary = document_info_dictionary(stores, parameters)?;
        dictionary.set_raw_entries(document_fragment_bytes(
            stores,
            PdfDocumentFragmentKind::Info,
        ));
        objects.push(indirect_dictionary(object_id(info)?, dictionary));
    }

    let raw_records = stores.pdf_raw_objects().to_vec();
    for record in raw_records {
        if !record.is_immediate() && !record.is_referenced() {
            continue;
        }
        let data = record
            .data()
            .ok_or(PdfBuildError::ReferencedRawObjectUninitialized(
                record.id().raw(),
            ))?;
        let payload = token_list_bytes(stores, data.data());
        let object = if data.is_stream() {
            let mut dictionary = PdfDictionary::new();
            if let Some(attr) = data.stream_attr() {
                dictionary.set_raw_entries(token_list_bytes(stores, attr));
            }
            let stream_data = if data.is_file() {
                let name = std::str::from_utf8(&payload)
                    .map_err(|_| PdfBuildError::InvalidRawObjectFileName(record.id().raw()))?;
                stores.world_mut().read_file(name)?.bytes().to_vec()
            } else {
                payload
            };
            PdfObject::Stream {
                dictionary,
                data: stream_data,
            }
        } else {
            PdfObject::Raw(payload)
        };
        objects.push(PdfIndirectObject {
            id: object_id(record.id().raw())?,
            object,
        });
    }

    let mut pdf_image_groups = BTreeMap::<u32, Option<PdfObjectId>>::new();
    let mut pdf_image_objects = BTreeMap::<u32, PdfObjectId>::new();
    let mut lowered_images = HashMap::<(ContentHash, PdfExternalImageMetadata), PdfObjectId>::new();
    let image_import_started = std::time::Instant::now();
    let mut image_telemetry = ImageImportTelemetry::default();
    let mut image_count = 0usize;
    let mut raster_image_count = 0usize;
    let mut pdf_image_count = 0usize;
    let mut image_input_bytes = 0usize;
    let mut unique_image_identities = BTreeSet::new();
    for image in stores.pdf_external_images() {
        image_count += 1;
        image_input_bytes = image_input_bytes.saturating_add(image.bytes().len());
        unique_image_identities.insert(image.identity());
        let cache_key = (image.identity(), image.metadata());
        if matches!(image.metadata(), PdfExternalImageMetadata::Raster(_))
            && let Some(&object) = lowered_images.get(&cache_key)
        {
            image_telemetry.cache_hits += 1;
            pdf_image_objects.insert(image.id().raw(), object);
            continue;
        }
        match image.metadata() {
            PdfExternalImageMetadata::Raster(metadata) => {
                raster_image_count += 1;
                let (color_data, filter, bits, color_space, alpha_data) = raster_image_streams(
                    image.bytes(),
                    metadata,
                    parameters,
                    &mut image_telemetry,
                )?;
                let image_object = object_id(image.id().raw())?;
                objects.push(PdfIndirectObject {
                    id: image_object,
                    object: PdfObject::ImageXObject {
                        image: PdfImageXObject {
                            width: metadata.width,
                            height: metadata.height,
                            bits_per_component: bits,
                            color_space,
                            filter,
                            soft_mask: image.mask_object().map(object_id).transpose()?,
                        },
                        data: color_data,
                    },
                });
                if let Some((alpha_data, alpha_filter)) = alpha_data {
                    let mask = image.mask_object().ok_or(PdfBuildError::InvalidPng)?;
                    objects.push(PdfIndirectObject {
                        id: object_id(mask)?,
                        object: PdfObject::ImageXObject {
                            image: PdfImageXObject {
                                width: metadata.width,
                                height: metadata.height,
                                bits_per_component: if metadata.png_color_type == Some(3) {
                                    8
                                } else {
                                    metadata.bits_per_component
                                },
                                color_space: PdfImageColorSpace::DeviceGray,
                                filter: alpha_filter,
                                soft_mask: None,
                            },
                            data: alpha_data,
                        },
                    });
                }
                pdf_image_objects.insert(image.id().raw(), image_object);
                lowered_images.insert(cache_key, image_object);
            }
            PdfExternalImageMetadata::PdfPage {
                page_box,
                rotation,
                page,
                ..
            } => {
                pdf_image_count += 1;
                let imported = import_pdf_page(image, page, page_box, rotation, &mut next_object)?;
                let image_object = imported.form.id;
                pdf_image_groups.insert(image.id().raw(), imported.group);
                pdf_image_objects.insert(image.id().raw(), image_object);
                objects.extend(imported.dependencies);
                objects.push(imported.form);
            }
        }
    }
    let image_import_ns = image_import_started.elapsed().as_nanos();

    for (page_index, record) in page_records.iter().copied().enumerate() {
        let bytes = artifact_bytes(stores, artifacts, record.artifact())?;
        let artifact = tex_out::PageArtifact::from_bytes(&bytes)?;
        let positioned = positioned_pages[page_index].clone();
        let (page_width, page_height) = pdf_page_extents(&artifact, record)?;
        let mut content_operations = Vec::new();
        let mut page_forms = BTreeMap::<u32, PdfObjectId>::new();
        let mut page_images = BTreeMap::<Vec<u8>, PdfObjectId>::new();
        let mut page_group_selector = stores.pdf_page_group_selector();
        let mut page_group = None;
        let mut has_pdf_graphics = false;
        let mut page_fonts = std::collections::BTreeMap::new();
        let mut fallback_space_on_page = false;
        for event in positioned.events {
            match event {
                PositionedEvent::Rule(rule) => {
                    content_operations.push(PdfContentOperation::Rectangle(PdfContentRectangle {
                        x: scaled_to_bp_f32(
                            rule.x
                                .checked_add(record.h_origin())
                                .ok_or(PdfBuildError::PageGeometryOverflow)?,
                            parameters.decimal_digits,
                        ),
                        y: scaled_to_bp_f32(
                            page_height
                                .checked_sub(rule.y)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_sub(rule.height))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?,
                            parameters.decimal_digits,
                        ),
                        width: scaled_to_bp_f32(rule.width, parameters.decimal_digits),
                        height: scaled_to_bp_f32(rule.height, parameters.decimal_digits),
                    }))
                }
                PositionedEvent::TextRun(run) if !run.units.is_empty() => {
                    let font = positioned
                        .fonts
                        .iter()
                        .find(|font| font.font_id == run.font_id)
                        .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
                    let resource = stores
                        .pdf_font_resource_by_identity(font.semantic_identity)
                        .ok_or(PdfBuildError::MissingFontResource(font.name.clone()))?;
                    let resource_name = format!("F{}", resource.resource_number()).into_bytes();
                    let font_id = match page_fonts.get(&resource.resource_number()).copied() {
                        Some(id) => id,
                        None => {
                            let id = object_id(resource.object_number())?;
                            page_fonts.insert(resource.resource_number(), id);
                            if emitted_fonts.insert(resource.object_number()) {
                                let live_font = stores
                                    .font_by_source_identity(font.semantic_identity)
                                    .ok_or_else(|| {
                                        PdfBuildError::MissingLiveFont(font.name.clone())
                                    })?;
                                let used_codes =
                                    font_usage.get(&resource.object_number()).ok_or_else(|| {
                                        PdfBuildError::MissingFontUsage(font.name.clone())
                                    })?;
                                let mapped = resolved_font_map.contains_key(font.name.as_bytes());
                                let ids = if mapped {
                                    let descriptor = object_id(next_object)?;
                                    let program = object_id(
                                        next_object
                                            .checked_add(1)
                                            .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?,
                                    )?;
                                    let wants_to_unicode =
                                        stores.pdf_font_configuration().generates_to_unicode()
                                            && !stores.pdf_builtin_to_unicode_disabled(live_font);
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
                                        char_procs,
                                    }
                                };
                                let font_started = std::time::Instant::now();
                                objects.extend(pdf_font_objects(
                                    stores,
                                    ids,
                                    font,
                                    &resource_name,
                                    used_codes,
                                    driver_dpi,
                                    &resolved_font_map,
                                )?);
                                font_embed_ns += font_started.elapsed().as_nanos();
                            }
                            id
                        }
                    };
                    debug_assert_eq!(page_fonts.get(&resource.resource_number()), Some(&font_id));
                    debug_assert_eq!(run.units.len(), run.positions.len());
                    debug_assert_eq!(run.units.len(), run.physical_codes.len());
                    let baseline = scaled_to_bp_f32(
                        page_height
                            .checked_sub(run.baseline)
                            .and_then(|value| value.checked_sub(record.v_origin()))
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        parameters.decimal_digits,
                    );
                    let font_size = scaled_to_bp_f32(font.at_size, parameters.decimal_digits);
                    let explicit_space =
                        font_has_explicit_space(stores, &resolved_font_map, font.name.as_bytes());
                    let mut segment = Vec::new();
                    let mut segment_x = None;
                    for ((unit, position), physical_code) in run
                        .units
                        .iter()
                        .zip(&run.positions)
                        .zip(&run.physical_codes)
                    {
                        match unit {
                            tex_out::positioned::TextUnit::Code(_) => {
                                if let Some(code) = physical_code {
                                    segment_x.get_or_insert(*position);
                                    segment.push(*code);
                                }
                            }
                            tex_out::positioned::TextUnit::Space => {
                                if !segment.is_empty() {
                                    content_operations.push(PdfContentOperation::Text(
                                        PdfContentTextRun {
                                            x: scaled_to_bp_f32(
                                                segment_x
                                                    .take()
                                                    .expect("nonempty segment has an anchor")
                                                    .checked_add(record.h_origin())
                                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                                parameters.decimal_digits,
                                            ),
                                            baseline,
                                            font_name: resource_name.clone(),
                                            font_size,
                                            bytes: std::mem::take(&mut segment),
                                        },
                                    ));
                                }
                                if interword_space_enabled {
                                    let (font_name, space_size) = if explicit_space {
                                        (resource_name.clone(), font_size)
                                    } else {
                                        ensure_fallback_space_font(
                                            stores,
                                            record.space_font_name_id(),
                                            &mut next_object,
                                            &mut objects,
                                            &mut fallback_space_font,
                                        )?;
                                        fallback_space_on_page = true;
                                        (b"UmberSpace".to_vec(), 10.0)
                                    };
                                    content_operations.push(PdfContentOperation::Text(
                                        PdfContentTextRun {
                                            x: scaled_to_bp_f32(
                                                position
                                                    .checked_add(record.h_origin())
                                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                                parameters.decimal_digits,
                                            ),
                                            baseline,
                                            font_name,
                                            font_size: space_size,
                                            bytes: vec![b' '],
                                        },
                                    ));
                                }
                            }
                        }
                    }
                    if !segment.is_empty() {
                        content_operations.push(PdfContentOperation::Text(PdfContentTextRun {
                            x: scaled_to_bp_f32(
                                segment_x
                                    .expect("nonempty segment has an anchor")
                                    .checked_add(record.h_origin())
                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                parameters.decimal_digits,
                            ),
                            baseline,
                            font_name: resource_name,
                            font_size,
                            bytes: segment,
                        }));
                    }
                }
                PositionedEvent::PdfAccessibility(control) => match control.control {
                    tex_out::PdfAccessibilityEffect::InterwordSpaceOn => {
                        interword_space_enabled = true;
                    }
                    tex_out::PdfAccessibilityEffect::InterwordSpaceOff => {
                        interword_space_enabled = false;
                    }
                    tex_out::PdfAccessibilityEffect::FakeSpace => {
                        ensure_fallback_space_font(
                            stores,
                            record.space_font_name_id(),
                            &mut next_object,
                            &mut objects,
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
                            baseline: scaled_to_bp_f32(
                                page_height
                                    .checked_sub(control.y)
                                    .and_then(|value| value.checked_sub(record.v_origin()))
                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                parameters.decimal_digits,
                            ),
                            font_name: b"UmberSpace".to_vec(),
                            font_size: 10.0,
                            bytes: vec![b' '],
                        }));
                    }
                },
                PositionedEvent::PdfAnnotation(_) => {}
                PositionedEvent::Special(special) => {
                    return Err(PdfBuildError::UnsupportedSpecial(special.class));
                }
                PositionedEvent::PdfGraphics(graphics) => {
                    has_pdf_graphics = true;
                    let x = scaled_to_bp_f32(
                        graphics
                            .x
                            .checked_add(record.h_origin())
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        parameters.decimal_digits,
                    );
                    let y = scaled_to_bp_f32(
                        page_height
                            .checked_sub(graphics.y)
                            .and_then(|value| value.checked_sub(record.v_origin()))
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        parameters.decimal_digits,
                    );
                    let operation = match graphics.effect {
                        tex_out::PageEffect::PdfLiteral { mode, payload } => {
                            PdfContentOperation::Literal {
                                mode,
                                x,
                                y,
                                bytes: payload,
                            }
                        }
                        tex_out::PageEffect::PdfSetMatrix { payload } => {
                            PdfContentOperation::SetMatrix {
                                x,
                                y,
                                matrix: parse_pdf_matrix(&payload)?,
                            }
                        }
                        tex_out::PageEffect::PdfSave => PdfContentOperation::Save { x, y },
                        tex_out::PageEffect::PdfRestore => PdfContentOperation::Restore { x, y },
                        tex_out::PageEffect::PdfColorStack { mode, payload, .. } => {
                            PdfContentOperation::ColorStack {
                                mode,
                                x,
                                y,
                                bytes: payload,
                            }
                        }
                        tex_out::PageEffect::PdfRefXForm { object, .. } => {
                            let form = stores
                                .pdf_form(object)
                                .ok_or(PdfBuildError::ReferencedFormNotFound(object))?;
                            let y = page_height
                                .checked_sub(graphics.y)
                                .and_then(|value| value.checked_sub(form.depth()))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let form_id = object_id(form.object())?;
                            referenced_forms.insert(form.object());
                            page_forms.insert(form.resource(), form_id);
                            PdfContentOperation::FormXObject {
                                x,
                                y: scaled_to_bp_f32(y, parameters.decimal_digits),
                                name: format!("Fm{}", form.resource()).into_bytes(),
                            }
                        }
                        tex_out::PageEffect::PdfRefXImage {
                            object,
                            width,
                            height,
                            depth,
                        } => {
                            let id = tex_state::PdfExternalImageId::new(object)
                                .map_err(|_| PdfBuildError::MissingRasterImage(object))?;
                            let image = stores
                                .pdf_external_image_record(id)
                                .ok_or(PdfBuildError::MissingRasterImage(object))?;
                            if matches!(image.metadata(), PdfExternalImageMetadata::PdfPage { .. })
                            {
                                let group = pdf_image_groups.get(&object).copied().flatten();
                                match page_group_selector.include(group.is_some()) {
                                    tex_state::PdfPageGroupInclusion::None => {}
                                    tex_state::PdfPageGroupInclusion::SelectForOutputPage => {
                                        page_group = group;
                                    }
                                    tex_state::PdfPageGroupInclusion::KeepOnIncludedForm {
                                        warning,
                                    } => {
                                        if let Some(warning) = warning {
                                            stores.world_mut().write_text(
                                                tex_state::PrintSink::TerminalAndLog,
                                                &format!("{}\n", warning.message()),
                                            );
                                        }
                                    }
                                }
                            }
                            let name = image_resource_name(&image, parameters);
                            let image_object = pdf_image_objects
                                .get(&object)
                                .copied()
                                .ok_or(PdfBuildError::MissingRasterImage(object))?;
                            page_images.insert(name.clone(), image_object);
                            let total_height = height
                                .checked_add(depth)
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let y = page_height
                                .checked_sub(graphics.y)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_sub(depth))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let (placed_width, placed_height) = match image.metadata() {
                                PdfExternalImageMetadata::PdfPage {
                                    page_box, rotation, ..
                                } => {
                                    let box_width = page_box
                                        .right
                                        .checked_sub(page_box.left)
                                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                                    let box_height = page_box
                                        .top
                                        .checked_sub(page_box.bottom)
                                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                                    let (natural_width, natural_height) = if rotation.swaps_axes() {
                                        (box_height, box_width)
                                    } else {
                                        (box_width, box_height)
                                    };
                                    (
                                        scaled_to_bp_f32(width, parameters.decimal_digits)
                                            / scaled_to_bp_f32(
                                                natural_width,
                                                parameters.decimal_digits,
                                            ),
                                        scaled_to_bp_f32(total_height, parameters.decimal_digits)
                                            / scaled_to_bp_f32(
                                                natural_height,
                                                parameters.decimal_digits,
                                            ),
                                    )
                                }
                                PdfExternalImageMetadata::Raster(_) => (
                                    scaled_to_bp_f32(width, parameters.decimal_digits),
                                    scaled_to_bp_f32(total_height, parameters.decimal_digits),
                                ),
                            };
                            PdfContentOperation::ImageXObject {
                                x,
                                y: scaled_to_bp_f32(y, parameters.decimal_digits),
                                width: placed_width,
                                height: placed_height,
                                name,
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
            resources.insert(
                "ProcSet",
                PdfValue::Array(vec![PdfValue::Name("PDF".into())]),
            )?;
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
        resources.set_raw_entries(token_list_bytes(stores, record.resources()));
        objects.push(indirect_dictionary(resources_id, resources));
        objects.push(PdfIndirectObject {
            id: contents_id,
            object: PdfObject::Stream {
                dictionary: PdfDictionary::new(),
                data: if has_pdf_graphics {
                    ordered_page_content(&content_operations)
                } else {
                    let rectangles = content_operations
                        .iter()
                        .filter_map(|operation| match operation {
                            PdfContentOperation::Rectangle(rectangle) => Some(*rectangle),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let text_runs = content_operations
                        .iter()
                        .filter_map(|operation| match operation {
                            PdfContentOperation::Text(run) => Some(run.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    page_content(&rectangles, &text_runs)
                },
            },
        });

        let mut page = PdfDictionary::new();
        page.insert("Type", PdfValue::Name("Page".into()))?;
        page.insert("Parent", PdfValue::Reference(pages_id))?;
        let page_attr = token_list_bytes(stores, record.page_attr());
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
                stores,
                *annotation,
                record,
                page_height,
                &page_records,
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
        let form = stores
            .pdf_form(object)
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
        let mut form_fonts = BTreeMap::<u32, PdfObjectId>::new();
        for event in positioned.events {
            match event {
                PositionedEvent::Rule(rule) => {
                    operations.push(PdfContentOperation::Rectangle(PdfContentRectangle {
                        x: scaled_to_bp_f32(rule.x, parameters.decimal_digits),
                        y: scaled_to_bp_f32(
                            total_height
                                .checked_sub(rule.y)
                                .and_then(|value| value.checked_sub(rule.height))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?,
                            parameters.decimal_digits,
                        ),
                        width: scaled_to_bp_f32(rule.width, parameters.decimal_digits),
                        height: scaled_to_bp_f32(rule.height, parameters.decimal_digits),
                    }))
                }
                PositionedEvent::PdfGraphics(graphics) => {
                    let x = scaled_to_bp_f32(graphics.x, parameters.decimal_digits);
                    let y = scaled_to_bp_f32(
                        total_height
                            .checked_sub(graphics.y)
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        parameters.decimal_digits,
                    );
                    let operation = match graphics.effect {
                        tex_out::PageEffect::PdfLiteral { mode, payload } => {
                            PdfContentOperation::Literal {
                                mode,
                                x,
                                y,
                                bytes: payload,
                            }
                        }
                        tex_out::PageEffect::PdfSetMatrix { payload } => {
                            PdfContentOperation::SetMatrix {
                                x,
                                y,
                                matrix: parse_pdf_matrix(&payload)?,
                            }
                        }
                        tex_out::PageEffect::PdfSave => PdfContentOperation::Save { x, y },
                        tex_out::PageEffect::PdfRestore => PdfContentOperation::Restore { x, y },
                        tex_out::PageEffect::PdfColorStack { mode, payload, .. } => {
                            PdfContentOperation::ColorStack {
                                mode,
                                x,
                                y,
                                bytes: payload,
                            }
                        }
                        tex_out::PageEffect::PdfRefXForm { object, .. } => {
                            let nested = stores
                                .pdf_form(object)
                                .ok_or(PdfBuildError::ReferencedFormNotFound(object))?;
                            if object == form.object() {
                                return Err(PdfBuildError::RecursiveForm(object));
                            }
                            nested_forms.insert(nested.resource(), object_id(object)?);
                            pending_forms.push_back(object);
                            let y = total_height
                                .checked_sub(graphics.y)
                                .and_then(|value| value.checked_sub(nested.depth()))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            PdfContentOperation::FormXObject {
                                x,
                                y: scaled_to_bp_f32(y, parameters.decimal_digits),
                                name: format!("Fm{}", nested.resource()).into_bytes(),
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
                    let resource = stores
                        .pdf_font_resource_by_identity(font.semantic_identity)
                        .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
                    let resource_name = format!("F{}", resource.resource_number()).into_bytes();
                    let font_id = object_id(resource.object_number())?;
                    form_fonts.insert(resource.resource_number(), font_id);
                    if emitted_fonts.insert(resource.object_number()) {
                        let live_font = stores
                            .font_by_source_identity(font.semantic_identity)
                            .ok_or_else(|| PdfBuildError::MissingLiveFont(font.name.clone()))?;
                        let used_codes = font_usage
                            .get(&resource.object_number())
                            .ok_or_else(|| PdfBuildError::MissingFontUsage(font.name.clone()))?;
                        let mapped = resolved_font_map.contains_key(font.name.as_bytes());
                        let ids = if mapped {
                            let descriptor = object_id(next_object)?;
                            let program = object_id(
                                next_object
                                    .checked_add(1)
                                    .ok_or(PdfBuildError::InvalidObjectId(u32::MAX))?,
                            )?;
                            let wants_to_unicode =
                                stores.pdf_font_configuration().generates_to_unicode()
                                    && !stores.pdf_builtin_to_unicode_disabled(live_font);
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
                                char_procs,
                            }
                        };
                        let font_started = std::time::Instant::now();
                        objects.extend(pdf_font_objects(
                            stores,
                            ids,
                            font,
                            &resource_name,
                            used_codes,
                            driver_dpi,
                            &resolved_font_map,
                        )?);
                        font_embed_ns += font_started.elapsed().as_nanos();
                    }
                    let bytes = run
                        .units
                        .iter()
                        .map(|unit| match unit {
                            tex_out::positioned::TextUnit::Code(code) => *code,
                            tex_out::positioned::TextUnit::Space => b' ',
                        })
                        .collect();
                    operations.push(PdfContentOperation::Text(PdfContentTextRun {
                        x: scaled_to_bp_f32(run.x, parameters.decimal_digits),
                        baseline: scaled_to_bp_f32(
                            total_height
                                .checked_sub(run.baseline)
                                .ok_or(PdfBuildError::PageGeometryOverflow)?,
                            parameters.decimal_digits,
                        ),
                        font_name: resource_name,
                        font_size: scaled_to_bp_f32(font.at_size, parameters.decimal_digits),
                        bytes,
                    }));
                }
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
        if let Some(tokens) = form.resources() {
            resources.set_raw_entries(token_list_bytes(stores, tokens));
        }
        let omit_procset = stores.int_param(IntParam::PDF_OMIT_PROCSET);
        if omit_procset < 0 || (omit_procset == 0 && parameters.major_version < 2) {
            resources.insert(
                "ProcSet",
                PdfValue::Array(vec![PdfValue::Name("PDF".into())]),
            )?;
        }
        if !nested_forms.is_empty() {
            let mut xobjects = PdfDictionary::new();
            for (resource, object) in nested_forms {
                xobjects.insert(
                    format!("Fm{resource}").as_str(),
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
        if let Some(tokens) = form.attr() {
            dictionary.set_raw_entries(token_list_bytes(stores, tokens));
        }
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

    let mut pages = PdfDictionary::new();
    pages.insert("Type", PdfValue::Name("Pages".into()))?;
    pages.insert("Count", PdfValue::Integer(page_records.len() as i64))?;
    pages.insert("Kids", PdfValue::Array(kids))?;
    pages.set_raw_entries(token_list_bytes(
        stores,
        stores.tok_param(TokParam::PDF_PAGES_ATTR),
    ));
    objects.push(indirect_dictionary(pages_id, pages));

    let trailer_id = document_fragment_bytes(stores, PdfDocumentFragmentKind::TrailerId);
    let file_id = if trailer_id.is_empty() {
        None
    } else {
        let digest = Md5::digest(&trailer_id).to_vec();
        Some((digest.clone(), digest))
    };

    let object_ns = object_started.elapsed().as_nanos();
    let object_count = objects.len();
    let validation_started = std::time::Instant::now();
    let document = UnvalidatedPdfDocument {
        version,
        catalog: catalog_id,
        objects,
        trailer: PdfTrailer {
            info: document_ids.info().map(object_id).transpose()?,
            file_id,
            raw_entries: document_fragment_bytes(stores, PdfDocumentFragmentKind::Trailer),
        },
    }
    .validate()?;
    let validation_ns = validation_started.elapsed().as_nanos();
    let serialization_started = std::time::Instant::now();
    let bytes = document.to_pdf_bytes_with_options(options)?;
    if std::env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
        eprintln!(
            "PDF_TELEMETRY map_resolve_ns={} positioning_ns={} vf_ns={} font_usage_ns={} destinations_ns={} annotations_ns={} object_ns={} image_import_ns={} image_parse_copy_ns={} image_decode_ns={} image_transform_ns={} image_encode_ns={} image_cache_hits={} font_embed_ns={} validation_ns={} serialization_ns={} total_ns={} pages={} forms={} fonts={} images={} raster_images={} pdf_images={} image_input_bytes={} unique_images={} lowered_images={} objects={} output_bytes={}",
            map_resolve_ns,
            positioning_ns,
            vf_ns,
            font_usage_ns,
            destinations_ns,
            annotations_ns,
            object_ns,
            image_import_ns,
            image_telemetry.parse_copy_ns,
            image_telemetry.decode_ns,
            image_telemetry.transform_ns,
            image_telemetry.encode_ns,
            image_telemetry.cache_hits,
            font_embed_ns,
            validation_ns,
            serialization_started.elapsed().as_nanos(),
            total_started.elapsed().as_nanos(),
            page_count,
            positioned_forms.len(),
            font_usage.len(),
            image_count,
            raster_image_count,
            pdf_image_count,
            image_input_bytes,
            unique_image_identities.len(),
            image_count.saturating_sub(image_telemetry.cache_hits),
            object_count,
            bytes.len()
        );
    }
    Ok(bytes)
}

fn collect_font_usage(
    stores: &Universe,
    positioned_pages: &[PositionedPage],
    positioned_forms: &BTreeMap<u32, PositionedPage>,
    resolved_font_map: &BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>,
) -> Result<BTreeMap<u32, BTreeSet<u8>>, PdfBuildError> {
    let mut font_metadata = BTreeMap::new();
    for font in positioned_pages
        .iter()
        .chain(positioned_forms.values())
        .flat_map(|positioned| &positioned.fonts)
    {
        if font_metadata.contains_key(&font.semantic_identity) {
            continue;
        }
        let resource = stores
            .pdf_font_resource_by_identity(font.semantic_identity)
            .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
        let live_font = stores
            .font_by_source_identity(font.semantic_identity)
            .ok_or_else(|| PdfBuildError::MissingLiveFont(font.name.clone()))?;
        font_metadata.insert(
            font.semantic_identity,
            (
                resource,
                stores.included_pdf_font_chars(live_font),
                font_has_explicit_space(stores, resolved_font_map, font.name.as_bytes()),
            ),
        );
    }
    let mut usage = BTreeMap::<u32, BTreeSet<u8>>::new();
    let mut interword_space_enabled = false;
    for positioned in positioned_pages {
        let fonts = positioned
            .fonts
            .iter()
            .map(|font| (font.font_id, font))
            .collect::<BTreeMap<_, _>>();
        for event in &positioned.events {
            let PositionedEvent::TextRun(run) = event else {
                if let PositionedEvent::PdfAccessibility(control) = event {
                    match control.control {
                        tex_out::PdfAccessibilityEffect::InterwordSpaceOn => {
                            interword_space_enabled = true;
                        }
                        tex_out::PdfAccessibilityEffect::InterwordSpaceOff => {
                            interword_space_enabled = false;
                        }
                        tex_out::PdfAccessibilityEffect::FakeSpace => {}
                    }
                }
                continue;
            };
            let font = fonts
                .get(&run.font_id)
                .copied()
                .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
            let (resource, included, has_explicit_space) = font_metadata
                .get(&font.semantic_identity)
                .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
            let codes = usage.entry(resource.object_number()).or_default();
            let explicit_space = interword_space_enabled && *has_explicit_space;
            codes.extend(run.units.iter().zip(&run.physical_codes).filter_map(
                |(unit, physical_code)| match unit {
                    tex_out::positioned::TextUnit::Code(_) => *physical_code,
                    tex_out::positioned::TextUnit::Space if explicit_space => Some(b' '),
                    tex_out::positioned::TextUnit::Space => None,
                },
            ));
            codes.extend(included);
        }
    }
    for positioned in positioned_forms.values() {
        let fonts = positioned
            .fonts
            .iter()
            .map(|font| (font.font_id, font))
            .collect::<BTreeMap<_, _>>();
        for event in &positioned.events {
            let PositionedEvent::TextRun(run) = event else {
                continue;
            };
            let font = fonts
                .get(&run.font_id)
                .copied()
                .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
            let (resource, included, _) = font_metadata
                .get(&font.semantic_identity)
                .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
            let codes = usage.entry(resource.object_number()).or_default();
            codes.extend(run.units.iter().map(|unit| match unit {
                tex_out::positioned::TextUnit::Code(code) => *code,
                tex_out::positioned::TextUnit::Space => b' ',
            }));
            codes.extend(included);
        }
    }
    Ok(usage)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShippedAnnotation {
    source_object: u32,
    object: u32,
    kind: ShippedAnnotationKind,
    rect: ShippedAnnotationRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShippedAnnotationKind {
    Annotation,
    Link,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShippedAnnotationRect {
    left: Scaled,
    top: Scaled,
    right: Scaled,
    bottom: Scaled,
}

#[derive(Clone, Copy, Debug)]
struct ActiveShippedLink {
    record: PdfLinkRecord,
    depth: u32,
    candidate: Option<(u32, Scaled)>,
}

fn positioned_pages(
    stores: &Universe,
    artifacts: &[CommittedArtifact],
    records: &[tex_state::PdfPageRecord],
) -> Result<Vec<PositionedPage>, PdfBuildError> {
    records
        .iter()
        .copied()
        .enumerate()
        .map(|(page_index, record)| {
            let bytes = artifact_bytes(stores, artifacts, record.artifact())?;
            let artifact = tex_out::PageArtifact::from_bytes(&bytes)?;
            Ok(tex_out::positioned::lower_page(
                &artifact,
                page_index as u32,
            )?)
        })
        .collect()
}

fn positioned_forms(stores: &Universe) -> Result<Vec<PositionedPage>, PdfBuildError> {
    stores
        .pdf_forms()
        .map(|form| {
            let staged = stores
                .pdf_form_artifact(form.object())
                .ok_or(PdfBuildError::MissingFormArtifact(form.object()))?;
            let artifact = tex_out::PageArtifact::from_bytes(staged.bytes())?;
            Ok(tex_out::positioned::lower_page(&artifact, 0)?)
        })
        .collect()
}

#[derive(Clone, Debug)]
struct ShippedDestination {
    object: u32,
    target: PdfObjectId,
    view: PdfDestinationView,
}

struct OutlineObjects {
    objects: Vec<PdfIndirectObject>,
    root: Option<PdfObjectId>,
}

fn outline_objects(
    stores: &Universe,
    pages: &[tex_state::PdfPageRecord],
    next_object: &mut u32,
) -> Result<OutlineObjects, PdfBuildError> {
    let records = stores.pdf_outlines();
    if records.is_empty() {
        return Ok(OutlineObjects {
            objects: Vec::new(),
            root: None,
        });
    }
    let root = object_id(*next_object)?;
    *next_object = next_object
        .checked_add(1)
        .ok_or(PdfBuildError::ObjectCapacity)?;
    let mut parents = vec![None; records.len()];
    let mut children = vec![Vec::new(); records.len()];
    let mut roots = Vec::new();
    let mut stack = Vec::<(usize, usize)>::new();
    for (index, record) in records.iter().enumerate() {
        while stack.last().is_some_and(|(_, remaining)| *remaining == 0) {
            stack.pop();
        }
        if let Some((parent, remaining)) = stack.last_mut() {
            parents[index] = Some(*parent);
            children[*parent].push(index);
            *remaining -= 1;
        } else {
            roots.push(index);
        }
        if record.count() != 0 {
            stack.push((index, record.count().unsigned_abs() as usize));
        }
    }
    let descendants = (0..records.len())
        .map(|index| outline_descendants(index, &children))
        .collect::<Vec<_>>();
    let visible_count: usize = roots
        .iter()
        .map(|&index| outline_visible(index, records, &children))
        .sum();
    let mut previous = vec![None; records.len()];
    let mut next = vec![None; records.len()];
    for siblings in std::iter::once(&roots).chain(children.iter()) {
        for pair in siblings.windows(2) {
            next[pair[0]] = Some(pair[1]);
            previous[pair[1]] = Some(pair[0]);
        }
    }
    let mut objects = Vec::with_capacity(records.len() * 3 + 1);
    for (index, record) in records.iter().copied().enumerate() {
        objects.push(PdfIndirectObject {
            id: object_id(record.action_object())?,
            object: PdfObject::Action(detached_link_action(stores, record.action(), pages)?),
        });
        objects.push(PdfIndirectObject {
            id: object_id(record.title_object())?,
            object: PdfObject::PdfStringSyntax(token_list_bytes(stores, record.title())),
        });
        let child_ids =
            if let Some((&first, &last)) = children[index].first().zip(children[index].last()) {
                Some((
                    object_id(records[first].item_object())?,
                    object_id(records[last].item_object())?,
                ))
            } else {
                None
            };
        let signed_count = (!children[index].is_empty()).then(|| {
            let count = i32::try_from(descendants[index]).unwrap_or(i32::MAX);
            if record.count() < 0 { -count } else { count }
        });
        objects.push(PdfIndirectObject {
            id: object_id(record.item_object())?,
            object: PdfObject::OutlineItem(PdfOutlineItemObject {
                title: object_id(record.title_object())?,
                action: object_id(record.action_object())?,
                parent: parents[index]
                    .map_or(Ok(root), |parent| object_id(records[parent].item_object()))?,
                previous: previous[index]
                    .map(|sibling| object_id(records[sibling].item_object()))
                    .transpose()?,
                next: next[index]
                    .map(|sibling| object_id(records[sibling].item_object()))
                    .transpose()?,
                first: child_ids.map(|ids| ids.0),
                last: child_ids.map(|ids| ids.1),
                count: signed_count,
                raw_entries: token_list_bytes(stores, record.attributes()),
            }),
        });
    }
    objects.push(PdfIndirectObject {
        id: root,
        object: PdfObject::Outline(PdfOutlineObject {
            first: object_id(records[*roots.first().expect("outline has root")].item_object())?,
            last: object_id(records[*roots.last().expect("outline has root")].item_object())?,
            visible_count: i32::try_from(visible_count).unwrap_or(i32::MAX),
        }),
    });
    Ok(OutlineObjects {
        objects,
        root: Some(root),
    })
}

fn outline_descendants(index: usize, children: &[Vec<usize>]) -> usize {
    children[index]
        .iter()
        .map(|&child| 1 + outline_descendants(child, children))
        .sum()
}

fn outline_visible(
    index: usize,
    records: &[tex_state::PdfOutlineRecord],
    children: &[Vec<usize>],
) -> usize {
    1 + if records[index].count() > 0 {
        children[index]
            .iter()
            .map(|&child| outline_visible(child, records, children))
            .sum()
    } else {
        0
    }
}

fn lower_page_destinations(
    stores: &Universe,
    artifacts: &[CommittedArtifact],
    records: &[tex_state::PdfPageRecord],
    pages: &[PositionedPage],
    decimal_digits: i32,
) -> Result<Vec<ShippedDestination>, PdfBuildError> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for (page, record) in pages.iter().zip(records) {
        let bytes = artifact_bytes(stores, artifacts, record.artifact())?;
        let artifact = tex_out::PageArtifact::from_bytes(&bytes)?;
        let (_, page_height) = pdf_page_extents(&artifact, *record)?;
        let page_object = object_id(record.page_object())?;
        let mut boxes = BTreeMap::new();
        for event in &page.events {
            match event {
                PositionedEvent::Box(positioned_box) => {
                    boxes.insert(positioned_box.id, *positioned_box);
                }
                PositionedEvent::PdfDestination(destination) => {
                    if !seen.insert(destination.marker.object) {
                        continue;
                    }
                    let target = destination
                        .marker
                        .structure
                        .map(object_id)
                        .transpose()?
                        .unwrap_or(page_object);
                    let x = destination
                        .x
                        .checked_add(record.h_origin())
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    let y = page_height
                        .checked_sub(destination.y)
                        .and_then(|value| value.checked_sub(record.v_origin()))
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    let number = |value| scaled_to_bp_number(value, decimal_digits);
                    let view = match destination.marker.kind {
                        tex_out::PdfDestinationKind::Xyz { zoom } => PdfDestinationView::Xyz {
                            left: number(x)?,
                            top: number(y)?,
                            zoom: zoom
                                .map(|zoom| PdfNumber::new(i64::from(zoom), 3))
                                .transpose()?,
                        },
                        tex_out::PdfDestinationKind::FitBoundingBoxHorizontal => {
                            PdfDestinationView::FitBoundingBoxHorizontal { top: number(y)? }
                        }
                        tex_out::PdfDestinationKind::FitBoundingBoxVertical => {
                            PdfDestinationView::FitBoundingBoxVertical { left: number(x)? }
                        }
                        tex_out::PdfDestinationKind::FitBoundingBox => {
                            PdfDestinationView::FitBoundingBox
                        }
                        tex_out::PdfDestinationKind::FitHorizontal => {
                            PdfDestinationView::FitHorizontal { top: number(y)? }
                        }
                        tex_out::PdfDestinationKind::FitVertical => {
                            PdfDestinationView::FitVertical { left: number(x)? }
                        }
                        tex_out::PdfDestinationKind::FitRectangle {
                            width,
                            height,
                            depth,
                        } => {
                            let positioned_box = boxes[&destination.containing_box];
                            let margin = destination.marker.margin;
                            let left = destination
                                .x
                                .checked_sub(margin)
                                .and_then(|value| value.checked_add(record.h_origin()))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let right = destination
                                .x
                                .checked_add(width.unwrap_or_else(|| {
                                    positioned_box
                                        .x
                                        .checked_add(positioned_box.width)
                                        .and_then(|right| right.checked_sub(destination.x))
                                        .unwrap_or(Scaled::from_raw(0))
                                }))
                                .and_then(|value| value.checked_add(margin))
                                .and_then(|value| value.checked_add(record.h_origin()))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let top_tex = height.map_or(positioned_box.y, |height| {
                                destination.y.checked_sub(height).unwrap_or(destination.y)
                            });
                            let bottom_tex = depth.map_or(
                                positioned_box
                                    .y
                                    .checked_add(positioned_box.height)
                                    .unwrap_or(positioned_box.y),
                                |depth| destination.y.checked_add(depth).unwrap_or(destination.y),
                            );
                            let top = page_height
                                .checked_sub(top_tex)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_add(margin))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let bottom = page_height
                                .checked_sub(bottom_tex)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_sub(margin))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            PdfDestinationView::FitRectangle {
                                left: number(left)?,
                                bottom: number(bottom)?,
                                right: number(right)?,
                                top: number(top)?,
                            }
                        }
                        tex_out::PdfDestinationKind::Fit => PdfDestinationView::Fit,
                    };
                    result.push(ShippedDestination {
                        object: destination.marker.object,
                        target,
                        view,
                    });
                }
                _ => {}
            }
        }
    }
    Ok(result)
}

fn destination_objects(
    stores: &Universe,
    pages: &[tex_state::PdfPageRecord],
    shipped: Vec<ShippedDestination>,
    next_object: &mut u32,
) -> Result<DestinationObjects, PdfBuildError> {
    let first_page = pages
        .first()
        .map(|page| object_id(page.page_object()))
        .transpose()?;
    let shipped = shipped
        .into_iter()
        .map(|value| (value.object, value))
        .collect::<BTreeMap<_, _>>();
    let mut objects = Vec::new();
    let mut names = Vec::new();
    for record in stores.pdf_destinations(false) {
        let explicit = if let Some(value) = shipped.get(&record.object()) {
            PdfExplicitDestination {
                page: value.target,
                view: value.view.clone(),
            }
        } else if let Some(page) = first_page {
            PdfExplicitDestination {
                page,
                view: PdfDestinationView::Fit,
            }
        } else {
            continue;
        };
        let named = match record.identity() {
            tex_state::PdfDestinationIdentity::Name(name) => {
                names.push((decode_pdf_string(name), object_id(record.object())?));
                true
            }
            tex_state::PdfDestinationIdentity::Number(_) => false,
        };
        objects.push(PdfIndirectObject {
            id: object_id(record.object())?,
            object: if named {
                PdfObject::NamedDestination(explicit)
            } else {
                PdfObject::Destination(explicit)
            },
        });
    }
    for record in stores.pdf_destinations(true) {
        let Some(value) = shipped.get(&record.object()) else {
            continue;
        };
        objects.push(PdfIndirectObject {
            id: object_id(record.object())?,
            object: PdfObject::Destination(PdfExplicitDestination {
                page: value.target,
                view: value.view.clone(),
            }),
        });
    }
    names.sort_by(|left, right| left.0.cmp(&right.0));
    let (tree, root) = build_destination_name_tree(names, next_object)?;
    Ok(DestinationObjects {
        destinations: objects,
        name_tree: tree,
        name_tree_root: root,
    })
}

struct DestinationObjects {
    destinations: Vec<PdfIndirectObject>,
    name_tree: Vec<PdfIndirectObject>,
    name_tree_root: Option<PdfObjectId>,
}

fn decode_pdf_string(source: &[u8]) -> Vec<u8> {
    if source.len() >= 2 && source[0] == b'<' && source[source.len() - 1] == b'>' {
        let hex = &source[1..source.len() - 1];
        if hex.iter().all(u8::is_ascii_hexdigit) {
            let mut result = Vec::with_capacity(hex.len().div_ceil(2));
            for pair in hex.chunks(2) {
                let high = (pair[0] as char).to_digit(16).expect("hex digit") as u8;
                let low = pair.get(1).map_or(0, |byte| {
                    (*byte as char).to_digit(16).expect("hex digit") as u8
                });
                result.push((high << 4) | low);
            }
            return result;
        }
    }
    let body = if source.len() >= 2 && source[0] == b'(' && source[source.len() - 1] == b')' {
        &source[1..source.len() - 1]
    } else {
        source
    };
    let mut result = Vec::with_capacity(body.len());
    let mut index = 0;
    while index < body.len() {
        if body[index] != b'\\' {
            result.push(body[index]);
            index += 1;
            continue;
        }
        index += 1;
        let Some(&escaped) = body.get(index) else {
            break;
        };
        if escaped.is_ascii_digit() && escaped < b'8' {
            let mut value = 0_u16;
            let mut count = 0;
            while count < 3 && index < body.len() && matches!(body[index], b'0'..=b'7') {
                value = value * 8 + u16::from(body[index] - b'0');
                index += 1;
                count += 1;
            }
            result.push(value as u8);
            continue;
        }
        match escaped {
            b'n' => result.push(b'\n'),
            b'r' => result.push(b'\r'),
            b't' => result.push(b'\t'),
            b'b' => result.push(8),
            b'f' => result.push(12),
            b'\n' => {}
            b'\r' => {
                if body.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
            }
            byte => result.push(byte),
        }
        index += 1;
    }
    result
}

fn build_destination_name_tree(
    names: Vec<(Vec<u8>, PdfObjectId)>,
    next_object: &mut u32,
) -> Result<(Vec<PdfIndirectObject>, Option<PdfObjectId>), PdfBuildError> {
    if names.is_empty() {
        return Ok((Vec::new(), None));
    }
    let mut objects = Vec::new();
    let mut level = Vec::new();
    for chunk in names.chunks(6) {
        let id = object_id(*next_object)?;
        *next_object = next_object
            .checked_add(1)
            .ok_or(PdfBuildError::ObjectCapacity)?;
        let min = chunk.first().expect("nonempty chunk").0.clone();
        let max = chunk.last().expect("nonempty chunk").0.clone();
        objects.push(PdfIndirectObject {
            id,
            object: PdfObject::DestinationNameTree(PdfDestinationNameTree {
                limits: Some((min.clone(), max.clone())),
                children: PdfDestinationNameTreeChildren::Names(chunk.to_vec()),
            }),
        });
        level.push((id, min, max));
    }
    while level.len() > 1 {
        let mut parent = Vec::new();
        for chunk in level.chunks(6) {
            let id = object_id(*next_object)?;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            let min = chunk.first().expect("nonempty chunk").1.clone();
            let max = chunk.last().expect("nonempty chunk").2.clone();
            objects.push(PdfIndirectObject {
                id,
                object: PdfObject::DestinationNameTree(PdfDestinationNameTree {
                    limits: Some((min.clone(), max.clone())),
                    children: PdfDestinationNameTreeChildren::Kids(
                        chunk.iter().map(|entry| entry.0).collect(),
                    ),
                }),
            });
            parent.push((id, min, max));
        }
        level = parent;
    }
    let root = level[0].0;
    Ok((objects, Some(root)))
}

fn lower_page_annotations(
    stores: &Universe,
    pages: &[PositionedPage],
    link_margins: &[Scaled],
) -> Result<Vec<Vec<ShippedAnnotation>>, PdfBuildError> {
    let annotations = stores
        .pdf_annotations()
        .iter()
        .copied()
        .map(|record| (record.object(), record))
        .collect::<BTreeMap<_, _>>();
    let links = stores
        .pdf_links()
        .iter()
        .copied()
        .map(|record| (record.object(), record))
        .collect::<BTreeMap<_, _>>();
    let mut active = Vec::<ActiveShippedLink>::new();
    let mut result = Vec::with_capacity(pages.len());

    for (page, link_margin) in pages.iter().zip(link_margins.iter().copied()) {
        let mut shipped = Vec::new();
        let mut boxes = BTreeMap::<u32, PositionedBox>::new();
        let mut running = true;
        for event in &page.events {
            match event {
                PositionedEvent::Box(positioned_box) => {
                    boxes.insert(positioned_box.id, *positioned_box);
                    if running && positioned_box.kind == BoxKind::Horizontal {
                        for link in &mut active {
                            if link.depth == positioned_box.depth
                                && link.record.dimensions().width.is_none()
                            {
                                link.candidate = Some((positioned_box.id, positioned_box.x));
                            }
                        }
                    }
                }
                PositionedEvent::BoxEnd(end) => {
                    let positioned_box = boxes[&end.id];
                    for link in &mut active {
                        if let Some((box_id, left)) = link.candidate
                            && box_id == end.id
                        {
                            shipped.push(link_segment(
                                link.record,
                                positioned_box,
                                left,
                                positioned_box
                                    .x
                                    .checked_add(positioned_box.width)
                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                link_margin,
                            )?);
                            link.candidate = None;
                        }
                    }
                }
                PositionedEvent::PdfAnnotation(marker) => {
                    let positioned_box = boxes[&marker.containing_box];
                    match marker.marker {
                        tex_out::PdfAnnotationEffect::Annotation { object } => {
                            let record = annotations
                                .get(&object)
                                .copied()
                                .ok_or(PdfBuildError::MissingAnnotationRecord(object))?;
                            let data = record
                                .data()
                                .ok_or(PdfBuildError::UninitializedAnnotation(object))?;
                            shipped.push(ShippedAnnotation {
                                source_object: object,
                                object,
                                kind: ShippedAnnotationKind::Annotation,
                                rect: marker_rect(
                                    marker.x,
                                    marker.y,
                                    positioned_box,
                                    data.dimensions,
                                    Scaled::from_raw(0),
                                )?,
                            });
                        }
                        tex_out::PdfAnnotationEffect::LinkStart { object } => {
                            let record = links
                                .get(&object)
                                .copied()
                                .ok_or(PdfBuildError::MissingLinkRecord(object))?;
                            let mut link = ActiveShippedLink {
                                record,
                                depth: marker.depth,
                                candidate: None,
                            };
                            if let Some(width) = record.dimensions().width {
                                shipped.push(link_segment(
                                    record,
                                    positioned_box,
                                    marker.x,
                                    marker
                                        .x
                                        .checked_add(width)
                                        .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                    link_margin,
                                )?);
                            } else {
                                link.candidate = Some((marker.containing_box, marker.x));
                            }
                            active.push(link);
                        }
                        tex_out::PdfAnnotationEffect::LinkEnd { object } => {
                            let index = active
                                .iter()
                                .rposition(|link| link.record.object() == object)
                                .ok_or(PdfBuildError::MissingOpenLink(object))?;
                            let link = active.remove(index);
                            if link.record.dimensions().width.is_none() {
                                let left = link
                                    .candidate
                                    .filter(|(box_id, _)| *box_id == marker.containing_box)
                                    .map_or(positioned_box.x, |(_, left)| left);
                                shipped.push(link_segment(
                                    link.record,
                                    positioned_box,
                                    left,
                                    marker.x,
                                    link_margin,
                                )?);
                            }
                        }
                        tex_out::PdfAnnotationEffect::RunningLink(enabled) => running = enabled,
                    }
                }
                PositionedEvent::TextRun(_)
                | PositionedEvent::Rule(_)
                | PositionedEvent::Special(_)
                | PositionedEvent::PdfAccessibility(_)
                | PositionedEvent::PdfGraphics(_)
                | PositionedEvent::PdfDestination(_)
                | PositionedEvent::PdfThread(_)
                | PositionedEvent::PdfEndThread { .. } => {}
            }
        }
        result.push(shipped);
    }
    Ok(result)
}

struct ThreadOutput {
    objects: Vec<PdfIndirectObject>,
    list: Option<PdfObjectId>,
    page_beads: Vec<Vec<PdfObjectId>>,
}

#[derive(Clone)]
struct ShippedBead {
    thread: PdfObjectId,
    bead: PdfObjectId,
    rectangle: PdfObjectId,
    page: PdfObjectId,
    rect: ShippedAnnotationRect,
    attributes: Vec<u8>,
    title: Vec<u8>,
    margin: Scaled,
}

fn thread_objects(
    thread_records: &[tex_state::PdfThreadRecord],
    pages: &[PositionedPage],
    page_records: &[tex_state::PdfPageRecord],
    decimal_digits: i32,
    next_object: &mut u32,
) -> Result<ThreadOutput, PdfBuildError> {
    let mut beads = Vec::<ShippedBead>::new();
    let mut page_beads = vec![Vec::new(); pages.len()];
    for (page_index, (page, record)) in pages.iter().zip(page_records).enumerate() {
        let mut boxes = BTreeMap::<u32, PositionedBox>::new();
        let mut running_bead: Option<usize> = None;
        let mut running_parent_depth = None;
        for event in &page.events {
            match event {
                PositionedEvent::Box(positioned) => {
                    boxes.insert(positioned.id, *positioned);
                    if running_parent_depth.is_some_and(|depth| positioned.depth == depth + 1)
                        && positioned.kind == BoxKind::Vertical
                        && let Some(previous) = running_bead
                    {
                        let bead = object_id(*next_object)?;
                        *next_object = next_object
                            .checked_add(1)
                            .ok_or(PdfBuildError::ObjectCapacity)?;
                        let rectangle = object_id(*next_object)?;
                        *next_object = next_object
                            .checked_add(1)
                            .ok_or(PdfBuildError::ObjectCapacity)?;
                        let source = beads[previous].clone();
                        page_beads[page_index].push(bead);
                        beads.push(ShippedBead {
                            thread: source.thread,
                            bead,
                            rectangle,
                            page: source.page,
                            rect: marker_rect(
                                positioned.x,
                                positioned.baseline,
                                *positioned,
                                PdfAnnotationDimensions {
                                    width: None,
                                    height: None,
                                    depth: None,
                                },
                                source.margin,
                            )?,
                            attributes: Vec::new(),
                            title: source.title,
                            margin: source.margin,
                        });
                        running_bead = Some(beads.len() - 1);
                    }
                }
                PositionedEvent::PdfThread(positioned) => {
                    let marker = &positioned.marker;
                    let dimensions = PdfAnnotationDimensions {
                        width: marker.width,
                        height: marker.height,
                        depth: marker.depth,
                    };
                    let rect = marker_rect(
                        positioned.x,
                        positioned.y,
                        boxes[&positioned.containing_box],
                        dimensions,
                        marker.margin,
                    )?;
                    let title = match &marker.identifier {
                        tex_out::PdfDestinationIdentifier::Name(name) => name.clone(),
                        tex_out::PdfDestinationIdentifier::Number(number) => {
                            number.to_string().into_bytes()
                        }
                    };
                    let bead = object_id(marker.bead_object)?;
                    page_beads[page_index].push(bead);
                    beads.push(ShippedBead {
                        thread: object_id(marker.thread_object)?,
                        bead,
                        rectangle: object_id(marker.rectangle_object)?,
                        page: object_id(record.page_object())?,
                        rect,
                        attributes: marker.attributes.clone(),
                        title,
                        margin: marker.margin,
                    });
                    running_bead = positioned.running.then_some(beads.len() - 1);
                    running_parent_depth = positioned
                        .running
                        .then(|| boxes[&positioned.containing_box].depth);
                }
                PositionedEvent::PdfEndThread { y, .. } => {
                    if let Some(index) = running_bead.take() {
                        beads[index].rect.bottom = y
                            .checked_add(beads[index].margin)
                            .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    }
                    running_parent_depth = None;
                }
                _ => {}
            }
        }
    }
    if let Some((page, page_record)) = pages.first().zip(page_records.first()) {
        for thread in thread_records {
            let thread_id = object_id(thread.object())?;
            if beads.iter().any(|bead| bead.thread == thread_id) {
                continue;
            }
            let bead = object_id(*next_object)?;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            let rectangle = object_id(*next_object)?;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            page_beads[0].push(bead);
            let title = match thread.identity() {
                tex_state::PdfDestinationIdentity::Name(name) => name.clone(),
                tex_state::PdfDestinationIdentity::Number(number) => {
                    number.to_string().into_bytes()
                }
            };
            beads.push(ShippedBead {
                thread: thread_id,
                bead,
                rectangle,
                page: object_id(page_record.page_object())?,
                rect: ShippedAnnotationRect {
                    left: Scaled::from_raw(0),
                    bottom: Scaled::from_raw(0),
                    right: page.width,
                    top: page.height,
                },
                attributes: Vec::new(),
                title,
                margin: Scaled::from_raw(0),
            });
        }
    }
    if beads.is_empty() {
        return Ok(ThreadOutput {
            objects: Vec::new(),
            list: None,
            page_beads,
        });
    }
    let mut by_thread = BTreeMap::<PdfObjectId, Vec<usize>>::new();
    for (index, bead) in beads.iter().enumerate() {
        by_thread.entry(bead.thread).or_default().push(index);
    }
    let list = object_id(*next_object)?;
    *next_object = next_object
        .checked_add(1)
        .ok_or(PdfBuildError::ObjectCapacity)?;
    let mut objects = vec![PdfIndirectObject {
        id: list,
        object: PdfObject::ThreadList(by_thread.keys().copied().collect()),
    }];
    for (&thread, indices) in &by_thread {
        let attributes = indices
            .iter()
            .rev()
            .find_map(|&index| {
                (!beads[index].attributes.is_empty()).then(|| beads[index].attributes.clone())
            })
            .unwrap_or_default();
        let default_title = attributes.is_empty().then(|| {
            let mut title = vec![b'('];
            title.extend_from_slice(&beads[indices[0]].title);
            title.push(b')');
            title
        });
        objects.push(PdfIndirectObject {
            id: thread,
            object: PdfObject::Thread(PdfThreadObject {
                first_bead: beads[indices[0]].bead,
                default_title,
                raw_entries: attributes,
            }),
        });
        for (position, &index) in indices.iter().enumerate() {
            let bead = &beads[index];
            let previous = beads[indices[(position + indices.len() - 1) % indices.len()]].bead;
            let next = beads[indices[(position + 1) % indices.len()]].bead;
            objects.push(PdfIndirectObject {
                id: bead.bead,
                object: PdfObject::Bead(PdfBeadObject {
                    thread: (position == 0).then_some(thread),
                    previous,
                    next,
                    page: bead.page,
                    rectangle: bead.rectangle,
                }),
            });
            let page_index = page_records
                .iter()
                .position(|record| object_id(record.page_object()).ok() == Some(bead.page))
                .expect("bead page belongs to page ledger");
            let page_height = pages[page_index].height;
            let rect = &bead.rect;
            objects.push(PdfIndirectObject {
                id: bead.rectangle,
                object: PdfObject::Value(PdfValue::Array(vec![
                    PdfValue::Number(scaled_to_bp_number(rect.left, decimal_digits)?),
                    PdfValue::Number(scaled_to_bp_number(
                        page_height
                            .checked_sub(rect.bottom)
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        decimal_digits,
                    )?),
                    PdfValue::Number(scaled_to_bp_number(rect.right, decimal_digits)?),
                    PdfValue::Number(scaled_to_bp_number(
                        page_height
                            .checked_sub(rect.top)
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        decimal_digits,
                    )?),
                ])),
            });
        }
    }
    Ok(ThreadOutput {
        objects,
        list: Some(list),
        page_beads,
    })
}

fn link_segment(
    record: PdfLinkRecord,
    positioned_box: PositionedBox,
    left: Scaled,
    right: Scaled,
    margin: Scaled,
) -> Result<ShippedAnnotation, PdfBuildError> {
    let dimensions = record.dimensions();
    let baseline = positioned_box.baseline;
    Ok(ShippedAnnotation {
        source_object: record.object(),
        object: record.object(),
        kind: ShippedAnnotationKind::Link,
        rect: marker_rect_with_right(left, right, baseline, positioned_box, dimensions, margin)?,
    })
}

fn marker_rect(
    left: Scaled,
    baseline: Scaled,
    positioned_box: PositionedBox,
    dimensions: PdfAnnotationDimensions,
    margin: Scaled,
) -> Result<ShippedAnnotationRect, PdfBuildError> {
    let right = left
        .checked_add(dimensions.width.unwrap_or_else(|| {
            positioned_box
                .x
                .checked_add(positioned_box.width)
                .and_then(|right| right.checked_sub(left))
                .unwrap_or(Scaled::from_raw(0))
        }))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    marker_rect_with_right(left, right, baseline, positioned_box, dimensions, margin)
}

fn marker_rect_with_right(
    left: Scaled,
    right: Scaled,
    baseline: Scaled,
    positioned_box: PositionedBox,
    dimensions: PdfAnnotationDimensions,
    margin: Scaled,
) -> Result<ShippedAnnotationRect, PdfBuildError> {
    let top = match dimensions.height {
        Some(height) => baseline
            .checked_sub(height)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        None => positioned_box.y,
    };
    let bottom = match dimensions.depth {
        Some(depth) => baseline
            .checked_add(depth)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        None => positioned_box
            .y
            .checked_add(positioned_box.height)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
    };
    Ok(ShippedAnnotationRect {
        left: left
            .checked_sub(margin)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        top: top
            .checked_sub(margin)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        right: right
            .checked_add(margin)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        bottom: bottom
            .checked_add(margin)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
    })
}

fn assign_annotation_objects(
    stores: &mut Universe,
    pages: &mut [Vec<ShippedAnnotation>],
) -> Result<(), PdfBuildError> {
    let mut used = BTreeSet::new();
    for annotation in pages.iter_mut().flatten() {
        annotation.object = if used.insert(annotation.source_object) {
            annotation.source_object
        } else {
            stores
                .reserve_pdf_link_continuation()
                .map_err(|_| PdfBuildError::ObjectCapacity)?
        };
    }
    Ok(())
}

fn annotation_object(
    stores: &Universe,
    shipped: ShippedAnnotation,
    page: tex_state::PdfPageRecord,
    page_height: Scaled,
    pages: &[tex_state::PdfPageRecord],
    decimal_digits: i32,
) -> Result<PdfIndirectObject, PdfBuildError> {
    let left = shipped
        .rect
        .left
        .checked_add(page.h_origin())
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let right = shipped
        .rect
        .right
        .checked_add(page.h_origin())
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let bottom = page_height
        .checked_sub(shipped.rect.bottom)
        .and_then(|value| value.checked_sub(page.v_origin()))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let top = page_height
        .checked_sub(shipped.rect.top)
        .and_then(|value| value.checked_sub(page.v_origin()))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let (subtype, action, raw_entries) = match shipped.kind {
        ShippedAnnotationKind::Annotation => {
            let record = stores
                .pdf_annotations()
                .iter()
                .find(|record| record.object() == shipped.source_object)
                .and_then(|record| record.data())
                .ok_or(PdfBuildError::MissingAnnotationRecord(
                    shipped.source_object,
                ))?;
            (None, None, token_list_bytes(stores, record.entries))
        }
        ShippedAnnotationKind::Link => {
            let record = stores
                .pdf_links()
                .iter()
                .copied()
                .find(|record| record.object() == shipped.source_object)
                .ok_or(PdfBuildError::MissingLinkRecord(shipped.source_object))?;
            let raw_entries = token_list_bytes(stores, record.attributes());
            let action = detached_link_action(stores, record.action(), pages)?;
            let subtype = (!matches!(action, PdfAnnotationAction::UserEntries(_)))
                .then_some(PdfAnnotationType::Link);
            (subtype, Some(action), raw_entries)
        }
    };
    Ok(PdfIndirectObject {
        id: object_id(shipped.object)?,
        object: PdfObject::Annotation(PdfAnnotationObject {
            rect: [
                scaled_to_bp_number(left, decimal_digits)?,
                scaled_to_bp_number(bottom, decimal_digits)?,
                scaled_to_bp_number(right, decimal_digits)?,
                scaled_to_bp_number(top, decimal_digits)?,
            ],
            subtype,
            action,
            raw_entries,
        }),
    })
}

fn detached_link_action(
    stores: &Universe,
    spec: PdfActionSpec,
    pages: &[tex_state::PdfPageRecord],
) -> Result<PdfAnnotationAction, PdfBuildError> {
    let destination = match spec {
        PdfActionSpec::User(tokens) => {
            return Ok(PdfAnnotationAction::UserEntries(token_list_bytes(
                stores, tokens,
            )));
        }
        PdfActionSpec::GoTo(destination) => (PdfDestinationActionKind::GoTo, destination),
        PdfActionSpec::Thread(destination) => (PdfDestinationActionKind::Thread, destination),
    };
    let (kind, destination) = destination;
    let external = destination.file.is_some();
    let target = match destination.target {
        PdfActionTarget::Page { number, view } => {
            let page = if external {
                PdfDestinationPage::External(number.saturating_sub(1))
            } else {
                PdfDestinationPage::Internal(object_id(
                    pages
                        .get((number - 1) as usize)
                        .ok_or(PdfBuildError::OpenActionPageNotFound(number))?
                        .page_object(),
                )?)
            };
            PdfDestinationTarget::Page {
                page,
                view: token_list_bytes(stores, view),
            }
        }
        PdfActionTarget::Destination(PdfActionIdentifier::Name(tokens)) => {
            PdfDestinationTarget::Name(token_list_bytes(stores, tokens))
        }
        PdfActionTarget::Destination(PdfActionIdentifier::Number(number)) => {
            if external {
                PdfDestinationTarget::Number(number)
            } else {
                let identity = tex_state::PdfDestinationIdentity::Number(number);
                PdfDestinationTarget::Reference(object_id(
                    if kind == PdfDestinationActionKind::Thread {
                        stores
                            .pdf_threads()
                            .iter()
                            .find(|thread| thread.identity() == &identity)
                            .expect("local numeric thread action reserves its thread")
                            .object()
                    } else {
                        stores
                            .pdf_destination(&identity, false)
                            .expect("local numeric action reserves its destination")
                            .object()
                    },
                )?)
            }
        }
        PdfActionTarget::Destination(PdfActionIdentifier::Raw(tokens)) => {
            PdfDestinationTarget::Name(token_list_bytes(stores, tokens))
        }
    };
    let structure = destination.structure.and_then(|identifier| {
        if external {
            Some(match identifier {
                PdfActionIdentifier::Name(tokens) | PdfActionIdentifier::Raw(tokens) => {
                    PdfDestinationStructure::External(token_list_bytes(stores, tokens))
                }
                PdfActionIdentifier::Number(number) => {
                    PdfDestinationStructure::External(number.to_string().into_bytes())
                }
            })
        } else {
            let identity = match identifier {
                PdfActionIdentifier::Name(tokens) | PdfActionIdentifier::Raw(tokens) => {
                    tex_state::PdfDestinationIdentity::Name(token_list_bytes(stores, tokens))
                }
                PdfActionIdentifier::Number(number) => {
                    tex_state::PdfDestinationIdentity::Number(number)
                }
            };
            stores
                .pdf_destination(&identity, true)
                .filter(|record| record.defined())
                .map(|record| {
                    PdfDestinationStructure::Internal(
                        object_id(record.object()).expect("valid reserved destination object"),
                    )
                })
        }
    });
    Ok(PdfAnnotationAction::Destination(PdfDestinationAction {
        kind,
        file: destination
            .file
            .map(|tokens| token_list_bytes(stores, tokens)),
        target,
        structure,
        new_window: match destination.window {
            PdfActionWindow::Unspecified => None,
            PdfActionWindow::New => Some(true),
            PdfActionWindow::Same => Some(false),
        },
    }))
}

#[derive(Clone)]
struct PdfFontObjectIds {
    font: PdfObjectId,
    descriptor: Option<PdfObjectId>,
    program: Option<PdfObjectId>,
    to_unicode: Option<PdfObjectId>,
    char_procs: BTreeMap<u8, PdfObjectId>,
}

#[derive(Clone, Copy)]
struct PdfFallbackSpaceFont {
    font: PdfObjectId,
}

fn allocate_fallback_space_font(
    stores: &Universe,
    space_font_name_id: u32,
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
    let selected_name = stores
        .pdf_space_font_name(space_font_name_id)
        .ok_or(PdfBuildError::MissingSpaceFontName(space_font_name_id))?;

    objects.push(PdfIndirectObject {
        id: char_proc,
        object: PdfObject::Stream {
            dictionary: PdfDictionary::new(),
            data: tex_out::pdf::type3_space_glyph_content(333.0),
        },
    });

    let matrix = PdfNumber::new(1, 3)?;
    let mut dictionary = PdfDictionary::new();
    dictionary.insert("Type", PdfValue::Name("Font".into()))?;
    dictionary.insert("Subtype", PdfValue::Name("Type3".into()))?;
    dictionary.insert("Name", PdfValue::Name(PdfName::new(selected_name)))?;
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

fn ensure_fallback_space_font(
    stores: &Universe,
    space_font_name_id: u32,
    next_object: &mut u32,
    objects: &mut Vec<PdfIndirectObject>,
    fallback: &mut Option<PdfFallbackSpaceFont>,
) -> Result<PdfFallbackSpaceFont, PdfBuildError> {
    if let Some(fallback) = *fallback {
        return Ok(fallback);
    }
    let allocated = allocate_fallback_space_font(stores, space_font_name_id, next_object, objects)?;
    *fallback = Some(allocated);
    Ok(allocated)
}

fn font_has_explicit_space(
    stores: &Universe,
    resolved_font_map: &BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>,
    tex_name: &[u8],
) -> bool {
    resolved_font_map
        .get(tex_name)
        .and_then(|entry| entry.encoding_files.first().cloned())
        .and_then(|encoding| stores.pdf_encoding(&encoding))
        .is_some_and(|encoding| encoding.glyph_names()[32] == b"space")
}

fn pdf_font_objects(
    stores: &Universe,
    ids: PdfFontObjectIds,
    font: &tex_out::FontResource,
    resource_name: &[u8],
    used_codes: &BTreeSet<u8>,
    driver_dpi: i32,
    resolved_font_map: &BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>,
) -> Result<Vec<PdfIndirectObject>, PdfBuildError> {
    let mapped = resolved_font_map.get(font.name.as_bytes());
    let subset_requested = mapped
        .as_ref()
        .is_some_and(|entry| entry.program == tex_fonts::PdfFontMapProgram::Subset);
    let program_name = mapped.as_ref().and_then(|entry| entry.font_file.as_deref());
    let resident = mapped
        .as_ref()
        .is_some_and(|entry| entry.program == tex_fonts::PdfFontMapProgram::Resident);
    if mapped.is_none() {
        return pdf_pk_font_objects(stores, ids, font, resource_name, used_codes, driver_dpi);
    }
    if program_name.is_none() && !resident {
        return Err(PdfBuildError::MissingFontProgram(
            font.name.as_bytes().to_vec(),
        ));
    }
    let is_truetype = program_name.is_some_and(|name| {
        name.rsplit(|byte| *byte == b'.')
            .next()
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case(b"ttf") || extension.eq_ignore_ascii_case(b"woff2")
            })
    });
    let type1 = (!is_truetype)
        .then(|| program_name.and_then(|name| stores.pdf_type1_program(name)))
        .flatten();
    let truetype = is_truetype
        .then(|| program_name.and_then(|name| stores.pdf_truetype_program(name)))
        .flatten();
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
    let encoding = mapped
        .as_ref()
        .and_then(|entry| entry.encoding_files.first())
        .map(|encoding_name| {
            stores
                .pdf_encoding(encoding_name)
                .ok_or_else(|| PdfBuildError::MissingEncoding(encoding_name.clone()))
        })
        .transpose()?;
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
            .map(|program| program.subset(&glyph_names, &subset_font_name))
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
    let mut dictionary = PdfDictionary::new();
    dictionary.insert("Type", PdfValue::Name("Font".into()))?;
    dictionary.insert(
        "Subtype",
        PdfValue::Name(if is_truetype { "TrueType" } else { "Type1" }.into()),
    )?;
    dictionary.insert("Name", PdfValue::Name(PdfName::new(resource_name)))?;
    dictionary.insert(
        "BaseFont",
        PdfValue::Name(PdfName::new(subset_font_name.clone())),
    )?;
    if let Some(encoding) = encoding {
        let differences = encoding_differences(encoding, used_codes, subset_requested);
        let mut encoding_dictionary = PdfDictionary::new();
        encoding_dictionary.insert("Type", PdfValue::Name("Encoding".into()))?;
        encoding_dictionary.insert("Differences", PdfValue::Array(differences))?;
        dictionary.insert("Encoding", PdfValue::Dictionary(encoding_dictionary))?;
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
    let font_id = stores
        .font_by_source_identity(font.semantic_identity)
        .ok_or(PdfBuildError::MissingLiveFont(font.name.clone()))?;
    let denominator = i64::from(font.at_size.raw()).max(1);
    let widths = (first_char as u8..=last_char as u8)
        .map(|code| {
            let width = stores
                .font_char_metrics(font_id, code)
                .map_or(0, |metrics| i64::from(metrics.width.raw()));
            PdfValue::Integer((width * 1000 + denominator / 2) / denominator)
        })
        .collect();
    dictionary.insert("Widths", PdfValue::Array(widths))?;
    let to_unicode = ids
        .to_unicode
        .map(|to_unicode_id| {
            to_unicode_stream(stores, font, used_codes, encoding, type1, to_unicode_id)
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
    let scale_metric =
        |value: Scaled| (i64::from(value.raw()) * 1000 + denominator / 2) / denominator;
    let tfm_ascent = (0u8..=255)
        .filter_map(|code| stores.font_char_metrics(font_id, code))
        .map(|metrics| scale_metric(metrics.height))
        .max()
        .unwrap_or(0);
    let tfm_descent = (0u8..=255)
        .filter_map(|code| stores.font_char_metrics(font_id, code))
        .map(|metrics| scale_metric(metrics.depth))
        .max()
        .unwrap_or(0);
    let tfm_cap_height = stores
        .font_char_metrics(font_id, b'H')
        .map_or(tfm_ascent, |metrics| scale_metric(metrics.height));
    let tfm_x_height = scale_metric(stores.font_parameter(font_id, 5));
    let (bbox, ascent, descent, cap_height, x_height, italic_angle, stem_v, fixed_pitch) =
        if let Some(program) = truetype {
            (
                program.bbox(),
                i64::from(program.ascent()),
                i64::from(program.descent()),
                i64::from(program.cap_height()),
                i64::from(program.x_height()),
                i64::from(program.italic_angle()),
                i64::from(program.stem_v()),
                program.fixed_pitch(),
            )
        } else {
            let program = type1.expect("program kind checked");
            (
                program.font_bbox().unwrap_or([-500, -500, 1500, 1500]),
                tfm_ascent,
                -tfm_descent,
                tfm_cap_height,
                tfm_x_height,
                i64::from(program.italic_angle().unwrap_or(0)),
                i64::from(program.stem_v().unwrap_or(80)),
                program.is_fixed_pitch(),
            )
        };
    let flags = 4 + i64::from(fixed_pitch) + if italic_angle != 0 { 64 } else { 0 };
    descriptor.insert("Flags", PdfValue::Integer(flags))?;
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
    descriptor.set_raw_entries(stores.pdf_font_attribute(font_id).to_vec());
    if subset_requested && !is_truetype && !stores.pdf_font_configuration().omits_charset() {
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

fn pdf_pk_font_objects(
    stores: &Universe,
    ids: PdfFontObjectIds,
    font: &tex_out::FontResource,
    resource_name: &[u8],
    used_codes: &BTreeSet<u8>,
    driver_dpi: i32,
) -> Result<Vec<PdfIndirectObject>, PdfBuildError> {
    let font_id = stores
        .font_by_source_identity(font.semantic_identity)
        .ok_or_else(|| PdfBuildError::MissingLiveFont(font.name.clone()))?;
    let request = pk_font_request(stores, font_id, driver_dpi).map_err(PdfBuildError::PkFont)?;
    let pk = stores
        .pdf_pk_font(&request)
        .ok_or_else(|| PdfBuildError::MissingPkFont(request.clone()))?;
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
        let metrics = stores.font_char_metrics(font_id, code);
        widths.push(PdfValue::Number(PdfNumber::new(
            metrics.map_or(0, |metrics| {
                pk_advance_hundredths(metrics.width, request.dpi())
            }),
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
        let advance = stores
            .font_char_metrics(font_id, code)
            .map_or(0.0, |metrics| {
                pk_advance_hundredths(metrics.width, request.dpi()) as f32 / 100.0
            });
        let data = tex_out::pdf::type3_bitmap_glyph_content(&tex_out::pdf::PdfType3BitmapGlyph {
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

    let mut dictionary = PdfDictionary::new();
    dictionary.insert("Type", PdfValue::Name("Font".into()))?;
    dictionary.insert("Subtype", PdfValue::Name("Type3".into()))?;
    dictionary.insert("Name", PdfValue::Name(PdfName::new(resource_name)))?;
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

fn rounded_pk_matrix(at_size: Scaled, dpi: u32) -> Result<PdfNumber, PdfBuildError> {
    let denominator = i64::from(at_size.raw())
        .checked_mul(i64::from(dpi))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    if denominator <= 0 {
        return Err(PdfBuildError::PageGeometryOverflow);
    }
    let numerator = 7_227_i64 * 65_536 * 1_000;
    PdfNumber::new((numerator + denominator / 2) / denominator, 5).map_err(Into::into)
}

fn pk_advance_hundredths(width: Scaled, dpi: u32) -> i64 {
    let numerator = i64::from(width.raw()) * i64::from(dpi) * 10_000;
    let denominator = 65_536_i64 * 7_227;
    (numerator + denominator / 2) / denominator
}

fn encoding_differences(
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

fn to_unicode_stream(
    stores: &Universe,
    font: &tex_out::FontResource,
    used_codes: &BTreeSet<u8>,
    encoding: Option<&tex_fonts::PdfEncoding>,
    type1: Option<&tex_fonts::PdfType1Program>,
    id: PdfObjectId,
) -> Result<(PdfObjectId, PdfIndirectObject), PdfBuildError> {
    let mut mappings = Vec::new();
    for &code in used_codes {
        let owned_glyph;
        let glyph = if let Some(encoding) = encoding {
            encoding.glyph_names()[usize::from(code)].as_slice()
        } else if let Some(type1) = type1 {
            owned_glyph = type1.builtin_glyph_name(code).ok_or_else(|| {
                PdfBuildError::MissingBuiltinGlyphName {
                    font: font.name.clone(),
                    code,
                }
            })?;
            owned_glyph.as_slice()
        } else {
            continue;
        };
        let unicode = stores
            .pdf_glyph_to_unicode(font.name.as_bytes(), glyph)
            .map(ToOwned::to_owned)
            .or_else(|| {
                stores
                    .has_pdf_glyph_to_unicode_mappings()
                    .then(|| inferred_glyph_unicode(glyph))
                    .flatten()
            });
        if let Some(unicode) = unicode {
            mappings.push((code, unicode));
        }
    }
    let data = build_to_unicode_cmap(&font.name, &mappings);
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

fn inferred_glyph_unicode(name: &[u8]) -> Option<Vec<u32>> {
    let name = name.split(|byte| *byte == b'.').next()?;
    if let Some(hex) = name.strip_prefix(b"uni")
        && !hex.is_empty()
        && hex.len() % 4 == 0
        && hex.iter().all(u8::is_ascii_hexdigit)
    {
        return hex
            .chunks(4)
            .map(|chunk| {
                std::str::from_utf8(chunk)
                    .ok()
                    .and_then(|text| u32::from_str_radix(text, 16).ok())
                    .filter(|value| char::from_u32(*value).is_some())
            })
            .collect();
    }
    if let Some(hex) = name.strip_prefix(b"u")
        && (4..=6).contains(&hex.len())
        && hex.iter().all(u8::is_ascii_hexdigit)
    {
        return std::str::from_utf8(hex)
            .ok()
            .and_then(|text| u32::from_str_radix(text, 16).ok())
            .filter(|value| char::from_u32(*value).is_some())
            .map(|value| vec![value]);
    }
    None
}

fn build_to_unicode_cmap(font_name: &str, mappings: &[(u8, Vec<u32>)]) -> Vec<u8> {
    let mut cmap = format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (TeX) /Ordering (glyphs) /Supplement 0 >> def\n/CMapName /TeX-{font_name}-0 def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n"
    )
    .into_bytes();
    for chunk in mappings.chunks(100) {
        cmap.extend_from_slice(format!("{} beginbfchar\n", chunk.len()).as_bytes());
        for (code, unicode) in chunk {
            cmap.extend_from_slice(format!("<{code:02X}> <").as_bytes());
            for scalar in unicode {
                let mut encoded = [0; 2];
                for unit in char::from_u32(*scalar)
                    .expect("validated Unicode scalar")
                    .encode_utf16(&mut encoded)
                {
                    cmap.extend_from_slice(format!("{unit:04X}").as_bytes());
                }
            }
            cmap.extend_from_slice(b">\n");
        }
        cmap.extend_from_slice(b"endbfchar\n");
    }
    cmap.extend_from_slice(b"endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

fn document_info_dictionary(
    stores: &Universe,
    parameters: PdfOutputParameters,
) -> Result<PdfDictionary, PdfModelError> {
    const PRODUCER: &[u8] = b"pdfTeX-1.40.27";
    const FULL_BANNER: &[u8] = b"This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025)";

    let mut info = PdfDictionary::new();
    info.insert("Producer", PdfValue::String(PRODUCER.to_vec()))?;
    info.insert("Creator", PdfValue::String(b"TeX".to_vec()))?;
    if stores.int_param(IntParam::PDF_INFO_OMIT_DATE) == 0 {
        let date = pdf_date(stores.world().job_clock());
        info.insert("CreationDate", PdfValue::String(date.clone()))?;
        info.insert("ModDate", PdfValue::String(date))?;
    }
    info.insert("Trapped", PdfValue::Name("False".into()))?;
    if stores.int_param(IntParam::PDF_SUPPRESS_PTEX_INFO) % 2 == 0 {
        let key = if stores.int_param(IntParam::PDF_PTEX_USE_UNDERSCORE) > 0
            || parameters.major_version >= 2
        {
            "PTEX_Fullbanner"
        } else {
            "PTEX.Fullbanner"
        };
        info.insert(key, PdfValue::String(FULL_BANNER.to_vec()))?;
    }
    Ok(info)
}

fn pdf_date(clock: tex_state::JobClock) -> Vec<u8> {
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
        clock.year,
        clock.month,
        clock.day,
        clock.time / 60,
        clock.time % 60,
        clock.second,
    )
    .into_bytes()
}

fn artifact_bytes(
    stores: &Universe,
    artifacts: &[CommittedArtifact],
    hash: ContentHash,
) -> Result<Vec<u8>, PdfBuildError> {
    if let Some(artifact) = artifacts.iter().find(|artifact| artifact.hash() == hash) {
        return Ok(artifact.bytes().to_vec());
    }
    stores
        .world()
        .read_artifact(hash)?
        .ok_or(PdfBuildError::MissingArtifact(hash))
}

type RasterStreams = (
    Vec<u8>,
    PdfImageFilter,
    u8,
    PdfImageColorSpace,
    Option<(Vec<u8>, PdfImageFilter)>,
);

#[derive(Default)]
struct ImageImportTelemetry {
    parse_copy_ns: u128,
    decode_ns: u128,
    transform_ns: u128,
    encode_ns: u128,
    cache_hits: usize,
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn raster_image_streams(
    bytes: &[u8],
    metadata: tex_state::PdfRasterImageMetadata,
    parameters: PdfOutputParameters,
    telemetry: &mut ImageImportTelemetry,
) -> Result<RasterStreams, PdfBuildError> {
    if metadata.width == 0 || metadata.height == 0 {
        return Err(PdfBuildError::InvalidRasterDimensions);
    }
    let color_space = match metadata.color_space {
        PdfRasterColorSpace::Gray => PdfImageColorSpace::DeviceGray,
        PdfRasterColorSpace::Rgb => PdfImageColorSpace::DeviceRgb,
        PdfRasterColorSpace::Cmyk => PdfImageColorSpace::DeviceCmyk,
    };
    let streams: Result<RasterStreams, PdfBuildError> = match metadata.format {
        PdfRasterFormat::Jpeg => Ok((
            {
                let started = std::time::Instant::now();
                let copy = bytes.to_vec();
                telemetry.parse_copy_ns += started.elapsed().as_nanos();
                copy
            },
            PdfImageFilter::Dct,
            metadata.bits_per_component,
            color_space,
            None,
        )),
        PdfRasterFormat::Png if metadata.png_color_type == Some(3) => {
            let (color, alpha) = png_indexed_streams(bytes, metadata, telemetry)?;
            Ok((
                color,
                PdfImageFilter::Flate,
                8,
                PdfImageColorSpace::DeviceRgb,
                alpha.map(|alpha| (alpha, PdfImageFilter::Flate)),
            ))
        }
        PdfRasterFormat::Png if metadata.alpha => {
            let (color, color_filter, alpha, alpha_filter) =
                png_alpha_streams(bytes, metadata, telemetry)?;
            Ok((
                color,
                color_filter,
                metadata.bits_per_component,
                color_space,
                Some((alpha, alpha_filter)),
            ))
        }
        PdfRasterFormat::Png => Ok((
            {
                let started = std::time::Instant::now();
                let data = png_idat(bytes)?;
                telemetry.parse_copy_ns += started.elapsed().as_nanos();
                data
            },
            PdfImageFilter::FlatePngPredictor {
                colors: raster_color_components(metadata.color_space),
            },
            metadata.bits_per_component,
            color_space,
            None,
        )),
    };
    let mut streams = streams?;
    if metadata.format == PdfRasterFormat::Png
        && metadata.bits_per_component == 16
        && (parameters.image_hicolor == 0
            || (parameters.major_version == 1 && parameters.minor_version < 5))
    {
        let samples = match streams.1 {
            PdfImageFilter::FlatePngPredictor { .. } => png_opaque_samples(bytes, metadata)?,
            PdfImageFilter::Flate => inflate(&streams.0)?,
            PdfImageFilter::Dct => unreachable!("PNG streams do not use DCT"),
        };
        streams.0 = zlib(&strip_png_16(&samples))?;
        streams.1 = PdfImageFilter::Flate;
        streams.2 = 8;
        if let Some((alpha, _)) = streams.4.take() {
            streams.4 = Some((
                zlib(&strip_png_16(&inflate(&alpha)?))?,
                PdfImageFilter::Flate,
            ));
        }
    }
    if metadata.format == PdfRasterFormat::Png && parameters.image_apply_gamma > 0 {
        let mut samples = match streams.1 {
            PdfImageFilter::FlatePngPredictor { .. } => png_opaque_samples(bytes, metadata)?,
            PdfImageFilter::Flate => inflate(&streams.0)?,
            PdfImageFilter::Dct => unreachable!("PNG streams do not use DCT"),
        };
        apply_png_gamma(&mut samples, bytes, streams.2, parameters)?;
        streams.0 = zlib(&samples)?;
        streams.1 = PdfImageFilter::Flate;
    }
    Ok(streams)
}

fn strip_png_16(samples: &[u8]) -> Vec<u8> {
    samples.chunks_exact(2).map(|sample| sample[0]).collect()
}

fn raster_color_components(color_space: PdfRasterColorSpace) -> u8 {
    match color_space {
        PdfRasterColorSpace::Gray => 1,
        PdfRasterColorSpace::Rgb => 3,
        PdfRasterColorSpace::Cmyk => 4,
    }
}

fn image_resource_name(
    image: &tex_state::PdfExternalImageRecord,
    parameters: PdfOutputParameters,
) -> Vec<u8> {
    if parameters.unique_resource_names > 0 {
        let prefix = image.identity().hex();
        format!("{}Im{}", &prefix[..6], image.id().raw()).into_bytes()
    } else {
        format!("Im{}", image.id().raw()).into_bytes()
    }
}

fn png_idat(bytes: &[u8]) -> Result<Vec<u8>, PdfBuildError> {
    let mut cursor = 8usize;
    let mut data = Vec::new();
    while cursor.checked_add(12).is_some_and(|end| end <= bytes.len()) {
        let length = u32::from_be_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or(PdfBuildError::InvalidPng)?;
        if end > bytes.len() {
            return Err(PdfBuildError::InvalidPng);
        }
        if &bytes[cursor + 4..cursor + 8] == b"IDAT" {
            data.extend_from_slice(&bytes[cursor + 8..cursor + 8 + length]);
        }
        cursor = end;
    }
    (!data.is_empty())
        .then_some(data)
        .ok_or(PdfBuildError::InvalidPng)
}

fn inflate(bytes: &[u8]) -> Result<Vec<u8>, PdfBuildError> {
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    Ok(output)
}

fn png_opaque_samples(
    bytes: &[u8],
    metadata: tex_state::PdfRasterImageMetadata,
) -> Result<Vec<u8>, PdfBuildError> {
    if !matches!(metadata.bits_per_component, 8 | 16) {
        return Err(PdfBuildError::InvalidPng);
    }
    let component_bytes = usize::from(metadata.bits_per_component / 8);
    let pixel_bytes = usize::from(raster_color_components(metadata.color_space)) * component_bytes;
    let row_bytes = usize::try_from(metadata.width)
        .ok()
        .and_then(|width| width.checked_mul(pixel_bytes))
        .ok_or(PdfBuildError::InvalidPng)?;
    let height = usize::try_from(metadata.height).map_err(|_| PdfBuildError::InvalidPng)?;
    let filtered = inflate(&png_idat(bytes)?)?;
    if filtered.len() != (row_bytes + 1).saturating_mul(height) {
        return Err(PdfBuildError::InvalidPng);
    }
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    let mut samples = Vec::with_capacity(row_bytes * height);
    for row in filtered.chunks_exact(row_bytes + 1) {
        unfilter_png_row(row[0], &row[1..], &previous, &mut current, pixel_bytes)?;
        samples.extend_from_slice(&current);
        std::mem::swap(&mut previous, &mut current);
    }
    Ok(samples)
}

fn apply_png_gamma(
    samples: &mut [u8],
    png: &[u8],
    bits_per_component: u8,
    parameters: PdfOutputParameters,
) -> Result<(), PdfBuildError> {
    let file_gamma = png_chunk(png, b"gAMA")
        .and_then(|chunk| <[u8; 4]>::try_from(chunk).ok())
        .map(u32::from_be_bytes)
        .map_or_else(
            || 1_000.0 / f64::from(parameters.image_gamma.max(1)),
            |gamma| f64::from(gamma) / 100_000.0,
        );
    let screen_gamma = f64::from(parameters.gamma.max(1)) / 1_000.0;
    let exponent = 1.0 / (file_gamma * screen_gamma);
    match bits_per_component {
        8 => {
            for sample in samples {
                let normalized = f64::from(*sample) / 255.0;
                *sample = (normalized.powf(exponent) * 255.0).round() as u8;
            }
        }
        16 => {
            for sample in samples.chunks_exact_mut(2) {
                let value = u16::from_be_bytes([sample[0], sample[1]]);
                let normalized = f64::from(value) / 65_535.0;
                let corrected = (normalized.powf(exponent) * 65_535.0).round() as u16;
                sample.copy_from_slice(&corrected.to_be_bytes());
            }
        }
        _ => return Err(PdfBuildError::InvalidPng),
    }
    Ok(())
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn png_alpha_streams(
    bytes: &[u8],
    metadata: tex_state::PdfRasterImageMetadata,
    telemetry: &mut ImageImportTelemetry,
) -> Result<(Vec<u8>, PdfImageFilter, Vec<u8>, PdfImageFilter), PdfBuildError> {
    if !matches!(metadata.bits_per_component, 8 | 16) {
        return Err(PdfBuildError::InvalidPng);
    }
    let color_components = usize::from(raster_color_components(metadata.color_space));
    let component_bytes = usize::from(metadata.bits_per_component / 8);
    let pixel_bytes = (color_components + 1) * component_bytes;
    let width = usize::try_from(metadata.width).map_err(|_| PdfBuildError::InvalidPng)?;
    let row_bytes = width
        .checked_mul(pixel_bytes)
        .ok_or(PdfBuildError::InvalidPng)?;
    let height = usize::try_from(metadata.height).map_err(|_| PdfBuildError::InvalidPng)?;
    let started = std::time::Instant::now();
    let compressed = png_idat(bytes)?;
    telemetry.parse_copy_ns += started.elapsed().as_nanos();
    let started = std::time::Instant::now();
    let mut decoder = flate2::read::ZlibDecoder::new(compressed.as_slice());
    let mut filtered = Vec::new();
    decoder
        .read_to_end(&mut filtered)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    telemetry.decode_ns += started.elapsed().as_nanos();
    if filtered.len() != (row_bytes + 1).saturating_mul(height) {
        return Err(PdfBuildError::InvalidPng);
    }
    let started = std::time::Instant::now();
    if metadata.bits_per_component == 16 {
        let mut previous = vec![0u8; row_bytes];
        let mut current = vec![0u8; row_bytes];
        let mut color = Vec::with_capacity(row_bytes * height);
        let mut alpha = Vec::with_capacity(width * component_bytes * height);
        for row in filtered.chunks_exact(row_bytes + 1) {
            unfilter_png_row(row[0], &row[1..], &previous, &mut current, pixel_bytes)?;
            for pixel in current.chunks_exact(pixel_bytes) {
                color.extend_from_slice(&pixel[..color_components * component_bytes]);
                alpha.extend_from_slice(&pixel[color_components * component_bytes..]);
            }
            std::mem::swap(&mut previous, &mut current);
        }
        telemetry.transform_ns += started.elapsed().as_nanos();
        let started = std::time::Instant::now();
        let streams = (zlib(&color)?, zlib(&alpha)?);
        telemetry.encode_ns += started.elapsed().as_nanos();
        return Ok((
            streams.0,
            PdfImageFilter::Flate,
            streams.1,
            PdfImageFilter::Flate,
        ));
    }
    let color_row_bytes = width
        .checked_mul(color_components * component_bytes)
        .ok_or(PdfBuildError::InvalidPng)?;
    let alpha_row_bytes = width
        .checked_mul(component_bytes)
        .ok_or(PdfBuildError::InvalidPng)?;
    let mut color = Vec::with_capacity((color_row_bytes + 1).saturating_mul(height));
    let mut alpha = Vec::with_capacity((alpha_row_bytes + 1).saturating_mul(height));
    for row in filtered.chunks_exact(row_bytes + 1) {
        if row[0] > 4 {
            return Err(PdfBuildError::InvalidPng);
        }
        color.push(row[0]);
        alpha.push(row[0]);
        for pixel in row[1..].chunks_exact(pixel_bytes) {
            color.extend_from_slice(&pixel[..color_components * component_bytes]);
            alpha.extend_from_slice(&pixel[color_components * component_bytes..]);
        }
    }
    telemetry.transform_ns += started.elapsed().as_nanos();
    let started = std::time::Instant::now();
    let streams = (zlib(&color)?, zlib(&alpha)?);
    telemetry.encode_ns += started.elapsed().as_nanos();
    Ok((
        streams.0,
        PdfImageFilter::FlatePngPredictor {
            colors: raster_color_components(metadata.color_space),
        },
        streams.1,
        PdfImageFilter::FlatePngPredictor { colors: 1 },
    ))
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn png_indexed_streams(
    bytes: &[u8],
    metadata: tex_state::PdfRasterImageMetadata,
    telemetry: &mut ImageImportTelemetry,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PdfBuildError> {
    let palette = png_chunk(bytes, b"PLTE").ok_or(PdfBuildError::InvalidPng)?;
    if palette.len() % 3 != 0 || !matches!(metadata.bits_per_component, 1 | 2 | 4 | 8) {
        return Err(PdfBuildError::InvalidPng);
    }
    let transparency = png_chunk(bytes, b"tRNS");
    let width = usize::try_from(metadata.width).map_err(|_| PdfBuildError::InvalidPng)?;
    let height = usize::try_from(metadata.height).map_err(|_| PdfBuildError::InvalidPng)?;
    let row_bytes = width
        .checked_mul(usize::from(metadata.bits_per_component))
        .and_then(|bits| bits.checked_add(7))
        .map(|bits| bits / 8)
        .ok_or(PdfBuildError::InvalidPng)?;
    let started = std::time::Instant::now();
    let compressed = png_idat(bytes)?;
    telemetry.parse_copy_ns += started.elapsed().as_nanos();
    let started = std::time::Instant::now();
    let mut decoder = flate2::read::ZlibDecoder::new(compressed.as_slice());
    let mut filtered = Vec::new();
    decoder
        .read_to_end(&mut filtered)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    telemetry.decode_ns += started.elapsed().as_nanos();
    if filtered.len() != (row_bytes + 1).saturating_mul(height) {
        return Err(PdfBuildError::InvalidPng);
    }
    let started = std::time::Instant::now();
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    let mut color = Vec::with_capacity(width * height * 3);
    let mut alpha = transparency.map(|_| Vec::with_capacity(width * height));
    let bits = metadata.bits_per_component;
    let mask = (1u16 << bits) - 1;
    for row in filtered.chunks_exact(row_bytes + 1) {
        unfilter_png_row(row[0], &row[1..], &previous, &mut current, 1)?;
        for pixel in 0..width {
            let bit = pixel * usize::from(bits);
            let shift = 8 - usize::from(bits) - (bit % 8);
            let index = usize::from((u16::from(current[bit / 8]) >> shift) & mask);
            let start = index.checked_mul(3).ok_or(PdfBuildError::InvalidPng)?;
            color.extend_from_slice(
                palette
                    .get(start..start + 3)
                    .ok_or(PdfBuildError::InvalidPng)?,
            );
            if let Some(alpha) = &mut alpha {
                alpha.push(
                    transparency
                        .and_then(|values| values.get(index))
                        .copied()
                        .unwrap_or(255),
                );
            }
        }
        std::mem::swap(&mut previous, &mut current);
    }
    telemetry.transform_ns += started.elapsed().as_nanos();
    let started = std::time::Instant::now();
    let streams = (zlib(&color)?, alpha.map(|data| zlib(&data)).transpose()?);
    telemetry.encode_ns += started.elapsed().as_nanos();
    Ok(streams)
}

fn png_chunk<'a>(bytes: &'a [u8], wanted: &[u8; 4]) -> Option<&'a [u8]> {
    let mut cursor = 8usize;
    while cursor + 12 <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]) as usize;
        let end = cursor.checked_add(length + 12)?;
        if end > bytes.len() {
            return None;
        }
        if &bytes[cursor + 4..cursor + 8] == wanted {
            return Some(&bytes[cursor + 8..cursor + 8 + length]);
        }
        cursor = end;
    }
    None
}

fn unfilter_png_row(
    filter: u8,
    source: &[u8],
    previous: &[u8],
    target: &mut [u8],
    bytes_per_pixel: usize,
) -> Result<(), PdfBuildError> {
    for index in 0..source.len() {
        let left = index.checked_sub(bytes_per_pixel).map_or(0, |i| target[i]);
        let up = previous[index];
        let upper_left = index
            .checked_sub(bytes_per_pixel)
            .map_or(0, |i| previous[i]);
        target[index] = source[index].wrapping_add(match filter {
            0 => 0,
            1 => left,
            2 => up,
            3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
            4 => paeth(left, up, upper_left),
            _ => return Err(PdfBuildError::InvalidPng),
        });
    }
    Ok(())
}

fn paeth(left: u8, up: u8, upper_left: u8) -> u8 {
    let left = i32::from(left);
    let up = i32::from(up);
    let upper_left = i32::from(upper_left);
    let estimate = left + up - upper_left;
    let left_distance = (estimate - left).abs();
    let up_distance = (estimate - up).abs();
    let upper_left_distance = (estimate - upper_left).abs();
    if left_distance <= up_distance && left_distance <= upper_left_distance {
        left as u8
    } else if up_distance <= upper_left_distance {
        up as u8
    } else {
        upper_left as u8
    }
}

fn zlib(bytes: &[u8]) -> Result<Vec<u8>, PdfBuildError> {
    // Generated image planes retain PNG prediction, so fast deflate bounds
    // finalization latency without discarding useful source compression structure.
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder
        .write_all(bytes)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    encoder.finish().map_err(|_| PdfBuildError::InvalidPng)
}

struct ImportedPdfPage {
    form: PdfIndirectObject,
    dependencies: Vec<PdfIndirectObject>,
    group: Option<PdfObjectId>,
}

// Imported page resources are attacker-controlled input. Keep a per-stream
// ceiling below the detached document's aggregate 1 GiB stream budget so a
// single pass-through image cannot consume the whole finalization allowance.
const MAX_IMPORTED_PDF_STREAM_BYTES: usize = 256 * 1024 * 1024;

fn import_pdf_page(
    image: &tex_state::PdfExternalImageRecord,
    page: u32,
    page_box: tex_state::PdfPageBox,
    rotation: tex_state::PdfPageRotation,
    next_object: &mut u32,
) -> Result<ImportedPdfPage, PdfBuildError> {
    let document = lopdf::Document::load_mem(image.bytes())
        .map_err(|error| PdfBuildError::InvalidPdfPage(error.to_string()))?;
    let page_id = document
        .get_pages()
        .get(&page)
        .copied()
        .ok_or_else(|| PdfBuildError::InvalidPdfPage(format!("page {page} does not exist")))?;
    let mut data = Vec::new();
    for content_id in document.get_page_contents(page_id) {
        let stream = document
            .get_object(content_id)
            .and_then(lopdf::Object::as_stream)
            .map_err(|error| PdfBuildError::InvalidPdfPage(error.to_string()))?;
        let remaining = MAX_IMPORTED_PDF_STREAM_BYTES
            .checked_sub(data.len())
            .and_then(|remaining| remaining.checked_sub(1))
            .ok_or_else(|| {
                imported_pdf_stream_limit_error("page content", MAX_IMPORTED_PDF_STREAM_BYTES)
            })?;
        data.extend_from_slice(&decompress_lopdf_stream_bounded(stream, remaining)?);
        data.push(b'\n');
    }
    let (direct_resources, resource_ids) = document
        .get_page_resources(page_id)
        .map_err(|error| PdfBuildError::InvalidPdfPage(error.to_string()))?;
    let mut dependencies = Vec::new();
    let mut imported = BTreeMap::new();
    let resources = if let Some(resources) = direct_resources {
        convert_lopdf_dictionary(
            &document,
            resources,
            next_object,
            &mut imported,
            &mut dependencies,
        )?
    } else if let Some(resource_id) = resource_ids.first() {
        let resources = document
            .get_dictionary(*resource_id)
            .map_err(|error| PdfBuildError::InvalidPdfPage(error.to_string()))?;
        convert_lopdf_dictionary(
            &document,
            resources,
            next_object,
            &mut imported,
            &mut dependencies,
        )?
    } else {
        PdfDictionary::new()
    };
    let page_dictionary = document
        .get_dictionary(page_id)
        .map_err(|error| PdfBuildError::InvalidPdfPage(error.to_string()))?;
    let group = page_dictionary
        .get(b"Group")
        .ok()
        .map(|value| {
            let value = convert_lopdf_value(
                &document,
                value,
                next_object,
                &mut imported,
                &mut dependencies,
            )?;
            match value {
                PdfValue::Reference(id) => Ok(id),
                PdfValue::Dictionary(dictionary) => {
                    let id = allocate_output_object(next_object)?;
                    dependencies.push(indirect_dictionary(id, dictionary));
                    Ok(id)
                }
                _ => Err(PdfBuildError::InvalidPdfPage(
                    "page Group is not a dictionary".to_owned(),
                )),
            }
        })
        .transpose()?;
    let mut dictionary = PdfDictionary::new();
    dictionary.insert("FormType", PdfValue::Integer(1))?;
    dictionary.insert("Resources", PdfValue::Dictionary(resources))?;
    if let Some(group) = group {
        dictionary.insert("Group", PdfValue::Reference(group))?;
    }
    let zero = PdfNumber::new(0, 0)?;
    let one = PdfNumber::new(1, 0)?;
    let width = page_box
        .right
        .checked_sub(page_box.left)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let height = page_box
        .top
        .checked_sub(page_box.bottom)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let width_bp = scaled_to_bp_f32(width, 4);
    let height_bp = scaled_to_bp_f32(height, 4);
    if width_bp <= 0.0 || height_bp <= 0.0 {
        return Err(PdfBuildError::InvalidPdfPage(
            "selected page box is empty".to_owned(),
        ));
    }
    let left_bp = scaled_to_bp_f32(page_box.left, 4);
    let bottom_bp = scaled_to_bp_f32(page_box.bottom, 4);
    let (form_width, form_height, matrix) = match rotation {
        tex_state::PdfPageRotation::None => {
            (width, height, [1.0, 0.0, 0.0, 1.0, -left_bp, -bottom_bp])
        }
        tex_state::PdfPageRotation::Clockwise90 => (
            height,
            width,
            [0.0, 1.0, -1.0, 0.0, height_bp + bottom_bp, -left_bp],
        ),
        tex_state::PdfPageRotation::UpsideDown => (
            width,
            height,
            [
                -1.0,
                0.0,
                0.0,
                -1.0,
                width_bp + left_bp,
                height_bp + bottom_bp,
            ],
        ),
        tex_state::PdfPageRotation::Clockwise270 => (
            height,
            width,
            [0.0, -1.0, 1.0, 0.0, -bottom_bp, width_bp + left_bp],
        ),
    };
    let [a, b, c, d, e, f] = matrix;
    let matrix = [
        pdf_number_from_f32(a)?,
        pdf_number_from_f32(b)?,
        pdf_number_from_f32(c)?,
        pdf_number_from_f32(d)?,
        pdf_number_from_f32(e)?,
        pdf_number_from_f32(f)?,
    ];
    Ok(ImportedPdfPage {
        form: PdfIndirectObject {
            id: object_id(image.id().raw())?,
            object: PdfObject::FormXObject {
                dictionary,
                data,
                bbox: [
                    zero,
                    zero,
                    scaled_to_bp_number(form_width, 4)?,
                    scaled_to_bp_number(form_height, 4)?,
                ],
                matrix: Some(matrix).filter(|matrix| *matrix != [one, zero, zero, one, zero, zero]),
            },
        },
        dependencies,
        group,
    })
}

fn allocate_output_object(next_object: &mut u32) -> Result<PdfObjectId, PdfBuildError> {
    let id = object_id(*next_object)?;
    *next_object = next_object
        .checked_add(1)
        .ok_or(PdfBuildError::ObjectCapacity)?;
    Ok(id)
}

fn convert_lopdf_dictionary(
    document: &lopdf::Document,
    source: &lopdf::Dictionary,
    next_object: &mut u32,
    imported: &mut BTreeMap<lopdf::ObjectId, PdfObjectId>,
    objects: &mut Vec<PdfIndirectObject>,
) -> Result<PdfDictionary, PdfBuildError> {
    let mut dictionary = PdfDictionary::new();
    for (name, value) in source.iter() {
        if matches!(name.as_slice(), b"Length" | b"Filter" | b"DecodeParms") {
            continue;
        }
        dictionary.insert(
            PdfName::new(name.clone()),
            convert_lopdf_value(document, value, next_object, imported, objects)?,
        )?;
    }
    Ok(dictionary)
}

fn convert_lopdf_value(
    document: &lopdf::Document,
    source: &lopdf::Object,
    next_object: &mut u32,
    imported: &mut BTreeMap<lopdf::ObjectId, PdfObjectId>,
    objects: &mut Vec<PdfIndirectObject>,
) -> Result<PdfValue, PdfBuildError> {
    Ok(match source {
        lopdf::Object::Null => PdfValue::Null,
        lopdf::Object::Boolean(value) => PdfValue::Bool(*value),
        lopdf::Object::Integer(value) => PdfValue::Integer(*value),
        lopdf::Object::Real(value) => PdfValue::Number(pdf_number_from_f32(*value)?),
        lopdf::Object::Name(name) => PdfValue::Name(PdfName::new(name.clone())),
        lopdf::Object::String(bytes, _) => PdfValue::String(bytes.clone()),
        lopdf::Object::Array(values) => PdfValue::Array(
            values
                .iter()
                .map(|value| convert_lopdf_value(document, value, next_object, imported, objects))
                .collect::<Result<_, _>>()?,
        ),
        lopdf::Object::Dictionary(dictionary) => PdfValue::Dictionary(convert_lopdf_dictionary(
            document,
            dictionary,
            next_object,
            imported,
            objects,
        )?),
        lopdf::Object::Reference(source_id) => PdfValue::Reference(import_lopdf_indirect(
            document,
            *source_id,
            next_object,
            imported,
            objects,
        )?),
        lopdf::Object::Stream(_) => {
            return Err(PdfBuildError::InvalidPdfPage(
                "direct resource streams are unsupported".to_owned(),
            ));
        }
    })
}

fn import_lopdf_indirect(
    document: &lopdf::Document,
    source_id: lopdf::ObjectId,
    next_object: &mut u32,
    imported: &mut BTreeMap<lopdf::ObjectId, PdfObjectId>,
    objects: &mut Vec<PdfIndirectObject>,
) -> Result<PdfObjectId, PdfBuildError> {
    if let Some(id) = imported.get(&source_id) {
        return Ok(*id);
    }
    let id = allocate_output_object(next_object)?;
    imported.insert(source_id, id);
    let source = document
        .get_object(source_id)
        .map_err(|error| PdfBuildError::InvalidPdfPage(error.to_string()))?;
    let object = match source {
        lopdf::Object::Stream(stream) => {
            if let Some(image) =
                import_lopdf_dct_image(document, stream, next_object, imported, objects)?
            {
                image
            } else {
                let data = decompress_lopdf_stream_bounded(stream, MAX_IMPORTED_PDF_STREAM_BYTES)?;
                PdfObject::Stream {
                    dictionary: convert_lopdf_dictionary(
                        document,
                        &stream.dict,
                        next_object,
                        imported,
                        objects,
                    )?,
                    data,
                }
            }
        }
        value => PdfObject::Value(convert_lopdf_value(
            document,
            value,
            next_object,
            imported,
            objects,
        )?),
    };
    objects.push(PdfIndirectObject { id, object });
    Ok(id)
}

fn imported_pdf_stream_limit_error(kind: &str, limit: usize) -> PdfBuildError {
    PdfBuildError::InvalidPdfPage(format!("{kind} stream exceeds {limit} bytes"))
}

fn decompress_lopdf_stream_bounded(
    stream: &lopdf::Stream,
    limit: usize,
) -> Result<Vec<u8>, PdfBuildError> {
    let filters = match stream.filters() {
        Ok(filters) => filters,
        Err(_) if stream.dict.get(b"Filter").is_err() => {
            if stream.content.len() > limit {
                return Err(imported_pdf_stream_limit_error("uncompressed", limit));
            }
            return Ok(stream.content.clone());
        }
        Err(error) => return Err(PdfBuildError::InvalidPdfPage(error.to_string())),
    };
    if filters.as_slice() != [b"FlateDecode".as_slice()] {
        return Err(PdfBuildError::InvalidPdfPage(format!(
            "resource stream filter {} is unsupported",
            filters
                .iter()
                .map(|name| String::from_utf8_lossy(name))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let decode_parameters = flate_decode_parameters(stream)?;
    if let Some(parameters) = decode_parameters {
        return decompress_png_predictor_bounded(stream, parameters, limit);
    }
    let read_limit = u64::try_from(limit)
        .ok()
        .and_then(|limit| limit.checked_add(1))
        .unwrap_or(u64::MAX);
    let mut decoder = flate2::read::ZlibDecoder::new(stream.content.as_slice()).take(read_limit);
    let mut data = Vec::new();
    decoder
        .read_to_end(&mut data)
        .map_err(|error| PdfBuildError::InvalidPdfPage(error.to_string()))?;
    if data.len() > limit {
        return Err(imported_pdf_stream_limit_error("decompressed", limit));
    }
    Ok(data)
}

#[derive(Clone, Copy)]
struct FlatePngPredictor {
    row_bytes: usize,
    bytes_per_pixel: usize,
}

fn flate_decode_parameters(
    stream: &lopdf::Stream,
) -> Result<Option<FlatePngPredictor>, PdfBuildError> {
    let parameters = match stream.dict.get(b"DecodeParms") {
        Err(_) | Ok(lopdf::Object::Null) => return Ok(None),
        Ok(lopdf::Object::Dictionary(parameters)) => parameters,
        Ok(lopdf::Object::Array(parameters)) if parameters.len() == 1 => match &parameters[0] {
            lopdf::Object::Null => return Ok(None),
            lopdf::Object::Dictionary(parameters) => parameters,
            _ => {
                return Err(invalid_flate_decode_parameters(
                    "array entry is not a dictionary",
                ));
            }
        },
        Ok(_) => return Err(invalid_flate_decode_parameters("value is not a dictionary")),
    };
    for (name, _) in parameters.iter() {
        if !matches!(
            name.as_slice(),
            b"Predictor" | b"Colors" | b"BitsPerComponent" | b"Columns"
        ) {
            return Err(invalid_flate_decode_parameters(&format!(
                "unsupported key {}",
                String::from_utf8_lossy(name)
            )));
        }
    }
    let integer = |name: &'static [u8], default: i64| -> Result<i64, PdfBuildError> {
        match parameters.get(name) {
            Ok(value) => value.as_i64().map_err(|_| {
                invalid_flate_decode_parameters(&format!(
                    "{} is not an integer",
                    String::from_utf8_lossy(name)
                ))
            }),
            Err(_) => Ok(default),
        }
    };
    let predictor = integer(b"Predictor", 1)?;
    if predictor == 1 {
        return Ok(None);
    }
    if !(10..=15).contains(&predictor) {
        return Err(invalid_flate_decode_parameters(&format!(
            "predictor {predictor} is unsupported"
        )));
    }
    let colors = positive_usize(integer(b"Colors", 1)?, "Colors")?;
    let columns = positive_usize(integer(b"Columns", 1)?, "Columns")?;
    let bits_per_component = positive_usize(integer(b"BitsPerComponent", 8)?, "BitsPerComponent")?;
    if !matches!(bits_per_component, 1 | 2 | 4 | 8 | 16) {
        return Err(invalid_flate_decode_parameters(
            "BitsPerComponent must be 1, 2, 4, 8, or 16",
        ));
    }
    let bits_per_pixel = colors.checked_mul(bits_per_component).ok_or_else(|| {
        invalid_flate_decode_parameters("sample width overflows the supported range")
    })?;
    let row_bits = bits_per_pixel.checked_mul(columns).ok_or_else(|| {
        invalid_flate_decode_parameters("row width overflows the supported range")
    })?;
    let bytes_per_pixel = bits_per_pixel
        .checked_add(7)
        .map(|bits| bits / 8)
        .ok_or_else(|| {
            invalid_flate_decode_parameters("sample width overflows the supported range")
        })?;
    let row_bytes = row_bits
        .checked_add(7)
        .map(|bits| bits / 8)
        .ok_or_else(|| {
            invalid_flate_decode_parameters("row width overflows the supported range")
        })?;
    Ok(Some(FlatePngPredictor {
        row_bytes,
        bytes_per_pixel,
    }))
}

fn positive_usize(value: i64, name: &str) -> Result<usize, PdfBuildError> {
    usize::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| invalid_flate_decode_parameters(&format!("{name} must be positive")))
}

fn invalid_flate_decode_parameters(detail: &str) -> PdfBuildError {
    PdfBuildError::InvalidPdfPage(format!(
        "Flate resource stream decode parameters are invalid: {detail}"
    ))
}

fn decompress_png_predictor_bounded(
    stream: &lopdf::Stream,
    parameters: FlatePngPredictor,
    limit: usize,
) -> Result<Vec<u8>, PdfBuildError> {
    if parameters.row_bytes > limit {
        return Err(imported_pdf_stream_limit_error("decompressed", limit));
    }
    let mut decoder = flate2::read::ZlibDecoder::new(stream.content.as_slice());
    let mut previous = vec![0_u8; parameters.row_bytes];
    let mut encoded = vec![0_u8; parameters.row_bytes];
    let mut decoded = vec![0_u8; parameters.row_bytes];
    let mut data = Vec::new();
    loop {
        let mut tag = [0_u8];
        let count = decoder
            .read(&mut tag)
            .map_err(|error| PdfBuildError::InvalidPdfPage(error.to_string()))?;
        if count == 0 {
            break;
        }
        decoder.read_exact(&mut encoded).map_err(|error| {
            invalid_flate_decode_parameters(&format!("PNG predictor row is truncated: {error}"))
        })?;
        let next_len = data
            .len()
            .checked_add(parameters.row_bytes)
            .ok_or_else(|| imported_pdf_stream_limit_error("decompressed", limit))?;
        if next_len > limit {
            return Err(imported_pdf_stream_limit_error("decompressed", limit));
        }
        decode_png_predictor_row(
            tag[0],
            parameters.bytes_per_pixel,
            &previous,
            &encoded,
            &mut decoded,
        )?;
        data.extend_from_slice(&decoded);
        std::mem::swap(&mut previous, &mut decoded);
    }
    Ok(data)
}

fn decode_png_predictor_row(
    tag: u8,
    bytes_per_pixel: usize,
    previous: &[u8],
    encoded: &[u8],
    decoded: &mut [u8],
) -> Result<(), PdfBuildError> {
    if tag > 4 {
        return Err(invalid_flate_decode_parameters(&format!(
            "PNG predictor row has invalid algorithm tag {tag}"
        )));
    }
    for index in 0..encoded.len() {
        let left = index
            .checked_sub(bytes_per_pixel)
            .map(|left| decoded[left])
            .unwrap_or(0);
        let above = previous[index];
        let upper_left = index
            .checked_sub(bytes_per_pixel)
            .map(|left| previous[left])
            .unwrap_or(0);
        let prediction = match tag {
            0 => 0,
            1 => left,
            2 => above,
            3 => ((u16::from(left) + u16::from(above)) / 2) as u8,
            4 => png_paeth(left, above, upper_left),
            _ => unreachable!("algorithm tag was range-checked"),
        };
        decoded[index] = encoded[index].wrapping_add(prediction);
    }
    Ok(())
}

fn png_paeth(left: u8, above: u8, upper_left: u8) -> u8 {
    let left = i32::from(left);
    let above = i32::from(above);
    let upper_left = i32::from(upper_left);
    let estimate = left + above - upper_left;
    let left_distance = (estimate - left).abs();
    let above_distance = (estimate - above).abs();
    let upper_left_distance = (estimate - upper_left).abs();
    if left_distance <= above_distance && left_distance <= upper_left_distance {
        left as u8
    } else if above_distance <= upper_left_distance {
        above as u8
    } else {
        upper_left as u8
    }
}

fn import_lopdf_dct_image(
    document: &lopdf::Document,
    stream: &lopdf::Stream,
    next_object: &mut u32,
    imported: &mut BTreeMap<lopdf::ObjectId, PdfObjectId>,
    objects: &mut Vec<PdfIndirectObject>,
) -> Result<Option<PdfObject>, PdfBuildError> {
    let filters = match stream.filters() {
        Ok(filters) => filters,
        Err(_) => return Ok(None),
    };
    if filters.as_slice() != [b"DCTDecode".as_slice()] {
        return Ok(None);
    }
    if stream.content.len() > MAX_IMPORTED_PDF_STREAM_BYTES {
        return Err(PdfBuildError::InvalidPdfPage(format!(
            "encoded image stream exceeds {} bytes",
            MAX_IMPORTED_PDF_STREAM_BYTES
        )));
    }
    let subtype = stream
        .dict
        .get(b"Subtype")
        .and_then(lopdf::Object::as_name)
        .map_err(|_| PdfBuildError::InvalidPdfPage("DCT stream has no image subtype".to_owned()))?;
    if subtype != b"Image" {
        return Err(PdfBuildError::InvalidPdfPage(
            "DCT resource stream is not an image".to_owned(),
        ));
    }
    if stream.dict.get(b"DecodeParms").is_ok()
        || stream.dict.get(b"Decode").is_ok()
        || stream.dict.get(b"Mask").is_ok()
    {
        return Err(PdfBuildError::InvalidPdfPage(
            "DCT image uses unsupported decode or mask parameters".to_owned(),
        ));
    }
    let dimension = |key: &'static [u8]| -> Result<u32, PdfBuildError> {
        let value = stream
            .dict
            .get(key)
            .and_then(lopdf::Object::as_i64)
            .map_err(|_| {
                PdfBuildError::InvalidPdfPage(format!(
                    "DCT image has no integer {}",
                    String::from_utf8_lossy(key)
                ))
            })?;
        u32::try_from(value)
            .ok()
            .filter(|value| *value != 0)
            .ok_or_else(|| {
                PdfBuildError::InvalidPdfPage(format!(
                    "DCT image has invalid {}",
                    String::from_utf8_lossy(key)
                ))
            })
    };
    let width = dimension(b"Width")?;
    let height = dimension(b"Height")?;
    let bits_per_component = u8::try_from(dimension(b"BitsPerComponent")?).map_err(|_| {
        PdfBuildError::InvalidPdfPage("DCT image bit depth is too large".to_owned())
    })?;
    let color_space = stream
        .dict
        .get(b"ColorSpace")
        .and_then(|value| document.dereference(value))
        .and_then(|(_, value)| value.as_name())
        .map_err(|_| {
            PdfBuildError::InvalidPdfPage("DCT image color space is invalid".to_owned())
        })?;
    let color_space = match color_space {
        b"DeviceGray" => PdfImageColorSpace::DeviceGray,
        b"DeviceRGB" => PdfImageColorSpace::DeviceRgb,
        b"DeviceCMYK" => PdfImageColorSpace::DeviceCmyk,
        _ => {
            return Err(PdfBuildError::InvalidPdfPage(format!(
                "DCT image color space {} is unsupported",
                String::from_utf8_lossy(color_space)
            )));
        }
    };
    let soft_mask = match stream.dict.get(b"SMask") {
        Ok(lopdf::Object::Reference(source_id)) => Some(import_lopdf_indirect(
            document,
            *source_id,
            next_object,
            imported,
            objects,
        )?),
        Ok(lopdf::Object::Name(name)) if name == b"None" => None,
        Err(_) => None,
        _ => {
            return Err(PdfBuildError::InvalidPdfPage(
                "DCT image soft mask is invalid".to_owned(),
            ));
        }
    };
    Ok(Some(PdfObject::ImageXObject {
        image: PdfImageXObject {
            width,
            height,
            bits_per_component,
            color_space,
            filter: PdfImageFilter::Dct,
            soft_mask,
        },
        data: stream.content.clone(),
    }))
}

fn pdf_number_from_f32(value: f32) -> Result<PdfNumber, PdfBuildError> {
    if !value.is_finite() {
        return Err(PdfBuildError::InvalidPdfPage(
            "page resource contains a non-finite number".to_owned(),
        ));
    }
    PdfNumber::new((f64::from(value) * 1_000_000_000.0).round() as i64, 9).map_err(Into::into)
}

fn output_parameters(stores: &Universe) -> PdfOutputParameters {
    stores.fixed_pdf_output_parameters().unwrap_or_else(|| {
        PdfOutputParameters {
            output: stores.int_param(IntParam::PDF_OUTPUT),
            major_version: stores.int_param(IntParam::PDF_MAJOR_VERSION),
            minor_version: stores.int_param(IntParam::PDF_MINOR_VERSION),
            compress_level: stores.int_param(IntParam::PDF_COMPRESS_LEVEL),
            object_compress_level: stores.int_param(IntParam::PDF_OBJ_COMPRESS_LEVEL),
            decimal_digits: stores.int_param(IntParam::PDF_DECIMAL_DIGITS),
            gamma: stores.int_param(IntParam::PDF_GAMMA),
            image_gamma: stores.int_param(IntParam::PDF_IMAGE_GAMMA),
            image_hicolor: stores.int_param(IntParam::PDF_IMAGE_HICOLOR),
            image_apply_gamma: stores.int_param(IntParam::PDF_IMAGE_APPLY_GAMMA),
            draft_mode: stores.int_param(IntParam::PDF_DRAFT_MODE),
            inclusion_copy_fonts: stores.int_param(IntParam::PDF_INCLUSION_COPY_FONTS),
            pk_resolution: stores.int_param(IntParam::PDF_PK_RESOLUTION),
            unique_resource_names: stores.int_param(IntParam::PDF_UNIQUE_RESNAME),
        }
        .normalized()
    })
}

fn pdf_version(parameters: PdfOutputParameters) -> Result<PdfVersion, PdfBuildError> {
    let major = u8::try_from(parameters.major_version)
        .map_err(|_| PdfBuildError::InvalidVersionParameters)?;
    let minor = u8::try_from(parameters.minor_version)
        .map_err(|_| PdfBuildError::InvalidVersionParameters)?;
    Ok(PdfVersion::new(major, minor)?)
}

fn serialization_options(
    parameters: PdfOutputParameters,
) -> Result<PdfSerializationOptions, PdfBuildError> {
    let level = parameters.compress_level;
    let stream_compression = match level {
        ..=0 => PdfStreamCompression::None,
        1..=9 => PdfStreamCompression::Flate { level: level as u8 },
        _ => return Err(PdfBuildError::InvalidCompressionLevel(level)),
    };
    let object_compression = match parameters.object_compress_level {
        0 => PdfObjectCompression::None,
        level @ 1..=3 => PdfObjectCompression::ObjectStreams { level: level as u8 },
        level => return Err(PdfBuildError::InvalidObjectCompressionLevel(level)),
    };
    Ok(PdfSerializationOptions {
        pretty: false,
        stream_compression,
        object_compression,
    })
}

fn object_id(raw: u32) -> Result<PdfObjectId, PdfBuildError> {
    PdfObjectId::new(raw).ok_or(PdfBuildError::InvalidObjectId(raw))
}

fn indirect_dictionary(id: PdfObjectId, dictionary: PdfDictionary) -> PdfIndirectObject {
    PdfIndirectObject {
        id,
        object: PdfObject::Value(PdfValue::Dictionary(dictionary)),
    }
}

fn pdf_page_extents(
    artifact: &tex_out::PageArtifact,
    record: tex_state::PdfPageRecord,
) -> Result<(Scaled, Scaled), PdfBuildError> {
    let root = match &artifact.root {
        PageNode::HList(root) | PageNode::VList(root) => root,
        _ => unreachable!("validated artifact root is a box"),
    };
    let h_offset = record
        .h_origin()
        .checked_add(artifact.job.h_offset)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let v_offset = record
        .v_origin()
        .checked_add(artifact.job.v_offset)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let width = if record.width().raw() == 0 {
        root.width
            .checked_add(h_offset)
            .and_then(|value| value.checked_add(h_offset))
            .ok_or(PdfBuildError::PageGeometryOverflow)?
    } else {
        record.width()
    };
    let height = if record.height().raw() == 0 {
        root.height
            .checked_add(root.depth)
            .and_then(|value| value.checked_add(v_offset))
            .and_then(|value| value.checked_add(v_offset))
            .ok_or(PdfBuildError::PageGeometryOverflow)?
    } else {
        record.height()
    };
    Ok((width, height))
}

fn token_list_bytes(stores: &Universe, id: TokenListId) -> Vec<u8> {
    let mut text = String::new();
    for &token in stores.tokens(id) {
        append_token_string_text(stores, token, &mut text);
    }
    text.into_bytes()
}

fn document_fragment_bytes(stores: &Universe, kind: PdfDocumentFragmentKind) -> Vec<u8> {
    let mut bytes = Vec::new();
    for tokens in stores.pdf_document_fragments(kind) {
        bytes.extend_from_slice(&token_list_bytes(stores, tokens));
    }
    bytes
}

fn scaled_to_bp_f32(value: Scaled, decimal_digits: i32) -> f32 {
    let scale = 10_f32.powi(decimal_digits);
    scaled_to_bp_coefficient(value, decimal_digits) as f32 / scale
}

fn scaled_to_bp_number(value: Scaled, decimal_digits: i32) -> Result<PdfNumber, PdfModelError> {
    PdfNumber::new(
        scaled_to_bp_coefficient(value, decimal_digits),
        decimal_digits as u8,
    )
}

fn scaled_to_bp_coefficient(value: Scaled, decimal_digits: i32) -> i64 {
    let scale = 10_i128.pow(decimal_digits as u32);
    const NUMERATOR: i128 = 7_200;
    const DENOMINATOR: i128 = 7_227 * 65_536;
    let numerator = i128::from(value.raw()) * NUMERATOR * scale;
    let rounded = if numerator >= 0 {
        (numerator + DENOMINATOR / 2) / DENOMINATOR
    } else {
        (numerator - DENOMINATOR / 2) / DENOMINATOR
    };
    rounded as i64
}

#[derive(Debug)]
pub enum PdfBuildError {
    PdfOutputDisabled,
    MissingArtifact(ContentHash),
    InvalidVersionParameters,
    InvalidCompressionLevel(i32),
    InvalidObjectCompressionLevel(i32),
    PageGeometryOverflow,
    InvalidObjectId(u32),
    ObjectCapacity,
    MissingAnnotationRecord(u32),
    UninitializedAnnotation(u32),
    MissingLinkRecord(u32),
    MissingOpenLink(u32),
    OpenActionPageNotFound(u32),
    OpenActionHasNoPage,
    ReferencedRawObjectUninitialized(u32),
    ReferencedFormNotFound(u32),
    MissingFormArtifact(u32),
    RecursiveForm(u32),
    InvalidRawObjectFileName(u32),
    TextRequiresFontResources,
    MissingPositionedFont(u32),
    MissingFontProgram(Vec<u8>),
    MissingFontResource(String),
    MissingFontUsage(String),
    PkFont(String),
    MissingPkFont(tex_fonts::PdfPkFontRequest),
    MissingPkGlyph { font: String, code: u8 },
    MissingEncoding(Vec<u8>),
    MissingSpaceFontName(u32),
    MissingBuiltinGlyphName { font: String, code: u8 },
    TrueTypeSubsetRequiresEncoding(String),
    Type1Subset(tex_fonts::PdfType1SubsetError),
    TrueTypeSubset(tex_fonts::PdfTrueTypeSubsetError),
    MissingLiveFont(String),
    VirtualFontDepthExceeded(usize),
    VirtualFontStackExceeded(usize),
    VirtualFontStackUnderflow,
    VirtualFontWorkExceeded(usize),
    VirtualFontOutputExceeded(usize),
    VirtualFontSpecialBytesExceeded(usize),
    VirtualFontCycle { font: String, code: u8 },
    MissingVirtualFontPacket { font: String, code: u32 },
    VirtualFontHasNoLocalFonts(String),
    MissingVirtualLocalFont { font: String, number: i32 },
    InvalidVirtualLocalFontName(String),
    MissingVirtualLocalTfm(String),
    InvalidVirtualLocalTfm { font: String, message: String },
    VirtualFontCharacterOutOfRange { font: String, code: u32 },
    MissingVirtualCharacter { font: String, code: u8 },
    VirtualFontArithmeticOverflow,
    UnsupportedSpecial(String),
    MissingRasterImage(u32),
    UnsupportedPdfPageImage(u32),
    InvalidRasterDimensions,
    InvalidPng,
    InvalidPdfPage(String),
    InvalidMatrix(Vec<u8>),
    World(WorldError),
    Parse(tex_out::ParseError),
    Positioned(PositionedError),
    Model(PdfModelError),
    Serialize(PdfSerializeError),
}

impl std::fmt::Display for PdfBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PdfOutputDisabled => {
                f.write_str("PDF output requires \\pdfoutput greater than zero")
            }
            Self::MissingArtifact(hash) => {
                write!(f, "shipped page artifact {} is missing", hash.hex())
            }
            Self::InvalidVersionParameters => {
                f.write_str("pdfTeX PDF version parameters are outside 0..=255")
            }
            Self::InvalidCompressionLevel(level) => {
                write!(f, "invalid \\pdfcompresslevel {level}; expected 0..=9")
            }
            Self::InvalidObjectCompressionLevel(level) => {
                write!(f, "invalid \\pdfobjcompresslevel {level}; expected 0..=3")
            }
            Self::PageGeometryOverflow => f.write_str("pdfTeX page geometry arithmetic overflowed"),
            Self::InvalidObjectId(id) => write!(f, "invalid PDF object id {id}"),
            Self::ObjectCapacity => f.write_str("pdfTeX error (obj): too many PDF objects."),
            Self::MissingAnnotationRecord(id) => {
                write!(f, "shipped annotation references missing object {id}")
            }
            Self::UninitializedAnnotation(id) => {
                write!(f, "shipped annotation object {id} was never initialized")
            }
            Self::MissingLinkRecord(id) => {
                write!(f, "shipped link references missing object {id}")
            }
            Self::MissingOpenLink(id) => {
                write!(f, "shipped link end {id} has no active start")
            }
            Self::OpenActionPageNotFound(page) => {
                write!(f, "PDF open action references missing page {page}")
            }
            Self::OpenActionHasNoPage => {
                f.write_str("PDF open action destination requires at least one page")
            }
            Self::ReferencedRawObjectUninitialized(id) => {
                write!(
                    f,
                    "referenced PDF object {id} was reserved but never initialized"
                )
            }
            Self::ReferencedFormNotFound(id) => {
                write!(f, "referenced PDF form object {id} was not captured")
            }
            Self::MissingFormArtifact(id) => {
                write!(f, "PDF form {id} was referenced before traversal")
            }
            Self::RecursiveForm(id) => write!(f, "PDF form {id} recursively references itself"),
            Self::InvalidRawObjectFileName(id) => {
                write!(f, "PDF stream object {id} has a non-UTF-8 file name")
            }
            Self::TextRequiresFontResources => {
                f.write_str("PDF text output requires embedded font resources")
            }
            Self::MissingPositionedFont(font) => {
                write!(f, "positioned text references missing font resource {font}")
            }
            Self::MissingFontProgram(name) => write!(
                f,
                "PDF font program resource {:?} was not supplied",
                String::from_utf8_lossy(name)
            ),
            Self::MissingFontResource(name) => {
                write!(f, "PDF font {name:?} has no checkpointed resource identity")
            }
            Self::MissingFontUsage(name) => {
                write!(f, "PDF font {name:?} has no committed glyph-use projection")
            }
            Self::PkFont(message) => f.write_str(message),
            Self::MissingPkFont(request) => write!(
                f,
                "PK font resource {:?} at {} DPI in mode {:?} was not supplied",
                String::from_utf8_lossy(request.tex_name()),
                request.dpi(),
                String::from_utf8_lossy(request.mode()),
            ),
            Self::MissingPkGlyph { font, code } => {
                write!(f, "PK font {font:?} has no glyph for character code {code}")
            }
            Self::MissingEncoding(name) => write!(
                f,
                "PDF encoding resource {:?} was not supplied",
                String::from_utf8_lossy(name)
            ),
            Self::MissingSpaceFontName(id) => {
                write!(f, "PDF page references missing space-font name id {id}")
            }
            Self::MissingBuiltinGlyphName { font, code } => write!(
                f,
                "PDF font {font:?} has no built-in glyph name for character code {code}"
            ),
            Self::TrueTypeSubsetRequiresEncoding(name) => write!(
                f,
                "subset TrueType font {name:?} requires an explicit PDF encoding"
            ),
            Self::Type1Subset(error) => error.fmt(f),
            Self::TrueTypeSubset(error) => error.fmt(f),
            Self::MissingLiveFont(name) => {
                write!(f, "PDF artifact font {name:?} has no live metric source")
            }
            Self::VirtualFontDepthExceeded(limit) => {
                write!(f, "virtual-font recursion exceeds depth {limit}")
            }
            Self::VirtualFontStackExceeded(limit) => {
                write!(f, "virtual-font stack exceeds depth {limit}")
            }
            Self::VirtualFontStackUnderflow => f.write_str("virtual-font stack underflow"),
            Self::VirtualFontWorkExceeded(limit) => {
                write!(f, "virtual-font packet execution exceeds {limit} commands")
            }
            Self::VirtualFontOutputExceeded(limit) => {
                write!(f, "virtual-font lowering exceeds {limit} output operations")
            }
            Self::VirtualFontSpecialBytesExceeded(limit) => {
                write!(f, "virtual-font specials exceed {limit} bytes")
            }
            Self::VirtualFontCycle { font, code } => {
                write!(f, "virtual-font cycle at {font} character {code}")
            }
            Self::MissingVirtualFontPacket { font, code } => {
                write!(f, "virtual font {font} has no packet for character {code}")
            }
            Self::VirtualFontHasNoLocalFonts(font) => {
                write!(f, "virtual font {font} has no default local font")
            }
            Self::MissingVirtualLocalFont { font, number } => {
                write!(f, "virtual font {font} has no local font {number}")
            }
            Self::InvalidVirtualLocalFontName(font) => {
                write!(f, "virtual font {font} has a non-UTF-8 local font name")
            }
            Self::MissingVirtualLocalTfm(font) => {
                write!(f, "virtual font requires unavailable local TFM {font}")
            }
            Self::InvalidVirtualLocalTfm { font, message } => {
                write!(f, "local TFM {font} is invalid: {message}")
            }
            Self::VirtualFontCharacterOutOfRange { font, code } => {
                write!(
                    f,
                    "virtual font {font} references character {code} outside 0..=255"
                )
            }
            Self::MissingVirtualCharacter { font, code } => {
                write!(f, "virtual-font local font {font} has no character {code}")
            }
            Self::VirtualFontArithmeticOverflow => {
                f.write_str("virtual-font positioned arithmetic overflowed")
            }
            Self::UnsupportedSpecial(class) => {
                write!(f, "PDF output does not support special class {class:?}")
            }
            Self::MissingRasterImage(object) => write!(f, "PDF image object {object} is missing"),
            Self::UnsupportedPdfPageImage(object) => {
                write!(f, "PDF-page image object {object} is not lowered yet")
            }
            Self::InvalidRasterDimensions => {
                f.write_str("registered raster image has zero width or height")
            }
            Self::InvalidPng => f.write_str("registered PNG image data is invalid"),
            Self::InvalidPdfPage(message) => {
                write!(f, "registered PDF-page image is invalid: {message}")
            }
            Self::InvalidMatrix(payload) => write!(
                f,
                "invalid \\pdfsetmatrix payload {:?}; expected exactly four finite numbers",
                String::from_utf8_lossy(payload)
            ),
            Self::World(error) => error.fmt(f),
            Self::Parse(error) => error.fmt(f),
            Self::Positioned(error) => error.fmt(f),
            Self::Model(error) => error.fmt(f),
            Self::Serialize(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PdfBuildError {}

impl From<WorldError> for PdfBuildError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}
impl From<tex_out::ParseError> for PdfBuildError {
    fn from(value: tex_out::ParseError) -> Self {
        Self::Parse(value)
    }
}
impl From<PositionedError> for PdfBuildError {
    fn from(value: PositionedError) -> Self {
        Self::Positioned(value)
    }
}
impl From<PdfModelError> for PdfBuildError {
    fn from(value: PdfModelError) -> Self {
        Self::Model(value)
    }
}
impl From<PdfSerializeError> for PdfBuildError {
    fn from(value: PdfSerializeError) -> Self {
        Self::Serialize(value)
    }
}

impl From<tex_fonts::PdfType1SubsetError> for PdfBuildError {
    fn from(value: tex_fonts::PdfType1SubsetError) -> Self {
        Self::Type1Subset(value)
    }
}

impl From<tex_fonts::PdfTrueTypeSubsetError> for PdfBuildError {
    fn from(value: tex_fonts::PdfTrueTypeSubsetError) -> Self {
        Self::TrueTypeSubset(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DirectFontResolver, RejectingMemoryInputResolver, RunResult, dvi_from_page_plans,
        prepare_pdftex_run_stores, run_input_collecting_artifacts,
    };
    use lopdf::dictionary;
    use tex_exec::ExecutionContext;
    use tex_lex::{InputStack, MemoryInput};
    use tex_state::{JobClock, World};

    struct StaticImageResolver {
        source: tex_state::PdfExternalImageSource,
    }

    struct QueueImageResolver {
        sources: VecDeque<tex_state::PdfExternalImageSource>,
    }

    struct RecordingImageResolver {
        source: tex_state::PdfExternalImageSource,
        requests: Vec<tex_exec::PdfImageRequest>,
    }

    impl tex_exec::PdfImageResolver for RecordingImageResolver {
        fn open_image(
            &mut self,
            _input: &mut dyn tex_state::InputReadState,
            request: &tex_exec::PdfImageRequest,
            _request_index: u64,
        ) -> tex_expand::ResourceResult<tex_state::PdfExternalImageSource> {
            self.requests.push(request.clone());
            Ok(tex_expand::ResourceLookup::Available(self.source.clone()))
        }
    }

    impl tex_exec::PdfImageResolver for QueueImageResolver {
        fn open_image(
            &mut self,
            _input: &mut dyn tex_state::InputReadState,
            _request: &tex_exec::PdfImageRequest,
            _request_index: u64,
        ) -> tex_expand::ResourceResult<tex_state::PdfExternalImageSource> {
            self.sources
                .pop_front()
                .map(tex_expand::ResourceLookup::Available)
                .ok_or_else(|| "test image queue is empty".to_owned())
        }
    }

    impl tex_exec::PdfImageResolver for StaticImageResolver {
        fn open_image(
            &mut self,
            _input: &mut dyn tex_state::InputReadState,
            _request: &tex_exec::PdfImageRequest,
            _request_index: u64,
        ) -> tex_expand::ResourceResult<tex_state::PdfExternalImageSource> {
            Ok(tex_expand::ResourceLookup::Available(self.source.clone()))
        }
    }

    fn run_with_image(
        stores: &mut Universe,
        source: &str,
        image: tex_state::PdfExternalImageSource,
    ) -> RunResult {
        let mut input = InputStack::new(MemoryInput::new(source));
        let mut input_resolver = RejectingMemoryInputResolver;
        let mut font_resolver = DirectFontResolver;
        let mut image_resolver = StaticImageResolver { source: image };
        let context = ExecutionContext::with_resource_resolvers(
            "pdf-test",
            &mut input_resolver,
            &mut font_resolver,
            &mut image_resolver,
        );
        run_input_collecting_artifacts(&mut input, stores, context).expect("image page ships")
    }

    fn run_with_images(
        stores: &mut Universe,
        source: &str,
        images: impl IntoIterator<Item = tex_state::PdfExternalImageSource>,
    ) -> RunResult {
        let mut input = InputStack::new(MemoryInput::new(source));
        let mut input_resolver = RejectingMemoryInputResolver;
        let mut font_resolver = DirectFontResolver;
        let mut image_resolver = QueueImageResolver {
            sources: images.into_iter().collect(),
        };
        let context = ExecutionContext::with_resource_resolvers(
            "pdf-test",
            &mut input_resolver,
            &mut font_resolver,
            &mut image_resolver,
        );
        run_input_collecting_artifacts(&mut input, stores, context).expect("image page ships")
    }

    fn test_png(color_type: u8, scanline: &[u8]) -> Vec<u8> {
        fn chunk(kind: &[u8; 4], data: &[u8], target: &mut Vec<u8>) {
            target.extend_from_slice(&(data.len() as u32).to_be_bytes());
            target.extend_from_slice(kind);
            target.extend_from_slice(data);
            target.extend_from_slice(&[0; 4]);
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::new();
        header.extend_from_slice(&2u32.to_be_bytes());
        header.extend_from_slice(&1u32.to_be_bytes());
        header.extend_from_slice(&[8, color_type, 0, 0, 0]);
        chunk(b"IHDR", &header, &mut png);
        chunk(b"IDAT", &zlib(scanline).expect("compress PNG"), &mut png);
        chunk(b"IEND", &[], &mut png);
        png
    }

    fn test_gamma_png(scanline: &[u8], gamma: u32) -> Vec<u8> {
        fn chunk(kind: &[u8; 4], data: &[u8], target: &mut Vec<u8>) {
            target.extend_from_slice(&(data.len() as u32).to_be_bytes());
            target.extend_from_slice(kind);
            target.extend_from_slice(data);
            target.extend_from_slice(&[0; 4]);
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::new();
        header.extend_from_slice(&2u32.to_be_bytes());
        header.extend_from_slice(&1u32.to_be_bytes());
        header.extend_from_slice(&[8, 0, 0, 0, 0]);
        chunk(b"IHDR", &header, &mut png);
        chunk(b"gAMA", &gamma.to_be_bytes(), &mut png);
        chunk(
            b"IDAT",
            &zlib(scanline).expect("compress gamma PNG"),
            &mut png,
        );
        chunk(b"IEND", &[], &mut png);
        png
    }

    fn test_indexed_png() -> Vec<u8> {
        fn chunk(kind: &[u8; 4], data: &[u8], target: &mut Vec<u8>) {
            target.extend_from_slice(&(data.len() as u32).to_be_bytes());
            target.extend_from_slice(kind);
            target.extend_from_slice(data);
            target.extend_from_slice(&[0; 4]);
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::new();
        header.extend_from_slice(&2u32.to_be_bytes());
        header.extend_from_slice(&1u32.to_be_bytes());
        header.extend_from_slice(&[1, 3, 0, 0, 0]);
        chunk(b"IHDR", &header, &mut png);
        chunk(b"PLTE", &[255, 0, 0, 0, 0, 255], &mut png);
        chunk(b"tRNS", &[32, 224], &mut png);
        chunk(
            b"IDAT",
            &zlib(&[0, 0b0100_0000]).expect("compress indexed PNG"),
            &mut png,
        );
        chunk(b"IEND", &[], &mut png);
        png
    }

    fn test_pdf_page(has_group: bool) -> Vec<u8> {
        let mut document = lopdf::Document::with_version("1.5");
        let pages = document.new_object_id();
        let page = document.new_object_id();
        let contents = document.add_object(lopdf::Stream::new(
            lopdf::dictionary! {},
            b"0 0 10 20 re f".to_vec(),
        ));
        let mut page_dictionary = lopdf::dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 10.into(), 20.into()],
            "Resources" => lopdf::dictionary! {},
            "Contents" => contents,
        };
        if has_group {
            page_dictionary.set(
                "Group",
                lopdf::dictionary! {
                    "S" => "Transparency",
                    "CS" => "DeviceRGB",
                },
            );
        }
        document.objects.insert(page, page_dictionary.into());
        document.objects.insert(
            pages,
            lopdf::dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page.into()],
                "Count" => 1,
            }
            .into(),
        );
        let catalog = document.add_object(lopdf::dictionary! {
            "Type" => "Catalog",
            "Pages" => pages,
        });
        document.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        document
            .save_to(&mut bytes)
            .expect("serialize PDF-page fixture");
        bytes
    }

    fn test_pdf_page_with_dct_image() -> Vec<u8> {
        let mut document = lopdf::Document::with_version("1.5");
        let pages = document.new_object_id();
        let page = document.new_object_id();
        let image = document.add_object(lopdf::Stream::new(
            lopdf::dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 1,
                "Height" => 1,
                "BitsPerComponent" => 8,
                "ColorSpace" => "DeviceRGB",
                "Filter" => "DCTDecode",
            },
            b"bounded-pass-through-jpeg".to_vec(),
        ));
        let contents = document.add_object(lopdf::Stream::new(
            lopdf::dictionary! {},
            b"q /Im0 Do Q".to_vec(),
        ));
        document.objects.insert(
            page,
            lopdf::dictionary! {
                "Type" => "Page",
                "Parent" => pages,
                "MediaBox" => vec![0.into(), 0.into(), 10.into(), 20.into()],
                "Resources" => lopdf::dictionary! {
                    "XObject" => lopdf::dictionary! { "Im0" => image },
                },
                "Contents" => contents,
            }
            .into(),
        );
        document.objects.insert(
            pages,
            lopdf::dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page.into()],
                "Count" => 1,
            }
            .into(),
        );
        let catalog = document.add_object(lopdf::dictionary! {
            "Type" => "Catalog",
            "Pages" => pages,
        });
        document.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        document
            .save_to(&mut bytes)
            .expect("serialize DCT-image PDF-page fixture");
        bytes
    }

    fn test_pdf_page_source(has_group: bool) -> tex_state::PdfExternalImageSource {
        let bytes = test_pdf_page(has_group);
        let page_box = tex_state::PdfPageBox {
            left: Scaled::from_raw(0),
            bottom: Scaled::from_raw(0),
            right: Scaled::from_raw(10 * 65_536),
            top: Scaled::from_raw(20 * 65_536),
        };
        tex_state::PdfExternalImageSource {
            identity: ContentHash::from_bytes(&bytes),
            metadata: PdfExternalImageMetadata::PdfPage {
                page_box,
                rotation: tex_state::PdfPageRotation::None,
                page: 1,
                total_pages: 1,
                has_page_group: has_group,
                pdf_version: (1, 5),
            },
            natural_width: page_box.right,
            natural_height: page_box.top,
            bytes: bytes.into(),
        }
    }

    #[test]
    fn imported_pdf_page_applies_clockwise_quarter_turn_to_form_geometry() {
        let mut source = test_pdf_page_source(false);
        let PdfExternalImageMetadata::PdfPage {
            page_box,
            page,
            total_pages,
            has_page_group,
            pdf_version,
            ..
        } = source.metadata
        else {
            unreachable!();
        };
        source.metadata = PdfExternalImageMetadata::PdfPage {
            page_box,
            rotation: tex_state::PdfPageRotation::Clockwise90,
            page,
            total_pages,
            has_page_group,
            pdf_version,
        };
        source.natural_width = page_box.top;
        source.natural_height = page_box.right;
        let mut stores = Universe::default();
        stores.enable_pdf_output();
        stores
            .allocate_pdf_external_image(
                source,
                tex_state::PdfExternalImageDimensions {
                    width: page_box.top,
                    height: page_box.right,
                    depth: Scaled::from_raw(0),
                },
            )
            .expect("allocate rotated PDF page");
        let image = stores
            .pdf_external_images()
            .first()
            .expect("allocated image record");
        let mut next_object = 100;
        let imported = import_pdf_page(
            image,
            1,
            page_box,
            tex_state::PdfPageRotation::Clockwise90,
            &mut next_object,
        )
        .expect("import rotated PDF page");
        let PdfObject::FormXObject { bbox, matrix, .. } = imported.form.object else {
            panic!("expected imported form XObject");
        };
        assert_eq!(
            bbox,
            [
                PdfNumber::new(0, 0).expect("zero is a valid PDF number"),
                PdfNumber::new(0, 0).expect("zero is a valid PDF number"),
                scaled_to_bp_number(page_box.top, 4).expect("fixture height is representable"),
                scaled_to_bp_number(page_box.right, 4).expect("fixture width is representable"),
            ]
        );
        assert_eq!(
            matrix,
            Some([
                PdfNumber::new(0, 0).expect("zero is a valid PDF number"),
                PdfNumber::new(1, 0).expect("one is a valid PDF number"),
                PdfNumber::new(-1, 0).expect("negative one is a valid PDF number"),
                PdfNumber::new(0, 0).expect("zero is a valid PDF number"),
                pdf_number_from_f32(scaled_to_bp_f32(page_box.top, 4))
                    .expect("fixture height is representable"),
                PdfNumber::new(0, 0).expect("zero is a valid PDF number"),
            ])
        );
    }

    #[test]
    fn imported_pdf_page_preserves_dct_image_as_a_typed_bounded_stream() {
        let bytes = test_pdf_page_with_dct_image();
        let page_box = tex_state::PdfPageBox {
            left: Scaled::from_raw(0),
            bottom: Scaled::from_raw(0),
            right: Scaled::from_raw(10 * 65_536),
            top: Scaled::from_raw(20 * 65_536),
        };
        let source = tex_state::PdfExternalImageSource {
            identity: ContentHash::from_bytes(&bytes),
            metadata: PdfExternalImageMetadata::PdfPage {
                page_box,
                rotation: tex_state::PdfPageRotation::None,
                page: 1,
                total_pages: 1,
                has_page_group: false,
                pdf_version: (1, 5),
            },
            natural_width: page_box.right,
            natural_height: page_box.top,
            bytes: bytes.into(),
        };
        let mut stores = Universe::default();
        stores.enable_pdf_output();
        stores
            .allocate_pdf_external_image(
                source,
                tex_state::PdfExternalImageDimensions {
                    width: page_box.right,
                    height: page_box.top,
                    depth: Scaled::from_raw(0),
                },
            )
            .expect("allocate included page");
        let image = stores
            .pdf_external_images()
            .first()
            .expect("allocated image record");
        let imported = import_pdf_page(
            image,
            1,
            page_box,
            tex_state::PdfPageRotation::None,
            &mut 100,
        )
        .expect("import page with DCT image resource");
        let (_, data) = imported
            .dependencies
            .iter()
            .find_map(|object| match &object.object {
                PdfObject::ImageXObject { image, data } => Some((image, data)),
                _ => None,
            })
            .expect("typed image dependency");
        assert_eq!(data, b"bounded-pass-through-jpeg");
    }

    #[test]
    fn imported_pdf_stream_decompression_stops_at_the_caller_limit() {
        let compressed = zlib(&[b'x'; 65]).expect("compress fixture");
        let stream =
            lopdf::Stream::new(lopdf::dictionary! { "Filter" => "FlateDecode" }, compressed);
        let error = decompress_lopdf_stream_bounded(&stream, 64)
            .expect_err("expanded stream must respect the import ceiling");
        assert!(error.to_string().contains("exceeds 64 bytes"));
    }

    #[test]
    fn imported_pdf_stream_decodes_bounded_png_predictor_parameters() {
        let predicted = [
            1, 10, 20, 30, 40, 5, 5, 5, 5, // Sub
            2, 1, 1, 1, 1, 1, 1, 1, 1, // Up
        ];
        let compressed = zlib(&predicted).expect("compress predictor fixture");
        let stream = lopdf::Stream::new(
            lopdf::dictionary! {
                "Filter" => "FlateDecode",
                "DecodeParms" => lopdf::dictionary! {
                    "Predictor" => 15,
                    "Columns" => 2,
                    "Colors" => 4,
                },
            },
            compressed,
        );
        let decoded = decompress_lopdf_stream_bounded(&stream, 16)
            .expect("PNG predictor parameters are supported");
        assert_eq!(
            decoded,
            [
                10, 20, 30, 40, 15, 25, 35, 45, 11, 21, 31, 41, 16, 26, 36, 46
            ]
        );
    }

    #[test]
    fn imported_pdf_stream_rejects_malformed_or_unsupported_decode_parameters() {
        let cases = [
            (
                lopdf::dictionary! { "Predictor" => 2 },
                "predictor 2 is unsupported",
            ),
            (
                lopdf::dictionary! { "Predictor" => 15, "Colors" => 0 },
                "Colors must be positive",
            ),
            (
                lopdf::dictionary! { "Predictor" => 15, "BitsPerComponent" => 3 },
                "BitsPerComponent must be 1, 2, 4, 8, or 16",
            ),
            (
                lopdf::dictionary! { "Predictor" => 15, "EarlyChange" => 1 },
                "unsupported key EarlyChange",
            ),
        ];
        for (parameters, expected) in cases {
            let stream = lopdf::Stream::new(
                lopdf::dictionary! {
                    "Filter" => "FlateDecode",
                    "DecodeParms" => parameters,
                },
                zlib(&[0, 1]).expect("compress malformed-parameter fixture"),
            );
            let error = decompress_lopdf_stream_bounded(&stream, 16)
                .expect_err("invalid parameters must be rejected");
            assert!(error.to_string().contains(expected), "{error}");
        }

        for (predicted, expected) in [
            (&[5, 1, 2][..], "invalid algorithm tag 5"),
            (&[0, 1][..], "PNG predictor row is truncated"),
        ] {
            let stream = lopdf::Stream::new(
                lopdf::dictionary! {
                    "Filter" => "FlateDecode",
                    "DecodeParms" => lopdf::dictionary! {
                        "Predictor" => 15,
                        "Columns" => 2,
                    },
                },
                zlib(predicted).expect("compress malformed-row fixture"),
            );
            let error = decompress_lopdf_stream_bounded(&stream, 16)
                .expect_err("malformed predictor rows must be rejected");
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn imported_pdf_png_predictor_stops_at_the_caller_limit() {
        let stream = lopdf::Stream::new(
            lopdf::dictionary! {
                "Filter" => "FlateDecode",
                "DecodeParms" => lopdf::dictionary! {
                    "Predictor" => 15,
                    "Columns" => 4,
                },
            },
            zlib(&[0, 1, 2, 3, 4, 0, 5, 6, 7, 8]).expect("compress bounded predictor fixture"),
        );
        let error = decompress_lopdf_stream_bounded(&stream, 7)
            .expect_err("predictor output must respect the import ceiling");
        assert!(error.to_string().contains("exceeds 7 bytes"));
    }

    #[test]
    fn ximage_applies_resolution_and_obsolete_pagebox_controls_to_the_host_request() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(concat!(
            "\\pdfoutput=1 \\pdfimageresolution=144 ",
            "\\pdfoptionalwaysusepdfpagebox=4 ",
            "\\pdfoptionpdfinclusionerrorlevel=-1 ",
            "\\pdfximage mediabox \"page.pdf\"\\end",
        )));
        let mut input_resolver = RejectingMemoryInputResolver;
        let mut font_resolver = DirectFontResolver;
        let mut image_resolver = RecordingImageResolver {
            source: test_pdf_page_source(false),
            requests: Vec::new(),
        };
        let context = ExecutionContext::with_resource_resolvers(
            "pdf-test",
            &mut input_resolver,
            &mut font_resolver,
            &mut image_resolver,
        );
        run_input_collecting_artifacts(&mut input, &mut stores, context)
            .expect("configured image opens");

        assert_eq!(image_resolver.requests.len(), 1);
        let request = &image_resolver.requests[0];
        assert_eq!(request.resolution, 144);
        assert_eq!(request.page_box, tex_exec::PdfImagePageBox::Trim);
        assert_eq!(stores.int_param(IntParam::PDF_FORCE_PAGE_BOX), 4);
        assert_eq!(
            stores.int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
            0
        );
        assert_eq!(stores.int_param(IntParam::PDF_INCLUSION_ERROR_LEVEL), -1);
        assert_eq!(
            stores.int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL),
            0
        );
        let diagnostics = stores
            .world()
            .effect_records()
            .iter()
            .filter_map(|effect| match effect {
                tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        assert!(diagnostics.contains("\\pdfoptionalwaysusepdfpagebox is obsolete"));
        assert!(diagnostics.contains("\\pdfoptionpdfinclusionerrorlevel is obsolete"));
        assert!(!diagnostics.contains("Primitive \\pdfforcepagebox is obsolete"));
    }

    #[test]
    fn ximage_enforces_the_configured_pdf_inclusion_version_policy() {
        let image = test_pdf_page_source(false);
        let mut warning_stores = Universe::default();
        prepare_pdftex_run_stores(&mut warning_stores);
        run_with_image(
            &mut warning_stores,
            "\\pdfoutput=1 \\pdfinclusionerrorlevel=0 \\pdfximage \"page.pdf\"\\end",
            image.clone(),
        );
        assert!(
            warning_stores
                .world()
                .effect_records()
                .iter()
                .any(|effect| {
                    matches!(effect, tex_state::EffectRecord::StreamWrite { text, .. }
                if text.contains("found PDF version <1.5>, but at most version <1.4> allowed"))
                })
        );

        let mut fatal_stores = Universe::default();
        prepare_pdftex_run_stores(&mut fatal_stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\pdfoutput=1 \\pdfinclusionerrorlevel=1 \\pdfximage \"page.pdf\"\\end",
        ));
        let mut input_resolver = RejectingMemoryInputResolver;
        let mut font_resolver = DirectFontResolver;
        let mut image_resolver = StaticImageResolver { source: image };
        let context = ExecutionContext::with_resource_resolvers(
            "pdf-test",
            &mut input_resolver,
            &mut font_resolver,
            &mut image_resolver,
        );
        let error = run_input_collecting_artifacts(&mut input, &mut fatal_stores, context)
            .expect_err("positive inclusion error level rejects a newer PDF");
        assert!(
            error
                .to_string()
                .contains("found PDF version <1.5>, but at most version <1.4> allowed")
        );
    }

    #[test]
    fn raster_png_ximage_is_reused_and_emitted_through_typed_xobjects() {
        let png = test_png(2, &[0, 255, 0, 0, 0, 0, 255]);
        let identity = ContentHash::from_bytes(&png);
        let image = tex_state::PdfExternalImageSource {
            identity,
            metadata: PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
                format: PdfRasterFormat::Png,
                width: 2,
                height: 1,
                bits_per_component: 8,
                color_space: PdfRasterColorSpace::Rgb,
                alpha: false,
                png_color_type: Some(2),
            }),
            natural_width: Scaled::from_raw(2 * 65_536),
            natural_height: Scaled::from_raw(65_536),
            bytes: png.into(),
        };
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let result = run_with_image(
            &mut stores,
            concat!(
                "\\pdfoutput=1 \\pdfcompresslevel=0 \\pdfuniqueresname=1 ",
                "\\pdfximage width 20pt height 10pt \"pixel.png\"",
                "\\setbox0=\\hbox{\\pdfrefximage\\pdflastximage\\kern5pt",
                "\\pdfrefximage\\pdflastximage}",
                "\\shipout\\box0\\end",
            ),
            image,
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
            .expect("lower raster image");
        let parsed = lopdf::Document::load_mem(&pdf).expect("parse raster PDF");
        let stream = parsed
            .get_object((1, 0))
            .expect("image object")
            .as_stream()
            .expect("image stream");
        assert_eq!(
            stream
                .dict
                .get(b"Subtype")
                .expect("image subtype")
                .as_name()
                .expect("subtype name"),
            b"Image"
        );
        assert_eq!(
            stream
                .dict
                .get(b"Width")
                .expect("image width")
                .as_i64()
                .expect("integer width"),
            2
        );
        let page_id = parsed.get_pages()[&1];
        let content = parsed.get_page_content(page_id).expect("page content");
        let resource_use = format!("/{}Im1 Do", &identity.hex()[..6]);
        assert_eq!(
            content
                .windows(resource_use.len())
                .filter(|window| *window == resource_use.as_bytes())
                .count(),
            2
        );
    }

    #[test]
    fn rgba_png_ximage_uses_a_typed_soft_mask() {
        let png = test_png(6, &[0, 255, 0, 0, 64, 0, 0, 255, 192]);
        let image = tex_state::PdfExternalImageSource {
            identity: ContentHash::from_bytes(&png),
            metadata: PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
                format: PdfRasterFormat::Png,
                width: 2,
                height: 1,
                bits_per_component: 8,
                color_space: PdfRasterColorSpace::Rgb,
                alpha: true,
                png_color_type: Some(6),
            }),
            natural_width: Scaled::from_raw(2 * 65_536),
            natural_height: Scaled::from_raw(65_536),
            bytes: png.into(),
        };
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let result = run_with_image(
            &mut stores,
            "\\pdfoutput=1 \\pdfximage \"alpha.png\"\\shipout\\hbox{\\pdfrefximage\\pdflastximage}\\end",
            image,
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
            .expect("lower alpha image");
        let parsed = lopdf::Document::load_mem(&pdf).expect("parse alpha PDF");
        let image = parsed
            .get_object((1, 0))
            .expect("image object")
            .as_stream()
            .expect("image stream");
        assert_eq!(
            image
                .dict
                .get(b"SMask")
                .expect("soft-mask reference")
                .as_reference()
                .expect("indirect mask"),
            (2, 0)
        );
        let mask = parsed
            .get_object((2, 0))
            .expect("mask object")
            .as_stream()
            .expect("mask stream");
        assert_eq!(
            mask.dict
                .get(b"ColorSpace")
                .expect("mask color space")
                .as_name()
                .expect("color-space name"),
            b"DeviceGray"
        );
        assert_eq!(
            mask.decompressed_content().expect("alpha samples"),
            vec![64, 192]
        );
    }

    #[test]
    fn repeated_rgba_content_shares_one_image_and_mask_pair() {
        let png = test_png(6, &[0, 255, 0, 0, 64, 0, 0, 255, 192]);
        let image = tex_state::PdfExternalImageSource {
            identity: ContentHash::from_bytes(&png),
            metadata: PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
                format: PdfRasterFormat::Png,
                width: 2,
                height: 1,
                bits_per_component: 8,
                color_space: PdfRasterColorSpace::Rgb,
                alpha: true,
                png_color_type: Some(6),
            }),
            natural_width: Scaled::from_raw(2 * 65_536),
            natural_height: Scaled::from_raw(65_536),
            bytes: png.into(),
        };
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let result = run_with_images(
            &mut stores,
            concat!(
                "\\pdfoutput=1 ",
                "\\pdfximage \"first.png\"\\edef\\first{\\the\\pdflastximage}",
                "\\pdfximage \"second.png\"",
                "\\shipout\\hbox{\\pdfrefximage\\first\\pdfrefximage\\pdflastximage}\\end",
            ),
            [image.clone(), image],
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
            .expect("lower repeated alpha image");
        let parsed = lopdf::Document::load_mem(&pdf).expect("parse repeated-image PDF");
        let image_objects = parsed
            .objects
            .values()
            .filter(|object| {
                object.as_stream().ok().is_some_and(|stream| {
                    stream
                        .dict
                        .get(b"Subtype")
                        .ok()
                        .and_then(|value| value.as_name().ok())
                        == Some(b"Image")
                })
            })
            .count();
        assert_eq!(image_objects, 2, "one color image and one shared mask");

        let page_id = parsed.get_pages()[&1];
        let (direct_resources, resource_ids) =
            parsed.get_page_resources(page_id).expect("page resources");
        let resources = direct_resources.unwrap_or_else(|| {
            parsed
                .get_dictionary(resource_ids[0])
                .expect("indirect resources")
        });
        let xobjects = resources
            .get(b"XObject")
            .expect("XObject dictionary")
            .as_dict()
            .expect("direct XObject dictionary");
        let references = xobjects
            .iter()
            .map(|(_, value)| value.as_reference().expect("image reference"))
            .collect::<BTreeSet<_>>();
        assert_eq!(references.len(), 1, "both resource names share one object");
    }

    #[test]
    fn png_gamma_controls_match_the_pinned_pdftex_sample_oracle() {
        let source_samples = [
            0, 0, 1, 17, 34, 51, 68, 85, 102, 119, 136, 153, 170, 187, 204, 221, 238, 255,
        ];
        let png = test_gamma_png(&source_samples, 50_000);
        let source = tex_state::PdfExternalImageSource {
            identity: ContentHash::from_bytes(&png),
            metadata: PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
                format: PdfRasterFormat::Png,
                width: 17,
                height: 1,
                bits_per_component: 8,
                color_space: PdfRasterColorSpace::Gray,
                alpha: false,
                png_color_type: Some(0),
            }),
            natural_width: Scaled::from_raw(17 * 65_536),
            natural_height: Scaled::from_raw(65_536),
            bytes: png.into(),
        };
        for (apply, expected) in [
            (0, source_samples[1..].to_vec()),
            (
                1,
                vec![
                    0, 0, 1, 5, 10, 18, 28, 41, 56, 73, 92, 113, 137, 163, 192, 222, 255,
                ],
            ),
        ] {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            let tex = format!(
                concat!(
                    "\\pdfoutput=1 \\pdfgamma=1000 \\pdfimagegamma=2200 ",
                    "\\pdfimageapplygamma={apply} \\pdfximage \"gamma.png\"",
                    "\\shipout\\hbox{{\\pdfrefximage\\pdflastximage}}\\end",
                ),
                apply = apply,
            );
            let result = run_with_image(&mut stores, &tex, source.clone());
            let pdf = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
                .expect("lower gamma-controlled PNG");
            let parsed = lopdf::Document::load_mem(&pdf).expect("parse gamma PDF");
            let image = parsed
                .get_object((1, 0))
                .expect("gamma image")
                .as_stream()
                .expect("gamma stream");
            assert_eq!(
                image.decompressed_content().expect("controlled samples"),
                expected,
                "\\pdfimageapplygamma={apply}",
            );
        }
    }

    #[test]
    fn png_hicolor_control_and_pdf_version_match_pdftex_sixteen_bit_policy() {
        let mut png = test_gamma_png(&[0, 0x12, 0x34], 100_000);
        png[24] = 16;
        let metadata = tex_state::PdfRasterImageMetadata {
            format: PdfRasterFormat::Png,
            width: 1,
            height: 1,
            bits_per_component: 16,
            color_space: PdfRasterColorSpace::Gray,
            alpha: false,
            png_color_type: Some(0),
        };
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        for (hicolor, minor_version, expected_bits, expected_samples) in [
            (0, 5, 8, vec![0x12]),
            (1, 4, 8, vec![0x12]),
            (1, 5, 16, vec![0x12, 0x34]),
        ] {
            let mut parameters = output_parameters(&stores);
            parameters.minor_version = minor_version;
            parameters.image_hicolor = hicolor;
            parameters.image_apply_gamma = 0;

            let (samples, filter, bits, color_space, alpha) = raster_image_streams(
                &png,
                metadata,
                parameters,
                &mut ImageImportTelemetry::default(),
            )
            .expect("transform 16-bit PNG");
            assert_eq!(color_space, PdfImageColorSpace::DeviceGray);
            assert!(alpha.is_none());
            assert_eq!(
                bits, expected_bits,
                "hicolor={hicolor}, PDF 1.{minor_version}"
            );
            let samples = match filter {
                PdfImageFilter::Flate => inflate(&samples).expect("inflate transformed samples"),
                PdfImageFilter::FlatePngPredictor { .. } => {
                    png_opaque_samples(&png, metadata).expect("decode retained samples")
                }
                PdfImageFilter::Dct => panic!("PNG cannot use DCT"),
            };
            assert_eq!(
                samples, expected_samples,
                "hicolor={hicolor}, PDF 1.{minor_version}",
            );
        }
    }

    #[test]
    fn indexed_png_expands_palette_and_transparency() {
        let png = test_indexed_png();
        let image = tex_state::PdfExternalImageSource {
            identity: ContentHash::from_bytes(&png),
            metadata: PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
                format: PdfRasterFormat::Png,
                width: 2,
                height: 1,
                bits_per_component: 1,
                color_space: PdfRasterColorSpace::Rgb,
                alpha: true,
                png_color_type: Some(3),
            }),
            natural_width: Scaled::from_raw(2 * 65_536),
            natural_height: Scaled::from_raw(65_536),
            bytes: png.into(),
        };
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let result = run_with_image(
            &mut stores,
            "\\pdfoutput=1 \\pdfximage \"indexed.png\"\\shipout\\hbox{\\pdfrefximage\\pdflastximage}\\end",
            image,
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
            .expect("lower indexed image");
        let parsed = lopdf::Document::load_mem(&pdf).expect("parse indexed-image PDF");
        let color = parsed
            .get_object((1, 0))
            .expect("color image")
            .as_stream()
            .expect("color stream");
        assert_eq!(
            color.decompressed_content().expect("color samples"),
            vec![255, 0, 0, 0, 0, 255]
        );
        let alpha = parsed
            .get_object((2, 0))
            .expect("alpha image")
            .as_stream()
            .expect("alpha stream");
        assert_eq!(
            alpha.decompressed_content().expect("alpha samples"),
            vec![32, 224]
        );
    }

    #[test]
    fn jpeg_bytes_are_preserved_behind_a_typed_dct_filter() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0xd9];
        let image = tex_state::PdfExternalImageSource {
            identity: ContentHash::from_bytes(&jpeg),
            metadata: PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
                format: PdfRasterFormat::Jpeg,
                width: 1,
                height: 1,
                bits_per_component: 8,
                color_space: PdfRasterColorSpace::Rgb,
                alpha: false,
                png_color_type: None,
            }),
            natural_width: Scaled::from_raw(65_536),
            natural_height: Scaled::from_raw(65_536),
            bytes: jpeg.clone().into(),
        };
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let result = run_with_image(
            &mut stores,
            "\\pdfoutput=1 \\pdfximage \"pixel.jpg\"\\shipout\\hbox{\\pdfrefximage\\pdflastximage}\\end",
            image,
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
            .expect("lower JPEG image");
        let parsed = lopdf::Document::load_mem(&pdf).expect("parse JPEG PDF");
        let stream = parsed
            .get_object((1, 0))
            .expect("JPEG object")
            .as_stream()
            .expect("JPEG stream");
        assert_eq!(
            stream
                .dict
                .get(b"Filter")
                .expect("JPEG filter")
                .as_name()
                .expect("filter name"),
            b"DCTDecode"
        );
        assert_eq!(stream.content, jpeg);
    }

    #[test]
    fn pdf_page_ximage_is_a_reused_typed_form_with_shared_page_group() {
        let image = test_pdf_page_source(true);
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let result = run_with_image(
            &mut stores,
            concat!(
                "\\pdfoutput=1 \\pdfcompresslevel=0 ",
                "\\pdfximage width 30pt height 40pt page 1 mediabox \"page.pdf\"",
                "\\shipout\\hbox{\\pdfrefximage\\pdflastximage}\\end",
            ),
            image,
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
            .expect("lower PDF-page image");
        let parsed = lopdf::Document::load_mem(&pdf).expect("parse included-page PDF");
        let form = parsed
            .get_object((1, 0))
            .expect("included form")
            .as_stream()
            .expect("form stream");
        assert_eq!(
            form.dict
                .get(b"Subtype")
                .expect("form subtype")
                .as_name()
                .expect("subtype name"),
            b"Form"
        );
        let form_group = form
            .dict
            .get(b"Group")
            .expect("form group")
            .as_reference()
            .expect("group reference");
        let page_id = parsed.get_pages()[&1];
        let page = parsed.get_dictionary(page_id).expect("output page");
        assert_eq!(
            page.get(b"Group")
                .expect("output page group")
                .as_reference()
                .expect("group reference"),
            form_group
        );
        let content = parsed.get_page_content(page_id).expect("page content");
        assert!(content.windows(7).any(|window| window == b"/Im1 Do"));
    }

    #[test]
    fn pdf_page_group_collision_warning_obeys_signed_suppression() {
        for (control, expects_warning) in [(0, true), (1, false), (-1, false)] {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            stores.set_int_param(IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP, control);
            let result = run_with_images(
                &mut stores,
                concat!(
                    "\\pdfoutput=1 ",
                    "\\pdfximage \"first.pdf\" ",
                    "\\pdfximage \"second.pdf\" ",
                    "\\shipout\\hbox{\\pdfrefximage1\\kern1pt\\pdfrefximage2}\\end",
                ),
                [test_pdf_page_source(true), test_pdf_page_source(true)],
            );
            let pdf = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
                .expect("lower two page groups");
            let warning_present = stores.world().effect_records().iter().any(|effect| {
                matches!(
                    effect,
                    tex_state::EffectRecord::StreamWrite { text, .. }
                        if text.contains(tex_state::PdfPageGroupWarning::MULTIPLE_GROUPS_ON_ONE_PAGE)
                )
            });
            let parsed = lopdf::Document::load_mem(&pdf).expect("parse page-group PDF");
            let first = parsed
                .get_object((1, 0))
                .expect("first form")
                .as_stream()
                .expect("first form stream");
            let second = parsed
                .get_object((2, 0))
                .expect("second form")
                .as_stream()
                .expect("second form stream");
            let first_group = first
                .dict
                .get(b"Group")
                .expect("first group")
                .as_reference()
                .expect("first group reference");
            let second_group = second
                .dict
                .get(b"Group")
                .expect("second group")
                .as_reference()
                .expect("second group reference");
            assert_ne!(first_group, second_group);
            let page_id = parsed.get_pages()[&1];
            let content = parsed.get_page_content(page_id).expect("page content");
            assert_eq!(
                content
                    .windows(3)
                    .filter(|window| *window == b" Do")
                    .count(),
                2,
                "both included forms must be painted",
            );
            let output_group = parsed
                .get_dictionary(page_id)
                .expect("output page")
                .get(b"Group")
                .expect("output group")
                .as_reference()
                .expect("output group reference");
            assert_eq!(output_group, first_group);
            assert_eq!(
                warning_present, expects_warning,
                "suppression value {control}",
            );
        }
    }

    fn positioned_fixture(events: Vec<PositionedEvent>, page_index: u32) -> PositionedPage {
        PositionedPage {
            page_index,
            width: Scaled::from_raw(200),
            height: Scaled::from_raw(200),
            page_origin_x: Scaled::from_raw(0),
            page_origin_y: Scaled::from_raw(0),
            mag: 1_000,
            counts: [0; 10],
            fonts: Vec::new(),
            events,
            diagnostics: Vec::new(),
            last_saved_position: None,
            snap_reference: (Scaled::from_raw(0), Scaled::from_raw(0)),
        }
    }

    #[test]
    fn running_link_geometry_continues_with_fresh_page_local_segments() {
        use tex_out::positioned::{PositionedBoxEnd, PositionedPdfAnnotation};

        let mut stores = Universe::default();
        let link = stores
            .create_pdf_link(
                PdfAnnotationDimensions::RUNNING,
                TokenListId::EMPTY,
                PdfActionSpec::User(TokenListId::EMPTY),
                0,
            )
            .expect("logical link");
        stores.end_pdf_link();
        let first_box = PositionedBox {
            id: 0,
            depth: 2,
            kind: BoxKind::Horizontal,
            x: Scaled::from_raw(10),
            y: Scaled::from_raw(20),
            width: Scaled::from_raw(100),
            height: Scaled::from_raw(30),
            baseline: Scaled::from_raw(40),
        };
        let second_box = PositionedBox {
            id: 0,
            depth: 2,
            kind: BoxKind::Horizontal,
            x: Scaled::from_raw(5),
            y: Scaled::from_raw(30),
            width: Scaled::from_raw(80),
            height: Scaled::from_raw(25),
            baseline: Scaled::from_raw(45),
        };
        let pages = vec![
            positioned_fixture(
                vec![
                    PositionedEvent::Box(first_box),
                    PositionedEvent::PdfAnnotation(PositionedPdfAnnotation {
                        x: Scaled::from_raw(30),
                        y: first_box.baseline,
                        containing_box: 0,
                        depth: 2,
                        marker: tex_out::PdfAnnotationEffect::LinkStart {
                            object: link.object(),
                        },
                    }),
                    PositionedEvent::BoxEnd(PositionedBoxEnd { id: 0, depth: 2 }),
                ],
                0,
            ),
            positioned_fixture(
                vec![
                    PositionedEvent::Box(second_box),
                    PositionedEvent::PdfAnnotation(PositionedPdfAnnotation {
                        x: Scaled::from_raw(25),
                        y: second_box.baseline,
                        containing_box: 0,
                        depth: 2,
                        marker: tex_out::PdfAnnotationEffect::LinkEnd {
                            object: link.object(),
                        },
                    }),
                    PositionedEvent::BoxEnd(PositionedBoxEnd { id: 0, depth: 2 }),
                ],
                1,
            ),
        ];
        let mut shipped =
            lower_page_annotations(&stores, &pages, &[Scaled::from_raw(2), Scaled::from_raw(3)])
                .expect("link lowering");
        assert_eq!(shipped[0][0].rect.left, Scaled::from_raw(28));
        assert_eq!(shipped[0][0].rect.right, Scaled::from_raw(112));
        assert_eq!(shipped[1][0].rect.left, Scaled::from_raw(2));
        assert_eq!(shipped[1][0].rect.right, Scaled::from_raw(28));
        assign_annotation_objects(&mut stores, &mut shipped).expect("continuation object");
        assert_eq!(shipped[0][0].object, link.object());
        assert_ne!(shipped[1][0].object, link.object());
    }

    #[test]
    fn annotations_are_page_owned_typed_indirect_objects() {
        let (mut stores, result) = run(concat!(
            "\\pdfoutput=1",
            "\\pdfannot reserveobjnum",
            "\\shipout\\hbox{",
            "\\pdfannot useobjnum 1 width 10pt {/Subtype /Text}",
            "\\pdfstartlink height 6pt attr{/Border [0 0 0]}",
            "user{/Subtype /Link /A << /S /URI /URI (u) >>}",
            "\\kern10pt\\pdfendlink}",
            "\\end",
        ));
        let bytes = pdf_from_committed_artifacts(&mut stores, &result.committed_artifacts)
            .expect("typed annotations serialize");
        let document = lopdf::Document::load_mem(&bytes).expect("parse generated PDF");
        let page_id = document.get_pages()[&1];
        let page = document
            .get_object(page_id)
            .and_then(lopdf::Object::as_dict)
            .expect("page dictionary");
        let annotations = page
            .get(b"Annots")
            .and_then(lopdf::Object::as_array)
            .expect("page annotations");
        assert_eq!(annotations.len(), 2);
        for annotation in annotations {
            let id = annotation.as_reference().expect("indirect annotation");
            let dictionary = document
                .get_object(id)
                .and_then(lopdf::Object::as_dict)
                .expect("annotation dictionary");
            assert_eq!(
                dictionary
                    .get(b"Type")
                    .expect("annotation type")
                    .as_name()
                    .expect("annotation type name"),
                b"Annot"
            );
            assert_eq!(
                dictionary
                    .get(b"Rect")
                    .expect("annotation rectangle")
                    .as_array()
                    .expect("annotation rectangle array")
                    .len(),
                4
            );
        }
    }

    fn run_in(stores: &mut Universe, source: &str) -> RunResult {
        let mut input = InputStack::new(MemoryInput::new(source));
        let mut input_resolver = RejectingMemoryInputResolver;
        let mut font_resolver = DirectFontResolver;
        let context =
            ExecutionContext::with_resolvers("pdf-test", &mut input_resolver, &mut font_resolver);
        run_input_collecting_artifacts(&mut input, stores, context).expect("minimal page ships")
    }

    fn try_run_in(stores: &mut Universe, source: &str) -> Result<RunResult, tex_exec::ExecError> {
        let mut input = InputStack::new(MemoryInput::new(source));
        let mut input_resolver = RejectingMemoryInputResolver;
        let mut font_resolver = DirectFontResolver;
        let context =
            ExecutionContext::with_resolvers("pdf-test", &mut input_resolver, &mut font_resolver);
        run_input_collecting_artifacts(&mut input, stores, context)
    }

    fn run(source: &str) -> (Universe, RunResult) {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let result = run_in(&mut stores, source);
        (stores, result)
    }

    fn pt(value: i32) -> Scaled {
        Scaled::from_raw(value * 65_536)
    }

    fn run_with_clock(source: &str, clock: JobClock) -> (Universe, RunResult) {
        let mut stores = Universe::with_world(World::memory_with_clock(clock));
        prepare_pdftex_run_stores(&mut stores);
        let result = run_in(&mut stores, source);
        (stores, result)
    }

    fn provide_abc_encoding(stores: &mut Universe) {
        let mut encoding = b"/FixtureEncoding [".to_vec();
        for code in 0..256 {
            let name = match code {
                65 => "A",
                66 => "B",
                67 => "C",
                _ => ".notdef",
            };
            encoding.extend_from_slice(format!("/{name} ").as_bytes());
        }
        encoding.extend_from_slice(b"] def");
        stores
            .provide_pdf_encoding(b"fixture.enc".to_vec(), &encoding)
            .expect("provide detached encoding");
    }

    fn provide_tagged_spacing_font(stores: &mut Universe, explicit_space: bool) {
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed TFM");
        stores
            .provide_pdf_type1_program(
                b"cmr10.pfb".to_vec(),
                include_bytes!("../../../tests/corpus/pdf/embedded_type1.pfb"),
            )
            .expect("committed PFB");
        let mut encoding = b"/TaggedSpacingEncoding [".to_vec();
        for code in 0..256 {
            let name = match code {
                32 if explicit_space => "space",
                65 => "A",
                66 => "B",
                67 => "C",
                68 => "D",
                69 => "E",
                _ => ".notdef",
            };
            encoding.extend_from_slice(format!("/{name} ").as_bytes());
        }
        encoding.extend_from_slice(b"] def");
        stores
            .provide_pdf_encoding(b"tagged-spacing.enc".to_vec(), &encoding)
            .expect("provide tagged-spacing encoding");
    }

    fn shown_text_operands(document: &lopdf::Document, page_number: u32) -> Vec<Vec<u8>> {
        let page = document.get_pages()[&page_number];
        let bytes = document.get_page_content(page).expect("page content");
        lopdf::content::Content::decode(&bytes)
            .expect("decode content operators")
            .operations
            .into_iter()
            .filter(|operation| operation.operator == "Tj")
            .map(|operation| {
                operation.operands[0]
                    .as_str()
                    .expect("Tj string operand")
                    .to_vec()
            })
            .collect()
    }

    #[test]
    fn minimal_rule_page_emits_deterministic_valid_pdf_structure() {
        let source =
            "\\pdfoutput=1\\pdfcompresslevel=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end";
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .begin_retained_session()
            .expect("retained test session starts");
        let before = stores.snapshot();
        let first_run = run_in(&mut stores, source);
        let first = pdf_from_committed_artifacts(&mut stores, &first_run.committed_artifacts)
            .expect("PDF assembles");
        let first_pages = stores.pdf_pages().to_vec();
        let first_hash = stores.snapshot().state_hash();

        stores.rollback(&before);
        let second_run = run_in(&mut stores, source);
        let second = pdf_from_committed_artifacts(&mut stores, &second_run.committed_artifacts)
            .expect("PDF replay assembles");

        assert_eq!(first, second);
        assert_eq!(stores.pdf_pages(), first_pages);
        assert_eq!(stores.snapshot().state_hash(), first_hash);
        assert!(first.starts_with(b"%PDF-1.4"));
        assert!(
            first
                .windows(b"/ProcSet[/PDF]".len())
                .any(|window| window == b"/ProcSet[/PDF]")
        );
        assert!(first.windows(2).any(|window| window == b"re"));
        assert_eq!(stores.pdf_pages().len(), 1);
        assert_eq!(stores.pdf_pages()[0].resources_object(), 1);
        assert_eq!(stores.pdf_pages()[0].page_object(), 2);
        assert_eq!(stores.pdf_pages()[0].contents_object(), 3);
    }

    #[test]
    fn accessibility_whatsits_survive_shipout_and_artifact_round_trip() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed metrics");
        let run = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1 ",
                "\\font\\a=cmr10 \\a ",
                "\\shipout\\hbox{A\\pdfinterwordspaceon B\\pdffakespace ",
                "C\\pdfinterwordspaceoff D}",
                "\\end",
            ),
        );
        let artifact = tex_out::PageArtifact::from_bytes(run.committed_artifacts[0].bytes())
            .expect("artifact round trip");
        assert_eq!(
            artifact
                .effects
                .iter()
                .filter_map(|effect| match effect {
                    tex_out::PageEffect::PdfAccessibility(control) => Some(*control),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![
                tex_out::PdfAccessibilityEffect::InterwordSpaceOn,
                tex_out::PdfAccessibilityEffect::FakeSpace,
                tex_out::PdfAccessibilityEffect::InterwordSpaceOff,
            ]
        );
        let positioned = tex_out::positioned::lower_page(&artifact, 0).expect("positioned page");
        assert_eq!(
            positioned
                .events
                .iter()
                .filter_map(|event| match event {
                    PositionedEvent::PdfAccessibility(control) => Some(control.control),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![
                tex_out::PdfAccessibilityEffect::InterwordSpaceOn,
                tex_out::PdfAccessibilityEffect::FakeSpace,
                tex_out::PdfAccessibilityEffect::InterwordSpaceOff,
            ]
        );
    }

    #[test]
    fn tagged_spacing_uses_explicit_space_and_reanchors_after_disabled_glue() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        provide_tagged_spacing_font(&mut stores, true);
        let run = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfcompresslevel=0 ",
                "\\font\\f=cmr10 ",
                "\\pdfmapline{=cmr10 CMR10 <tagged-spacing.enc <<cmr10.pfb}",
                "\\shipout\\hbox{\\f A\\pdfinterwordspaceon\\hskip3pt ",
                "B\\pdfinterwordspaceoff\\hskip3pt C}\\end",
            ),
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("tagged PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("tagged PDF parses");
        assert_eq!(
            shown_text_operands(&parsed, 1),
            vec![b"A".to_vec(), b" ".to_vec(), b"B".to_vec(), b"C".to_vec()]
        );
        assert!(
            !pdf.windows(b"/UmberSpace".len())
                .any(|w| w == b"/UmberSpace")
        );
        assert_eq!(
            parsed.extract_text(&[1]).expect("text extracts").trim(),
            "A BC"
        );
    }

    #[test]
    fn fallback_space_font_is_lazy_shared_and_keeps_first_selection_across_pages() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        provide_tagged_spacing_font(&mut stores, false);
        let run = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfcompresslevel=0 ",
                "\\font\\f=cmr10 ",
                "\\pdfmapline{=cmr10 CMR10 <tagged-spacing.enc <<cmr10.pfb}",
                "\\pdfspacefont{first-space}",
                "\\shipout\\hbox{\\f A\\pdfinterwordspaceon\\hskip3pt B}",
                "\\pdfspacefont{second-space}",
                "\\shipout\\hbox{\\f C\\hskip3pt D\\pdffakespace E",
                "\\pdfinterwordspaceoff}\\end",
            ),
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("fallback-space PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("fallback-space PDF parses");
        assert_eq!(
            shown_text_operands(&parsed, 1),
            vec![b"A".to_vec(), b" ".to_vec(), b"B".to_vec()]
        );
        assert_eq!(
            shown_text_operands(&parsed, 2),
            vec![
                b"C".to_vec(),
                b" ".to_vec(),
                b"D".to_vec(),
                b" ".to_vec(),
                b"E".to_vec()
            ]
        );
        let fallback_fonts = parsed
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .filter(|dictionary| {
                dictionary
                    .get(b"Subtype")
                    .ok()
                    .and_then(|value| value.as_name().ok())
                    == Some(b"Type3".as_slice())
                    && dictionary
                        .get(b"Name")
                        .ok()
                        .and_then(|value| value.as_name().ok())
                        == Some(b"first-space".as_slice())
            })
            .count();
        assert_eq!(fallback_fonts, 1);
        assert!(
            !pdf.windows(b"second-space".len())
                .any(|w| w == b"second-space")
        );
        assert_eq!(
            parsed.extract_text(&[1]).expect("page one extracts").trim(),
            "A B"
        );
        assert_eq!(
            parsed.extract_text(&[2]).expect("page two extracts").trim(),
            "C D E"
        );
    }

    #[test]
    fn text_page_emits_font_resources_and_pdf_writer_text_operators() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed TFM");
        let parts: &[&[u8]] = &[
            &[0x80, 1, 3, 0, 0, 0],
            b"abc",
            &[0x80, 2, 2, 0, 0, 0],
            b"de",
            &[0x80, 1, 1, 0, 0, 0],
            b"f",
            &[0x80, 3],
        ];
        let pfb = parts.concat();
        stores
            .provide_pdf_type1_program(b"cmr10.pfb".to_vec(), &pfb)
            .expect("provide detached Type-1 program");
        let mut encoding = b"/FixtureEncoding [".to_vec();
        for code in 0..256 {
            let name = match code {
                65 => "A",
                66 => "B",
                67 => "C",
                _ => ".notdef",
            };
            encoding.extend_from_slice(format!("/{name} ").as_bytes());
        }
        encoding.extend_from_slice(b"] def");
        stores
            .provide_pdf_encoding(b"fixture.enc".to_vec(), &encoding)
            .expect("provide detached encoding");
        let run_result = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfcompresslevel=0",
                "\\font\\f=cmr10 ",
                "\\pdfmapline{=cmr10 CMR10 <fixture.enc <<cmr10.pfb}",
                "\\pdffontattr\\f{/TestAttr 42}",
                "\\immediate\\pdfobj{<< /Kind /AllocatorProbe >>}",
                "\\shipout\\hbox{\\f\\char65\\char66\\char67}\\end",
            ),
        );
        let artifact = tex_out::PageArtifact::from_bytes(run_result.committed_artifacts[0].bytes())
            .expect("artifact parses");
        let positioned = tex_out::positioned::lower_page(&artifact, 0).expect("page positions");
        assert!(!positioned.fonts.is_empty(), "{positioned:?}");
        assert!(
            positioned.events.iter().any(
                |event| matches!(event, PositionedEvent::TextRun(run) if !run.units.is_empty())
            ),
            "{positioned:?}"
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("text PDF assembles");
        let replay = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("text PDF replay assembles");
        assert_eq!(pdf, replay);
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        assert_eq!(
            parsed
                .get_object((1, 0))
                .expect("raw allocator probe")
                .as_dict()
                .expect("raw probe dictionary")
                .get(b"Kind")
                .expect("probe Kind")
                .as_name()
                .expect("probe name"),
            b"AllocatorProbe"
        );
        assert_eq!(
            parsed
                .extract_text(&[1])
                .expect("extract Type1 text")
                .trim(),
            "ABC"
        );
        let page_id = parsed.get_pages()[&1];
        let page = parsed
            .get_object(page_id)
            .expect("page")
            .as_dict()
            .expect("page dictionary");
        let resources_id = page
            .get(b"Resources")
            .expect("resources")
            .as_reference()
            .expect("indirect resources");
        let resources = parsed
            .get_object(resources_id)
            .expect("resources object")
            .as_dict()
            .expect("resources dictionary");
        let fonts = resources
            .get(b"Font")
            .expect("font resources")
            .as_dict()
            .expect("font resource dictionary");
        let font_id = fonts
            .get(b"F1")
            .expect("F1")
            .as_reference()
            .expect("indirect font");
        let font = parsed
            .get_object(font_id)
            .expect("font object")
            .as_dict()
            .expect("font dictionary");
        assert_eq!(
            font.get(b"BaseFont")
                .expect("BaseFont")
                .as_name()
                .expect("BaseFont name"),
            b"CMR10"
        );
        let encoding = font
            .get(b"Encoding")
            .expect("custom Encoding")
            .as_dict()
            .expect("inline Encoding dictionary");
        let differences = encoding
            .get(b"Differences")
            .expect("Differences")
            .as_array()
            .expect("Differences array");
        assert_eq!(differences.len(), 257);
        assert_eq!(differences[66].as_name().expect("code 65 glyph"), b"A");
        let descriptor_id = font
            .get(b"FontDescriptor")
            .expect("FontDescriptor")
            .as_reference()
            .expect("indirect descriptor");
        let descriptor = parsed
            .get_object(descriptor_id)
            .expect("descriptor object")
            .as_dict()
            .expect("descriptor dictionary");
        assert_eq!(
            descriptor
                .get(b"TestAttr")
                .expect("pdffontattr entry")
                .as_i64()
                .expect("integer attribute"),
            42
        );
        let program_id = descriptor
            .get(b"FontFile")
            .expect("embedded FontFile")
            .as_reference()
            .expect("indirect FontFile");
        let program = parsed
            .get_object(program_id)
            .expect("FontFile stream")
            .as_stream()
            .expect("FontFile is a stream");
        assert_eq!(program.content, b"abcdef");
        for (key, expected) in [(b"Length1", 3), (b"Length2", 2), (b"Length3", 1)] {
            assert_eq!(
                program
                    .dict
                    .get(key)
                    .expect("segment length")
                    .as_i64()
                    .expect("integer segment length"),
                expected
            );
        }
        let content = parsed
            .get_page_content(page_id)
            .expect("decoded page content");
        for operator in [b"BT".as_slice(), b"Tf", b"Tm", b"Tj", b"ET"] {
            assert!(
                content
                    .windows(operator.len())
                    .any(|window| window == operator),
                "missing {}",
                String::from_utf8_lossy(operator)
            );
        }
        assert!(content.windows(3).any(|window| window == b"ABC"));
    }

    #[test]
    fn resident_map_entry_omits_embedded_program_and_descriptor() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed TFM");
        let run_result = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfcompresslevel=0",
                "\\font\\f=cmr10 ",
                "\\pdfmapline{=cmr10 Helvetica}",
                "\\shipout\\hbox{\\f ABC}\\end",
            ),
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("resident-font PDF assembles");
        assert!(
            pdf.windows(b"/BaseFont/Helvetica".len())
                .any(|window| window == b"/BaseFont/Helvetica")
        );
        assert!(
            !pdf.windows(b"/FontDescriptor".len())
                .any(|window| window == b"/FontDescriptor")
        );
        assert!(
            !pdf.windows(b"/FontFile".len())
                .any(|window| window == b"/FontFile")
        );
    }

    #[test]
    fn subset_map_entry_embeds_only_used_and_included_type1_glyphs() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed TFM");
        let pfb = include_bytes!("../../../tests/corpus/pdf/embedded_type1.pfb");
        stores
            .provide_pdf_type1_program(b"cmr10.pfb".to_vec(), pfb)
            .expect("committed PFB");
        let run_result = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\font\\f=cmr10 ",
                "\\pdfmapline{=cmr10 CMR10 <cmr10.pfb}",
                "\\pdfincludechars\\f{C}",
                "\\shipout\\hbox{\\f A}\\end",
            ),
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("subset PDF assembles");
        assert!(
            pdf.windows(b"/BaseFont/KMCZIW+CMR10".len())
                .any(|window| { window == b"/BaseFont/KMCZIW+CMR10" })
        );
        assert!(
            pdf.windows(b"/CharSet(/A/C)".len())
                .any(|window| { window == b"/CharSet(/A/C)" })
        );
        let parsed = lopdf::Document::load_mem(&pdf).expect("subset parses");
        let embedded = parsed
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .find(|stream| stream.dict.has(b"Length2"))
            .expect("subset FontFile stream");
        let full = tex_fonts::PdfType1Program::from_pfb(pfb).expect("full PFB decodes");
        assert!(embedded.content.len() < full.bytes().len());
    }

    #[test]
    fn explicit_glyph_mappings_emit_to_unicode_and_extract_exact_text() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed TFM");
        stores
            .provide_pdf_type1_program(
                b"cmr10.pfb".to_vec(),
                include_bytes!("../../../tests/corpus/pdf/embedded_type1.pfb"),
            )
            .expect("committed PFB");
        let run_result = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfcompresslevel=0\\pdfgentounicode=1 ",
                "\\font\\f=cmr10 ",
                "\\pdfmapline{=cmr10 CMR10 <cmr10.pfb}",
                "\\pdfglyphtounicode{A}{0041}",
                "\\pdfglyphtounicode{B}{0066 0066}",
                "\\pdfglyphtounicode{C}{1F600}",
                "\\shipout\\hbox{\\f ABC}\\end",
            ),
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("ToUnicode PDF assembles");
        assert!(
            pdf.windows(b"/ToUnicode".len())
                .any(|window| window == b"/ToUnicode")
        );
        assert!(
            pdf.windows(b"<41> <0041>".len())
                .any(|window| window == b"<41> <0041>")
        );
        assert!(
            pdf.windows(b"<42> <00660066>".len())
                .any(|window| { window == b"<42> <00660066>" })
        );
        assert!(
            pdf.windows(b"<43> <D83DDE00>".len())
                .any(|window| { window == b"<43> <D83DDE00>" })
        );
        let parsed = lopdf::Document::load_mem(&pdf).expect("ToUnicode PDF parses");
        assert_eq!(
            parsed.extract_text(&[1]).expect("text extracts").trim(),
            "Aff😀"
        );
    }

    #[test]
    fn unicode_style_glyph_names_use_pdftex_builtin_inference() {
        assert_eq!(
            inferred_glyph_unicode(b"uni00410066.alt"),
            Some(vec![0x41, 0x66])
        );
        assert_eq!(inferred_glyph_unicode(b"u1F600"), Some(vec![0x1f600]));
        assert_eq!(inferred_glyph_unicode(b"A"), None);
        assert_eq!(inferred_glyph_unicode(b"uniD800"), None);
    }

    #[test]
    fn no_builtin_and_nonpositive_generation_omit_to_unicode() {
        for control in [
            "\\pdfgentounicode=-1",
            "\\pdfgentounicode=1\\pdfnobuiltintounicode\\f",
        ] {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            stores
                .world_mut()
                .set_memory_file(
                    "cmr10.tfm",
                    include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
                )
                .expect("seed TFM");
            stores
                .provide_pdf_type1_program(
                    b"cmr10.pfb".to_vec(),
                    include_bytes!("../../../tests/corpus/pdf/embedded_type1.pfb"),
                )
                .expect("committed PFB");
            let source = format!(
                "\\pdfoutput=1\\font\\f=cmr10 {control} \\
                 \\pdfmapline{{=cmr10 CMR10 <<cmr10.pfb}}\\pdfglyphtounicode{{A}}{{0041}}\\shipout\\hbox{{\\f A}}\\end"
            );
            let run_result = run_in(&mut stores, &source);
            let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
                .expect("PDF assembles");
            assert!(
                !pdf.windows(b"/ToUnicode".len())
                    .any(|window| window == b"/ToUnicode")
            );
        }
    }

    #[test]
    fn committed_woff2_embeds_as_valid_truetype_fontfile2() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed TFM");
        let logical_name = b"cmu-serif-500-roman.woff2".to_vec();
        stores
            .provide_pdf_truetype_program(
                logical_name,
                include_bytes!("../../umber-wasm/assets/cmu-serif-500-roman.woff2"),
            )
            .expect("decode committed WOFF2");
        let run_result = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfcompresslevel=0",
                "\\font\\f=cmr10 ",
                "\\pdfmapline{=cmr10 CMUSerif <<cmu-serif-500-roman.woff2}",
                "\\shipout\\hbox{\\f ABC}\\end",
            ),
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("TrueType PDF assembles");
        assert!(
            pdf.windows(b"/Subtype/TrueType".len())
                .any(|w| w == b"/Subtype/TrueType")
        );
        assert!(pdf.windows(b"/FontFile2".len()).any(|w| w == b"/FontFile2"));
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses TrueType output");
        assert_eq!(
            parsed
                .extract_text(&[1])
                .expect("extract TrueType text")
                .trim(),
            "ABC"
        );
        let embedded = parsed
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .find(|stream| stream.content.starts_with(&[0, 1, 0, 0]))
            .expect("decoded SFNT is embedded");
        assert_eq!(
            embedded
                .dict
                .get(b"Length1")
                .expect("Length1")
                .as_i64()
                .expect("integer Length1") as usize,
            embedded.content.len()
        );
    }

    #[test]
    fn subset_truetype_uses_named_glyph_closure_and_simple_pdf_encoding() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed TFM");
        let logical_name = b"cmu-serif-500-roman.woff2".to_vec();
        stores
            .provide_pdf_truetype_program(
                logical_name,
                include_bytes!("../../umber-wasm/assets/cmu-serif-500-roman.woff2"),
            )
            .expect("decode committed WOFF2");
        provide_abc_encoding(&mut stores);
        let full_len = stores
            .pdf_truetype_program(b"cmu-serif-500-roman.woff2")
            .expect("full TrueType program")
            .bytes()
            .len();
        let run_result = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfcompresslevel=0",
                "\\font\\f=cmr10 ",
                "\\pdfmapline{=cmr10 CMUSerif <fixture.enc <cmu-serif-500-roman.woff2}",
                "\\shipout\\hbox{\\f ABC}\\end",
            ),
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("subset TrueType PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("subset TrueType output parses");
        assert_eq!(
            parsed
                .extract_text(&[1])
                .expect("extract subset text")
                .trim(),
            "ABC"
        );
        let embedded = parsed
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .find(|stream| stream.content.starts_with(&[0, 1, 0, 0]))
            .expect("subset SFNT embedded");
        assert!(embedded.content.len() < full_len / 4);
        let face = ttf_parser::Face::parse(&embedded.content, 0).expect("subset SFNT parses");
        for name in ["A", "B", "C"] {
            assert!(
                (0..face.number_of_glyphs())
                    .map(ttf_parser::GlyphId)
                    .any(|glyph| face.glyph_name(glyph) == Some(name))
            );
        }
        assert!(
            !(0..face.number_of_glyphs())
                .map(ttf_parser::GlyphId)
                .any(|glyph| face.glyph_name(glyph) == Some("D"))
        );
    }

    #[test]
    fn default_info_dictionary_uses_the_pinned_job_clock() {
        let clock = JobClock {
            time: 13 * 60 + 36,
            second: 7,
            day: 9,
            month: 7,
            year: 2026,
        };
        let (mut stores, run_result) = run_with_clock(
            "\\pdfoutput=1\\pdfcompresslevel=0\\shipout\\vbox{\\hrule width1pt height1pt}\\end",
            clock,
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let info_id = parsed
            .trailer
            .get(b"Info")
            .expect("default Info trailer entry")
            .as_reference()
            .expect("Info reference");
        let info = parsed
            .get_object(info_id)
            .expect("Info object")
            .as_dict()
            .expect("Info dictionary");
        for (key, expected) in [
            (b"Producer".as_slice(), b"pdfTeX-1.40.27".as_slice()),
            (b"Creator".as_slice(), b"TeX".as_slice()),
            (b"CreationDate".as_slice(), b"D:20260709133607Z".as_slice()),
            (b"ModDate".as_slice(), b"D:20260709133607Z".as_slice()),
            (
                b"PTEX.Fullbanner".as_slice(),
                b"This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025)".as_slice(),
            ),
        ] {
            assert_eq!(
                info.get(key)
                    .unwrap_or_else(|_| panic!("missing {}", String::from_utf8_lossy(key)))
                    .as_str()
                    .expect("metadata string"),
                expected
            );
        }
        assert_eq!(
            info.get(b"Trapped")
                .expect("Trapped")
                .as_name()
                .expect("Trapped name"),
            b"False"
        );
    }

    #[test]
    fn info_omission_date_suppression_and_ptex_key_policy_match_pdftex() {
        let source = concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\shipout\\vbox{\\hrule width1pt height1pt}",
            "\\pdfinfoomitdate=-1\\pdfsuppressptexinfo=-1\\end",
        );
        let (mut stores, run_result) = run(source);
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let info_id = parsed
            .trailer
            .get(b"Info")
            .expect("Info trailer entry")
            .as_reference()
            .expect("Info reference");
        let info = parsed
            .get_object(info_id)
            .expect("Info object")
            .as_dict()
            .expect("Info dictionary");
        assert!(!info.has(b"CreationDate"));
        assert!(!info.has(b"ModDate"));
        assert!(!info.has(b"PTEX.Fullbanner"));
        assert!(!info.has(b"PTEX_Fullbanner"));

        let (mut stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfptexuseunderscore=1",
            "\\shipout\\vbox{\\hrule width1pt height1pt}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let info_id = parsed
            .trailer
            .get(b"Info")
            .expect("Info trailer entry")
            .as_reference()
            .expect("Info reference");
        let info = parsed
            .get_object(info_id)
            .expect("Info object")
            .as_dict()
            .expect("Info dictionary");
        assert!(info.has(b"PTEX_Fullbanner"));
        assert!(!info.has(b"PTEX.Fullbanner"));

        let (mut stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\shipout\\vbox{\\hrule width1pt height1pt}",
            "\\pdfomitinfodict=-1\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        assert!(!parsed.trailer.has(b"Info"));
    }

    #[test]
    fn procset_policy_is_captured_at_each_shipout() {
        let (mut stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "{\\pdfomitprocset=1\\shipout\\vbox{\\hrule width1pt height1pt}}",
            "\\shipout\\vbox{\\hrule width1pt height1pt}",
            "{\\pdfomitprocset=-1\\shipout\\vbox{\\hrule width1pt height1pt}}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let pages = parsed.get_pages();
        for (page_number, expected) in [(1, false), (2, true), (3, true)] {
            let page = parsed
                .get_object(pages[&page_number])
                .expect("page object")
                .as_dict()
                .expect("page dictionary");
            let resources_id = page
                .get(b"Resources")
                .expect("Resources entry")
                .as_reference()
                .expect("Resources reference");
            let resources = parsed
                .get_object(resources_id)
                .expect("resources object")
                .as_dict()
                .expect("resources dictionary");
            assert_eq!(resources.has(b"ProcSet"), expected);
        }

        let (mut stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfmajorversion=2\\pdfminorversion=0\\pdfcompresslevel=0",
            "\\shipout\\vbox{\\hrule width1pt height1pt}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let page_id = parsed.get_pages()[&1];
        let page = parsed
            .get_object(page_id)
            .expect("page object")
            .as_dict()
            .expect("page dictionary");
        let resources_id = page
            .get(b"Resources")
            .expect("Resources entry")
            .as_reference()
            .expect("Resources reference");
        let resources = parsed
            .get_object(resources_id)
            .expect("resources object")
            .as_dict()
            .expect("resources dictionary");
        assert!(!resources.has(b"ProcSet"));
    }

    fn pdf_number(object: &lopdf::Object) -> f32 {
        match object {
            lopdf::Object::Integer(value) => *value as f32,
            lopdf::Object::Real(value) => *value,
            other => panic!("expected PDF number, got {other:?}"),
        }
    }

    #[test]
    fn page_parameters_are_consumed_at_pdftex_scopes() {
        let (mut stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfdecimaldigits=3",
            "\\pdfpagesattr{/Lang (early)}",
            "\\pdfpagewidth=100bp\\pdfpageheight=200bp",
            "\\pdfhorigin=10bp\\pdfvorigin=20bp",
            "\\pdfpageattr{/Rotate 90}",
            "\\pdfpageresources{/ExtGState << /A << /Type /ExtGState >> >>}",
            "\\shipout\\vbox{\\hrule width1bp height2bp}",
            "\\pdfpagewidth=300bp\\pdfpageheight=400bp",
            "\\pdfhorigin=30bp\\pdfvorigin=40bp",
            "\\pdfpageattr{/Rotate 180}",
            "\\pdfpageresources{/ColorSpace << /C /DeviceRGB >>}",
            "\\shipout\\vbox{\\hrule width3bp height4bp}",
            "\\pdfpagesattr{/Lang (final)}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let pages = parsed.get_pages();
        assert_eq!(pages.len(), 2);

        let pages_id = parsed
            .catalog()
            .expect("catalog")
            .get(b"Pages")
            .expect("page-tree reference")
            .as_reference()
            .expect("Pages is indirect");
        let pages_root = parsed
            .get_object(pages_id)
            .expect("pages root")
            .as_dict()
            .expect("pages dictionary");
        assert_eq!(
            pages_root
                .get(b"Lang")
                .expect("final pages attribute")
                .as_str()
                .expect("language string"),
            b"final"
        );

        for (number, expected_box, expected_rotate, resource_key) in [
            (1, [0.0, 0.0, 100.0, 200.0], 90, b"ExtGState".as_slice()),
            (2, [0.0, 0.0, 300.0, 400.0], 180, b"ColorSpace".as_slice()),
        ] {
            let page_id = pages[&number];
            let page = parsed
                .get_object(page_id)
                .expect("page")
                .as_dict()
                .expect("page dictionary");
            let media_box = page
                .get(b"MediaBox")
                .expect("MediaBox")
                .as_array()
                .expect("MediaBox array");
            for (actual, expected) in media_box.iter().map(pdf_number).zip(expected_box) {
                assert!((actual - expected).abs() < 0.002, "{actual} != {expected}");
            }
            assert_eq!(
                page.get(b"Rotate")
                    .expect("rotation")
                    .as_i64()
                    .expect("integer rotation"),
                expected_rotate
            );
            let resources_id = page
                .get(b"Resources")
                .expect("resources")
                .as_reference()
                .expect("resources reference");
            let resources = parsed
                .get_object(resources_id)
                .expect("resources")
                .as_dict()
                .expect("resources dictionary");
            assert!(resources.has(resource_key));
        }

        assert!(
            pdf.windows(b"10 178 1 2 re".len())
                .any(|window| { window == b"10 178 1 2 re" })
        );
        assert!(
            pdf.windows(b"30 356 3 4 re".len())
                .any(|window| { window == b"30 356 3 4 re" })
        );
    }

    #[test]
    fn raw_media_box_overrides_automatic_box_and_pk_mode_freezes() {
        let (mut stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfpkmode{first}",
            "\\pdfpagewidth=100bp\\pdfpageheight=200bp",
            "\\pdfpageattr{/MediaBox [1 2 3 4] /Rotate 90}",
            "\\shipout\\vbox{\\hrule width1bp height1bp}",
            "\\pdfpkmode{second}\\end",
        ));
        let fixed_pk_mode = stores.fixed_pdf_pk_mode().expect("PK mode frozen");
        assert_eq!(token_list_bytes(&stores, fixed_pk_mode), b"first");
        assert_eq!(
            token_list_bytes(&stores, stores.tok_param(TokParam::PDF_PK_MODE)),
            b"second"
        );

        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        assert_eq!(
            pdf.windows(b"/MediaBox".len())
                .filter(|window| *window == b"/MediaBox")
                .count(),
            1
        );
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let page_id = parsed.get_pages()[&1];
        let page = parsed
            .get_object(page_id)
            .expect("page")
            .as_dict()
            .expect("page dictionary");
        let media_box = page
            .get(b"MediaBox")
            .expect("raw MediaBox")
            .as_array()
            .expect("MediaBox array");
        assert_eq!(
            media_box.iter().map(pdf_number).collect::<Vec<_>>(),
            vec![1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn pk_request_resolves_default_and_clamps_explicit_resolution() {
        for (resolution, driver_dpi, expected) in
            [(0, 600, 600), (9_000, 300, 8_000), (-8, 300, 72)]
        {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            stores
                .world_mut()
                .set_memory_file(
                    "cmr10.tfm",
                    include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
                )
                .expect("seed TFM");
            let source = format!(
                concat!(
                    "\\pdfoutput=1\\pdfpkresolution={resolution}\\pdfpkmode{{fixture}}",
                    "\\font\\f=cmr10 \\pdfmapline{{-cmr10}}",
                    "\\shipout\\hbox{{\\f A}}\\end",
                ),
                resolution = resolution
            );
            run_in(&mut stores, &source);
            let font = stores
                .pdf_font_resources()
                .next()
                .expect("font resource")
                .font();
            let request = pk_font_request(&stores, font, driver_dpi).expect("PK request");
            assert_eq!(request.dpi(), expected);
            assert_eq!(request.mode(), b"fixture");
        }
    }

    #[test]
    fn zero_page_dimensions_fall_back_to_box_plus_twice_the_origins() {
        let (mut stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfdecimaldigits=3",
            "\\pdfpagewidth=0pt\\pdfpageheight=0pt",
            "\\pdfhorigin=10bp\\pdfvorigin=20bp",
            "\\shipout\\vbox{\\hrule width1bp height2bp}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let page_id = parsed.get_pages()[&1];
        let page = parsed
            .get_object(page_id)
            .expect("page")
            .as_dict()
            .expect("page dictionary");
        let media_box = page
            .get(b"MediaBox")
            .expect("MediaBox")
            .as_array()
            .expect("MediaBox array");
        let actual = media_box.iter().map(pdf_number).collect::<Vec<_>>();
        for (actual, expected) in actual.iter().zip([0.0, 0.0, 21.0, 42.0]) {
            assert!((*actual - expected).abs() < 0.002, "{actual} != {expected}");
        }
        assert!(
            pdf.windows(b"10 20 1 2 re".len())
                .any(|window| window == b"10 20 1 2 re")
        );
    }

    #[test]
    fn enabling_pdf_mode_does_not_change_dvi_page_bytes() {
        let (_, dvi_run) = run("\\pdfoutput=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end");
        let (_, pdf_run) = run("\\pdfoutput=1\\shipout\\vbox{\\hrule width10pt height5pt}\\end");
        assert_eq!(
            dvi_from_page_plans(&dvi_run.dvi_pages).expect("DVI assembles"),
            dvi_from_page_plans(&pdf_run.dvi_pages).expect("DVI assembles"),
        );
    }

    #[test]
    fn pdf_color_stack_allocation_does_not_change_dvi_bytes() {
        let (_, plain) = run("\\pdfoutput=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end");
        let (_, allocated) = run(concat!(
            "\\pdfoutput=0\\edef\\colors{\\pdfcolorstackinit page direct{0 g}}",
            "\\shipout\\vbox{\\hrule width10pt height5pt}\\end",
        ));
        assert_eq!(
            dvi_from_page_plans(&plain.dvi_pages).expect("plain DVI"),
            dvi_from_page_plans(&allocated.dvi_pages).expect("allocated DVI"),
        );
    }

    #[test]
    fn fixed_policy_drives_version_compression_and_decimal_output() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfmajorversion=1\\pdfminorversion=5",
            "\\pdfcompresslevel=0\\pdfobjcompresslevel=1\\pdfdecimaldigits=0",
            "\\shipout\\vbox{\\hrule width10pt height5pt}",
            "\\pdfcompresslevel=9\\pdfobjcompresslevel=0\\pdfdecimaldigits=4",
            "\\shipout\\vbox{\\hrule width10pt height5pt}\\end",
        ));
        let bytes = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("fixed-policy PDF assembles");
        let first_contents = stores.pdf_pages()[0].contents_object();

        assert!(bytes.starts_with(b"%PDF-1.5"));
        assert!(bytes.windows(12).any(|window| window == b"/Type/ObjStm"));
        let parsed = lopdf::Document::load_mem(&bytes).expect("fixed-policy PDF parses");
        assert_eq!(parsed.get_pages().len(), 2);
        let contents = parsed
            .get_object((first_contents, 0))
            .expect("first contents")
            .as_stream()
            .expect("contents stream");
        assert!(contents.dict.get(b"Filter").is_err());
    }

    #[test]
    fn frozen_output_mode_and_version_changes_are_fatal_setup_errors() {
        for (assignment, expected) in [
            ("\\pdfminorversion=7", "PDF version cannot be changed"),
            ("\\pdfoutput=0", "\\pdfoutput can only be changed"),
            (
                "\\pdfdraftmode=1",
                "\\pdfdraftmode can only be changed before anything is written",
            ),
        ] {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            let source = format!(
                "\\pdfoutput=1\\pdfminorversion=5\\shipout\\vbox{{\\hrule width1pt height1pt}}{assignment}\\shipout\\vbox{{\\hrule width1pt height1pt}}\\end"
            );
            let error = try_run_in(&mut stores, &source).expect_err("setup error must succumb");
            assert!(error.to_string().contains(expected), "{error}");
            assert_eq!(stores.pdf_pages().len(), 1);
        }
    }

    #[test]
    fn object_compression_levels_one_through_three_emit_type_two_xrefs() {
        for level in 1..=3 {
            let (mut stores, run) = run(&format!(
                "\\pdfoutput=1\\pdfminorversion=5\\pdfcompresslevel=6\\pdfobjcompresslevel={level}\\shipout\\vbox{{\\hrule width10pt height5pt}}\\end"
            ));
            let first = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
                .expect("object-stream PDF assembles");
            let second = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
                .expect("object-stream PDF repeats");
            assert_eq!(first, second);
            assert!(first.windows(12).any(|window| window == b"/Type/ObjStm"));
            assert!(first.windows(10).any(|window| window == b"/Type/XRef"));

            let parsed = lopdf::Document::load_mem(&first).expect("object-stream PDF parses");
            assert_eq!(parsed.get_pages().len(), 1);
            let contents_id = stores.pdf_pages()[0].contents_object();
            let contents = parsed
                .get_object((contents_id, 0))
                .expect("ordinary content stream")
                .as_stream()
                .expect("contents stream");
            assert_eq!(
                contents
                    .dict
                    .get(b"Filter")
                    .expect("flate filter")
                    .as_name()
                    .expect("filter name"),
                b"FlateDecode"
            );
        }
    }

    #[test]
    fn raw_objects_and_document_fragments_lower_exclusively_through_pdf_writer() {
        let mut world = tex_state::World::memory();
        world
            .set_memory_file("payload.bin", b"file payload".to_vec())
            .expect("seed stream file");
        let mut stores = Universe::with_world(world);
        prepare_pdftex_run_stores(&mut stores);
        let run_result = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfminorversion=5\\pdfcompresslevel=0\\pdfobjcompresslevel=1",
                "\\pdfobj{<< /Kind /Ordinary >>}\\pdfrefobj 1",
                "\\immediate\\pdfobj stream attr {/Subtype /XML}{stream payload}",
                "\\immediate\\pdfobj stream file {payload.bin}",
                "\\pdfcatalog{/PageMode /UseNone}",
                "\\pdfnames{/EmbeddedFiles << >>}",
                "\\pdfinfo{/Title (Info)}",
                "\\pdftrailer{/Custom true}",
                "\\pdftrailerid{custom-id}",
                "\\end",
            ),
        );
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("raw PDF extensions assemble");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses extension output");

        let ordinary = parsed
            .get_object((1, 0))
            .expect("referenced ordinary object")
            .as_dict()
            .expect("ordinary raw dictionary");
        assert_eq!(
            ordinary
                .get(b"Kind")
                .expect("Kind")
                .as_name()
                .expect("Kind name"),
            b"Ordinary"
        );
        let stream = parsed
            .get_object((2, 0))
            .expect("immediate stream")
            .as_stream()
            .expect("stream object");
        assert_eq!(stream.content, b"stream payload");
        assert_eq!(
            stream
                .dict
                .get(b"Subtype")
                .expect("Subtype")
                .as_name()
                .expect("Subtype name"),
            b"XML"
        );
        assert_eq!(
            parsed
                .get_object((3, 0))
                .expect("file stream")
                .as_stream()
                .expect("file stream object")
                .content,
            b"file payload"
        );

        let catalog = parsed.catalog().expect("catalog");
        assert_eq!(
            catalog
                .get(b"PageMode")
                .expect("PageMode")
                .as_name()
                .expect("PageMode name"),
            b"UseNone"
        );
        let names_id = catalog
            .get(b"Names")
            .expect("Names")
            .as_reference()
            .expect("Names reference");
        assert_eq!(names_id, (8, 0));
        assert!(
            parsed
                .get_object(names_id)
                .expect("Names object")
                .as_dict()
                .expect("Names dictionary")
                .has(b"EmbeddedFiles")
        );
        let info_id = parsed
            .trailer
            .get(b"Info")
            .expect("Info")
            .as_reference()
            .expect("Info reference");
        assert_eq!(
            parsed
                .get_object(info_id)
                .expect("Info object")
                .as_dict()
                .expect("Info dictionary")
                .get(b"Title")
                .expect("Title")
                .as_str()
                .expect("Title string"),
            b"Info"
        );
        assert!(
            parsed
                .trailer
                .get(b"Custom")
                .expect("Custom")
                .as_bool()
                .expect("Custom boolean")
        );
        let expected_id = Md5::digest(b"custom-id").to_vec();
        let ids = parsed
            .trailer
            .get(b"ID")
            .expect("ID")
            .as_array()
            .expect("ID array");
        assert_eq!(ids[0].as_str().expect("first ID string"), expected_id);
        assert_eq!(ids[1].as_str().expect("second ID string"), expected_id);
    }

    #[test]
    fn catalog_openaction_uses_canonical_object_ids_and_pdf_writer_catalog_reference() {
        let (mut stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfobjcompresslevel=0",
            "\\pdfcatalog{/PageMode /UseNone} openaction goto page 1 {/Fit}",
            "\\pdfobj{(raw)}\\pdfrefobj 3",
            "\\shipout\\vbox{\\hrule width1pt height1pt}\\end",
        ));
        let action = stores
            .pdf_catalog_open_action()
            .expect("open action record");
        assert_eq!(action.id(), 1);
        assert_eq!(action.target_object(), Some(2));
        assert_eq!(stores.pdf_raw_objects()[0].id().raw(), 3);
        assert_eq!(stores.pdf_pages()[0].resources_object(), 4);
        assert_eq!(stores.pdf_pages()[0].contents_object(), 5);
        assert_eq!(stores.pdf_pages()[0].page_object(), 2);

        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("open action PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses open action PDF");
        let catalog = parsed.catalog().expect("catalog");
        assert_eq!(
            catalog
                .get(b"OpenAction")
                .expect("OpenAction")
                .as_reference()
                .expect("action reference"),
            (1, 0)
        );
        let action = parsed
            .get_object((1, 0))
            .expect("action object")
            .as_dict()
            .expect("action dictionary");
        assert_eq!(
            action
                .get(b"S")
                .expect("action subtype")
                .as_name()
                .expect("subtype name"),
            b"GoTo"
        );
        let destination = action
            .get(b"D")
            .expect("action destination")
            .as_array()
            .expect("destination array");
        assert_eq!(
            destination[0]
                .as_reference()
                .expect("destination page reference"),
            (2, 0)
        );
        assert_eq!(destination[1].as_name().expect("destination view"), b"Fit");
    }

    #[test]
    fn catalog_openaction_serializes_user_and_remote_action_forms() {
        for (source, expected_subtype) in [
            (
                "\\pdfcatalog{} openaction user{<< /S /Named /N /Print >>}",
                b"Named".as_slice(),
            ),
            (
                "\\pdfcatalog{} openaction goto file{other.pdf} page 2 {/FitH 20} newwindow",
                b"GoToR".as_slice(),
            ),
            (
                "\\pdfcatalog{} openaction thread file{other.pdf} name{article}",
                b"Thread".as_slice(),
            ),
        ] {
            let (mut stores, run_result) = run(&format!(
                "\\pdfoutput=1\\pdfcompresslevel=0{source}\\shipout\\hbox{{}}\\end"
            ));
            let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
                .expect("action PDF assembles");
            let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses action PDF");
            let action_id = parsed
                .catalog()
                .expect("catalog")
                .get(b"OpenAction")
                .expect("OpenAction")
                .as_reference()
                .expect("action reference");
            let action = parsed
                .get_object(action_id)
                .expect("action object")
                .as_dict()
                .expect("action dictionary");
            assert_eq!(
                action
                    .get(b"S")
                    .expect("action subtype")
                    .as_name()
                    .expect("subtype name"),
                expected_subtype
            );
        }
    }

    #[test]
    fn referenced_reserved_object_fails_before_pdf_writer_publication() {
        let (mut stores, run_result) = run("\\pdfoutput=1\\pdfobj reserveobjnum\\pdfrefobj 1\\end");
        assert!(matches!(
            pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts),
            Err(PdfBuildError::ReferencedRawObjectUninitialized(1))
        ));
    }

    #[test]
    fn referenced_form_uses_typed_pdf_writer_xobject_and_page_resource() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\setbox0=\\hbox to10pt{\\vrule width10pt height5pt}",
            "\\pdfxform attr {/OC 7} resources {/ExtGState <<>>} 0",
            "\\pdfrefxform1\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("serialize referenced form");
        assert!(
            pdf.windows(b"/Subtype/Form".len())
                .any(|w| w == b"/Subtype/Form")
        );
        assert!(pdf.windows(b"/XObject".len()).any(|w| w == b"/XObject"));
        assert!(pdf.windows(b"/Fm1 Do".len()).any(|w| w == b"/Fm1 Do"));
        assert!(pdf.windows(b"/BBox[0 0".len()).any(|w| w == b"/BBox[0 0"));
        let parsed = lopdf::Document::load_mem(&pdf).expect("parse typed form PDF");
        let form = parsed
            .get_object((1, 0))
            .expect("form object")
            .as_stream()
            .expect("form stream");
        assert!(form.content.windows(2).any(|window| window == b"re"));
    }

    #[test]
    fn nested_forms_reuse_recursive_xobjects_and_publish_form_savepos() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\setbox0=\\hbox{\\kern10pt\\pdfsavepos\\vrule width2pt height3pt}",
            "\\pdfxform0",
            "\\setbox1=\\hbox{\\pdfrefxform1}",
            "\\pdfxform1\\pdfrefxform3\\end",
        ));
        assert_eq!(stores.pdf_last_position(), (pt(10), Scaled::from_raw(0)));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("serialize nested forms");
        assert_eq!(
            pdf.windows(b"/Subtype/Form".len())
                .filter(|w| *w == b"/Subtype/Form")
                .count(),
            2
        );
        assert!(pdf.windows(b"/Fm1 Do".len()).any(|w| w == b"/Fm1 Do"));
        assert!(pdf.windows(b"/Fm2 Do".len()).any(|w| w == b"/Fm2 Do"));
    }

    #[test]
    fn form_color_state_persists_separately_and_immediate_forms_serialize_without_references() {
        let (mut stores, first_run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\setbox0=\\hbox{\\pdfcolorstack0 push {1 g}}\\pdfxform0",
            "\\setbox1=\\hbox{\\pdfcolorstack0 current}\\pdfxform1",
            "\\pdfrefxform1\\pdfrefxform3\\end",
        ));
        let second = stores
            .pdf_form_artifact(3)
            .expect("second form staged after the first");
        let artifact =
            tex_out::PageArtifact::from_bytes(second.bytes()).expect("parse form artifact");
        assert!(artifact.effects.iter().any(|effect| matches!(
            effect,
            tex_out::PageEffect::PdfColorStack { payload, .. } if payload == b"1 g"
        )));
        pdf_from_committed_artifacts(&mut stores, &first_run.committed_artifacts)
            .expect("serialize persistent form colors");

        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\setbox0=\\hbox{\\vrule width2pt height3pt}",
            "\\immediate\\pdfxform0",
            "\\shipout\\vbox{\\hrule width1pt height1pt}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("serialize immediate unreferenced form");
        assert!(
            pdf.windows(b"/Subtype/Form".len())
                .any(|w| w == b"/Subtype/Form")
        );
    }

    #[test]
    fn form_snap_coordinates_are_local_and_do_not_replace_page_grid() {
        let (stores, _) = run(concat!(
            "\\pdfoutput=1",
            "\\setbox0=\\vbox{\\kern10pt\\pdfsnaprefpoint\\kern20pt\\pdfsavepos}",
            "\\pdfxform0",
            "\\shipout\\vbox{\\kern5pt\\pdfsnaprefpoint\\pdfrefxform1}\\end",
        ));
        let form = stores
            .pdf_form_artifact(1)
            .expect("form traversal artifact");
        assert_eq!(
            form.last_position(),
            Some((Scaled::from_raw(0), Scaled::from_raw(0)))
        );
        assert_eq!(form.snap_reference(), (Scaled::from_raw(0), pt(20)));
        assert_eq!(stores.pdf_snap_reference(), (Scaled::from_raw(0), pt(5)));
    }

    #[test]
    fn failed_form_traversal_rolls_back_colors_positions_and_artifact() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let before = stores.pdf_last_position();
        let error = try_run_in(
            &mut stores,
            include_str!("../../../tests/corpus/tex_exec/pdf_form_traversal_diagnostics.tex"),
        )
        .expect_err("malformed form traversal must fail transactionally");
        assert_eq!(
            error.to_string(),
            "pdfTeX error: 1 unmatched \\pdfsave after form shipout"
        );
        assert!(stores.pdf_form_artifact(1).is_none());
        assert_eq!(stores.pdf_last_position(), before);
        let current = stores
            .apply_pdf_color_stack(
                0,
                tex_state::PdfColorStackTarget::Form,
                &tex_state::PdfColorStackAction::Current,
            )
            .expect("default form stack remains usable");
        assert_eq!(current.payload, b"0 g 0 G");
    }

    #[test]
    fn invalid_version_and_object_policy_recover_like_pdftex() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfmajorversion=0\\pdfminorversion=12",
            "\\pdfobjcompresslevel=9\\pdfdecimaldigits=9",
            "\\shipout\\vbox{\\hrule width10pt height5pt}\\end",
        ));
        let fixed = stores
            .fixed_pdf_output_parameters()
            .expect("shipout freezes recovered values");
        assert_eq!(fixed.major_version, 1);
        assert_eq!(fixed.minor_version, 4);
        assert_eq!(fixed.object_compress_level, 0);
        assert_eq!(fixed.decimal_digits, 4);
        let diagnostics = String::from_utf8_lossy(
            stores
                .world()
                .memory_terminal_output()
                .expect("memory terminal output"),
        );
        assert!(
            diagnostics.contains("pdfTeX error (invalid pdfmajorversion)"),
            "{diagnostics}"
        );
        assert!(
            diagnostics.contains("pdfTeX error (invalid pdfminorversion)"),
            "{diagnostics}"
        );
        assert!(
            diagnostics.contains("Object streams disabled now"),
            "{diagnostics}"
        );
        let bytes = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("recovered PDF assembles");
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(!bytes.windows(12).any(|window| window == b"/Type/ObjStm"));
    }

    #[test]
    fn pdf_graphics_literals_expand_at_the_selected_time_and_survive_artifacts() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\def\\value{ONE}",
            "\\setbox0=\\hbox{",
            "\\pdfliteral page{IMMEDIATE-\\value}",
            "\\pdfliteral shipout direct{DEFERRED-\\value}",
            "}",
            "\\def\\value{TWO}\\shipout\\box0\\end",
        ));
        let artifact = tex_out::PageArtifact::from_bytes(run.committed_artifacts[0].bytes())
            .expect("artifact parses");
        assert!(artifact.effects.iter().any(|effect| matches!(
            effect,
            tex_out::PageEffect::PdfLiteral { mode: tex_out::PdfLiteralMode::Page, payload }
                if payload == b"IMMEDIATE-ONE"
        )));
        assert!(artifact.effects.iter().any(|effect| matches!(
            effect,
            tex_out::PageEffect::PdfLiteral { mode: tex_out::PdfLiteralMode::Direct, payload }
                if payload == b"DEFERRED-TWO"
        )));

        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("graphics PDF assembles");
        assert!(
            pdf.windows(b"IMMEDIATE-ONE".len())
                .any(|w| w == b"IMMEDIATE-ONE")
        );
        assert!(
            pdf.windows(b"DEFERRED-TWO".len())
                .any(|w| w == b"DEFERRED-TWO")
        );
    }

    #[test]
    fn pdf_destinations_emit_typed_arrays_dictionaries_and_six_way_name_tree() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\shipout\\vbox{",
            "\\pdfdest name{z} fit \\pdfdest name{a} xyz zoom 0 ",
            "\\pdfdest name{m} fith \\pdfdest name{b} fitv ",
            "\\pdfdest name{q} fitb \\pdfdest name{c} fitbh ",
            "\\pdfdest name{x} fitbv \\pdfdest name{d} fitr width 2pt height 3pt depth 1pt ",
            "\\pdfdest num 42 fit}",
            "\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("destination PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("destination PDF parses");
        assert_eq!(parsed.get_pages().len(), 1);
        for marker in [
            b"/Dests".as_slice(),
            b"/Names",
            b"/Kids",
            b"/Limits",
            b"/FitR",
            b"/XYZ",
        ] {
            assert!(
                pdf.windows(marker.len()).any(|window| window == marker),
                "missing {:?}",
                String::from_utf8_lossy(marker)
            );
        }
    }

    #[test]
    fn pdf_outlines_emit_typed_hierarchy_actions_and_indirect_titles() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\pdfoutline attr{/F 2} user{/S /Named /N /NextPage} count 2 {Root}",
            "\\pdfoutline user{/S /Named /N /NextPage} count -1 {(Closed)}",
            "\\pdfoutline user{/S /Named /N /NextPage} {Leaf}",
            "\\pdfoutline user{/S /Named /N /NextPage} {Sibling}",
            "\\shipout\\hbox{}\\end",
        ));
        assert_eq!(stores.pdf_outlines().len(), 4);
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("outline PDF assembles");
        lopdf::Document::load_mem(&pdf).expect("outline PDF parses");
        for marker in [
            b"/Outlines".as_slice(),
            b"/First",
            b"/Last",
            b"/Parent",
            b"/Prev",
            b"/Next",
            b"/Count -1",
            b"/Title",
        ] {
            assert!(
                pdf.windows(marker.len()).any(|window| window == marker),
                "missing {:?}",
                String::from_utf8_lossy(marker)
            );
        }
    }

    #[test]
    fn pdf_graphics_matrix_and_state_lower_to_typed_ordered_operators() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\shipout\\hbox{\\pdfsave\\pdfsetmatrix{1 .25 -.5 1}",
            "\\pdfliteral direct{0.1 g}\\pdfrestore}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("graphics PDF assembles");
        let q = pdf.windows(3).position(|w| w == b"\nq\n").expect("typed q");
        let cm = pdf
            .windows(b"1 0.25 -0.5 1 0 0 cm".len())
            .position(|w| w == b"1 0.25 -0.5 1 0 0 cm")
            .expect("typed matrix");
        let literal = pdf.windows(5).position(|w| w == b"0.1 g").expect("literal");
        let restore = pdf.windows(2).position(|w| w == b"Q\n").expect("typed Q");
        assert!(q < cm && cm < literal && literal < restore);
    }

    #[test]
    fn pdf_color_stacks_mutate_at_traversal_and_restore_on_the_next_page() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0",
            "\\edef\\colors{\\pdfcolorstackinit page page{0 0 1 rg}}",
            "\\shipout\\vbox{\\pdfcolorstack\\colors push{1 0 0 rg}",
            "\\hrule width10pt height5pt}",
            "\\shipout\\vbox{\\pdfcolorstack\\colors pop",
            "\\hrule width10pt height5pt}\\end",
        ));
        let first = tex_out::PageArtifact::from_bytes(run.committed_artifacts[0].bytes())
            .expect("first artifact");
        assert!(matches!(
            &first.effects[0],
            tex_out::PageEffect::PdfColorStack { mode: tex_out::PdfLiteralMode::Page, payload, page_start: true }
                if payload == b"0 0 1 rg"
        ));
        assert!(first.effects.iter().any(|effect| matches!(
            effect,
            tex_out::PageEffect::PdfColorStack { payload, page_start: false, .. }
                if payload == b"1 0 0 rg"
        )));
        let second = tex_out::PageArtifact::from_bytes(run.committed_artifacts[1].bytes())
            .expect("second artifact");
        assert!(matches!(
            &second.effects[0],
            tex_out::PageEffect::PdfColorStack { payload, page_start: true, .. }
                if payload == b"1 0 0 rg"
        ));
        assert!(second.effects.iter().any(|effect| matches!(
            effect,
            tex_out::PageEffect::PdfColorStack { payload, page_start: false, .. }
                if payload == b"0 0 1 rg"
        )));

        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("color stack PDF assembles");
        assert!(pdf.windows(8).any(|window| window == b"1 0 0 rg"));
    }

    #[test]
    fn pdf_color_stack_diagnostics_recover_to_default_and_ignore_underflow() {
        let (stores, run) = run(concat!(
            "\\pdfoutput=1",
            "\\shipout\\vbox{\\pdfcolorstack-1 current",
            "\\pdfcolorstack999 current\\pdfcolorstack0 pop",
            "\\pdfcolorstack0 missing\\hrule width1pt height1pt}\\end",
        ));
        assert_eq!(run.committed_artifacts.len(), 1);
        let diagnostics = String::from_utf8_lossy(
            stores
                .world()
                .memory_terminal_output()
                .expect("terminal output"),
        );
        assert!(diagnostics.contains("Invalid negative color stack number"));
        assert!(diagnostics.contains("Unknown color stack number 999"));
        assert!(diagnostics.contains("pop empty color page stack 0"));
        assert!(diagnostics.contains("Color stack action is missing"));
    }

    #[test]
    fn pdf_save_position_publishes_only_after_pdf_and_dvi_shipout() {
        let mut pdf_stores = Universe::default();
        prepare_pdftex_run_stores(&mut pdf_stores);
        assert_eq!(
            pdf_stores.pdf_last_position(),
            (Scaled::from_raw(0), Scaled::from_raw(0))
        );
        let _ = run_in(
            &mut pdf_stores,
            concat!(
                "\\pdfoutput=1\\pdfpageheight=100pt",
                "\\pdfhorigin=10pt\\pdfvorigin=20pt",
                "\\setbox0=\\vbox{\\kern5pt\\hbox{\\kern7pt\\pdfsavepos}}",
                "\\shipout\\box0\\end",
            ),
        );
        assert_eq!(pdf_stores.pdf_last_position(), (pt(17), pt(75)),);

        let (dvi_stores, _) = run(concat!(
            "\\pdfoutput=0",
            "\\shipout\\vbox{\\kern5pt\\hbox{\\kern7pt\\pdfsavepos}}\\end",
        ));
        assert_eq!(
            dvi_stores.pdf_last_position(),
            (
                Scaled::from_raw(pt(7).raw() + 4_736_286),
                Scaled::from_raw(-4_736_286),
            ),
        );
    }

    #[test]
    fn pdf_save_position_observes_boxing_math_shifts_and_failed_shipout_commit() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmsy10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmsy10.tfm").to_vec(),
            )
            .expect("seed symbol font");
        stores
            .world_mut()
            .set_memory_file(
                "cmex10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmex10.tfm").to_vec(),
            )
            .expect("seed extension font");
        let run = run_in(
            &mut stores,
            concat!(
                "\\pdfoutput=1\\pdfpageheight=100pt",
                "\\pdfhorigin=10pt\\pdfvorigin=20pt",
                "\\font\\sym=cmsy10 \\font\\ext=cmex10 ",
                "\\textfont2=\\sym\\scriptfont2=\\sym\\scriptscriptfont2=\\sym",
                "\\textfont3=\\ext\\scriptfont3=\\ext\\scriptscriptfont3=\\ext",
                "\\message{initial=(\\the\\pdflastxpos,\\the\\pdflastypos)}",
                "\\setbox0=\\vbox{\\kern5pt\\hbox{\\kern7pt",
                "\\lower3pt\\hbox{$\\pdfsavepos$}}}",
                "\\message{boxed=(\\the\\pdflastxpos,\\the\\pdflastypos)}",
                "\\shipout\\box0",
                "\\message{shipped=(\\the\\pdflastxpos,\\the\\pdflastypos)}\\end",
            ),
        );
        assert_eq!(run.committed_artifacts.len(), 1);
        let artifact = tex_out::PageArtifact::from_bytes(run.committed_artifacts[0].bytes())
            .expect("save-position artifact parses");
        assert!(
            artifact
                .effects
                .iter()
                .any(|effect| matches!(effect, tex_out::PageEffect::PdfSavePosition)),
            "math save-position effect missing: {:?}",
            artifact.effects
        );
        assert_eq!(stores.pdf_last_position(), (pt(17), pt(72)));
        let output = String::from_utf8_lossy(
            stores
                .world()
                .memory_terminal_output()
                .expect("terminal output"),
        );
        assert!(output.contains("initial=(0,0)"), "{output}");
        assert!(output.contains("boxed=(0,0)"), "{output}");
        let before = stores.pdf_last_position();
        let error = try_run_in(
            &mut stores,
            "\\pdfoutput=1\\shipout\\hbox{\\pdfsavepos\\pdfsetmatrix{bad}}\\end",
        )
        .expect_err("malformed traversal fails after encountering savepos");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (\\pdfsetmatrix): Unrecognized format."
        );
        assert_eq!(stores.pdf_last_position(), before);
    }

    #[test]
    fn pdf_snap_y_and_compensation_move_only_vertical_traversal() {
        let (snapped, _) = run(concat!(
            "\\pdfoutput=1\\pdfpageheight=100pt\\pdfhorigin=0pt\\pdfvorigin=0pt",
            "\\shipout\\vbox{\\pdfsnaprefpoint\\kern6pt",
            "\\pdfsnapy 10pt plus10pt minus10pt\\pdfsavepos}\\end",
        ));
        assert_eq!(snapped.pdf_last_position().1, pt(90));

        let (compensated, _) = run(concat!(
            "\\pdfoutput=1\\pdfpageheight=100pt\\pdfhorigin=0pt\\pdfvorigin=0pt",
            "\\shipout\\vbox{\\pdfsnaprefpoint\\kern6pt\\pdfsnapycomp500",
            "\\pdfsavepos\\pdfsnapy 10pt plus10pt minus10pt}\\end",
        ));
        assert_eq!(compensated.pdf_last_position().1, pt(92),);

        let (horizontal, _) = run(concat!(
            "\\pdfoutput=1\\pdfpageheight=100pt\\pdfhorigin=0pt\\pdfvorigin=0pt",
            "\\shipout\\hbox{\\pdfsnaprefpoint\\kern6pt",
            "\\pdfsnapy 10pt plus10pt minus10pt\\pdfsavepos}\\end",
        ));
        assert_eq!(horizontal.pdf_last_position().0, pt(6));
    }

    #[test]
    fn pdf_snap_y_honors_reference_flex_limits_orders_and_forward_ties() {
        let cases = [
            ("\\kern6pt\\pdfsnapy 10pt plus4pt", 94),
            ("\\kern6pt\\pdfsnapy 10pt plus5pt", 90),
            ("\\kern6pt\\pdfsnapy 10pt minus6pt", 94),
            ("\\kern6pt\\pdfsnapy 10pt minus7pt", 100),
            ("\\kern6pt\\pdfsnapy 10pt plus1fil", 90),
            ("\\kern6pt\\pdfsnapy 10pt minus1fil", 100),
            ("\\kern5pt\\pdfsnapy 10pt plus6pt minus6pt", 90),
            (
                "\\kern3pt\\pdfsnaprefpoint\\kern6pt\\pdfsnapy 10pt plus5pt",
                87,
            ),
        ];
        for (material, expected_y) in cases {
            let source = format!(
                concat!(
                    "\\pdfoutput=1\\pdfpageheight=100pt",
                    "\\pdfhorigin=0pt\\pdfvorigin=0pt",
                    "\\shipout\\vbox{{\\pdfsnaprefpoint{material}\\pdfsavepos}}\\end",
                ),
                material = material,
            );
            let (stores, _) = run(&source);
            assert_eq!(
                stores.pdf_last_position().1,
                pt(expected_y),
                "material: {material}"
            );
        }

        let (stores, _) = run(concat!(
            "\\pdfoutput=1\\pdfpageheight=100pt\\pdfhorigin=0pt\\pdfvorigin=0pt",
            "\\shipout\\vbox{\\kern3pt\\pdfsnaprefpoint}",
            "\\shipout\\vbox{\\kern6pt\\pdfsnapy 10pt plus10pt minus10pt",
            "\\pdfsavepos}\\end",
        ));
        assert_eq!(stores.pdf_snap_reference(), (pt(0), pt(3)));
        assert_eq!(stores.pdf_last_position().1, pt(97));
    }

    #[test]
    fn pdf_save_position_and_snap_reference_replay_exactly() {
        let source = concat!(
            "\\pdfoutput=1\\pdfpageheight=100pt\\pdfhorigin=0pt\\pdfvorigin=0pt",
            "\\shipout\\vbox{\\kern3pt\\pdfsnaprefpoint\\kern6pt",
            "\\pdfsnapycomp500\\pdfsavepos",
            "\\pdfsnapy 10pt plus10pt minus10pt}\\end",
        );
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .begin_retained_session()
            .expect("retained test session starts");
        let before = stores.snapshot();
        let first = run_in(&mut stores, source);
        let first_artifact = first.committed_artifacts[0].bytes().to_vec();
        let first_position = stores.pdf_last_position();
        let first_reference = stores.pdf_snap_reference();
        let first_hash = stores.snapshot().state_hash();

        stores.rollback(&before);
        let second = run_in(&mut stores, source);
        assert_eq!(second.committed_artifacts[0].bytes(), first_artifact);
        assert_eq!(stores.pdf_last_position(), first_position);
        assert_eq!(stores.pdf_snap_reference(), first_reference);
        assert_eq!(stores.snapshot().state_hash(), first_hash);
        assert_eq!(first_position, (pt(0), pt(89)));
        assert_eq!(first_reference, (pt(0), pt(3)));
    }

    #[test]
    fn pdf_snap_y_compensation_clamps_and_dvi_plan_matches_equivalent_kern() {
        for (ratio, expected_y) in [(-1, 94), (0, 94), (500, 92), (1000, 90), (1001, 90)] {
            let source = format!(
                concat!(
                    "\\pdfoutput=1\\pdfpageheight=100pt",
                    "\\pdfhorigin=0pt\\pdfvorigin=0pt",
                    "\\shipout\\vbox{{\\pdfsnaprefpoint\\kern6pt",
                    "\\pdfsnapycomp{ratio}\\pdfsavepos",
                    "\\pdfsnapy 10pt plus10pt minus10pt}}\\end",
                ),
                ratio = ratio,
            );
            let (stores, _) = run(&source);
            assert_eq!(
                stores.pdf_last_position().1,
                pt(expected_y),
                "ratio {ratio}"
            );
        }

        let (_, snapped) = run(concat!(
            "\\pdfoutput=1\\pdfhorigin=0pt\\pdfvorigin=0pt",
            "\\shipout\\vbox{\\pdfsnaprefpoint\\kern6pt",
            "\\pdfsnapy 10pt plus10pt minus10pt\\hrule width1pt height1pt}\\end",
        ));
        let (_, explicit) = run(concat!(
            "\\pdfoutput=1\\pdfhorigin=0pt\\pdfvorigin=0pt",
            "\\shipout\\vbox{\\kern10pt\\hrule width1pt height1pt}\\end",
        ));
        let snapped_dvi = dvi_from_page_plans(&snapped.dvi_pages).expect("snapped DVI");
        let explicit_dvi = dvi_from_page_plans(&explicit.dvi_pages).expect("explicit-kern DVI");
        let snapped_file =
            tex_out::dvi::disasm::DviFile::parse(&snapped_dvi).expect("snapped DVI parses");
        let explicit_file =
            tex_out::dvi::disasm::DviFile::parse(&explicit_dvi).expect("explicit-kern DVI parses");
        let snapped_page = &snapped_file.pages[0];
        let explicit_page = &explicit_file.pages[0];
        assert_eq!(
            &snapped_dvi[snapped_page.bop_offset..snapped_page.eop_end.expect("snapped eop")],
            &explicit_dvi[explicit_page.bop_offset..explicit_page.eop_end.expect("explicit eop")],
            "snapping emits the same DVI page program as its equivalent explicit kern"
        );
    }

    #[test]
    fn pdf_snap_y_rejects_negative_natural_glue_without_publishing() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = try_run_in(
            &mut stores,
            "\\pdfoutput=1\\shipout\\vbox{\\pdfsnapy -1pt plus2pt}\\end",
        )
        .expect_err("negative snap glue is fatal");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (\\pdfsnapy): negative glue"
        );
        assert_eq!(stores.pdf_last_position(), (pt(0), pt(0)));
        assert!(stores.pdf_pages().is_empty());
    }

    #[test]
    fn pdf_graphics_reports_matrix_and_save_restore_failures_at_traversal() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = try_run_in(
            &mut stores,
            "\\pdfoutput=1\\shipout\\hbox{\\pdfsetmatrix{1 0 0}}\\end",
        )
        .expect_err("malformed matrix fails during shipout");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (\\pdfsetmatrix): Unrecognized format."
        );

        let (_stores, restore_run) = run("\\pdfoutput=1\\shipout\\hbox{\\pdfrestore}\\end");
        let artifact =
            tex_out::PageArtifact::from_bytes(restore_run.committed_artifacts[0].bytes())
                .expect("restore artifact parses");
        let positioned = tex_out::positioned::lower_page(&artifact, 0)
            .expect("missing restore remains a warning");
        assert_eq!(positioned.diagnostics, ["\\pdfrestore: missing \\pdfsave"]);

        let (_stores, misplaced_run) =
            run("\\pdfoutput=1\\shipout\\hbox{\\pdfsave\\kern1sp\\pdfrestore}\\end");
        let artifact =
            tex_out::PageArtifact::from_bytes(misplaced_run.committed_artifacts[0].bytes())
                .expect("misplaced restore artifact parses");
        let positioned = tex_out::positioned::lower_page(&artifact, 0)
            .expect("misplaced restore remains a warning");
        assert_eq!(
            positioned.diagnostics,
            ["Misplaced \\pdfrestore by (1sp, 0sp)"]
        );

        let (mut stores, save_run) = run("\\pdfoutput=1\\shipout\\hbox{\\pdfsave}\\end");
        assert!(matches!(
            pdf_from_committed_artifacts(&mut stores, &save_run.committed_artifacts),
            Err(PdfBuildError::Positioned(
                PositionedError::UnmatchedPdfSaves { count: 1 }
            ))
        ));
    }

    #[test]
    fn running_threads_add_vbox_beads_and_missing_actions_get_fixed_beads() {
        let (mut stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfpagewidth=40pt\\pdfpageheight=40pt",
            "\\pdfhorigin=0pt\\pdfvorigin=0pt",
            "\\pdfoutline thread num 99 {Missing}",
            "\\shipout\\vbox{\\pdfstartthread name{running}",
            "\\vbox{\\hrule width3pt height2pt}",
            "\\vbox{\\hrule width4pt height2pt}\\pdfendthread}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
            .expect("thread PDF assembles");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Threads"));
        let document = lopdf::Document::load_mem(&pdf).expect("thread PDF parses");
        assert_eq!(
            document
                .objects
                .values()
                .filter_map(|object| object.as_dict().ok())
                .filter(|dict| {
                    dict.has(b"V") && dict.has(b"N") && dict.has(b"P") && dict.has(b"R")
                })
                .count(),
            4
        );
        assert_eq!(stores.pdf_threads().len(), 2);
        assert!(stores.pdf_threads()[0].beads().is_empty());
        assert_eq!(stores.pdf_threads()[1].beads().len(), 1);
    }

    #[test]
    fn pdf_graphics_are_rejected_when_pdf_output_is_disabled() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = try_run_in(&mut stores, "\\pdfoutput=0\\pdfliteral{}\\end")
            .expect_err("DVI-mode literal is rejected");
        assert!(error.to_string().contains("PDF output is disabled"));
    }

    #[test]
    fn pdf_literals_are_legal_in_vertical_horizontal_and_math_modes() {
        for source in [
            "\\pdfoutput=1\\shipout\\vbox{\\pdfliteral direct{V}}\\end",
            "\\pdfoutput=1\\shipout\\hbox{\\pdfliteral direct{H}}\\end",
            "\\pdfoutput=1\\shipout\\hbox{$\\pdfliteral direct{M}$}\\end",
        ] {
            let (mut stores, run) = run(source);
            pdf_from_committed_artifacts(&mut stores, &run.committed_artifacts)
                .expect("mode-independent literal assembles");
        }
    }
}
