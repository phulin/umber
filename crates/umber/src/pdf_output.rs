//! Detached PDF assembly from checkpointed shipout receipts.

use md5::{Digest, Md5};
use tex_arith::Scaled;
use tex_expand::append_token_string_text;
use tex_out::PageNode;
use tex_out::pdf::{
    PdfContentRectangle, PdfContentTextRun, PdfDictionary, PdfIndirectObject, PdfModelError,
    PdfName, PdfNumber, PdfObject, PdfObjectCompression, PdfObjectId, PdfSerializationOptions,
    PdfSerializeError, PdfStreamCompression, PdfTrailer, PdfValue, PdfVersion,
    UnvalidatedPdfDocument, page_content,
};
use tex_out::positioned::{PositionedError, PositionedEvent};
use tex_state::env::banks::{IntParam, TokParam};
use tex_state::ids::FontId;
use tex_state::ids::TokenListId;
use tex_state::{
    CommittedArtifact, ContentHash, PdfActionDestination, PdfActionIdentifier, PdfActionRecord,
    PdfActionSpec, PdfActionTarget, PdfActionWindow, PdfDocumentFragmentKind, PdfOutputParameters,
    Universe, WorldError,
};

use std::collections::{BTreeMap, BTreeSet};

pub(crate) const DEFAULT_PDF_PK_RESOLUTION: i32 = 600;

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
    pdf_from_committed_artifacts_at_dpi(stores, artifacts, DEFAULT_PDF_PK_RESOLUTION)
}

