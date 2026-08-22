//! Checkpointed pdfTeX document allocation ledger.

mod action;
mod annotation;
pub(crate) mod completion;
mod destination;
mod document;
mod object;
mod outline;
mod thread;

pub use action::{
    PdfActionDestination, PdfActionIdentifier, PdfActionRecord, PdfActionSpec, PdfActionTarget,
    PdfActionWindow,
};
pub use annotation::{
    PdfAnnotationData, PdfAnnotationDimensions, PdfAnnotationInitializeError, PdfAnnotationRecord,
    PdfLinkRecord, PdfOpenLink,
};
pub use completion::{
    DetachedPdfAction, DetachedPdfActionDestination, DetachedPdfActionIdentifier,
    DetachedPdfActionRecord, DetachedPdfActionTarget, DetachedPdfAnnotation, DetachedPdfCompletion,
    DetachedPdfDocumentFragments, DetachedPdfDocumentState, DetachedPdfFontOperation,
    DetachedPdfFontResource, DetachedPdfForm, DetachedPdfLink, DetachedPdfOutline, DetachedPdfPage,
    DetachedPdfRawObject, DetachedPdfRawObjectFileNeed, DetachedPdfRawObjectPayload,
    PdfCompletionError,
};
pub use destination::{PdfDestinationDefinition, PdfDestinationIdentity, PdfDestinationRecord};
use document::PdfDocumentFragments;
pub use document::{PdfDocumentFragmentKind, PdfDocumentObjectIds};
use object::PdfRawObjects;
pub use object::{
    PdfRawObjectData, PdfRawObjectId, PdfRawObjectInitializeError, PdfRawObjectRecord,
};
pub use outline::PdfOutlineRecord;
pub use thread::{PdfThreadBeadRecord, PdfThreadRecord};

/// Handle-free unresolved PDF navigation selected before terminal rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfNavigationWarning {
    Destination(PdfDestinationIdentity),
    StructureDestination(PdfDestinationIdentity),
    Thread(PdfDestinationIdentity),
}

use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::ContentHash;
use crate::durable_arena::TokenListId;
use crate::ids::FontId;
use crate::node_arena::DurableListId;
use crate::scaled::Scaled;
use crate::state_hash::{StateHashFragment, StateHasher};
use std::collections::BTreeMap;

const PDF_STATE_DOMAIN: u64 = 0x7064_665f_7374_6174;
const PDF_PAGE_DOMAIN: u64 = 0x7064_665f_7061_6765;
const PDF_FONT_DOMAIN: u64 = 0x7064_665f_666f_6e74;
const PDF_EXTERNAL_IMAGE_DOMAIN: u64 = 0x7064_665f_7869_6d67;
const PDF_COLOR_STACK_DOMAIN: u64 = 0x7064_665f_636f_6c72;
const PDF_FORM_DOMAIN: u64 = 0x7064_665f_666f_726d;
const FIRST_DYNAMIC_OBJECT: u32 = 1;
const OBJECTS_PER_PAGE: u32 = 3;
const MAX_OBJECT_ID: u32 = i32::MAX as u32;
const MAX_COLOR_STACKS: usize = 32_768;

/// How color-stack bytes are framed in a page content stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfColorStackMode {
    Origin,
    Page,
    Direct,
}

/// Selects pdfTeX's deliberately independent page and form color-stack state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfColorStackTarget {
    Page,
    Form,
}

/// A color-stack mutation retained on the whatsit until final traversal.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum PdfColorStackAction {
    Set(Vec<u8>),
    Push(Vec<u8>),
    Pop,
    Current,
}

/// Bytes emitted by a successful color-stack action or page restoration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfColorStackEmission {
    pub mode: PdfColorStackMode,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PdfColorStackRuntime {
    current: Vec<u8>,
    pushed: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct PdfFormColorRollback(Vec<PdfColorStackRuntime>, StateHashFragment);

#[derive(Clone, Debug, Eq, PartialEq)]
struct PdfColorStack {
    mode: PdfColorStackMode,
    restore_at_page_start: bool,
    page: PdfColorStackRuntime,
    form: PdfColorStackRuntime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfColorStackCapacityError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfColorStackApplyError {
    Unknown,
    Underflow,
}

/// Typed identity assigned to an external-image object by pdfTeX's object table.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfExternalImageId(u32);

impl PdfExternalImageId {
    pub fn new(raw: u32) -> Result<Self, PdfExternalImageIdError> {
        (raw > 0 && raw <= MAX_OBJECT_ID)
            .then_some(Self(raw))
            .ok_or(PdfExternalImageIdError)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfExternalImageIdError;

impl std::fmt::Display for PdfExternalImageIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PDF external-image object number must be in 1..=2147483647")
    }
}

impl std::error::Error for PdfExternalImageIdError {}

/// The selected PDF page box, already normalized into TeX scaled points.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PdfPageBox {
    pub left: Scaled,
    pub bottom: Scaled,
    pub right: Scaled,
    pub top: Scaled,
}

/// The inherited clockwise rotation of an imported PDF page.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum PdfPageRotation {
    #[default]
    None,
    Clockwise90,
    UpsideDown,
    Clockwise270,
}

impl PdfPageRotation {
    #[must_use]
    pub const fn swaps_axes(self) -> bool {
        matches!(self, Self::Clockwise90 | Self::Clockwise270)
    }
}

/// Metadata retained after host-neutral external-image validation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum PdfExternalImageMetadata {
    PdfPage {
        page_box: PdfPageBox,
        rotation: PdfPageRotation,
        page: u32,
        total_pages: u32,
        has_page_group: bool,
        pdf_version: (u8, u8),
    },
    Raster(PdfRasterImageMetadata),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum PdfRasterFormat {
    Jpeg,
    Png,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum PdfRasterColorSpace {
    Gray,
    Rgb,
    Cmyk,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PdfRasterImageMetadata {
    pub format: PdfRasterFormat,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: PdfRasterColorSpace,
    pub alpha: bool,
    pub png_color_type: Option<u8>,
}

impl PdfRasterImageMetadata {
    #[must_use]
    pub const fn placeholder() -> Self {
        Self {
            format: PdfRasterFormat::Png,
            width: 0,
            height: 0,
            bits_per_component: 8,
            color_space: PdfRasterColorSpace::Gray,
            alpha: false,
            png_color_type: Some(0),
        }
    }
}

/// Detached, host-validated image facts returned to the engine scanner.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfExternalImageSource {
    pub identity: ContentHash,
    pub metadata: PdfExternalImageMetadata,
    pub natural_width: Scaled,
    pub natural_height: Scaled,
    pub bytes: Vec<u8>,
}

/// Final dimensions recorded by `\pdfximage` after optional scaling.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PdfExternalImageDimensions {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
}

impl PdfExternalImageMetadata {
    /// Returns pdfTeX's `\pdflastximagepages` value for this image.
    #[must_use]
    pub const fn page_count(self) -> u32 {
        match self {
            Self::PdfPage { total_pages, .. } => total_pages,
            Self::Raster(_) => 1,
        }
    }

    /// Returns pdfTeX's `\pdflastximagecolordepth` value for this image.
    #[must_use]
    pub const fn color_depth(self) -> u8 {
        match self {
            Self::PdfPage { .. } => 0,
            Self::Raster(metadata) => metadata.bits_per_component,
        }
    }

    #[must_use]
    pub const fn bbox_coordinate(self, index: u8) -> Option<Scaled> {
        match (self, index) {
            (Self::PdfPage { page_box, .. }, 1) => Some(page_box.left),
            (Self::PdfPage { page_box, .. }, 2) => Some(page_box.bottom),
            (Self::PdfPage { page_box, .. }, 3) => Some(page_box.right),
            (Self::PdfPage { page_box, .. }, 4) => Some(page_box.top),
            (Self::Raster(_), 1..=4) => Some(Scaled::from_raw(0)),
            (_, _) => None,
        }
    }
}

/// The page-group placement selected while including a PDF page.
///
/// pdfTeX shares the first included page group with the output page. Later
/// groups remain local to their included forms and do not replace that first
/// selection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfPageGroupInclusion {
    /// The included page has no `/Group` entry.
    None,
    /// Share this group between the included form and the output page.
    SelectForOutputPage,
    /// Keep this group on the included form without replacing the page group.
    KeepOnIncludedForm {
        warning: Option<PdfPageGroupWarning>,
    },
}

/// A diagnostic raised when multiple PDF page groups meet on one output page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfPageGroupWarning {
    MultipleGroupsOnOnePage,
}

impl PdfPageGroupWarning {
    pub const MULTIPLE_GROUPS_ON_ONE_PAGE: &'static str =
        "PDF inclusion: multiple pdfs with page group included in a single page";

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::MultipleGroupsOnOnePage => Self::MULTIPLE_GROUPS_ON_ONE_PAGE,
        }
    }
}

/// Per-output-page pdfTeX page-group selection policy.
///
/// Construct one selector at the start of each page shipout, then visit PDF
/// images in output order. The signed suppression parameter is interpreted
/// exactly like pdfTeX: only zero permits the collision warning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfPageGroupSelector {
    selected: bool,
    suppress_collision_warning: bool,
}

impl PdfPageGroupSelector {
    #[must_use]
    pub const fn new(suppress_warning_page_group: i32) -> Self {
        Self {
            selected: false,
            suppress_collision_warning: suppress_warning_page_group != 0,
        }
    }

    #[must_use]
    pub const fn has_selection(self) -> bool {
        self.selected
    }

    #[must_use]
    pub fn include(&mut self, has_page_group: bool) -> PdfPageGroupInclusion {
        if !has_page_group {
            return PdfPageGroupInclusion::None;
        }
        if !self.selected {
            self.selected = true;
            return PdfPageGroupInclusion::SelectForOutputPage;
        }
        PdfPageGroupInclusion::KeepOnIncludedForm {
            warning: (!self.suppress_collision_warning)
                .then_some(PdfPageGroupWarning::MultipleGroupsOnOnePage),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfExternalImageRecord {
    id: PdfExternalImageId,
    identity: ContentHash,
    metadata: PdfExternalImageMetadata,
    dimensions: PdfExternalImageDimensions,
    color_space_object: i32,
    bytes: Vec<u8>,
    mask_object: Option<u32>,
}

impl PdfExternalImageRecord {
    #[must_use]
    pub const fn id(&self) -> PdfExternalImageId {
        self.id
    }
    #[must_use]
    pub const fn identity(&self) -> ContentHash {
        self.identity
    }
    #[must_use]
    pub const fn metadata(&self) -> PdfExternalImageMetadata {
        self.metadata
    }
    #[must_use]
    pub const fn dimensions(&self) -> PdfExternalImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn color_space_object(&self) -> i32 {
        self.color_space_object
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn mask_object(&self) -> Option<u32> {
        self.mask_object
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PdfPageReservation {
    number: u32,
    object: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfExternalImageRegistrationError {
    Duplicate(PdfExternalImageId),
}

impl std::fmt::Display for PdfExternalImageRegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(id) => write!(
                f,
                "PDF external-image object {} is already registered",
                id.raw()
            ),
        }
    }
}

impl std::error::Error for PdfExternalImageRegistrationError {}

/// The PDF object ledger cannot reserve another indirect object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfObjectCapacityError;

impl std::fmt::Display for PdfObjectCapacityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PDF object number exceeds 2147483647")
    }
}

impl std::error::Error for PdfObjectCapacityError {}

/// Stable page resource and indirect-object identities for one PDF font.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfFontResourceRecord {
    font: FontId,
    source_identity: tex_fonts::FontSourceIdentity,
    resource_number: u32,
    object_number: u32,
    identity: tex_fonts::PdfFontResourceIdentity,
}

impl PdfFontResourceRecord {
    #[must_use]
    pub const fn font(self) -> FontId {
        self.font
    }
    #[must_use]
    pub const fn resource_number(self) -> u32 {
        self.resource_number
    }
    #[must_use]
    pub const fn object_number(self) -> u32 {
        self.object_number
    }
}

/// A host-neutral font-map mutation recorded by a pdfTeX action primitive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfFontMapOperation {
    /// An empty first `\pdfmapfile{}` or `\pdfmapline{}` suppresses the
    /// implicit default map without adding an entry.
    BlockDefault,
    File(tex_fonts::PdfFontMapFile),
    Line(tex_fonts::PdfFontMapEntry),
}

/// One validated `\pdfglyphtounicode` mapping. A `tfm:` prefix scopes the
/// mapping to one TeX metric name; otherwise it is global across fonts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PdfGlyphToUnicode {
    pub tfm_name: Option<Vec<u8>>,
    pub glyph_name: Vec<u8>,
    pub unicode: Vec<u32>,
}

