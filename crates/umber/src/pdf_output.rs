//! Engine/host adapter for detached `tex-out` PDF finalization.

mod finalization_input;

pub use finalization_input::{
    pdf_finalization_input, pdf_finalization_input_with_raw_object_files,
};
use finalization_input::{
    pdf_finalization_input_with_page_records, reserve_virtual_font_resources,
};

use tex_out::pdf::{
    PdfModelError, PdfObjectCompression, PdfSerializationOptions, PdfSerializeError,
    PdfStreamCompression, PdfVersion,
};
use tex_out::positioned::{PositionedError, PositionedPage};
use tex_state::TokenListId;
use tex_state::env::banks::{IntParam, TokParam};
use tex_state::ids::FontId;
use tex_state::token_show::append_token_string_text;
use tex_state::{
    CommittedArtifact, ContentHash, PdfDocumentFragmentKind, PdfOutputParameters, Universe,
    WorldError,
};

pub(crate) const DEFAULT_PDF_PK_RESOLUTION: i32 = 600;

pub(crate) fn is_pdf_sfnt_program(name: &[u8]) -> bool {
    name.rsplit(|byte| *byte == b'.')
        .next()
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case(b"ttf")
                || extension.eq_ignore_ascii_case(b"otf")
                || extension.eq_ignore_ascii_case(b"woff2")
        })
}

