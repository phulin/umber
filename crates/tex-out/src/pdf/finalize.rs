//! Pure PDF finalization from a complete detached input.

use super::{
    PdfAnnotationAction, PdfAnnotationObject, PdfAnnotationType, PdfBeadObject,
    PdfContentOperation, PdfContentRectangle, PdfContentTextRun, PdfDestinationAction,
    PdfDestinationActionKind, PdfDestinationNameTree, PdfDestinationNameTreeChildren,
    PdfDestinationPage, PdfDestinationStructure, PdfDestinationTarget, PdfDestinationView,
    PdfDictionary, PdfExplicitDestination, PdfFinalizationInput, PdfFontInput, PdfFontProgramInput,
    PdfImageColorSpace, PdfImageFilter, PdfImageGammaInput, PdfImageMetadataInput, PdfImageXObject,
    PdfIndirectObject, PdfModelError, PdfName, PdfNamesObject, PdfNumber, PdfObject, PdfObjectId,
    PdfOutlineItemObject, PdfOutlineObject, PdfPageRotationInput, PdfRasterColorSpaceInput,
    PdfRasterFormatInput, PdfSerializeError, PdfThreadObject, PdfTrailer, PdfValue, PdfVersion,
    UnvalidatedPdfDocument, ordered_page_content, page_content,
};
use crate::positioned::{BoxKind, PositionedBox, PositionedError, PositionedEvent, PositionedPage};
use crate::{ContentHash, PageArtifact, PageNode};
use md5::{Digest, Md5};
use tex_arith::Scaled;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::{Read, Write};

#[derive(Clone, Copy)]
struct PdfFormTraversalLimits {
    max_depth: usize,
    max_work: usize,
}

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

/// Successful detached finalization, including ordered diagnostics that the
/// host adapter may publish only after final bytes have been accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfFinalizationOutput {
    pub bytes: Vec<u8>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Copy)]
struct FinalizationParameters {
    major_version: i32,
    decimal_digits: i32,
    unique_resource_names: i32,
}

impl super::PdfCommittedPageInput {
    fn resources_object(&self) -> u32 {
        self.resources_object
    }
    fn contents_object(&self) -> u32 {
        self.contents_object
    }
    fn page_object(&self) -> u32 {
        self.page_object
    }
    fn h_origin(&self) -> Scaled {
        self.h_origin
    }
    fn v_origin(&self) -> Scaled {
        self.v_origin
    }
    fn width(&self) -> Scaled {
        self.width
    }
    fn height(&self) -> Scaled {
        self.height
    }
    fn link_margin(&self) -> Scaled {
        self.link_margin
    }
    fn omit_procset(&self) -> i32 {
        self.omit_procset
    }
}

impl super::PdfFormInput {
    fn object(&self) -> u32 {
        self.object
    }
    fn resource(&self) -> u32 {
        self.resource
    }
    fn width(&self) -> Scaled {
        self.width
    }
    fn height(&self) -> Scaled {
        self.height
    }
    fn depth(&self) -> Scaled {
        self.depth
    }
}

impl PdfFinalizationInput {
    fn pdf_outlines(&self) -> &[super::PdfOutlineInput] {
        &self.navigation.outlines
    }
    fn pdf_destinations(&self, structure: bool) -> &[super::PdfDestinationInput] {
        if structure {
            &self.navigation.structure_destinations
        } else {
            &self.navigation.destinations
        }
    }
    fn pdf_annotations(&self) -> &[super::PdfAnnotationInput] {
        &self.navigation.annotations
    }
    fn pdf_links(&self) -> &[super::PdfLinkInput] {
        &self.navigation.links
    }
    fn pdf_threads(&self) -> &[super::PdfThreadInput] {
        &self.navigation.threads
    }
    fn pdf_destination(
        &self,
        identity: &super::PdfDestinationIdentityInput,
        structure: bool,
    ) -> Option<&super::PdfDestinationInput> {
        self.pdf_destinations(structure)
            .iter()
            .find(|record| &record.identity == identity)
    }
}

impl super::PdfLinkInput {
    fn object(&self) -> u32 {
        self.object
    }
    fn dimensions(&self) -> super::PdfAnnotationDimensionsInput {
        self.dimensions
    }
}

impl super::PdfThreadInput {
    fn object(&self) -> u32 {
        self.object
    }
    fn beads(&self) -> &[super::PdfThreadBeadInput] {
        &self.beads
    }
    fn identity(&self) -> &super::PdfDestinationIdentityInput {
        &self.identity
    }
}

impl super::PdfOutlineInput {
    fn count(&self) -> i32 {
        self.count
    }
    fn action_object(&self) -> u32 {
        self.action_object
    }
    fn item_object(&self) -> u32 {
        self.item_object
    }
    fn title_object(&self) -> u32 {
        self.title_object
    }
    fn action(&self) -> &super::PdfActionInput {
        &self.action
    }
}

impl super::PdfDestinationInput {
    fn object(&self) -> u32 {
        self.object
    }
    fn identity(&self) -> &super::PdfDestinationIdentityInput {
        &self.identity
    }
}

impl super::PdfThreadBeadInput {
    fn bead_object(&self) -> u32 {
        self.bead_object
    }
    fn rectangle_object(&self) -> u32 {
        self.rectangle_object
    }
}

impl super::PdfAnnotationInput {
    fn object(&self) -> u32 {
        self.object
    }
}