/// PDF state that pdfTeX deliberately retains in a dumped format.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct PdfFormatState {
    version: u32,
    enabled: bool,
    next_object: u32,
    next_form_resource: u32,
    raw_objects: Vec<PdfFormatRawObject>,
    forms: Vec<PdfFormatForm>,
    external_images: Vec<PdfFormatImage>,
    glyph_to_unicode: Vec<PdfGlyphToUnicode>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PdfFormatRawObject {
    id: u32,
    data: Option<PdfFormatRawObjectData>,
    immediate: bool,
    referenced: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PdfFormatRawObjectData {
    stream: bool,
    stream_attr: Option<Vec<u8>>,
    file: bool,
    data: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PdfFormatForm {
    object: u32,
    resource: u32,
    nodes: Vec<u8>,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    attr: Option<Vec<u8>>,
    resources: Option<Vec<u8>>,
    immediate: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PdfFormatImage {
    id: u32,
    identity: [u8; 32],
    metadata: PdfExternalImageMetadata,
    dimensions: PdfExternalImageDimensions,
    color_space_object: i32,
    bytes: Vec<u8>,
    mask_object: Option<u32>,
}

/// An append-only font-output mutation. The log makes snapshots cheap and
/// ensures rollback discards the exact suffix produced after a checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PdfFontOperation {
    Map(PdfFontMapOperation),
    MapFileContent {
        logical_name: Vec<u8>,
        map: tex_fonts::PdfFontMap,
    },
    Attribute {
        font: FontId,
        bytes: Vec<u8>,
    },
    IncludeChars {
        font: FontId,
        chars: Vec<u8>,
    },
    GlyphToUnicode(PdfGlyphToUnicode),
    NoBuiltinToUnicode {
        font: FontId,
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

/// Live pdfTeX microtype and font-output controls.
///
/// The raw values remain ordinary grouped integer parameters in `Env`; this
/// projection gives downstream paragraph and font backends one typed,
/// host-neutral contract without introducing shadow state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfFontConfiguration {
    pub adjust_spacing: i32,
    pub protrude_chars: i32,
    pub tracing_fonts: i32,
    pub adjust_interword_glue: i32,
    pub prepend_kern: i32,
    pub append_kern: i32,
    pub generate_to_unicode: i32,
    pub pk_resolution: i32,
    pub omit_charset: i32,
}

impl PdfFontConfiguration {
    /// Enables expansion while final line boxes are packed.
    #[must_use]
    pub const fn adjusts_spacing(self) -> bool {
        self.adjust_spacing > 0
    }

    /// Enables expansion-aware line-breaking passes 7 and 8.
    #[must_use]
    pub const fn adjusts_line_breaking(self) -> bool {
        self.adjust_spacing > 1
    }

    /// Enables margin-kern insertion in materialized lines.
    #[must_use]
    pub const fn protrudes_chars(self) -> bool {
        self.protrude_chars > 0
    }

    /// Enables protrusion-aware line-breaking width calculations.
    #[must_use]
    pub const fn protrudes_during_line_breaking(self) -> bool {
        self.protrude_chars > 1
    }

    #[must_use]
    pub const fn traces_fonts(self) -> bool {
        self.tracing_fonts > 0
    }

    #[must_use]
    pub const fn adjusts_interword_glue(self) -> bool {
        self.adjust_interword_glue > 0
    }

    #[must_use]
    pub const fn prepends_kerns(self) -> bool {
        self.prepend_kern > 0
    }

    #[must_use]
    pub const fn appends_kerns(self) -> bool {
        self.append_kern > 0
    }

    #[must_use]
    pub const fn generates_to_unicode(self) -> bool {
        self.generate_to_unicode > 0
    }

    #[must_use]
    pub const fn omits_charset(self) -> bool {
        self.omit_charset != 0
    }

    /// Resolves pdfTeX's zero sentinel against driver configuration, then
    /// applies the engine's `72..=8000` DPI output-time clamp.
    #[must_use]
    pub const fn resolved_pk_resolution(self, driver_dpi: i32) -> i32 {
        let dpi = if self.pk_resolution == 0 {
            driver_dpi
        } else {
            self.pk_resolution
        };
        if dpi < 72 {
            72
        } else if dpi > 8_000 {
            8_000
        } else {
            dpi
        }
    }
}

/// pdfTeX output controls frozen by the first shipped page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfOutputParameters {
    pub output: i32,
    pub major_version: i32,
    pub minor_version: i32,
    pub compress_level: i32,
    pub object_compress_level: i32,
    pub decimal_digits: i32,
    /// Gamma controls fixed when PDF output is initialized.
    pub gamma: i32,
    pub image_gamma: i32,
    pub image_hicolor: i32,
    pub image_apply_gamma: i32,
    /// Raw draft value fixed by the first output write; positive enables it.
    pub draft_mode: i32,
    pub inclusion_copy_fonts: i32,
    /// PK resolution remains zero until a driver supplies its configured DPI.
    pub pk_resolution: i32,
    /// Normalized boolean controlling document-wide resource-name prefixes.
    pub unique_resource_names: i32,
}

impl PdfOutputParameters {
    /// Applies pdfTeX's first-PDF-write recovery and clamping policy.
    #[must_use]
    pub fn normalized(self) -> Self {
        let major_version = self.major_version.max(1);
        let minor_version = if (0..=9).contains(&self.minor_version) {
            self.minor_version
        } else {
            4
        };
        let mut object_compress_level = self.object_compress_level.clamp(0, 3);
        if major_version == 1 && minor_version < 5 {
            object_compress_level = 0;
        }
        Self {
            major_version,
            minor_version,
            object_compress_level,
            decimal_digits: self.decimal_digits.clamp(0, 4),
            gamma: self.gamma.clamp(0, 1_000_000),
            image_gamma: self.image_gamma.clamp(0, 1_000_000),
            image_hicolor: self.image_hicolor.clamp(0, 1),
            image_apply_gamma: self.image_apply_gamma.clamp(0, 1),
            inclusion_copy_fonts: self.inclusion_copy_fonts.clamp(0, 1),
            pk_resolution: if self.pk_resolution == 0 {
                0
            } else {
                self.pk_resolution.clamp(72, 8_000)
            },
            unique_resource_names: i32::from(self.unique_resource_names > 0),
            ..self
        }
    }
}

pub(crate) struct PdfTokenParameter<G> {
    pub(crate) tokens: TokenListId<G>,
    pub(crate) semantic_id: StateHashFragment,
}

impl<G> Copy for PdfTokenParameter<G> {}

impl<G> Clone for PdfTokenParameter<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> std::fmt::Debug for PdfTokenParameter<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PdfTokenParameter")
            .field("tokens", &self.tokens)
            .field("semantic_id", &self.semantic_id)
            .finish()
    }
}

impl<G> PartialEq for PdfTokenParameter<G> {
    fn eq(&self, other: &Self) -> bool {
        self.tokens == other.tokens && self.semantic_id == other.semantic_id
    }
}

impl<G> Eq for PdfTokenParameter<G> {}

impl<G> Hash for PdfTokenParameter<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tokens.hash(state);
        self.semantic_id.hash(state);
    }
}

impl<G> PdfTokenParameter<G> {
    #[must_use]
    pub(crate) const fn id(&self) -> TokenListId<G> {
        self.tokens
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfPageParameters<G> {
    pub(crate) h_origin: Scaled,
    pub(crate) v_origin: Scaled,
    pub(crate) width: Scaled,
    pub(crate) height: Scaled,
    pub(crate) link_margin: Scaled,
    pub(crate) page_attr: PdfTokenParameter<G>,
    pub(crate) resources: PdfTokenParameter<G>,
    /// Raw `\pdfomitprocset` value captured when this page is shipped.
    pub(crate) omit_procset: i32,
    pub(crate) space_font_name: u32,
}

impl<G> Copy for PdfPageParameters<G> {}

impl<G> Clone for PdfPageParameters<G> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Stable object identities assigned to one committed PDF page.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfPageRecord<G> {
    artifact: ContentHash,
    resources_object: u32,
    contents_object: u32,
    page_object: u32,
    parameters: PdfPageParameters<G>,
}

impl<G> Copy for PdfPageRecord<G> {}

impl<G> Clone for PdfPageRecord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Immutable captured box and canonical identities for one `\pdfxform`.
pub struct PdfFormRecord<G> {
    object: u32,
    resource: u32,
    box_list: DurableListId<G>,
    box_semantic_id: StateHashFragment,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    attr: Option<PdfTokenParameter<G>>,
    resources: Option<PdfTokenParameter<G>>,
    immediate: bool,
}

impl<G> Clone for PdfFormRecord<G> {
    fn clone(&self) -> Self {
        Self {
            object: self.object,
            resource: self.resource,
            box_list: self.box_list,
            box_semantic_id: self.box_semantic_id,
            width: self.width,
            height: self.height,
            depth: self.depth,
            attr: self.attr,
            resources: self.resources,
            immediate: self.immediate,
        }
    }
}

impl<G> std::fmt::Debug for PdfFormRecord<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PdfFormRecord")
            .field("object", &self.object)
            .field("resource", &self.resource)
            .field("box_list", &self.box_list)
            .field("box_semantic_id", &self.box_semantic_id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("depth", &self.depth)
            .field("attr", &self.attr)
            .field("resources", &self.resources)
            .field("immediate", &self.immediate)
            .finish()
    }
}

impl<G> PartialEq for PdfFormRecord<G> {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object
            && self.resource == other.resource
            && self.box_list == other.box_list
            && self.box_semantic_id == other.box_semantic_id
            && self.width == other.width
            && self.height == other.height
            && self.depth == other.depth
            && self.attr == other.attr
            && self.resources == other.resources
            && self.immediate == other.immediate
    }
}

impl<G> Eq for PdfFormRecord<G> {}

impl<G> std::hash::Hash for PdfFormRecord<G> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.object.hash(state);
        self.resource.hash(state);
        self.box_list.hash(state);
        self.box_semantic_id.hash(state);
        self.width.hash(state);
        self.height.hash(state);
        self.depth.hash(state);
        self.attr.hash(state);
        self.resources.hash(state);
        self.immediate.hash(state);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfFormArtifact {
    bytes: Vec<u8>,
    last_position: Option<(Scaled, Scaled)>,
    snap_reference: (Scaled, Scaled),
}

