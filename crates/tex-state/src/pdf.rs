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

#[derive(Clone, Copy, Debug)]
pub struct PdfFormColorRollback(u64, StateHashFragment, PdfVersionRoot);

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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PdfPayloadId(usize);

/// Dense rows participating in one exclusive PDF candidate transaction.
///
/// Outside a transaction `accepted` is the ordinary contiguous hot store.
/// Candidate creation only records a logical prefix length; accepted-only
/// suffix rows stay in place while candidate rows append to `delta`. Rejection
/// drops the delta and reveals the original rows, while acceptance truncates
/// the obsolete suffix and moves the delta into the retained allocation.
#[derive(Debug)]
pub(crate) struct PdfRows<T> {
    accepted: Vec<T>,
    base_len: Option<usize>,
    delta: Vec<T>,
}

impl<T> Default for PdfRows<T> {
    fn default() -> Self {
        Self {
            accepted: Vec::new(),
            base_len: None,
            delta: Vec::new(),
        }
    }
}

impl<T> PdfRows<T> {
    fn from_vec(accepted: Vec<T>) -> Self {
        Self {
            accepted,
            base_len: None,
            delta: Vec::new(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.base_len.unwrap_or(self.accepted.len()) + self.delta.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn retained_floor(&self) -> usize {
        self.base_len.unwrap_or(0)
    }

    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.accepted[..self.base_len.unwrap_or(self.accepted.len())]
            .iter()
            .chain(&self.delta)
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        let base_len = self.base_len.unwrap_or(self.accepted.len());
        if index < base_len {
            self.accepted.get(index)
        } else {
            self.delta.get(index - base_len)
        }
    }

    fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let base_len = self.base_len.unwrap_or(self.accepted.len());
        if index < base_len {
            self.accepted.get_mut(index)
        } else {
            self.delta.get_mut(index - base_len)
        }
    }

    fn last(&self) -> Option<&T> {
        self.delta.last().or_else(|| {
            self.accepted
                .get(..self.base_len.unwrap_or(self.accepted.len()))
                .and_then(<[T]>::last)
        })
    }

    fn push(&mut self, value: T) {
        if self.base_len.is_some() {
            self.delta.push(value);
        } else {
            self.accepted.push(value);
        }
    }

    fn truncate(&mut self, len: usize) {
        if let Some(base_len) = self.base_len {
            assert!(len >= base_len, "PDF transaction base is still retained");
            self.delta.truncate(len - base_len);
        } else {
            self.accepted.truncate(len);
        }
    }

    #[cfg(all(feature = "profiling", feature = "testing"))]
    fn clear(&mut self) {
        self.truncate(0);
    }

    fn begin_transaction(&mut self, base_len: usize) {
        assert!(self.base_len.is_none() && self.delta.is_empty());
        assert!(base_len <= self.accepted.len());
        self.base_len = Some(base_len);
    }

    fn reject_transaction(&mut self) {
        assert!(self.base_len.take().is_some());
        self.delta.clear();
    }

    fn accept_transaction(&mut self) {
        let base_len = self.base_len.take().expect("PDF transaction is active");
        self.accepted.truncate(base_len);
        self.accepted.append(&mut self.delta);
    }

    fn binary_search_by_key<K: Ord>(
        &self,
        key: &K,
        mut f: impl FnMut(&T) -> K,
    ) -> Result<usize, usize> {
        let mut left = 0;
        let mut right = self.len();
        while left < right {
            let middle = left + (right - left) / 2;
            match f(self.get(middle).expect("binary-search row exists")).cmp(key) {
                std::cmp::Ordering::Less => left = middle + 1,
                std::cmp::Ordering::Equal => return Ok(middle),
                std::cmp::Ordering::Greater => right = middle,
            }
        }
        Err(left)
    }
}

impl<T> Extend<T> for PdfRows<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        if self.base_len.is_some() {
            self.delta.extend(iter);
        } else {
            self.accepted.extend(iter);
        }
    }
}

impl<T> std::ops::Index<usize> for PdfRows<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("PDF dense-log index is in bounds")
    }
}

impl<T> std::ops::IndexMut<usize> for PdfRows<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index)
            .expect("PDF dense-row index is in bounds")
    }
}

impl<'a, T> IntoIterator for &'a PdfRows<T> {
    type Item = &'a T;
    type IntoIter = std::iter::Chain<std::slice::Iter<'a, T>, std::slice::Iter<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        let base_len = self.base_len.unwrap_or(self.accepted.len());
        self.accepted[..base_len].iter().chain(&self.delta)
    }
}

#[derive(Debug)]
struct PdfDenseMap<T> {
    rows: Vec<Option<T>>,
    len: usize,
}

impl<T> Default for PdfDenseMap<T> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            len: 0,
        }
    }
}

impl<T> PdfDenseMap<T> {
    fn len(&self) -> usize {
        self.len
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn get(&self, key: &u32) -> Option<&T> {
        self.rows.get(*key as usize)?.as_ref()
    }

    fn insert(&mut self, key: u32, value: T) -> Option<T> {
        let index = key as usize;
        if index >= self.rows.len() {
            self.rows.resize_with(index + 1, || None);
        }
        let old = self.rows[index].replace(value);
        self.len += usize::from(old.is_none());
        old
    }
}

impl<T> std::ops::Index<&u32> for PdfDenseMap<T> {
    type Output = T;

    fn index(&self, key: &u32) -> &Self::Output {
        self.get(key).expect("PDF dense-map key is live")
    }
}

impl<T> Extend<(u32, T)> for PdfDenseMap<T> {
    fn extend<I: IntoIterator<Item = (u32, T)>>(&mut self, iter: I) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

#[derive(Debug, Default)]
struct PdfPayloadArena {
    rows: PdfRows<Box<[u8]>>,
    bytes: usize,
    accepted_bytes: Option<usize>,
}

impl PdfPayloadArena {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn store(&mut self, bytes: Vec<u8>) -> PdfPayloadId {
        let id = PdfPayloadId(self.len());
        self.bytes = self.bytes.saturating_add(bytes.len());
        self.rows.push(bytes.into_boxed_slice());
        id
    }

    fn get(&self, id: PdfPayloadId) -> &[u8] {
        self.rows.get(id.0).expect("PDF payload id is live")
    }

    fn truncate(&mut self, len: usize) {
        self.bytes = self
            .bytes
            .saturating_sub(self.rows.iter().skip(len).map(|row| row.len()).sum());
        self.rows.truncate(len);
    }

    fn begin_transaction(&mut self, len: usize, bytes: usize) {
        assert!(self.accepted_bytes.replace(self.bytes).is_none());
        self.rows.begin_transaction(len);
        self.bytes = bytes;
    }

    fn reject_transaction(&mut self) {
        self.rows.reject_transaction();
        self.bytes = self
            .accepted_bytes
            .take()
            .expect("PDF payload transaction is active");
    }

