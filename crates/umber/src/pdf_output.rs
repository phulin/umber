//! Engine/host adapter for detached `tex-out` PDF finalization.

mod finalization_input;

pub use finalization_input::{
    pdf_finalization_input, pdf_finalization_input_with_raw_object_files,
};

use tex_out::pdf::{
    PdfModelError, PdfObjectCompression, PdfSerializationOptions, PdfSerializeError,
    PdfStreamCompression, PdfVersion,
};
use tex_out::positioned::PositionedError;
use tex_state::{ContentHash, DetachedPdfCompletion, PdfOutputParameters, WorldError};

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

pub fn pdf_from_accepted_artifacts_with_virtual_fonts(
    pdf: &DetachedPdfCompletion,
    virtual_fonts: &crate::PdfVirtualFontResources,
    raw_object_files: &crate::PdfRawObjectFileReceipt,
) -> Result<Vec<u8>, PdfBuildError> {
    pdf_from_completion_at_dpi(
        pdf,
        DEFAULT_PDF_PK_RESOLUTION,
        virtual_fonts,
        raw_object_files,
    )
}

pub fn pdf_from_completion_at_dpi(
    pdf: &DetachedPdfCompletion,
    driver_dpi: i32,
    virtual_fonts: &crate::PdfVirtualFontResources,
    raw_object_files: &crate::PdfRawObjectFileReceipt,
) -> Result<Vec<u8>, PdfBuildError> {
    let input = pdf_finalization_input_with_raw_object_files(
        pdf,
        driver_dpi,
        virtual_fonts,
        raw_object_files,
    )?;
    let output = tex_out::pdf::finalize_pdf(&input).map_err(map_finalization_error)?;
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

pub(super) fn pdf_date(clock: tex_state::JobClock) -> Vec<u8> {
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

pub(super) fn pdf_version(parameters: PdfOutputParameters) -> Result<PdfVersion, PdfBuildError> {
    let major = u8::try_from(parameters.major_version)
        .map_err(|_| PdfBuildError::InvalidVersionParameters)?;
    let minor = u8::try_from(parameters.minor_version)
        .map_err(|_| PdfBuildError::InvalidVersionParameters)?;
    Ok(PdfVersion::new(major, minor)?)
}

pub(super) fn serialization_options(
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