impl PdfFormArtifact {
    #[must_use]
    pub fn new(
        bytes: Vec<u8>,
        last_position: Option<(Scaled, Scaled)>,
        snap_reference: (Scaled, Scaled),
    ) -> Self {
        Self {
            bytes,
            last_position,
            snap_reference,
        }
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub const fn last_position(&self) -> Option<(Scaled, Scaled)> {
        self.last_position
    }
    #[must_use]
    pub const fn snap_reference(&self) -> (Scaled, Scaled) {
        self.snap_reference
    }
}

impl<G> PdfFormRecord<G> {
    #[must_use]
    pub const fn object(&self) -> u32 {
        self.object
    }
    #[must_use]
    pub const fn resource(&self) -> u32 {
        self.resource
    }
    #[must_use]
    pub const fn box_list(&self) -> DurableListId<G> {
        self.box_list
    }
    #[must_use]
    pub const fn width(&self) -> Scaled {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> Scaled {
        self.height
    }
    #[must_use]
    pub const fn depth(&self) -> Scaled {
        self.depth
    }
    #[must_use]
    pub fn attr(&self) -> Option<TokenListId<G>> {
        self.attr.as_ref().map(PdfTokenParameter::<G>::id)
    }
    #[must_use]
    pub fn resources(&self) -> Option<TokenListId<G>> {
        self.resources.as_ref().map(PdfTokenParameter::<G>::id)
    }
    #[must_use]
    pub const fn immediate(&self) -> bool {
        self.immediate
    }
}

impl<G> PdfPageRecord<G> {
    pub(crate) fn retarget_artifact(&mut self, artifact: ContentHash) {
        self.artifact = artifact;
    }
    #[must_use]
    pub const fn artifact(&self) -> ContentHash {
        self.artifact
    }
    #[must_use]
    pub const fn resources_object(&self) -> u32 {
        self.resources_object
    }
    #[must_use]
    pub const fn contents_object(&self) -> u32 {
        self.contents_object
    }
    #[must_use]
    pub const fn page_object(&self) -> u32 {
        self.page_object
    }
    #[must_use]
    pub const fn h_origin(&self) -> Scaled {
        self.parameters.h_origin
    }
    #[must_use]
    pub const fn v_origin(&self) -> Scaled {
        self.parameters.v_origin
    }
    #[must_use]
    pub const fn width(&self) -> Scaled {
        self.parameters.width
    }
    #[must_use]
    pub const fn height(&self) -> Scaled {
        self.parameters.height
    }
    #[must_use]
    pub const fn link_margin(&self) -> Scaled {
        self.parameters.link_margin
    }
    #[must_use]
    pub fn page_attr(&self) -> TokenListId<G> {
        self.parameters.page_attr.id()
    }
    #[must_use]
    pub fn resources(&self) -> TokenListId<G> {
        self.parameters.resources.id()
    }
    #[must_use]
    pub const fn omit_procset(&self) -> i32 {
        self.parameters.omit_procset
    }
    #[must_use]
    pub const fn space_font_name_id(&self) -> u32 {
        self.parameters.space_font_name
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfStateCursor<G> {
    enabled: bool,
    next_object: u32,
    page_count: usize,
    output_parameters: Option<PdfOutputParameters>,
    pk_mode: Option<PdfTokenParameter<G>>,
    font_operation_count: usize,
    font_resource_count: usize,
    fingerprint: StateHashFragment,
    match_fingerprint: StateHashFragment,
    external_image_fingerprint: StateHashFragment,
    raw_object_fingerprint: StateHashFragment,
    document_fragment_fingerprint: StateHashFragment,
    document_objects: PdfDocumentObjectIds,
    catalog_open_action: Option<PdfActionRecord<G>>,
    action_fingerprint: StateHashFragment,
    page_reservation_fingerprint: StateHashFragment,
    space_font_name_count: usize,
    current_space_font_name: u32,
    space_font_name_fingerprint: StateHashFragment,
    annotation_fingerprint: StateHashFragment,
    link_fingerprint: StateHashFragment,
    open_link_fingerprint: StateHashFragment,
    color_stack_fingerprint: StateHashFragment,
    last_position: (Scaled, Scaled),
    snap_reference: (Scaled, Scaled),
    form_fingerprint: StateHashFragment,
    next_form_resource: u32,
    form_artifact_fingerprint: StateHashFragment,
    return_value: i32,
    destination_fingerprint: StateHashFragment,
    structure_destination_fingerprint: StateHashFragment,
    outline_fingerprint: StateHashFragment,
    thread_fingerprint: StateHashFragment,
}

impl<G> Copy for PdfStateCursor<G> {}

impl<G> Clone for PdfStateCursor<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug)]
pub(crate) struct PdfStateSnapshot<G> {
    cursor: PdfStateCursor<G>,
    match_state: PdfMatchState,
    external_images: Vec<PdfExternalImageRecord>,
    raw_objects: PdfRawObjects<G>,
    document_fragments: PdfDocumentFragments<G>,
    page_reservations: Vec<PdfPageReservation>,
    annotations: Vec<PdfAnnotationRecord<G>>,
    links: Vec<PdfLinkRecord<G>>,
    open_links: Vec<PdfOpenLink<G>>,
    color_stacks: Vec<PdfColorStack>,
    forms: Vec<PdfFormRecord<G>>,
    form_artifacts: BTreeMap<u32, PdfFormArtifact>,
    destinations: Vec<PdfDestinationRecord>,
    structure_destinations: Vec<PdfDestinationRecord>,
    outlines: Vec<PdfOutlineRecord<G>>,
    threads: Vec<PdfThreadRecord>,
}

impl<G> Clone for PdfStateSnapshot<G> {
    fn clone(&self) -> Self {
        Self {
            cursor: self.cursor,
            match_state: self.match_state.clone(),
            external_images: self.external_images.clone(),
            raw_objects: self.raw_objects.clone(),
            document_fragments: self.document_fragments.clone(),
            page_reservations: self.page_reservations.clone(),
            annotations: self.annotations.clone(),
            links: self.links.clone(),
            open_links: self.open_links.clone(),
            color_stacks: self.color_stacks.clone(),
            forms: self.forms.clone(),
            form_artifacts: self.form_artifacts.clone(),
            destinations: self.destinations.clone(),
            structure_destinations: self.structure_destinations.clone(),
            outlines: self.outlines.clone(),
            threads: self.threads.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct PdfMatchState {
    haystack: Vec<u8>,
    captures: Vec<Option<(u32, u32)>>,
    slot_count: u32,
    matched: bool,
    fingerprint: StateHashFragment,
}

impl Default for PdfMatchState {
    fn default() -> Self {
        Self {
            haystack: Vec::new(),
            captures: Vec::new(),
            slot_count: 0,
            matched: false,
            fingerprint: match_fingerprint(&[], &[], 0, false),
        }
    }
}

/// Live append-only PDF allocation state owned by one Universe timeline.
#[derive(Debug)]
pub(crate) struct PdfState<G> {
    enabled: bool,
    next_object: u32,
    pages: Vec<PdfPageRecord<G>>,
    output_parameters: Option<PdfOutputParameters>,
    pk_mode: Option<PdfTokenParameter<G>>,
    font_operations: Vec<PdfFontOperation>,
    font_resources: Vec<PdfFontResourceRecord>,
    fingerprint: StateHashFragment,
    match_state: PdfMatchState,
    external_images: Vec<PdfExternalImageRecord>,
    external_image_fingerprint: StateHashFragment,
    raw_objects: PdfRawObjects<G>,
    document_fragments: PdfDocumentFragments<G>,
    document_objects: PdfDocumentObjectIds,
    catalog_open_action: Option<PdfActionRecord<G>>,
    action_fingerprint: StateHashFragment,
    page_reservations: Vec<PdfPageReservation>,
    page_reservation_fingerprint: StateHashFragment,
    space_font_names: Vec<Vec<u8>>,
    space_font_name_lookup: BTreeMap<Vec<u8>, u32>,
    current_space_font_name: u32,
    space_font_name_fingerprint: StateHashFragment,
    annotations: Vec<PdfAnnotationRecord<G>>,
    annotation_fingerprint: StateHashFragment,
    links: Vec<PdfLinkRecord<G>>,
    link_fingerprint: StateHashFragment,
    open_links: Vec<PdfOpenLink<G>>,
    open_link_fingerprint: StateHashFragment,
    color_stacks: Vec<PdfColorStack>,
    color_stack_fingerprint: StateHashFragment,
    last_position: (Scaled, Scaled),
    snap_reference: (Scaled, Scaled),
    forms: Vec<PdfFormRecord<G>>,
    form_fingerprint: StateHashFragment,
    next_form_resource: u32,
    form_artifacts: BTreeMap<u32, PdfFormArtifact>,
    form_artifact_fingerprint: StateHashFragment,
    return_value: i32,
    destinations: Vec<PdfDestinationRecord>,
    destination_fingerprint: StateHashFragment,
    structure_destinations: Vec<PdfDestinationRecord>,
    structure_destination_fingerprint: StateHashFragment,
    outlines: Vec<PdfOutlineRecord<G>>,
    outline_fingerprint: StateHashFragment,
    threads: Vec<PdfThreadRecord>,
    thread_fingerprint: StateHashFragment,
}

impl<G> Default for PdfState<G> {
    fn default() -> Self {
        let default_space_font = b"pdftexspace".to_vec();
        Self {
            enabled: false,
            next_object: FIRST_DYNAMIC_OBJECT,
            pages: Vec::new(),
            output_parameters: None,
            pk_mode: None,
            font_operations: Vec::new(),
            font_resources: Vec::new(),
            fingerprint: base_fingerprint(false),
            match_state: PdfMatchState::default(),
            external_images: Vec::new(),
            external_image_fingerprint: external_image_base_fingerprint(),
            raw_objects: PdfRawObjects::<G>::default(),
            document_fragments: PdfDocumentFragments::<G>::default(),
            document_objects: PdfDocumentObjectIds::default(),
            catalog_open_action: None,
            action_fingerprint: StateHasher::new_exact(0x7064_665f_6163_746e).finish_fragment(),
            page_reservations: Vec::new(),
            page_reservation_fingerprint: StateHasher::new_exact(0x7064_665f_7067_7273)
                .finish_fragment(),
            space_font_names: vec![default_space_font.clone()],
            space_font_name_lookup: BTreeMap::from([(default_space_font.clone(), 0)]),
            current_space_font_name: 0,
            space_font_name_fingerprint: space_font_name_fingerprint(&default_space_font),
            annotations: Vec::new(),
            annotation_fingerprint: annotation_fingerprint::<G>(&[]),
            links: Vec::new(),
            link_fingerprint: StateHasher::new_exact(0x7064_665f_6c69_6e6b).finish_fragment(),
            open_links: Vec::new(),
            open_link_fingerprint: open_link_fingerprint::<G>(&[]),
            color_stacks: Vec::new(),
            color_stack_fingerprint: color_stack_fingerprint(&[]),
            last_position: (Scaled::from_raw(0), Scaled::from_raw(0)),
            snap_reference: (Scaled::from_raw(0), Scaled::from_raw(0)),
            forms: Vec::new(),
            form_fingerprint: StateHasher::new_exact(PDF_FORM_DOMAIN).finish_fragment(),
            next_form_resource: 1,
            form_artifacts: BTreeMap::new(),
            form_artifact_fingerprint: StateHasher::new_exact(0x7064_665f_666d_6172)
                .finish_fragment(),
            return_value: 0,
            destinations: Vec::new(),
            destination_fingerprint: destination_fingerprint(&[], false),
            structure_destinations: Vec::new(),
            structure_destination_fingerprint: destination_fingerprint(&[], true),
            outlines: Vec::new(),
            outline_fingerprint: outline_fingerprint::<G>(&[]),
            threads: Vec::new(),
            thread_fingerprint: thread_fingerprint(&[]),
        }
    }
}

impl<G> PdfState<G> {
    pub(crate) fn enable(&mut self) {
        if self.enabled {
            return;
        }
        debug_assert!(self.pages.is_empty());
        self.enabled = true;
        self.next_object = FIRST_DYNAMIC_OBJECT;
        self.fingerprint = base_fingerprint(true);
    }

    #[must_use]
    pub(crate) const fn enabled(&self) -> bool {
        self.enabled
    }
    #[must_use]
    pub(crate) fn pages(&self) -> &[PdfPageRecord<G>] {
        &self.pages
    }

    pub(crate) fn take_page_suffix(&mut self, start: usize) -> Vec<PdfPageRecord<G>> {
        self.pages.split_off(start.min(self.pages.len()))
    }

    pub(crate) fn restore_page_suffix(&mut self, pages: Vec<PdfPageRecord<G>>) {
        self.pages.extend(pages);
    }
    pub(crate) fn set_space_font_name(&mut self, name: Vec<u8>) {
        let id = if let Some(&id) = self.space_font_name_lookup.get(&name) {
            id
        } else {
            let id = u32::try_from(self.space_font_names.len())
                .expect("PDF space-font name count fits u32");
            self.space_font_names.push(name.clone());
            self.space_font_name_lookup.insert(name, id);
            id
        };
        self.current_space_font_name = id;
        self.space_font_name_fingerprint =
            space_font_name_fingerprint(&self.space_font_names[id as usize]);
    }
    #[must_use]
    pub(crate) const fn current_space_font_name_id(&self) -> u32 {
        self.current_space_font_name
    }
    #[must_use]
    pub(crate) fn space_font_name(&self, id: u32) -> Option<&[u8]> {
        self.space_font_names.get(id as usize).map(Vec::as_slice)
    }
    #[must_use]
    pub(crate) const fn next_object(&self) -> u32 {
        self.next_object
    }
    pub(crate) fn capture_format(
        &self,
        mut detach_tokens: impl FnMut(TokenListId<G>) -> Result<Vec<u8>, String>,
        mut detach_nodes: impl FnMut(DurableListId<G>) -> Result<Vec<u8>, String>,
    ) -> Result<Option<PdfFormatState>, String> {
        let glyph_to_unicode = self
            .font_operations
            .iter()
            .map(|operation| match operation {
                PdfFontOperation::GlyphToUnicode(mapping) => Some(mapping.clone()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>();
        let Some(glyph_to_unicode) = glyph_to_unicode else {
            return Ok(None);
        };
        let has_only_format_state = self.pages.is_empty()
            && self.output_parameters.is_none()
            && self.pk_mode.is_none()
            && self.font_resources.is_empty()
            && self.document_fragments.is_empty()
            && self.document_objects == PdfDocumentObjectIds::default()
            && self.catalog_open_action.is_none()
            && self.page_reservations.is_empty()
            && self.space_font_names.len() == 1
            && self.current_space_font_name == 0
            && self.annotations.is_empty()
            && self.links.is_empty()
            && self.open_links.is_empty()
            && self.color_stacks.is_empty()
            && self.last_position == (Scaled::from_raw(0), Scaled::from_raw(0))
            && self.snap_reference == (Scaled::from_raw(0), Scaled::from_raw(0))
            && self.form_artifacts.is_empty()
            && self.destinations.is_empty()
            && self.structure_destinations.is_empty()
            && self.outlines.is_empty()
            && self.threads.is_empty();
        if !has_only_format_state {
            return Ok(None);
        }
        let raw_objects = self
            .raw_objects
            .records()
            .iter()
            .map(|record| {
                let data = record
                    .data()
                    .map(|data| {
                        Ok::<_, String>(PdfFormatRawObjectData {
                            stream: data.is_stream(),
                            stream_attr: data.stream_attr().map(&mut detach_tokens).transpose()?,
                            file: data.is_file(),
                            data: detach_tokens(data.data())?,
                        })
                    })
                    .transpose()?;
                Ok(PdfFormatRawObject {
                    id: record.id().raw(),
                    data,
                    immediate: record.is_immediate(),
                    referenced: record.is_referenced(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let forms = self
            .forms
            .iter()
            .map(|form| {
                Ok(PdfFormatForm {
                    object: form.object,
                    resource: form.resource,
                    nodes: detach_nodes(form.box_list)?,
                    width: form.width,
                    height: form.height,
                    depth: form.depth,
                    attr: form
                        .attr
                        .as_ref()
                        .map(|value| detach_tokens(value.id()))
                        .transpose()?,
                    resources: form
                        .resources
                        .as_ref()
                        .map(|value| detach_tokens(value.id()))
                        .transpose()?,
                    immediate: form.immediate,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let external_images = self
            .external_images
            .iter()
            .map(|image| PdfFormatImage {
                id: image.id.raw(),
                identity: image.identity.bytes(),
                metadata: image.metadata,
                dimensions: image.dimensions,
                color_space_object: image.color_space_object,
                bytes: image.bytes.to_vec(),
                mask_object: image.mask_object,
            })
            .collect();
        Ok(Some(PdfFormatState {
            version: 1,
            enabled: self.enabled,
            next_object: self.next_object,
            next_form_resource: self.next_form_resource,
            raw_objects,
            forms,
            external_images,
            glyph_to_unicode,
        }))
    }

    /// Detaches the format-retained PDF ledger into a handle-free wire image.
    ///
    /// Token and node closures run only while capturing the live generation;
    /// the resulting bytes contain no arena coordinates or generation owner.
    pub(crate) fn capture_format_bytes(
        &self,
        detach_tokens: impl FnMut(TokenListId<G>) -> Result<Vec<u8>, String>,
        detach_nodes: impl FnMut(DurableListId<G>) -> Result<Vec<u8>, String>,
    ) -> Result<Option<Vec<u8>>, String> {
        self.capture_format(detach_tokens, detach_nodes)?
            .map(|format| {
                bincode::serialize(&format)
                    .map_err(|error| format!("cannot encode PDF format resource state: {error}"))
            })
            .transpose()
    }

    /// Validates and materializes one handle-free PDF format wire image.
    ///
    /// The returned state is destination-local and unpublished. Its caller
    /// can therefore stage it beside the other format sections and move it
    /// into the destination aggregate only after every section succeeds.
    pub(crate) fn restore_format_bytes(
        bytes: &[u8],
        import_tokens: impl FnMut(&[u8]) -> Result<PdfTokenParameter<G>, String>,
        import_nodes: impl FnMut(&[u8]) -> Result<(DurableListId<G>, StateHashFragment), String>,
    ) -> Result<Self, String> {
        let format = bincode::deserialize(bytes)
            .map_err(|error| format!("cannot decode PDF format resource state: {error}"))?;
        Self::restore_format(format, import_tokens, import_nodes)
    }

    pub(crate) fn restore_format(
        format: PdfFormatState,
        mut import_tokens: impl FnMut(&[u8]) -> Result<PdfTokenParameter<G>, String>,
        mut import_nodes: impl FnMut(&[u8]) -> Result<(DurableListId<G>, StateHashFragment), String>,
    ) -> Result<Self, String> {
        if format.version != 1 || format.next_object == 0 || format.next_form_resource == 0 {
            return Err("unsupported or invalid PDF format resource state".to_owned());
        }
        let mut allocated = format
            .raw_objects
            .iter()
            .map(|record| record.id)
            .chain(
                format
                    .forms
                    .iter()
                    .flat_map(|form| [form.object, form.object.saturating_add(1)]),
            )
            .chain(
                format
                    .external_images
                    .iter()
                    .flat_map(|image| std::iter::once(image.id).chain(image.mask_object)),
            );
        if allocated.any(|object| object == 0 || object >= format.next_object) {
            return Err("PDF format resource identity is outside the allocation ledger".to_owned());
        }
        if !format
            .raw_objects
            .windows(2)
            .all(|pair| pair[0].id < pair[1].id)
            || !format
                .forms
                .windows(2)
                .all(|pair| pair[0].object < pair[1].object)
            || !format
                .external_images
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id)
        {
            return Err("PDF format resources are not in canonical identity order".to_owned());
        }
        let mut state = Self::default();
        if format.enabled {
            state.enable();
        }
        for mapping in format.glyph_to_unicode {
            state.set_glyph_to_unicode(mapping);
        }
        for record in format.raw_objects {
            let id = PdfRawObjectId::from_allocated(record.id);
            state.raw_objects.reserve(id);
            if let Some(data) = record.data {
                let attr = data
                    .stream_attr
                    .as_deref()
                    .map(&mut import_tokens)
                    .transpose()?;
                let body = import_tokens(&data.data)?;
                state
                    .initialize_raw_object(
                        id,
                        PdfRawObjectData::<G>::new(data.stream, attr, data.file, body),
                        record.immediate,
                    )
                    .map_err(|_| "invalid PDF raw-object initialization".to_owned())?;
            }
            if record.referenced {
                state
                    .reference_raw_object(id)
                    .map_err(|_| "invalid PDF raw-object reference".to_owned())?;
            }
        }
        for form in format.forms {
            let (nodes, semantic_id) = import_nodes(&form.nodes)?;
            let attr = form.attr.as_deref().map(&mut import_tokens).transpose()?;
            let resources = form
                .resources
                .as_deref()
                .map(&mut import_tokens)
                .transpose()?;
            state
                .initialize_form(
                    (form.object, form.resource),
                    nodes,
                    semantic_id,
                    (form.width, form.height, form.depth),
                    (attr, resources),
                    form.immediate,
                )
                .map_err(|error| error.to_string())?;
        }
        if !format.external_images.is_empty() {
            state.external_images = format
                .external_images
                .into_iter()
                .map(|image| PdfExternalImageRecord {
                    id: PdfExternalImageId(image.id),
                    identity: ContentHash::new(image.identity),
                    metadata: image.metadata,
                    dimensions: image.dimensions,
                    color_space_object: image.color_space_object,
                    bytes: image.bytes,
                    mask_object: image.mask_object,
                })
                .collect();
            state.external_image_fingerprint = external_image_fingerprint(&state.external_images);
        }
        state.next_object = format.next_object;
        state.next_form_resource = format.next_form_resource;
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) fn is_format_empty(&self) -> bool {
        self.next_object == FIRST_DYNAMIC_OBJECT
            && self.raw_objects.records().is_empty()
            && self.external_images.is_empty()
            && self.forms.is_empty()
            && self.font_operations.is_empty()
            && self.document_fragments.is_empty()
    }

    pub(crate) fn ensure_page_capacity(&self, parameters: PdfOutputParameters) -> Result<(), ()> {
        if !self.enabled || self.output_parameters.unwrap_or(parameters).output <= 0 {
            return Ok(());
        }
        let object_count = if self
            .reserved_page_object((self.pages.len() + 1) as u32)
            .is_some()
        {
            2
        } else {
            OBJECTS_PER_PAGE
        };
        let last = self.next_object.checked_add(object_count - 1).ok_or(())?;
        (last <= MAX_OBJECT_ID).then_some(()).ok_or(())
    }

    pub(crate) fn commit_page(
        &mut self,
        artifact: ContentHash,
        output: PdfOutputParameters,
        page: PdfPageParameters<G>,
        pk_mode: PdfTokenParameter<G>,
    ) {
        if !self.enabled {
            return;
        }
        let output = match self.output_parameters {
            Some(parameters) => parameters,
            None => {
                self.output_parameters = Some(output);
                self.fingerprint = freeze_fingerprint(self.fingerprint, output);
                output
            }
        };
        if output.output <= 0 {
            return;
        }
        if self.pk_mode.is_none() {
            self.fingerprint = freeze_pk_mode_fingerprint(self.fingerprint, &pk_mode);
            self.pk_mode = Some(pk_mode);
        }
        self.ensure_page_capacity(output)
            .expect("PDF page object capacity was preflighted");
        let page_number =
            u32::try_from(self.pages.len() + 1).expect("page count fits PDF object cap");
        let reserved_page = self.reserved_page_object(page_number);
        let record = PdfPageRecord {
            artifact,
            resources_object: self.next_object,
            contents_object: self.next_object + u32::from(reserved_page.is_none()) + 1,
            page_object: reserved_page.unwrap_or(self.next_object + 1),
            parameters: page,
        };
        self.next_object += if reserved_page.is_some() {
            2
        } else {
            OBJECTS_PER_PAGE
        };
        self.fingerprint = append_fingerprint(self.fingerprint, &record);
        self.pages.push(record);
    }

    #[must_use]
    pub(crate) const fn output_parameters(&self) -> Option<PdfOutputParameters> {
        self.output_parameters
    }

    #[must_use]
    pub(crate) fn pk_mode(&self) -> Option<TokenListId<G>> {
        self.pk_mode.as_ref().map(PdfTokenParameter::<G>::id)
    }

    pub(crate) fn push_font_map(&mut self, operation: PdfFontMapOperation) {
        self.push_font_operation(PdfFontOperation::Map(operation));
    }

    pub(crate) fn provide_font_map_file(
        &mut self,
        logical_name: Vec<u8>,
        map: tex_fonts::PdfFontMap,
    ) {
        self.push_font_operation(PdfFontOperation::MapFileContent { logical_name, map });
    }

    pub(crate) fn has_font_map_file(&self, logical_name: &[u8]) -> bool {
        self.font_operations.iter().rev().any(|operation| {
            matches!(
                operation,
                PdfFontOperation::MapFileContent {
                    logical_name: candidate,
                    ..
                } if candidate == logical_name
            )
        })
    }

    pub(crate) fn set_font_attribute(&mut self, font: FontId, bytes: Vec<u8>) {
        self.push_font_operation(PdfFontOperation::Attribute { font, bytes });
    }

    pub(crate) fn include_font_chars(&mut self, font: FontId, chars: Vec<u8>) {
        self.push_font_operation(PdfFontOperation::IncludeChars { font, chars });
    }

    pub(crate) fn set_glyph_to_unicode(&mut self, mapping: PdfGlyphToUnicode) {
        self.push_font_operation(PdfFontOperation::GlyphToUnicode(mapping));
    }

    pub(crate) fn disable_builtin_to_unicode(&mut self, font: FontId) {
        self.push_font_operation(PdfFontOperation::NoBuiltinToUnicode { font });
    }

    pub(crate) fn provide_type1_program(
        &mut self,
        logical_name: Vec<u8>,
        program: tex_fonts::PdfType1Program,
    ) {
        self.push_font_operation(PdfFontOperation::Type1Program {
            logical_name,
            program,
        });
    }

    pub(crate) fn ensure_font_resource(
        &mut self,
        font: FontId,
        source_identity: tex_fonts::FontSourceIdentity,
        identity: tex_fonts::PdfFontResourceIdentity,
    ) -> Result<PdfFontResourceRecord, PdfObjectCapacityError> {
        if let Some(record) = self
            .font_resources
            .iter()
            .copied()
            .find(|record| record.font == font)
        {
            return Ok(record);
        }
        if let Some(record) = self
            .font_resources
            .iter()
            .copied()
            .find(|record| record.identity == identity)
        {
            let alias = PdfFontResourceRecord {
                font,
                source_identity,
                ..record
            };
            self.font_resources.push(alias);
            self.fingerprint = append_font_resource_fingerprint(self.fingerprint, alias);
            return Ok(alias);
        }
        if self.next_object > MAX_OBJECT_ID {
            return Err(PdfObjectCapacityError);
        }
        let record = PdfFontResourceRecord {
            font,
            source_identity,
            resource_number: font.raw(),
            object_number: self.next_object,
            identity,
        };
        self.next_object += 1;
        self.font_resources.push(record);
        self.fingerprint = append_font_resource_fingerprint(self.fingerprint, record);
        Ok(record)
    }

    pub(crate) fn font_resource(&self, font: FontId) -> Option<PdfFontResourceRecord> {
        self.font_resources
            .iter()
            .copied()
            .find(|record| record.font == font)
    }

    pub(crate) fn font_resource_by_identity(
        &self,
        identity: tex_fonts::FontSourceIdentity,
    ) -> Option<PdfFontResourceRecord> {
        self.font_resources
            .iter()
            .copied()
            .find(|record| record.source_identity == identity)
    }

    pub(crate) fn font_resources(&self) -> impl Iterator<Item = PdfFontResourceRecord> + '_ {
        self.font_resources
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, record)| {
                (!self.font_resources[..index]
                    .iter()
                    .any(|prior| prior.object_number == record.object_number))
                .then_some(record)
            })
    }

    /// Every live font-to-resource association, including aliases that share
    /// one emitted PDF object.
    ///
    /// [`Self::font_resources`] is the object-enumeration view used by the
    /// live ledger. Terminal detachment instead needs this identity view:
    /// page artifacts address realized semantic font identities, and two TeX
    /// fonts with different scale recipes may intentionally share one PDF
    /// resource object.
    pub(crate) fn font_resource_records(&self) -> impl Iterator<Item = PdfFontResourceRecord> + '_ {
        self.font_resources.iter().copied()
    }

    pub(crate) fn reserve_annotation(
        &mut self,
    ) -> Result<PdfAnnotationRecord<G>, PdfObjectCapacityError> {
        let object = self.reserve_document_object()?;
        let record = PdfAnnotationRecord::<G>::reserved(object);
        self.annotations.push(record);
        self.annotation_fingerprint =
            append_annotation_reservation_fingerprint(self.annotation_fingerprint, object);
        Ok(record)
    }

    pub(crate) fn initialize_annotation(
        &mut self,
        object: u32,
        data: PdfAnnotationData<G>,
        entries_semantic_id: StateHashFragment,
    ) -> Result<PdfAnnotationRecord<G>, PdfAnnotationInitializeError> {
        let records = &mut self.annotations;
        let record = records
            .iter_mut()
            .find(|record| record.object() == object)
            .ok_or(PdfAnnotationInitializeError(object))?;
        let dimensions = data.dimensions;
        record
            .initialize(data)
            .map_err(|()| PdfAnnotationInitializeError(object))?;
        self.annotation_fingerprint = append_annotation_data_fingerprint(
            self.annotation_fingerprint,
            object,
            dimensions,
            entries_semantic_id,
        );
        Ok(*record)
    }

    #[must_use]
    pub(crate) fn annotations(&self) -> &[PdfAnnotationRecord<G>] {
        &self.annotations
    }

    pub(crate) fn destination(
        &self,
        identity: &PdfDestinationIdentity,
        structure: bool,
    ) -> Option<&PdfDestinationRecord> {
        let records = if structure {
            &self.structure_destinations
        } else {
            &self.destinations
        };
        records.iter().find(|record| record.identity() == identity)
    }

    pub(crate) fn reserve_destination(
        &mut self,
        identity: PdfDestinationIdentity,
        structure: bool,
    ) -> Result<PdfDestinationRecord, PdfObjectCapacityError> {
        if let Some(record) = self.destination(&identity, structure) {
            return Ok(record.clone());
        }
        let object = self.reserve_document_object()?;
        let record = PdfDestinationRecord::reserved(identity, object);
        let records = if structure {
            &mut self.structure_destinations
        } else {
            &mut self.destinations
        };
        records.push(record.clone());
        if structure {
            self.structure_destination_fingerprint = destination_fingerprint(records, true);
        } else {
            self.destination_fingerprint = destination_fingerprint(records, false);
        }
        Ok(record)
    }

    pub(crate) fn define_destination(
        &mut self,
        identity: PdfDestinationIdentity,
        structure_target: Option<u32>,
    ) -> Result<PdfDestinationDefinition, PdfObjectCapacityError> {
        let structure = structure_target.is_some();
        let reserved = self.reserve_destination(identity, structure)?;
        let records = if structure {
            &mut self.structure_destinations
        } else {
            &mut self.destinations
        };
        let record = records
            .iter_mut()
            .find(|record| record.object() == reserved.object())
            .expect("reserved destination exists");
        let duplicate = !record.define(structure_target);
        let result = record.clone();
        if structure {
            self.structure_destination_fingerprint = destination_fingerprint(records, true);
        } else {
            self.destination_fingerprint = destination_fingerprint(records, false);
        }
        Ok(PdfDestinationDefinition {
            record: result,
            duplicate,
        })
    }

    pub(crate) fn destinations(&self, structure: bool) -> &[PdfDestinationRecord] {
        if structure {
            &self.structure_destinations
        } else {
            &self.destinations
        }
    }

    pub(crate) fn append_thread_bead(
        &mut self,
        identity: PdfDestinationIdentity,
    ) -> Result<(PdfThreadRecord, PdfThreadBeadRecord), PdfObjectCapacityError> {
        let index = self
            .threads
            .iter()
            .position(|thread| thread.identity() == &identity);
        let index = match index {
            Some(index) => index,
            None => {
                let object = self.reserve_document_object()?;
                self.threads.push(PdfThreadRecord::new(identity, object));
                self.threads.len() - 1
            }
        };
        let bead = PdfThreadBeadRecord::new(
            self.reserve_document_object()?,
            self.reserve_document_object()?,
        );
        let threads = &mut self.threads;
        threads[index].push_bead(bead);
        self.thread_fingerprint = thread_fingerprint(threads);
        Ok((threads[index].clone(), bead))
    }

    pub(crate) fn reserve_thread(
        &mut self,
        identity: PdfDestinationIdentity,
    ) -> Result<PdfThreadRecord, PdfObjectCapacityError> {
        if let Some(thread) = self
            .threads
            .iter()
            .find(|thread| thread.identity() == &identity)
        {
            return Ok(thread.clone());
        }
        let object = self.reserve_document_object()?;
        let record = PdfThreadRecord::new(identity, object);
        let threads = &mut self.threads;
        threads.push(record.clone());
        self.thread_fingerprint = thread_fingerprint(threads);
        Ok(record)
    }

    pub(crate) fn threads(&self) -> &[PdfThreadRecord] {
        &self.threads
    }

    /// Detaches unresolved navigation identities in pdfTeX's finalization
    /// order without exposing the checkpointed destination/thread ledgers.
    pub(crate) fn unresolved_navigation_warnings(&self) -> Vec<PdfNavigationWarning> {
        self.destinations
            .iter()
            .filter(|record| !record.defined())
            .map(|record| PdfNavigationWarning::Destination(record.identity().clone()))
            .chain(
                self.structure_destinations
                    .iter()
                    .filter(|record| !record.defined())
                    .map(|record| {
                        PdfNavigationWarning::StructureDestination(record.identity().clone())
                    }),
            )
            .chain(
                self.threads
                    .iter()
                    .filter(|record| record.beads().is_empty())
                    .map(|record| PdfNavigationWarning::Thread(record.identity().clone())),
            )
            .collect()
    }

    pub(crate) fn create_outline(
        &mut self,
        attributes: TokenListId<G>,
        action: PdfActionSpec<G>,
        count: i32,
        title: TokenListId<G>,
        semantic_ids: [StateHashFragment; 3],
    ) -> Result<PdfOutlineRecord<G>, PdfObjectCapacityError> {
        let action_object = self.reserve_document_object()?;
        let item_object = self.reserve_document_object()?;
        let title_object = self.reserve_document_object()?;
        let record = PdfOutlineRecord::<G>::new(
            action_object,
            item_object,
            title_object,
            attributes,
            action,
            count,
            title,
        );
        self.outline_fingerprint = append_outline_fingerprint(
            self.outline_fingerprint,
            &record,
            semantic_ids[0],
            semantic_ids[1],
            semantic_ids[2],
        );
        self.outlines.push(record);
        Ok(record)
    }

    pub(crate) fn outlines(&self) -> &[PdfOutlineRecord<G>] {
        &self.outlines
    }

    #[must_use]
    pub(crate) fn last_annotation(&self) -> u32 {
        self.annotations.last().map_or(0, |record| record.object())
    }

    pub(crate) fn create_link(
        &mut self,
        dimensions: PdfAnnotationDimensions,
        attributes: TokenListId<G>,
        action: PdfActionSpec<G>,
        attributes_semantic_id: StateHashFragment,
        action_semantic_id: StateHashFragment,
        nesting_depth: u32,
    ) -> Result<PdfLinkRecord<G>, PdfObjectCapacityError> {
        let object = self.reserve_document_object()?;
        let record = PdfLinkRecord::<G>::new(object, dimensions, attributes, action);
        self.link_fingerprint = append_link_fingerprint(
            self.link_fingerprint,
            &record,
            attributes_semantic_id,
            action_semantic_id,
        );
        self.open_links.push(PdfOpenLink {
            record,
            nesting_depth,
        });
        self.links.push(record);
        self.open_link_fingerprint = open_link_fingerprint(&self.open_links);
        Ok(record)
    }

    pub(crate) fn reserve_link_continuation(&mut self) -> Result<u32, PdfObjectCapacityError> {
        self.reserve_document_object()
    }

    pub(crate) fn end_link(&mut self) -> Option<PdfOpenLink<G>> {
        let open = self.open_links.pop();
        self.open_link_fingerprint = open_link_fingerprint(&self.open_links);
        open
    }

    #[must_use]
    pub(crate) fn links(&self) -> &[PdfLinkRecord<G>] {
        &self.links
    }

    #[must_use]
    pub(crate) fn last_link(&self) -> u32 {
        self.links.last().map_or(0, |record| record.object())
    }

    #[must_use]
    pub(crate) fn open_links(&self) -> &[PdfOpenLink<G>] {
        &self.open_links
    }

    #[must_use]
    pub(crate) fn type1_program(&self, logical_name: &[u8]) -> Option<&tex_fonts::PdfType1Program> {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::Type1Program {
                    logical_name: candidate,
                    program,
                } if candidate == logical_name => Some(program),
                _ => None,
            })
    }

    pub(crate) fn provide_encoding(
        &mut self,
        logical_name: Vec<u8>,
        encoding: tex_fonts::PdfEncoding,
    ) {
        self.push_font_operation(PdfFontOperation::Encoding {
            logical_name,
            encoding,
        });
    }

    pub(crate) fn encoding(&self, logical_name: &[u8]) -> Option<&tex_fonts::PdfEncoding> {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::Encoding {
                    logical_name: candidate,
                    encoding,
                } if candidate == logical_name => Some(encoding),
                _ => None,
            })
    }

    pub(crate) fn provide_truetype_program(
        &mut self,
        logical_name: Vec<u8>,
        program: tex_fonts::PdfTrueTypeProgram,
    ) {
        self.push_font_operation(PdfFontOperation::TrueTypeProgram {
            logical_name,
            program,
        });
    }

    pub(crate) fn provide_pk_font(
        &mut self,
        request: tex_fonts::PdfPkFontRequest,
        font: tex_fonts::PdfPkFont,
    ) {
        self.push_font_operation(PdfFontOperation::PkFont { request, font });
    }

    pub(crate) fn pk_font(
        &self,
        request: &tex_fonts::PdfPkFontRequest,
    ) -> Option<&tex_fonts::PdfPkFont> {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::PkFont {
                    request: candidate,
                    font,
                } if candidate == request => Some(font),
                _ => None,
            })
    }

    pub(crate) fn truetype_program(
        &self,
        logical_name: &[u8],
    ) -> Option<&tex_fonts::PdfTrueTypeProgram> {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::TrueTypeProgram {
                    logical_name: candidate,
                    program,
                } if candidate == logical_name => Some(program),
                _ => None,
            })
    }

    fn push_font_operation(&mut self, operation: PdfFontOperation) {
        self.fingerprint = append_font_fingerprint(self.fingerprint, &operation);
        self.font_operations.push(operation);
    }

    pub(crate) fn font_maps(&self) -> impl Iterator<Item = &PdfFontMapOperation> {
        self.font_operations
            .iter()
            .filter_map(|operation| match operation {
                PdfFontOperation::Map(map) => Some(map),
                PdfFontOperation::MapFileContent { .. }
                | PdfFontOperation::Attribute { .. }
                | PdfFontOperation::IncludeChars { .. }
                | PdfFontOperation::GlyphToUnicode(_)
                | PdfFontOperation::NoBuiltinToUnicode { .. }
                | PdfFontOperation::Type1Program { .. }
                | PdfFontOperation::Encoding { .. }
                | PdfFontOperation::TrueTypeProgram { .. }
                | PdfFontOperation::PkFont { .. } => None,
            })
    }

    #[must_use]
    pub(crate) fn font_map_file_requests(&self) -> Vec<Vec<u8>> {
        let maps = self.font_maps().collect::<Vec<_>>();
        let loads_default = maps.first().is_none_or(|operation| {
            Self::font_map_operation_directive(operation) != tex_fonts::PdfFontMapDirective::Default
        });
        let mut requests = BTreeMap::<Vec<u8>, ()>::new();
        if loads_default {
            requests.insert(b"pdftex.map".to_vec(), ());
        }
        for operation in maps {
            if let PdfFontMapOperation::File(file) = operation {
                requests.insert(file.logical_name.clone(), ());
            }
        }
        requests.into_keys().collect()
    }

    #[must_use]
    pub(crate) fn authoritative_font_map_names(&self) -> BTreeMap<Vec<u8>, ()> {
        let mut names = BTreeMap::new();
        for operation in self.font_maps() {
            match operation {
                PdfFontMapOperation::Line(entry)
                    if matches!(
                        entry.directive,
                        tex_fonts::PdfFontMapDirective::Replace
                            | tex_fonts::PdfFontMapDirective::Remove
                    ) =>
                {
                    names.insert(entry.tex_name.clone(), ());
                }
                PdfFontMapOperation::File(file)
                    if matches!(
                        file.directive,
                        tex_fonts::PdfFontMapDirective::Replace
                            | tex_fonts::PdfFontMapDirective::Remove
                    ) =>
                {
                    if let Some(map) =
                        self.font_operations
                            .iter()
                            .rev()
                            .find_map(|operation| match operation {
                                PdfFontOperation::MapFileContent { logical_name, map }
                                    if logical_name == &file.logical_name =>
                                {
                                    Some(map)
                                }
                                _ => None,
                            })
                    {
                        for entry in map.entries() {
                            names.insert(entry.tex_name.clone(), ());
                        }
                    }
                }
                _ => {}
            }
        }
        names
    }

    #[must_use]
    pub(crate) fn resolved_font_map_lines(&self) -> Vec<tex_fonts::PdfFontMapEntry> {
        self.resolve_font_map_lines().0.into_values().collect()
    }

    #[must_use]
    pub(crate) fn font_map_duplicate_names(&self) -> Vec<Vec<u8>> {
        self.resolve_font_map_lines().1
    }

    fn resolve_font_map_lines(
        &self,
    ) -> (BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>, Vec<Vec<u8>>) {
        let mut entries = BTreeMap::new();
        let mut duplicates = Vec::new();
        let maps = self.font_maps().collect::<Vec<_>>();
        if maps.first().is_none_or(|operation| {
            Self::font_map_operation_directive(operation) != tex_fonts::PdfFontMapDirective::Default
        }) {
            self.apply_font_map_file(
                b"pdftex.map",
                tex_fonts::PdfFontMapDirective::Default,
                &mut entries,
                &mut duplicates,
            );
        }
        for operation in maps {
            match operation {
                PdfFontMapOperation::BlockDefault => {}
                PdfFontMapOperation::Line(entry) => {
                    Self::apply_font_map_entry(entry.clone(), &mut entries, &mut duplicates);
                }
                PdfFontMapOperation::File(file) => self.apply_font_map_file(
                    &file.logical_name,
                    file.directive,
                    &mut entries,
                    &mut duplicates,
                ),
            }
        }
        (entries, duplicates)
    }

    fn apply_font_map_file(
        &self,
        logical_name: &[u8],
        directive: tex_fonts::PdfFontMapDirective,
        entries: &mut BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>,
        duplicates: &mut Vec<Vec<u8>>,
    ) {
        let Some(map) = self
            .font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::MapFileContent {
                    logical_name: candidate,
                    map,
                } if candidate == logical_name => Some(map),
                _ => None,
            })
        else {
            return;
        };
        for entry in map.entries() {
            let mut entry = entry.clone();
            entry.directive = directive;
            Self::apply_font_map_entry(entry, entries, duplicates);
        }
    }

