//! Umber adapter from terminal PDF completion to the pure `tex-out` boundary.

mod virtual_fonts;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tex_out::pdf::{
    PdfActionInput, PdfActionTargetInput, PdfAllocationInput, PdfAnnotationDimensionsInput,
    PdfAnnotationInput, PdfCommittedPageInput, PdfDestinationIdentityInput, PdfDestinationInput,
    PdfDocumentInput, PdfDocumentMetadataInput, PdfExternalImageInput, PdfFinalizationInput,
    PdfFinalizationLimits, PdfFontInput, PdfFontMetricsInput, PdfFontProgramInput, PdfFormInput,
    PdfImageGammaInput, PdfImageMetadataInput, PdfIndirectActionInput, PdfLinkInput,
    PdfNavigationInput, PdfOutlineInput, PdfPageBoxInput, PdfPageRotationInput,
    PdfRasterColorSpaceInput, PdfRasterFormatInput, PdfRawObjectInput, PdfRawObjectPayloadInput,
    PdfReservedDocumentObjects, PdfThreadBeadInput, PdfThreadInput, PdfVirtualFontInput,
    PdfVirtualLocalTfmInput,
};
use tex_state::{
    DetachedPdfAction, DetachedPdfActionIdentifier, DetachedPdfActionRecord,
    DetachedPdfActionTarget, DetachedPdfCompletion, DetachedPdfFontOperation,
    DetachedPdfRawObjectPayload, PdfActionWindow, PdfAnnotationDimensions, PdfDestinationIdentity,
    PdfExternalImageMetadata, PdfRasterColorSpace, PdfRasterFormat,
};

use super::{PdfBuildError, is_pdf_sfnt_program, pdf_date, pdf_version, serialization_options};

pub fn pdf_finalization_input(
    pdf: &DetachedPdfCompletion,
    driver_dpi: i32,
    resources: &crate::PdfVirtualFontResources,
) -> Result<PdfFinalizationInput, PdfBuildError> {
    pdf_finalization_input_with_raw_object_files(
        pdf,
        driver_dpi,
        resources,
        &crate::PdfRawObjectFileReceipt::default(),
    )
}