/// Purely validates and lowers one complete detached PDF input.
#[allow(clippy::disallowed_methods)] // Optional process telemetry is observational only.
pub fn finalize_pdf(input: &PdfFinalizationInput) -> Result<PdfFinalizationOutput, PdfBuildError> {
    let total_started = std::time::Instant::now();
    let parameters = FinalizationParameters {
        major_version: i32::from(input.document.version.0),
        decimal_digits: i32::from(input.document.decimal_digits),
        unique_resource_names: i32::from(input.document.unique_resource_names),
    };
    let version = PdfVersion::new(input.document.version.0, input.document.version.1)?;
    let options = input.document.serialization;
    let page_records = &input.pages;
    let map_started = std::time::Instant::now();
    let resolved_font_map = input
        .fonts
        .values()
        .filter_map(|font| font.map_entry.as_ref())
        .map(|entry| (entry.tex_name.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    let map_resolve_ns = map_started.elapsed().as_nanos();
    let positioning_started = std::time::Instant::now();
    let mut positioned_pages = positioned_pages(input)?;
    let page_count = positioned_pages.len();
    let positioned_form_entries = positioned_forms(input)?;
    let positioned_form_objects = positioned_form_entries
        .iter()
        .map(|(object, _)| *object)
        .collect::<Vec<_>>();
    positioned_pages.extend(
        positioned_form_entries
            .into_iter()
            .map(|(_, positioned)| positioned),
    );
    let positioning_ns = positioning_started.elapsed().as_nanos();
    let vf_started = std::time::Instant::now();
    super::vf::lower_pages(input, &mut positioned_pages)?;
    let vf_ns = vf_started.elapsed().as_nanos();
    let positioned_forms = positioned_pages.split_off(page_count);
    let positioned_forms = positioned_form_objects
        .into_iter()
        .zip(positioned_forms)
        .collect::<BTreeMap<_, _>>();
    validate_form_graph(
        input,
        &positioned_pages,
        &positioned_forms,
        PdfFormTraversalLimits {
            max_depth: input.limits.max_form_depth,
            max_work: input.limits.max_form_work,
        },
    )?;
    let font_usage_started = std::time::Instant::now();
    let font_usage = collect_font_usage(input, &positioned_pages, &positioned_forms)?;
    let font_usage_ns = font_usage_started.elapsed().as_nanos();
    let destinations_started = std::time::Instant::now();
    let shipped_destinations = lower_page_destinations(
        input,
        page_records,
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
        lower_page_annotations(input, &positioned_pages, &page_link_margins)?;
    let annotations_ns = annotations_started.elapsed().as_nanos();
    let document_ids = input.allocation.document;
    let catalog_id = object_id(document_ids.catalog)?;
    let pages_id = object_id(document_ids.pages)?;
    let mut next_object = input.allocation.next_object;
    assign_annotation_objects(&mut page_annotations, &mut next_object)?;
    let outline_output = outline_objects(input, page_records, &mut next_object)?;
    let destination_output =
        destination_objects(input, page_records, shipped_destinations, &mut next_object)?;
    let thread_output = thread_objects(
        &input.navigation.threads,
        &positioned_pages,
        page_records,
        parameters.decimal_digits,
        &mut next_object,
    )?;
    let mut objects = Vec::with_capacity(2 + page_records.len() * 3 + input.raw_objects.len() + 2);
    let mut kids = Vec::with_capacity(page_records.len());
    let mut emitted_fonts = std::collections::BTreeSet::new();
    let mut interword_space_enabled = false;
    let mut fallback_space_font = None;
    let mut diagnostics = Vec::new();
    let mut referenced_forms = BTreeSet::<u32>::new();
    let object_started = std::time::Instant::now();
    let mut font_embed_ns = 0_u128;
    referenced_forms.extend(
        input
            .forms
            .values()
            .filter(|form| form.immediate)
            .map(|form| form.object),
    );

    let mut catalog = PdfDictionary::new();
    catalog.insert("Type", PdfValue::Name("Catalog".into()))?;
    catalog.insert("Pages", PdfValue::Reference(pages_id))?;
    if let Some(names) = document_ids.names {
        catalog.insert("Names", PdfValue::Reference(object_id(names)?))?;
    }
    if let Some(outlines) = outline_output.root {
        catalog.insert("Outlines", PdfValue::Reference(outlines))?;
    }
    if let Some(threads) = thread_output.list {
        catalog.insert("Threads", PdfValue::Reference(threads))?;
    }
    let open_action = input.document.metadata.open_action.as_ref();
    if let Some(action) = open_action {
        catalog.insert("OpenAction", PdfValue::Reference(object_id(action.object)?))?;
    }
    catalog.set_raw_entries(input.document.metadata.catalog_entries.clone());
    objects.push(indirect_dictionary(catalog_id, catalog));

    if let Some(action) = open_action {
        objects.push(PdfIndirectObject {
            id: object_id(action.object)?,
            object: PdfObject::Action(detached_link_action(input, &action.action, page_records)?),
        });
    }

    if let Some(names) = document_ids.names {
        objects.push(PdfIndirectObject {
            id: object_id(names)?,
            object: PdfObject::Names(PdfNamesObject {
                destinations: destination_output.name_tree_root,
                raw_entries: input.document.metadata.names_entries.clone(),
            }),
        });
    }
    objects.extend(outline_output.objects);
    objects.extend(destination_output.destinations);
    objects.extend(destination_output.name_tree);
    objects.extend(thread_output.objects.clone());

    if let Some(info) = document_ids.info {
        let mut dictionary = document_info_dictionary(&input.document.metadata)?;
        dictionary.set_raw_entries(input.document.metadata.info_entries.clone());
        objects.push(indirect_dictionary(object_id(info)?, dictionary));
    }

    for record in &input.raw_objects {
        if !record.immediate && !record.referenced {
            continue;
        }
        let data =
            record
                .payload
                .as_ref()
                .ok_or(PdfBuildError::ReferencedRawObjectUninitialized(
                    record.object,
                ))?;
        let object = if let super::PdfRawObjectPayloadInput::Stream { entries, data } = data {
            let mut dictionary = PdfDictionary::new();
            dictionary.set_raw_entries(entries.clone());
            PdfObject::Stream {
                dictionary,
                data: data.to_vec(),
            }
        } else {
            let super::PdfRawObjectPayloadInput::Value(payload) = data else {
                unreachable!()
            };
            PdfObject::Raw(payload.clone())
        };
        objects.push(PdfIndirectObject {
            id: object_id(record.object)?,
            object,
        });
    }

    let mut pdf_image_groups = BTreeMap::<u32, Option<PdfObjectId>>::new();
    let mut pdf_image_objects = BTreeMap::<u32, PdfObjectId>::new();
    let mut lowered_images =
        HashMap::<(ContentHash, PdfImageMetadataInput, Option<u32>), PdfObjectId>::new();
    let image_import_started = std::time::Instant::now();
    let mut image_telemetry = ImageImportTelemetry::default();
    let mut image_count = 0usize;
    let mut raster_image_count = 0usize;
    let mut pdf_image_count = 0usize;
    let mut image_input_bytes = 0usize;
    let mut unique_image_identities = BTreeSet::new();
    for image in input.images.values() {
        image_count += 1;
        image_input_bytes = image_input_bytes.saturating_add(image.bytes.len());
        unique_image_identities.insert(image.identity);
        let cache_key = (image.identity, image.metadata, image.color_space_object);
        if matches!(image.metadata, PdfImageMetadataInput::Raster { .. })
            && let Some(&object) = lowered_images.get(&cache_key)
        {
            image_telemetry.cache_hits += 1;
            pdf_image_objects.insert(image.object, object);
            continue;
        }
        match image.metadata {
            PdfImageMetadataInput::Raster {
                format,
                width,
                height,
                bits_per_component,
                color_space,
                alpha,
                png_color_type,
            } => {
                let metadata = RasterMetadata {
                    format,
                    width,
                    height,
                    bits_per_component,
                    color_space,
                    alpha,
                    png_color_type,
                };
                raster_image_count += 1;
                let (color_data, filter, bits, color_space, alpha_data) = raster_image_streams(
                    &image.bytes,
                    metadata,
                    input.document.image_gamma,
                    input.document.version,
                    &mut image_telemetry,
                )?;
                let color_space = image.color_space_object.map_or(color_space, |object| {
                    PdfImageColorSpace::IndirectObject(object as i32)
                });
                let image_object = object_id(image.object)?;
                objects.push(PdfIndirectObject {
                    id: image_object,
                    object: PdfObject::ImageXObject {
                        image: PdfImageXObject {
                            width: metadata.width,
                            height: metadata.height,
                            bits_per_component: bits,
                            color_space,
                            filter,
                            soft_mask: image.mask_object.map(object_id).transpose()?,
                        },
                        data: color_data,
                    },
                });
                if let Some((alpha_data, alpha_filter)) = alpha_data {
                    let mask = image.mask_object.ok_or(PdfBuildError::InvalidPng)?;
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
                pdf_image_objects.insert(image.object, image_object);
                lowered_images.insert(cache_key, image_object);
            }
            PdfImageMetadataInput::PdfPage {
                page_box,
                rotation,
                page,
                ..
            } => {
                pdf_image_count += 1;
                let imported = import_pdf_page(
                    image,
                    page,
                    page_box,
                    rotation,
                    &mut next_object,
                    input.limits,
                )?;
                let image_object = imported.form.id;
                pdf_image_groups.insert(image.object, imported.group);
                pdf_image_objects.insert(image.object, image_object);
                objects.extend(imported.dependencies);
                objects.push(imported.form);
            }
        }
    }
    let image_import_ns = image_import_started.elapsed().as_nanos();

    for (page_index, record) in page_records.iter().enumerate() {
        let artifact = PageArtifact::from_bytes(&record.artifact_bytes)?;
        let positioned = positioned_pages[page_index].clone();
        let (page_width, page_height) = pdf_page_extents(&artifact, record)?;
        let mut content_operations = Vec::new();
        let mut page_forms = BTreeMap::<u32, PdfObjectId>::new();
        let mut page_images = BTreeMap::<Vec<u8>, PdfObjectId>::new();
        let mut page_group_selected = false;
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
                    let resource = input
                        .fonts
                        .get(&font.semantic_identity)
                        .ok_or(PdfBuildError::MissingFontResource(font.name.clone()))?;
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
                                let mapped = resolved_font_map.contains_key(font.name.as_bytes());
                                let ids = if mapped {
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
                                    resource,
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
                    let baseline = scaled_to_bp_f32(
                        page_height
                            .checked_sub(run.baseline)
                            .and_then(|value| value.checked_sub(record.v_origin()))
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        parameters.decimal_digits,
                    );
                    let font_size = scaled_to_bp_f32(font.at_size, parameters.decimal_digits);
                    let horizontal_scale = font_horizontal_scale(&font.construction);
                    let explicit_space = font_has_explicit_space(resource);
                    let mut segment = Vec::new();
                    let mut segment_x = None;
                    for ((unit, position), physical_code) in run
                        .units
                        .iter()
                        .zip(&run.positions)
                        .zip(&run.physical_codes)
                    {
                        match unit {
                            crate::positioned::TextUnit::Code(_) => {
                                if let Some(code) = physical_code {
                                    segment_x.get_or_insert(*position);
                                    segment.push(*code);
                                }
                            }
                            crate::positioned::TextUnit::Space => {
                                if !segment.is_empty() {
                                    let advance = scalable_text_advance(
                                        resource,
                                        font,
                                        &segment,
                                        font_size,
                                        horizontal_scale,
                                    );
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
                                            horizontal_scale,
                                            bytes: std::mem::take(&mut segment),
                                            advance,
                                        },
                                    ));
                                }
                                if interword_space_enabled {
                                    let (font_name, space_size, space_horizontal_scale) =
                                        if explicit_space {
                                            (resource_name.clone(), font_size, horizontal_scale)
                                        } else {
                                            ensure_fallback_space_font(
                                                &record.space_font_name,
                                                &mut next_object,
                                                &mut objects,
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
                                            baseline,
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
                            horizontal_scale,
                            advance: scalable_text_advance(
                                resource,
                                font,
                                &segment,
                                font_size,
                                horizontal_scale,
                            ),
                            bytes: segment,
                        }));
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
                        crate::PageEffect::PdfLiteral { mode, payload } => {
                            PdfContentOperation::Literal {
                                mode,
                                x,
                                y,
                                bytes: payload,
                            }
                        }
                        crate::PageEffect::PdfSetMatrix { payload } => {
                            PdfContentOperation::SetMatrix {
                                x,
                                y,
                                matrix: parse_pdf_matrix(&payload)?,
                            }
                        }
                        crate::PageEffect::PdfSave => PdfContentOperation::Save { x, y },
                        crate::PageEffect::PdfRestore => PdfContentOperation::Restore { x, y },
                        crate::PageEffect::PdfColorStack { mode, payload, .. } => {
                            PdfContentOperation::ColorStack {
                                mode,
                                x,
                                y,
                                bytes: payload,
                            }
                        }
                        crate::PageEffect::PdfRefXForm { object, .. } => {
                            let form = input
                                .forms
                                .get(&object)
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
                            let y = page_height
                                .checked_sub(graphics.y)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_sub(depth))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let (placed_width, placed_height) = match image.metadata {
                                PdfImageMetadataInput::PdfPage {
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
                                    let (natural_width, natural_height) =
                                        if rotation_swaps_axes(rotation) {
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
                                PdfImageMetadataInput::Raster { .. } => (
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
        resources.set_raw_entries(record.resource_entries.clone());
        objects.push(indirect_dictionary(resources_id, resources));
        objects.push(PdfIndirectObject {
            id: contents_id,
            object: PdfObject::Stream {
                dictionary: PdfDictionary::new(),
                data: if has_pdf_graphics {
                    ordered_page_content(&content_operations)
                } else {
                    page_content(&content_operations)
                },
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
                        crate::PageEffect::PdfLiteral { mode, payload } => {
                            PdfContentOperation::Literal {
                                mode,
                                x,
                                y,
                                bytes: payload,
                            }
                        }
                        crate::PageEffect::PdfSetMatrix { payload } => {
                            PdfContentOperation::SetMatrix {
                                x,
                                y,
                                matrix: parse_pdf_matrix(&payload)?,
                            }
                        }
                        crate::PageEffect::PdfSave => PdfContentOperation::Save { x, y },
                        crate::PageEffect::PdfRestore => PdfContentOperation::Restore { x, y },
                        crate::PageEffect::PdfColorStack { mode, payload, .. } => {
                            PdfContentOperation::ColorStack {
                                mode,
                                x,
                                y,
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
                    let resource = input
                        .fonts
                        .get(&font.semantic_identity)
                        .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
                    let resource_name = format!("F{}", resource.resource_number).into_bytes();
                    let font_id = object_id(resource.object_number)?;
                    form_fonts.insert(resource.resource_number, font_id);
                    if emitted_fonts.insert(resource.object_number) {
                        let used_codes = font_usage
                            .get(&resource.object_number)
                            .ok_or_else(|| PdfBuildError::MissingFontUsage(font.name.clone()))?;
                        let mapped = resolved_font_map.contains_key(font.name.as_bytes());
                        let ids = if mapped {
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
                            resource,
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
                        baseline: scaled_to_bp_f32(
                            total_height
                                .checked_sub(run.baseline)
                                .ok_or(PdfBuildError::PageGeometryOverflow)?,
                            parameters.decimal_digits,
                        ),
                        font_name: resource_name,
                        font_size: scaled_to_bp_f32(font.at_size, parameters.decimal_digits),
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

    let mut pages = PdfDictionary::new();
    pages.insert("Type", PdfValue::Name("Pages".into()))?;
    pages.insert("Count", PdfValue::Integer(page_records.len() as i64))?;
    pages.insert("Kids", PdfValue::Array(kids))?;
    pages.set_raw_entries(input.document.pages_entries.clone());
    objects.push(indirect_dictionary(pages_id, pages));

    let trailer_id = input.document.metadata.trailer_id.clone();
    let file_id = if trailer_id.is_empty() {
        None
    } else {
        let digest = Md5::digest(&trailer_id).to_vec();
        Some((digest.clone(), digest))
    };

    let object_ns = object_started.elapsed().as_nanos();
    let object_count = objects.len();
    if objects
        .iter()
        .any(|object| object.id.get() > input.limits.max_object_id)
    {
        return Err(PdfBuildError::ObjectCapacity);
    }
    let validation_started = std::time::Instant::now();
    let document = UnvalidatedPdfDocument {
        version,
        catalog: catalog_id,
        objects,
        trailer: PdfTrailer {
            info: document_ids.info.map(object_id).transpose()?,
            file_id,
            raw_entries: input.document.metadata.trailer_entries.clone(),
        },
    }
    .validate()?;
    let validation_ns = validation_started.elapsed().as_nanos();
    let serialization_started = std::time::Instant::now();
    let bytes = document.to_pdf_bytes_with_options(options)?;
    if std::env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
        eprintln!(
            "PDF_TELEMETRY map_resolve_ns={} positioning_ns={} vf_ns={} font_usage_ns={} destinations_ns={} annotations_ns={} object_ns={} image_import_ns={} image_parse_copy_ns={} image_decode_ns={} image_transform_ns={} image_encode_ns={} image_cache_hits={} image_pixels={} image_rows={} image_raw_bytes={} image_color_bytes={} image_alpha_bytes={} image_peak_row_bytes={} image_deflate_level={} image_deflate_window_bits={} font_embed_ns={} validation_ns={} serialization_ns={} total_ns={} pages={} forms={} fonts={} images={} raster_images={} pdf_images={} image_input_bytes={} unique_images={} lowered_images={} objects={} output_bytes={}",
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
            image_telemetry.pixels,
            image_telemetry.rows,
            image_telemetry.raw_bytes,
            image_telemetry.color_bytes,
            image_telemetry.alpha_bytes,
            image_telemetry.peak_row_bytes,
            DERIVED_IMAGE_COMPRESSION_LEVEL,
            DERIVED_IMAGE_WINDOW_BITS,
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
    Ok(PdfFinalizationOutput { bytes, diagnostics })
}

fn validate_form_graph(
    input: &PdfFinalizationInput,
    pages: &[PositionedPage],
    forms: &BTreeMap<u32, PositionedPage>,
    limits: PdfFormTraversalLimits,
) -> Result<(), PdfBuildError> {
    fn references(page: &PositionedPage) -> impl Iterator<Item = u32> + '_ {
        page.events.iter().filter_map(|event| match event {
            PositionedEvent::PdfGraphics(graphics) => match graphics.effect {
                crate::PageEffect::PdfRefXForm { object, .. } => Some(object),
                _ => None,
            },
            _ => None,
        })
    }

    struct Traversal<'a> {
        input: &'a PdfFinalizationInput,
        forms: &'a BTreeMap<u32, PositionedPage>,
        limits: PdfFormTraversalLimits,
        work: usize,
        active: BTreeSet<u32>,
        complete: BTreeSet<u32>,
    }

    impl Traversal<'_> {
        fn visit(&mut self, object: u32, depth: usize) -> Result<(), PdfBuildError> {
            self.work =
                self.work
                    .checked_add(1)
                    .ok_or(PdfBuildError::FormTraversalWorkExceeded(
                        self.limits.max_work,
                    ))?;
            if self.work > self.limits.max_work {
                return Err(PdfBuildError::FormTraversalWorkExceeded(
                    self.limits.max_work,
                ));
            }
            if depth > self.limits.max_depth {
                return Err(PdfBuildError::FormTraversalDepthExceeded(
                    self.limits.max_depth,
                ));
            }
            if self.active.contains(&object) {
                return Err(PdfBuildError::FormCycle(object));
            }
            if self.complete.contains(&object) {
                return Ok(());
            }
            self.input
                .forms
                .get(&object)
                .ok_or(PdfBuildError::ReferencedFormNotFound(object))?;
            let page = self
                .forms
                .get(&object)
                .ok_or(PdfBuildError::MissingFormArtifact(object))?;
            let nested = references(page).collect::<Vec<_>>();
            self.active.insert(object);
            for nested in nested {
                if nested == object {
                    return Err(PdfBuildError::RecursiveForm(object));
                }
                self.visit(nested, depth + 1)?;
            }
            self.active.remove(&object);
            self.complete.insert(object);
            Ok(())
        }
    }

    let roots = pages.iter().flat_map(references).collect::<BTreeSet<_>>();
    let mut traversal = Traversal {
        input,
        forms,
        limits,
        work: 0,
        active: BTreeSet::new(),
        complete: BTreeSet::new(),
    };
    for object in roots {
        traversal.visit(object, 1)?;
    }
    Ok(())
}

fn collect_font_usage(
    input: &PdfFinalizationInput,
    positioned_pages: &[PositionedPage],
    positioned_forms: &BTreeMap<u32, PositionedPage>,
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
        let resource = input
            .fonts
            .get(&font.semantic_identity)
            .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
        font_metadata.insert(
            font.semantic_identity,
            (
                resource,
                resource.included_codes.clone(),
                font_has_explicit_space(resource),
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
                        crate::PdfAccessibilityEffect::InterwordSpaceOn => {
                            interword_space_enabled = true;
                        }
                        crate::PdfAccessibilityEffect::InterwordSpaceOff => {
                            interword_space_enabled = false;
                        }
                        crate::PdfAccessibilityEffect::FakeSpace => {}
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
            let codes = usage.entry(resource.object_number).or_default();
            let explicit_space = interword_space_enabled && *has_explicit_space;
            codes.extend(run.units.iter().zip(&run.physical_codes).filter_map(
                |(unit, physical_code)| match unit {
                    crate::positioned::TextUnit::Code(_) => *physical_code,
                    crate::positioned::TextUnit::Space if explicit_space => Some(b' '),
                    crate::positioned::TextUnit::Space => None,
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
            let codes = usage.entry(resource.object_number).or_default();
            for unit in &run.units {
                let code = match unit {
                    crate::positioned::TextUnit::Code(code) => {
                        u8::try_from(*code).map_err(|_| {
                            PdfBuildError::PositionedCharacterOutOfRange {
                                font: font.name.clone(),
                                code: *code,
                            }
                        })?
                    }
                    crate::positioned::TextUnit::Space => b' ',
                };
                codes.insert(code);
            }
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
struct ActiveShippedLink<'a> {
    record: &'a super::PdfLinkInput,
    depth: u32,
    candidate: Option<(u32, Scaled)>,
}

fn positioned_pages(input: &PdfFinalizationInput) -> Result<Vec<PositionedPage>, PdfBuildError> {
    input
        .pages
        .iter()
        .enumerate()
        .map(|(page_index, record)| {
            let artifact = PageArtifact::from_bytes(&record.artifact_bytes)?;
            Ok(crate::positioned::lower_page(&artifact, page_index as u32)?)
        })
        .collect()
}

fn positioned_forms(
    input: &PdfFinalizationInput,
) -> Result<Vec<(u32, PositionedPage)>, PdfBuildError> {
    input
        .forms
        .values()
        .map(|form| {
            let artifact = PageArtifact::from_bytes(&form.artifact_bytes)?;
            Ok((form.object, crate::positioned::lower_page(&artifact, 0)?))
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
    stores: &PdfFinalizationInput,
    pages: &[super::PdfCommittedPageInput],
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
    while stack.last().is_some_and(|(_, remaining)| *remaining == 0) {
        stack.pop();
    }
    if let Some(&(parent, remaining)) = stack.last() {
        return Err(PdfBuildError::OutlineCountIncomplete {
            object: records[parent].item_object(),
            missing: remaining,
        });
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
    for (index, record) in records.iter().enumerate() {
        objects.push(PdfIndirectObject {
            id: object_id(record.action_object())?,
            object: PdfObject::Action(detached_link_action(stores, record.action(), pages)?),
        });
        objects.push(PdfIndirectObject {
            id: object_id(record.title_object())?,
            object: PdfObject::PdfStringSyntax(record.title.clone()),
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
                raw_entries: record.entries.clone(),
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
    records: &[super::PdfOutlineInput],
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
    _input: &PdfFinalizationInput,
    records: &[super::PdfCommittedPageInput],
    pages: &[PositionedPage],
    decimal_digits: i32,
) -> Result<Vec<ShippedDestination>, PdfBuildError> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for (page, record) in pages.iter().zip(records) {
        let artifact = PageArtifact::from_bytes(&record.artifact_bytes)?;
        let (_, page_height) = pdf_page_extents(&artifact, record)?;
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
                        crate::PdfDestinationKind::Xyz { zoom } => PdfDestinationView::Xyz {
                            left: number(x)?,
                            top: number(y)?,
                            zoom: zoom
                                .map(|zoom| PdfNumber::new(i64::from(zoom), 3))
                                .transpose()?,
                        },
                        crate::PdfDestinationKind::FitBoundingBoxHorizontal => {
                            PdfDestinationView::FitBoundingBoxHorizontal { top: number(y)? }
                        }
                        crate::PdfDestinationKind::FitBoundingBoxVertical => {
                            PdfDestinationView::FitBoundingBoxVertical { left: number(x)? }
                        }
                        crate::PdfDestinationKind::FitBoundingBox => {
                            PdfDestinationView::FitBoundingBox
                        }
                        crate::PdfDestinationKind::FitHorizontal => {
                            PdfDestinationView::FitHorizontal { top: number(y)? }
                        }
                        crate::PdfDestinationKind::FitVertical => {
                            PdfDestinationView::FitVertical { left: number(x)? }
                        }
                        crate::PdfDestinationKind::FitRectangle {
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
                        crate::PdfDestinationKind::Fit => PdfDestinationView::Fit,
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
    stores: &PdfFinalizationInput,
    pages: &[super::PdfCommittedPageInput],
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
            super::PdfDestinationIdentityInput::Name(name)
            | super::PdfDestinationIdentityInput::Raw(name) => {
                names.push((decode_pdf_string(name), object_id(record.object())?));
                true
            }
            super::PdfDestinationIdentityInput::Number(_) => false,
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
    stores: &PdfFinalizationInput,
    pages: &[PositionedPage],
    link_margins: &[Scaled],
) -> Result<Vec<Vec<ShippedAnnotation>>, PdfBuildError> {
    let annotations = stores
        .pdf_annotations()
        .iter()
        .map(|record| (record.object(), record))
        .collect::<BTreeMap<_, _>>();
    let links = stores
        .pdf_links()
        .iter()
        .map(|record| (record.object(), record))
        .collect::<BTreeMap<_, _>>();
    let mut active = Vec::<ActiveShippedLink<'_>>::new();
    let mut result = Vec::with_capacity(pages.len());
    // pdftex.web §1597 initializes `gen_running_link` once, while
    // §§37031–37034/37116–37119 mutate it as ordered whatsits are shipped.
    // It therefore persists across pages rather than resetting per shipout.
    let mut running = true;

    for (page, link_margin) in pages.iter().zip(link_margins.iter().copied()) {
        let mut shipped = Vec::new();
        let mut boxes = BTreeMap::<u32, PositionedBox>::new();
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
                        crate::PdfAnnotationEffect::Annotation { object } => {
                            let record = annotations
                                .get(&object)
                                .copied()
                                .ok_or(PdfBuildError::MissingAnnotationRecord(object))?;
                            let data = record
                                .data
                                .as_ref()
                                .ok_or(PdfBuildError::UninitializedAnnotation(object))?;
                            shipped.push(ShippedAnnotation {
                                source_object: object,
                                object,
                                kind: ShippedAnnotationKind::Annotation,
                                rect: marker_rect(
                                    marker.x,
                                    marker.y,
                                    positioned_box,
                                    data.0,
                                    Scaled::from_raw(0),
                                )?,
                            });
                        }
                        crate::PdfAnnotationEffect::LinkStart { object } => {
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
                        crate::PdfAnnotationEffect::LinkEnd { object } => {
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
                        crate::PdfAnnotationEffect::RunningLink(enabled) => running = enabled,
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
    thread_records: &[super::PdfThreadInput],
    pages: &[PositionedPage],
    page_records: &[super::PdfCommittedPageInput],
    decimal_digits: i32,
    next_object: &mut u32,
) -> Result<ThreadOutput, PdfBuildError> {
    let mut thread_beads = BTreeMap::<u32, BTreeSet<(u32, u32)>>::new();
    for thread in thread_records {
        if thread_beads.contains_key(&thread.object()) {
            return Err(PdfBuildError::DuplicateThreadObject(thread.object()));
        }
        thread_beads.insert(
            thread.object(),
            thread
                .beads()
                .iter()
                .map(|bead| (bead.bead_object(), bead.rectangle_object()))
                .collect(),
        );
    }
    let mut beads = Vec::<ShippedBead>::new();
    let mut shipped_beads = BTreeSet::new();
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
                                super::PdfAnnotationDimensionsInput {
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
                    let thread = object_id(marker.thread_object)?;
                    let bead = object_id(marker.bead_object)?;
                    let rectangle = object_id(marker.rectangle_object)?;
                    let Some(owned_beads) = thread_beads.get(&marker.thread_object) else {
                        return Err(PdfBuildError::MissingThreadRecord(marker.thread_object));
                    };
                    if !owned_beads.contains(&(marker.bead_object, marker.rectangle_object)) {
                        return Err(PdfBuildError::ThreadBeadOwnership {
                            thread: marker.thread_object,
                            bead: marker.bead_object,
                            rectangle: marker.rectangle_object,
                        });
                    }
                    if !shipped_beads.insert(marker.bead_object) {
                        return Err(PdfBuildError::DuplicateThreadBead(marker.bead_object));
                    }
                    let positioned_box = boxes.get(&positioned.containing_box).copied().ok_or(
                        PdfBuildError::MissingThreadContainingBox(positioned.containing_box),
                    )?;
                    let dimensions = super::PdfAnnotationDimensionsInput {
                        width: marker.width,
                        height: marker.height,
                        depth: marker.depth,
                    };
                    let rect = marker_rect(
                        positioned.x,
                        positioned.y,
                        positioned_box,
                        dimensions,
                        marker.margin,
                    )?;
                    let title = match &marker.identifier {
                        crate::PdfDestinationIdentifier::Name(name) => name.clone(),
                        crate::PdfDestinationIdentifier::Number(number) => {
                            number.to_string().into_bytes()
                        }
                    };
                    page_beads[page_index].push(bead);
                    beads.push(ShippedBead {
                        thread,
                        bead,
                        rectangle,
                        page: object_id(record.page_object())?,
                        rect,
                        attributes: marker.attributes.clone(),
                        title,
                        margin: marker.margin,
                    });
                    running_bead = positioned.running.then_some(beads.len() - 1);
                    running_parent_depth = positioned.running.then_some(positioned_box.depth);
                }
                PositionedEvent::PdfEndThread { y, .. } => {
                    let index = running_bead
                        .take()
                        .ok_or(PdfBuildError::UnmatchedThreadEnd { page: page_index })?;
                    beads[index].rect.bottom = y
                        .checked_add(beads[index].margin)
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    running_parent_depth = None;
                }
                _ => {}
            }
        }
        if let Some(index) = running_bead {
            return Err(PdfBuildError::UnfinishedThread {
                page: page_index,
                thread: beads[index].thread.get(),
            });
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
                super::PdfDestinationIdentityInput::Name(name)
                | super::PdfDestinationIdentityInput::Raw(name) => name.clone(),
                super::PdfDestinationIdentityInput::Number(number) => {
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
    record: &super::PdfLinkInput,
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
    dimensions: super::PdfAnnotationDimensionsInput,
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
    dimensions: super::PdfAnnotationDimensionsInput,
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
    pages: &mut [Vec<ShippedAnnotation>],
    next_object: &mut u32,
) -> Result<(), PdfBuildError> {
    let mut used = BTreeSet::new();
    for annotation in pages.iter_mut().flatten() {
        annotation.object = if used.insert(annotation.source_object) {
            annotation.source_object
        } else {
            let object = *next_object;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            object
        };
    }
    Ok(())
}

fn annotation_object(
    stores: &PdfFinalizationInput,
    shipped: ShippedAnnotation,
    page: &super::PdfCommittedPageInput,
    page_height: Scaled,
    pages: &[super::PdfCommittedPageInput],
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
                .and_then(|record| record.data.as_ref())
                .ok_or(PdfBuildError::MissingAnnotationRecord(
                    shipped.source_object,
                ))?;
            (None, None, record.1.clone())
        }
        ShippedAnnotationKind::Link => {
            let record = stores
                .pdf_links()
                .iter()
                .find(|record| record.object() == shipped.source_object)
                .ok_or(PdfBuildError::MissingLinkRecord(shipped.source_object))?;
            let raw_entries = record.entries.clone();
            let action = detached_link_action(stores, &record.action, pages)?;
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
    stores: &PdfFinalizationInput,
    spec: &super::PdfActionInput,
    pages: &[super::PdfCommittedPageInput],
) -> Result<PdfAnnotationAction, PdfBuildError> {
    let (kind, file, structure_identity, target_input, new_window) = match spec {
        super::PdfActionInput::User(bytes) => {
            return Ok(PdfAnnotationAction::UserEntries(bytes.clone()));
        }
        super::PdfActionInput::GoTo {
            file,
            structure,
            target,
            new_window,
        } => (
            PdfDestinationActionKind::GoTo,
            file,
            structure,
            target,
            *new_window,
        ),
        super::PdfActionInput::Thread {
            file,
            structure,
            target,
            new_window,
        } => (
            PdfDestinationActionKind::Thread,
            file,
            structure,
            target,
            *new_window,
        ),
    };
    let external = file.is_some();
    let target = match target_input {
        super::PdfActionTargetInput::Page { number, view } => {
            let page = if external {
                PdfDestinationPage::External(number.saturating_sub(1))
            } else {
                PdfDestinationPage::Internal(object_id(
                    pages
                        .get((*number - 1) as usize)
                        .ok_or(PdfBuildError::OpenActionPageNotFound(*number))?
                        .page_object(),
                )?)
            };
            PdfDestinationTarget::Page {
                page,
                view: view.clone(),
            }
        }
        super::PdfActionTargetInput::Destination(super::PdfDestinationIdentityInput::Name(
            name,
        ))
        | super::PdfActionTargetInput::Destination(super::PdfDestinationIdentityInput::Raw(name)) => {
            PdfDestinationTarget::Name(name.clone())
        }
        super::PdfActionTargetInput::Destination(super::PdfDestinationIdentityInput::Number(
            number,
        )) => {
            if external {
                PdfDestinationTarget::Number(*number)
            } else {
                let identity = super::PdfDestinationIdentityInput::Number(*number);
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
    };
    let structure = structure_identity.as_ref().and_then(|identifier| {
        if external {
            Some(match identifier {
                super::PdfDestinationIdentityInput::Name(bytes)
                | super::PdfDestinationIdentityInput::Raw(bytes) => {
                    PdfDestinationStructure::External(bytes.clone())
                }
                super::PdfDestinationIdentityInput::Number(number) => {
                    PdfDestinationStructure::External(number.to_string().into_bytes())
                }
            })
        } else {
            let identity = identifier.clone();
            stores
                .pdf_destination(&identity, true)
                .filter(|record| record.defined)
                .map(|record| {
                    PdfDestinationStructure::Internal(
                        object_id(record.object()).expect("valid reserved destination object"),
                    )
                })
        }
    });
    Ok(PdfAnnotationAction::Destination(PdfDestinationAction {
        kind,
        file: file.clone(),
        target,
        structure,
        new_window,
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

fn font_has_explicit_space(font: &PdfFontInput) -> bool {
    font.encoding
        .as_ref()
        .is_some_and(|encoding| encoding.glyph_names()[32] == b"space")
}

pub(super) fn font_horizontal_scale(construction: &crate::FontResourceConstruction) -> f32 {
    match construction {
        crate::FontResourceConstruction::Expanded { ratio, .. } => {
            (1000.0 + f32::from(*ratio)) / 1000.0
        }
        crate::FontResourceConstruction::Loaded
        | crate::FontResourceConstruction::Copied { .. }
        | crate::FontResourceConstruction::Letterspaced { .. } => 1.0,
    }
}

fn scalable_text_advance(
    input: &PdfFontInput,
    font: &crate::FontResource,
    bytes: &[u8],
    font_size: f32,
    horizontal_scale: f32,
) -> Option<f32> {
    input.map_entry.as_ref()?;
    let denominator = i64::from(font.at_size.raw()).max(1);
    let width_units = bytes.iter().try_fold(0_i64, |total, &code| {
        let width = i64::from(input.metrics.widths[usize::from(code)].raw());
        total.checked_add((width * 1000 + denominator / 2) / denominator)
    })?;
    Some(width_units as f32 * font_size * horizontal_scale / 1000.0)
}

fn pdf_font_objects(
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
    let denominator = i64::from(font.at_size.raw()).max(1);
    let widths = (first_char as u8..=last_char as u8)
        .map(|code| {
            let width = i64::from(input.metrics.widths[usize::from(code)].raw());
            PdfValue::Integer((width * 1000 + denominator / 2) / denominator)
        })
        .collect();
    dictionary.insert("Widths", PdfValue::Array(widths))?;
    let to_unicode = ids
        .to_unicode
        .map(|to_unicode_id| {
            to_unicode_stream(input, font, used_codes, encoding, type1, to_unicode_id)
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
    let tfm_ascent = input
        .metrics
        .heights
        .iter()
        .copied()
        .map(scale_metric)
        .max()
        .unwrap_or(0);
    let tfm_descent = input
        .metrics
        .depths
        .iter()
        .copied()
        .map(scale_metric)
        .max()
        .unwrap_or(0);
    let tfm_cap_height = scale_metric(input.metrics.heights[usize::from(b'H')]);
    let tfm_x_height = scale_metric(input.metrics.x_height);
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

fn pdf_pk_font_objects(
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
    input: &PdfFontInput,
    font: &crate::FontResource,
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
        let unicode = input.glyph_to_unicode.get(glyph).cloned().or_else(|| {
            input
                .infer_builtin_glyph_unicode
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
    metadata: &super::PdfDocumentMetadataInput,
) -> Result<PdfDictionary, PdfModelError> {
    const PRODUCER: &[u8] = b"pdfTeX-1.40.29";

    let mut info = PdfDictionary::new();
    info.insert("Producer", PdfValue::String(PRODUCER.to_vec()))?;
    info.insert("Creator", PdfValue::String(b"TeX".to_vec()))?;
    if metadata.include_dates {
        let date = metadata.creation_date.clone();
        info.insert("CreationDate", PdfValue::String(date.clone()))?;
        info.insert("ModDate", PdfValue::String(date))?;
    }
    info.insert("Trapped", PdfValue::Name("False".into()))?;
    if let Some(key) = &metadata.ptex_banner_key {
        info.insert(
            PdfName::new(key.clone()),
            PdfValue::String(metadata.ptex_banner.clone()),
        )?;
    }
    Ok(info)
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
    pixels: usize,
    rows: usize,
    raw_bytes: usize,
    color_bytes: usize,
    alpha_bytes: usize,
    peak_row_bytes: usize,
}

const DERIVED_IMAGE_COMPRESSION_LEVEL: u32 = 1;
const DERIVED_IMAGE_WINDOW_BITS: u8 = 15;

#[derive(Clone, Copy)]
struct RasterMetadata {
    format: PdfRasterFormatInput,
    width: u32,
    height: u32,
    bits_per_component: u8,
    color_space: PdfRasterColorSpaceInput,
    alpha: bool,
    png_color_type: Option<u8>,
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn raster_image_streams(
    bytes: &[u8],
    metadata: RasterMetadata,
    parameters: PdfImageGammaInput,
    version: (u8, u8),
    telemetry: &mut ImageImportTelemetry,
) -> Result<RasterStreams, PdfBuildError> {
    if metadata.width == 0 || metadata.height == 0 {
        return Err(PdfBuildError::InvalidRasterDimensions);
    }
    if metadata.format == PdfRasterFormatInput::Png {
        validate_png_decoded_size(metadata)?;
    }
    let color_space = match metadata.color_space {
        PdfRasterColorSpaceInput::Gray => PdfImageColorSpace::DeviceGray,
        PdfRasterColorSpaceInput::Rgb => PdfImageColorSpace::DeviceRgb,
        PdfRasterColorSpaceInput::Cmyk => PdfImageColorSpace::DeviceCmyk,
    };
    let streams: Result<RasterStreams, PdfBuildError> = match metadata.format {
        PdfRasterFormatInput::Jpeg => Ok((
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
        PdfRasterFormatInput::Png if metadata.png_color_type == Some(3) => {
            let (color, alpha) = png_indexed_streams(bytes, metadata, telemetry)?;
            Ok((
                color,
                PdfImageFilter::Flate,
                8,
                PdfImageColorSpace::DeviceRgb,
                alpha.map(|alpha| (alpha, PdfImageFilter::Flate)),
            ))
        }
        PdfRasterFormatInput::Png if metadata.alpha => {
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
        PdfRasterFormatInput::Png => Ok((
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
    if metadata.format == PdfRasterFormatInput::Png
        && metadata.bits_per_component == 16
        && (!parameters.high_color || (version.0 == 1 && version.1 < 5))
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
    if metadata.format == PdfRasterFormatInput::Png && parameters.apply_gamma {
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

fn validate_png_decoded_size(metadata: RasterMetadata) -> Result<(), PdfBuildError> {
    let components = match metadata.png_color_type {
        Some(0 | 3) => 1usize,
        Some(2) => 3,
        Some(4) => 2,
        Some(6) => 4,
        _ => return Err(PdfBuildError::InvalidPng),
    };
    let row_bytes = usize::try_from(metadata.width)
        .ok()
        .and_then(|width| width.checked_mul(components))
        .and_then(|samples| samples.checked_mul(usize::from(metadata.bits_per_component)))
        .and_then(|bits| bits.checked_add(7))
        .map(|bits| bits / 8)
        .ok_or(PdfBuildError::InvalidPng)?;
    let height = usize::try_from(metadata.height).map_err(|_| PdfBuildError::InvalidPng)?;
    let decoded_bytes = row_bytes
        .checked_add(1)
        .and_then(|row| row.checked_mul(height))
        .ok_or(PdfBuildError::InvalidPng)?;
    if decoded_bytes > MAX_IMPORTED_PDF_STREAM_BYTES {
        return Err(PdfBuildError::InvalidPng);
    }
    Ok(())
}

fn strip_png_16(samples: &[u8]) -> Vec<u8> {
    samples.chunks_exact(2).map(|sample| sample[0]).collect()
}

fn raster_color_components(color_space: PdfRasterColorSpaceInput) -> u8 {
    match color_space {
        PdfRasterColorSpaceInput::Gray => 1,
        PdfRasterColorSpaceInput::Rgb => 3,
        PdfRasterColorSpaceInput::Cmyk => 4,
    }
}

fn image_resource_name(
    image: &super::PdfExternalImageInput,
    parameters: FinalizationParameters,
) -> Vec<u8> {
    if parameters.unique_resource_names > 0 {
        let prefix = image.identity.hex();
        format!("{}Im{}", &prefix[..6], image.object).into_bytes()
    } else {
        format!("Im{}", image.object).into_bytes()
    }
}

fn png_idat(bytes: &[u8]) -> Result<Vec<u8>, PdfBuildError> {
    validate_png_crc(bytes)?;
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

fn strict_png_decoder() -> png::StreamingDecoder {
    let mut options = png::DecodeOptions::default();
    options.set_ignore_adler32(false);
    options.set_ignore_crc(false);
    options.set_ignore_text_chunk(true);
    options.set_ignore_iccp_chunk(true);
    options.set_skip_ancillary_crc_failures(false);
    png::StreamingDecoder::new_with_options(options)
}

fn validate_png_crc(bytes: &[u8]) -> Result<(), PdfBuildError> {
    let mut decoder = strict_png_decoder();
    let mut input = bytes;
    let mut saw_iend = false;
    let mut stalled_updates = 0u8;
    while !input.is_empty() && !saw_iend {
        let (consumed, decoded) = decoder
            .update(input, None)
            .map_err(|_| PdfBuildError::InvalidPng)?;
        input = &input[consumed..];
        if let png::Decoded::ChunkBegin(length, _) = decoded
            && usize::try_from(length)
                .ok()
                .is_none_or(|length| length > bytes.len() || length > MAX_IMPORTED_PDF_STREAM_BYTES)
        {
            return Err(PdfBuildError::InvalidPng);
        }
        saw_iend = matches!(decoded, png::Decoded::ChunkComplete(kind) if kind == png::chunk::IEND);
        if consumed == 0 {
            stalled_updates = stalled_updates.saturating_add(1);
            if stalled_updates > 8 {
                return Err(PdfBuildError::InvalidPng);
            }
        } else {
            stalled_updates = 0;
        }
    }
    if saw_iend && input.is_empty() {
        Ok(())
    } else {
        Err(PdfBuildError::InvalidPng)
    }
}

fn inflate(bytes: &[u8]) -> Result<Vec<u8>, PdfBuildError> {
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    Ok(output)
}

fn png_opaque_samples(bytes: &[u8], metadata: RasterMetadata) -> Result<Vec<u8>, PdfBuildError> {
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
    parameters: PdfImageGammaInput,
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
    metadata: RasterMetadata,
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
    let pixels = width.checked_mul(height).ok_or(PdfBuildError::InvalidPng)?;
    telemetry.pixels = telemetry.pixels.saturating_add(pixels);
    telemetry.rows = telemetry.rows.saturating_add(height);
    telemetry.raw_bytes = telemetry
        .raw_bytes
        .saturating_add(row_bytes.saturating_mul(height));
    if metadata.bits_per_component == 8 {
        return png_alpha_streams_filtered(
            bytes,
            metadata,
            width,
            height,
            row_bytes,
            pixel_bytes,
            telemetry,
        );
    }
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
    let mut color = Vec::with_capacity(row_bytes * height);
    let mut alpha = Vec::with_capacity(width * component_bytes * height);
    telemetry.color_bytes = telemetry.color_bytes.saturating_add(
        width
            .saturating_mul(color_components * component_bytes)
            .saturating_mul(height),
    );
    telemetry.alpha_bytes = telemetry
        .alpha_bytes
        .saturating_add(width.saturating_mul(component_bytes).saturating_mul(height));
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
    Ok((
        streams.0,
        PdfImageFilter::Flate,
        streams.1,
        PdfImageFilter::Flate,
    ))
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn png_alpha_streams_filtered(
    png_bytes: &[u8],
    metadata: RasterMetadata,
    width: usize,
    height: usize,
    row_bytes: usize,
    pixel_bytes: usize,
    telemetry: &mut ImageImportTelemetry,
) -> Result<(Vec<u8>, PdfImageFilter, Vec<u8>, PdfImageFilter), PdfBuildError> {
    let color_space = metadata.color_space;
    let color_components = usize::from(raster_color_components(color_space));
    let color_row_bytes = width
        .checked_mul(color_components)
        .ok_or(PdfBuildError::InvalidPng)?;
    telemetry.color_bytes = telemetry
        .color_bytes
        .saturating_add((color_row_bytes + 1).saturating_mul(height));
    telemetry.alpha_bytes = telemetry
        .alpha_bytes
        .saturating_add((width + 1).saturating_mul(height));
    let filtered_row_bytes = row_bytes.checked_add(1).ok_or(PdfBuildError::InvalidPng)?;
    let decoder_buffer_bytes = (32 * 1024usize)
        .checked_add(8 * 1024)
        .and_then(|size| size.checked_add(filtered_row_bytes.checked_mul(2)?))
        .ok_or(PdfBuildError::InvalidPng)?;
    telemetry.peak_row_bytes = telemetry.peak_row_bytes.max(
        decoder_buffer_bytes
            .saturating_add(color_row_bytes + 1)
            .saturating_add(width + 1),
    );
    let mut decoder = strict_png_decoder();
    let mut decoder_buffer = vec![0; decoder_buffer_bytes];
    let mut decoder_region = png::UnfilterRegion::default();
    let mut color_encoder = flate2::write::ZlibEncoder::new(
        Vec::new(),
        flate2::Compression::new(DERIVED_IMAGE_COMPRESSION_LEVEL),
    );
    let mut alpha_encoder = flate2::write::ZlibEncoder::new(
        Vec::new(),
        flate2::Compression::new(DERIVED_IMAGE_COMPRESSION_LEVEL),
    );
    let mut color_row = vec![0; color_row_bytes + 1];
    let mut alpha_row = vec![0; width + 1];
    let mut input = png_bytes;
    let mut rows = 0usize;
    let mut saw_iend = false;
    let mut stalled_updates = 0u8;
    while !input.is_empty() && !saw_iend {
        let started = std::time::Instant::now();
        let (consumed, decoded) = decoder
            .update(input, Some(&mut decoder_region.as_buf(&mut decoder_buffer)))
            .map_err(|_| PdfBuildError::InvalidPng)?;
        input = &input[consumed..];
        telemetry.decode_ns += started.elapsed().as_nanos();
        if let png::Decoded::ChunkBegin(length, _) = decoded
            && usize::try_from(length).ok().is_none_or(|length| {
                length > png_bytes.len() || length > MAX_IMPORTED_PDF_STREAM_BYTES
            })
        {
            return Err(PdfBuildError::InvalidPng);
        }
        if let Some(info) = decoder.info()
            && (info.width != metadata.width
                || info.height != metadata.height
                || info.bit_depth != png::BitDepth::Eight
                || info.color_type
                    != match metadata.png_color_type {
                        Some(4) => png::ColorType::GrayscaleAlpha,
                        Some(6) => png::ColorType::Rgba,
                        _ => return Err(PdfBuildError::InvalidPng),
                    }
                || info.interlaced)
        {
            return Err(PdfBuildError::InvalidPng);
        }
        rows = rows
            .checked_add(split_available_png_rows(
                &mut decoder_buffer,
                &mut decoder_region,
                filtered_row_bytes,
                pixel_bytes,
                color_components,
                &mut color_row,
                &mut alpha_row,
                &mut color_encoder,
                &mut alpha_encoder,
                telemetry,
            )?)
            .ok_or(PdfBuildError::InvalidPng)?;
        if matches!(decoded, png::Decoded::ImageDataFlushed) {
            decoder_region.available = decoder_region.filled;
            rows = rows
                .checked_add(split_available_png_rows(
                    &mut decoder_buffer,
                    &mut decoder_region,
                    filtered_row_bytes,
                    pixel_bytes,
                    color_components,
                    &mut color_row,
                    &mut alpha_row,
                    &mut color_encoder,
                    &mut alpha_encoder,
                    telemetry,
                )?)
                .ok_or(PdfBuildError::InvalidPng)?;
        }
        saw_iend = matches!(decoded, png::Decoded::ChunkComplete(kind) if kind == png::chunk::IEND);
        if consumed == 0 {
            stalled_updates = stalled_updates.saturating_add(1);
            if stalled_updates > 8 {
                return Err(PdfBuildError::InvalidPng);
            }
        } else {
            stalled_updates = 0;
        }
    }
    if !saw_iend || !input.is_empty() || rows != height || decoder_region.filled != 0 {
        return Err(PdfBuildError::InvalidPng);
    }
    let started = std::time::Instant::now();
    let color = color_encoder
        .finish()
        .map_err(|_| PdfBuildError::InvalidPng)?;
    let alpha = alpha_encoder
        .finish()
        .map_err(|_| PdfBuildError::InvalidPng)?;
    telemetry.encode_ns += started.elapsed().as_nanos();
    Ok((
        color,
        PdfImageFilter::FlatePngPredictor {
            colors: raster_color_components(color_space),
        },
        alpha,
        PdfImageFilter::FlatePngPredictor { colors: 1 },
    ))
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn split_available_png_rows(
    decoder_buffer: &mut [u8],
    decoder_region: &mut png::UnfilterRegion,
    filtered_row_bytes: usize,
    pixel_bytes: usize,
    color_components: usize,
    color_row: &mut [u8],
    alpha_row: &mut [u8],
    color_encoder: &mut flate2::write::ZlibEncoder<Vec<u8>>,
    alpha_encoder: &mut flate2::write::ZlibEncoder<Vec<u8>>,
    telemetry: &mut ImageImportTelemetry,
) -> Result<usize, PdfBuildError> {
    let rows = decoder_region.available / filtered_row_bytes;
    for row in decoder_buffer[..rows * filtered_row_bytes].chunks_exact(filtered_row_bytes) {
        let started = std::time::Instant::now();
        if row[0] > 4 {
            return Err(PdfBuildError::InvalidPng);
        }
        color_row[0] = row[0];
        alpha_row[0] = row[0];
        for (index, pixel) in row[1..].chunks_exact(pixel_bytes).enumerate() {
            let color_start = 1 + index * color_components;
            color_row[color_start..color_start + color_components]
                .copy_from_slice(&pixel[..color_components]);
            alpha_row[index + 1] = pixel[color_components];
        }
        telemetry.transform_ns += started.elapsed().as_nanos();

        let started = std::time::Instant::now();
        color_encoder
            .write_all(color_row)
            .map_err(|_| PdfBuildError::InvalidPng)?;
        alpha_encoder
            .write_all(alpha_row)
            .map_err(|_| PdfBuildError::InvalidPng)?;
        telemetry.encode_ns += started.elapsed().as_nanos();
    }
    let consumed = rows * filtered_row_bytes;
    if consumed != 0 {
        decoder_buffer.copy_within(consumed..decoder_region.filled, 0);
        decoder_region.available -= consumed;
        decoder_region.filled -= consumed;
    }
    Ok(rows)
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn png_indexed_streams(
    bytes: &[u8],
    metadata: RasterMetadata,
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
    let mut encoder = flate2::write::ZlibEncoder::new(
        Vec::new(),
        flate2::Compression::new(DERIVED_IMAGE_COMPRESSION_LEVEL),
    );
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
    image: &super::PdfExternalImageInput,
    page: u32,
    page_box: super::PdfPageBoxInput,
    rotation: PdfPageRotationInput,
    next_object: &mut u32,
    limits: super::PdfFinalizationLimits,
) -> Result<ImportedPdfPage, PdfBuildError> {
    let imported = super::import::import_pdf_page(image.bytes.clone(), page, next_object, limits)
        .map_err(PdfBuildError::InvalidPdfPage)?;
    let mut dictionary = PdfDictionary::new();
    dictionary.insert("FormType", PdfValue::Integer(1))?;
    dictionary.insert("Resources", PdfValue::Dictionary(imported.resources))?;
    if let Some(group) = imported.group {
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
        PdfPageRotationInput::None => (width, height, [1.0, 0.0, 0.0, 1.0, -left_bp, -bottom_bp]),
        PdfPageRotationInput::Clockwise90 => (
            height,
            width,
            [0.0, 1.0, -1.0, 0.0, height_bp + bottom_bp, -left_bp],
        ),
        PdfPageRotationInput::UpsideDown => (
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
        PdfPageRotationInput::Clockwise270 => (
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
            id: object_id(image.object)?,
            object: PdfObject::FormXObject {
                dictionary,
                data: imported.data,
                bbox: [
                    zero,
                    zero,
                    scaled_to_bp_number(form_width, 4)?,
                    scaled_to_bp_number(form_height, 4)?,
                ],
                matrix: Some(matrix).filter(|matrix| *matrix != [one, zero, zero, one, zero, zero]),
            },
        },
        dependencies: imported.dependencies,
        group: imported.group,
    })
}

fn rotation_swaps_axes(rotation: PdfPageRotationInput) -> bool {
    matches!(
        rotation,
        PdfPageRotationInput::Clockwise90 | PdfPageRotationInput::Clockwise270
    )
}

fn pdf_number_from_f32(value: f32) -> Result<PdfNumber, PdfBuildError> {
    if !value.is_finite() {
        return Err(PdfBuildError::InvalidPdfPage(
            "page resource contains a non-finite number".to_owned(),
        ));
    }
    PdfNumber::new((f64::from(value) * 1_000_000_000.0).round() as i64, 9).map_err(Into::into)
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
    artifact: &crate::PageArtifact,
    record: &super::PdfCommittedPageInput,
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
    OutlineCountIncomplete {
        object: u32,
        missing: usize,
    },
    DuplicateThreadObject(u32),
    MissingThreadRecord(u32),
    ThreadBeadOwnership {
        thread: u32,
        bead: u32,
        rectangle: u32,
    },
    DuplicateThreadBead(u32),
    MissingThreadContainingBox(u32),
    UnmatchedThreadEnd {
        page: usize,
    },
    UnfinishedThread {
        page: usize,
        thread: u32,
    },
    ReferencedRawObjectUninitialized(u32),
    ReferencedFormNotFound(u32),
    MissingFormArtifact(u32),
    RecursiveForm(u32),
    FormCycle(u32),
    FormTraversalDepthExceeded(usize),
    FormTraversalWorkExceeded(usize),
    InvalidRawObjectFileName(u32),
    TextRequiresFontResources,
    MissingPositionedFont(u32),
    PositionedCharacterOutOfRange {
        font: String,
        code: u32,
    },
    MissingFontProgram(Vec<u8>),
    MissingFontResource(String),
    MissingFontUsage(String),
    PkFont(String),
    MissingPkFont(tex_fonts::PdfPkFontRequest),
    MissingPkGlyph {
        font: String,
        code: u8,
    },
    MissingEncoding(Vec<u8>),
    MissingSpaceFontName(u32),
    MissingBuiltinGlyphName {
        font: String,
        code: u8,
    },
    TrueTypeSubsetRequiresEncoding(String),
    Type1Subset {
        font: String,
        error: tex_fonts::PdfType1SubsetError,
    },
    TrueTypeSubset(tex_fonts::PdfTrueTypeSubsetError),
    MissingLiveFont(String),
    UnsupportedMappedVirtualFont(String),
    VirtualFontDepthExceeded(usize),
    VirtualFontStackExceeded(usize),
    VirtualFontStackUnderflow,
    VirtualFontWorkExceeded(usize),
    VirtualFontOutputExceeded(usize),
    VirtualFontSpecialBytesExceeded(usize),
    VirtualFontCycle {
        font: String,
        code: u8,
    },
    MissingVirtualFontPacket {
        font: String,
        code: u32,
    },
    VirtualFontHasNoLocalFonts(String),
    MissingVirtualLocalFont {
        font: String,
        number: i32,
    },
    InvalidVirtualLocalFontName(String),
    MissingVirtualLocalTfm(String),
    InvalidVirtualLocalTfm {
        font: String,
        message: String,
    },
    VirtualFontCharacterOutOfRange {
        font: String,
        code: u32,
    },
    MissingVirtualCharacter {
        font: String,
        code: u8,
    },
    VirtualFontArithmeticOverflow,
    UnsupportedSpecial(String),
    MissingRasterImage(u32),
    UnsupportedPdfPageImage(u32),
    InvalidRasterDimensions,
    InvalidPng,
    InvalidPdfPage(String),
    InvalidMatrix(Vec<u8>),
    Parse(crate::ParseError),
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
            Self::OutlineCountIncomplete { object, missing } => write!(
                f,
                "PDF outline item {object} is missing {missing} declared child entries"
            ),
            Self::DuplicateThreadObject(object) => {
                write!(f, "PDF thread object {object} has duplicate ledger entries")
            }
            Self::MissingThreadRecord(object) => {
                write!(
                    f,
                    "shipped article thread references missing object {object}"
                )
            }
            Self::ThreadBeadOwnership {
                thread,
                bead,
                rectangle,
            } => write!(
                f,
                "shipped article bead {bead} with rectangle {rectangle} is not owned by thread {thread}"
            ),
            Self::DuplicateThreadBead(bead) => {
                write!(f, "article bead object {bead} was shipped more than once")
            }
            Self::MissingThreadContainingBox(box_id) => {
                write!(f, "shipped article bead references missing box {box_id}")
            }
            Self::UnmatchedThreadEnd { page } => {
                write!(
                    f,
                    "page {} has \\pdfendthread without a running thread",
                    page + 1
                )
            }
            Self::UnfinishedThread { page, thread } => write!(
                f,
                "page {} ends with PDF thread object {thread} still running",
                page + 1
            ),
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
            Self::FormCycle(id) => write!(f, "PDF form cycle detected at object {id}"),
            Self::FormTraversalDepthExceeded(limit) => {
                write!(f, "PDF form traversal exceeds depth {limit}")
            }
            Self::FormTraversalWorkExceeded(limit) => {
                write!(f, "PDF form traversal exceeds {limit} references")
            }
            Self::InvalidRawObjectFileName(id) => {
                write!(f, "PDF stream object {id} has a non-UTF-8 file name")
            }
            Self::TextRequiresFontResources => {
                f.write_str("PDF text output requires embedded font resources")
            }
            Self::MissingPositionedFont(font) => {
                write!(f, "positioned text references missing font resource {font}")
            }
            Self::PositionedCharacterOutOfRange { font, code } => write!(
                f,
                "PDF font {font:?} character code {code} is outside 0..=255"
            ),
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
            Self::Type1Subset { font, error } => {
                write!(f, "cannot subset Type-1 PDF font {font:?}: {error:?}")
            }
            Self::TrueTypeSubset(error) => error.fmt(f),
            Self::MissingLiveFont(name) => {
                write!(f, "PDF artifact font {name:?} has no live metric source")
            }
            Self::UnsupportedMappedVirtualFont(name) => write!(
                f,
                "mapped OpenType text font {name:?} cannot execute a classic virtual-font program"
            ),
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
            Self::Parse(error) => error.fmt(f),
            Self::Positioned(error) => error.fmt(f),
            Self::Model(error) => error.fmt(f),
            Self::Serialize(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PdfBuildError {}

impl From<crate::ParseError> for PdfBuildError {
    fn from(value: crate::ParseError) -> Self {
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

impl From<tex_fonts::PdfTrueTypeSubsetError> for PdfBuildError {
    fn from(value: tex_fonts::PdfTrueTypeSubsetError) -> Self {
        Self::TrueTypeSubset(value)
    }
}
