//! Checkpointed pdfTeX document allocation ledger.

mod action;
mod annotation;
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
pub use destination::{PdfDestinationDefinition, PdfDestinationIdentity, PdfDestinationRecord};
use document::PdfDocumentFragments;
pub use document::{PdfDocumentFragmentKind, PdfDocumentObjectIds};
use object::PdfRawObjects;
pub use object::{
    PdfRawObjectData, PdfRawObjectId, PdfRawObjectInitializeError, PdfRawObjectRecord,
};
pub use outline::PdfOutlineRecord;
pub use thread::{PdfThreadBeadRecord, PdfThreadRecord};

use std::sync::Arc;

use crate::ContentHash;
use crate::ids::{FontId, NodeListId, TokenListId};
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
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
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
pub struct PdfFormColorRollback(Vec<PdfColorStackRuntime>, u64);

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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfPageBox {
    pub left: Scaled,
    pub bottom: Scaled,
    pub right: Scaled,
    pub top: Scaled,
}

/// Metadata retained after host-neutral external-image validation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfExternalImageMetadata {
    PdfPage {
        page_box: PdfPageBox,
        page: u32,
        has_page_group: bool,
        pdf_version: (u8, u8),
    },
    Raster(PdfRasterImageMetadata),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfRasterFormat {
    Jpeg,
    Png,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfRasterColorSpace {
    Gray,
    Rgb,
    Cmyk,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    pub bytes: Arc<[u8]>,
}

/// Final dimensions recorded by `\pdfximage` after optional scaling.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfExternalImageDimensions {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
}

impl PdfExternalImageMetadata {
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
    bytes: Arc<[u8]>,
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
    tfm_content_hash: [u8; 32],
    program_identity: Option<[u8; 32]>,
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
    File(tex_fonts::PdfFontMapFile),
    Line(tex_fonts::PdfFontMapEntry),
}

/// One validated `\pdfglyphtounicode` mapping. A `tfm:` prefix scopes the
/// mapping to one TeX metric name; otherwise it is global across fonts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfGlyphToUnicode {
    pub tfm_name: Option<Vec<u8>>,
    pub glyph_name: Vec<u8>,
    pub unicode: Vec<u32>,
}

/// An append-only font-output mutation. The log makes snapshots cheap and
/// ensures rollback discards the exact suffix produced after a checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PdfFontOperation {
    Map(PdfFontMapOperation),
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfTokenParameter {
    pub(crate) tokens: TokenListId,
    pub(crate) semantic_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfPageParameters {
    pub(crate) h_origin: Scaled,
    pub(crate) v_origin: Scaled,
    pub(crate) width: Scaled,
    pub(crate) height: Scaled,
    pub(crate) link_margin: Scaled,
    pub(crate) page_attr: PdfTokenParameter,
    pub(crate) resources: PdfTokenParameter,
    /// Raw `\pdfomitprocset` value captured when this page is shipped.
    pub(crate) omit_procset: i32,
    pub(crate) space_font_name: u32,
}

/// Stable object identities assigned to one committed PDF page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfPageRecord {
    artifact: ContentHash,
    resources_object: u32,
    contents_object: u32,
    page_object: u32,
    parameters: PdfPageParameters,
}

/// Immutable captured box and canonical identities for one `\pdfxform`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfFormRecord {
    object: u32,
    resource: u32,
    box_list: NodeListId,
    box_semantic_id: u64,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    attr: Option<PdfTokenParameter>,
    resources: Option<PdfTokenParameter>,
    immediate: bool,
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

impl PdfFormRecord {
    #[must_use]
    pub const fn object(self) -> u32 {
        self.object
    }
    #[must_use]
    pub const fn resource(self) -> u32 {
        self.resource
    }
    #[must_use]
    pub const fn box_list(self) -> NodeListId {
        self.box_list
    }
    #[must_use]
    pub const fn width(self) -> Scaled {
        self.width
    }
    #[must_use]
    pub const fn height(self) -> Scaled {
        self.height
    }
    #[must_use]
    pub const fn depth(self) -> Scaled {
        self.depth
    }
    #[must_use]
    pub const fn attr(self) -> Option<TokenListId> {
        match self.attr {
            Some(v) => Some(v.tokens),
            None => None,
        }
    }
    #[must_use]
    pub const fn resources(self) -> Option<TokenListId> {
        match self.resources {
            Some(v) => Some(v.tokens),
            None => None,
        }
    }
    #[must_use]
    pub const fn immediate(self) -> bool {
        self.immediate
    }
}

