//! Pure PDF finalization from a complete detached input.

use super::{
    PdfAnnotationAction, PdfAnnotationObject, PdfAnnotationType, PdfBeadObject,
    PdfContentGlyphRaster, PdfContentOperation, PdfContentRule, PdfContentTextExactRaster,
    PdfContentTextPosition, PdfContentTextRaster, PdfContentTextRun, PdfDestinationAction,
    PdfDestinationActionKind, PdfDestinationNameTree, PdfDestinationNameTreeChildren,
    PdfDestinationPage, PdfDestinationStructure, PdfDestinationTarget, PdfDestinationView,
    PdfDictionary, PdfExplicitDestination, PdfFinalizationInput, PdfFontInput, PdfFontMetricsInput,
    PdfFontProgramInput, PdfImageColorSpace, PdfImageFilter, PdfImageGammaInput,
    PdfImageMetadataInput, PdfImageXObject, PdfIndirectObject, PdfModelError, PdfName,
    PdfNamesObject, PdfNumber, PdfObject, PdfObjectId, PdfOutlineItemObject, PdfOutlineObject,
    PdfPageRotationInput, PdfRasterColorSpaceInput, PdfRasterFormatInput, PdfSerializeError,
    PdfThreadObject, PdfTrailer, PdfValue, PdfVersion, UnvalidatedPdfDocument,
    ordered_page_content,
};
use crate::positioned::{BoxKind, PositionedBox, PositionedError, PositionedEvent, PositionedPage};
use crate::{ContentHash, PageArtifact, PageNode};
use md5::{Digest, Md5};
use tex_arith::Scaled;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::{Read, Write};

mod content;
mod errors;
mod fonts;
mod images;
mod navigation;
mod numeric;

pub use errors::PdfBuildError;

use content::*;
use fonts::*;
use images::*;
use navigation::*;
use numeric::*;

pub(super) use fonts::{font_horizontal_scale, positioned_char_end};
pub(super) use numeric::{pdftex_font_size, pdftex_scalable_width_tenths};

#[cfg(test)]
mod tests;

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
    let mapped_font_names = input
        .fonts
        .values()
        .filter_map(|font| font.map_entry.as_ref())
        .map(|entry| entry.tex_name.clone())
        .collect::<BTreeSet<_>>();
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
    let mut font_encodings = PdfFontEncodings::collect(input, &font_usage)?;
    let object_started = std::time::Instant::now();
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

    // writepng.c creates one document-wide transparency group for alpha PNGs
    // and attaches it to every output page that uses one. The group changes
    // the page's compositing color space, so omitting it is render-visible
    // even when the decoded RGB and soft-mask samples are byte-identical.
    let transparent_raster_group = input
        .images
        .values()
        .any(|image| raster_needs_transparency_page_group(image.metadata, input.document.version))
        .then(|| {
            let id = object_id(next_object)?;
            next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            let mut group = PdfDictionary::new();
            group.insert("Type", PdfValue::Name("Group".into()))?;
            group.insert("S", PdfValue::Name("Transparency".into()))?;
            group.insert("CS", PdfValue::Name("DeviceRGB".into()))?;
            group.insert("I", PdfValue::Bool(true))?;
            objects.push(indirect_dictionary(id, group));
            Ok::<_, PdfBuildError>(id)
        })
        .transpose()?;

    let mut pdf_image_groups = BTreeMap::<u32, Option<PdfObjectId>>::new();
    let mut pdf_image_objects = BTreeMap::<u32, PdfObjectId>::new();
    let mut lowered_images =
        HashMap::<(ContentHash, PdfImageMetadataInput, Option<u32>, Vec<u8>), PdfObjectId>::new();
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
        let cache_key = (
            image.identity,
            image.metadata,
            image.color_space_object,
            image.attributes.clone(),
        );
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
                let mut dictionary = PdfDictionary::new();
                dictionary.set_raw_entries(image.attributes.clone());
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
                        dictionary,
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
                            dictionary: PdfDictionary::new(),
                            data: alpha_data,
                        },
                    });
                }
                pdf_image_objects.insert(image.object, image_object);
                lowered_images.insert(cache_key, image_object);
            }
            PdfImageMetadataInput::PdfPage {
                page_box,
                rotation: _,
                page,
                ..
            } => {
                pdf_image_count += 1;
                let imported =
                    import_pdf_page(image, page, page_box, &mut next_object, input.limits)?;
                let image_object = imported.form.id;
                pdf_image_groups.insert(image.object, imported.group);
                pdf_image_objects.insert(image.object, image_object);
                objects.extend(imported.dependencies);
                objects.push(imported.form);
            }
        }
    }
    let image_import_ns = image_import_started.elapsed().as_nanos();

    let content_output = content::append_content_objects(content::ContentInputs {
        input,
        positioned_pages: &positioned_pages,
        positioned_forms: &positioned_forms,
        mapped_font_names: &mapped_font_names,
        font_usage: &font_usage,
        pdf_image_objects: &pdf_image_objects,
        pdf_image_groups: &pdf_image_groups,
        transparent_raster_group,
        parameters,
        pages_id,
        page_annotations: &page_annotations,
        thread_output: &thread_output,
        objects: &mut objects,
        next_object: &mut next_object,
        font_encodings: &mut font_encodings,
    })?;
    let kids = content_output.kids;
    let diagnostics = content_output.diagnostics;
    let font_embed_ns = content_output.font_embed_ns;

    objects.extend(font_encodings.into_objects()?);

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
