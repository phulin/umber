//! Umber compatibility adapter for the detached `tex-out` PDF boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tex_arith::Scaled;
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
use tex_state::env::banks::{IntParam, TokParam};
use tex_state::{
    CommittedArtifact, PdfActionIdentifier, PdfActionRecord, PdfActionSpec, PdfActionTarget,
    PdfActionWindow, PdfAnnotationDimensions, PdfDestinationIdentity, PdfDocumentFragmentKind,
    PdfExternalImageMetadata, PdfOutputParameters, PdfPageRecord, PdfRasterColorSpace,
    PdfRasterFormat, Universe,
};

use super::{
    PdfBuildError, artifact_bytes, document_fragment_bytes, is_pdf_sfnt_program, output_parameters,
    pdf_date, pdf_version, pk_font_request, serialization_options, token_list_bytes,
};

/// Freezes the accepted engine ledger and host-owned resources into the sole
/// input contract consumed by `tex-out` PDF finalization.
///
/// This is intentionally the last Umber-owned step: artifact lookup, accepted
/// raw-object payload binding, token expansion, engine identifiers, and
/// diagnostics remain on this side of the boundary.
pub fn pdf_finalization_input(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    driver_dpi: i32,
    virtual_fonts: &crate::PdfVirtualFontResources,
) -> Result<PdfFinalizationInput, PdfBuildError> {
    pdf_finalization_input_with_raw_object_files(
        stores,
        artifacts,
        driver_dpi,
        virtual_fonts,
        &crate::PdfRawObjectFileReceipt::default(),
    )
}

/// Freezes finalization input using immutable raw-object file payloads captured
/// by the accepted resource session.
pub fn pdf_finalization_input_with_raw_object_files(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    driver_dpi: i32,
    virtual_fonts: &crate::PdfVirtualFontResources,
    raw_object_files: &crate::PdfRawObjectFileReceipt,
) -> Result<PdfFinalizationInput, PdfBuildError> {
    let page_records = stores.pdf_pages().to_vec();
    pdf_finalization_input_with_page_records(
        stores,
        artifacts,
        &page_records,
        driver_dpi,
        virtual_fonts,
        raw_object_files,
    )
}