impl PdfPageRecord {
    #[must_use]
    pub const fn artifact(self) -> ContentHash {
        self.artifact
    }
    #[must_use]
    pub const fn resources_object(self) -> u32 {
        self.resources_object
    }
    #[must_use]
    pub const fn contents_object(self) -> u32 {
        self.contents_object
    }
    #[must_use]
    pub const fn page_object(self) -> u32 {
        self.page_object
    }
    #[must_use]
    pub const fn h_origin(self) -> Scaled {
        self.parameters.h_origin
    }
    #[must_use]
    pub const fn v_origin(self) -> Scaled {
        self.parameters.v_origin
    }
    #[must_use]
    pub const fn width(self) -> Scaled {
        self.parameters.width
    }
    #[must_use]
    pub const fn height(self) -> Scaled {
        self.parameters.height
    }
    #[must_use]
    pub const fn link_margin(self) -> Scaled {
        self.parameters.link_margin
    }
    #[must_use]
    pub const fn page_attr(self) -> TokenListId {
        self.parameters.page_attr.tokens
    }
    #[must_use]
    pub const fn resources(self) -> TokenListId {
        self.parameters.resources.tokens
    }
    #[must_use]
    pub const fn omit_procset(self) -> i32 {
        self.parameters.omit_procset
    }
    #[must_use]
    pub const fn space_font_name_id(self) -> u32 {
        self.parameters.space_font_name
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfStateCursor {
    enabled: bool,
    next_object: u32,
    page_count: usize,
    output_parameters: Option<PdfOutputParameters>,
    pk_mode: Option<PdfTokenParameter>,
    font_operation_count: usize,
    font_resource_count: usize,
    fingerprint: u64,
    match_fingerprint: u64,
    external_image_fingerprint: u64,
    raw_object_fingerprint: u64,
    document_fragment_fingerprint: u64,
    document_objects: PdfDocumentObjectIds,
    catalog_open_action: Option<PdfActionRecord>,
    action_fingerprint: u64,
    page_reservation_fingerprint: u64,
    space_font_name_count: usize,
    current_space_font_name: u32,
    space_font_name_fingerprint: u64,
    annotation_fingerprint: u64,
    link_fingerprint: u64,
    open_link_fingerprint: u64,
    color_stack_fingerprint: u64,
    last_position: (Scaled, Scaled),
    snap_reference: (Scaled, Scaled),
    form_fingerprint: u64,
    next_form_resource: u32,
    form_artifact_fingerprint: u64,
    return_value: i32,
    destination_fingerprint: u64,
    structure_destination_fingerprint: u64,
    outline_fingerprint: u64,
    thread_fingerprint: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct PdfStateSnapshot {
    cursor: PdfStateCursor,
    match_state: Arc<PdfMatchState>,
    external_images: Arc<Vec<PdfExternalImageRecord>>,
    raw_objects: PdfRawObjects,
    document_fragments: PdfDocumentFragments,
    page_reservations: Arc<Vec<PdfPageReservation>>,
    annotations: Arc<Vec<PdfAnnotationRecord>>,
    links: Arc<Vec<PdfLinkRecord>>,
    open_links: Arc<Vec<PdfOpenLink>>,
    color_stacks: Arc<Vec<PdfColorStack>>,
    forms: Arc<Vec<PdfFormRecord>>,
    form_artifacts: Arc<BTreeMap<u32, PdfFormArtifact>>,
    destinations: Arc<Vec<PdfDestinationRecord>>,
    structure_destinations: Arc<Vec<PdfDestinationRecord>>,
    outlines: Arc<Vec<PdfOutlineRecord>>,
    threads: Arc<Vec<PdfThreadRecord>>,
}

#[derive(Clone, Debug, Default)]
struct PdfMatchState {
    haystack: Vec<u8>,
    captures: Vec<Option<(u32, u32)>>,
    slot_count: u32,
    matched: bool,
    fingerprint: u64,
}

/// Live append-only PDF allocation state owned by one Universe timeline.
#[derive(Clone, Debug)]
pub(crate) struct PdfState {
    enabled: bool,
    next_object: u32,
    pages: Vec<PdfPageRecord>,
    output_parameters: Option<PdfOutputParameters>,
    pk_mode: Option<PdfTokenParameter>,
    font_operations: Vec<PdfFontOperation>,
    font_resources: Vec<PdfFontResourceRecord>,
    fingerprint: u64,
    match_state: Arc<PdfMatchState>,
    external_images: Arc<Vec<PdfExternalImageRecord>>,
    external_image_fingerprint: u64,
    raw_objects: PdfRawObjects,
    document_fragments: PdfDocumentFragments,
    document_objects: PdfDocumentObjectIds,
    catalog_open_action: Option<PdfActionRecord>,
    action_fingerprint: u64,
    page_reservations: Arc<Vec<PdfPageReservation>>,
    page_reservation_fingerprint: u64,
    space_font_names: Vec<Vec<u8>>,
    space_font_name_lookup: BTreeMap<Vec<u8>, u32>,
    current_space_font_name: u32,
    space_font_name_fingerprint: u64,
    annotations: Arc<Vec<PdfAnnotationRecord>>,
    annotation_fingerprint: u64,
    links: Arc<Vec<PdfLinkRecord>>,
    link_fingerprint: u64,
    open_links: Arc<Vec<PdfOpenLink>>,
    open_link_fingerprint: u64,
    color_stacks: Arc<Vec<PdfColorStack>>,
    color_stack_fingerprint: u64,
    last_position: (Scaled, Scaled),
    snap_reference: (Scaled, Scaled),
    forms: Arc<Vec<PdfFormRecord>>,
    form_fingerprint: u64,
    next_form_resource: u32,
    form_artifacts: Arc<BTreeMap<u32, PdfFormArtifact>>,
    form_artifact_fingerprint: u64,
    return_value: i32,
    destinations: Arc<Vec<PdfDestinationRecord>>,
    destination_fingerprint: u64,
    structure_destinations: Arc<Vec<PdfDestinationRecord>>,
    structure_destination_fingerprint: u64,
    outlines: Arc<Vec<PdfOutlineRecord>>,
    outline_fingerprint: u64,
    threads: Arc<Vec<PdfThreadRecord>>,
    thread_fingerprint: u64,
}

impl Default for PdfState {
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
            match_state: Arc::new(PdfMatchState::default()),
            external_images: Arc::new(Vec::new()),
            external_image_fingerprint: external_image_base_fingerprint(),
            raw_objects: PdfRawObjects::default(),
            document_fragments: PdfDocumentFragments::default(),
            document_objects: PdfDocumentObjectIds::default(),
            catalog_open_action: None,
            action_fingerprint: StateHasher::new(0x7064_665f_6163_746e).finish(),
            page_reservations: Arc::new(Vec::new()),
            page_reservation_fingerprint: StateHasher::new(0x7064_665f_7067_7273).finish(),
            space_font_names: vec![default_space_font.clone()],
            space_font_name_lookup: BTreeMap::from([(default_space_font.clone(), 0)]),
            current_space_font_name: 0,
            space_font_name_fingerprint: space_font_name_fingerprint(&default_space_font),
            annotations: Arc::new(Vec::new()),
            annotation_fingerprint: annotation_fingerprint(&[]),
            links: Arc::new(Vec::new()),
            link_fingerprint: StateHasher::new(0x7064_665f_6c69_6e6b).finish(),
            open_links: Arc::new(Vec::new()),
            open_link_fingerprint: open_link_fingerprint(&[]),
            color_stacks: Arc::new(Vec::new()),
            color_stack_fingerprint: color_stack_fingerprint(&[]),
            last_position: (Scaled::from_raw(0), Scaled::from_raw(0)),
            snap_reference: (Scaled::from_raw(0), Scaled::from_raw(0)),
            forms: Arc::new(Vec::new()),
            form_fingerprint: StateHasher::new(PDF_FORM_DOMAIN).finish(),
            next_form_resource: 1,
            form_artifacts: Arc::new(BTreeMap::new()),
            form_artifact_fingerprint: StateHasher::new(0x7064_665f_666d_6172).finish(),
            return_value: 0,
            destinations: Arc::new(Vec::new()),
            destination_fingerprint: destination_fingerprint(&[], false),
            structure_destinations: Arc::new(Vec::new()),
            structure_destination_fingerprint: destination_fingerprint(&[], true),
            outlines: Arc::new(Vec::new()),
            outline_fingerprint: outline_fingerprint(&[]),
            threads: Arc::new(Vec::new()),
            thread_fingerprint: thread_fingerprint(&[]),
        }
    }
}

impl PdfState {
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
    pub(crate) fn pages(&self) -> &[PdfPageRecord] {
        &self.pages
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
    #[must_use]
    pub(crate) fn is_format_empty(&self) -> bool {
        self.pages.is_empty()
            && self.next_object == FIRST_DYNAMIC_OBJECT
            && self.output_parameters.is_none()
            && self.pk_mode.is_none()
            && self.font_operations.is_empty()
            && self.font_resources.is_empty()
            && self.external_images.is_empty()
            && self.raw_objects.is_empty()
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
            && self.forms.is_empty()
            && self.threads.is_empty()
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
        page: PdfPageParameters,
        pk_mode: PdfTokenParameter,
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
            self.pk_mode = Some(pk_mode);
            self.fingerprint = freeze_pk_mode_fingerprint(self.fingerprint, pk_mode);
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
        self.pages.push(record);
        self.fingerprint = append_fingerprint(self.fingerprint, record);
    }

    #[must_use]
    pub(crate) const fn output_parameters(&self) -> Option<PdfOutputParameters> {
        self.output_parameters
    }

    #[must_use]
    pub(crate) const fn pk_mode(&self) -> Option<TokenListId> {
        match self.pk_mode {
            Some(mode) => Some(mode.tokens),
            None => None,
        }
    }

    pub(crate) fn push_font_map(&mut self, operation: PdfFontMapOperation) {
        self.push_font_operation(PdfFontOperation::Map(operation));
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
        tfm_content_hash: [u8; 32],
        program_identity: Option<[u8; 32]>,
    ) -> Result<PdfFontResourceRecord, PdfObjectCapacityError> {
        if let Some(record) = self
            .font_resources
            .iter()
            .copied()
            .find(|record| record.font == font)
        {
            return Ok(record);
        }
        if let Some(record) = self.font_resources.iter().copied().find(|record| {
            record.tfm_content_hash == tfm_content_hash
                && record.program_identity == program_identity
        }) {
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
            tfm_content_hash,
            program_identity,
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

    pub(crate) fn reserve_annotation(
        &mut self,
    ) -> Result<PdfAnnotationRecord, PdfObjectCapacityError> {
        let object = self.reserve_document_object()?;
        let record = PdfAnnotationRecord::reserved(object);
        Arc::make_mut(&mut self.annotations).push(record);
        self.annotation_fingerprint =
            append_annotation_reservation_fingerprint(self.annotation_fingerprint, object);
        Ok(record)
    }

    pub(crate) fn initialize_annotation(
        &mut self,
        object: u32,
        data: PdfAnnotationData,
        entries_semantic_id: u64,
    ) -> Result<PdfAnnotationRecord, PdfAnnotationInitializeError> {
        let records = Arc::make_mut(&mut self.annotations);
        let record = records
            .iter_mut()
            .find(|record| record.object() == object)
            .ok_or(PdfAnnotationInitializeError(object))?;
        record
            .initialize(data)
            .map_err(|()| PdfAnnotationInitializeError(object))?;
        self.annotation_fingerprint = append_annotation_data_fingerprint(
            self.annotation_fingerprint,
            object,
            data.dimensions,
            entries_semantic_id,
        );
        Ok(*record)
    }

    #[must_use]
    pub(crate) fn annotations(&self) -> &[PdfAnnotationRecord] {
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
        Arc::make_mut(records).push(record.clone());
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
        let record = Arc::make_mut(records)
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
                Arc::make_mut(&mut self.threads).push(PdfThreadRecord::new(identity, object));
                self.threads.len() - 1
            }
        };
        let bead = PdfThreadBeadRecord::new(
            self.reserve_document_object()?,
            self.reserve_document_object()?,
        );
        let threads = Arc::make_mut(&mut self.threads);
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
        let threads = Arc::make_mut(&mut self.threads);
        threads.push(record.clone());
        self.thread_fingerprint = thread_fingerprint(threads);
        Ok(record)
    }

    pub(crate) fn threads(&self) -> &[PdfThreadRecord] {
        &self.threads
    }

    pub(crate) fn create_outline(
        &mut self,
        attributes: TokenListId,
        action: PdfActionSpec,
        count: i32,
        title: TokenListId,
        semantic_ids: [u64; 3],
    ) -> Result<PdfOutlineRecord, PdfObjectCapacityError> {
        let action_object = self.reserve_document_object()?;
        let item_object = self.reserve_document_object()?;
        let title_object = self.reserve_document_object()?;
        let record = PdfOutlineRecord::new(
            action_object,
            item_object,
            title_object,
            attributes,
            action,
            count,
            title,
        );
        Arc::make_mut(&mut self.outlines).push(record);
        self.outline_fingerprint = append_outline_fingerprint(
            self.outline_fingerprint,
            record,
            semantic_ids[0],
            semantic_ids[1],
            semantic_ids[2],
        );
        Ok(record)
    }

    pub(crate) fn outlines(&self) -> &[PdfOutlineRecord] {
        &self.outlines
    }

    #[must_use]
    pub(crate) fn last_annotation(&self) -> u32 {
        self.annotations.last().map_or(0, |record| record.object())
    }

    pub(crate) fn create_link(
        &mut self,
        dimensions: PdfAnnotationDimensions,
        attributes: TokenListId,
        action: PdfActionSpec,
        attributes_semantic_id: u64,
        action_semantic_id: u64,
        nesting_depth: u32,
    ) -> Result<PdfLinkRecord, PdfObjectCapacityError> {
        let object = self.reserve_document_object()?;
        let record = PdfLinkRecord::new(object, dimensions, attributes, action);
        Arc::make_mut(&mut self.links).push(record);
        self.link_fingerprint = append_link_fingerprint(
            self.link_fingerprint,
            record,
            attributes_semantic_id,
            action_semantic_id,
        );
        Arc::make_mut(&mut self.open_links).push(PdfOpenLink {
            record,
            nesting_depth,
        });
        self.open_link_fingerprint = open_link_fingerprint(&self.open_links);
        Ok(record)
    }

    pub(crate) fn reserve_link_continuation(&mut self) -> Result<u32, PdfObjectCapacityError> {
        self.reserve_document_object()
    }

    pub(crate) fn end_link(&mut self) -> Option<PdfOpenLink> {
        let open = Arc::make_mut(&mut self.open_links).pop();
        self.open_link_fingerprint = open_link_fingerprint(&self.open_links);
        open
    }

    #[must_use]
    pub(crate) fn links(&self) -> &[PdfLinkRecord] {
        &self.links
    }

    #[must_use]
    pub(crate) fn last_link(&self) -> u32 {
        self.links.last().map_or(0, |record| record.object())
    }

    #[must_use]
    pub(crate) fn open_links(&self) -> &[PdfOpenLink] {
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
                PdfFontOperation::Attribute { .. }
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
        for operation in self.font_maps() {
            let PdfFontMapOperation::Line(entry) = operation else {
                continue;
            };
            match entry.directive {
                tex_fonts::PdfFontMapDirective::Default | tex_fonts::PdfFontMapDirective::Add => {
                    if entries.contains_key(&entry.tex_name) {
                        duplicates.push(entry.tex_name.clone());
                    } else {
                        entries.insert(entry.tex_name.clone(), entry.clone());
                    }
                }
                tex_fonts::PdfFontMapDirective::Replace => {
                    entries.insert(entry.tex_name.clone(), entry.clone());
                }
                tex_fonts::PdfFontMapDirective::Remove => {
                    entries.remove(&entry.tex_name);
                }
            }
        }
        (entries, duplicates)
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
        let images = Arc::make_mut(&mut self.external_images);
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
                    bytes: Arc::from([]),
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
            bytes: source.bytes,
            mask_object,
        };
        Arc::make_mut(&mut self.external_images).push(record.clone());
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
        box_list: NodeListId,
        box_semantic_id: u64,
        dimensions: (Scaled, Scaled, Scaled),
        options: (Option<PdfTokenParameter>, Option<PdfTokenParameter>),
        immediate: bool,
    ) -> Result<PdfFormRecord, PdfObjectCapacityError> {
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
        Arc::make_mut(&mut self.forms).push(record);
        self.form_fingerprint = append_form_fingerprint(self.form_fingerprint, record);
        Ok(record)
    }