pub fn pdf_finalization_input_with_raw_object_files(
    pdf: &DetachedPdfCompletion,
    driver_dpi: i32,
    resources: &crate::PdfVirtualFontResources,
    raw_object_files: &crate::PdfRawObjectFileReceipt,
) -> Result<PdfFinalizationInput, PdfBuildError> {
    let parameters = pdf
        .output_parameters()
        .ok_or(PdfBuildError::PdfOutputDisabled)?;
    if parameters.output <= 0 {
        return Err(PdfBuildError::PdfOutputDisabled);
    }
    let version = pdf_version(parameters)?;
    let pages = pdf
        .pages()
        .iter()
        .map(|page| PdfCommittedPageInput {
            artifact_hash: page.artifact,
            artifact_bytes: Arc::from(page.artifact_bytes.as_slice()),
            resources_object: page.resources_object,
            contents_object: page.contents_object,
            page_object: page.page_object,
            h_origin: page.h_origin,
            v_origin: page.v_origin,
            width: page.width,
            height: page.height,
            link_margin: page.link_margin,
            page_entries: page.page_entries.clone(),
            resource_entries: page.resource_entries.clone(),
            omit_procset: page.omit_procset,
            space_font_name: page.space_font_name.clone(),
        })
        .collect::<Vec<_>>();
    let forms = pdf
        .forms()
        .iter()
        .map(|form| {
            (
                form.object,
                PdfFormInput {
                    object: form.object,
                    resource: form.resource,
                    artifact_bytes: Arc::from(form.artifact_bytes.as_slice()),
                    width: form.width,
                    height: form.height,
                    depth: form.depth,
                    entries: form.entries.clone(),
                    resource_entries: form.resource_entries.clone(),
                    immediate: form.immediate,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let artifacts = pages
        .iter()
        .map(|page| page.artifact_bytes.as_ref())
        .chain(forms.values().map(|form| form.artifact_bytes.as_ref()))
        .map(tex_out::PageArtifact::from_bytes)
        .collect::<Result<Vec<_>, _>>()?;
    let artifacts_by_font = artifacts
        .iter()
        .flat_map(|artifact| artifact.fonts.iter().cloned())
        .map(|font| (font.semantic_identity, font))
        .collect::<BTreeMap<_, _>>();
    let mut artifact_font_usage = BTreeMap::<_, BTreeSet<_>>::new();
    for (page_index, artifact) in artifacts.iter().enumerate() {
        let positioned = tex_out::positioned::lower_page(
            artifact,
            u32::try_from(page_index).unwrap_or(u32::MAX),
        )?;
        for run in positioned.events.iter().filter_map(|event| match event {
            tex_out::positioned::PositionedEvent::TextRun(run) => Some(run),
            _ => None,
        }) {
            let Some(font) = positioned
                .fonts
                .iter()
                .find(|font| font.font_id == run.font_id)
            else {
                continue;
            };
            artifact_font_usage
                .entry(font.semantic_identity)
                .or_default()
                .extend(run.physical_codes.iter().flatten().copied());
        }
    }
    let resolved_map = crate::virtual_compile::resolved_font_map_lines(pdf, resources)
        .into_iter()
        .map(|entry| (entry.tex_name.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let glyph_mappings = pdf
        .font_operations()
        .iter()
        .filter_map(|operation| match operation {
            DetachedPdfFontOperation::GlyphToUnicode(mapping) => Some(mapping),
            _ => None,
        })
        .collect::<Vec<_>>();
    let configuration = pdf.font_configuration();
    let mut fonts = BTreeMap::new();
    for detached in pdf.fonts() {
        let identity = detached.recipe.semantic_identity;
        let artifact_resource = artifacts_by_font
            .get(&identity)
            .cloned()
            .ok_or_else(|| PdfBuildError::MissingFontResource(detached.recipe.name.clone()))?;
        let map_entry = resolved_map.get(detached.recipe.name.as_bytes()).cloned();
        let encoding = map_entry
            .as_ref()
            .and_then(|entry| entry.encoding_files.first())
            .map(|name| {
                detached_encoding(pdf, resources, name)
                    .ok_or_else(|| PdfBuildError::MissingEncoding(name.clone()))
            })
            .transpose()?;
        let program = if resources
            .virtual_fonts
            .contains_key(detached.recipe.name.as_str())
        {
            PdfFontProgramInput::Resident
        } else {
            detached_font_program(
                pdf,
                resources,
                &detached.recipe,
                map_entry.as_ref(),
                driver_dpi,
            )?
        };
        let mut glyph_names = encoding
            .as_ref()
            .map(|encoding| {
                encoding
                    .glyph_names()
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        if let PdfFontProgramInput::Type1(type1) = &program {
            glyph_names.extend((0..=255).filter_map(|code| type1.builtin_glyph_name(code)));
        }
        let glyph_to_unicode = glyph_names
            .into_iter()
            .filter_map(|name| {
                glyph_mappings
                    .iter()
                    .rev()
                    .find(|mapping| {
                        mapping.glyph_name == name
                            && mapping
                                .tfm_name
                                .as_deref()
                                .is_none_or(|tfm| tfm == detached.recipe.name.as_bytes())
                    })
                    .map(|mapping| (name, mapping.unicode.clone()))
            })
            .collect();
        fonts.insert(
            identity,
            PdfFontInput {
                artifact_resource,
                resource_number: detached.resource_number,
                object_number: detached.object_number,
                metrics: PdfFontMetricsInput {
                    widths: detached.widths.clone().try_into().map_err(|_| {
                        PdfBuildError::MissingFontResource(detached.recipe.name.clone())
                    })?,
                    heights: detached.heights.clone().try_into().map_err(|_| {
                        PdfBuildError::MissingFontResource(detached.recipe.name.clone())
                    })?,
                    depths: detached.depths.clone().try_into().map_err(|_| {
                        PdfBuildError::MissingFontResource(detached.recipe.name.clone())
                    })?,
                    x_height: detached.x_height,
                },
                included_codes: detached.included_codes.iter().copied().collect(),
                descriptor_entries: detached.descriptor_entries.clone(),
                generate_to_unicode: configuration.generates_to_unicode(),
                disable_builtin_to_unicode: detached.disable_builtin_to_unicode,
                infer_builtin_glyph_unicode: !glyph_mappings.is_empty(),
                omit_charset: configuration.omits_charset(),
                glyph_to_unicode,
                map_entry,
                encoding,
                program,
            },
        );
    }
    let (document_objects, mut next_object) = document_objects(pdf)?;
    virtual_fonts::materialize_destination_font_instances(
        pdf,
        resources,
        driver_dpi,
        &artifact_font_usage,
        &mut fonts,
        &mut next_object,
    )?;

    let images = pdf
        .images()
        .iter()
        .map(|image| {
            let dimensions = image.dimensions();
            (
                image.id().raw(),
                PdfExternalImageInput {
                    object: image.id().raw(),
                    identity: image.identity(),
                    metadata: image_metadata(image.metadata()),
                    width: dimensions.width,
                    height: dimensions.height,
                    depth: dimensions.depth,
                    color_space_object: u32::try_from(image.color_space_object())
                        .ok()
                        .filter(|object| *object != 0),
                    mask_object: image.mask_object(),
                    bytes: Arc::from(image.bytes()),
                },
            )
        })
        .collect();
    let raw_objects = pdf
        .raw_objects()
        .iter()
        .map(|record| {
            let payload = match &record.payload {
                None => None,
                Some(DetachedPdfRawObjectPayload::Value(bytes)) => {
                    Some(PdfRawObjectPayloadInput::Value(bytes.clone()))
                }
                Some(DetachedPdfRawObjectPayload::Stream { entries, data }) => {
                    Some(PdfRawObjectPayloadInput::Stream {
                        entries: entries.clone(),
                        data: Arc::from(data.as_slice()),
                    })
                }
                Some(DetachedPdfRawObjectPayload::FileStream {
                    entries,
                    source_name,
                }) => {
                    let receipt = raw_object_files
                        .entries
                        .get(&record.object)
                        .ok_or(PdfBuildError::MissingRawObjectFilePayload(record.object))?;
                    if receipt.source_name.as_bytes() != source_name
                        || crate::FileContentId::for_bytes(&receipt.bytes) != receipt.content_id
                    {
                        return Err(PdfBuildError::RawObjectFilePayloadMismatch(record.object));
                    }
                    Some(PdfRawObjectPayloadInput::Stream {
                        entries: entries.clone(),
                        data: Arc::from(receipt.bytes.as_slice()),
                    })
                }
            };
            Ok(PdfRawObjectInput {
                object: record.object,
                payload,
                immediate: record.immediate,
                referenced: record.referenced,
            })
        })
        .collect::<Result<Vec<_>, PdfBuildError>>()?;

    let document = pdf.document();
    let metadata = PdfDocumentMetadataInput {
        include_info_dictionary: document.include_info_dictionary,
        include_dates: document.include_dates,
        creation_date: pdf_date(document.clock),
        ptex_banner_key: (document.suppress_ptex_info % 2 == 0).then(|| {
            if document.ptex_use_underscore || parameters.major_version >= 2 {
                b"PTEX_Fullbanner".to_vec()
            } else {
                b"PTEX.Fullbanner".to_vec()
            }
        }),
        ptex_banner: tex_exec::BANNER.as_bytes().to_vec(),
        info_entries: document.fragments.info.clone(),
        catalog_entries: document.fragments.catalog.clone(),
        names_entries: document.fragments.names.clone(),
        trailer_entries: document.fragments.trailer.clone(),
        trailer_id: document.fragments.trailer_id.clone(),
        open_action: document.open_action.as_ref().map(indirect_action),
    };
    let virtual_fonts = resources
        .virtual_fonts
        .iter()
        .map(|(name, cached)| {
            (
                name.as_bytes().to_vec(),
                PdfVirtualFontInput {
                    program: cached.program.clone(),
                    local_tfms: resources
                        .local_tfms
                        .iter()
                        .map(|(name, cached)| {
                            (
                                name.as_bytes().to_vec(),
                                PdfVirtualLocalTfmInput {
                                    content_hash: tex_fonts::font_content_hash(&cached.bytes),
                                    bytes: Arc::from(cached.bytes.as_slice()),
                                    design_font: cached.font.clone(),
                                },
                            )
                        })
                        .collect(),
                },
            )
        })
        .collect();
    let dpi = configuration.resolved_pk_resolution(driver_dpi);
    Ok(PdfFinalizationInput {
        document: PdfDocumentInput {
            version: (version.major(), version.minor()),
            serialization: serialization_options(parameters)?,
            decimal_digits: parameters.decimal_digits as u8,
            draft_mode: parameters.draft_mode > 0,
            inclusion_copy_fonts: parameters.inclusion_copy_fonts > 0,
            unique_resource_names: parameters.unique_resource_names > 0,
            driver_dpi: dpi as u32,
            image_gamma: PdfImageGammaInput {
                gamma: parameters.gamma,
                image_gamma: parameters.image_gamma,
                high_color: parameters.image_hicolor > 0,
                apply_gamma: parameters.image_apply_gamma > 0,
            },
            pages_entries: document.pages_entries.clone(),
            form_omit_procset: document.form_omit_procset,
            suppress_page_group_warning: document.suppress_page_group_warning,
            metadata,
        },
        pages,
        forms,
        fonts,
        virtual_fonts,
        images,
        raw_objects,
        navigation: navigation(pdf),
        allocation: PdfAllocationInput {
            document: document_objects,
            next_object,
        },
        limits: PdfFinalizationLimits::default(),
    })
}

fn detached_encoding(
    pdf: &DetachedPdfCompletion,
    resources: &crate::PdfVirtualFontResources,
    name: &[u8],
) -> Option<tex_fonts::PdfEncoding> {
    pdf.font_operations()
        .iter()
        .rev()
        .find_map(|operation| match operation {
            DetachedPdfFontOperation::Encoding {
                logical_name,
                encoding,
            } if logical_name == name => Some(encoding.clone()),
            _ => None,
        })
        .or_else(|| resources.encodings.get(name).cloned())
}

fn detached_font_program(
    pdf: &DetachedPdfCompletion,
    resources: &crate::PdfVirtualFontResources,
    recipe: &tex_state::FontArtifactRecipe,
    map: Option<&tex_fonts::PdfFontMapEntry>,
    driver_dpi: i32,
) -> Result<PdfFontProgramInput, PdfBuildError> {
    let Some(map) = map else {
        let request = crate::virtual_compile::detached_pk_request(
            recipe,
            pdf.font_configuration().resolved_pk_resolution(driver_dpi),
        )
        .map_err(PdfBuildError::PkFont)?;
        let detached = pdf
            .font_operations()
            .iter()
            .rev()
            .find_map(|operation| match operation {
                DetachedPdfFontOperation::PkFont {
                    request: candidate,
                    font,
                } if candidate == &request => Some(font.clone()),
                _ => None,
            });
        let font = detached
            .or_else(|| resources.pk_fonts.get(&request).cloned())
            .ok_or_else(|| PdfBuildError::MissingPkFont(request.clone()))?;
        return Ok(PdfFontProgramInput::Pk { request, font });
    };
    if map.program == tex_fonts::PdfFontMapProgram::Resident {
        return Ok(PdfFontProgramInput::Resident);
    }
    let name = map
        .font_file
        .as_deref()
        .ok_or_else(|| PdfBuildError::MissingFontProgram(map.tex_name.clone()))?;
    if is_pdf_sfnt_program(name) {
        let program = pdf
            .font_operations()
            .iter()
            .rev()
            .find_map(|operation| match operation {
                DetachedPdfFontOperation::TrueTypeProgram {
                    logical_name,
                    program,
                } if logical_name == name => Some(program.clone()),
                _ => None,
            })
            .or_else(|| resources.truetype_programs.get(name).cloned());
        program
            .map(PdfFontProgramInput::TrueType)
            .ok_or_else(|| PdfBuildError::MissingFontProgram(name.to_vec()))
    } else {
        let program = pdf
            .font_operations()
            .iter()
            .rev()
            .find_map(|operation| match operation {
                DetachedPdfFontOperation::Type1Program {
                    logical_name,
                    program,
                } if logical_name == name => Some(program.clone()),
                _ => None,
            })
            .or_else(|| resources.type1_programs.get(name).cloned());
        program
            .map(PdfFontProgramInput::Type1)
            .ok_or_else(|| PdfBuildError::MissingFontProgram(name.to_vec()))
    }
}

fn document_objects(
    pdf: &DetachedPdfCompletion,
) -> Result<(PdfReservedDocumentObjects, u32), PdfBuildError> {
    let document = pdf.document();
    let mut next = pdf.next_object();
    let mut allocate = || -> Result<u32, PdfBuildError> {
        let object = (next <= i32::MAX as u32)
            .then_some(next)
            .ok_or(PdfBuildError::ObjectCapacity)?;
        next = next.checked_add(1).ok_or(PdfBuildError::ObjectCapacity)?;
        Ok(object)
    };
    let pages = document.objects.pages().map_or_else(&mut allocate, Ok)?;
    let needs_names = !document.fragments.names.is_empty()
        || pdf
            .destinations()
            .iter()
            .any(|record| matches!(record.identity(), PdfDestinationIdentity::Name(_)));
    let names = match document.objects.names() {
        Some(object) => Some(object),
        None if needs_names => Some(allocate()?),
        None => None,
    };
    let catalog = document.objects.catalog().map_or_else(&mut allocate, Ok)?;
    let info = match document.objects.info() {
        Some(object) => Some(object),
        None if document.include_info_dictionary => Some(allocate()?),
        None => None,
    };
    Ok((
        PdfReservedDocumentObjects {
            pages,
            names,
            catalog,
            info,
        },
        next,
    ))
}

fn image_metadata(metadata: PdfExternalImageMetadata) -> PdfImageMetadataInput {
    match metadata {
        PdfExternalImageMetadata::PdfPage {
            page_box,
            rotation,
            page,
            total_pages,
            has_page_group,
            pdf_version,
        } => PdfImageMetadataInput::PdfPage {
            page_box: PdfPageBoxInput {
                left: page_box.left,
                bottom: page_box.bottom,
                right: page_box.right,
                top: page_box.top,
            },
            rotation: match rotation {
                tex_state::PdfPageRotation::None => PdfPageRotationInput::None,
                tex_state::PdfPageRotation::Clockwise90 => PdfPageRotationInput::Clockwise90,
                tex_state::PdfPageRotation::UpsideDown => PdfPageRotationInput::UpsideDown,
                tex_state::PdfPageRotation::Clockwise270 => PdfPageRotationInput::Clockwise270,
            },
            page,
            total_pages,
            has_page_group,
            version: pdf_version,
        },
        PdfExternalImageMetadata::Raster(metadata) => PdfImageMetadataInput::Raster {
            format: match metadata.format {
                PdfRasterFormat::Jpeg => PdfRasterFormatInput::Jpeg,
                PdfRasterFormat::Png => PdfRasterFormatInput::Png,
            },
            width: metadata.width,
            height: metadata.height,
            bits_per_component: metadata.bits_per_component,
            color_space: match metadata.color_space {
                PdfRasterColorSpace::Gray => PdfRasterColorSpaceInput::Gray,
                PdfRasterColorSpace::Rgb => PdfRasterColorSpaceInput::Rgb,
                PdfRasterColorSpace::Cmyk => PdfRasterColorSpaceInput::Cmyk,
            },
            alpha: metadata.alpha,
            png_color_type: metadata.png_color_type,
        },
    }
}

fn navigation(pdf: &DetachedPdfCompletion) -> PdfNavigationInput {
    PdfNavigationInput {
        annotations: pdf
            .annotations()
            .iter()
            .map(|record| PdfAnnotationInput {
                object: record.object,
                data: record
                    .dimensions
                    .zip(record.entries.clone())
                    .map(|(dimensions, entries)| (annotation_dimensions(dimensions), entries)),
            })
            .collect(),
        links: pdf
            .links()
            .iter()
            .map(|record| PdfLinkInput {
                object: record.object,
                dimensions: annotation_dimensions(record.dimensions),
                entries: record.entries.clone(),
                action: action(&record.action),
            })
            .collect(),
        destinations: destinations(pdf.destinations()),
        structure_destinations: destinations(pdf.structure_destinations()),
        outlines: pdf
            .outlines()
            .iter()
            .map(|record| PdfOutlineInput {
                action_object: record.action_object,
                item_object: record.item_object,
                title_object: record.title_object,
                entries: record.entries.clone(),
                action: action(&record.action),
                count: record.count,
                title: record.title.clone(),
            })
            .collect(),
        threads: pdf
            .threads()
            .iter()
            .map(|thread| PdfThreadInput {
                identity: destination_identity(thread.identity()),
                object: thread.object(),
                beads: thread
                    .beads()
                    .iter()
                    .copied()
                    .map(|bead| PdfThreadBeadInput {
                        bead_object: bead.bead_object(),
                        rectangle_object: bead.rectangle_object(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn destinations(records: &[tex_state::PdfDestinationRecord]) -> Vec<PdfDestinationInput> {
    records
        .iter()
        .map(|record| PdfDestinationInput {
            identity: destination_identity(record.identity()),
            object: record.object(),
            structure_object: record.structure(),
            defined: record.defined(),
        })
        .collect()
}

fn indirect_action(record: &DetachedPdfActionRecord) -> PdfIndirectActionInput {
    PdfIndirectActionInput {
        object: record.id,
        target_object: record.target_object,
        structure_object: record.structure_object,
        action: action(&record.action),
    }
}

fn action(spec: &DetachedPdfAction) -> PdfActionInput {
    match spec {
        DetachedPdfAction::User(bytes) => PdfActionInput::User(bytes.clone()),
        DetachedPdfAction::GoTo(destination) => PdfActionInput::GoTo {
            file: destination.file.clone(),
            structure: destination.structure.as_ref().map(action_identity),
            target: action_target(&destination.target),
            new_window: action_window(destination.window),
        },
        DetachedPdfAction::Thread(destination) => PdfActionInput::Thread {
            file: destination.file.clone(),
            structure: destination.structure.as_ref().map(action_identity),
            target: action_target(&destination.target),
            new_window: action_window(destination.window),
        },
    }
}

fn action_target(target: &DetachedPdfActionTarget) -> PdfActionTargetInput {
    match target {
        DetachedPdfActionTarget::Page { number, view } => PdfActionTargetInput::Page {
            number: *number,
            view: view.clone(),
        },
        DetachedPdfActionTarget::Destination(identity) => {
            PdfActionTargetInput::Destination(action_identity(identity))
        }
    }
}

fn action_identity(identity: &DetachedPdfActionIdentifier) -> PdfDestinationIdentityInput {
    match identity {
        DetachedPdfActionIdentifier::Name(bytes) => {
            PdfDestinationIdentityInput::Name(bytes.clone())
        }
        DetachedPdfActionIdentifier::Number(number) => PdfDestinationIdentityInput::Number(*number),
        DetachedPdfActionIdentifier::Raw(bytes) => PdfDestinationIdentityInput::Raw(bytes.clone()),
    }
}

fn destination_identity(identity: &PdfDestinationIdentity) -> PdfDestinationIdentityInput {
    match identity {
        PdfDestinationIdentity::Name(name) => PdfDestinationIdentityInput::Name(name.clone()),
        PdfDestinationIdentity::Number(number) => PdfDestinationIdentityInput::Number(*number),
    }
}

fn action_window(window: PdfActionWindow) -> Option<bool> {
    match window {
        PdfActionWindow::Unspecified => None,
        PdfActionWindow::New => Some(true),
        PdfActionWindow::Same => Some(false),
    }
}

fn annotation_dimensions(dimensions: PdfAnnotationDimensions) -> PdfAnnotationDimensionsInput {
    PdfAnnotationDimensionsInput {
        width: dimensions.width,
        height: dimensions.height,
        depth: dimensions.depth,
    }
}
