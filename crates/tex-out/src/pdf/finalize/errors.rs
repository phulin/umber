//! Public PDF finalization error and source conversions.

use super::*;

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
    ConflictingEncoding(Vec<u8>),
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
            Self::ConflictingEncoding(name) => write!(
                f,
                "PDF encoding resource {:?} resolved to conflicting vectors",
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