    fn font_map_operation_directive(
        operation: &PdfFontMapOperation,
    ) -> tex_fonts::PdfFontMapDirective {
        match operation {
            PdfFontMapOperation::BlockDefault => tex_fonts::PdfFontMapDirective::Default,
            PdfFontMapOperation::File(file) => file.directive,
            PdfFontMapOperation::Line(line) => line.directive,
        }
    }

    fn apply_font_map_entry(
        entry: tex_fonts::PdfFontMapEntry,
        entries: &mut BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>,
        duplicates: &mut Vec<Vec<u8>>,
    ) {
        match entry.directive {
            tex_fonts::PdfFontMapDirective::Default | tex_fonts::PdfFontMapDirective::Add => {
                if entries.contains_key(&entry.tex_name) {
                    duplicates.push(entry.tex_name.clone());
                } else {
                    entries.insert(entry.tex_name.clone(), entry);
                }
            }
            tex_fonts::PdfFontMapDirective::Replace => {
                entries.insert(entry.tex_name.clone(), entry);
            }
            tex_fonts::PdfFontMapDirective::Remove => {
                entries.remove(&entry.tex_name);
            }
        }
    }

    #[must_use]
    pub(crate) fn font_attribute(&self, font: FontId) -> &[u8] {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::Attribute {
                    font: candidate,
                    bytes,
                } if *candidate == font => Some(bytes.as_slice()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub(crate) fn included_font_chars(&self, font: FontId) -> Vec<u8> {
        let mut included = [false; 256];
        for operation in &self.font_operations {
            if let PdfFontOperation::IncludeChars {
                font: candidate,
                chars,
            } = operation
                && *candidate == font
            {
                for &character in chars {
                    included[usize::from(character)] = true;
                }
            }
        }
        included
            .into_iter()
            .enumerate()
            .filter_map(|(character, present)| present.then_some(character as u8))
            .collect()
    }

    #[must_use]
    pub(crate) fn builtin_to_unicode_disabled(&self, font: FontId) -> bool {
        self.font_operations.iter().any(|operation| {
            matches!(operation, PdfFontOperation::NoBuiltinToUnicode { font: candidate } if *candidate == font)
        })
    }

    #[must_use]
    pub(crate) fn has_glyph_to_unicode_mappings(&self) -> bool {
        self.font_operations
            .iter()
            .any(|operation| matches!(operation, PdfFontOperation::GlyphToUnicode(_)))
    }

    #[must_use]
    pub(crate) fn glyph_to_unicode(&self, tfm_name: &[u8], glyph_name: &[u8]) -> Option<&[u32]> {
        let glyph_name = glyph_name
            .split(|byte| *byte == b'.')
            .next()
            .unwrap_or(glyph_name);
        for scoped in [true, false] {
            if let Some(mapping) = self.font_operations.iter().rev().find_map(|operation| {
                let PdfFontOperation::GlyphToUnicode(mapping) = operation else {
                    return None;
                };
                let scope_matches = if scoped {
                    mapping.tfm_name.as_deref() == Some(tfm_name)
                } else {
                    mapping.tfm_name.is_none()
                };
                (scope_matches && mapping.glyph_name == glyph_name).then_some(mapping)
            }) {
                return Some(&mapping.unicode);
            }
        }
        None
    }

    pub(crate) fn register_external_image(
        &mut self,
        id: PdfExternalImageId,
        metadata: PdfExternalImageMetadata,
    ) -> Result<(), PdfExternalImageRegistrationError> {
        let images = &mut self.external_images;
        match images.binary_search_by_key(&id, |record| record.id) {
            Ok(_) => return Err(PdfExternalImageRegistrationError::Duplicate(id)),
            Err(index) => images.insert(
                index,
                PdfExternalImageRecord {
                    id,
                    identity: ContentHash::new([0; 32]),
                    metadata,
                    dimensions: PdfExternalImageDimensions {
                        width: Scaled::from_raw(0),
                        height: Scaled::from_raw(0),
                        depth: Scaled::from_raw(0),
                    },
                    color_space_object: 0,
                    bytes: Vec::new(),
                    mask_object: None,
                },
            ),
        }
        self.external_image_fingerprint = external_image_fingerprint(images);
        Ok(())
    }

    #[must_use]
    pub(crate) fn external_image(
        &self,
        id: PdfExternalImageId,
    ) -> Option<PdfExternalImageMetadata> {
        self.external_images
            .binary_search_by_key(&id, |record| record.id)
            .ok()
            .map(|index| self.external_images[index].metadata)
    }

    #[must_use]
    pub(crate) fn external_image_record(
        &self,
        id: PdfExternalImageId,
    ) -> Option<PdfExternalImageRecord> {
        self.external_images
            .binary_search_by_key(&id, |record| record.id)
            .ok()
            .map(|index| self.external_images[index].clone())
    }

    pub(crate) fn allocate_external_image(
        &mut self,
        source: PdfExternalImageSource,
        dimensions: PdfExternalImageDimensions,
        color_space_object: i32,
    ) -> Result<PdfExternalImageRecord, PdfObjectCapacityError> {
        let needs_mask = matches!(
            source.metadata,
            PdfExternalImageMetadata::Raster(PdfRasterImageMetadata { alpha: true, .. })
        );
        self.next_object
            .checked_add(u32::from(needs_mask))
            .filter(|last| *last <= MAX_OBJECT_ID)
            .ok_or(PdfObjectCapacityError)?;
        let raw = self.reserve_document_object()?;
        let mask_object = needs_mask
            .then(|| self.reserve_document_object())
            .transpose()?;
        let record = PdfExternalImageRecord {
            id: PdfExternalImageId(raw),
            identity: source.identity,
            metadata: source.metadata,
            dimensions,
            color_space_object,
            bytes: source.bytes,
            mask_object,
        };
        self.external_images.push(record.clone());
        self.external_image_fingerprint = external_image_fingerprint(&self.external_images);
        Ok(record)
    }

    pub(crate) fn last_external_image(&self) -> Option<PdfExternalImageRecord> {
        self.external_images.last().cloned()
    }

    #[must_use]
    pub(crate) fn external_images(&self) -> &[PdfExternalImageRecord] {
        &self.external_images
    }
    pub(crate) fn reserve_raw_object(&mut self) -> Result<PdfRawObjectId, PdfObjectCapacityError> {
        let raw = (self.next_object <= MAX_OBJECT_ID)
            .then_some(self.next_object)
            .ok_or(PdfObjectCapacityError)?;
        let id = PdfRawObjectId::from_allocated(raw);
        self.next_object += 1;
        self.raw_objects.reserve(id);
        Ok(id)
    }

    pub(crate) fn reserve_form(&mut self) -> Result<(u32, u32), PdfObjectCapacityError> {
        let object = (self.next_object < MAX_OBJECT_ID)
            .then_some(self.next_object)
            .ok_or(PdfObjectCapacityError)?;
        let resource = self.next_form_resource;
        // pdfTeX reserves the Form XObject followed by its resource dictionary
        // in the shared object ledger. The latter may be represented inline by
        // the typed backend, but its identity remains observable through the
        // next form/object number and must therefore stay reserved.
        self.next_object += 2;
        self.next_form_resource = self
            .next_form_resource
            .checked_add(1)
            .ok_or(PdfObjectCapacityError)?;
        Ok((object, resource))
    }

    pub(crate) fn initialize_form(
        &mut self,
        identity: (u32, u32),
        box_list: DurableListId<G>,
        box_semantic_id: StateHashFragment,
        dimensions: (Scaled, Scaled, Scaled),
        options: (Option<PdfTokenParameter<G>>, Option<PdfTokenParameter<G>>),
        immediate: bool,
    ) -> Result<PdfFormRecord<G>, PdfObjectCapacityError> {
        let (object, resource) = identity;
        let (attr, resources) = options;
        let record = PdfFormRecord {
            object,
            resource,
            box_list,
            box_semantic_id,
            width: dimensions.0,
            height: dimensions.1,
            depth: dimensions.2,
            attr,
            resources,
            immediate,
        };
        self.form_fingerprint = append_form_fingerprint(self.form_fingerprint, &record);
        self.forms.push(record.clone());
        Ok(record)
    }

    #[must_use]
    pub(crate) fn form(&self, object: u32) -> Option<PdfFormRecord<G>> {
        self.forms
            .iter()
            .find(|form| form.object == object)
            .cloned()
    }

    pub(crate) fn forms(&self) -> impl ExactSizeIterator<Item = PdfFormRecord<G>> + '_ {
        self.forms.iter().cloned()
    }

    #[must_use]
    pub(crate) fn last_form(&self) -> u32 {
        self.forms.last().map_or(0, |form| form.object)
    }

    pub(crate) fn set_form_artifact(&mut self, object: u32, artifact: PdfFormArtifact) {
        let mut hasher = StateHasher::new_exact(0x7064_665f_666d_6172);
        self.form_artifact_fingerprint.apply(&mut hasher);
        hasher.u32(object);
        hasher.bytes(&artifact.bytes);
        if let Some((x, y)) = artifact.last_position {
            hasher.bool(true);
            hasher.i32(x.raw());
            hasher.i32(y.raw());
        } else {
            hasher.bool(false);
        }
        hasher.i32(artifact.snap_reference.0.raw());
        hasher.i32(artifact.snap_reference.1.raw());
        self.form_artifact_fingerprint = hasher.finish_fragment();
        self.form_artifacts.insert(object, artifact);
    }

    #[must_use]
    pub(crate) fn form_artifact(&self, object: u32) -> Option<&PdfFormArtifact> {
        self.form_artifacts.get(&object)
    }

    pub(crate) fn initialize_raw_object(
        &mut self,
        id: PdfRawObjectId,
        data: PdfRawObjectData<G>,
        immediate: bool,
    ) -> Result<(), PdfRawObjectInitializeError> {
        self.raw_objects.initialize(id, data, immediate)
    }

    #[must_use]
    pub(crate) fn raw_object(&self, id: PdfRawObjectId) -> Option<PdfRawObjectRecord<G>> {
        self.raw_objects.record(id)
    }

    pub(crate) fn reference_raw_object(
        &mut self,
        id: PdfRawObjectId,
    ) -> Result<(), PdfRawObjectInitializeError> {
        self.raw_objects.reference(id)
    }

    #[must_use]
    pub(crate) fn raw_objects(&self) -> &[PdfRawObjectRecord<G>] {
        self.raw_objects.records()
    }

    #[must_use]
    pub(crate) fn last_raw_object(&self) -> u32 {
        self.raw_objects.last_object()
    }

    pub(crate) fn append_document_fragment(
        &mut self,
        kind: PdfDocumentFragmentKind,
        value: PdfTokenParameter<G>,
    ) {
        self.document_fragments.append(kind, value);
    }

    pub(crate) fn document_fragments(
        &self,
        kind: PdfDocumentFragmentKind,
    ) -> impl Iterator<Item = TokenListId<G>> + '_ {
        self.document_fragments.values(kind)
    }