    #[must_use]
    pub(crate) fn form(&self, object: u32) -> Option<PdfFormRecord> {
        self.forms
            .iter()
            .copied()
            .find(|form| form.object == object)
    }

    pub(crate) fn forms(&self) -> impl ExactSizeIterator<Item = PdfFormRecord> + '_ {
        self.forms.iter().copied()
    }

    #[must_use]
    pub(crate) fn last_form(&self) -> u32 {
        self.forms.last().map_or(0, |form| form.object)
    }

    pub(crate) fn set_form_artifact(&mut self, object: u32, artifact: PdfFormArtifact) {
        let mut hasher = StateHasher::new(0x7064_665f_666d_6172);
        hasher.u64(self.form_artifact_fingerprint);
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
        self.form_artifact_fingerprint = hasher.finish();
        Arc::make_mut(&mut self.form_artifacts).insert(object, artifact);
    }

    #[must_use]
    pub(crate) fn form_artifact(&self, object: u32) -> Option<&PdfFormArtifact> {
        self.form_artifacts.get(&object)
    }

    pub(crate) fn initialize_raw_object(
        &mut self,
        id: PdfRawObjectId,
        data: PdfRawObjectData,
        immediate: bool,
    ) -> Result<(), PdfRawObjectInitializeError> {
        self.raw_objects.initialize(id, data, immediate)
    }

    #[must_use]
    pub(crate) fn raw_object(&self, id: PdfRawObjectId) -> Option<PdfRawObjectRecord> {
        self.raw_objects.record(id)
    }

    pub(crate) fn reference_raw_object(
        &mut self,
        id: PdfRawObjectId,
    ) -> Result<(), PdfRawObjectInitializeError> {
        self.raw_objects.reference(id)
    }

    #[must_use]
    pub(crate) fn raw_objects(&self) -> &[PdfRawObjectRecord] {
        self.raw_objects.records()
    }

    #[must_use]
    pub(crate) fn last_raw_object(&self) -> u32 {
        self.raw_objects.last_object()
    }

    pub(crate) fn append_document_fragment(
        &mut self,
        kind: PdfDocumentFragmentKind,
        value: PdfTokenParameter,
    ) {
        self.document_fragments.append(kind, value);
    }