/// Builds a PDF using an explicit host bitmap-device DPI when
/// `\pdfpkresolution` retains its zero sentinel.
pub fn pdf_from_committed_artifacts_at_dpi(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    driver_dpi: i32,
) -> Result<Vec<u8>, PdfBuildError> {
    let parameters = output_parameters(stores);
    if parameters.output <= 0 {
        return Err(PdfBuildError::PdfOutputDisabled);
    }
    let version = pdf_version(parameters)?;
    let options = serialization_options(parameters)?;
    let page_records = stores.pdf_pages().to_vec();
    let font_usage = collect_font_usage(stores, artifacts, &page_records)?;
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
    let mut objects =
        Vec::with_capacity(2 + page_records.len() * 3 + stores.pdf_raw_objects().len() + 2);
    let mut kids = Vec::with_capacity(page_records.len());
    let mut emitted_fonts = std::collections::BTreeSet::new();
    let mut interword_space_enabled = false;
    let mut fallback_space_font = None;

    let mut catalog = PdfDictionary::new();
    catalog.insert("Type", PdfValue::Name("Catalog".into()))?;
    catalog.insert("Pages", PdfValue::Reference(pages_id))?;
    if let Some(names) = document_ids.names() {
        catalog.insert("Names", PdfValue::Reference(object_id(names)?))?;
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
            object: PdfObject::Raw(pdf_action_bytes(stores, action, &page_records)?),
        });
        if let Some(target) = action.target_object()
            && matches!(
                action.spec(),
                PdfActionSpec::GoTo(PdfActionDestination {
                    target: PdfActionTarget::Destination(_),
                    ..
                }) | PdfActionSpec::Thread(PdfActionDestination {
                    target: PdfActionTarget::Destination(_),
                    ..
                })
            )
        {
            objects.push(PdfIndirectObject {
                id: object_id(target)?,
                object: PdfObject::Raw(pdf_action_placeholder(action, &page_records)?),
            });
        }
        if let Some(structure) = action.structure_object() {
            objects.push(PdfIndirectObject {
                id: object_id(structure)?,
                object: PdfObject::Raw(b"null".to_vec()),
            });
        }
    }

    if let Some(names) = document_ids.names() {
        let mut dictionary = PdfDictionary::new();
        dictionary.set_raw_entries(document_fragment_bytes(
            stores,
            PdfDocumentFragmentKind::Names,
        ));
        objects.push(indirect_dictionary(object_id(names)?, dictionary));
    }

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

    for (page_index, record) in page_records.iter().copied().enumerate() {
        let bytes = artifact_bytes(stores, artifacts, record.artifact())?;
        let artifact = tex_out::PageArtifact::from_bytes(&bytes)?;
        let positioned = tex_out::positioned::lower_page(&artifact, page_index as u32)?;
        let (page_width, page_height) = pdf_page_extents(&artifact, record)?;
        let mut rectangles = Vec::new();
        let mut text_runs = Vec::new();
        let mut page_fonts = std::collections::BTreeMap::new();
        let mut fallback_space_on_page = false;
        for event in positioned.events {
            match event {
                PositionedEvent::Rule(rule) => rectangles.push(PdfContentRectangle {
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
                }),
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
                                let mapped = stores
                                    .resolved_pdf_font_map_lines()
                                    .into_iter()
                                    .any(|entry| entry.tex_name == font.name.as_bytes());
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
                                objects.extend(pdf_font_objects(
                                    stores,
                                    ids,
                                    font,
                                    &resource_name,
                                    used_codes,
                                    driver_dpi,
                                )?);
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
                    let explicit_space = font_has_explicit_space(stores, font.name.as_bytes());
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
                                    text_runs.push(PdfContentTextRun {
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
                                    });
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
                                    text_runs.push(PdfContentTextRun {
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
                                    });
                                }
                            }
                        }
                    }
                    if !segment.is_empty() {
                        text_runs.push(PdfContentTextRun {
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
                        });
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
                        text_runs.push(PdfContentTextRun {
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
                        });
                    }
                },
                PositionedEvent::PdfAnnotation(_) => {}
                PositionedEvent::Special(special) => {
                    return Err(PdfBuildError::UnsupportedSpecial(special.class));
                }
                PositionedEvent::Box(_) | PositionedEvent::TextRun(_) => {}
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
        resources.set_raw_entries(token_list_bytes(stores, record.resources()));
        objects.push(indirect_dictionary(resources_id, resources));
        objects.push(PdfIndirectObject {
            id: contents_id,
            object: PdfObject::Stream {
                dictionary: PdfDictionary::new(),
                data: page_content(&rectangles, &text_runs),
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
        page.set_raw_entries(page_attr);
        objects.push(indirect_dictionary(page_id, page));
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
    Ok(document.to_pdf_bytes_with_options(options)?)
}

fn collect_font_usage(
    stores: &Universe,
    artifacts: &[CommittedArtifact],
    page_records: &[tex_state::PdfPageRecord],
) -> Result<BTreeMap<u32, BTreeSet<u8>>, PdfBuildError> {
    let mut usage = BTreeMap::<u32, BTreeSet<u8>>::new();
    let mut interword_space_enabled = false;
    for (page_index, record) in page_records.iter().copied().enumerate() {
        let bytes = artifact_bytes(stores, artifacts, record.artifact())?;
        let artifact = tex_out::PageArtifact::from_bytes(&bytes)?;
        let positioned = tex_out::positioned::lower_page(&artifact, page_index as u32)?;
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
            let font = positioned
                .fonts
                .iter()
                .find(|font| font.font_id == run.font_id)
                .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
            let resource = stores
                .pdf_font_resource_by_identity(font.semantic_identity)
                .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
            let codes = usage.entry(resource.object_number()).or_default();
            let explicit_space =
                interword_space_enabled && font_has_explicit_space(stores, font.name.as_bytes());
            codes.extend(run.units.iter().zip(&run.physical_codes).filter_map(
                |(unit, physical_code)| match unit {
                    tex_out::positioned::TextUnit::Code(_) => *physical_code,
                    tex_out::positioned::TextUnit::Space if explicit_space => Some(b' '),
                    tex_out::positioned::TextUnit::Space => None,
                },
            ));
            let live_font = stores
                .font_by_source_identity(font.semantic_identity)
                .ok_or_else(|| PdfBuildError::MissingLiveFont(font.name.clone()))?;
            codes.extend(stores.included_pdf_font_chars(live_font));
        }
    }
    Ok(usage)
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

fn font_has_explicit_space(stores: &Universe, tex_name: &[u8]) -> bool {
    stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .find(|entry| entry.tex_name == tex_name)
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
) -> Result<Vec<PdfIndirectObject>, PdfBuildError> {
    let mapped = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .find(|entry| entry.tex_name == font.name.as_bytes());
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

fn pdf_action_bytes(
    stores: &Universe,
    record: PdfActionRecord,
    pages: &[tex_state::PdfPageRecord],
) -> Result<Vec<u8>, PdfBuildError> {
    let mut out = Vec::new();
    match record.spec() {
        PdfActionSpec::User(tokens) => return Ok(token_list_bytes(stores, tokens)),
        PdfActionSpec::GoTo(action) => {
            out.extend_from_slice(b"<< ");
            write_action_destination(stores, record, action, pages, true, &mut out)?;
        }
        PdfActionSpec::Thread(action) => {
            out.extend_from_slice(b"<< ");
            write_action_destination(stores, record, action, pages, false, &mut out)?;
        }
    }
    out.extend_from_slice(b" >>");
    Ok(out)
}

fn write_action_destination(
    stores: &Universe,
    record: PdfActionRecord,
    action: PdfActionDestination,
    pages: &[tex_state::PdfPageRecord],
    goto: bool,
    out: &mut Vec<u8>,
) -> Result<(), PdfBuildError> {
    if let Some(file) = action.file {
        out.extend_from_slice(b"/F ");
        write_file_string(&token_list_bytes(stores, file), out);
        out.push(b' ');
        match action.window {
            PdfActionWindow::New => out.extend_from_slice(b"/NewWindow true "),
            PdfActionWindow::Same => out.extend_from_slice(b"/NewWindow false "),
            PdfActionWindow::Unspecified => {}
        }
    }
    match action.target {
        PdfActionTarget::Page { number, view } => {
            if action.file.is_some() {
                out.extend_from_slice(b"/S /GoToR /D [");
                out.extend_from_slice((number - 1).to_string().as_bytes());
            } else {
                let page = pages
                    .get((number - 1) as usize)
                    .ok_or(PdfBuildError::OpenActionPageNotFound(number))?;
                out.extend_from_slice(b"/S /GoTo /D [");
                out.extend_from_slice(page.page_object().to_string().as_bytes());
                out.extend_from_slice(b" 0 R");
            }
            out.push(b' ');
            out.extend_from_slice(&token_list_bytes(stores, view));
            out.push(b']');
        }
        PdfActionTarget::Destination(identifier) => {
            if goto {
                out.extend_from_slice(if action.file.is_some() {
                    b"/S /GoToR /D ".as_slice()
                } else {
                    b"/S /GoTo /D ".as_slice()
                });
            } else {
                out.extend_from_slice(b"/S /Thread /D ");
            }
            write_action_identifier(
                stores,
                identifier,
                action.file.is_some(),
                record.target_object(),
                out,
            );
        }
    }
    if let Some(structure) = action.structure {
        out.extend_from_slice(b" /SD ");
        if action.file.is_some() {
            match structure {
                PdfActionIdentifier::Raw(tokens) => {
                    out.extend_from_slice(&token_list_bytes(stores, tokens));
                }
                _ => unreachable!("external structure identifier is raw"),
            }
        } else {
            out.extend_from_slice(
                record
                    .structure_object()
                    .expect("internal structure object")
                    .to_string()
                    .as_bytes(),
            );
            out.extend_from_slice(b" 0 R");
        }
    }
    Ok(())
}

fn write_action_identifier(
    stores: &Universe,
    identifier: PdfActionIdentifier,
    external: bool,
    target_object: Option<u32>,
    out: &mut Vec<u8>,
) {
    match identifier {
        PdfActionIdentifier::Name(tokens) => {
            write_pdf_string(&token_list_bytes(stores, tokens), out)
        }
        PdfActionIdentifier::Number(number) if external => {
            out.extend_from_slice(number.to_string().as_bytes());
        }
        PdfActionIdentifier::Number(_) => {
            out.extend_from_slice(
                target_object
                    .expect("internal target object")
                    .to_string()
                    .as_bytes(),
            );
            out.extend_from_slice(b" 0 R");
        }
        PdfActionIdentifier::Raw(_) => unreachable!("raw identifier is structure-only"),
    }
}

fn write_file_string(bytes: &[u8], out: &mut Vec<u8>) {
    if bytes.first() == Some(&b'(') && bytes.last() == Some(&b')') {
        out.extend_from_slice(bytes);
    } else {
        write_pdf_string(bytes, out);
    }
}

fn write_pdf_string(bytes: &[u8], out: &mut Vec<u8>) {
    out.push(b'(');
    for &byte in bytes {
        if matches!(byte, b'\\' | b'(' | b')') {
            out.push(b'\\');
        }
        out.push(byte);
    }
    out.push(b')');
}

fn pdf_action_placeholder(
    action: PdfActionRecord,
    pages: &[tex_state::PdfPageRecord],
) -> Result<Vec<u8>, PdfBuildError> {
    if matches!(action.spec(), PdfActionSpec::GoTo(_)) {
        let page = pages.first().ok_or(PdfBuildError::OpenActionHasNoPage)?;
        Ok(format!("[{} 0 R /Fit]", page.page_object()).into_bytes())
    } else {
        Ok(b"null".to_vec())
    }
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
    OpenActionPageNotFound(u32),
    OpenActionHasNoPage,
    ReferencedRawObjectUninitialized(u32),
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
    UnsupportedSpecial(String),
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
            Self::UnsupportedSpecial(class) => {
                write!(f, "PDF output does not support special class {class:?}")
            }
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
    use tex_exec::ExecutionContext;
    use tex_lex::{InputStack, MemoryInput};
    use tex_state::{JobClock, World};

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
            "\\pdfinfoomitdate=1\\pdfsuppressptexinfo=1\\end",
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
            "\\shipout\\vbox{\\hrule width1pt height1pt}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&mut stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let pages = parsed.get_pages();
        for (page_number, expected) in [(1, false), (2, true)] {
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
}