    pub(crate) fn set_catalog_open_action(
        &mut self,
        spec: PdfActionSpec<G>,
        fingerprint: StateHashFragment,
        destination_identity: Option<PdfDestinationIdentity>,
        structure_identity: Option<PdfDestinationIdentity>,
        thread_identity: Option<PdfDestinationIdentity>,
    ) -> Result<PdfActionRecord<G>, PdfObjectCapacityError> {
        debug_assert!(self.catalog_open_action.is_none());
        let id = self.reserve_document_object()?;
        let target_object = if let Some(identity) = thread_identity {
            Some(self.reserve_thread(identity)?.object())
        } else if let Some(identity) = destination_identity {
            Some(self.reserve_destination(identity, false)?.object())
        } else {
            spec.needs_target_object()
                .then(|| self.reserve_document_object())
                .transpose()?
        };
        let structure_object = if let Some(identity) = structure_identity {
            Some(self.reserve_destination(identity, true)?.object())
        } else {
            spec.needs_structure_object()
                .then(|| self.reserve_document_object())
                .transpose()?
        };
        if let PdfActionSpec::<G>::GoTo(PdfActionDestination {
            file: None,
            target: PdfActionTarget::Page { number, .. },
            ..
        }) = &spec
        {
            self.page_reservations.push(PdfPageReservation {
                number: *number,
                object: target_object.expect("internal page action reserves its page object"),
            });
            self.page_reservation_fingerprint =
                page_reservation_fingerprint(&self.page_reservations);
        }
        let record = PdfActionRecord::<G>::new(id, spec, target_object, structure_object);
        self.catalog_open_action = Some(record);
        self.action_fingerprint = fingerprint;
        Ok(record)
    }