    pub(crate) fn document_fragments(
        &self,
        kind: PdfDocumentFragmentKind,
    ) -> impl Iterator<Item = TokenListId> + '_ {
        self.document_fragments.values(kind)
    }

    pub(crate) fn set_catalog_open_action(
        &mut self,
        spec: PdfActionSpec,
        fingerprint: u64,
        destination_identity: Option<PdfDestinationIdentity>,
        structure_identity: Option<PdfDestinationIdentity>,
    ) -> Result<PdfActionRecord, PdfObjectCapacityError> {
        debug_assert!(self.catalog_open_action.is_none());
        let id = self.reserve_document_object()?;
        let target_object = if let Some(identity) = destination_identity {
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
        let record = PdfActionRecord::new(id, spec, target_object, structure_object);
        if let PdfActionSpec::GoTo(PdfActionDestination {
            file: None,
            target: PdfActionTarget::Page { number, .. },
            ..
        }) = spec
        {
            Arc::make_mut(&mut self.page_reservations).push(PdfPageReservation {
                number,
                object: target_object.expect("internal page action reserves its page object"),
            });
            self.page_reservation_fingerprint =
                page_reservation_fingerprint(&self.page_reservations);
        }
        self.catalog_open_action = Some(record);
        self.action_fingerprint = fingerprint;
        Ok(record)
    }

    #[must_use]
    pub(crate) const fn catalog_open_action(&self) -> Option<PdfActionRecord> {
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
    pub(crate) fn cursor(&self) -> PdfStateCursor {
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
    pub(crate) fn snapshot(&self) -> PdfStateSnapshot {
        PdfStateSnapshot {
            cursor: self.cursor(),
            match_state: Arc::clone(&self.match_state),
            external_images: Arc::clone(&self.external_images),
            raw_objects: self.raw_objects.clone(),
            document_fragments: self.document_fragments.clone(),
            page_reservations: Arc::clone(&self.page_reservations),
            annotations: Arc::clone(&self.annotations),
            links: Arc::clone(&self.links),
            open_links: Arc::clone(&self.open_links),
            color_stacks: Arc::clone(&self.color_stacks),
            forms: Arc::clone(&self.forms),
            form_artifacts: Arc::clone(&self.form_artifacts),
            destinations: Arc::clone(&self.destinations),
            structure_destinations: Arc::clone(&self.structure_destinations),
            outlines: Arc::clone(&self.outlines),
            threads: Arc::clone(&self.threads),
        }
    }

    pub(crate) fn rollback(&mut self, snapshot: PdfStateSnapshot) {
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
        self.match_state = Arc::new(PdfMatchState {
            haystack,
            captures,
            slot_count,
            matched,
            fingerprint,
        });
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
        StateHashFragment::from_builder(PDF_STATE_DOMAIN, |hasher| {
            hasher.bool(cursor.enabled);
            hasher.u32(cursor.next_object);
            hasher.usize(cursor.page_count);
            hash_output_parameters(hasher, cursor.output_parameters);
            hasher.usize(cursor.font_operation_count);
            hasher.usize(cursor.font_resource_count);
            hasher.u64(cursor.fingerprint);
            hasher.u64(cursor.match_fingerprint);
            hasher.u64(cursor.external_image_fingerprint);
            hasher.u64(cursor.raw_object_fingerprint);
            hasher.u64(cursor.document_fragment_fingerprint);
            hasher.u64(cursor.action_fingerprint);
            hasher.u64(cursor.page_reservation_fingerprint);
            hasher.u64(cursor.space_font_name_fingerprint);
            hasher.u64(cursor.annotation_fingerprint);
            hasher.u64(cursor.link_fingerprint);
            hasher.u64(cursor.open_link_fingerprint);
            hasher.u64(cursor.form_fingerprint);
            hasher.u32(cursor.next_form_resource);
            hasher.u64(cursor.form_artifact_fingerprint);
            hasher.i32(cursor.return_value);
            hasher.u64(cursor.destination_fingerprint);
            hasher.u64(cursor.structure_destination_fingerprint);
            hasher.u64(cursor.outline_fingerprint);
            hasher.u64(cursor.thread_fingerprint);
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
            hasher.u64(cursor.color_stack_fingerprint);
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
        for (stack, runtime) in Arc::make_mut(&mut self.color_stacks)
            .iter_mut()
            .zip(runtimes)
        {
            stack.form = runtime;
        }
        self.color_stack_fingerprint = fingerprint;
    }

    fn ensure_default_color_stack(&mut self) {
        if !self.color_stacks.is_empty() {
            return;
        }
        let initial = b"0 g 0 G".to_vec();
        Arc::make_mut(&mut self.color_stacks).push(PdfColorStack {
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
        Arc::make_mut(&mut self.color_stacks).push(PdfColorStack {
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
        let Some(stack) = Arc::make_mut(&mut self.color_stacks).get_mut(id as usize) else {
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

fn color_stack_fingerprint(stacks: &[PdfColorStack]) -> u64 {
    let mut hasher = StateHasher::new(PDF_COLOR_STACK_DOMAIN);
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
    hasher.finish()
}

fn external_image_base_fingerprint() -> u64 {
    StateHasher::new(PDF_EXTERNAL_IMAGE_DOMAIN).finish()
}

fn page_reservation_fingerprint(reservations: &[PdfPageReservation]) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_7067_7273);
    hasher.usize(reservations.len());
    for reservation in reservations {
        hasher.u32(reservation.number);
        hasher.u32(reservation.object);
    }
    hasher.finish()
}

fn annotation_fingerprint(_records: &[PdfAnnotationRecord]) -> u64 {
    StateHasher::new(0x7064_665f_616e_6e6f).finish()
}

fn append_annotation_reservation_fingerprint(previous: u64, object: u32) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_616e_6e6f);
    hasher.u64(previous);
    hasher.u8(0);
    hasher.u32(object);
    hasher.finish()
}

fn append_annotation_data_fingerprint(
    previous: u64,
    object: u32,
    dimensions: PdfAnnotationDimensions,
    entries_semantic_id: u64,
) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_616e_6e6f);
    hasher.u64(previous);
    hasher.u8(1);
    hasher.u32(object);
    hash_annotation_dimensions(&mut hasher, dimensions);
    hasher.u64(entries_semantic_id);
    hasher.finish()
}

fn append_link_fingerprint(
    previous: u64,
    record: PdfLinkRecord,
    attributes_semantic_id: u64,
    action_semantic_id: u64,
) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_6c69_6e6b);
    hasher.u64(previous);
    hasher.u32(record.object());
    hash_annotation_dimensions(&mut hasher, record.dimensions());
    hasher.u64(attributes_semantic_id);
    hasher.u64(action_semantic_id);
    hasher.finish()
}

fn open_link_fingerprint(links: &[PdfOpenLink]) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_6f70_6c6e);
    hasher.usize(links.len());
    for link in links {
        hasher.u32(link.record.object());
        hasher.u32(link.nesting_depth);
    }
    hasher.finish()
}

fn destination_fingerprint(records: &[PdfDestinationRecord], structure: bool) -> u64 {
    let mut hasher = StateHasher::new(if structure {
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
    hasher.finish()
}

fn outline_fingerprint(_records: &[PdfOutlineRecord]) -> u64 {
    StateHasher::new(0x7064_665f_6f75_746c).finish()
}

fn thread_fingerprint(records: &[PdfThreadRecord]) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_7468_7264);
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
    hasher.finish()
}

fn append_outline_fingerprint(
    previous: u64,
    record: PdfOutlineRecord,
    attributes_semantic_id: u64,
    action_semantic_id: u64,
    title_semantic_id: u64,
) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_6f75_746c);
    hasher.u64(previous);
    hasher.u32(record.action_object());
    hasher.u32(record.item_object());
    hasher.u32(record.title_object());
    hasher.u64(attributes_semantic_id);
    hasher.u64(action_semantic_id);
    hasher.i32(record.count());
    hasher.u64(title_semantic_id);
    hasher.finish()
}

fn hash_annotation_dimensions(hasher: &mut StateHasher, dimensions: PdfAnnotationDimensions) {
    for value in [dimensions.width, dimensions.height, dimensions.depth] {
        hasher.bool(value.is_some());
        if let Some(value) = value {
            hasher.i32(value.raw());
        }
    }
}