    fn accept_transaction(&mut self) {
        self.rows.accept_transaction();
        self.accepted_bytes
            .take()
            .expect("PDF payload transaction is active");
    }
}

#[derive(Clone, Debug)]
struct PdfExternalImageEntry {
    id: PdfExternalImageId,
    identity: ContentHash,
    metadata: PdfExternalImageMetadata,
    dimensions: PdfExternalImageDimensions,
    color_space_object: i32,
    payload: PdfPayloadId,
    mask_object: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct PdfFormArtifactEntry {
    payload: PdfPayloadId,
    last_position: Option<(Scaled, Scaled)>,
    snap_reference: (Scaled, Scaled),
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
    Attribute { font: FontId, bytes: Vec<u8> },
    IncludeChars { font: FontId, chars: Vec<u8> },
    GlyphToUnicode(PdfGlyphToUnicode),
    NoBuiltinToUnicode { font: FontId },
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

impl<G> Clone for PdfTokenParameter<G> {
    fn clone(&self) -> Self {
        Self {
            tokens: self.tokens.clone(),
            semantic_id: self.semantic_id,
        }
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
    pub(crate) fn id(&self) -> TokenListId<G> {
        self.tokens.clone()
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

impl<G> Clone for PdfPageParameters<G> {
    fn clone(&self) -> Self {
        Self {
            h_origin: self.h_origin,
            v_origin: self.v_origin,
            width: self.width,
            height: self.height,
            link_margin: self.link_margin,
            page_attr: self.page_attr.clone(),
            resources: self.resources.clone(),
            omit_procset: self.omit_procset,
            space_font_name: self.space_font_name,
        }
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

impl<G> Clone for PdfPageRecord<G> {
    fn clone(&self) -> Self {
        Self {
            artifact: self.artifact,
            resources_object: self.resources_object,
            contents_object: self.contents_object,
            page_object: self.page_object,
            parameters: self.parameters.clone(),
        }
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
            attr: self.attr.clone(),
            resources: self.resources.clone(),
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

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PdfStateCursor<G> {
    enabled: bool,
    next_object: u32,
    page_count: usize,
    output_parameters: Option<PdfOutputParameters>,
    pk_mode_row: Option<usize>,
    font_operation_count: usize,
    font_resource_count: usize,
    fingerprint: StateHashFragment,
    match_fingerprint: StateHashFragment,
    external_image_count: usize,
    payload_count: usize,
    payload_bytes: usize,
    color_undo_pos: u64,
    external_image_fingerprint: StateHashFragment,
    raw_object_fingerprint: StateHashFragment,
    raw_object_count: usize,
    raw_last_object: u32,
    document_fragment_fingerprint: StateHashFragment,
    document_fragment_count: usize,
    document_objects: PdfDocumentObjectIds,
    catalog_open_action_row: Option<usize>,
    action_fingerprint: StateHashFragment,
    page_reservation_fingerprint: StateHashFragment,
    page_reservation_count: usize,
    space_font_name_count: usize,
    current_space_font_name: u32,
    space_font_name_fingerprint: StateHashFragment,
    annotation_fingerprint: StateHashFragment,
    annotation_count: usize,
    link_fingerprint: StateHashFragment,
    link_count: usize,
    open_link_fingerprint: StateHashFragment,
    open_link_count: usize,
    color_stack_fingerprint: StateHashFragment,
    color_stack_count: usize,
    last_position: (Scaled, Scaled),
    snap_reference: (Scaled, Scaled),
    form_fingerprint: StateHashFragment,
    form_count: usize,
    next_form_resource: u32,
    form_artifact_fingerprint: StateHashFragment,
    form_artifact_count: usize,
    return_value: i32,
    destination_fingerprint: StateHashFragment,
    destination_count: usize,
    structure_destination_fingerprint: StateHashFragment,
    structure_destination_count: usize,
    outline_fingerprint: StateHashFragment,
    outline_count: usize,
    thread_fingerprint: StateHashFragment,
    thread_count: usize,
    _generation: std::marker::PhantomData<fn(G) -> G>,
}

impl<G> Hash for PdfStateCursor<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.enabled.hash(state);
        self.next_object.hash(state);
        self.page_count.hash(state);
        self.output_parameters.hash(state);
        self.pk_mode_row.hash(state);
        self.font_operation_count.hash(state);
        self.font_resource_count.hash(state);
        self.fingerprint.hash(state);
        self.match_fingerprint.hash(state);
        self.external_image_count.hash(state);
        self.payload_count.hash(state);
        self.payload_bytes.hash(state);
        self.color_undo_pos.hash(state);
        self.external_image_fingerprint.hash(state);
        self.raw_object_fingerprint.hash(state);
        self.raw_object_count.hash(state);
        self.raw_last_object.hash(state);
        self.document_fragment_fingerprint.hash(state);
        self.document_fragment_count.hash(state);
        self.document_objects.hash(state);
        self.catalog_open_action_row.hash(state);
        self.action_fingerprint.hash(state);
        self.page_reservation_fingerprint.hash(state);
        self.page_reservation_count.hash(state);
        self.space_font_name_count.hash(state);
        self.current_space_font_name.hash(state);
        self.space_font_name_fingerprint.hash(state);
        self.annotation_fingerprint.hash(state);
        self.annotation_count.hash(state);
        self.link_fingerprint.hash(state);
        self.link_count.hash(state);
        self.open_link_fingerprint.hash(state);
        self.open_link_count.hash(state);
        self.color_stack_fingerprint.hash(state);
        self.color_stack_count.hash(state);
        self.last_position.hash(state);
        self.snap_reference.hash(state);
        self.form_fingerprint.hash(state);
        self.form_count.hash(state);
        self.next_form_resource.hash(state);
        self.form_artifact_fingerprint.hash(state);
        self.form_artifact_count.hash(state);
        self.return_value.hash(state);
        self.destination_fingerprint.hash(state);
        self.destination_count.hash(state);
        self.structure_destination_fingerprint.hash(state);
        self.structure_destination_count.hash(state);
        self.outline_fingerprint.hash(state);
        self.outline_count.hash(state);
        self.thread_fingerprint.hash(state);
        self.thread_count.hash(state);
    }
}

impl<G> Clone for PdfStateCursor<G> {
    fn clone(&self) -> Self {
        Self {
            enabled: self.enabled,
            next_object: self.next_object,
            page_count: self.page_count,
            output_parameters: self.output_parameters,
            pk_mode_row: self.pk_mode_row,
            font_operation_count: self.font_operation_count,
            font_resource_count: self.font_resource_count,
            fingerprint: self.fingerprint,
            match_fingerprint: self.match_fingerprint,
            external_image_count: self.external_image_count,
            payload_count: self.payload_count,
            payload_bytes: self.payload_bytes,
            color_undo_pos: self.color_undo_pos,
            external_image_fingerprint: self.external_image_fingerprint,
            raw_object_fingerprint: self.raw_object_fingerprint,
            raw_object_count: self.raw_object_count,
            raw_last_object: self.raw_last_object,
            document_fragment_fingerprint: self.document_fragment_fingerprint,
            document_fragment_count: self.document_fragment_count,
            document_objects: self.document_objects,
            catalog_open_action_row: self.catalog_open_action_row,
            action_fingerprint: self.action_fingerprint,
            page_reservation_fingerprint: self.page_reservation_fingerprint,
            page_reservation_count: self.page_reservation_count,
            space_font_name_count: self.space_font_name_count,
            current_space_font_name: self.current_space_font_name,
            space_font_name_fingerprint: self.space_font_name_fingerprint,
            annotation_fingerprint: self.annotation_fingerprint,
            annotation_count: self.annotation_count,
            link_fingerprint: self.link_fingerprint,
            link_count: self.link_count,
            open_link_fingerprint: self.open_link_fingerprint,
            open_link_count: self.open_link_count,
            color_stack_fingerprint: self.color_stack_fingerprint,
            color_stack_count: self.color_stack_count,
            last_position: self.last_position,
            snap_reference: self.snap_reference,
            form_fingerprint: self.form_fingerprint,
            form_count: self.form_count,
            next_form_resource: self.next_form_resource,
            form_artifact_fingerprint: self.form_artifact_fingerprint,
            form_artifact_count: self.form_artifact_count,
            return_value: self.return_value,
            destination_fingerprint: self.destination_fingerprint,
            destination_count: self.destination_count,
            structure_destination_fingerprint: self.structure_destination_fingerprint,
            structure_destination_count: self.structure_destination_count,
            outline_fingerprint: self.outline_fingerprint,
            outline_count: self.outline_count,
            thread_fingerprint: self.thread_fingerprint,
            thread_count: self.thread_count,
            _generation: std::marker::PhantomData,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PdfStateSnapshot<G> {
    cursor: PdfStateCursor<G>,
    undo_pos: u64,
    general_root: PdfVersionRoot,
    color_root: PdfVersionRoot,
}

impl<G> Clone for PdfStateSnapshot<G> {
    fn clone(&self) -> Self {
        Self {
            cursor: self.cursor.clone(),
            undo_pos: self.undo_pos,
            general_root: self.general_root,
            color_root: self.color_root,
        }
    }
}

impl<G> PdfStateSnapshot<G> {
    pub(crate) fn history_position(&self) -> (u64, u64) {
        (self.undo_pos, self.cursor.color_undo_pos)
    }

    /// Canonical O(1) semantic root published by the PDF owner.
    ///
    /// `PdfStateCursor` contains only canonical scalars, counts, row-selection
    /// frontiers, and mutation-maintained semantic fragments. The version-lane
    /// roots and undo positions are restore coordinates and deliberately do
    /// not participate.
    pub(crate) fn reachable_state_identity_root(&self) -> u64 {
        let mut hasher = StateHasher::new_exact(0x7064_665f_7273_6901);
        self.cursor.apply_reachable_state_identity(&mut hasher);
        hasher.finish()
    }
}

impl<G> PdfStateCursor<G> {
    fn apply_reachable_state_identity(&self, hasher: &mut StateHasher) {
        hasher.bool(self.enabled);
        hasher.u32(self.next_object);
        hasher.usize(self.page_count);
        hash_output_parameters(hasher, self.output_parameters);
        hash_optional_usize(hasher, self.pk_mode_row);
        hasher.usize(self.font_operation_count);
        hasher.usize(self.font_resource_count);
        self.fingerprint.apply(hasher);
        self.match_fingerprint.apply(hasher);
        hasher.usize(self.external_image_count);
        hasher.usize(self.payload_count);
        hasher.usize(self.payload_bytes);
        self.external_image_fingerprint.apply(hasher);
        self.raw_object_fingerprint.apply(hasher);
        hasher.usize(self.raw_object_count);
        hasher.u32(self.raw_last_object);
        self.document_fragment_fingerprint.apply(hasher);
        hasher.usize(self.document_fragment_count);
        for object in [
            self.document_objects.pages(),
            self.document_objects.names(),
            self.document_objects.catalog(),
            self.document_objects.info(),
        ] {
            hash_optional_u32(hasher, object);
        }
        hash_optional_usize(hasher, self.catalog_open_action_row);
        self.action_fingerprint.apply(hasher);
        self.page_reservation_fingerprint.apply(hasher);
        hasher.usize(self.page_reservation_count);
        hasher.usize(self.space_font_name_count);
        hasher.u32(self.current_space_font_name);
        self.space_font_name_fingerprint.apply(hasher);
        self.annotation_fingerprint.apply(hasher);
        hasher.usize(self.annotation_count);
        self.link_fingerprint.apply(hasher);
        hasher.usize(self.link_count);
        self.open_link_fingerprint.apply(hasher);
        hasher.usize(self.open_link_count);
        self.color_stack_fingerprint.apply(hasher);
        hasher.usize(self.color_stack_count);
        hasher.i32(self.last_position.0.raw());
        hasher.i32(self.last_position.1.raw());
        hasher.i32(self.snap_reference.0.raw());
        hasher.i32(self.snap_reference.1.raw());
        self.form_fingerprint.apply(hasher);
        hasher.usize(self.form_count);
        hasher.u32(self.next_form_resource);
        self.form_artifact_fingerprint.apply(hasher);
        hasher.usize(self.form_artifact_count);
        hasher.i32(self.return_value);
        self.destination_fingerprint.apply(hasher);
        hasher.usize(self.destination_count);
        self.structure_destination_fingerprint.apply(hasher);
        hasher.usize(self.structure_destination_count);
        self.outline_fingerprint.apply(hasher);
        hasher.usize(self.outline_count);
        self.thread_fingerprint.apply(hasher);
        hasher.usize(self.thread_count);
    }
}

fn hash_optional_usize(hasher: &mut StateHasher, value: Option<usize>) {
    hasher.bool(value.is_some());
    if let Some(value) = value {
        hasher.usize(value);
    }
}

fn hash_optional_u32(hasher: &mut StateHasher, value: Option<u32>) {
    hasher.bool(value.is_some());
    if let Some(value) = value {
        hasher.u32(value);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
struct PdfVersionRoot(Option<u32>);

#[derive(Clone, Copy, Debug, Default)]
struct PdfVersionIndexNode {
    children: [Option<u32>; 2],
    value: Option<u32>,
}

#[derive(Debug, Default)]
struct PdfVersionIndex {
    accepted: Vec<PdfVersionIndexNode>,
    candidate: Vec<PdfVersionIndexNode>,
}

impl PdfVersionIndex {
    #[cfg(all(feature = "profiling", feature = "testing"))]
    const PROBES: u32 = u64::BITS;

    fn node(&self, index: u32) -> PdfVersionIndexNode {
        let index = index as usize;
        if index < self.accepted.len() {
            self.accepted[index]
        } else {
            self.candidate[index - self.accepted.len()]
        }
    }

    fn get(&self, root: PdfVersionRoot, key: u64) -> Option<u32> {
        let mut node = root.0?;
        for shift in (0..u64::BITS).rev() {
            node = self.node(node).children[((key >> shift) & 1) as usize]?;
        }
        self.node(node).value
    }

    fn insert(
        &mut self,
        root: PdfVersionRoot,
        key: u64,
        value: u32,
        candidate: bool,
    ) -> PdfVersionRoot {
        let mut path = [None; u64::BITS as usize + 1];
        path[0] = root.0;
        for (depth, shift) in (0..u64::BITS).rev().enumerate() {
            path[depth + 1] = path[depth]
                .and_then(|node| self.node(node).children[((key >> shift) & 1) as usize]);
        }

        let mut leaf = path[u64::BITS as usize]
            .map_or_else(PdfVersionIndexNode::default, |node| self.node(node));
        leaf.value = Some(value);
        let mut child = self.push(leaf, candidate);
        for depth in (0..u64::BITS as usize).rev() {
            let shift = u64::BITS as usize - depth - 1;
            let branch = ((key >> shift) & 1) as usize;
            let mut parent =
                path[depth].map_or_else(PdfVersionIndexNode::default, |node| self.node(node));
            parent.children[branch] = Some(child);
            child = self.push(parent, candidate);
        }
        PdfVersionRoot(Some(child))
    }

    fn push(&mut self, node: PdfVersionIndexNode, candidate: bool) -> u32 {
        let absolute = self.accepted.len() + self.candidate.len();
        let absolute = u32::try_from(absolute).expect("PDF version-index capacity");
        if candidate {
            self.candidate.push(node);
        } else {
            debug_assert!(self.candidate.is_empty());
            self.accepted.push(node);
        }
        absolute
    }

    fn reject_candidate(&mut self) {
        self.candidate.clear();
    }

    fn accept_candidate(&mut self) {
        self.accepted.append(&mut self.candidate);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfGeneralVersionKey {
    Match,
    RawObject(u32),
    Annotation(u32),
    OpenLinks,
    FormArtifact(u32),
    Destination {
        structure: bool,
        row: u32,
    },
    Thread(u32),
    Color {
        row: u32,
        target: PdfColorStackTarget,
    },
}

impl PdfGeneralVersionKey {
    const fn packed(self) -> u64 {
        match self {
            Self::Match => 0,
            Self::RawObject(row) => (1_u64 << 56) | row as u64,
            Self::Annotation(row) => (2_u64 << 56) | row as u64,
            Self::OpenLinks => 3_u64 << 56,
            Self::FormArtifact(object) => (4_u64 << 56) | object as u64,
            Self::Destination { structure, row } => {
                (5_u64 << 56) | ((structure as u64) << 55) | row as u64
            }
            Self::Thread(row) => (6_u64 << 56) | row as u64,
            Self::Color { row, target } => {
                (7_u64 << 56)
                    | ((matches!(target, PdfColorStackTarget::Form) as u64) << 55)
                    | row as u64
            }
        }
    }
}

#[derive(Clone, Debug)]
enum PdfVersionValue<G> {
    Match(PdfMatchState),
    RawObject(PdfRawObjectRecord<G>),
    Annotation {
        data: Option<PdfAnnotationData<G>>,
    },
    OpenLinks(Option<u32>),
    FormArtifact {
        entry: Option<PdfFormArtifactEntry>,
    },
    Destination {
        structure: Option<u32>,
        defined: bool,
    },
    Thread {
        bead_head: Option<u32>,
        len: u32,
    },
    Color(PdfColorRuntimeRoot),
}

#[derive(Debug)]
struct PdfCandidateTransaction<G> {
    accepted: PdfStateSnapshot<G>,
    base: PdfStateSnapshot<G>,
    undo_low_water: u64,
    color_undo_low_water: u64,
}

#[derive(Debug)]
struct PdfBranchArena<T> {
    accepted: Vec<T>,
    candidate: Vec<T>,
}

impl<T> Default for PdfBranchArena<T> {
    fn default() -> Self {
        Self {
            accepted: Vec::new(),
            candidate: Vec::new(),
        }
    }
}

impl<T> PdfBranchArena<T> {
    fn get(&self, index: u32) -> Option<&T> {
        let index = index as usize;
        if index < self.accepted.len() {
            self.accepted.get(index)
        } else {
            self.candidate.get(index - self.accepted.len())
        }
    }

    fn push(&mut self, value: T, candidate: bool) -> u32 {
        let absolute = self.accepted.len() + self.candidate.len();
        let absolute = u32::try_from(absolute).expect("PDF branch-arena capacity");
        if candidate {
            self.candidate.push(value);
        } else {
            debug_assert!(self.candidate.is_empty());
            self.accepted.push(value);
        }
        absolute
    }

    fn reject_candidate(&mut self) {
        self.candidate.clear();
    }

    fn accept_candidate(&mut self) {
        self.accepted.append(&mut self.candidate);
    }
}

#[derive(Debug)]
struct PdfOpenLinkNode<G> {
    value: PdfOpenLink<G>,
    previous: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct PdfThreadBeadNode {
    value: PdfThreadBeadRecord,
    previous: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct PdfColorRuntimeRoot {
    current: u32,
    pushed: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct PdfColorPushNode {
    value: u32,
    previous: Option<u32>,
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
    pages: PdfRows<PdfPageRecord<G>>,
    output_parameters: Option<PdfOutputParameters>,
    pk_modes: PdfRows<PdfTokenParameter<G>>,
    pk_mode_row: Option<usize>,
    font_operations: PdfRows<PdfFontOperation>,
    font_resources: PdfRows<PdfFontResourceRecord>,
    fingerprint: StateHashFragment,
    match_state: PdfMatchState,
    external_images: PdfRows<PdfExternalImageEntry>,
    payloads: PdfPayloadArena,
    external_image_fingerprint: StateHashFragment,
    raw_objects: PdfRawObjects<G>,
    document_fragments: PdfDocumentFragments<G>,
    document_objects: PdfDocumentObjectIds,
    catalog_open_actions: PdfRows<PdfActionRecord<G>>,
    catalog_open_action_row: Option<usize>,
    action_fingerprint: StateHashFragment,
    page_reservations: PdfRows<PdfPageReservation>,
    page_reservation_fingerprint: StateHashFragment,
    space_font_names: PdfRows<Vec<u8>>,
    space_font_name_lookup: BTreeMap<Vec<u8>, u32>,
    space_font_name_delta_lookup: BTreeMap<Vec<u8>, u32>,
    current_space_font_name: u32,
    space_font_name_fingerprint: StateHashFragment,
    annotations: PdfRows<PdfAnnotationRecord<G>>,
    annotation_fingerprint: StateHashFragment,
    links: PdfRows<PdfLinkRecord<G>>,
    link_fingerprint: StateHashFragment,
    open_links: PdfRows<PdfOpenLink<G>>,
    open_link_fingerprint: StateHashFragment,
    color_stacks: PdfRows<PdfColorStack>,
    color_stack_fingerprint: StateHashFragment,
    last_position: (Scaled, Scaled),
    snap_reference: (Scaled, Scaled),
    forms: PdfRows<PdfFormRecord<G>>,
    form_fingerprint: StateHashFragment,
    next_form_resource: u32,
    form_artifacts: PdfDenseMap<PdfFormArtifactEntry>,
    form_artifact_fingerprint: StateHashFragment,
    return_value: i32,
    destinations: PdfRows<PdfDestinationRecord>,
    destination_fingerprint: StateHashFragment,
    structure_destinations: PdfRows<PdfDestinationRecord>,
    structure_destination_fingerprint: StateHashFragment,
    outlines: PdfRows<PdfOutlineRecord<G>>,
    outline_fingerprint: StateHashFragment,
    threads: PdfRows<PdfThreadRecord>,
    thread_fingerprint: StateHashFragment,
    general_root: PdfVersionRoot,
    general_index: PdfVersionIndex,
    color_root: PdfVersionRoot,
    color_index: PdfVersionIndex,
    general_versions: PdfBranchArena<PdfVersionValue<G>>,
    open_link_nodes: PdfBranchArena<PdfOpenLinkNode<G>>,
    thread_bead_nodes: PdfBranchArena<PdfThreadBeadNode>,
    color_values: PdfBranchArena<Box<[u8]>>,
    color_push_nodes: PdfBranchArena<PdfColorPushNode>,
    undo_base: u64,
    undo_len: u64,
    candidate_undo_len: u64,
    color_undo_base: u64,
    color_undo_len: u64,
    candidate_color_undo_len: u64,
    transaction: Option<PdfCandidateTransaction<G>>,
}

/// Type-state slot for the unique mutable PDF authority.
///
/// An accepted generation becomes `Loaned` for the full candidate lifetime;
/// only the candidate owns the direct mutable state. The reachability store
/// returns or commits that state before either physical slot is released.
#[derive(Debug)]
pub(crate) enum PdfStateSlot<G> {
    Owned(Box<PdfState<G>>),
    Loaned,
}

impl<G> Default for PdfStateSlot<G> {
    fn default() -> Self {
        Self::Owned(Box::default())
    }
}

impl<G> PdfStateSlot<G> {
    pub(crate) fn take_candidate(&mut self, base: &PdfStateSnapshot<G>) -> Self {
        let Self::Owned(mut state) = std::mem::replace(self, Self::Loaned) else {
            panic!("accepted PDF state already has an exclusive candidate");
        };
        state.open_candidate_lineage(base);
        Self::Owned(state)
    }

    pub(crate) fn return_rejected(&mut self, candidate: &mut Self) {
        assert!(matches!(self, Self::Loaned));
        let Self::Owned(mut state) = std::mem::replace(candidate, Self::Loaned) else {
            panic!("rejected PDF candidate owns its transaction");
        };
        state.reject_candidate_transaction();
        *self = Self::Owned(state);
    }

    pub(crate) fn commit_candidate(&mut self) {
        let Self::Owned(state) = self else {
            panic!("committed PDF candidate owns its transaction");
        };
        state.accept_candidate_transaction();
    }
}

impl<G> std::ops::Deref for PdfStateSlot<G> {
    type Target = PdfState<G>;

    fn deref(&self) -> &Self::Target {
        let Self::Owned(state) = self else {
            panic!("accepted PDF state is exclusively owned by its candidate");
        };
        state
    }
}

impl<G> std::ops::DerefMut for PdfStateSlot<G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let Self::Owned(state) = self else {
            panic!("accepted PDF state is exclusively owned by its candidate");
        };
        state
    }
}

impl<G> PdfState<G> {
    #[cfg(feature = "profiling")]
    pub(crate) fn payload_bytes(&self) -> usize {
        self.payloads.bytes
    }

    pub(crate) fn checkpoint_retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.payloads.bytes)
            .saturating_add(
                self.pages
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfPageRecord<G>>()),
            )
            .saturating_add(
                self.font_operations
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfFontOperation>()),
            )
            .saturating_add(
                self.font_resources
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfFontResourceRecord>()),
            )
            .saturating_add(
                self.external_images
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfExternalImageEntry>()),
            )
            .saturating_add(
                self.annotations
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfAnnotationRecord<G>>()),
            )
            .saturating_add(
                self.links
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfLinkRecord<G>>()),
            )
            .saturating_add(
                self.color_stacks
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfColorStack>()),
            )
            .saturating_add(
                self.forms
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfFormRecord<G>>()),
            )
            .saturating_add(
                self.destinations
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfDestinationRecord>()),
            )
            .saturating_add(
                self.outlines
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfOutlineRecord<G>>()),
            )
            .saturating_add(
                self.threads
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfThreadRecord>()),
            )
            .saturating_add(
                (self.general_index.accepted.len() + self.general_index.candidate.len())
                    .saturating_mul(std::mem::size_of::<PdfVersionIndexNode>()),
            )
            .saturating_add(
                (self.color_index.accepted.len() + self.color_index.candidate.len())
                    .saturating_mul(std::mem::size_of::<PdfVersionIndexNode>()),
            )
            .saturating_add(
                (self.general_versions.accepted.len() + self.general_versions.candidate.len())
                    .saturating_mul(std::mem::size_of::<PdfVersionValue<G>>()),
            )
            .saturating_add(
                (self.open_link_nodes.accepted.len() + self.open_link_nodes.candidate.len())
                    .saturating_mul(std::mem::size_of::<PdfOpenLinkNode<G>>()),
            )
            .saturating_add(
                (self.thread_bead_nodes.accepted.len() + self.thread_bead_nodes.candidate.len())
                    .saturating_mul(std::mem::size_of::<PdfThreadBeadNode>()),
            )
            .saturating_add(
                (self.color_values.accepted.len() + self.color_values.candidate.len())
                    .saturating_mul(std::mem::size_of::<Box<[u8]>>()),
            )
            .saturating_add(
                (self.color_push_nodes.accepted.len() + self.color_push_nodes.candidate.len())
                    .saturating_mul(std::mem::size_of::<PdfColorPushNode>()),
            )
    }

    pub(crate) fn history_head(&self) -> (u64, u64) {
        if let Some(transaction) = &self.transaction {
            (
                transaction.base.undo_pos + self.candidate_undo_len,
                transaction.base.cursor.color_undo_pos + self.candidate_color_undo_len,
            )
        } else {
            (
                self.undo_base + self.undo_len,
                self.color_undo_base + self.color_undo_len,
            )
        }
    }

    pub(crate) fn open_candidate_lineage(&mut self, base: &PdfStateSnapshot<G>) {
        assert!(self.transaction.is_none());
        assert!(self.snapshot_is_retained(base));
        assert!(self.candidate_undo_len == 0 && self.candidate_color_undo_len == 0);
        let accepted = self.snapshot();

        let cursor = &base.cursor;
        self.pages.begin_transaction(cursor.page_count);
        self.pk_modes
            .begin_transaction(cursor.pk_mode_row.map_or(0, |row| row + 1));
        self.font_operations
            .begin_transaction(cursor.font_operation_count);
        self.font_resources
            .begin_transaction(cursor.font_resource_count);
        self.external_images
            .begin_transaction(cursor.external_image_count);
        self.raw_objects.begin_transaction(cursor.raw_object_count);
        self.document_fragments
            .begin_transaction(cursor.document_fragment_count);
        self.catalog_open_actions
            .begin_transaction(cursor.catalog_open_action_row.map_or(0, |row| row + 1));
        self.page_reservations
            .begin_transaction(cursor.page_reservation_count);
        self.space_font_names
            .begin_transaction(cursor.space_font_name_count);
        self.annotations.begin_transaction(cursor.annotation_count);
        self.links.begin_transaction(cursor.link_count);
        self.color_stacks
            .begin_transaction(cursor.color_stack_count);
        self.forms.begin_transaction(cursor.form_count);
        self.destinations
            .begin_transaction(cursor.destination_count);
        self.structure_destinations
            .begin_transaction(cursor.structure_destination_count);
        self.outlines.begin_transaction(cursor.outline_count);
        self.threads.begin_transaction(cursor.thread_count);
        self.payloads
            .begin_transaction(cursor.payload_count, cursor.payload_bytes);
        self.restore_cursor_scalars(cursor);
        self.general_root = base.general_root;
        self.color_root = base.color_root;
        self.candidate_undo_len = 0;
        self.candidate_color_undo_len = 0;
        self.transaction = Some(PdfCandidateTransaction {
            accepted,
            base: base.clone(),
            undo_low_water: base.undo_pos,
            color_undo_low_water: base.cursor.color_undo_pos,
        });
    }

    pub(crate) fn reject_candidate_transaction(&mut self) {
        let base = self
            .transaction
            .as_ref()
            .expect("PDF candidate transaction is active")
            .base
            .clone();
        self.rollback(base);
        let transaction = self.transaction.take().expect("PDF transaction exists");
        self.reject_row_transaction();
        self.general_root = transaction.accepted.general_root;
        self.color_root = transaction.accepted.color_root;
        self.general_index.reject_candidate();
        self.color_index.reject_candidate();
        self.general_versions.reject_candidate();
        self.open_link_nodes.reject_candidate();
        self.thread_bead_nodes.reject_candidate();
        self.color_values.reject_candidate();
        self.color_push_nodes.reject_candidate();
        self.candidate_undo_len = 0;
        self.candidate_color_undo_len = 0;
        self.restore_cursor_scalars(&transaction.accepted.cursor);
    }

    pub(crate) fn accept_candidate_transaction(&mut self) {
        let transaction = self.transaction.take().expect("PDF transaction exists");
        self.accept_row_transaction();
        self.general_index.accept_candidate();
        self.color_index.accept_candidate();
        self.general_versions.accept_candidate();
        self.open_link_nodes.accept_candidate();
        self.thread_bead_nodes.accept_candidate();
        self.color_values.accept_candidate();
        self.color_push_nodes.accept_candidate();
        self.undo_base = transaction.undo_low_water;
        self.undo_len =
            self.candidate_undo_len - (transaction.undo_low_water - transaction.base.undo_pos);
        self.candidate_undo_len = 0;
        self.color_undo_base = transaction.color_undo_low_water;
        self.color_undo_len = self.candidate_color_undo_len
            - (transaction.color_undo_low_water - transaction.base.cursor.color_undo_pos);
        self.candidate_color_undo_len = 0;
    }

    fn reject_row_transaction(&mut self) {
        self.pages.reject_transaction();
        self.pk_modes.reject_transaction();
        self.font_operations.reject_transaction();
        self.font_resources.reject_transaction();
        self.external_images.reject_transaction();
        self.raw_objects.reject_transaction();
        self.document_fragments.reject_transaction();
        self.catalog_open_actions.reject_transaction();
        self.page_reservations.reject_transaction();
        self.space_font_names.reject_transaction();
        self.space_font_name_delta_lookup.clear();
        self.annotations.reject_transaction();
        self.links.reject_transaction();
        self.color_stacks.reject_transaction();
        self.forms.reject_transaction();
        self.destinations.reject_transaction();
        self.structure_destinations.reject_transaction();
        self.outlines.reject_transaction();
        self.threads.reject_transaction();
        self.payloads.reject_transaction();
    }

    fn accept_row_transaction(&mut self) {
        self.pages.accept_transaction();
        self.pk_modes.accept_transaction();
        self.font_operations.accept_transaction();
        self.font_resources.accept_transaction();
        self.external_images.accept_transaction();
        self.raw_objects.accept_transaction();
        self.document_fragments.accept_transaction();
        self.catalog_open_actions.accept_transaction();
        self.page_reservations.accept_transaction();
        let space_base = self
            .space_font_names
            .base_len
            .expect("PDF space-name transaction is active");
        self.space_font_name_lookup
            .retain(|_, id| (*id as usize) < space_base);
        self.space_font_names.accept_transaction();
        self.space_font_name_lookup
            .append(&mut self.space_font_name_delta_lookup);
        self.annotations.accept_transaction();
        self.links.accept_transaction();
        self.color_stacks.accept_transaction();
        self.forms.accept_transaction();
        self.destinations.accept_transaction();
        self.structure_destinations.accept_transaction();
        self.outlines.accept_transaction();
        self.threads.accept_transaction();
        self.payloads.accept_transaction();
    }

    fn restore_cursor_scalars(&mut self, cursor: &PdfStateCursor<G>) {
        self.enabled = cursor.enabled;
        self.next_object = cursor.next_object;
        self.output_parameters = cursor.output_parameters;
        self.pk_mode_row = cursor.pk_mode_row;
        self.fingerprint = cursor.fingerprint;
        self.external_image_fingerprint = cursor.external_image_fingerprint;
        self.raw_objects
            .set_fingerprint(cursor.raw_object_fingerprint);
        self.raw_objects.set_last_object(cursor.raw_last_object);
        self.document_fragments
            .set_fingerprint(cursor.document_fragment_fingerprint);
        self.document_objects = cursor.document_objects;
        self.catalog_open_action_row = cursor.catalog_open_action_row;
        self.action_fingerprint = cursor.action_fingerprint;
        self.page_reservation_fingerprint = cursor.page_reservation_fingerprint;
        self.current_space_font_name = cursor.current_space_font_name;
        self.space_font_name_fingerprint = cursor.space_font_name_fingerprint;
        self.annotation_fingerprint = cursor.annotation_fingerprint;
        self.link_fingerprint = cursor.link_fingerprint;
        self.open_link_fingerprint = cursor.open_link_fingerprint;
        self.color_stack_fingerprint = cursor.color_stack_fingerprint;
        self.last_position = cursor.last_position;
        self.snap_reference = cursor.snap_reference;
        self.form_fingerprint = cursor.form_fingerprint;
        self.next_form_resource = cursor.next_form_resource;
        self.form_artifact_fingerprint = cursor.form_artifact_fingerprint;
        self.return_value = cursor.return_value;
        self.destination_fingerprint = cursor.destination_fingerprint;
        self.structure_destination_fingerprint = cursor.structure_destination_fingerprint;
        self.outline_fingerprint = cursor.outline_fingerprint;
        self.thread_fingerprint = cursor.thread_fingerprint;
    }

    pub(crate) fn prune_history(&mut self, low_water: (u64, u64)) {
        if let Some(transaction) = &mut self.transaction {
            let undo_head = transaction.base.undo_pos + self.candidate_undo_len;
            let color_head = transaction.base.cursor.color_undo_pos + self.candidate_color_undo_len;
            assert!(low_water.0 >= transaction.undo_low_water && low_water.0 <= undo_head);
            assert!(low_water.1 >= transaction.color_undo_low_water && low_water.1 <= color_head);
            transaction.undo_low_water = low_water.0;
            transaction.color_undo_low_water = low_water.1;
            return;
        }
        let undo_head = self.undo_base + self.undo_len;
        let color_head = self.color_undo_base + self.color_undo_len;
        assert!(low_water.0 >= self.undo_base && low_water.0 <= undo_head);
        assert!(low_water.1 >= self.color_undo_base && low_water.1 <= color_head);
        self.undo_len = undo_head - low_water.0;
        self.undo_base = low_water.0;
        self.color_undo_len = color_head - low_water.1;
        self.color_undo_base = low_water.1;
    }

    fn general_version(&self, key: PdfGeneralVersionKey) -> Option<&PdfVersionValue<G>> {
        let event = self.general_index.get(self.general_root, key.packed())?;
        self.general_versions.get(event)
    }

    fn push_general_version(&mut self, key: PdfGeneralVersionKey, value: PdfVersionValue<G>) {
        let candidate = self.transaction.is_some();
        let event = self.general_versions.push(value, candidate);
        self.general_root =
            self.general_index
                .insert(self.general_root, key.packed(), event, candidate);
        if candidate {
            self.candidate_undo_len += 1;
        } else {
            self.undo_len += 1;
        }
    }

    fn thread_state(&self, row: usize) -> (Option<u32>, u32) {
        match self.general_version(PdfGeneralVersionKey::Thread(row as u32)) {
            Some(PdfVersionValue::Thread { bead_head, len }) => (*bead_head, *len),
            Some(_) => unreachable!("PDF thread version key has one value family"),
            None => (None, 0),
        }
    }

    fn thread_record(&self, row: usize) -> PdfThreadRecord {
        let (mut head, len) = self.thread_state(row);
        let mut beads = Vec::with_capacity(len as usize);
        while let Some(index) = head {
            let node = self
                .thread_bead_nodes
                .get(index)
                .expect("PDF thread-bead root is live");
            beads.push(node.value);
            head = node.previous;
        }
        beads.reverse();
        self.threads[row].clone().with_beads(beads)
    }

    fn open_link_root(&self) -> Option<u32> {
        match self.general_version(PdfGeneralVersionKey::OpenLinks) {
            Some(PdfVersionValue::OpenLinks(root)) => *root,
            Some(_) => unreachable!("PDF open-link version key has one value family"),
            None => None,
        }
    }

    fn open_link_values(&self) -> Vec<PdfOpenLink<G>> {
        let mut root = self.open_link_root();
        let mut values = Vec::new();
        while let Some(index) = root {
            let node = self
                .open_link_nodes
                .get(index)
                .expect("PDF open-link root is live");
            values.push(node.value.clone());
            root = node.previous;
        }
        values.reverse();
        values
    }

    fn color_version(
        &self,
        row: usize,
        target: PdfColorStackTarget,
    ) -> Option<PdfColorRuntimeRoot> {
        let key = PdfGeneralVersionKey::Color {
            row: row as u32,
            target,
        };
        let event = self.color_index.get(self.color_root, key.packed())?;
        match self.general_versions.get(event) {
            Some(PdfVersionValue::Color(root)) => Some(*root),
            Some(_) => unreachable!("PDF color version key has one value family"),
            None => unreachable!("PDF color version root is live"),
        }
    }

    fn push_color_version(
        &mut self,
        row: usize,
        target: PdfColorStackTarget,
        root: PdfColorRuntimeRoot,
    ) {
        let candidate = self.transaction.is_some();
        let event = self
            .general_versions
            .push(PdfVersionValue::Color(root), candidate);
        let key = PdfGeneralVersionKey::Color {
            row: row as u32,
            target,
        };
        self.color_root = self
            .color_index
            .insert(self.color_root, key.packed(), event, candidate);
        if candidate {
            self.candidate_color_undo_len += 1;
        } else {
            self.color_undo_len += 1;
        }
    }

    fn store_color_value(&mut self, value: Vec<u8>) -> u32 {
        self.color_values
            .push(value.into_boxed_slice(), self.transaction.is_some())
    }

    fn color_value(&self, value: u32) -> &[u8] {
        self.color_values
            .get(value)
            .expect("PDF color value root is live")
    }

    fn materialize_color_runtime(
        &mut self,
        row: usize,
        target: PdfColorStackTarget,
    ) -> PdfColorRuntimeRoot {
        if let Some(root) = self.color_version(row, target) {
            return root;
        }
        let runtime = match target {
            PdfColorStackTarget::Page => self.color_stacks[row].page.clone(),
            PdfColorStackTarget::Form => self.color_stacks[row].form.clone(),
        };
        let candidate = self.transaction.is_some();
        let mut pushed = None;
        for value in runtime.pushed {
            let value = self.color_values.push(value.into_boxed_slice(), candidate);
            pushed = Some(self.color_push_nodes.push(
                PdfColorPushNode {
                    value,
                    previous: pushed,
                },
                candidate,
            ));
        }
        PdfColorRuntimeRoot {
            current: self
                .color_values
                .push(runtime.current.into_boxed_slice(), candidate),
            pushed,
        }
    }

    fn color_current_bytes(&self, row: usize, target: PdfColorStackTarget) -> &[u8] {
        if let Some(root) = self.color_version(row, target) {
            return self.color_value(root.current);
        }
        match target {
            PdfColorStackTarget::Page => &self.color_stacks[row].page.current,
            PdfColorStackTarget::Form => &self.color_stacks[row].form.current,
        }
    }
}

impl<G> Default for PdfState<G> {
    fn default() -> Self {
        let default_space_font = b"pdftexspace".to_vec();
        Self {
            enabled: false,
            next_object: FIRST_DYNAMIC_OBJECT,
            pages: PdfRows::default(),
            output_parameters: None,
            pk_modes: PdfRows::default(),
            pk_mode_row: None,
            font_operations: PdfRows::default(),
            font_resources: PdfRows::default(),
            fingerprint: base_fingerprint(false),
            match_state: PdfMatchState::default(),
            external_images: PdfRows::default(),
            payloads: PdfPayloadArena::default(),
            external_image_fingerprint: external_image_base_fingerprint(),
            raw_objects: PdfRawObjects::<G>::default(),
            document_fragments: PdfDocumentFragments::<G>::default(),
            document_objects: PdfDocumentObjectIds::default(),
            catalog_open_actions: PdfRows::default(),
            catalog_open_action_row: None,
            action_fingerprint: StateHasher::new_exact(0x7064_665f_6163_746e).finish_fragment(),
            page_reservations: PdfRows::default(),
            page_reservation_fingerprint: StateHasher::new_exact(0x7064_665f_7067_7273)
                .finish_fragment(),
            space_font_names: PdfRows::from_vec(vec![default_space_font.clone()]),
            space_font_name_lookup: BTreeMap::from([(default_space_font.clone(), 0)]),
            space_font_name_delta_lookup: BTreeMap::new(),
            current_space_font_name: 0,
            space_font_name_fingerprint: space_font_name_fingerprint(&default_space_font),
            annotations: PdfRows::default(),
            annotation_fingerprint: annotation_fingerprint::<G>(&[]),
            links: PdfRows::default(),
            link_fingerprint: StateHasher::new_exact(0x7064_665f_6c69_6e6b).finish_fragment(),
            open_links: PdfRows::default(),
            open_link_fingerprint: open_link_fingerprint::<G>(&PdfRows::default()),
            color_stacks: PdfRows::default(),
            color_stack_fingerprint: color_stack_fingerprint(&PdfRows::default()),
            last_position: (Scaled::from_raw(0), Scaled::from_raw(0)),
            snap_reference: (Scaled::from_raw(0), Scaled::from_raw(0)),
            forms: PdfRows::default(),
            form_fingerprint: StateHasher::new_exact(PDF_FORM_DOMAIN).finish_fragment(),
            next_form_resource: 1,
            form_artifacts: PdfDenseMap::default(),
            form_artifact_fingerprint: StateHasher::new_exact(0x7064_665f_666d_6172)
                .finish_fragment(),
            return_value: 0,
            destinations: PdfRows::default(),
            destination_fingerprint: destination_fingerprint(&PdfRows::default(), false),
            structure_destinations: PdfRows::default(),
            structure_destination_fingerprint: destination_fingerprint(&PdfRows::default(), true),
            outlines: PdfRows::default(),
            outline_fingerprint: outline_fingerprint::<G>(&[]),
            threads: PdfRows::default(),
            thread_fingerprint: thread_fingerprint(&PdfRows::default()),
            general_root: PdfVersionRoot::default(),
            general_index: PdfVersionIndex::default(),
            color_root: PdfVersionRoot::default(),
            color_index: PdfVersionIndex::default(),
            general_versions: PdfBranchArena::default(),
            open_link_nodes: PdfBranchArena::default(),
            thread_bead_nodes: PdfBranchArena::default(),
            color_values: PdfBranchArena::default(),
            color_push_nodes: PdfBranchArena::default(),
            undo_base: 0,
            undo_len: 0,
            candidate_undo_len: 0,
            color_undo_base: 0,
            color_undo_len: 0,
            candidate_color_undo_len: 0,
            transaction: None,
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

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub(crate) fn pages(&self) -> &PdfRows<PdfPageRecord<G>> {
        &self.pages
    }

    pub(crate) fn set_space_font_name(&mut self, name: Vec<u8>) {
        let accepted_id = self
            .space_font_name_lookup
            .get(&name)
            .copied()
            .filter(|id| (*id as usize) < self.space_font_names.retained_floor());
        let id = if let Some(id) = self
            .space_font_name_delta_lookup
            .get(&name)
            .copied()
            .or(accepted_id)
            .or_else(|| {
                self.space_font_names
                    .base_len
                    .is_none()
                    .then(|| self.space_font_name_lookup.get(&name).copied())
                    .flatten()
            }) {
            id
        } else {
            let id = u32::try_from(self.space_font_names.len())
                .expect("PDF space-font name count fits u32");
            self.space_font_names.push(name.clone());
            if self.space_font_names.base_len.is_some() {
                self.space_font_name_delta_lookup.insert(name, id);
            } else {
                self.space_font_name_lookup.insert(name, id);
            }
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
            && self.pk_mode_row.is_none()
            && self.font_resources.is_empty()
            && self.document_fragments.is_empty()
            && self.document_objects == PdfDocumentObjectIds::default()
            && self.catalog_open_action_row.is_none()
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
            .raw_objects()
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
                bytes: self.payloads.get(image.payload).to_vec(),
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
        capacities: Option<crate::PdfEngineCapacities>,
        import_tokens: impl FnMut(&[u8]) -> Result<PdfTokenParameter<G>, String>,
        import_nodes: impl FnMut(&[u8]) -> Result<(DurableListId<G>, StateHashFragment), String>,
    ) -> Result<Self, String> {
        let format = bincode::deserialize(bytes)
            .map_err(|error| format!("cannot decode PDF format resource state: {error}"))?;
        Self::restore_format(format, capacities, import_tokens, import_nodes)
    }

    pub(crate) fn restore_format(
        format: PdfFormatState,
        capacities: Option<crate::PdfEngineCapacities>,
        mut import_tokens: impl FnMut(&[u8]) -> Result<PdfTokenParameter<G>, String>,
        mut import_nodes: impl FnMut(&[u8]) -> Result<(DurableListId<G>, StateHashFragment), String>,
    ) -> Result<Self, String> {
        if format.version != 1 || format.next_object == 0 || format.next_form_resource == 0 {
            return Err("unsupported or invalid PDF format resource state".to_owned());
        }
        let retains_pdf_state = format.enabled
            || format.next_object != FIRST_DYNAMIC_OBJECT
            || format.next_form_resource != 1
            || !format.raw_objects.is_empty()
            || !format.forms.is_empty()
            || !format.external_images.is_empty()
            || !format.glyph_to_unicode.is_empty();
        let Some(capacities) = capacities else {
            if retains_pdf_state {
                return Err("non-pdfTeX format profile retains PDF resource state".to_owned());
            }
            return Ok(Self::default());
        };
        if format.next_object as usize > capacities.object_table_entries {
            return Err(format!(
                "invalid PDF format object-table coordinate: next_object={}, capacity={}",
                format.next_object, capacities.object_table_entries
            ));
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
            for image in format.external_images {
                let payload = state.payloads.store(image.bytes);
                state.external_images.push(PdfExternalImageEntry {
                    id: PdfExternalImageId(image.id),
                    identity: ContentHash::new(image.identity),
                    metadata: image.metadata,
                    dimensions: image.dimensions,
                    color_space_object: image.color_space_object,
                    payload,
                    mask_object: image.mask_object,
                });
            }
            state.external_image_fingerprint =
                external_image_fingerprint(&state.external_images, &state.payloads);
        }
        state.next_object = format.next_object;
        state.next_form_resource = format.next_form_resource;
        Ok(state)
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
        if self.pk_mode_row.is_none() {
            self.fingerprint = freeze_pk_mode_fingerprint(self.fingerprint, &pk_mode);
            let row = self.pk_modes.len();
            self.pk_modes.push(pk_mode);
            self.pk_mode_row = Some(row);
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
        let shared = self
            .font_resources
            .iter()
            .copied()
            .find(|record| record.identity == identity);
        if let Some(record) = shared {
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

    #[cfg(test)]
    pub(crate) fn font_resources(&self) -> impl Iterator<Item = PdfFontResourceRecord> + '_ {
        self.font_resources
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, record)| {
                (!self
                    .font_resources
                    .iter()
                    .take(index)
                    .any(|prior| prior.object_number == record.object_number))
                .then_some(record)
            })
    }

    /// Every live font-to-resource association, including aliases that share
    /// one emitted PDF object.
    ///
    /// Terminal detachment needs the complete identity view: page artifacts
    /// address realized semantic font identities, and two TeX fonts with
    /// different scale recipes may intentionally share one PDF resource
    /// object.
    pub(crate) fn font_resource_records(&self) -> impl Iterator<Item = PdfFontResourceRecord> + '_ {
        self.font_resources.iter().copied()
    }

    pub(crate) fn reserve_annotation(
        &mut self,
    ) -> Result<PdfAnnotationRecord<G>, PdfObjectCapacityError> {
        let object = self.reserve_document_object()?;
        let record = PdfAnnotationRecord::<G>::reserved(object);
        self.annotations.push(record.clone());
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
        let row = self
            .annotations
            .iter()
            .position(|record| record.object() == object)
            .ok_or(PdfAnnotationInitializeError(object))?;
        let key = PdfGeneralVersionKey::Annotation(row as u32);
        let prior = match self.general_version(key) {
            Some(PdfVersionValue::Annotation { data }) => data.clone(),
            Some(_) => unreachable!("PDF annotation version key has one value family"),
            None => self.annotations[row].data(),
        };
        if prior.is_some() {
            return Err(PdfAnnotationInitializeError(object));
        }
        let dimensions = data.dimensions;
        self.push_general_version(
            key,
            PdfVersionValue::Annotation {
                data: Some(data.clone()),
            },
        );
        self.annotation_fingerprint = append_annotation_data_fingerprint(
            self.annotation_fingerprint,
            object,
            dimensions,
            entries_semantic_id,
        );
        let mut record = PdfAnnotationRecord::reserved(object);
        record
            .initialize(data)
            .map_err(|()| PdfAnnotationInitializeError(object))?;
        Ok(record)
    }

    #[must_use]
    pub(crate) fn annotations(&self) -> Vec<PdfAnnotationRecord<G>> {
        self.annotations
            .iter()
            .enumerate()
            .map(|(row, record)| {
                let mut record = record.clone();
                if let Some(PdfVersionValue::Annotation { data }) =
                    self.general_version(PdfGeneralVersionKey::Annotation(row as u32))
                {
                    record.restore_data(data.clone());
                }
                record
            })
            .collect()
    }

    fn destination_records(&self, structure: bool) -> Vec<PdfDestinationRecord> {
        let records = if structure {
            &self.structure_destinations
        } else {
            &self.destinations
        };
        records
            .iter()
            .enumerate()
            .map(|(row, record)| {
                let mut record = record.clone();
                if let Some(PdfVersionValue::Destination {
                    structure: target,
                    defined,
                }) = self.general_version(PdfGeneralVersionKey::Destination {
                    structure,
                    row: row as u32,
                }) {
                    record.restore_definition(*target, *defined);
                }
                record
            })
            .collect()
    }

    fn thread_records(&self) -> Vec<PdfThreadRecord> {
        (0..self.threads.len())
            .map(|row| self.thread_record(row))
            .collect()
    }

    pub(crate) fn destination(
        &self,
        identity: &PdfDestinationIdentity,
        structure: bool,
    ) -> Option<PdfDestinationRecord> {
        let records = if structure {
            &self.structure_destinations
        } else {
            &self.destinations
        };
        let (row, record) = records
            .iter()
            .enumerate()
            .find(|(_, record)| record.identity() == identity)?;
        let mut record = record.clone();
        if let Some(PdfVersionValue::Destination { structure, defined }) =
            self.general_version(PdfGeneralVersionKey::Destination {
                structure,
                row: row as u32,
            })
        {
            record.restore_definition(*structure, *defined);
        }
        Some(record)
    }

    pub(crate) fn reserve_destination(
        &mut self,
        identity: PdfDestinationIdentity,
        structure: bool,
    ) -> Result<PdfDestinationRecord, PdfObjectCapacityError> {
        if let Some(record) = self.destination(&identity, structure) {
            return Ok(record);
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
            self.structure_destination_fingerprint = append_destination_fingerprint(
                self.structure_destination_fingerprint,
                &record,
                true,
                0,
            );
        } else {
            self.destination_fingerprint =
                append_destination_fingerprint(self.destination_fingerprint, &record, false, 0);
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
        let row = if structure {
            &self.structure_destinations
        } else {
            &self.destinations
        }
        .iter()
        .position(|record| record.object() == reserved.object())
        .expect("reserved destination exists");
        let key = PdfGeneralVersionKey::Destination {
            structure,
            row: row as u32,
        };
        let records = if structure {
            &self.structure_destinations
        } else {
            &self.destinations
        };
        let mut record = records[row].clone();
        if let Some(PdfVersionValue::Destination { structure, defined }) = self.general_version(key)
        {
            record.restore_definition(*structure, *defined);
        }
        let duplicate = !record.define(structure_target);
        let result = record.clone();
        self.push_general_version(
            key,
            PdfVersionValue::Destination {
                structure: result.structure(),
                defined: result.defined(),
            },
        );
        if structure {
            self.structure_destination_fingerprint = append_destination_fingerprint(
                self.structure_destination_fingerprint,
                &result,
                true,
                1,
            );
        } else {
            self.destination_fingerprint =
                append_destination_fingerprint(self.destination_fingerprint, &result, false, 1);
        }
        Ok(PdfDestinationDefinition {
            record: result,
            duplicate,
        })
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
        let (previous, len) = self.thread_state(index);
        let candidate = self.transaction.is_some();
        let bead_head = self.thread_bead_nodes.push(
            PdfThreadBeadNode {
                value: bead,
                previous,
            },
            candidate,
        );
        self.push_general_version(
            PdfGeneralVersionKey::Thread(index as u32),
            PdfVersionValue::Thread {
                bead_head: Some(bead_head),
                len: len + 1,
            },
        );
        let record = self.thread_record(index);
        self.thread_fingerprint =
            append_thread_bead_fingerprint(self.thread_fingerprint, record.object(), bead);
        Ok((record, bead))
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
        self.thread_fingerprint =
            append_thread_reservation_fingerprint(self.thread_fingerprint, &record);
        Ok(record)
    }

    /// Detaches unresolved navigation identities in pdfTeX's finalization
    /// order without exposing the checkpointed destination/thread ledgers.
    pub(crate) fn unresolved_navigation_warnings(&self) -> Vec<PdfNavigationWarning> {
        self.destination_records(false)
            .into_iter()
            .filter(|record| !record.defined())
            .map(|record| PdfNavigationWarning::Destination(record.identity().clone()))
            .chain(
                self.destination_records(true)
                    .into_iter()
                    .filter(|record| !record.defined())
                    .map(|record| {
                        PdfNavigationWarning::StructureDestination(record.identity().clone())
                    }),
            )
            .chain(
                self.thread_records()
                    .into_iter()
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
        self.outlines.push(record.clone());
        Ok(record)
    }

    #[cfg(test)]
    pub(crate) fn outlines(&self) -> &PdfRows<PdfOutlineRecord<G>> {
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
        let open = PdfOpenLink {
            record: record.clone(),
            nesting_depth,
        };
        let candidate = self.transaction.is_some();
        let root = self.open_link_nodes.push(
            PdfOpenLinkNode {
                value: open,
                previous: self.open_link_root(),
            },
            candidate,
        );
        self.push_general_version(
            PdfGeneralVersionKey::OpenLinks,
            PdfVersionValue::OpenLinks(Some(root)),
        );
        self.links.push(record.clone());
        self.open_link_fingerprint = open_link_fingerprint_values(&self.open_link_values());
        Ok(record)
    }

    pub(crate) fn end_link(&mut self) -> Option<PdfOpenLink<G>> {
        let root = self.open_link_root()?;
        let node = self
            .open_link_nodes
            .get(root)
            .expect("PDF open-link root is live");
        let open = node.value.clone();
        let previous = node.previous;
        self.push_general_version(
            PdfGeneralVersionKey::OpenLinks,
            PdfVersionValue::OpenLinks(previous),
        );
        self.open_link_fingerprint = open_link_fingerprint_values(&self.open_link_values());
        open.into()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn links(&self) -> &PdfRows<PdfLinkRecord<G>> {
        &self.links
    }

    #[must_use]
    pub(crate) fn last_link(&self) -> u32 {
        self.links.last().map_or(0, |record| record.object())
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn open_links(&self) -> Vec<PdfOpenLink<G>> {
        self.open_link_values()
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
                | PdfFontOperation::NoBuiltinToUnicode { .. } => None,
            })
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
            match operation {
                PdfFontMapOperation::BlockDefault | PdfFontMapOperation::File(_) => {}
                PdfFontMapOperation::Line(entry) => {
                    Self::apply_font_map_entry(entry.clone(), &mut entries, &mut duplicates);
                }
            }
        }
        (entries, duplicates)
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
    pub(crate) fn external_image(
        &self,
        id: PdfExternalImageId,
    ) -> Option<PdfExternalImageMetadata> {
        self.external_images
            .binary_search_by_key(&id, |record| record.id)
            .ok()
            .and_then(|index| self.external_images.get(index))
            .map(|record| record.metadata)
    }

    #[must_use]
    pub(crate) fn external_image_record(
        &self,
        id: PdfExternalImageId,
    ) -> Option<PdfExternalImageRecord> {
        self.external_images
            .binary_search_by_key(&id, |record| record.id)
            .ok()
            .and_then(|index| self.external_images.get(index))
            .map(|entry| self.materialize_external_image(entry))
    }

    fn materialize_external_image(&self, entry: &PdfExternalImageEntry) -> PdfExternalImageRecord {
        PdfExternalImageRecord {
            id: entry.id,
            identity: entry.identity,
            metadata: entry.metadata,
            dimensions: entry.dimensions,
            color_space_object: entry.color_space_object,
            bytes: self.payloads.get(entry.payload).to_vec(),
            mask_object: entry.mask_object,
        }
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
        let payload = self.payloads.store(source.bytes);
        let entry = PdfExternalImageEntry {
            id: PdfExternalImageId(raw),
            identity: source.identity,
            metadata: source.metadata,
            dimensions,
            color_space_object,
            payload,
            mask_object,
        };
        let record = self.materialize_external_image(&entry);
        self.external_images.push(entry);
        self.external_image_fingerprint =
            external_image_fingerprint(&self.external_images, &self.payloads);
        Ok(record)
    }

    pub(crate) fn last_external_image(&self) -> Option<PdfExternalImageRecord> {
        self.external_images
            .last()
            .map(|entry| self.materialize_external_image(entry))
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
        let entry = PdfFormArtifactEntry {
            payload: self.payloads.store(artifact.bytes),
            last_position: artifact.last_position,
            snap_reference: artifact.snap_reference,
        };
        self.push_general_version(
            PdfGeneralVersionKey::FormArtifact(object),
            PdfVersionValue::FormArtifact { entry: Some(entry) },
        );
    }

    #[must_use]
    pub(crate) fn form_artifact(&self, object: u32) -> Option<PdfFormArtifact> {
        let entry = match self.general_version(PdfGeneralVersionKey::FormArtifact(object)) {
            Some(PdfVersionValue::FormArtifact { entry }) => entry.as_ref()?,
            Some(_) => unreachable!("PDF form-artifact version key has one value family"),
            None => self.form_artifacts.get(&object)?,
        };
        Some(PdfFormArtifact {
            bytes: self.payloads.get(entry.payload).to_vec(),
            last_position: entry.last_position,
            snap_reference: entry.snap_reference,
        })
    }

    #[cfg(test)]
    fn form_artifact_payload(&self, object: u32) -> Option<PdfPayloadId> {
        match self.general_version(PdfGeneralVersionKey::FormArtifact(object)) {
            Some(PdfVersionValue::FormArtifact { entry }) => {
                entry.as_ref().map(|entry| entry.payload)
            }
            Some(_) => unreachable!("PDF form-artifact version key has one value family"),
            None => self.form_artifacts.get(&object).map(|entry| entry.payload),
        }
    }

    pub(crate) fn initialize_raw_object(
        &mut self,
        id: PdfRawObjectId,
        data: PdfRawObjectData<G>,
        immediate: bool,
    ) -> Result<(), PdfRawObjectInitializeError> {
        let record = self
            .raw_object(id)
            .ok_or(PdfRawObjectInitializeError::NotFound(id))?
            .initialize_version(data, immediate)?;
        self.push_general_version(
            PdfGeneralVersionKey::RawObject(id.raw()),
            PdfVersionValue::RawObject(record.clone()),
        );
        self.raw_objects.set_last_object(id.raw());
        self.raw_objects.append_version_fingerprint(1, &record);
        Ok(())
    }

    #[must_use]
    pub(crate) fn raw_object(&self, id: PdfRawObjectId) -> Option<PdfRawObjectRecord<G>> {
        match self.general_version(PdfGeneralVersionKey::RawObject(id.raw())) {
            Some(PdfVersionValue::RawObject(record)) => Some(record.clone()),
            Some(_) => unreachable!("PDF raw-object version key has one value family"),
            None => self.raw_objects.record(id),
        }
    }

    pub(crate) fn reference_raw_object(
        &mut self,
        id: PdfRawObjectId,
    ) -> Result<(), PdfRawObjectInitializeError> {
        let record = self
            .raw_object(id)
            .ok_or(PdfRawObjectInitializeError::NotFound(id))?
            .reference_version();
        self.push_general_version(
            PdfGeneralVersionKey::RawObject(id.raw()),
            PdfVersionValue::RawObject(record.clone()),
        );
        self.raw_objects.append_version_fingerprint(2, &record);
        Ok(())
    }

    pub(crate) fn raw_objects(&self) -> impl Iterator<Item = PdfRawObjectRecord<G>> + '_ {
        self.raw_objects.records().map(|record| {
            self.raw_object(record.id())
                .expect("PDF raw-object row is live")
        })
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
        debug_assert!(self.catalog_open_action_row.is_none());
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
        let row = self.catalog_open_actions.len();
        self.catalog_open_actions.push(record.clone());
        self.catalog_open_action_row = Some(row);
        self.action_fingerprint = fingerprint;
        Ok(record)
    }

    #[must_use]
    pub(crate) fn catalog_open_action(&self) -> Option<PdfActionRecord<G>> {
        self.catalog_open_action_row
            .and_then(|row| self.catalog_open_actions.get(row))
            .cloned()
    }

    fn reserved_page_object(&self, number: u32) -> Option<u32> {
        self.page_reservations
            .iter()
            .find(|reservation| reservation.number == number)
            .map(|reservation| reservation.object)
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
            pk_mode_row: self.pk_mode_row,
            font_operation_count: self.font_operations.len(),
            font_resource_count: self.font_resources.len(),
            fingerprint: self.fingerprint,
            match_fingerprint: self.match_state().fingerprint,
            external_image_count: self.external_images.len(),
            payload_count: self.payloads.len(),
            payload_bytes: self.payloads.bytes,
            color_undo_pos: self.history_head().1,
            external_image_fingerprint: self.external_image_fingerprint,
            raw_object_fingerprint: self.raw_objects.fingerprint(),
            raw_object_count: self.raw_objects.len(),
            raw_last_object: self.raw_objects.last_object(),
            document_fragment_fingerprint: self.document_fragments.fingerprint(),
            document_fragment_count: self.document_fragments.len(),
            document_objects: self.document_objects,
            catalog_open_action_row: self.catalog_open_action_row,
            action_fingerprint: self.action_fingerprint,
            page_reservation_fingerprint: self.page_reservation_fingerprint,
            page_reservation_count: self.page_reservations.len(),
            space_font_name_count: self.space_font_names.len(),
            current_space_font_name: self.current_space_font_name,
            space_font_name_fingerprint: self.space_font_name_fingerprint,
            annotation_fingerprint: self.annotation_fingerprint,
            annotation_count: self.annotations.len(),
            link_fingerprint: self.link_fingerprint,
            link_count: self.links.len(),
            open_link_fingerprint: self.open_link_fingerprint,
            open_link_count: self.open_link_values().len(),
            color_stack_fingerprint: self.color_stack_fingerprint,
            color_stack_count: self.color_stacks.len(),
            last_position: self.last_position,
            snap_reference: self.snap_reference,
            form_fingerprint: self.form_fingerprint,
            form_count: self.forms.len(),
            next_form_resource: self.next_form_resource,
            form_artifact_fingerprint: self.form_artifact_fingerprint,
            form_artifact_count: self.form_artifacts.len(),
            return_value: self.return_value,
            destination_fingerprint: self.destination_fingerprint,
            destination_count: self.destinations.len(),
            structure_destination_fingerprint: self.structure_destination_fingerprint,
            structure_destination_count: self.structure_destinations.len(),
            outline_fingerprint: self.outline_fingerprint,
            outline_count: self.outlines.len(),
            thread_fingerprint: self.thread_fingerprint,
            thread_count: self.threads.len(),
            _generation: std::marker::PhantomData,
        }
    }
    #[must_use]
    pub(crate) fn snapshot(&self) -> PdfStateSnapshot<G> {
        PdfStateSnapshot {
            cursor: self.cursor(),
            undo_pos: self.history_head().0,
            general_root: self.general_root,
            color_root: self.color_root,
        }
    }

    pub(crate) fn snapshot_is_retained(&self, snapshot: &PdfStateSnapshot<G>) -> bool {
        let cursor = &snapshot.cursor;
        cursor.page_count <= self.pages.len()
            && cursor.font_operation_count <= self.font_operations.len()
            && cursor.font_resource_count <= self.font_resources.len()
            && cursor.space_font_name_count <= self.space_font_names.len()
            && cursor.external_image_count <= self.external_images.len()
            && cursor.payload_count >= self.payloads.rows.retained_floor()
            && cursor.payload_count <= self.payloads.len()
            && cursor.raw_object_count <= self.raw_objects.len()
            && cursor.document_fragment_count <= self.document_fragments.len()
            && cursor.page_reservation_count <= self.page_reservations.len()
            && cursor.annotation_count <= self.annotations.len()
            && cursor.link_count <= self.links.len()
            && cursor.color_stack_count <= self.color_stacks.len()
            && cursor.form_count <= self.forms.len()
            && cursor.outline_count <= self.outlines.len()
            && cursor.thread_count <= self.threads.len()
            && if let Some(transaction) = &self.transaction {
                snapshot.undo_pos >= transaction.undo_low_water
                    && snapshot.undo_pos <= transaction.base.undo_pos + self.candidate_undo_len
                    && cursor.color_undo_pos >= transaction.color_undo_low_water
                    && cursor.color_undo_pos
                        <= transaction.base.cursor.color_undo_pos + self.candidate_color_undo_len
            } else {
                snapshot.undo_pos >= self.undo_base
                    && snapshot.undo_pos <= self.undo_base + self.undo_len
                    && cursor.color_undo_pos >= self.color_undo_base
                    && cursor.color_undo_pos <= self.color_undo_base + self.color_undo_len
            }
    }

    pub(crate) fn snapshot_font_roots_are_live(
        &self,
        snapshot: &PdfStateSnapshot<G>,
        mut is_live: impl FnMut(FontId) -> bool,
    ) -> bool {
        let cursor = &snapshot.cursor;
        self.font_operations
            .iter()
            .take(cursor.font_operation_count)
            .all(|operation| match operation {
                PdfFontOperation::Attribute { font, .. }
                | PdfFontOperation::IncludeChars { font, .. }
                | PdfFontOperation::NoBuiltinToUnicode { font } => is_live(*font),
                PdfFontOperation::Map(_) | PdfFontOperation::GlyphToUnicode(_) => true,
            })
            && self
                .font_resources
                .iter()
                .take(cursor.font_resource_count)
                .all(|record| is_live(record.font))
    }

    pub(crate) fn rollback(&mut self, snapshot: PdfStateSnapshot<G>) {
        let general_root = snapshot.general_root;
        let color_root = snapshot.color_root;
        let cursor = snapshot.cursor;
        assert!(
            cursor.page_count <= self.pages.len(),
            "PDF snapshot suffix was discarded"
        );
        self.pages.truncate(cursor.page_count);
        self.enabled = cursor.enabled;
        self.next_object = cursor.next_object;
        self.output_parameters = cursor.output_parameters;
        self.pk_modes
            .truncate(cursor.pk_mode_row.map_or(0, |row| row + 1));
        self.pk_mode_row = cursor.pk_mode_row;
        self.font_operations.truncate(cursor.font_operation_count);
        self.font_resources.truncate(cursor.font_resource_count);
        self.fingerprint = cursor.fingerprint;
        self.general_root = general_root;
        self.color_root = color_root;
        if let Some(transaction) = &self.transaction {
            self.candidate_undo_len = snapshot.undo_pos - transaction.base.undo_pos;
            self.candidate_color_undo_len =
                cursor.color_undo_pos - transaction.base.cursor.color_undo_pos;
        } else {
            self.undo_len = snapshot.undo_pos - self.undo_base;
            self.color_undo_len = cursor.color_undo_pos - self.color_undo_base;
        }
        self.external_images.truncate(cursor.external_image_count);
        self.external_image_fingerprint = cursor.external_image_fingerprint;
        self.raw_objects.truncate(cursor.raw_object_count);
        self.document_fragments
            .truncate(cursor.document_fragment_count);
        self.document_objects = cursor.document_objects;
        self.catalog_open_actions
            .truncate(cursor.catalog_open_action_row.map_or(0, |row| row + 1));
        self.catalog_open_action_row = cursor.catalog_open_action_row;
        self.action_fingerprint = cursor.action_fingerprint;
        self.page_reservations
            .truncate(cursor.page_reservation_count);
        self.page_reservation_fingerprint = cursor.page_reservation_fingerprint;
        for name in self
            .space_font_names
            .iter()
            .skip(cursor.space_font_name_count)
        {
            if self.space_font_names.base_len.is_some() {
                self.space_font_name_delta_lookup.remove(name.as_slice());
            } else {
                self.space_font_name_lookup.remove(name.as_slice());
            }
        }
        self.space_font_names.truncate(cursor.space_font_name_count);
        self.current_space_font_name = cursor.current_space_font_name;
        self.space_font_name_fingerprint = cursor.space_font_name_fingerprint;
        self.annotations.truncate(cursor.annotation_count);
        self.annotation_fingerprint = cursor.annotation_fingerprint;
        self.links.truncate(cursor.link_count);
        self.link_fingerprint = cursor.link_fingerprint;
        self.open_link_fingerprint = cursor.open_link_fingerprint;
        self.color_stacks.truncate(cursor.color_stack_count);
        self.color_stack_fingerprint = cursor.color_stack_fingerprint;
        self.last_position = cursor.last_position;
        self.snap_reference = cursor.snap_reference;
        self.forms.truncate(cursor.form_count);
        self.form_fingerprint = cursor.form_fingerprint;
        self.next_form_resource = cursor.next_form_resource;
        debug_assert_eq!(self.form_artifacts.len(), cursor.form_artifact_count);
        self.form_artifact_fingerprint = cursor.form_artifact_fingerprint;
        self.return_value = cursor.return_value;
        self.destinations.truncate(cursor.destination_count);
        self.destination_fingerprint = cursor.destination_fingerprint;
        self.structure_destinations
            .truncate(cursor.structure_destination_count);
        self.structure_destination_fingerprint = cursor.structure_destination_fingerprint;
        self.outlines.truncate(cursor.outline_count);
        self.outline_fingerprint = cursor.outline_fingerprint;
        self.threads.truncate(cursor.thread_count);
        self.thread_fingerprint = cursor.thread_fingerprint;
        self.payloads.truncate(cursor.payload_count);
    }

    pub(crate) fn set_match(
        &mut self,
        haystack: Vec<u8>,
        captures: Vec<Option<(u32, u32)>>,
        slot_count: u32,
        matched: bool,
    ) {
        let fingerprint = match_fingerprint(&haystack, &captures, slot_count, matched);
        self.push_general_version(
            PdfGeneralVersionKey::Match,
            PdfVersionValue::Match(PdfMatchState {
                haystack,
                captures,
                slot_count,
                matched,
                fingerprint,
            }),
        );
    }

    fn match_state(&self) -> &PdfMatchState {
        match self.general_version(PdfGeneralVersionKey::Match) {
            Some(PdfVersionValue::Match(state)) => state,
            Some(_) => unreachable!("PDF match version key has one value family"),
            None => &self.match_state,
        }
    }

    pub(crate) fn match_capture(&self, index: u32) -> Option<(u32, &[u8])> {
        let state = self.match_state();
        if !state.matched || index >= state.slot_count {
            return None;
        }
        let &(start, end) = state.captures.get(index as usize)?.as_ref()?;
        let bytes = state.haystack.get(start as usize..end as usize)?;
        Some((start, bytes))
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
            self.history_head().1,
            self.color_stack_fingerprint,
            self.color_root,
        )
    }

    pub(crate) fn rollback_form_colors(&mut self, rollback: PdfFormColorRollback) {
        let PdfFormColorRollback(undo_len, fingerprint, root) = rollback;
        let base = self
            .transaction
            .as_ref()
            .map_or(self.color_undo_base, |transaction| {
                transaction.base.cursor.color_undo_pos
            });
        if self.transaction.is_some() {
            self.candidate_color_undo_len = undo_len - base;
        } else {
            self.color_undo_len = undo_len - base;
        }
        self.color_root = root;
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
        self.color_stack_fingerprint = append_color_stack_definition_fingerprint(
            self.color_stack_fingerprint,
            0,
            PdfColorStackMode::Direct,
            true,
            b"0 g 0 G",
        );
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
        self.color_stack_fingerprint = append_color_stack_definition_fingerprint(
            self.color_stack_fingerprint,
            id,
            mode,
            restore_at_page_start,
            &self.color_stacks[id as usize].page.current,
        );
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
        let row = id as usize;
        let Some(mode) = self.color_stacks.get(row).map(|stack| stack.mode) else {
            return Err(PdfColorStackApplyError::Unknown);
        };
        let mut root = self.materialize_color_runtime(row, target);
        let mutated = match action {
            PdfColorStackAction::Set(bytes) => {
                root.current = self.store_color_value(bytes.clone());
                self.push_color_version(row, target, root);
                true
            }
            PdfColorStackAction::Push(bytes) => {
                let candidate = self.transaction.is_some();
                root.pushed = Some(self.color_push_nodes.push(
                    PdfColorPushNode {
                        value: root.current,
                        previous: root.pushed,
                    },
                    candidate,
                ));
                root.current = self.store_color_value(bytes.clone());
                self.push_color_version(row, target, root);
                true
            }
            PdfColorStackAction::Pop => {
                let node = self
                    .color_push_nodes
                    .get(root.pushed.ok_or(PdfColorStackApplyError::Underflow)?)
                    .expect("PDF color push root is live");
                root.current = node.value;
                root.pushed = node.previous;
                self.push_color_version(row, target, root);
                true
            }
            PdfColorStackAction::Current => false,
        };
        let payload = self.color_value(root.current).to_vec();
        if mutated {
            self.color_stack_fingerprint = append_color_stack_action_fingerprint(
                self.color_stack_fingerprint,
                id,
                target,
                action,
                &payload,
            );
        }
        Ok(PdfColorStackEmission { mode, payload })
    }

    pub(crate) fn page_color_stack_restorations(&mut self) -> Vec<PdfColorStackEmission> {
        if !self.enabled {
            return Vec::new();
        }
        self.ensure_default_color_stack();
        self.color_stacks
            .iter()
            .enumerate()
            .filter_map(|(id, stack)| {
                let payload = self.color_current_bytes(id, PdfColorStackTarget::Page);
                (stack.restore_at_page_start
                    && !payload.is_empty()
                    && !(id == 0 && payload == b"0 g 0 G"))
                    .then(|| PdfColorStackEmission {
                        mode: stack.mode,
                        payload: payload.to_vec(),
                    })
            })
            .collect()
    }
}

fn color_stack_fingerprint(stacks: &PdfRows<PdfColorStack>) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_COLOR_STACK_DOMAIN);
    hasher.usize(stacks.len());
    for stack in stacks.iter() {
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

fn append_color_stack_definition_fingerprint(
    previous: StateHashFragment,
    id: u32,
    mode: PdfColorStackMode,
    restore_at_page_start: bool,
    initial: &[u8],
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_6373_6476);
    previous.apply(&mut hasher);
    hasher.u8(0);
    hasher.u32(id);
    hasher.u8(match mode {
        PdfColorStackMode::Origin => 0,
        PdfColorStackMode::Page => 1,
        PdfColorStackMode::Direct => 2,
    });
    hasher.bool(restore_at_page_start);
    hasher.bytes(initial);
    hasher.finish_fragment()
}

fn append_color_stack_action_fingerprint(
    previous: StateHashFragment,
    id: u32,
    target: PdfColorStackTarget,
    action: &PdfColorStackAction,
    current: &[u8],
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_6373_6176);
    previous.apply(&mut hasher);
    hasher.u8(1);
    hasher.u32(id);
    hasher.u8(u8::from(matches!(target, PdfColorStackTarget::Form)));
    hasher.u8(match action {
        PdfColorStackAction::Set(_) => 0,
        PdfColorStackAction::Push(_) => 1,
        PdfColorStackAction::Pop => 2,
        PdfColorStackAction::Current => 3,
    });
    hasher.bytes(current);
    hasher.finish_fragment()
}

fn external_image_base_fingerprint() -> StateHashFragment {
    StateHasher::new_exact(PDF_EXTERNAL_IMAGE_DOMAIN).finish_fragment()
}

fn page_reservation_fingerprint(reservations: &PdfRows<PdfPageReservation>) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_7067_7273);
    hasher.usize(reservations.len());
    for reservation in reservations.iter() {
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

fn open_link_fingerprint<G>(links: &PdfRows<PdfOpenLink<G>>) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_6f70_6c6e);
    hasher.usize(links.len());
    for link in links.iter() {
        hasher.u32(link.record.object());
        hasher.u32(link.nesting_depth);
    }
    hasher.finish_fragment()
}

fn open_link_fingerprint_values<G>(links: &[PdfOpenLink<G>]) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_6f70_6c6e);
    hasher.usize(links.len());
    for link in links {
        hasher.u32(link.record.object());
        hasher.u32(link.nesting_depth);
    }
    hasher.finish_fragment()
}