    #[must_use]
    pub(crate) fn catalog_open_action(&self) -> Option<PdfActionRecord<G>> {
        self.catalog_open_action
    }

    fn reserved_page_object(&self, number: u32) -> Option<u32> {
        self.page_reservations
            .iter()
            .find(|reservation| reservation.number == number)
            .map(|reservation| reservation.object)
    }

    pub(crate) fn finalize_document_objects(
        &mut self,
        include_info: bool,
    ) -> Result<PdfDocumentObjectIds, PdfObjectCapacityError> {
        if self.document_objects.pages().is_none() {
            let id = self.reserve_document_object()?;
            self.document_objects.set_pages(id);
        }
        if self.document_objects.names().is_none()
            && (self
                .document_fragments(PdfDocumentFragmentKind::Names)
                .next()
                .is_some()
                || self
                    .destinations(false)
                    .iter()
                    .any(|record| matches!(record.identity(), PdfDestinationIdentity::Name(_))))
        {
            let id = self.reserve_document_object()?;
            self.document_objects.set_names(id);
        }
        if self.document_objects.catalog().is_none() {
            let id = self.reserve_document_object()?;
            self.document_objects.set_catalog(id);
        }
        if include_info && self.document_objects.info().is_none() {
            let id = self.reserve_document_object()?;
            self.document_objects.set_info(id);
        }
        Ok(self.document_objects)
    }

    fn reserve_document_object(&mut self) -> Result<u32, PdfObjectCapacityError> {
        let id = (self.next_object <= MAX_OBJECT_ID)
            .then_some(self.next_object)
            .ok_or(PdfObjectCapacityError)?;
        self.next_object += 1;
        Ok(id)
    }

    #[must_use]
    pub(crate) fn cursor(&self) -> PdfStateCursor<G> {
        PdfStateCursor {
            enabled: self.enabled,
            next_object: self.next_object,
            page_count: self.pages.len(),
            output_parameters: self.output_parameters,
            pk_mode: self.pk_mode,
            font_operation_count: self.font_operations.len(),
            font_resource_count: self.font_resources.len(),
            fingerprint: self.fingerprint,
            match_fingerprint: self.match_state.fingerprint,
            external_image_fingerprint: self.external_image_fingerprint,
            raw_object_fingerprint: self.raw_objects.fingerprint(),
            document_fragment_fingerprint: self.document_fragments.fingerprint(),
            document_objects: self.document_objects,
            catalog_open_action: self.catalog_open_action,
            action_fingerprint: self.action_fingerprint,
            page_reservation_fingerprint: self.page_reservation_fingerprint,
            space_font_name_count: self.space_font_names.len(),
            current_space_font_name: self.current_space_font_name,
            space_font_name_fingerprint: self.space_font_name_fingerprint,
            annotation_fingerprint: self.annotation_fingerprint,
            link_fingerprint: self.link_fingerprint,
            open_link_fingerprint: self.open_link_fingerprint,
            color_stack_fingerprint: self.color_stack_fingerprint,
            last_position: self.last_position,
            snap_reference: self.snap_reference,
            form_fingerprint: self.form_fingerprint,
            next_form_resource: self.next_form_resource,
            form_artifact_fingerprint: self.form_artifact_fingerprint,
            return_value: self.return_value,
            destination_fingerprint: self.destination_fingerprint,
            structure_destination_fingerprint: self.structure_destination_fingerprint,
            outline_fingerprint: self.outline_fingerprint,
            thread_fingerprint: self.thread_fingerprint,
        }
    }
    #[must_use]
    pub(crate) fn snapshot(&self) -> PdfStateSnapshot<G> {
        PdfStateSnapshot {
            cursor: self.cursor(),
            match_state: self.match_state.clone(),
            external_images: self.external_images.clone(),
            raw_objects: self.raw_objects.clone(),
            document_fragments: self.document_fragments.clone(),
            page_reservations: self.page_reservations.clone(),
            annotations: self.annotations.clone(),
            links: self.links.clone(),
            open_links: self.open_links.clone(),
            color_stacks: self.color_stacks.clone(),
            forms: self.forms.clone(),
            form_artifacts: self.form_artifacts.clone(),
            destinations: self.destinations.clone(),
            structure_destinations: self.structure_destinations.clone(),
            outlines: self.outlines.clone(),
            threads: self.threads.clone(),
        }
    }

    pub(crate) fn snapshot_is_retained(&self, snapshot: &PdfStateSnapshot<G>) -> bool {
        let cursor = snapshot.cursor;
        cursor.page_count <= self.pages.len()
            && cursor.font_operation_count <= self.font_operations.len()
            && cursor.font_resource_count <= self.font_resources.len()
            && cursor.space_font_name_count <= self.space_font_names.len()
    }