fn external_image_fingerprint(images: &[PdfExternalImageRecord]) -> u64 {
    let mut hasher = StateHasher::new(PDF_EXTERNAL_IMAGE_DOMAIN);
    hasher.usize(images.len());
    for record in images {
        hasher.u32(record.id.raw());
        hasher.bytes(&record.identity.bytes());
        match record.metadata {
            PdfExternalImageMetadata::PdfPage {
                page_box,
                page,
                has_page_group,
                pdf_version,
            } => {
                hasher.u8(0);
                hasher.i32(page_box.left.raw());
                hasher.i32(page_box.bottom.raw());
                hasher.i32(page_box.right.raw());
                hasher.i32(page_box.top.raw());
                hasher.u32(page);
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
        hasher.bytes(&record.bytes);
        hasher.bool(record.mask_object.is_some());
        if let Some(mask) = record.mask_object {
            hasher.u32(mask);
        }
    }
    hasher.finish()
}

fn append_font_resource_fingerprint(previous: u64, record: PdfFontResourceRecord) -> u64 {
    let mut hasher = StateHasher::new(PDF_FONT_DOMAIN);
    hasher.u64(previous);
    hasher.tag(5);
    hasher.u32(record.font.raw());
    hasher.bytes(&record.source_identity.bytes());
    hasher.u32(record.resource_number);
    hasher.u32(record.object_number);
    hasher.bytes(&record.tfm_content_hash);
    hasher.bool(record.program_identity.is_some());
    if let Some(identity) = record.program_identity {
        hasher.bytes(&identity);
    }
    hasher.finish()
}

fn append_font_fingerprint(previous: u64, operation: &PdfFontOperation) -> u64 {
    let mut hasher = StateHasher::new(PDF_FONT_DOMAIN);
    hasher.u64(previous);
    match operation {
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
            hasher.bool(line.font_file.is_some());
            if let Some(file) = &line.font_file {
                hasher.bytes(file);
            }
            hasher.tag(line.program as u8);
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
    hasher.finish()
}

fn match_fingerprint(
    haystack: &[u8],
    captures: &[Option<(u32, u32)>],
    slot_count: u32,
    matched: bool,
) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_6d61_7463);
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
    hasher.finish()
}

fn base_fingerprint(enabled: bool) -> u64 {
    let mut hasher = StateHasher::new(PDF_STATE_DOMAIN);
    hasher.bool(enabled);
    hasher.u32(FIRST_DYNAMIC_OBJECT);
    hasher.finish()
}

fn space_font_name_fingerprint(name: &[u8]) -> u64 {
    let mut hasher = StateHasher::new(0x7064_665f_7370_666e);
    hasher.bytes(name);
    hasher.finish()
}

fn freeze_fingerprint(previous: u64, parameters: PdfOutputParameters) -> u64 {
    let mut hasher = StateHasher::new(PDF_PAGE_DOMAIN);
    hasher.u64(previous);
    hash_output_parameters(&mut hasher, Some(parameters));
    hasher.finish()
}

fn append_fingerprint(previous: u64, record: PdfPageRecord) -> u64 {
    let mut hasher = StateHasher::new(PDF_PAGE_DOMAIN);
    hasher.u64(previous);
    hasher.bytes(&record.artifact.bytes());
    hasher.u32(record.resources_object);
    hasher.u32(record.contents_object);
    hasher.u32(record.page_object);
    hasher.i32(record.parameters.h_origin.raw());
    hasher.i32(record.parameters.v_origin.raw());
    hasher.i32(record.parameters.width.raw());
    hasher.i32(record.parameters.height.raw());
    hasher.u64(record.parameters.page_attr.semantic_id);
    hasher.u64(record.parameters.resources.semantic_id);
    hasher.i32(record.parameters.omit_procset);
    hasher.u32(record.parameters.space_font_name);
    hasher.finish()
}

fn append_form_fingerprint(previous: u64, record: PdfFormRecord) -> u64 {
    let mut hasher = StateHasher::new(PDF_FORM_DOMAIN);
    hasher.u64(previous);
    hasher.u32(record.object);
    hasher.u32(record.resource);
    hasher.u64(record.box_semantic_id);
    hasher.i32(record.width.raw());
    hasher.i32(record.height.raw());
    hasher.i32(record.depth.raw());
    for value in [record.attr, record.resources] {
        hasher.bool(value.is_some());
        if let Some(value) = value {
            hasher.u64(value.semantic_id);
        }
    }
    hasher.bool(record.immediate);
    hasher.finish()
}

fn freeze_pk_mode_fingerprint(previous: u64, mode: PdfTokenParameter) -> u64 {
    let mut hasher = StateHasher::new(PDF_PAGE_DOMAIN);
    hasher.u64(previous);
    hasher.u64(mode.semantic_id);
    hasher.finish()
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
mod tests {
    use super::*;

    #[test]
    fn annotation_and_link_objects_are_typed_hashed_and_rollback_safe() {
        let mut state = PdfState::default();
        state.enable();
        let base = state.snapshot();
        let base_hash = state.hash_fragment();

        let reserved = state.reserve_annotation().expect("reserve annotation");
        assert_eq!(reserved.object(), 1);
        assert_eq!(reserved.data(), None);
        assert_eq!(state.last_annotation(), 1);
        let dimensions = PdfAnnotationDimensions {
            width: Some(Scaled::from_raw(10)),
            height: None,
            depth: Some(Scaled::from_raw(2)),
        };
        let annotation = state
            .initialize_annotation(
                reserved.object(),
                PdfAnnotationData {
                    dimensions,
                    entries: TokenListId::EMPTY,
                },
                17,
            )
            .expect("initialize annotation");
        assert_eq!(
            annotation.data().expect("initialized").dimensions,
            dimensions
        );
        assert!(
            state
                .initialize_annotation(
                    reserved.object(),
                    PdfAnnotationData {
                        dimensions,
                        entries: TokenListId::EMPTY,
                    },
                    17,
                )
                .is_err(),
            "useobjnum cannot initialize an annotation twice"
        );

        let link = state
            .create_link(
                PdfAnnotationDimensions::RUNNING,
                TokenListId::EMPTY,
                PdfActionSpec::User(TokenListId::EMPTY),
                19,
                23,
                1,
            )
            .expect("create link");
        assert_eq!(link.object(), 2);
        assert_eq!(state.last_link(), 2);
        assert_eq!(state.end_link().expect("open link").record, link);
        assert_ne!(state.hash_fragment(), base_hash);

        state.rollback(base);
        assert_eq!(state.next_object(), 1);
        assert_eq!(state.last_annotation(), 0);
        assert_eq!(state.last_link(), 0);
        assert_eq!(state.hash_fragment(), base_hash);
    }

    #[test]
    fn page_group_selector_keeps_first_group_on_page_and_later_groups_on_forms() {
        let mut selector = PdfPageGroupSelector::new(0);
        assert_eq!(selector.include(false), PdfPageGroupInclusion::None);
        assert!(!selector.has_selection());
        assert_eq!(
            selector.include(true),
            PdfPageGroupInclusion::SelectForOutputPage
        );
        assert!(selector.has_selection());
        assert_eq!(
            selector.include(true),
            PdfPageGroupInclusion::KeepOnIncludedForm {
                warning: Some(PdfPageGroupWarning::MultipleGroupsOnOnePage),
            }
        );
        assert_eq!(
            selector.include(false),
            PdfPageGroupInclusion::None,
            "images without page groups do not disturb the first selection"
        );
        assert!(selector.has_selection());
    }

    #[test]
    fn page_group_warning_matches_pdftex_for_zero_positive_and_negative_controls() {
        for (control, warning) in [
            (0, Some(PdfPageGroupWarning::MultipleGroupsOnOnePage)),
            (1, None),
            (-1, None),
        ] {
            let mut selector = PdfPageGroupSelector::new(control);
            assert_eq!(
                selector.include(true),
                PdfPageGroupInclusion::SelectForOutputPage
            );
            assert_eq!(
                selector.include(true),
                PdfPageGroupInclusion::KeepOnIncludedForm { warning },
                "control {control}"
            );
        }
        assert_eq!(
            PdfPageGroupWarning::MultipleGroupsOnOnePage.message(),
            "PDF inclusion: multiple pdfs with page group included in a single page"
        );
    }

    #[test]
    fn font_configuration_preserves_pdftex_thresholds_and_pk_resolution() {
        let mut configuration = PdfFontConfiguration {
            adjust_spacing: 1,
            protrude_chars: 1,
            tracing_fonts: 1,
            adjust_interword_glue: 1,
            prepend_kern: 1,
            append_kern: 1,
            generate_to_unicode: 1,
            pk_resolution: 0,
            omit_charset: 1,
        };
        assert!(configuration.adjusts_spacing());
        assert!(!configuration.adjusts_line_breaking());
        assert!(configuration.protrudes_chars());
        assert!(!configuration.protrudes_during_line_breaking());
        assert!(configuration.traces_fonts());
        assert!(configuration.adjusts_interword_glue());
        assert!(configuration.prepends_kerns());
        assert!(configuration.appends_kerns());
        assert!(configuration.generates_to_unicode());
        assert!(configuration.omits_charset());
        assert_eq!(configuration.resolved_pk_resolution(600), 600);

        configuration.adjust_spacing = 2;
        configuration.protrude_chars = 2;
        configuration.pk_resolution = 9_000;
        assert!(configuration.adjusts_line_breaking());
        assert!(configuration.protrudes_during_line_breaking());
        assert_eq!(configuration.resolved_pk_resolution(600), 8_000);

        configuration.pk_resolution = -1;
        configuration.omit_charset = -1;
        assert_eq!(configuration.resolved_pk_resolution(600), 72);
        assert!(configuration.omits_charset());
    }

    #[test]
    fn image_output_controls_use_pdftex_consumer_ranges() {
        let parameters = PdfOutputParameters {
            output: 1,
            major_version: 1,
            minor_version: 4,
            compress_level: 9,
            object_compress_level: 0,
            decimal_digits: 3,
            gamma: -1,
            image_gamma: 1_000_001,
            image_hicolor: 2,
            image_apply_gamma: -1,
            draft_mode: 2,
            inclusion_copy_fonts: -1,
            pk_resolution: 9_000,
            unique_resource_names: -2,
        }
        .normalized();
        assert_eq!(parameters.gamma, 0);
        assert_eq!(parameters.image_gamma, 1_000_000);
        assert_eq!(parameters.image_hicolor, 1);
        assert_eq!(parameters.image_apply_gamma, 0);
        assert_eq!(parameters.draft_mode, 2);
        assert_eq!(parameters.inclusion_copy_fonts, 0);
        assert_eq!(parameters.pk_resolution, 8_000);
        assert_eq!(parameters.unique_resource_names, 0);
    }

    #[test]
    fn rollback_reuses_page_object_suffix_and_fingerprint() {
        let mut state = PdfState::default();
        state.enable();
        let snapshot = state.snapshot();
        let hash = ContentHash::new([7; 32]);
        let parameters = PdfOutputParameters {
            output: 1,
            major_version: 1,
            minor_version: 4,
            compress_level: 9,
            object_compress_level: 0,
            decimal_digits: 3,
            gamma: 1_000,
            image_gamma: 2_200,
            image_hicolor: 1,
            image_apply_gamma: 0,
            draft_mode: 0,
            inclusion_copy_fonts: 0,
            pk_resolution: 0,
            unique_resource_names: 0,
        };
        let token = PdfTokenParameter {
            tokens: TokenListId::EMPTY,
            semantic_id: 0,
        };
        let page = PdfPageParameters {
            h_origin: Scaled::from_raw(10),
            v_origin: Scaled::from_raw(20),
            width: Scaled::from_raw(30),
            height: Scaled::from_raw(40),
            link_margin: Scaled::from_raw(0),
            page_attr: token,
            resources: token,
            omit_procset: 0,
            space_font_name: 0,
        };
        state.commit_page(hash, parameters, page, token);
        let first = (state.pages()[0], state.cursor());
        state.rollback(snapshot);
        state.commit_page(hash, parameters, page, token);
        assert_eq!((state.pages()[0], state.cursor()), first);
    }

    #[test]
    fn font_output_log_rolls_back_and_projects_last_attribute_and_char_union() {
        let mut state = PdfState::default();
        state.enable();
        let font = crate::font::NULL_FONT;
        state.set_font_attribute(font, b"/StemV 70".to_vec());
        state.include_font_chars(font, vec![b'B', b'A', b'B']);
        state.set_glyph_to_unicode(PdfGlyphToUnicode {
            tfm_name: None,
            glyph_name: b"A".to_vec(),
            unicode: vec![0x41],
        });
        let checkpoint = state.snapshot();
        let checkpoint_hash = state.hash_fragment();

        state.set_font_attribute(font, b"/StemV 80".to_vec());
        state.include_font_chars(font, vec![b'C']);
        state.disable_builtin_to_unicode(font);
        state.set_glyph_to_unicode(PdfGlyphToUnicode {
            tfm_name: None,
            glyph_name: b"A".to_vec(),
            unicode: vec![0x391],
        });
        state.push_font_map(PdfFontMapOperation::Line(
            tex_fonts::PdfFontMapEntry::parse(b"cmr10 CMR10 <cmr10.pfb").expect("valid map entry"),
        ));
        assert_eq!(state.font_attribute(font), b"/StemV 80");
        assert_eq!(state.included_font_chars(font), b"ABC");
        assert_eq!(state.font_maps().count(), 1);
        assert_eq!(
            state.glyph_to_unicode(b"cmr10", b"A"),
            Some([0x391].as_slice())
        );
        assert!(state.builtin_to_unicode_disabled(font));

        state.rollback(checkpoint);
        assert_eq!(state.font_attribute(font), b"/StemV 70");
        assert_eq!(state.included_font_chars(font), b"AB");
        assert_eq!(state.font_maps().count(), 0);
        assert_eq!(
            state.glyph_to_unicode(b"cmr10", b"A"),
            Some([0x41].as_slice())
        );
        assert!(!state.builtin_to_unicode_disabled(font));
        assert_eq!(state.hash_fragment(), checkpoint_hash);
    }

    #[test]
    fn pk_font_provision_is_typed_hashed_and_rollback_owned() {
        let mut bytes = vec![247, 89, 0];
        bytes.extend_from_slice(&[0; 16]);
        bytes.extend_from_slice(&[0xe0, 9, 65, 0, 0, 0, 3, 3, 2, 0, 1, 0b1010_1000]);
        bytes.push(245);
        let font = tex_fonts::PdfPkFont::parse(&bytes).expect("synthetic PK parses");
        let request = tex_fonts::PdfPkFontRequest::new(b"cmr10".to_vec(), 300, b"cx".to_vec());
        let mut state = PdfState::default();
        let before = state.hash_fragment();
        let snapshot = state.snapshot();
        state.provide_pk_font(request.clone(), font.clone());
        assert_eq!(state.pk_font(&request), Some(&font));
        assert_ne!(state.hash_fragment(), before);
        state.rollback(snapshot);
        assert!(state.pk_font(&request).is_none());
        assert_eq!(state.hash_fragment(), before);
    }

    #[test]
    fn map_line_resolution_keeps_first_duplicate_and_honors_replace_and_remove() {
        let mut state = PdfState::default();
        for line in [
            b"cmr10 First <cmr10.pfb".as_slice(),
            b"+cmr10 Ignored <ignored.pfb",
            b"=cmr10 Replacement <replacement.pfb",
            b"-cmr10",
            b"cmtt10 CMTT10 <cmtt10.pfb",
        ] {
            state.push_font_map(PdfFontMapOperation::Line(
                tex_fonts::PdfFontMapEntry::parse(line).expect("valid map entry"),
            ));
        }
        assert_eq!(state.font_map_duplicate_names(), [b"cmr10"]);
        let entries = state.resolved_font_map_lines();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tex_name, b"cmtt10");
    }

    #[test]
    fn external_image_metadata_is_typed_hashed_and_rollback_safe() {
        let mut state = PdfState::default();
        state.enable();
        let initial = state.snapshot();
        let initial_hash = state.hash_fragment();
        let id = PdfExternalImageId::new(7).expect("valid image object");
        let metadata = PdfExternalImageMetadata::PdfPage {
            page_box: PdfPageBox {
                left: Scaled::from_raw(-2),
                bottom: Scaled::from_raw(3),
                right: Scaled::from_raw(40),
                top: Scaled::from_raw(50),
            },
            page: 1,
            has_page_group: false,
            pdf_version: (1, 4),
        };

        state
            .register_external_image(id, metadata)
            .expect("register image metadata");
        let registered_hash = state.hash_fragment();
        assert_ne!(registered_hash, initial_hash);
        assert_eq!(state.external_image(id), Some(metadata));
        assert!(!state.is_format_empty());
        assert_eq!(metadata.bbox_coordinate(1), Some(Scaled::from_raw(-2)));
        assert_eq!(metadata.bbox_coordinate(4), Some(Scaled::from_raw(50)));
        assert_eq!(metadata.bbox_coordinate(5), None);
        assert_eq!(
            state.register_external_image(
                id,
                PdfExternalImageMetadata::Raster(PdfRasterImageMetadata::placeholder()),
            ),
            Err(PdfExternalImageRegistrationError::Duplicate(id))
        );

        state.rollback(initial.clone());
        assert_eq!(state.external_image(id), None);
        assert_eq!(state.hash_fragment(), initial_hash);
        state
            .register_external_image(id, metadata)
            .expect("replay image metadata");
        assert_eq!(state.hash_fragment(), registered_hash);

        assert_eq!(
            PdfExternalImageMetadata::Raster(PdfRasterImageMetadata::placeholder())
                .bbox_coordinate(3),
            Some(Scaled::from_raw(0))
        );
    }

    #[test]
    fn allocated_external_images_share_the_object_ledger_and_replay_exactly() {
        let mut state = PdfState::default();
        state.enable();
        let snapshot = state.snapshot();
        let source = PdfExternalImageSource {
            identity: ContentHash::new([19; 32]),
            metadata: PdfExternalImageMetadata::Raster(PdfRasterImageMetadata::placeholder()),
            natural_width: Scaled::from_raw(640),
            natural_height: Scaled::from_raw(480),
            bytes: Arc::from([1, 2, 3]),
        };
        let dimensions = PdfExternalImageDimensions {
            width: Scaled::from_raw(320),
            height: Scaled::from_raw(240),
            depth: Scaled::from_raw(7),
        };

        let allocated = state
            .allocate_external_image(source.clone(), dimensions)
            .expect("allocate image");
        let record = state.last_external_image().expect("last image");
        assert_eq!(allocated.id().raw(), 1);
        assert_eq!(record.id(), allocated.id());
        assert_eq!(record.identity(), source.identity);
        assert_eq!(record.metadata(), source.metadata);
        assert_eq!(record.dimensions(), dimensions);
        assert_eq!(state.cursor().next_object, 2);
        let allocated_hash = state.hash_fragment();

        state.rollback(snapshot);
        assert_eq!(state.last_external_image(), None);
        assert_eq!(
            state
                .allocate_external_image(source, dimensions)
                .expect("replay allocation"),
            allocated
        );
        assert_eq!(state.hash_fragment(), allocated_hash);
    }

    #[test]
    fn color_stacks_are_checkpointed_and_page_and_form_state_stay_independent() {
        let mut state = PdfState::default();
        let before_hash = state.hash_fragment();
        let before = state.snapshot();
        let id = state
            .allocate_color_stack(PdfColorStackMode::Page, true, b"0 0 1 rg".to_vec())
            .expect("color stack capacity");
        assert_eq!(id, 1);
        let allocated_hash = state.hash_fragment();
        assert_ne!(allocated_hash, before_hash);

        let page = state
            .apply_color_stack(
                id,
                PdfColorStackTarget::Page,
                &PdfColorStackAction::Push(b"1 0 0 rg".to_vec()),
            )
            .expect("page push");
        assert_eq!(page.payload, b"1 0 0 rg");
        let form = state
            .apply_color_stack(id, PdfColorStackTarget::Form, &PdfColorStackAction::Current)
            .expect("form current");
        assert_eq!(form.payload, b"0 0 1 rg");
        assert_eq!(
            state.apply_color_stack(0, PdfColorStackTarget::Page, &PdfColorStackAction::Pop),
            Err(PdfColorStackApplyError::Underflow)
        );

        state.rollback(before);
        assert_eq!(
            state.allocate_color_stack(PdfColorStackMode::Page, true, b"0 0 1 rg".to_vec()),
            Ok(1)
        );
    }

    #[test]
    fn raw_object_reservation_initialization_and_rollback_share_one_ledger() {
        let mut state = PdfState::default();
        state.enable();
        let initial = state.snapshot();
        let initial_hash = state.hash_fragment();

        let first = state.reserve_raw_object().expect("reserve raw object");
        assert_eq!(first.raw(), 1);
        assert_eq!(state.last_raw_object(), 1);
        assert_eq!(state.next_object(), 2);
        assert_eq!(state.raw_object(first).expect("reserved").data(), None);
        let tokens = PdfTokenParameter {
            tokens: TokenListId::EMPTY,
            semantic_id: 17,
        };
        let data = PdfRawObjectData::new(true, Some(tokens), false, tokens);
        state
            .initialize_raw_object(first, data, true)
            .expect("initialize reservation");
        let record = state.raw_object(first).expect("initialized");
        assert_eq!(record.data(), Some(data));
        assert!(record.is_immediate());
        assert!(!state.is_format_empty());
        assert_eq!(
            state.initialize_raw_object(first, data, false),
            Err(PdfRawObjectInitializeError::AlreadyInitialized(first))
        );
        let allocated_hash = state.hash_fragment();
        assert_ne!(allocated_hash, initial_hash);

        state.rollback(initial);
        assert_eq!(state.raw_object(first), None);
        assert_eq!(state.last_raw_object(), 0);
        assert_eq!(state.next_object(), 1);
        assert_eq!(state.hash_fragment(), initial_hash);
        let replay = state.reserve_raw_object().expect("replay reservation");
        state
            .initialize_raw_object(replay, data, true)
            .expect("replay initialization");
        assert_eq!(replay, first);
        assert_eq!(state.hash_fragment(), allocated_hash);
    }

    #[test]
    fn document_fragments_preserve_kind_order_hash_and_rollback() {
        let mut state = PdfState::default();
        state.enable();
        let initial = state.snapshot();
        let initial_hash = state.hash_fragment();
        let first = PdfTokenParameter {
            tokens: TokenListId::EMPTY,
            semantic_id: 11,
        };
        let second = PdfTokenParameter {
            tokens: TokenListId::EMPTY,
            semantic_id: 22,
        };

        state.append_document_fragment(PdfDocumentFragmentKind::Info, first);
        state.append_document_fragment(PdfDocumentFragmentKind::Catalog, first);
        state.append_document_fragment(PdfDocumentFragmentKind::Info, second);
        assert_eq!(
            state
                .document_fragments(PdfDocumentFragmentKind::Info)
                .collect::<Vec<_>>(),
            vec![first.tokens, second.tokens]
        );
        assert_eq!(
            state
                .document_fragments(PdfDocumentFragmentKind::Catalog)
                .collect::<Vec<_>>(),
            vec![first.tokens]
        );
        assert!(!state.is_format_empty());
        let appended_hash = state.hash_fragment();
        assert_ne!(appended_hash, initial_hash);

        state.rollback(initial);
        assert_eq!(
            state
                .document_fragments(PdfDocumentFragmentKind::Info)
                .count(),
            0
        );
        assert_eq!(state.hash_fragment(), initial_hash);
        state.append_document_fragment(PdfDocumentFragmentKind::Info, first);
        state.append_document_fragment(PdfDocumentFragmentKind::Catalog, first);
        state.append_document_fragment(PdfDocumentFragmentKind::Info, second);
        assert_eq!(state.hash_fragment(), appended_hash);
    }

    #[test]
    fn return_value_is_checkpointed_hashed_and_excluded_from_formats() {
        let mut state = PdfState::default();
        assert_eq!(state.return_value(), 0);
        assert!(state.is_format_empty());
        let initial = state.snapshot();
        let initial_hash = state.hash_fragment();

        state.set_return_value(-1);
        assert_eq!(state.return_value(), -1);
        assert!(state.is_format_empty());
        let failed_hash = state.hash_fragment();
        assert_ne!(failed_hash, initial_hash);

        state.rollback(initial.clone());
        assert_eq!(state.return_value(), 0);
        assert_eq!(state.hash_fragment(), initial_hash);
        state.set_return_value(-1);
        assert_eq!(state.hash_fragment(), failed_hash);
        state.rollback(initial);
    }

    #[test]
    fn space_font_names_are_interned_checkpointed_and_page_addressable() {
        let mut state = PdfState::default();
        assert_eq!(state.space_font_name(0), Some(b"pdftexspace".as_slice()));
        assert!(state.is_format_empty());
        let initial = state.snapshot();
        let initial_hash = state.hash_fragment();

        state.set_space_font_name(b"fixture-space".to_vec());
        let selected = state.current_space_font_name_id();
        assert_eq!(selected, 1);
        assert_eq!(
            state.space_font_name(selected),
            Some(b"fixture-space".as_slice())
        );
        let selected_hash = state.hash_fragment();
        assert_ne!(selected_hash, initial_hash);
        state.set_space_font_name(b"fixture-space".to_vec());
        assert_eq!(state.current_space_font_name_id(), selected);
        assert_eq!(state.space_font_names.len(), 2);

        state.rollback(initial.clone());
        assert_eq!(state.current_space_font_name_id(), 0);
        assert_eq!(state.space_font_name(selected), None);
        assert_eq!(state.hash_fragment(), initial_hash);

        state.set_space_font_name(b"fixture-space".to_vec());
        assert_eq!(state.current_space_font_name_id(), selected);
        assert_eq!(state.hash_fragment(), selected_hash);
        state.rollback(initial);
        assert!(state.is_format_empty());
    }

    #[test]
    fn mixed_resource_allocation_is_collision_free_and_replays_exactly() {
        let mut pk_bytes = vec![247, 89, 0];
        pk_bytes.extend_from_slice(&[0; 16]);
        pk_bytes.extend_from_slice(&[0xe0, 9, 65, 0, 0, 0, 3, 3, 2, 0, 1, 0b1010_1000]);
        pk_bytes.push(245);
        let pk_font = tex_fonts::PdfPkFont::parse(&pk_bytes).expect("synthetic PK parses");
        let pk_request = tex_fonts::PdfPkFontRequest::new(b"cmr10".to_vec(), 300, b"cx".to_vec());
        let token = PdfTokenParameter {
            tokens: TokenListId::EMPTY,
            semantic_id: 29,
        };
        let output = PdfOutputParameters {
            output: 1,
            major_version: 1,
            minor_version: 4,
            compress_level: 0,
            object_compress_level: 0,
            decimal_digits: 3,
            gamma: 0,
            image_gamma: 0,
            image_hicolor: 0,
            image_apply_gamma: 0,
            draft_mode: 0,
            inclusion_copy_fonts: 0,
            pk_resolution: 300,
            unique_resource_names: 0,
        };
        let page = PdfPageParameters {
            h_origin: Scaled::from_raw(0),
            v_origin: Scaled::from_raw(0),
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(1),
            link_margin: Scaled::from_raw(0),
            page_attr: token,
            resources: token,
            omit_procset: 0,
            space_font_name: 0,
        };
        let exercise = |state: &mut PdfState| {
            state.provide_pk_font(pk_request.clone(), pk_font.clone());
            state
                .register_external_image(
                    PdfExternalImageId::new(99).expect("image identity"),
                    PdfExternalImageMetadata::Raster(PdfRasterImageMetadata::placeholder()),
                )
                .expect("image metadata");
            let font = state
                .ensure_font_resource(
                    crate::font::NULL_FONT,
                    tex_fonts::FontSourceIdentity::from_bytes([7; 32]),
                    [11; 32],
                    None,
                )
                .expect("font object");
            let raw = state.reserve_raw_object().expect("raw object");
            state
                .initialize_raw_object(raw, PdfRawObjectData::new(false, None, false, token), true)
                .expect("raw data");
            state.commit_page(ContentHash::new([13; 32]), output, page, token);
            state.append_document_fragment(PdfDocumentFragmentKind::Names, token);
            let document = state
                .finalize_document_objects(true)
                .expect("document objects");
            let page = state.pages()[0];
            vec![
                font.object_number(),
                raw.raw(),
                page.resources_object(),
                page.page_object(),
                page.contents_object(),
                document.pages().expect("pages"),
                document.names().expect("names"),
                document.catalog().expect("catalog"),
                document.info().expect("info"),
            ]
        };

        let mut state = PdfState::default();
        state.enable();
        let initial = state.snapshot();
        let first = exercise(&mut state);
        assert_eq!(first, (1..=9).collect::<Vec<_>>());
        let completed_hash = state.hash_fragment();
        let completed_cursor = state.cursor();

        state.rollback(initial);
        let replay = exercise(&mut state);
        assert_eq!(replay, first);
        assert_eq!(state.cursor(), completed_cursor);
        assert_eq!(state.hash_fragment(), completed_hash);
    }

    #[test]
    fn final_document_objects_allocate_once_through_the_shared_ledger() {
        let mut state = PdfState::default();
        state.enable();
        let token = PdfTokenParameter {
            tokens: TokenListId::EMPTY,
            semantic_id: 7,
        };
        let raw = state.reserve_raw_object().expect("raw object");
        assert_eq!(raw.raw(), 1);
        state.append_document_fragment(PdfDocumentFragmentKind::Names, token);
        let before = state.snapshot();

        let objects = state
            .finalize_document_objects(true)
            .expect("final dictionaries");
        assert_eq!(objects.pages(), Some(2));
        assert_eq!(objects.names(), Some(3));
        assert_eq!(objects.catalog(), Some(4));
        assert_eq!(objects.info(), Some(5));
        assert_eq!(state.next_object(), 6);
        assert_eq!(
            state
                .finalize_document_objects(true)
                .expect("repeated finalization"),
            objects,
            "finalization is idempotent"
        );

        state.rollback(before);
        assert_eq!(state.next_object(), 2);
        let replay = state
            .finalize_document_objects(true)
            .expect("replayed finalization");
        assert_eq!(replay, objects);
    }

    #[test]
    fn catalog_page_action_reserves_and_replays_the_target_page_identity() {
        let mut state = PdfState::default();
        state.enable();
        let initial = state.snapshot();
        let initial_hash = state.hash_fragment();
        let action = PdfActionSpec::GoTo(PdfActionDestination {
            file: None,
            structure: None,
            target: PdfActionTarget::Page {
                number: 1,
                view: TokenListId::EMPTY,
            },
            window: PdfActionWindow::Unspecified,
        });
        let record = state
            .set_catalog_open_action(action, action.fingerprint(|_| 17), None, None)
            .expect("reserve action and page target");
        assert_eq!(record.id(), 1);
        assert_eq!(record.target_object(), Some(2));
        assert_eq!(state.next_object(), 3);

        let parameters = PdfOutputParameters {
            output: 1,
            major_version: 1,
            minor_version: 4,
            compress_level: 0,
            object_compress_level: 0,
            decimal_digits: 3,
            gamma: 0,
            image_gamma: 0,
            image_hicolor: 0,
            image_apply_gamma: 0,
            draft_mode: 0,
            inclusion_copy_fonts: 0,
            pk_resolution: 0,
            unique_resource_names: 0,
        };
        let token = PdfTokenParameter {
            tokens: TokenListId::EMPTY,
            semantic_id: 17,
        };
        state.commit_page(
            ContentHash::new([4; 32]),
            parameters,
            PdfPageParameters {
                h_origin: Scaled::from_raw(0),
                v_origin: Scaled::from_raw(0),
                width: Scaled::from_raw(1),
                height: Scaled::from_raw(1),
                link_margin: Scaled::from_raw(0),
                page_attr: token,
                resources: token,
                omit_procset: 0,
                space_font_name: 0,
            },
            token,
        );
        assert_eq!(state.pages()[0].resources_object(), 3);
        assert_eq!(state.pages()[0].contents_object(), 4);
        assert_eq!(state.pages()[0].page_object(), 2);
        let completed_hash = state.hash_fragment();

        state.rollback(initial.clone());
        assert_eq!(state.catalog_open_action(), None);
        assert_eq!(state.hash_fragment(), initial_hash);
        let replay = state
            .set_catalog_open_action(action, action.fingerprint(|_| 17), None, None)
            .expect("replay action reservation");
        assert_eq!(replay, record);
        state.rollback(initial);
        assert_eq!(state.hash_fragment(), initial_hash);
        assert_ne!(completed_hash, initial_hash);
    }

    #[test]
    fn color_stacks_are_checkpointed_and_page_and_form_state_stay_independent() {
        let mut state = PdfState::default();
        let before_hash = state.hash_fragment();
        let before = state.snapshot();
        let id = state
            .allocate_color_stack(PdfColorStackMode::Page, true, b"0 0 1 rg".to_vec())
            .expect("color stack capacity");
        assert_eq!(id, 1);
        let allocated_hash = state.hash_fragment();
        assert_ne!(allocated_hash, before_hash);

        let page = state
            .apply_color_stack(
                id,
                PdfColorStackTarget::Page,
                &PdfColorStackAction::Push(b"1 0 0 rg".to_vec()),
            )
            .expect("page push");
        assert_eq!(page.payload, b"1 0 0 rg");
        let form = state
            .apply_color_stack(id, PdfColorStackTarget::Form, &PdfColorStackAction::Current)
            .expect("form current");
        assert_eq!(form.payload, b"0 0 1 rg");
        assert_eq!(
            state.apply_color_stack(0, PdfColorStackTarget::Page, &PdfColorStackAction::Pop),
            Err(PdfColorStackApplyError::Underflow)
        );

        state.rollback(before);
        assert_eq!(
            state.allocate_color_stack(PdfColorStackMode::Page, true, b"0 0 1 rg".to_vec()),
            Ok(1)
        );
        assert_eq!(state.hash_fragment(), allocated_hash);
    }

    #[test]
    fn saved_positions_and_snap_reference_rollback_and_replay_exactly() {
        let mut state = PdfState::default();
        let before = state.snapshot();
        state.publish_traversal_positions(
            Some((Scaled::from_raw(17), Scaled::from_raw(-23))),
            (Scaled::from_raw(31), Scaled::from_raw(47)),
        );
        let changed = state.hash_fragment();
        assert_eq!(
            state.last_position(),
            (Scaled::from_raw(17), Scaled::from_raw(-23))
        );
        assert_eq!(
            state.snap_reference(),
            (Scaled::from_raw(31), Scaled::from_raw(47))
        );
        state.rollback(before.clone());
        assert_eq!(
            state.last_position(),
            (Scaled::from_raw(0), Scaled::from_raw(0))
        );
        state.publish_traversal_positions(
            Some((Scaled::from_raw(17), Scaled::from_raw(-23))),
            (Scaled::from_raw(31), Scaled::from_raw(47)),
        );
        assert_eq!(state.hash_fragment(), changed);
    }

    #[test]
    fn destination_maps_are_disjoint_duplicate_aware_and_rollback_safe() {
        let mut state = PdfState::default();
        state.enable();
        let checkpoint = state.snapshot();
        let initial_hash = state.hash_fragment();
        let identity = PdfDestinationIdentity::Name(b"same".to_vec());
        let regular = state
            .reserve_destination(identity.clone(), false)
            .expect("regular reservation");
        let structure = state
            .reserve_destination(identity.clone(), true)
            .expect("structure reservation");
        assert_eq!((regular.object(), structure.object()), (1, 2));
        assert!(
            !state
                .define_destination(identity.clone(), None)
                .expect("regular definition")
                .duplicate
        );
        assert!(
            state
                .define_destination(identity.clone(), None)
                .expect("regular duplicate")
                .duplicate
        );
        let structure_definition = state
            .define_destination(identity.clone(), Some(99))
            .expect("structure definition");
        assert!(!structure_definition.duplicate);
        assert_eq!(structure_definition.record.structure(), Some(99));
        let completed_hash = state.hash_fragment();

        state.rollback(checkpoint.clone());
        assert!(state.destinations(false).is_empty());
        assert!(state.destinations(true).is_empty());
        assert_eq!(state.hash_fragment(), initial_hash);
        assert_eq!(
            state
                .reserve_destination(identity, false)
                .expect("replay")
                .object(),
            1
        );
        state.rollback(checkpoint);
        assert_ne!(completed_hash, initial_hash);
    }

    #[test]
    fn outlines_allocate_action_item_title_and_rollback_as_one_ledger_entry() {
        let mut state = PdfState::default();
        state.enable();
        let checkpoint = state.snapshot();
        let record = state
            .create_outline(
                TokenListId::EMPTY,
                PdfActionSpec::User(TokenListId::EMPTY),
                -2,
                TokenListId::EMPTY,
                [1, 2, 3],
            )
            .expect("outline");
        assert_eq!(
            (
                record.action_object(),
                record.item_object(),
                record.title_object()
            ),
            (1, 2, 3)
        );
        assert_eq!(state.next_object(), 4);
        let hash = state.hash_fragment();
        state.rollback(checkpoint.clone());
        assert!(state.outlines().is_empty());
        let replay = state
            .create_outline(
                TokenListId::EMPTY,
                PdfActionSpec::User(TokenListId::EMPTY),
                -2,
                TokenListId::EMPTY,
                [1, 2, 3],
            )
            .expect("replay");
        assert_eq!(replay, record);
        assert_eq!(state.hash_fragment(), hash);
    }
}