fn destination_fingerprint(
    records: &PdfRows<PdfDestinationRecord>,
    structure: bool,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(if structure {
        0x7064_665f_7364_7374
    } else {
        0x7064_665f_6465_7374
    });
    hasher.usize(records.len());
    for record in records.iter() {
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

fn append_destination_fingerprint(
    previous: StateHashFragment,
    record: &PdfDestinationRecord,
    structure: bool,
    operation: u8,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(if structure {
        0x7064_665f_7364_6476
    } else {
        0x7064_665f_6465_6476
    });
    previous.apply(&mut hasher);
    hasher.u8(operation);
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
    hasher.finish_fragment()
}

fn outline_fingerprint<G>(_records: &[PdfOutlineRecord<G>]) -> StateHashFragment {
    StateHasher::new_exact(0x7064_665f_6f75_746c).finish_fragment()
}

fn thread_fingerprint(records: &PdfRows<PdfThreadRecord>) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_7468_7264);
    for record in records.iter() {
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

fn append_thread_reservation_fingerprint(
    previous: StateHashFragment,
    record: &PdfThreadRecord,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_7468_7276);
    previous.apply(&mut hasher);
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
    hasher.finish_fragment()
}

fn append_thread_bead_fingerprint(
    previous: StateHashFragment,
    object: u32,
    bead: PdfThreadBeadRecord,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(0x7064_665f_7468_6264);
    previous.apply(&mut hasher);
    hasher.u32(object);
    hasher.u32(bead.bead_object());
    hasher.u32(bead.rectangle_object());
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

fn external_image_fingerprint(
    images: &PdfRows<PdfExternalImageEntry>,
    payloads: &PdfPayloadArena,
) -> StateHashFragment {
    let mut hasher = StateHasher::new_exact(PDF_EXTERNAL_IMAGE_DOMAIN);
    hasher.usize(images.len());
    for record in images.iter() {
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
        hasher.bytes(payloads.get(record.payload));
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

/// One family isolated by the focused retained-generation fork profiler.
#[cfg(all(feature = "profiling", feature = "testing"))]
#[derive(Clone, Copy, Debug)]
pub enum PdfForkProfileFamily {
    PageReservations,
    FontResources,
    ExternalImageMetadata,
    RawObjectReservations,
    AnnotationReservations,
    Destinations,
    Threads,
    FormArtifactIndex,
    SpaceFontNames,
    ColorStacks,
    MatchBytes,
}

#[cfg(all(feature = "profiling", feature = "testing"))]
impl PdfForkProfileFamily {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PageReservations => "page_reservations",
            Self::FontResources => "font_resources",
            Self::ExternalImageMetadata => "external_image_metadata",
            Self::RawObjectReservations => "raw_object_reservations",
            Self::AnnotationReservations => "annotation_reservations",
            Self::Destinations => "destinations",
            Self::Threads => "threads",
            Self::FormArtifactIndex => "form_artifact_index",
            Self::SpaceFontNames => "space_font_names",
            Self::ColorStacks => "color_stacks",
            Self::MatchBytes => "match_bytes",
        }
    }

    pub const ALL: [Self; 11] = [
        Self::PageReservations,
        Self::FontResources,
        Self::ExternalImageMetadata,
        Self::RawObjectReservations,
        Self::AnnotationReservations,
        Self::Destinations,
        Self::Threads,
        Self::FormArtifactIndex,
        Self::SpaceFontNames,
        Self::ColorStacks,
        Self::MatchBytes,
    ];
}

/// Actual allocation and CPU cost of forking one isolated PDF metadata family.
#[cfg(all(feature = "profiling", feature = "testing"))]
#[derive(Clone, Copy, Debug)]
pub struct PdfForkProfileMeasurement {
    pub rows: usize,
    pub iterations: usize,
    pub elapsed_ns: u128,
    pub allocations: u64,
    pub requested_bytes: u64,
}

/// One replay-free candidate lifecycle phase measured against an early PDF
/// checkpoint. `lifecycle_work` counts fixed control operations; historical
/// key probes are reported separately on the enclosing measurement.
#[cfg(all(feature = "profiling", feature = "testing"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfUndoDistancePhase {
    pub elapsed_ns: u128,
    pub allocations: u64,
    pub requested_bytes: u64,
    pub lifecycle_work: u32,
    pub replay_work: u64,
}

/// Cost evidence for opening, mutating, rejecting, and accepting a candidate
/// rooted at an early retained PDF checkpoint.
#[cfg(all(feature = "profiling", feature = "testing"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfUndoDistanceMeasurement {
    pub accepted_undo_distance: usize,
    pub open: PdfUndoDistancePhase,
    pub first_mutation: PdfUndoDistancePhase,
    pub reject: PdfUndoDistancePhase,
    pub accept: PdfUndoDistancePhase,
    pub historical_lookup_probes: u32,
}

