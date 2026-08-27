//! Handle-free terminal projection of the checkpointed PDF ledger.
//!
//! This is an admitted cold boundary, not a second PDF state authority.  The
//! live ledger keeps generation coordinates while commands run; terminal
//! completion resolves them once into this owned value and then retires the
//! generation independently.

use tex_arith::Scaled;

use crate::{ContentHash, FontArtifactRecipe, JobClock, PdfExternalImageRecord};

use super::{
    PdfActionDestination, PdfActionIdentifier, PdfActionRecord, PdfActionSpec, PdfActionTarget,
    PdfActionWindow, PdfAnnotationDimensions, PdfDestinationRecord, PdfDocumentFragmentKind,
    PdfDocumentObjectIds, PdfFontConfiguration, PdfFontMapOperation, PdfFontOperation,
    PdfGlyphToUnicode, PdfOutputParameters, PdfState, PdfThreadRecord,
};

/// Failure while resolving one live PDF ledger into terminal owned values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfCompletionError {
    MissingPageArtifact(ContentHash),
    MissingSpaceFontName(u32),
    MissingFormArtifact(u32),
    InvalidTokenList,
    ArtifactRead { hash: ContentHash, message: String },
}

impl std::fmt::Display for PdfCompletionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPageArtifact(hash) => {
                write!(formatter, "PDF page artifact {} is unavailable", hash.hex())
            }
            Self::MissingSpaceFontName(id) => {
                write!(formatter, "PDF space-font spelling {id} is unavailable")
            }
            Self::MissingFormArtifact(object) => {
                write!(
                    formatter,
                    "PDF form object {object} has no committed artifact"
                )
            }
            Self::InvalidTokenList => formatter.write_str("PDF token-list coordinate is stale"),
            Self::ArtifactRead { hash, message } => {
                write!(
                    formatter,
                    "cannot read PDF page artifact {}: {message}",
                    hash.hex()
                )
            }
        }
    }
}