    pub(crate) fn snapshot_font_roots_are_live(
        &self,
        snapshot: &PdfStateSnapshot<G>,
        mut is_live: impl FnMut(FontId) -> bool,
    ) -> bool {
        let cursor = snapshot.cursor;
        self.font_operations[..cursor.font_operation_count]
            .iter()
            .all(|operation| match operation {
                PdfFontOperation::Attribute { font, .. }
                | PdfFontOperation::IncludeChars { font, .. }
                | PdfFontOperation::NoBuiltinToUnicode { font } => is_live(*font),
                PdfFontOperation::Map(_)
                | PdfFontOperation::MapFileContent { .. }
                | PdfFontOperation::GlyphToUnicode(_)
                | PdfFontOperation::Type1Program { .. }
                | PdfFontOperation::Encoding { .. }
                | PdfFontOperation::TrueTypeProgram { .. }
                | PdfFontOperation::PkFont { .. } => true,
            })
            && self.font_resources[..cursor.font_resource_count]
                .iter()
                .all(|record| is_live(record.font))
    }

    pub(crate) fn rollback(&mut self, snapshot: PdfStateSnapshot<G>) {
        let cursor = snapshot.cursor;
        assert!(
            cursor.page_count <= self.pages.len(),
            "PDF snapshot suffix was discarded"
        );
        self.pages.truncate(cursor.page_count);
        self.enabled = cursor.enabled;
        self.next_object = cursor.next_object;
        self.output_parameters = cursor.output_parameters;
        self.pk_mode = cursor.pk_mode;
        self.font_operations.truncate(cursor.font_operation_count);
        self.font_resources.truncate(cursor.font_resource_count);
        self.fingerprint = cursor.fingerprint;
        self.match_state = snapshot.match_state;
        self.external_images = snapshot.external_images;
        self.external_image_fingerprint = cursor.external_image_fingerprint;
        self.raw_objects = snapshot.raw_objects;
        self.document_fragments = snapshot.document_fragments;
        self.document_objects = cursor.document_objects;
        self.catalog_open_action = cursor.catalog_open_action;
        self.action_fingerprint = cursor.action_fingerprint;
        self.page_reservations = snapshot.page_reservations;
        self.page_reservation_fingerprint = cursor.page_reservation_fingerprint;
        self.space_font_names.truncate(cursor.space_font_name_count);
        self.space_font_name_lookup.clear();
        self.space_font_name_lookup.extend(
            self.space_font_names
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, name)| (name, index as u32)),
        );
        self.current_space_font_name = cursor.current_space_font_name;
        self.space_font_name_fingerprint = cursor.space_font_name_fingerprint;
        self.annotations = snapshot.annotations;
        self.annotation_fingerprint = cursor.annotation_fingerprint;
        self.links = snapshot.links;
        self.link_fingerprint = cursor.link_fingerprint;
        self.open_links = snapshot.open_links;
        self.open_link_fingerprint = cursor.open_link_fingerprint;
        self.color_stacks = snapshot.color_stacks;
        self.color_stack_fingerprint = cursor.color_stack_fingerprint;
        self.last_position = cursor.last_position;
        self.snap_reference = cursor.snap_reference;
        self.forms = snapshot.forms;
        self.form_fingerprint = cursor.form_fingerprint;
        self.next_form_resource = cursor.next_form_resource;
        self.form_artifacts = snapshot.form_artifacts;
        self.form_artifact_fingerprint = cursor.form_artifact_fingerprint;
        self.return_value = cursor.return_value;
        self.destinations = snapshot.destinations;
        self.destination_fingerprint = cursor.destination_fingerprint;
        self.structure_destinations = snapshot.structure_destinations;
        self.structure_destination_fingerprint = cursor.structure_destination_fingerprint;
        self.outlines = snapshot.outlines;
        self.outline_fingerprint = cursor.outline_fingerprint;
        self.threads = snapshot.threads;
        self.thread_fingerprint = cursor.thread_fingerprint;
    }

    pub(crate) fn set_match(
        &mut self,
        haystack: Vec<u8>,
        captures: Vec<Option<(u32, u32)>>,
        slot_count: u32,
        matched: bool,
    ) {
        let fingerprint = match_fingerprint(&haystack, &captures, slot_count, matched);
        self.match_state = PdfMatchState {
            haystack,
            captures,
            slot_count,
            matched,
            fingerprint,
        };
    }

    pub(crate) fn match_capture(&self, index: u32) -> Option<(u32, &[u8])> {
        if !self.match_state.matched || index >= self.match_state.slot_count {
            return None;
        }
        let &(start, end) = self.match_state.captures.get(index as usize)?.as_ref()?;
        let bytes = self
            .match_state
            .haystack
            .get(start as usize..end as usize)?;
        Some((start, bytes))
    }

    #[must_use]
    pub(crate) fn hash_fragment(&self) -> StateHashFragment {
        let cursor = self.cursor();
        StateHashFragment::from_exact_builder(PDF_STATE_DOMAIN, |hasher| {
            hasher.bool(cursor.enabled);
            hasher.u32(cursor.next_object);
            hasher.usize(cursor.page_count);
            hash_output_parameters(hasher, cursor.output_parameters);
            hasher.bool(cursor.pk_mode.is_some());
            if let Some(pk_mode) = cursor.pk_mode {
                hasher.bytes(&pk_mode.semantic_id.bytes());
            }
            hasher.usize(cursor.font_operation_count);
            hasher.usize(cursor.font_resource_count);
            cursor.fingerprint.apply(hasher);
            cursor.match_fingerprint.apply(hasher);
            cursor.external_image_fingerprint.apply(hasher);
            cursor.raw_object_fingerprint.apply(hasher);
            cursor.document_fragment_fingerprint.apply(hasher);
            cursor.action_fingerprint.apply(hasher);
            cursor.page_reservation_fingerprint.apply(hasher);
            hasher.usize(cursor.space_font_name_count);
            hasher.u32(cursor.current_space_font_name);
            cursor.space_font_name_fingerprint.apply(hasher);
            cursor.annotation_fingerprint.apply(hasher);
            cursor.link_fingerprint.apply(hasher);
            cursor.open_link_fingerprint.apply(hasher);
            cursor.form_fingerprint.apply(hasher);
            hasher.u32(cursor.next_form_resource);
            cursor.form_artifact_fingerprint.apply(hasher);
            hasher.i32(cursor.return_value);
            cursor.destination_fingerprint.apply(hasher);
            cursor.structure_destination_fingerprint.apply(hasher);
            cursor.outline_fingerprint.apply(hasher);
            cursor.thread_fingerprint.apply(hasher);
            hasher.bool(cursor.document_objects.pages().is_some());
            if let Some(id) = cursor.document_objects.pages() {
                hasher.u32(id);
            }
            hasher.bool(cursor.document_objects.names().is_some());
            if let Some(id) = cursor.document_objects.names() {
                hasher.u32(id);
            }
            hasher.bool(cursor.document_objects.catalog().is_some());
            if let Some(id) = cursor.document_objects.catalog() {
                hasher.u32(id);
            }
            hasher.bool(cursor.document_objects.info().is_some());
            if let Some(id) = cursor.document_objects.info() {
                hasher.u32(id);
            }
            cursor.color_stack_fingerprint.apply(hasher);
            hasher.i32(cursor.last_position.0.raw());
            hasher.i32(cursor.last_position.1.raw());
            hasher.i32(cursor.snap_reference.0.raw());
            hasher.i32(cursor.snap_reference.1.raw());
        })
    }

    pub(crate) const fn last_position(&self) -> (Scaled, Scaled) {
        self.last_position
    }

    /// Returns pdfTeX's session-global multi-purpose result value.
    #[must_use]
    pub(crate) const fn return_value(&self) -> i32 {
        self.return_value
    }

    /// Updates pdfTeX's session-global multi-purpose result value.
    pub(crate) const fn set_return_value(&mut self, value: i32) {
        self.return_value = value;
    }

    pub(crate) const fn snap_reference(&self) -> (Scaled, Scaled) {
        self.snap_reference
    }

    pub(crate) fn publish_traversal_positions(
        &mut self,
        last_position: Option<(Scaled, Scaled)>,
        snap_reference: (Scaled, Scaled),
    ) {
        if let Some(position) = last_position {
            self.last_position = position;
        }
        self.snap_reference = snap_reference;
    }

    pub(crate) fn form_color_rollback(&self) -> PdfFormColorRollback {
        PdfFormColorRollback(
            self.color_stacks
                .iter()
                .map(|stack| stack.form.clone())
                .collect(),
            self.color_stack_fingerprint,
        )
    }

    pub(crate) fn rollback_form_colors(&mut self, rollback: PdfFormColorRollback) {
        let PdfFormColorRollback(runtimes, fingerprint) = rollback;
        for (stack, runtime) in self.color_stacks.iter_mut().zip(runtimes) {
            stack.form = runtime;
        }
        self.color_stack_fingerprint = fingerprint;
    }

    fn ensure_default_color_stack(&mut self) {
        if !self.color_stacks.is_empty() {
            return;
        }
        let initial = b"0 g 0 G".to_vec();
        self.color_stacks.push(PdfColorStack {
            mode: PdfColorStackMode::Direct,
            restore_at_page_start: true,
            page: PdfColorStackRuntime {
                current: initial.clone(),
                pushed: Vec::new(),
            },
            form: PdfColorStackRuntime {
                current: initial,
                pushed: Vec::new(),
            },
        });
        self.color_stack_fingerprint = color_stack_fingerprint(&self.color_stacks);
    }

    pub(crate) fn allocate_color_stack(
        &mut self,
        mode: PdfColorStackMode,
        restore_at_page_start: bool,
        initial: Vec<u8>,
    ) -> Result<u32, PdfColorStackCapacityError> {
        self.ensure_default_color_stack();
        if self.color_stacks.len() >= MAX_COLOR_STACKS {
            return Err(PdfColorStackCapacityError);
        }
        let id = self.color_stacks.len() as u32;
        self.color_stacks.push(PdfColorStack {
            mode,
            restore_at_page_start,
            page: PdfColorStackRuntime {
                current: initial.clone(),
                pushed: Vec::new(),
            },
            form: PdfColorStackRuntime {
                current: initial,
                pushed: Vec::new(),
            },
        });
        self.color_stack_fingerprint = color_stack_fingerprint(&self.color_stacks);
        Ok(id)
    }

    pub(crate) fn has_color_stack(&mut self, id: u32) -> bool {
        self.ensure_default_color_stack();
        (id as usize) < self.color_stacks.len()
    }

    pub(crate) fn apply_color_stack(
        &mut self,
        id: u32,
        target: PdfColorStackTarget,
        action: &PdfColorStackAction,
    ) -> Result<PdfColorStackEmission, PdfColorStackApplyError> {
        self.ensure_default_color_stack();
        let Some(stack) = self.color_stacks.get_mut(id as usize) else {
            return Err(PdfColorStackApplyError::Unknown);
        };
        let runtime = match target {
            PdfColorStackTarget::Page => &mut stack.page,
            PdfColorStackTarget::Form => &mut stack.form,
        };
        match action {
            PdfColorStackAction::Set(bytes) => runtime.current.clone_from(bytes),
            PdfColorStackAction::Push(bytes) => {
                runtime
                    .pushed
                    .push(std::mem::replace(&mut runtime.current, bytes.clone()));
            }
            PdfColorStackAction::Pop => {
                runtime.current = runtime
                    .pushed
                    .pop()
                    .ok_or(PdfColorStackApplyError::Underflow)?;
            }
            PdfColorStackAction::Current => {}
        }
        let emission = PdfColorStackEmission {
            mode: stack.mode,
            payload: runtime.current.clone(),
        };
        self.color_stack_fingerprint = color_stack_fingerprint(&self.color_stacks);
        Ok(emission)
    }

    pub(crate) fn page_color_stack_restorations(&mut self) -> Vec<PdfColorStackEmission> {
        if !self.enabled {
            return Vec::new();
        }
        self.ensure_default_color_stack();
        self.color_stacks
            .iter()
            .enumerate()
            .filter(|(id, stack)| {
                stack.restore_at_page_start
                    && !stack.page.current.is_empty()
                    && !(*id == 0 && stack.page.current == b"0 g 0 G")
            })
            .map(|(_, stack)| PdfColorStackEmission {
                mode: stack.mode,
                payload: stack.page.current.clone(),
            })
            .collect()
    }
}

fn color_stack_fingerprint(stacks: &[PdfColorStack]) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_COLOR_STACK_DOMAIN);
    hasher.usize(stacks.len());
    for stack in stacks {
        hasher.u8(match stack.mode {
            PdfColorStackMode::Origin => 0,
            PdfColorStackMode::Page => 1,
            PdfColorStackMode::Direct => 2,
        });
        hasher.bool(stack.restore_at_page_start);
        for runtime in [&stack.page, &stack.form] {
            hasher.bytes(&runtime.current);
            hasher.usize(runtime.pushed.len());
            for bytes in &runtime.pushed {
                hasher.bytes(bytes);
            }
        }
    }
    hasher.finish_fragment()
}

fn external_image_base_fingerprint() -> StateHashFragment {
    StateHasher::new_exact(PDF_EXTERNAL_IMAGE_DOMAIN).finish_fragment()
}

fn page_reservation_fingerprint(reservations: &[PdfPageReservation]) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_7067_7273);
    hasher.usize(reservations.len());
    for reservation in reservations {
        hasher.u32(reservation.number);
        hasher.u32(reservation.object);
    }
    hasher.finish_fragment()
}

fn annotation_fingerprint<G>(_records: &[PdfAnnotationRecord<G>]) -> StateHashFragment {
    StateHasher::new_exact(0x7064_665f_616e_6e6f).finish_fragment()
}

fn append_annotation_reservation_fingerprint(
    previous: StateHashFragment,
    object: u32,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_616e_6e6f);
    previous.apply(&mut hasher);
    hasher.u8(0);
    hasher.u32(object);
    hasher.finish_fragment()
}