#[cfg(all(feature = "profiling", feature = "testing"))]
fn measure_pdf_lifecycle_phase(
    lifecycle_work: u32,
    operation: impl FnOnce(),
) -> PdfUndoDistancePhase {
    use crate::measurement::{
        HotCoreAllocationOwner, hot_core_allocation_scope, hot_core_thread_allocation_measurement,
    };

    let owner = HotCoreAllocationOwner::GenerationBoundary;
    let before = hot_core_thread_allocation_measurement(owner);
    let start = std::time::Instant::now();
    {
        let _scope = hot_core_allocation_scope(owner);
        operation();
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let after = hot_core_thread_allocation_measurement(owner);
    PdfUndoDistancePhase {
        elapsed_ns,
        allocations: after.calls - before.calls,
        requested_bytes: after.requested_bytes - before.requested_bytes,
        lifecycle_work,
        replay_work: 0,
    }
}

/// Profiles lifecycle work at a retained checkpoint followed by `distance`
/// accepted general and color versions. Historical key resolution remains a
/// fixed 64-probe trie walk and is intentionally excluded from lifecycle work.
#[cfg(all(feature = "profiling", feature = "testing"))]
pub fn profile_pdf_undo_distance(distance: usize) -> PdfUndoDistanceMeasurement {
    let mut state = PdfState::<()>::default();
    state.enable();
    state.ensure_default_color_stack();
    state.set_match(vec![0], Vec::new(), 0, false);
    state
        .apply_color_stack(
            0,
            PdfColorStackTarget::Page,
            &PdfColorStackAction::Set(vec![0]),
        )
        .expect("default PDF color stack exists");
    let base = state.snapshot();

    for value in 0..distance {
        let byte = value as u8;
        state.set_match(vec![byte], Vec::new(), 0, true);
        state
            .apply_color_stack(
                0,
                PdfColorStackTarget::Page,
                &PdfColorStackAction::Set(vec![byte]),
            )
            .expect("default PDF color stack exists");
    }

    let open = measure_pdf_lifecycle_phase(18, || state.open_candidate_lineage(&base));
    let first_mutation = measure_pdf_lifecycle_phase(2, || {
        state.set_match(vec![255], Vec::new(), 0, true);
        state
            .apply_color_stack(
                0,
                PdfColorStackTarget::Page,
                &PdfColorStackAction::Set(vec![255]),
            )
            .expect("default PDF color stack exists");
    });
    let reject = measure_pdf_lifecycle_phase(21, || state.reject_candidate_transaction());

    state.open_candidate_lineage(&base);
    state.set_match(vec![254], Vec::new(), 0, true);
    state
        .apply_color_stack(
            0,
            PdfColorStackTarget::Page,
            &PdfColorStackAction::Set(vec![254]),
        )
        .expect("default PDF color stack exists");
    let accept = measure_pdf_lifecycle_phase(20, || state.accept_candidate_transaction());

    PdfUndoDistanceMeasurement {
        accepted_undo_distance: distance,
        open,
        first_mutation,
        reject,
        accept,
        historical_lookup_probes: PdfVersionIndex::PROBES * 2,
    }
}

#[cfg(all(feature = "profiling", feature = "testing"))]
pub fn profile_pdf_fork_family(
    family: PdfForkProfileFamily,
    rows: usize,
    iterations: usize,
) -> PdfForkProfileMeasurement {
    use crate::measurement::{
        HotCoreAllocationOwner, hot_core_allocation_scope, hot_core_thread_allocation_measurement,
    };

    let mut state = PdfState::<()>::default();
    match family {
        PdfForkProfileFamily::PageReservations => {
            state
                .page_reservations
                .extend((0..rows).map(|row| PdfPageReservation {
                    number: row as u32,
                    object: row as u32 + 1,
                }));
        }
        PdfForkProfileFamily::FontResources => {
            state
                .font_resources
                .extend((0..rows).map(|row| PdfFontResourceRecord {
                    font: FontId::testing_new(row as u32),
                    source_identity: tex_fonts::FontSourceIdentity::from_bytes([row as u8; 8]),
                    resource_number: row as u32,
                    object_number: row as u32 + 1,
                    identity: tex_fonts::PdfFontResourceIdentity::new([row as u8; 8], None),
                }));
        }
        PdfForkProfileFamily::ExternalImageMetadata => {
            let payload = state.payloads.store(vec![7]);
            state
                .external_images
                .extend((0..rows).map(|row| PdfExternalImageEntry {
                    id: PdfExternalImageId(row as u32 + 1),
                    identity: ContentHash::new([row as u8; 32]),
                    metadata: PdfExternalImageMetadata::Raster(PdfRasterImageMetadata {
                        format: PdfRasterFormat::Png,
                        width: 1,
                        height: 1,
                        bits_per_component: 8,
                        color_space: PdfRasterColorSpace::Gray,
                        alpha: false,
                        png_color_type: Some(0),
                    }),
                    dimensions: PdfExternalImageDimensions {
                        width: Scaled::from_raw(1),
                        height: Scaled::from_raw(1),
                        depth: Scaled::from_raw(0),
                    },
                    color_space_object: 0,
                    payload,
                    mask_object: None,
                }));
        }
        PdfForkProfileFamily::RawObjectReservations => {
            for row in 0..rows {
                state
                    .raw_objects
                    .reserve(PdfRawObjectId::from_allocated(row as u32 + 1));
            }
        }
        PdfForkProfileFamily::AnnotationReservations => {
            state
                .annotations
                .extend((0..rows).map(|row| PdfAnnotationRecord::reserved(row as u32 + 1)));
        }
        PdfForkProfileFamily::Destinations => {
            state.destinations.extend((0..rows).map(|row| {
                PdfDestinationRecord::reserved(
                    PdfDestinationIdentity::Number(row as u32),
                    row as u32 + 1,
                )
            }));
        }
        PdfForkProfileFamily::Threads => {
            state.threads.extend((0..rows).map(|row| {
                PdfThreadRecord::new(PdfDestinationIdentity::Number(row as u32), row as u32 + 1)
            }));
        }
        PdfForkProfileFamily::FormArtifactIndex => {
            let payload = state.payloads.store(vec![7]);
            state.form_artifacts.extend((0..rows).map(|row| {
                (
                    row as u32 + 1,
                    PdfFormArtifactEntry {
                        payload,
                        last_position: None,
                        snap_reference: (Scaled::from_raw(0), Scaled::from_raw(0)),
                    },
                )
            }));
        }
        PdfForkProfileFamily::SpaceFontNames => {
            state.space_font_names.clear();
            state.space_font_name_lookup.clear();
            for row in 0..rows {
                let name = format!("space-font-{row:08}").into_bytes();
                state
                    .space_font_name_lookup
                    .insert(name.clone(), row as u32);
                state.space_font_names.push(name);
            }
            state.current_space_font_name = rows.saturating_sub(1) as u32;
        }
        PdfForkProfileFamily::ColorStacks => {
            state.color_stacks.extend((0..rows).map(|_| PdfColorStack {
                mode: PdfColorStackMode::Direct,
                restore_at_page_start: true,
                page: PdfColorStackRuntime {
                    current: vec![7; 32],
                    pushed: Vec::new(),
                },
                form: PdfColorStackRuntime {
                    current: vec![7; 32],
                    pushed: Vec::new(),
                },
            }));
        }
        PdfForkProfileFamily::MatchBytes => {
            state.match_state.haystack = vec![7; rows];
        }
    }
    let mark = state.snapshot();
    let owner = HotCoreAllocationOwner::GenerationBoundary;
    let before = hot_core_thread_allocation_measurement(owner);
    let start = std::time::Instant::now();
    {
        let _scope = hot_core_allocation_scope(owner);
        for _ in 0..iterations {
            state.open_candidate_lineage(std::hint::black_box(&mark));
            state.reject_candidate_transaction();
        }
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let after = hot_core_thread_allocation_measurement(owner);
    PdfForkProfileMeasurement {
        rows,
        iterations,
        elapsed_ns,
        allocations: after.calls - before.calls,
        requested_bytes: after.requested_bytes - before.requested_bytes,
    }
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