pub(crate) fn pk_font_request<G>(
    stores: &Universe<G>,
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

pub fn pdf_from_committed_artifacts<G>(
    stores: &mut Universe<G>,
    artifacts: &[CommittedArtifact],
) -> Result<Vec<u8>, PdfBuildError> {
    pdf_from_committed_artifacts_with_virtual_fonts(
        stores,
        artifacts,
        &crate::PdfVirtualFontResources::default(),
    )
}

pub fn pdf_from_committed_artifacts_with_virtual_fonts<G>(
    stores: &mut Universe<G>,
    artifacts: &[CommittedArtifact],
    virtual_fonts: &crate::PdfVirtualFontResources,
) -> Result<Vec<u8>, PdfBuildError> {
    let page_records = stores.pdf_pages().to_vec();
    pdf_from_artifacts_and_page_records_at_dpi_with_virtual_fonts(
        stores,
        artifacts,
        &page_records,
        DEFAULT_PDF_PK_RESOLUTION,
        virtual_fonts,
        &crate::PdfRawObjectFileReceipt::default(),
    )
}

/// Finalizes an accepted native run while its fallible page suffix remains
/// unpublished.
///
/// Prepared pages must remain outside the live universe until their effects
/// commit, but they are already part of the accepted document. This adapter
/// presents the ordered live prefix and prepared suffix as one PDF page ledger
/// without publishing either effects or artifacts early.
pub fn pdf_from_accepted_artifacts_with_virtual_fonts<G>(
    stores: &mut Universe<G>,
    artifacts: &[CommittedArtifact],
    prepared_pages: Option<&tex_state::PreparedPageSuffix>,
    virtual_fonts: &crate::PdfVirtualFontResources,
    raw_object_files: &crate::PdfRawObjectFileReceipt,
) -> Result<Vec<u8>, PdfBuildError> {
    let mut page_records = stores.pdf_pages().to_vec();
    if let Some(prepared_pages) = prepared_pages {
        page_records.extend_from_slice(prepared_pages.pdf_pages());
    }
    pdf_from_artifacts_and_page_records_at_dpi_with_virtual_fonts(
        stores,
        artifacts,
        &page_records,
        DEFAULT_PDF_PK_RESOLUTION,
        virtual_fonts,
        raw_object_files,
    )
}

pub fn pdf_from_committed_artifacts_at_dpi<G>(
    stores: &mut Universe<G>,
    artifacts: &[CommittedArtifact],
    driver_dpi: i32,
) -> Result<Vec<u8>, PdfBuildError> {
    let page_records = stores.pdf_pages().to_vec();
    pdf_from_artifacts_and_page_records_at_dpi_with_virtual_fonts(
        stores,
        artifacts,
        &page_records,
        driver_dpi,
        &crate::PdfVirtualFontResources::default(),
        &crate::PdfRawObjectFileReceipt::default(),
    )
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
fn pdf_from_artifacts_and_page_records_at_dpi_with_virtual_fonts<G>(
    stores: &mut Universe<G>,
    artifacts: &[CommittedArtifact],
    page_records: &[tex_state::PdfPageRecord<G>],
    driver_dpi: i32,
    virtual_fonts: &crate::PdfVirtualFontResources,
    raw_object_files: &crate::PdfRawObjectFileReceipt,
) -> Result<Vec<u8>, PdfBuildError> {
    let input = pdf_finalization_input_with_page_records(
        stores,
        artifacts,
        page_records,
        driver_dpi,
        virtual_fonts,
        raw_object_files,
    )?;
    let include_info = input.document.metadata.include_info_dictionary;
    let output = tex_out::pdf::finalize_pdf(&input).map_err(map_finalization_error)?;

    // Replay only the allocation receipt proven by detached finalization. No
    // output lowering or serialization is repeated against live engine state.
    reserve_virtual_font_resources(stores, artifacts, page_records, virtual_fonts)?;
    stores
        .finalize_pdf_document_objects(include_info)
        .map_err(|_| PdfBuildError::ObjectCapacity)?;
    for diagnostic in output.diagnostics {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            &format!("{diagnostic}\n"),
        );
    }
    Ok(output.bytes)
}

fn map_finalization_error(error: tex_out::pdf::PdfBuildError) -> PdfBuildError {
    match error {
        tex_out::pdf::PdfBuildError::FormCycle(object) => PdfBuildError::FormCycle(object),
        tex_out::pdf::PdfBuildError::RecursiveForm(object) => PdfBuildError::RecursiveForm(object),
        tex_out::pdf::PdfBuildError::ReferencedFormNotFound(object) => {
            PdfBuildError::ReferencedFormNotFound(object)
        }
        tex_out::pdf::PdfBuildError::MissingFormArtifact(object) => {
            PdfBuildError::MissingFormArtifact(object)
        }
        tex_out::pdf::PdfBuildError::FormTraversalDepthExceeded(limit) => {
            PdfBuildError::FormTraversalDepthExceeded(limit)
        }
        tex_out::pdf::PdfBuildError::FormTraversalWorkExceeded(limit) => {
            PdfBuildError::FormTraversalWorkExceeded(limit)
        }
        tex_out::pdf::PdfBuildError::InvalidPng => PdfBuildError::InvalidPng,
        tex_out::pdf::PdfBuildError::MissingRasterImage(object) => {
            PdfBuildError::MissingRasterImage(object)
        }
        tex_out::pdf::PdfBuildError::OpenActionPageNotFound(page) => {
            PdfBuildError::OpenActionPageNotFound(page)
        }
        tex_out::pdf::PdfBuildError::OutlineCountIncomplete { object, missing } => {
            PdfBuildError::OutlineCountIncomplete { object, missing }
        }
        tex_out::pdf::PdfBuildError::ReferencedRawObjectUninitialized(object) => {
            PdfBuildError::ReferencedRawObjectUninitialized(object)
        }
        other => PdfBuildError::DetachedFinalization(other),
    }
}

fn positioned_pages<G>(
    stores: &Universe<G>,
    artifacts: &[CommittedArtifact],
    records: &[tex_state::PdfPageRecord<G>],
) -> Result<Vec<PositionedPage>, PdfBuildError> {
    records
        .iter()
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

fn positioned_forms<G>(stores: &Universe<G>) -> Result<Vec<(u32, PositionedPage)>, PdfBuildError> {
    stores
        .pdf_forms()
        .filter_map(|form| {
            stores.pdf_form_artifact(form.object()).map(|staged| {
                let artifact = tex_out::PageArtifact::from_bytes(staged.bytes())?;
                Ok((
                    form.object(),
                    tex_out::positioned::lower_page(&artifact, 0)?,
                ))
            })
        })
        .collect()
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

fn artifact_bytes<G>(
    stores: &Universe<G>,
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

fn output_parameters<G>(stores: &Universe<G>) -> PdfOutputParameters {
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
    let stream_compression = match parameters.compress_level {
        ..=0 => PdfStreamCompression::None,
        level @ 1..=9 => PdfStreamCompression::Flate { level: level as u8 },
        level => return Err(PdfBuildError::InvalidCompressionLevel(level)),
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

pub(crate) fn token_list_bytes<G>(stores: &Universe<G>, id: TokenListId<G>) -> Vec<u8> {
    let mut text = String::new();
    for &token in stores.tokens(id).iter() {
        append_token_string_text(stores, token, &mut text);
    }
    text.into_bytes()
}

fn document_fragment_bytes<G>(stores: &Universe<G>, kind: PdfDocumentFragmentKind) -> Vec<u8> {
    let mut bytes = Vec::new();
    for tokens in stores.pdf_document_fragments(kind) {
        bytes.extend_from_slice(&token_list_bytes(stores, tokens));
    }
    bytes
}

#[derive(Debug)]
pub enum PdfBuildError {
    PdfOutputDisabled,
    MissingArtifact(ContentHash),
    InvalidVersionParameters,
    InvalidCompressionLevel(i32),
    InvalidObjectCompressionLevel(i32),
    ObjectCapacity,
    OpenActionPageNotFound(u32),
    OutlineCountIncomplete { object: u32, missing: usize },
    ReferencedRawObjectUninitialized(u32),
    ReferencedFormNotFound(u32),
    MissingFormArtifact(u32),
    RecursiveForm(u32),
    FormCycle(u32),
    FormTraversalDepthExceeded(usize),
    FormTraversalWorkExceeded(usize),
    MissingRawObjectFilePayload(u32),
    RawObjectFilePayloadMismatch(u32),
    MissingPositionedFont(u32),
    MissingFontProgram(Vec<u8>),
    MissingFontResource(String),
    PkFont(String),
    MissingPkFont(tex_fonts::PdfPkFontRequest),
    MissingEncoding(Vec<u8>),
    MissingSpaceFontName(u32),
    MissingLiveFont(String),
    UnsupportedMappedVirtualFont(String),
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
    MissingRasterImage(u32),
    InvalidPng,
    World(WorldError),
    Parse(tex_out::ParseError),
    Positioned(PositionedError),
    Model(PdfModelError),
    Serialize(PdfSerializeError),
    DetachedFinalization(tex_out::pdf::PdfBuildError),
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
            Self::ObjectCapacity => f.write_str("pdfTeX error (obj): too many PDF objects."),
            Self::OpenActionPageNotFound(page) => {
                write!(f, "PDF open action references missing page {page}")
            }
            Self::OutlineCountIncomplete { object, missing } => write!(
                f,
                "PDF outline item {object} is missing {missing} declared child entries"
            ),
            Self::ReferencedRawObjectUninitialized(id) => write!(
                f,
                "referenced PDF object {id} was reserved but never initialized"
            ),
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
            Self::MissingRawObjectFilePayload(id) => {
                write!(f, "PDF stream object {id} has no accepted file payload")
            }
            Self::RawObjectFilePayloadMismatch(id) => write!(
                f,
                "PDF stream object {id} file payload does not match its accepted identity"
            ),
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
            Self::PkFont(message) => f.write_str(message),
            Self::MissingPkFont(request) => write!(
                f,
                "PK font resource {:?} at {} DPI in mode {:?} was not supplied",
                String::from_utf8_lossy(request.tex_name()),
                request.dpi(),
                String::from_utf8_lossy(request.mode())
            ),
            Self::MissingEncoding(name) => write!(
                f,
                "PDF encoding resource {:?} was not supplied",
                String::from_utf8_lossy(name)
            ),
            Self::MissingSpaceFontName(id) => {
                write!(f, "PDF page references missing space-font name id {id}")
            }
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
            Self::VirtualFontCharacterOutOfRange { font, code } => write!(
                f,
                "virtual font {font} references character {code} outside 0..=255"
            ),
            Self::MissingVirtualCharacter { font, code } => {
                write!(f, "virtual-font local font {font} has no character {code}")
            }
            Self::VirtualFontArithmeticOverflow => {
                f.write_str("virtual-font positioned arithmetic overflowed")
            }
            Self::MissingRasterImage(object) => write!(f, "PDF image object {object} is missing"),
            Self::InvalidPng => f.write_str("registered PNG image data is invalid"),
            Self::World(error) => error.fmt(f),
            Self::Parse(error) => error.fmt(f),
            Self::Positioned(error) => error.fmt(f),
            Self::Model(error) => error.fmt(f),
            Self::Serialize(error) => error.fmt(f),
            Self::DetachedFinalization(error) => error.fmt(f),
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

#[cfg(test)]
mod tests;