fn append_annotation_data_fingerprint(
    previous: StateHashFragment,
    object: u32,
    dimensions: PdfAnnotationDimensions,
    entries_semantic_id: StateHashFragment,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_616e_6e6f);
    previous.apply(&mut hasher);
    hasher.u8(1);
    hasher.u32(object);
    hash_annotation_dimensions(&mut hasher, dimensions);
    hasher.bytes(&entries_semantic_id.bytes());
    hasher.finish_fragment()
}

fn append_link_fingerprint<G>(
    previous: StateHashFragment,
    record: &PdfLinkRecord<G>,
    attributes_semantic_id: StateHashFragment,
    action_semantic_id: StateHashFragment,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_6c69_6e6b);
    previous.apply(&mut hasher);
    hasher.u32(record.object());
    hash_annotation_dimensions(&mut hasher, record.dimensions());
    hasher.bytes(&attributes_semantic_id.bytes());
    hasher.bytes(&action_semantic_id.bytes());
    hasher.finish_fragment()
}

fn open_link_fingerprint<G>(links: &[PdfOpenLink<G>]) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_6f70_6c6e);
    hasher.usize(links.len());
    for link in links {
        hasher.u32(link.record.object());
        hasher.u32(link.nesting_depth);
    }
    hasher.finish_fragment()
}

fn destination_fingerprint(records: &[PdfDestinationRecord], structure: bool) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(if structure {
        0x7064_665f_7364_7374
    } else {
        0x7064_665f_6465_7374
    });
    hasher.usize(records.len());
    for record in records {
        match record.identity() {
            PdfDestinationIdentity::Name(name) => {
                hasher.u8(0);
                hasher.bytes(name);
            }
            PdfDestinationIdentity::Number(number) => {
                hasher.u8(1);
                hasher.u32(*number);
            }
        }
        hasher.u32(record.object());
        hasher.bool(record.defined());
        hasher.bool(record.structure().is_some());
        if let Some(target) = record.structure() {
            hasher.u32(target);
        }
    }
    hasher.finish_fragment()
}

fn outline_fingerprint<G>(_records: &[PdfOutlineRecord<G>]) -> StateHashFragment {
    StateHasher::new_exact(0x7064_665f_6f75_746c).finish_fragment()
}

fn thread_fingerprint(records: &[PdfThreadRecord]) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_7468_7264);
    for record in records {
        match record.identity() {
            PdfDestinationIdentity::Name(name) => {
                hasher.u8(0);
                hasher.bytes(name);
            }
            PdfDestinationIdentity::Number(number) => {
                hasher.u8(1);
                hasher.u32(*number);
            }
        }
        hasher.u32(record.object());
        for bead in record.beads() {
            hasher.u32(bead.bead_object());
            hasher.u32(bead.rectangle_object());
        }
    }
    hasher.finish_fragment()
}

fn append_outline_fingerprint<G>(
    previous: StateHashFragment,
    record: &PdfOutlineRecord<G>,
    attributes_semantic_id: StateHashFragment,
    action_semantic_id: StateHashFragment,
    title_semantic_id: StateHashFragment,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_6f75_746c);
    previous.apply(&mut hasher);
    hasher.u32(record.action_object());
    hasher.u32(record.item_object());
    hasher.u32(record.title_object());
    hasher.bytes(&attributes_semantic_id.bytes());
    hasher.bytes(&action_semantic_id.bytes());
    hasher.i32(record.count());
    hasher.bytes(&title_semantic_id.bytes());
    hasher.finish_fragment()
}

fn hash_annotation_dimensions(hasher: &mut StateHasher, dimensions: PdfAnnotationDimensions) {
    for value in [dimensions.width, dimensions.height, dimensions.depth] {
        hasher.bool(value.is_some());
        if let Some(value) = value {
            hasher.i32(value.raw());
        }
    }
}

fn external_image_fingerprint(images: &[PdfExternalImageRecord]) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_EXTERNAL_IMAGE_DOMAIN);
    hasher.usize(images.len());
    for record in images {
        hasher.u32(record.id.raw());
        hasher.bytes(&record.identity.bytes());
        match record.metadata {
            PdfExternalImageMetadata::PdfPage {
                page_box,
                rotation,
                page,
                total_pages,
                has_page_group,
                pdf_version,
            } => {
                hasher.u8(0);
                hasher.i32(page_box.left.raw());
                hasher.i32(page_box.bottom.raw());
                hasher.i32(page_box.right.raw());
                hasher.i32(page_box.top.raw());
                hasher.u8(rotation as u8);
                hasher.u32(page);
                hasher.u32(total_pages);
                hasher.bool(has_page_group);
                hasher.u8(pdf_version.0);
                hasher.u8(pdf_version.1);
            }
            PdfExternalImageMetadata::Raster(metadata) => {
                hasher.u8(1);
                hasher.u8(metadata.format as u8);
                hasher.u32(metadata.width);
                hasher.u32(metadata.height);
                hasher.u8(metadata.bits_per_component);
                hasher.u8(metadata.color_space as u8);
                hasher.bool(metadata.alpha);
                hasher.bool(metadata.png_color_type.is_some());
                if let Some(color_type) = metadata.png_color_type {
                    hasher.u8(color_type);
                }
            }
        }
        hasher.i32(record.dimensions.width.raw());
        hasher.i32(record.dimensions.height.raw());
        hasher.i32(record.dimensions.depth.raw());
        hasher.i32(record.color_space_object);
        hasher.bytes(&record.bytes);
        hasher.bool(record.mask_object.is_some());
        if let Some(mask) = record.mask_object {
            hasher.u32(mask);
        }
    }
    hasher.finish_fragment()
}

fn append_font_resource_fingerprint(
    previous: StateHashFragment,
    record: PdfFontResourceRecord,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_FONT_DOMAIN);
    previous.apply(&mut hasher);
    hasher.tag(5);
    hasher.u32(record.font.raw());
    hasher.bytes(&record.source_identity.bytes());
    hasher.u32(record.resource_number);
    hasher.u32(record.object_number);
    hasher.bytes(&record.identity.tfm_content_hash());
    hasher.bool(record.identity.program_identity().is_some());
    if let Some(identity) = record.identity.program_identity() {
        hasher.bytes(&identity.bytes());
    }
    hasher.finish_fragment()
}

fn append_font_fingerprint(
    previous: StateHashFragment,
    operation: &PdfFontOperation,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_FONT_DOMAIN);
    previous.apply(&mut hasher);
    match operation {
        PdfFontOperation::Map(PdfFontMapOperation::BlockDefault) => {
            hasher.tag(12);
        }
        PdfFontOperation::Map(PdfFontMapOperation::File(file)) => {
            hasher.tag(0);
            hasher.tag(file.directive as u8);
            hasher.bytes(&file.logical_name);
        }
        PdfFontOperation::Map(PdfFontMapOperation::Line(line)) => {
            hasher.tag(1);
            hasher.tag(line.directive as u8);
            hasher.bytes(&line.tex_name);
            hasher.bool(line.postscript_name.is_some());
            if let Some(name) = &line.postscript_name {
                hasher.bytes(name);
            }
            for instruction in &line.special_instructions {
                hasher.bytes(instruction);
            }
            for encoding in &line.encoding_files {
                hasher.bytes(encoding);
            }
            for header in &line.header_files {
                hasher.bytes(header);
            }
            hasher.bool(line.font_file.is_some());
            if let Some(file) = &line.font_file {
                hasher.bytes(file);
            }
            hasher.tag(line.program as u8);
        }
        PdfFontOperation::MapFileContent { logical_name, map } => {
            hasher.tag(11);
            hasher.bytes(logical_name);
            for entry in map.entries() {
                hasher.bytes(&entry.tex_name);
                hasher.bool(entry.postscript_name.is_some());
                if let Some(name) = &entry.postscript_name {
                    hasher.bytes(name);
                }
                for instruction in &entry.special_instructions {
                    hasher.bytes(instruction);
                }
                for encoding in &entry.encoding_files {
                    hasher.bytes(encoding);
                }
                for header in &entry.header_files {
                    hasher.bytes(header);
                }
                hasher.bool(entry.font_file.is_some());
                if let Some(file) = &entry.font_file {
                    hasher.bytes(file);
                }
                hasher.tag(entry.program as u8);
            }
        }
        PdfFontOperation::Attribute { font, bytes } => {
            hasher.tag(2);
            hasher.u32(font.raw());
            hasher.bytes(bytes);
        }
        PdfFontOperation::IncludeChars { font, chars } => {
            hasher.tag(3);
            hasher.u32(font.raw());
            hasher.bytes(chars);
        }
        PdfFontOperation::GlyphToUnicode(mapping) => {
            hasher.tag(8);
            hasher.bool(mapping.tfm_name.is_some());
            if let Some(name) = &mapping.tfm_name {
                hasher.bytes(name);
            }
            hasher.bytes(&mapping.glyph_name);
            for value in &mapping.unicode {
                hasher.u32(*value);
            }
        }
        PdfFontOperation::NoBuiltinToUnicode { font } => {
            hasher.tag(9);
            hasher.u32(font.raw());
        }
        PdfFontOperation::Type1Program {
            logical_name,
            program,
        } => {
            hasher.tag(4);
            hasher.bytes(logical_name);
            hasher.bytes(&program.identity().bytes());
        }
        PdfFontOperation::Encoding {
            logical_name,
            encoding,
        } => {
            hasher.tag(6);
            hasher.bytes(logical_name);
            hasher.bytes(encoding.name());
            for name in encoding.glyph_names() {
                hasher.bytes(name);
            }
        }
        PdfFontOperation::TrueTypeProgram {
            logical_name,
            program,
        } => {
            hasher.tag(7);
            hasher.bytes(logical_name);
            hasher.bytes(&program.identity().bytes());
        }
        PdfFontOperation::PkFont { request, font } => {
            hasher.tag(10);
            hasher.bytes(request.tex_name());
            hasher.u32(request.dpi());
            hasher.bytes(request.mode());
            hasher.bytes(&font.identity().bytes());
        }
    }
    hasher.finish_fragment()
}

fn match_fingerprint(
    haystack: &[u8],
    captures: &[Option<(u32, u32)>],
    slot_count: u32,
    matched: bool,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_6d61_7463);
    hasher.bytes(haystack);
    hasher.u32(slot_count);
    hasher.bool(matched);
    hasher.usize(captures.len());
    for capture in captures {
        match capture {
            Some((start, end)) => {
                hasher.bool(true);
                hasher.u32(*start);
                hasher.u32(*end);
            }
            None => hasher.bool(false),
        }
    }
    hasher.finish_fragment()
}

fn base_fingerprint(enabled: bool) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_STATE_DOMAIN);
    hasher.bool(enabled);
    hasher.u32(FIRST_DYNAMIC_OBJECT);
    hasher.finish_fragment()
}

fn space_font_name_fingerprint(name: &[u8]) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_7370_666e);
    hasher.bytes(name);
    hasher.finish_fragment()
}

fn freeze_fingerprint(
    previous: StateHashFragment,
    parameters: PdfOutputParameters,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_PAGE_DOMAIN);
    previous.apply(&mut hasher);
    hash_output_parameters(&mut hasher, Some(parameters));
    hasher.finish_fragment()
}

fn append_fingerprint<G>(
    previous: StateHashFragment,
    record: &PdfPageRecord<G>,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_PAGE_DOMAIN);
    previous.apply(&mut hasher);
    hasher.bytes(&record.artifact.bytes());
    hasher.u32(record.resources_object);
    hasher.u32(record.contents_object);
    hasher.u32(record.page_object);
    hasher.i32(record.parameters.h_origin.raw());
    hasher.i32(record.parameters.v_origin.raw());
    hasher.i32(record.parameters.width.raw());
    hasher.i32(record.parameters.height.raw());
    hasher.bytes(&record.parameters.page_attr.semantic_id.bytes());
    hasher.bytes(&record.parameters.resources.semantic_id.bytes());
    hasher.i32(record.parameters.omit_procset);
    hasher.u32(record.parameters.space_font_name);
    hasher.finish_fragment()
}

fn append_form_fingerprint<G>(
    previous: StateHashFragment,
    record: &PdfFormRecord<G>,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_FORM_DOMAIN);
    previous.apply(&mut hasher);
    hasher.u32(record.object);
    hasher.u32(record.resource);
    hasher.bytes(&record.box_semantic_id.bytes());
    hasher.i32(record.width.raw());
    hasher.i32(record.height.raw());
    hasher.i32(record.depth.raw());
    for value in [&record.attr, &record.resources] {
        hasher.bool(value.is_some());
        if let Some(value) = value {
            hasher.bytes(&value.semantic_id.bytes());
        }
    }
    hasher.bool(record.immediate);
    hasher.finish_fragment()
}

fn freeze_pk_mode_fingerprint<G>(
    previous: StateHashFragment,
    mode: &PdfTokenParameter<G>,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_PAGE_DOMAIN);
    previous.apply(&mut hasher);
    hasher.bytes(&mode.semantic_id.bytes());
    hasher.finish_fragment()
}

fn hash_output_parameters(hasher: &mut StateHasher, parameters: Option<PdfOutputParameters>) {
    hasher.bool(parameters.is_some());
    if let Some(parameters) = parameters {
        hasher.i32(parameters.output);
        hasher.i32(parameters.major_version);
        hasher.i32(parameters.minor_version);
        hasher.i32(parameters.compress_level);
        hasher.i32(parameters.object_compress_level);
        hasher.i32(parameters.decimal_digits);
        hasher.i32(parameters.gamma);
        hasher.i32(parameters.image_gamma);
        hasher.i32(parameters.image_hicolor);
        hasher.i32(parameters.image_apply_gamma);
        hasher.i32(parameters.draft_mode);
        hasher.i32(parameters.inclusion_copy_fonts);
        hasher.i32(parameters.pk_resolution);
        hasher.i32(parameters.unique_resource_names);
    }
}

#[cfg(test)]
mod tests;