impl std::error::Error for PdfCompletionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfPage {
    pub artifact: ContentHash,
    pub artifact_bytes: Vec<u8>,
    pub resources_object: u32,
    pub contents_object: u32,
    pub page_object: u32,
    pub h_origin: Scaled,
    pub v_origin: Scaled,
    pub width: Scaled,
    pub height: Scaled,
    pub link_margin: Scaled,
    pub page_entries: Vec<u8>,
    pub resource_entries: Vec<u8>,
    pub omit_procset: i32,
    pub space_font_name: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfForm {
    pub object: u32,
    pub resource: u32,
    pub artifact_bytes: Vec<u8>,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub entries: Vec<u8>,
    pub resource_entries: Vec<u8>,
    pub immediate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfFontResource {
    pub recipe: FontArtifactRecipe,
    pub resource_number: u32,
    pub object_number: u32,
    pub widths: Vec<Scaled>,
    pub heights: Vec<Scaled>,
    pub depths: Vec<Scaled>,
    pub x_height: Scaled,
    pub descriptor_entries: Vec<u8>,
    pub included_codes: Vec<u8>,
    pub disable_builtin_to_unicode: bool,
}

/// Ordered font-output mutations with every live `FontId` rewritten to its
/// stable artifact recipe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DetachedPdfFontOperation {
    Map(PdfFontMapOperation),
    MapFileContent {
        logical_name: Vec<u8>,
        map: tex_fonts::PdfFontMap,
    },
    Attribute {
        font: FontArtifactRecipe,
        bytes: Vec<u8>,
    },
    IncludeChars {
        font: FontArtifactRecipe,
        chars: Vec<u8>,
    },
    GlyphToUnicode(PdfGlyphToUnicode),
    NoBuiltinToUnicode {
        font: FontArtifactRecipe,
    },
    Type1Program {
        logical_name: Vec<u8>,
        program: tex_fonts::PdfType1Program,
    },
    Encoding {
        logical_name: Vec<u8>,
        encoding: tex_fonts::PdfEncoding,
    },
    TrueTypeProgram {
        logical_name: Vec<u8>,
        program: tex_fonts::PdfTrueTypeProgram,
    },
    PkFont {
        request: tex_fonts::PdfPkFontRequest,
        font: tex_fonts::PdfPkFont,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DetachedPdfRawObjectPayload {
    Value(Vec<u8>),
    Stream {
        entries: Vec<u8>,
        data: Vec<u8>,
    },
    FileStream {
        entries: Vec<u8>,
        source_name: Vec<u8>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfRawObject {
    pub object: u32,
    pub payload: Option<DetachedPdfRawObjectPayload>,
    pub immediate: bool,
    pub referenced: bool,
}

/// Host resource request for a `stream file` raw object.  The source spelling
/// is detached here; file acquisition remains an outer capability operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfRawObjectFileNeed {
    pub object: u32,
    pub source_name: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DetachedPdfActionIdentifier {
    Name(Vec<u8>),
    Number(u32),
    Raw(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DetachedPdfActionTarget {
    Page { number: u32, view: Vec<u8> },
    Destination(DetachedPdfActionIdentifier),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfActionDestination {
    pub file: Option<Vec<u8>>,
    pub structure: Option<DetachedPdfActionIdentifier>,
    pub target: DetachedPdfActionTarget,
    pub window: PdfActionWindow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DetachedPdfAction {
    User(Vec<u8>),
    GoTo(DetachedPdfActionDestination),
    Thread(DetachedPdfActionDestination),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfActionRecord {
    pub id: u32,
    pub action: DetachedPdfAction,
    pub target_object: Option<u32>,
    pub structure_object: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfAnnotation {
    pub object: u32,
    pub dimensions: Option<PdfAnnotationDimensions>,
    pub entries: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfLink {
    pub object: u32,
    pub dimensions: PdfAnnotationDimensions,
    pub entries: Vec<u8>,
    pub action: DetachedPdfAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfOutline {
    pub action_object: u32,
    pub item_object: u32,
    pub title_object: u32,
    pub entries: Vec<u8>,
    pub action: DetachedPdfAction,
    pub count: i32,
    pub title: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DetachedPdfDocumentFragments {
    pub info: Vec<u8>,
    pub catalog: Vec<u8>,
    pub names: Vec<u8>,
    pub trailer: Vec<u8>,
    pub trailer_id: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfDocumentState {
    pub pages_entries: Vec<u8>,
    pub include_info_dictionary: bool,
    pub include_dates: bool,
    pub suppress_ptex_info: i32,
    pub ptex_use_underscore: bool,
    pub form_omit_procset: i32,
    pub suppress_page_group_warning: bool,
    pub clock: JobClock,
    pub fragments: DetachedPdfDocumentFragments,
    pub objects: PdfDocumentObjectIds,
    pub open_action: Option<DetachedPdfActionRecord>,
}

/// Complete cold PDF state selected at the terminal engine boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedPdfCompletion {
    enabled: bool,
    output_parameters: Option<PdfOutputParameters>,
    font_configuration: PdfFontConfiguration,
    pages: Vec<DetachedPdfPage>,
    forms: Vec<DetachedPdfForm>,
    fonts: Vec<DetachedPdfFontResource>,
    font_operations: Vec<DetachedPdfFontOperation>,
    images: Vec<PdfExternalImageRecord>,
    raw_objects: Vec<DetachedPdfRawObject>,
    raw_object_file_needs: Vec<DetachedPdfRawObjectFileNeed>,
    document: DetachedPdfDocumentState,
    annotations: Vec<DetachedPdfAnnotation>,
    links: Vec<DetachedPdfLink>,
    destinations: Vec<PdfDestinationRecord>,
    structure_destinations: Vec<PdfDestinationRecord>,
    outlines: Vec<DetachedPdfOutline>,
    threads: Vec<PdfThreadRecord>,
    next_object: u32,
}

impl DetachedPdfCompletion {
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
    #[must_use]
    pub const fn output_parameters(&self) -> Option<PdfOutputParameters> {
        self.output_parameters
    }
    #[must_use]
    pub const fn font_configuration(&self) -> PdfFontConfiguration {
        self.font_configuration
    }
    #[must_use]
    pub fn pages(&self) -> &[DetachedPdfPage] {
        &self.pages
    }
    #[must_use]
    pub fn forms(&self) -> &[DetachedPdfForm] {
        &self.forms
    }
    #[must_use]
    pub fn fonts(&self) -> &[DetachedPdfFontResource] {
        &self.fonts
    }
    #[must_use]
    pub fn font_operations(&self) -> &[DetachedPdfFontOperation] {
        &self.font_operations
    }
    #[must_use]
    pub fn images(&self) -> &[PdfExternalImageRecord] {
        &self.images
    }
    #[must_use]
    pub fn raw_objects(&self) -> &[DetachedPdfRawObject] {
        &self.raw_objects
    }
    #[must_use]
    pub fn raw_object_file_needs(&self) -> &[DetachedPdfRawObjectFileNeed] {
        &self.raw_object_file_needs
    }
    #[must_use]
    pub const fn document(&self) -> &DetachedPdfDocumentState {
        &self.document
    }
    #[must_use]
    pub fn annotations(&self) -> &[DetachedPdfAnnotation] {
        &self.annotations
    }
    #[must_use]
    pub fn links(&self) -> &[DetachedPdfLink] {
        &self.links
    }
    #[must_use]
    pub fn destinations(&self) -> &[PdfDestinationRecord] {
        &self.destinations
    }
    #[must_use]
    pub fn structure_destinations(&self) -> &[PdfDestinationRecord] {
        &self.structure_destinations
    }
    #[must_use]
    pub fn outlines(&self) -> &[DetachedPdfOutline] {
        &self.outlines
    }
    #[must_use]
    pub fn threads(&self) -> &[PdfThreadRecord] {
        &self.threads
    }
    #[must_use]
    pub const fn next_object(&self) -> u32 {
        self.next_object
    }

    /// Retargets every PDF page row naming one prepared artifact. This is
    /// used only by the outer completion transaction after it has rewritten
    /// the corresponding page bytes for an unavailable `\openout` target.
    #[doc(hidden)]
    pub fn retarget_page_artifact(
        &mut self,
        old: ContentHash,
        new: ContentHash,
        bytes: &[u8],
    ) -> usize {
        let mut changed = 0;
        for page in &mut self.pages {
            if page.artifact == old {
                page.artifact = new;
                bytes.clone_into(&mut page.artifact_bytes);
                changed += 1;
            }
        }
        changed
    }
}

pub(crate) struct PdfCompletionScalars {
    pub font_configuration: PdfFontConfiguration,
    pub pages_entries: Vec<u8>,
    pub include_info_dictionary: bool,
    pub include_dates: bool,
    pub suppress_ptex_info: i32,
    pub ptex_use_underscore: bool,
    pub form_omit_procset: i32,
    pub suppress_page_group_warning: bool,
    pub clock: JobClock,
}

pub(crate) fn detach<G>(
    pdf: &PdfState<G>,
    scalars: PdfCompletionScalars,
    mut tokens: impl FnMut(crate::TokenListId<G>) -> Result<Vec<u8>, PdfCompletionError>,
    mut font_recipe: impl FnMut(crate::ids::FontId) -> FontArtifactRecipe,
    mut font_metrics: impl FnMut(crate::ids::FontId, u8) -> Option<crate::font::CharMetrics>,
    mut font_parameter: impl FnMut(crate::ids::FontId, u32) -> Scaled,
    mut read_artifact: impl FnMut(ContentHash) -> Result<Option<Vec<u8>>, String>,
) -> Result<DetachedPdfCompletion, PdfCompletionError> {
    let pages = pdf
        .pages
        .iter()
        .map(|page| {
            let artifact_bytes = read_artifact(page.artifact)
                .map_err(|message| PdfCompletionError::ArtifactRead {
                    hash: page.artifact,
                    message,
                })?
                .ok_or(PdfCompletionError::MissingPageArtifact(page.artifact))?;
            Ok(DetachedPdfPage {
                artifact: page.artifact,
                artifact_bytes,
                resources_object: page.resources_object,
                contents_object: page.contents_object,
                page_object: page.page_object,
                h_origin: page.parameters.h_origin,
                v_origin: page.parameters.v_origin,
                width: page.parameters.width,
                height: page.parameters.height,
                link_margin: page.parameters.link_margin,
                page_entries: tokens(page.parameters.page_attr.id())?,
                resource_entries: tokens(page.parameters.resources.id())?,
                omit_procset: page.parameters.omit_procset,
                space_font_name: pdf
                    .space_font_names
                    .get(page.parameters.space_font_name as usize)
                    .cloned()
                    .ok_or(PdfCompletionError::MissingSpaceFontName(
                        page.parameters.space_font_name,
                    ))?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let forms = pdf
        .forms
        .iter()
        .filter_map(|form| {
            // Lazy forms are only output resources after an immediate form
            // publication or a shipped `\pdfrefxform` has staged their
            // artifact. Merely creating or enquiring about an unreferenced
            // form must not force an incomplete resource into the terminal
            // PDF projection.
            pdf.form_artifact(form.object).map(|artifact| {
                Ok(DetachedPdfForm {
                    object: form.object,
                    resource: form.resource,
                    artifact_bytes: artifact.bytes,
                    width: form.width,
                    height: form.height,
                    depth: form.depth,
                    entries: form
                        .attr
                        .clone()
                        .map(|value| tokens(value.id()))
                        .transpose()?
                        .unwrap_or_default(),
                    resource_entries: form
                        .resources
                        .clone()
                        .map(|value| tokens(value.id()))
                        .transpose()?
                        .unwrap_or_default(),
                    immediate: form.immediate,
                })
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let fonts = pdf
        .font_resource_records()
        .map(|resource| {
            let metrics: Vec<_> = (0..=u8::MAX)
                .map(|code| font_metrics(resource.font, code))
                .collect();
            DetachedPdfFontResource {
                recipe: font_recipe(resource.font),
                resource_number: resource.resource_number,
                object_number: resource.object_number,
                widths: metrics
                    .iter()
                    .map(|metric| metric.map_or(Scaled::from_raw(0), |metric| metric.width))
                    .collect(),
                heights: metrics
                    .iter()
                    .map(|metric| metric.map_or(Scaled::from_raw(0), |metric| metric.height))
                    .collect(),
                depths: metrics
                    .iter()
                    .map(|metric| metric.map_or(Scaled::from_raw(0), |metric| metric.depth))
                    .collect(),
                x_height: font_parameter(resource.font, 5),
                descriptor_entries: pdf.font_attribute(resource.font).to_vec(),
                included_codes: pdf.included_font_chars(resource.font),
                disable_builtin_to_unicode: pdf.builtin_to_unicode_disabled(resource.font),
            }
        })
        .collect();

    let font_operations = pdf
        .font_operations
        .iter()
        .map(|operation| match operation {
            PdfFontOperation::Map(value) => DetachedPdfFontOperation::Map(value.clone()),
            PdfFontOperation::Attribute { font, bytes } => DetachedPdfFontOperation::Attribute {
                font: font_recipe(*font),
                bytes: bytes.clone(),
            },
            PdfFontOperation::IncludeChars { font, chars } => {
                DetachedPdfFontOperation::IncludeChars {
                    font: font_recipe(*font),
                    chars: chars.clone(),
                }
            }
            PdfFontOperation::GlyphToUnicode(value) => {
                DetachedPdfFontOperation::GlyphToUnicode(value.clone())
            }
            PdfFontOperation::NoBuiltinToUnicode { font } => {
                DetachedPdfFontOperation::NoBuiltinToUnicode {
                    font: font_recipe(*font),
                }
            }
        })
        .collect();

    let mut raw_object_file_needs = Vec::new();
    let raw_objects = pdf
        .raw_objects()
        .map(|record| {
            let payload = record
                .data()
                .map(|data| -> Result<_, PdfCompletionError> {
                    let data_bytes = tokens(data.data())?;
                    if data.is_file() {
                        raw_object_file_needs.push(DetachedPdfRawObjectFileNeed {
                            object: record.id().raw(),
                            source_name: data_bytes.clone(),
                        });
                        Ok(Some(DetachedPdfRawObjectPayload::FileStream {
                            entries: data
                                .stream_attr()
                                .map(&mut tokens)
                                .transpose()?
                                .unwrap_or_default(),
                            source_name: data_bytes,
                        }))
                    } else if data.is_stream() {
                        Ok(Some(DetachedPdfRawObjectPayload::Stream {
                            entries: data
                                .stream_attr()
                                .map(&mut tokens)
                                .transpose()?
                                .unwrap_or_default(),
                            data: data_bytes,
                        }))
                    } else {
                        Ok(Some(DetachedPdfRawObjectPayload::Value(data_bytes)))
                    }
                })
                .transpose()?
                .flatten();
            Ok(DetachedPdfRawObject {
                object: record.id().raw(),
                payload,
                immediate: record.is_immediate(),
                referenced: record.is_referenced(),
            })
        })
        .collect::<Result<Vec<_>, PdfCompletionError>>()?;

    let fragments = DetachedPdfDocumentFragments {
        info: fragment_bytes(pdf, PdfDocumentFragmentKind::Info, &mut tokens)?,
        catalog: fragment_bytes(pdf, PdfDocumentFragmentKind::Catalog, &mut tokens)?,
        names: fragment_bytes(pdf, PdfDocumentFragmentKind::Names, &mut tokens)?,
        trailer: fragment_bytes(pdf, PdfDocumentFragmentKind::Trailer, &mut tokens)?,
        trailer_id: fragment_bytes(pdf, PdfDocumentFragmentKind::TrailerId, &mut tokens)?,
    };
    let open_action = pdf
        .catalog_open_action()
        .clone()
        .map(|record| detach_action_record(record, &mut tokens))
        .transpose()?;
    let annotations = pdf
        .annotations()
        .into_iter()
        .map(|record| {
            let data = record.data();
            Ok(DetachedPdfAnnotation {
                object: record.object(),
                dimensions: data.as_ref().map(|value| value.dimensions),
                entries: data.map(|value| tokens(value.entries)).transpose()?,
            })
        })
        .collect::<Result<Vec<_>, PdfCompletionError>>()?;
    let links = pdf
        .links
        .iter()
        .map(|record| {
            Ok(DetachedPdfLink {
                object: record.object(),
                dimensions: record.dimensions(),
                entries: tokens(record.attributes())?,
                action: detach_action(record.action(), &mut tokens)?,
            })
        })
        .collect::<Result<Vec<_>, PdfCompletionError>>()?;
    let outlines = pdf
        .outlines
        .iter()
        .map(|record| {
            Ok(DetachedPdfOutline {
                action_object: record.action_object(),
                item_object: record.item_object(),
                title_object: record.title_object(),
                entries: tokens(record.attributes())?,
                action: detach_action(record.action(), &mut tokens)?,
                count: record.count(),
                title: tokens(record.title())?,
            })
        })
        .collect::<Result<Vec<_>, PdfCompletionError>>()?;

    Ok(DetachedPdfCompletion {
        enabled: pdf.enabled,
        output_parameters: pdf.output_parameters.map(PdfOutputParameters::normalized),
        font_configuration: scalars.font_configuration,
        pages,
        forms,
        fonts,
        font_operations,
        images: pdf
            .external_images
            .iter()
            .map(|entry| pdf.materialize_external_image(entry))
            .collect(),
        raw_objects,
        raw_object_file_needs,
        document: DetachedPdfDocumentState {
            pages_entries: scalars.pages_entries,
            include_info_dictionary: scalars.include_info_dictionary,
            include_dates: scalars.include_dates,
            suppress_ptex_info: scalars.suppress_ptex_info,
            ptex_use_underscore: scalars.ptex_use_underscore,
            form_omit_procset: scalars.form_omit_procset,
            suppress_page_group_warning: scalars.suppress_page_group_warning,
            clock: scalars.clock,
            fragments,
            objects: pdf.document_objects,
            open_action,
        },
        annotations,
        links,
        destinations: pdf.destination_records(false),
        structure_destinations: pdf.destination_records(true),
        outlines,
        threads: pdf.thread_records(),
        next_object: pdf.next_object,
    })
}

fn fragment_bytes<G>(
    pdf: &PdfState<G>,
    kind: PdfDocumentFragmentKind,
    tokens: &mut impl FnMut(crate::TokenListId<G>) -> Result<Vec<u8>, PdfCompletionError>,
) -> Result<Vec<u8>, PdfCompletionError> {
    let mut result = Vec::new();
    for value in pdf.document_fragments(kind) {
        result.extend(tokens(value)?);
    }
    Ok(result)
}

fn detach_action_record<G>(
    record: PdfActionRecord<G>,
    tokens: &mut impl FnMut(crate::TokenListId<G>) -> Result<Vec<u8>, PdfCompletionError>,
) -> Result<DetachedPdfActionRecord, PdfCompletionError> {
    Ok(DetachedPdfActionRecord {
        id: record.id(),
        action: detach_action(record.spec(), tokens)?,
        target_object: record.target_object(),
        structure_object: record.structure_object(),
    })
}

fn detach_action<G>(
    action: PdfActionSpec<G>,
    tokens: &mut impl FnMut(crate::TokenListId<G>) -> Result<Vec<u8>, PdfCompletionError>,
) -> Result<DetachedPdfAction, PdfCompletionError> {
    Ok(match action {
        PdfActionSpec::User(value) => DetachedPdfAction::User(tokens(value)?),
        PdfActionSpec::GoTo(value) => DetachedPdfAction::GoTo(detach_destination(value, tokens)?),
        PdfActionSpec::Thread(value) => {
            DetachedPdfAction::Thread(detach_destination(value, tokens)?)
        }
    })
}

fn detach_destination<G>(
    destination: PdfActionDestination<G>,
    tokens: &mut impl FnMut(crate::TokenListId<G>) -> Result<Vec<u8>, PdfCompletionError>,
) -> Result<DetachedPdfActionDestination, PdfCompletionError> {
    Ok(DetachedPdfActionDestination {
        file: destination.file.map(&mut *tokens).transpose()?,
        structure: destination
            .structure
            .map(|value| detach_identifier(value, tokens))
            .transpose()?,
        target: match destination.target {
            PdfActionTarget::Page { number, view } => DetachedPdfActionTarget::Page {
                number,
                view: tokens(view)?,
            },
            PdfActionTarget::Destination(value) => {
                DetachedPdfActionTarget::Destination(detach_identifier(value, tokens)?)
            }
        },
        window: destination.window,
    })
}

fn detach_identifier<G>(
    identifier: PdfActionIdentifier<G>,
    tokens: &mut impl FnMut(crate::TokenListId<G>) -> Result<Vec<u8>, PdfCompletionError>,
) -> Result<DetachedPdfActionIdentifier, PdfCompletionError> {
    Ok(match identifier {
        PdfActionIdentifier::Name(value) => DetachedPdfActionIdentifier::Name(tokens(value)?),
        PdfActionIdentifier::Number(value) => DetachedPdfActionIdentifier::Number(value),
        PdfActionIdentifier::Raw(value) => DetachedPdfActionIdentifier::Raw(tokens(value)?),
    })
}