pub(super) fn pdf_finalization_input_with_page_records(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    page_records: &[PdfPageRecord],
    driver_dpi: i32,
    virtual_fonts: &crate::PdfVirtualFontResources,
    raw_object_files: &crate::PdfRawObjectFileReceipt,
) -> Result<PdfFinalizationInput, PdfBuildError> {
    let parameters = output_parameters(stores);
    if parameters.output <= 0 {
        return Err(PdfBuildError::PdfOutputDisabled);
    }
    let version = pdf_version(parameters)?;
    let pages = page_records
        .iter()
        .map(|record| {
            let bytes = artifact_bytes(stores, artifacts, record.artifact())?;
            Ok(PdfCommittedPageInput {
                artifact_hash: record.artifact(),
                artifact_bytes: Arc::from(bytes),
                resources_object: record.resources_object(),
                contents_object: record.contents_object(),
                page_object: record.page_object(),
                h_origin: record.h_origin(),
                v_origin: record.v_origin(),
                width: record.width(),
                height: record.height(),
                link_margin: record.link_margin(),
                page_entries: token_list_bytes(stores, record.page_attr()),
                resource_entries: token_list_bytes(stores, record.resources()),
                omit_procset: record.omit_procset(),
                space_font_name: stores
                    .pdf_space_font_name(record.space_font_name_id())
                    .ok_or(PdfBuildError::MissingSpaceFontName(
                        record.space_font_name_id(),
                    ))?
                    .to_vec(),
            })
        })
        .collect::<Result<Vec<_>, PdfBuildError>>()?;

    let forms = stores
        .pdf_forms()
        .filter_map(|record| {
            let artifact = stores.pdf_form_artifact(record.object())?;
            Some(Ok((
                record.object(),
                PdfFormInput {
                    object: record.object(),
                    resource: record.resource(),
                    artifact_bytes: Arc::from(artifact.bytes()),
                    width: record.width(),
                    height: record.height(),
                    depth: record.depth(),
                    entries: record
                        .attr()
                        .map(|tokens| token_list_bytes(stores, tokens))
                        .unwrap_or_default(),
                    resource_entries: record
                        .resources()
                        .map(|tokens| token_list_bytes(stores, tokens))
                        .unwrap_or_default(),
                    immediate: record.immediate(),
                },
            )))
        })
        .collect::<Result<BTreeMap<_, _>, PdfBuildError>>()?;

    // Run the same bounded, pure packet walk once at the host boundary to
    // materialize the live font instances and pdfTeX resource reservations
    // that first packet use would allocate. The positioned candidate is
    // discarded: tex-out repeats lowering from the committed artifacts using
    // only the detached closure captured below.
    let mut detached_stores = stores.clone();
    let (virtual_positioned, reserved_virtual_fonts) = reserve_virtual_font_resources(
        &mut detached_stores,
        artifacts,
        page_records,
        virtual_fonts,
    )?;

    let artifacts_by_font = pages
        .iter()
        .map(|page| page.artifact_bytes.as_ref())
        .chain(forms.values().map(|form| form.artifact_bytes.as_ref()))
        .map(tex_out::PageArtifact::from_bytes)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flat_map(|artifact| artifact.fonts.clone())
        .chain(
            virtual_positioned
                .iter()
                .flat_map(|positioned| positioned.fonts.iter().cloned()),
        )
        .chain(reserved_virtual_fonts.into_iter().map(|font_id| {
            let resource = detached_stores
                .pdf_font_resource(font_id)
                .expect("VF reservation receipt names a checkpointed resource");
            let font = detached_stores.font(resource.font());
            tex_out::FontResource {
                font_id: resource.resource_number(),
                name: font.name().to_owned(),
                tfm_content_hash: tex_out::ContentIdentity::new(font.content_hash()),
                tfm_checksum: font.checksum(),
                design_size: font.design_size(),
                at_size: font.size(),
                layout_policy: font.layout_policy(),
                mapping_fallback: font.mapping_fallback(),
                opentype: None,
                semantic_identity: font.source_identity(),
                construction: tex_out::FontResourceConstruction::Loaded,
            }
        }))
        .map(|font| (font.semantic_identity, font))
        .collect::<BTreeMap<_, _>>();
    let resolved_map = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .map(|entry| (entry.tex_name.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let font_configuration = stores.pdf_font_configuration();
    let mut fonts = BTreeMap::new();
    for (identity, artifact_resource) in artifacts_by_font {
        let font_id = detached_stores
            .font_by_source_identity(identity)
            .ok_or_else(|| PdfBuildError::MissingLiveFont(artifact_resource.name.clone()))?;
        let resource = detached_stores
            .pdf_font_resource_by_identity(identity)
            .ok_or_else(|| PdfBuildError::MissingFontResource(artifact_resource.name.clone()))?;
        let loaded = detached_stores.font(font_id);
        let map_entry = resolved_map.get(artifact_resource.name.as_bytes()).cloned();
        let encoding = map_entry
            .as_ref()
            .and_then(|entry| entry.encoding_files.first())
            .map(|name| {
                detached_stores
                    .pdf_encoding(name)
                    .cloned()
                    .ok_or_else(|| PdfBuildError::MissingEncoding(name.clone()))
            })
            .transpose()?;
        let program = if virtual_fonts
            .virtual_fonts
            .contains_key(artifact_resource.name.as_str())
        {
            // A virtual font is composition only and is never emitted as a
            // PDF font dictionary. Its exact VF/TFM closure is below.
            PdfFontProgramInput::Resident
        } else {
            detached_font_program(&detached_stores, font_id, map_entry.as_ref(), driver_dpi)?
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
                detached_stores
                    .pdf_glyph_to_unicode(loaded.name().as_bytes(), &name)
                    .or_else(|| detached_stores.pdf_glyph_to_unicode(&[], &name))
                    .map(|unicode| (name, unicode.to_vec()))
            })
            .collect();
        let metrics = PdfFontMetricsInput {
            widths: std::array::from_fn(|code| {
                detached_stores
                    .font_char_metrics(font_id, code as u8)
                    .map_or(Scaled::from_raw(0), |metric| metric.width)
            }),
            heights: std::array::from_fn(|code| {
                detached_stores
                    .font_char_metrics(font_id, code as u8)
                    .map_or(Scaled::from_raw(0), |metric| metric.height)
            }),
            depths: std::array::from_fn(|code| {
                detached_stores
                    .font_char_metrics(font_id, code as u8)
                    .map_or(Scaled::from_raw(0), |metric| metric.depth)
            }),
            x_height: detached_stores.font_parameter(font_id, 5),
        };
        fonts.insert(
            identity,
            PdfFontInput {
                artifact_resource,
                resource_number: resource.resource_number(),
                object_number: resource.object_number(),
                metrics,
                included_codes: detached_stores
                    .included_pdf_font_chars(font_id)
                    .into_iter()
                    .collect(),
                descriptor_entries: detached_stores.pdf_font_attribute(font_id).to_vec(),
                generate_to_unicode: font_configuration.generates_to_unicode(),
                disable_builtin_to_unicode: detached_stores
                    .pdf_builtin_to_unicode_disabled(font_id),
                infer_builtin_glyph_unicode: detached_stores.has_pdf_glyph_to_unicode_mappings(),
                omit_charset: font_configuration.omits_charset(),
                glyph_to_unicode,
                map_entry,
                encoding,
                program,
            },
        );
    }

    let images = stores
        .pdf_external_images()
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

    let mut raw_objects = Vec::new();
    let raw_records = stores.pdf_raw_objects().to_vec();
    for record in raw_records {
        let payload = match record.data() {
            None => None,
            Some(data) if data.is_stream() => {
                let source = token_list_bytes(stores, data.data());
                let bytes = if data.is_file() {
                    let entry = raw_object_files.entries.get(&record.id().raw()).ok_or(
                        PdfBuildError::MissingRawObjectFilePayload(record.id().raw()),
                    )?;
                    if entry.source_name.as_bytes() != source
                        || crate::FileContentId::for_bytes(&entry.bytes) != entry.content_id
                    {
                        return Err(PdfBuildError::RawObjectFilePayloadMismatch(
                            record.id().raw(),
                        ));
                    }
                    Arc::from(entry.bytes.as_slice())
                } else {
                    Arc::from(source)
                };
                Some(PdfRawObjectPayloadInput::Stream {
                    entries: data
                        .stream_attr()
                        .map(|tokens| token_list_bytes(stores, tokens))
                        .unwrap_or_default(),
                    data: bytes,
                })
            }
            Some(data) => Some(PdfRawObjectPayloadInput::Value(token_list_bytes(
                stores,
                data.data(),
            ))),
        };
        raw_objects.push(PdfRawObjectInput {
            object: record.id().raw(),
            payload,
            immediate: record.is_immediate(),
            referenced: record.is_referenced(),
        });
    }

    let include_info = stores.int_param(IntParam::PDF_OMIT_INFO_DICT) == 0;
    let mut allocation_state = detached_stores.clone();
    let ids = allocation_state
        .finalize_pdf_document_objects(include_info)
        .map_err(|_| PdfBuildError::ObjectCapacity)?;
    let document_objects = PdfReservedDocumentObjects {
        pages: ids.pages().expect("finalization reserves pages"),
        names: ids.names(),
        catalog: ids.catalog().expect("finalization reserves catalog"),
        info: ids.info(),
    };
    let open_action = stores
        .pdf_catalog_open_action()
        .map(|record| indirect_action(stores, record));
    let clock = stores.world().job_clock();
    let metadata = PdfDocumentMetadataInput {
        include_info_dictionary: include_info,
        include_dates: stores.int_param(IntParam::PDF_INFO_OMIT_DATE) == 0,
        creation_date: pdf_date(clock),
        ptex_banner_key: (stores.int_param(IntParam::PDF_SUPPRESS_PTEX_INFO) % 2 == 0).then(|| {
            if stores.int_param(IntParam::PDF_PTEX_USE_UNDERSCORE) > 0
                || parameters.major_version >= 2
            {
                b"PTEX_Fullbanner".to_vec()
            } else {
                b"PTEX.Fullbanner".to_vec()
            }
        }),
        ptex_banner: tex_exec::BANNER.as_bytes().to_vec(),
        info_entries: document_fragment_bytes(stores, PdfDocumentFragmentKind::Info),
        catalog_entries: document_fragment_bytes(stores, PdfDocumentFragmentKind::Catalog),
        names_entries: document_fragment_bytes(stores, PdfDocumentFragmentKind::Names),
        trailer_entries: document_fragment_bytes(stores, PdfDocumentFragmentKind::Trailer),
        trailer_id: document_fragment_bytes(stores, PdfDocumentFragmentKind::TrailerId),
        open_action,
    };
    let navigation = navigation(stores);
    let virtual_fonts = virtual_fonts
        .virtual_fonts
        .iter()
        .map(|(name, cached)| {
            (
                name.as_bytes().to_vec(),
                PdfVirtualFontInput {
                    program: cached.program.clone(),
                    local_tfms: virtual_fonts
                        .local_tfms
                        .iter()
                        .map(|(name, cached)| {
                            (
                                name.as_bytes().to_vec(),
                                PdfVirtualLocalTfmInput {
                                    content_hash: cached.content_id.bytes(),
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
    let dpi = if parameters.pk_resolution == 0 {
        driver_dpi.clamp(72, 8_000)
    } else {
        parameters.pk_resolution
    };
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
            pages_entries: token_list_bytes(stores, stores.tok_param(TokParam::PDF_PAGES_ATTR)),
            form_omit_procset: stores.int_param(IntParam::PDF_OMIT_PROCSET),
            suppress_page_group_warning: stores
                .int_param(IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP)
                != 0,
            metadata,
        },
        pages,
        forms,
        fonts,
        virtual_fonts,
        images,
        raw_objects,
        navigation,
        allocation: PdfAllocationInput {
            document: document_objects,
            next_object: allocation_state.pdf_next_object_id(),
        },
        limits: PdfFinalizationLimits::default(),
    })
}

/// Replays the bounded VF first-use order against the supplied private state
/// and returns the lowered candidate solely as a font-closure receipt.
pub(crate) fn reserve_virtual_font_resources(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    page_records: &[tex_state::PdfPageRecord],
    virtual_fonts: &crate::PdfVirtualFontResources,
) -> Result<
    (
        Vec<tex_out::positioned::PositionedPage>,
        BTreeSet<tex_state::ids::FontId>,
    ),
    PdfBuildError,
> {
    let mut positioned = super::positioned_pages(stores, artifacts, page_records)?;
    positioned.extend(
        super::positioned_forms(stores)?
            .into_iter()
            .map(|(_, positioned)| positioned),
    );
    let reserved = crate::pdf_vf::lower_pages_with_resource_receipt(
        stores,
        &mut positioned,
        virtual_fonts,
        crate::pdf_vf::PdfVfLimits::default(),
    )?;
    Ok((positioned, reserved))
}

fn detached_font_program(
    stores: &Universe,
    font_id: tex_state::ids::FontId,
    map: Option<&tex_fonts::PdfFontMapEntry>,
    driver_dpi: i32,
) -> Result<PdfFontProgramInput, PdfBuildError> {
    let Some(map) = map else {
        let request =
            pk_font_request(stores, font_id, driver_dpi).map_err(PdfBuildError::PkFont)?;
        let font = stores
            .pdf_pk_font(&request)
            .cloned()
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
        stores
            .pdf_truetype_program(name)
            .cloned()
            .map(PdfFontProgramInput::TrueType)
            .ok_or_else(|| PdfBuildError::MissingFontProgram(name.to_vec()))
    } else {
        stores
            .pdf_type1_program(name)
            .cloned()
            .map(PdfFontProgramInput::Type1)
            .ok_or_else(|| PdfBuildError::MissingFontProgram(name.to_vec()))
    }
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

fn navigation(stores: &Universe) -> PdfNavigationInput {
    PdfNavigationInput {
        annotations: stores
            .pdf_annotations()
            .iter()
            .map(|record| PdfAnnotationInput {
                object: record.object(),
                data: record.data().map(|data| {
                    (
                        dimensions(data.dimensions),
                        token_list_bytes(stores, data.entries.id()),
                    )
                }),
            })
            .collect(),
        links: stores
            .pdf_links()
            .iter()
            .map(|record| PdfLinkInput {
                object: record.object(),
                dimensions: dimensions(record.dimensions()),
                entries: token_list_bytes(stores, record.attributes()),
                action: action(stores, record.action()),
            })
            .collect(),
        destinations: destinations(stores, false),
        structure_destinations: destinations(stores, true),
        outlines: stores
            .pdf_outlines()
            .iter()
            .map(|record| PdfOutlineInput {
                action_object: record.action_object(),
                item_object: record.item_object(),
                title_object: record.title_object(),
                entries: token_list_bytes(stores, record.attributes()),
                action: action(stores, record.action()),
                count: record.count(),
                title: token_list_bytes(stores, record.title()),
            })
            .collect(),
        threads: stores
            .pdf_threads()
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

fn destinations(stores: &Universe, structure: bool) -> Vec<PdfDestinationInput> {
    stores
        .pdf_destinations(structure)
        .iter()
        .map(|record| PdfDestinationInput {
            identity: destination_identity(record.identity()),
            object: record.object(),
            structure_object: record.structure(),
            defined: record.defined(),
        })
        .collect()
}

fn indirect_action(stores: &Universe, record: PdfActionRecord) -> PdfIndirectActionInput {
    PdfIndirectActionInput {
        object: record.id(),
        target_object: record.target_object(),
        structure_object: record.structure_object(),
        action: action(stores, record.spec()),
    }
}

fn action(stores: &Universe, spec: PdfActionSpec) -> PdfActionInput {
    match spec {
        PdfActionSpec::User(tokens) => PdfActionInput::User(token_list_bytes(stores, tokens.id())),
        PdfActionSpec::GoTo(destination) => PdfActionInput::GoTo {
            file: destination
                .file
                .map(|tokens| token_list_bytes(stores, tokens.id())),
            structure: destination
                .structure
                .map(|identity| action_identity(stores, identity)),
            target: action_target(stores, destination.target),
            new_window: action_window(destination.window),
        },
        PdfActionSpec::Thread(destination) => PdfActionInput::Thread {
            file: destination
                .file
                .map(|tokens| token_list_bytes(stores, tokens.id())),
            structure: destination
                .structure
                .map(|identity| action_identity(stores, identity)),
            target: action_target(stores, destination.target),
            new_window: action_window(destination.window),
        },
    }
}

fn action_target(stores: &Universe, target: PdfActionTarget) -> PdfActionTargetInput {
    match target {
        PdfActionTarget::Page { number, view } => PdfActionTargetInput::Page {
            number,
            view: token_list_bytes(stores, view.id()),
        },
        PdfActionTarget::Destination(identity) => {
            PdfActionTargetInput::Destination(action_identity(stores, identity))
        }
    }
}

fn action_identity(
    stores: &Universe,
    identity: PdfActionIdentifier,
) -> PdfDestinationIdentityInput {
    match identity {
        PdfActionIdentifier::Name(tokens) => {
            PdfDestinationIdentityInput::Name(token_list_bytes(stores, tokens.id()))
        }
        PdfActionIdentifier::Number(number) => PdfDestinationIdentityInput::Number(number),
        PdfActionIdentifier::Raw(tokens) => {
            PdfDestinationIdentityInput::Raw(token_list_bytes(stores, tokens.id()))
        }
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

fn dimensions(dimensions: PdfAnnotationDimensions) -> PdfAnnotationDimensionsInput {
    PdfAnnotationDimensionsInput {
        width: dimensions.width,
        height: dimensions.height,
        depth: dimensions.depth,
    }
}

#[allow(dead_code)]
fn _assert_parameter_is_copy(_: PdfOutputParameters) {}
