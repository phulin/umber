//! Detached deterministic PDF document graph.
//!
//! This module owns structural PDF state and semantic identity. Final PDF byte
//! serialization is deliberately separate and must use the `pdf_writer` crate.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use pdf_writer::{Name, Str};
use sha2::{Digest, Sha256};

mod serialize;

pub use serialize::{
    PdfObjectCompression, PdfSerializationOptions, PdfSerializeError, PdfStreamCompression,
};

/// One filled rectangle in PDF user-space points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfContentRectangle {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// One absolutely positioned byte-encoded PDF text run.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfContentTextRun {
    pub x: f32,
    pub baseline: f32,
    pub font_name: Vec<u8>,
    pub font_size: f32,
    pub bytes: Vec<u8>,
}

/// One ordered page-content operation. Generated PDF syntax is lowered only
/// through typed `pdf_writer` methods; only `Literal` contains user-authored
/// opaque operations.
#[derive(Clone, Debug, PartialEq)]
pub enum PdfContentOperation {
    Rectangle(PdfContentRectangle),
    Text(PdfContentTextRun),
    Literal {
        mode: crate::PdfLiteralMode,
        x: f32,
        y: f32,
        bytes: Vec<u8>,
    },
    ColorStack {
        mode: crate::PdfLiteralMode,
        x: f32,
        y: f32,
        bytes: Vec<u8>,
    },
    SetMatrix {
        x: f32,
        y: f32,
        matrix: [f32; 4],
    },
    Save {
        x: f32,
        y: f32,
    },
    Restore {
        x: f32,
        y: f32,
    },
    FormXObject {
        x: f32,
        y: f32,
        name: Vec<u8>,
    },
    ImageXObject {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        name: Vec<u8>,
    },
}

/// One PK-derived monochrome Type-3 glyph procedure.
pub struct PdfType3BitmapGlyph<'a> {
    pub advance: f32,
    pub bbox: [i32; 4],
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub bitmap: &'a [u8],
}

/// Encodes a Type-3 glyph procedure entirely through the vendored
/// `pdf_writer`, including its typed inline-image builder.
#[must_use]
pub fn type3_bitmap_glyph_content(glyph: &PdfType3BitmapGlyph<'_>) -> Vec<u8> {
    let mut content = pdf_writer::Content::new();
    content.start_shape_glyph(
        glyph.advance,
        glyph.bbox[0] as f32,
        glyph.bbox[1] as f32,
        glyph.bbox[2] as f32,
        glyph.bbox[3] as f32,
    );
    content.save_state();
    content.transform([
        glyph.width as f32,
        0.0,
        0.0,
        glyph.height as f32,
        glyph.x as f32,
        glyph.y as f32,
    ]);
    content
        .inline_image(
            i32::try_from(glyph.width).expect("validated PK width fits i32"),
            i32::try_from(glyph.height).expect("validated PK height fits i32"),
            glyph.bitmap,
        )
        .image_mask()
        .bits_per_component(1)
        .decode([1.0, 0.0])
        .finish();
    content.finish().to_vec()
}

/// Encodes an invisible Type-3 space glyph through `pdf_writer`.
#[must_use]
pub fn type3_space_glyph_content(advance: f32) -> Vec<u8> {
    let mut content = pdf_writer::Content::new();
    content.start_shape_glyph(advance, 0.0, 0.0, 0.0, 0.0);
    content.finish().to_vec()
}

/// Encodes filled rule rectangles exclusively through `pdf_writer`.
#[must_use]
pub fn filled_rectangle_content(rectangles: &[PdfContentRectangle]) -> Vec<u8> {
    page_content(rectangles, &[])
}

/// Encodes page painting operators exclusively through `pdf_writer`.
#[must_use]
pub fn page_content(
    rectangles: &[PdfContentRectangle],
    text_runs: &[PdfContentTextRun],
) -> Vec<u8> {
    let mut content = pdf_writer::Content::new();
    content.save_state();
    for rectangle in rectangles {
        content
            .rect(rectangle.x, rectangle.y, rectangle.width, rectangle.height)
            .fill_nonzero();
    }
    content.restore_state();
    if !text_runs.is_empty() {
        content.begin_text();
        for run in text_runs {
            content
                .set_font(Name(&run.font_name), run.font_size)
                .set_text_matrix([1.0, 0.0, 0.0, 1.0, run.x, run.baseline])
                .show(Str(&run.bytes));
        }
        content.end_text();
    }
    content.finish().to_vec()
}

/// Encodes ordered pdfTeX page operations through the vendored `pdf_writer`.
#[must_use]
pub fn ordered_page_content(operations: &[PdfContentOperation]) -> Vec<u8> {
    let mut content = pdf_writer::Content::new();
    let mut origin = (0.0, 0.0);
    let mut saved_origins = Vec::new();
    let mut in_text = false;
    let set_origin = |content: &mut pdf_writer::Content, origin: &mut (f32, f32), x, y| {
        let dx = x - origin.0;
        let dy = y - origin.1;
        if dx != 0.0 || dy != 0.0 {
            content.transform([1.0, 0.0, 0.0, 1.0, dx, dy]);
            *origin = (x, y);
        }
    };
    for operation in operations {
        match operation {
            PdfContentOperation::Rectangle(rectangle) => {
                end_pdf_text(&mut content, &mut in_text);
                content.save_state();
                content
                    .rect(rectangle.x, rectangle.y, rectangle.width, rectangle.height)
                    .fill_nonzero();
                content.restore_state();
            }
            PdfContentOperation::Text(run) => {
                if !in_text {
                    content.begin_text();
                    in_text = true;
                }
                content
                    .set_font(Name(&run.font_name), run.font_size)
                    .set_text_matrix([1.0, 0.0, 0.0, 1.0, run.x, run.baseline])
                    .show(Str(&run.bytes));
            }
            PdfContentOperation::Literal { mode, x, y, bytes } => {
                if *mode != crate::PdfLiteralMode::Direct {
                    end_pdf_text(&mut content, &mut in_text);
                }
                if *mode == crate::PdfLiteralMode::Origin {
                    set_origin(&mut content, &mut origin, *x, *y);
                }
                content.verbatim_operations(bytes);
            }
            PdfContentOperation::ColorStack { mode, x, y, bytes } => {
                if *mode != crate::PdfLiteralMode::Direct {
                    end_pdf_text(&mut content, &mut in_text);
                }
                if *mode == crate::PdfLiteralMode::Origin {
                    set_origin(&mut content, &mut origin, *x, *y);
                }
                content.color_stack_operations(bytes);
            }
            PdfContentOperation::SetMatrix { x, y, matrix } => {
                end_pdf_text(&mut content, &mut in_text);
                set_origin(&mut content, &mut origin, *x, *y);
                content.transform([matrix[0], matrix[1], matrix[2], matrix[3], 0.0, 0.0]);
            }
            PdfContentOperation::Save { x, y } => {
                end_pdf_text(&mut content, &mut in_text);
                set_origin(&mut content, &mut origin, *x, *y);
                saved_origins.push(origin);
                content.save_state();
            }
            PdfContentOperation::Restore { x, y } => {
                end_pdf_text(&mut content, &mut in_text);
                set_origin(&mut content, &mut origin, *x, *y);
                content.restore_state();
                if let Some(saved) = saved_origins.pop() {
                    origin = saved;
                }
            }
            PdfContentOperation::FormXObject { x, y, name } => {
                end_pdf_text(&mut content, &mut in_text);
                content.save_state();
                content.transform([1.0, 0.0, 0.0, 1.0, *x, *y]);
                content.x_object(Name(name));
                content.restore_state();
            }
            PdfContentOperation::ImageXObject {
                x,
                y,
                width,
                height,
                name,
            } => {
                end_pdf_text(&mut content, &mut in_text);
                content.save_state();
                content.transform([*width, 0.0, 0.0, *height, *x, *y]);
                content.x_object(Name(name));
                content.restore_state();
            }
        }
    }
    end_pdf_text(&mut content, &mut in_text);
    content.finish().to_vec()
}

fn end_pdf_text(content: &mut pdf_writer::Content, in_text: &mut bool) {
    if *in_text {
        content.end_text();
        *in_text = false;
    }
}

/// Stable indirect-object identity within one PDF document timeline.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfObjectId(NonZeroU32);

impl PdfObjectId {
    /// Creates an identity, rejecting PDF's reserved object number zero.
    #[must_use]
    pub const fn new(raw: u32) -> Option<Self> {
        match NonZeroU32::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// Returns the PDF indirect-object number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// PDF file-format version requested by the engine.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfVersion {
    major: u8,
    minor: u8,
}

impl PdfVersion {
    /// Creates a supported `1.x` PDF version.
    pub fn new(major: u8, minor: u8) -> Result<Self, PdfModelError> {
        if (1..=9).contains(&major) && minor <= 9 {
            Ok(Self { major, minor })
        } else {
            Err(PdfModelError::UnsupportedVersion { major, minor })
        }
    }

    #[must_use]
    pub const fn major(self) -> u8 {
        self.major
    }

    #[must_use]
    pub const fn minor(self) -> u8 {
        self.minor
    }
}

/// Canonical fixed-point PDF number without binary floating point.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfNumber {
    coefficient: i64,
    decimal_places: u8,
}

impl PdfNumber {
    /// Creates a number and removes redundant fractional trailing zeroes.
    pub fn new(mut coefficient: i64, mut decimal_places: u8) -> Result<Self, PdfModelError> {
        if decimal_places > 9 {
            return Err(PdfModelError::NumberPrecisionTooLarge(decimal_places));
        }
        while decimal_places != 0 && coefficient % 10 == 0 {
            coefficient /= 10;
            decimal_places -= 1;
        }
        Ok(Self {
            coefficient,
            decimal_places,
        })
    }

    #[must_use]
    pub const fn coefficient(self) -> i64 {
        self.coefficient
    }

    #[must_use]
    pub const fn decimal_places(self) -> u8 {
        self.decimal_places
    }
}

/// A PDF name as uninterpreted bytes; the serializer owns escaping.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfName(Vec<u8>);

impl PdfName {
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<&str> for PdfName {
    fn from(value: &str) -> Self {
        Self::new(value.as_bytes())
    }
}

/// A key-ordered PDF dictionary with an optional verbatim extension fragment.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PdfDictionary {
    entries: BTreeMap<PdfName, PdfValue>,
    raw_entries: Vec<u8>,
}

impl PdfDictionary {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a key while rejecting accidental replacement.
    pub fn insert(
        &mut self,
        key: impl Into<PdfName>,
        value: PdfValue,
    ) -> Result<(), PdfModelError> {
        let key = key.into();
        if self.entries.insert(key.clone(), value).is_some() {
            return Err(PdfModelError::DuplicateDictionaryKey(key));
        }
        Ok(())
    }

    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<&PdfValue> {
        self.entries.get(&PdfName::new(key))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&PdfName, &PdfValue)> {
        self.entries.iter()
    }

    /// Replaces the verbatim dictionary-entry fragment.
    ///
    /// The bytes must contain complete `name value` pairs without the
    /// surrounding `<< >>`. This escape hatch intentionally mirrors pdfTeX's
    /// token-list attributes, which are copied into PDF dictionaries without
    /// parsing.
    pub fn set_raw_entries(&mut self, entries: impl Into<Vec<u8>>) {
        self.raw_entries = entries.into();
    }

    #[must_use]
    pub fn raw_entries(&self) -> &[u8] {
        &self.raw_entries
    }

    #[must_use]
    pub fn raw_entries_contain(&self, needle: &[u8]) -> bool {
        !needle.is_empty()
            && self
                .raw_entries
                .windows(needle.len())
                .any(|window| window == needle)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.raw_entries.is_empty()
    }
}

/// Detached PDF value with deterministic, lossless scalar representations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfValue {
    Null,
    Bool(bool),
    Integer(i64),
    Number(PdfNumber),
    Name(PdfName),
    String(Vec<u8>),
    Array(Vec<Self>),
    Dictionary(PdfDictionary),
    Reference(PdfObjectId),
}

/// An indirect PDF object. Streams are indirect by construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfObject {
    Value(PdfValue),
    Annotation(PdfAnnotationObject),
    /// A typed page destination array.
    Destination(PdfExplicitDestination),
    /// A typed named-destination dictionary containing `/D`.
    NamedDestination(PdfExplicitDestination),
    /// One typed node in the catalog destination name tree.
    DestinationNameTree(PdfDestinationNameTree),
    /// The indirect catalog `/Names` dictionary.
    Names(PdfNamesObject),
    Action(PdfAnnotationAction),
    PdfStringSyntax(Vec<u8>),
    Outline(PdfOutlineObject),
    OutlineItem(PdfOutlineItemObject),
    /// One complete direct object body retained for pdfTeX compatibility.
    Raw(Vec<u8>),
    Stream {
        dictionary: PdfDictionary,
        data: Vec<u8>,
    },
    /// A typed reusable Form XObject serialized through `pdf_writer::FormXObject`.
    FormXObject {
        dictionary: PdfDictionary,
        data: Vec<u8>,
        bbox: [PdfNumber; 4],
        matrix: Option<[PdfNumber; 6]>,
    },
    /// A typed raster image serialized through `pdf_writer::ImageXObject`.
    ImageXObject {
        image: PdfImageXObject,
        data: Vec<u8>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfOutlineObject {
    pub first: PdfObjectId,
    pub last: PdfObjectId,
    pub visible_count: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOutlineItemObject {
    pub title: PdfObjectId,
    pub action: PdfObjectId,
    pub parent: PdfObjectId,
    pub previous: Option<PdfObjectId>,
    pub next: Option<PdfObjectId>,
    pub first: Option<PdfObjectId>,
    pub last: Option<PdfObjectId>,
    pub count: Option<i32>,
    pub raw_entries: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfExplicitDestination {
    pub page: PdfObjectId,
    pub view: PdfDestinationView,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfDestinationView {
    Xyz {
        left: PdfNumber,
        top: PdfNumber,
        zoom: Option<PdfNumber>,
    },
    FitBoundingBoxHorizontal {
        top: PdfNumber,
    },
    FitBoundingBoxVertical {
        left: PdfNumber,
    },
    FitBoundingBox,
    FitHorizontal {
        top: PdfNumber,
    },
    FitVertical {
        left: PdfNumber,
    },
    FitRectangle {
        left: PdfNumber,
        bottom: PdfNumber,
        right: PdfNumber,
        top: PdfNumber,
    },
    Fit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDestinationNameTree {
    pub limits: Option<(Vec<u8>, Vec<u8>)>,
    pub children: PdfDestinationNameTreeChildren,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfDestinationNameTreeChildren {
    Names(Vec<(Vec<u8>, PdfObjectId)>),
    Kids(Vec<PdfObjectId>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfNamesObject {
    pub destinations: Option<PdfObjectId>,
    pub raw_entries: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfImageColorSpace {
    DeviceGray,
    DeviceRgb,
    DeviceCmyk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfImageFilter {
    Dct,
    Flate,
    FlatePngPredictor { colors: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfImageXObject {
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: PdfImageColorSpace,
    pub filter: PdfImageFilter,
    pub soft_mask: Option<PdfObjectId>,
}

/// A detached annotation serialized through `pdf_writer`'s typed builder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfAnnotationObject {
    pub rect: [PdfNumber; 4],
    pub subtype: Option<PdfAnnotationType>,
    pub action: Option<PdfAnnotationAction>,
    /// User-supplied pdfTeX annotation or link-attribute entries only.
    pub raw_entries: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfAnnotationType {
    Link,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfAnnotationAction {
    /// Complete user-supplied annotation entries, retained as a compatibility escape hatch.
    UserEntries(Vec<u8>),
    Destination(PdfDestinationAction),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDestinationAction {
    pub kind: PdfDestinationActionKind,
    pub file: Option<Vec<u8>>,
    pub target: PdfDestinationTarget,
    pub structure: Option<PdfDestinationStructure>,
    pub new_window: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfDestinationActionKind {
    GoTo,
    Thread,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfDestinationTarget {
    Reference(PdfObjectId),
    Page {
        page: PdfDestinationPage,
        /// User-supplied destination view operands.
        view: Vec<u8>,
    },
    Name(Vec<u8>),
    Number(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfDestinationPage {
    Internal(PdfObjectId),
    External(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfDestinationStructure {
    Internal(PdfObjectId),
    External(Vec<u8>),
}

/// File-trailer extensions owned by the detached document model.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PdfTrailer {
    pub info: Option<PdfObjectId>,
    pub file_id: Option<(Vec<u8>, Vec<u8>)>,
    pub raw_entries: Vec<u8>,
}

/// An identity paired with detached object content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfIndirectObject {
    pub id: PdfObjectId,
    pub object: PdfObject,
}

/// Detached graph awaiting validation and canonical ordering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnvalidatedPdfDocument {
    pub version: PdfVersion,
    pub catalog: PdfObjectId,
    pub objects: Vec<PdfIndirectObject>,
    pub trailer: PdfTrailer,
}

impl UnvalidatedPdfDocument {
    pub fn validate(self) -> Result<PdfDocument, PdfModelError> {
        self.validate_with_limits(PdfModelLimits::default())
    }

    pub fn validate_with_limits(
        mut self,
        limits: PdfModelLimits,
    ) -> Result<PdfDocument, PdfModelError> {
        validate_document(&mut self, limits)?;
        Ok(PdfDocument(self))
    }
}

/// Validated, canonically ordered PDF structural graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDocument(UnvalidatedPdfDocument);

impl PdfDocument {
    #[must_use]
    pub const fn version(&self) -> PdfVersion {
        self.0.version
    }

    #[must_use]
    pub const fn catalog(&self) -> PdfObjectId {
        self.0.catalog
    }

    #[must_use]
    pub const fn info(&self) -> Option<PdfObjectId> {
        self.0.trailer.info
    }

    pub fn objects(&self) -> impl ExactSizeIterator<Item = &PdfIndirectObject> {
        self.0.objects.iter()
    }

    #[must_use]
    pub const fn trailer(&self) -> &PdfTrailer {
        &self.0.trailer
    }

    /// Hashes a versioned canonical structural encoding, independent of input order.
    #[must_use]
    pub fn semantic_hash(&self) -> PdfDocumentHash {
        let mut hasher = CanonicalHasher::new();
        hasher.byte(self.version().major());
        hasher.byte(self.version().minor());
        hasher.u32(self.catalog().get());
        hasher.len(self.0.objects.len());
        for indirect in &self.0.objects {
            hasher.u32(indirect.id.get());
            hash_object(&indirect.object, &mut hasher);
        }
        hasher.bool(self.0.trailer.info.is_some());
        if let Some(info) = self.0.trailer.info {
            hasher.u32(info.get());
        }
        hasher.bool(self.0.trailer.file_id.is_some());
        if let Some((first, second)) = &self.0.trailer.file_id {
            hasher.bytes(first);
            hasher.bytes(second);
        }
        hasher.bytes(&self.0.trailer.raw_entries);
        PdfDocumentHash(hasher.finish())
    }
}

/// Stable semantic identity of a validated detached PDF graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfDocumentHash([u8; 32]);

impl PdfDocumentHash {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Validation budgets for untrusted detached PDF graphs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfModelLimits {
    pub max_objects: usize,
    pub max_depth: usize,
    pub max_values: usize,
    pub max_stream_bytes: usize,
}

impl Default for PdfModelLimits {
    fn default() -> Self {
        Self {
            max_objects: 1_000_000,
            max_depth: 256,
            max_values: 4_000_000,
            max_stream_bytes: 1 << 30,
        }
    }
}

/// Typed detached-model construction or validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfModelError {
    UnsupportedVersion { major: u8, minor: u8 },
    NumberPrecisionTooLarge(u8),
    DuplicateDictionaryKey(PdfName),
    DuplicateObject(PdfObjectId),
    MissingObject(PdfObjectId),
    TooManyObjects { actual: usize, limit: usize },
    TooManyValues { actual: usize, limit: usize },
    NestingTooDeep { actual: usize, limit: usize },
    TooManyStreamBytes { actual: usize, limit: usize },
    ReservedStreamLength(PdfObjectId),
    CatalogNotDictionary(PdfObjectId),
    InfoNotDictionary(PdfObjectId),
    CatalogTypeMissing(PdfObjectId),
    CatalogPagesMissing(PdfObjectId),
    PagesRootNotDictionary(PdfObjectId),
    PagesTypeMissing(PdfObjectId),
    PagesKidsInvalid(PdfObjectId),
    PagesCountInvalid(PdfObjectId),
    PageNotDictionary(PdfObjectId),
    PageTypeMissing(PdfObjectId),
    PageParentInvalid(PdfObjectId),
    PageMediaBoxInvalid(PdfObjectId),
    PageResourcesInvalid(PdfObjectId),
    PageContentsInvalid(PdfObjectId),
    PageAnnotationsInvalid(PdfObjectId),
    AnnotationNotTyped(PdfObjectId),
    AnnotationOwnedByMultiplePages(PdfObjectId),
}

impl std::fmt::Display for PdfModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid detached PDF model: {self:?}")
    }
}

impl std::error::Error for PdfModelError {}

fn validate_document(
    document: &mut UnvalidatedPdfDocument,
    limits: PdfModelLimits,
) -> Result<(), PdfModelError> {
    if document.objects.len() > limits.max_objects {
        return Err(PdfModelError::TooManyObjects {
            actual: document.objects.len(),
            limit: limits.max_objects,
        });
    }
    document.objects.sort_by_key(|object| object.id);
    for pair in document.objects.windows(2) {
        if pair[0].id == pair[1].id {
            return Err(PdfModelError::DuplicateObject(pair[0].id));
        }
    }

    let ids = document
        .objects
        .iter()
        .map(|object| object.id)
        .collect::<BTreeSet<_>>();
    if !ids.contains(&document.catalog) {
        return Err(PdfModelError::MissingObject(document.catalog));
    }
    if let Some(info) = document.trailer.info
        && !ids.contains(&info)
    {
        return Err(PdfModelError::MissingObject(info));
    }
    if let Some(info) = document.trailer.info
        && object_dictionary(document, info).is_none()
    {
        return Err(PdfModelError::InfoNotDictionary(info));
    }

    let mut value_count = 0_usize;
    let mut stream_bytes = 0_usize;
    for indirect in &document.objects {
        match &indirect.object {
            PdfObject::Stream { dictionary, data }
            | PdfObject::FormXObject {
                dictionary, data, ..
            } => {
                if dictionary.get(b"Length").is_some() {
                    return Err(PdfModelError::ReservedStreamLength(indirect.id));
                }
                stream_bytes = stream_bytes.saturating_add(data.len());
            }
            PdfObject::Raw(data) => stream_bytes = stream_bytes.saturating_add(data.len()),
            PdfObject::ImageXObject { data, .. } => {
                stream_bytes = stream_bytes.saturating_add(data.len());
            }
            PdfObject::Value(_)
            | PdfObject::Annotation(_)
            | PdfObject::Destination(_)
            | PdfObject::NamedDestination(_)
            | PdfObject::DestinationNameTree(_)
            | PdfObject::Names(_)
            | PdfObject::Action(_)
            | PdfObject::PdfStringSyntax(_)
            | PdfObject::Outline(_)
            | PdfObject::OutlineItem(_) => {}
        }
        if stream_bytes > limits.max_stream_bytes {
            return Err(PdfModelError::TooManyStreamBytes {
                actual: stream_bytes,
                limit: limits.max_stream_bytes,
            });
        }
        validate_object_values(
            &indirect.object,
            &ids,
            &mut value_count,
            limits.max_values,
            limits.max_depth,
        )?;
    }
    validate_page_graph(document)
}

fn validate_object_values(
    object: &PdfObject,
    ids: &BTreeSet<PdfObjectId>,
    value_count: &mut usize,
    max_values: usize,
    max_depth: usize,
) -> Result<(), PdfModelError> {
    let mut stack = Vec::new();
    match object {
        PdfObject::Value(value) => stack.push((value, 1_usize)),
        PdfObject::Stream { dictionary, .. } | PdfObject::FormXObject { dictionary, .. } => {
            stack.extend(dictionary.iter().map(|(_, value)| (value, 1)))
        }
        PdfObject::Raw(_) => {}
        PdfObject::ImageXObject { image, .. } => {
            if let Some(mask) = image.soft_mask
                && !ids.contains(&mask)
            {
                return Err(PdfModelError::MissingObject(mask));
            }
        }
        PdfObject::Annotation(annotation) => {
            if let Some(PdfAnnotationAction::Destination(action)) = &annotation.action {
                if let PdfDestinationTarget::Page {
                    page: PdfDestinationPage::Internal(id),
                    ..
                } = action.target
                    && !ids.contains(&id)
                {
                    return Err(PdfModelError::MissingObject(id));
                }
                if let PdfDestinationTarget::Reference(id) = action.target
                    && !ids.contains(&id)
                {
                    return Err(PdfModelError::MissingObject(id));
                }
                if let Some(PdfDestinationStructure::Internal(id)) = action.structure
                    && !ids.contains(&id)
                {
                    return Err(PdfModelError::MissingObject(id));
                }
            }
        }
        PdfObject::Destination(destination) | PdfObject::NamedDestination(destination) => {
            if !ids.contains(&destination.page) {
                return Err(PdfModelError::MissingObject(destination.page));
            }
        }
        PdfObject::DestinationNameTree(tree) => match &tree.children {
            PdfDestinationNameTreeChildren::Names(entries) => {
                for (_, id) in entries {
                    if !ids.contains(id) {
                        return Err(PdfModelError::MissingObject(*id));
                    }
                }
            }
            PdfDestinationNameTreeChildren::Kids(kids) => {
                for id in kids {
                    if !ids.contains(id) {
                        return Err(PdfModelError::MissingObject(*id));
                    }
                }
            }
        },
        PdfObject::Names(names) => {
            if let Some(id) = names.destinations
                && !ids.contains(&id)
            {
                return Err(PdfModelError::MissingObject(id));
            }
        }
        PdfObject::Action(action) => {
            if let PdfAnnotationAction::Destination(action) = action {
                if let PdfDestinationTarget::Page {
                    page: PdfDestinationPage::Internal(id),
                    ..
                } = action.target
                    && !ids.contains(&id)
                {
                    return Err(PdfModelError::MissingObject(id));
                }
                if let PdfDestinationTarget::Reference(id) = action.target
                    && !ids.contains(&id)
                {
                    return Err(PdfModelError::MissingObject(id));
                }
                if let Some(PdfDestinationStructure::Internal(id)) = action.structure
                    && !ids.contains(&id)
                {
                    return Err(PdfModelError::MissingObject(id));
                }
            }
        }
        PdfObject::PdfStringSyntax(_) => {}
        PdfObject::Outline(outline) => {
            for id in [outline.first, outline.last] {
                if !ids.contains(&id) {
                    return Err(PdfModelError::MissingObject(id));
                }
            }
        }
        PdfObject::OutlineItem(item) => {
            for id in [
                Some(item.title),
                Some(item.action),
                Some(item.parent),
                item.previous,
                item.next,
                item.first,
                item.last,
            ]
            .into_iter()
            .flatten()
            {
                if !ids.contains(&id) {
                    return Err(PdfModelError::MissingObject(id));
                }
            }
        }
    }
    while let Some((value, depth)) = stack.pop() {
        if depth > max_depth {
            return Err(PdfModelError::NestingTooDeep {
                actual: depth,
                limit: max_depth,
            });
        }
        *value_count = value_count.saturating_add(1);
        if *value_count > max_values {
            return Err(PdfModelError::TooManyValues {
                actual: *value_count,
                limit: max_values,
            });
        }
        match value {
            PdfValue::Reference(id) if !ids.contains(id) => {
                return Err(PdfModelError::MissingObject(*id));
            }
            PdfValue::Array(values) => {
                stack.extend(values.iter().map(|value| (value, depth + 1)));
            }
            PdfValue::Dictionary(dictionary) => {
                stack.extend(dictionary.iter().map(|(_, value)| (value, depth + 1)));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_page_graph(document: &UnvalidatedPdfDocument) -> Result<(), PdfModelError> {
    let catalog = object_dictionary(document, document.catalog)
        .ok_or(PdfModelError::CatalogNotDictionary(document.catalog))?;
    if !is_type(catalog, b"Catalog") {
        return Err(PdfModelError::CatalogTypeMissing(document.catalog));
    }
    let pages_id = reference_value(catalog.get(b"Pages"))
        .ok_or(PdfModelError::CatalogPagesMissing(document.catalog))?;
    let pages = object_dictionary(document, pages_id)
        .ok_or(PdfModelError::PagesRootNotDictionary(pages_id))?;
    if !is_type(pages, b"Pages") {
        return Err(PdfModelError::PagesTypeMissing(pages_id));
    }
    let kids = match pages.get(b"Kids") {
        Some(PdfValue::Array(kids)) => kids,
        _ => return Err(PdfModelError::PagesKidsInvalid(pages_id)),
    };
    let count = match pages.get(b"Count") {
        Some(PdfValue::Integer(count)) => usize::try_from(*count).ok(),
        _ => None,
    };
    if count != Some(kids.len()) {
        return Err(PdfModelError::PagesCountInvalid(pages_id));
    }
    let mut annotation_owners = BTreeSet::new();
    for kid in kids {
        let page_id =
            reference_value(Some(kid)).ok_or(PdfModelError::PagesKidsInvalid(pages_id))?;
        validate_page(document, page_id, pages_id, &mut annotation_owners)?;
    }
    Ok(())
}

fn validate_page(
    document: &UnvalidatedPdfDocument,
    page_id: PdfObjectId,
    pages_id: PdfObjectId,
    annotation_owners: &mut BTreeSet<PdfObjectId>,
) -> Result<(), PdfModelError> {
    let page =
        object_dictionary(document, page_id).ok_or(PdfModelError::PageNotDictionary(page_id))?;
    if !is_type(page, b"Page") {
        return Err(PdfModelError::PageTypeMissing(page_id));
    }
    if reference_value(page.get(b"Parent")) != Some(pages_id) {
        return Err(PdfModelError::PageParentInvalid(page_id));
    }
    let media_box_valid = page.raw_entries_contain(b"/MediaBox")
        || matches!(
            page.get(b"MediaBox"),
            Some(PdfValue::Array(values)) if values.len() == 4 && values.iter().all(is_number)
        );
    if !media_box_valid {
        return Err(PdfModelError::PageMediaBoxInvalid(page_id));
    }
    let resources_valid = match page.get(b"Resources") {
        Some(PdfValue::Dictionary(_)) => true,
        value => reference_value(value)
            .and_then(|id| object_dictionary(document, id))
            .is_some(),
    };
    if !resources_valid {
        return Err(PdfModelError::PageResourcesInvalid(page_id));
    }
    let contents_valid = match page.get(b"Contents") {
        value @ Some(PdfValue::Reference(_)) => reference_value(value)
            .and_then(|id| object(document, id))
            .is_some_and(|object| matches!(object, PdfObject::Stream { .. })),
        Some(PdfValue::Array(values)) => values.iter().all(|value| {
            reference_value(Some(value))
                .and_then(|id| object(document, id))
                .is_some_and(|object| matches!(object, PdfObject::Stream { .. }))
        }),
        _ => false,
    };
    if !contents_valid {
        return Err(PdfModelError::PageContentsInvalid(page_id));
    }
    if let Some(annotations) = page.get(b"Annots") {
        let PdfValue::Array(annotations) = annotations else {
            return Err(PdfModelError::PageAnnotationsInvalid(page_id));
        };
        for annotation in annotations {
            let Some(id) = reference_value(Some(annotation)) else {
                return Err(PdfModelError::PageAnnotationsInvalid(page_id));
            };
            if !matches!(object(document, id), Some(PdfObject::Annotation(_))) {
                return Err(PdfModelError::AnnotationNotTyped(id));
            }
            if !annotation_owners.insert(id) {
                return Err(PdfModelError::AnnotationOwnedByMultiplePages(id));
            }
        }
    }
    Ok(())
}

fn object(document: &UnvalidatedPdfDocument, id: PdfObjectId) -> Option<&PdfObject> {
    document
        .objects
        .binary_search_by_key(&id, |object| object.id)
        .ok()
        .map(|index| &document.objects[index].object)
}

fn object_dictionary(document: &UnvalidatedPdfDocument, id: PdfObjectId) -> Option<&PdfDictionary> {
    match object(document, id) {
        Some(PdfObject::Value(PdfValue::Dictionary(dictionary))) => Some(dictionary),
        _ => None,
    }
}

fn reference_value(value: Option<&PdfValue>) -> Option<PdfObjectId> {
    match value {
        Some(PdfValue::Reference(id)) => Some(*id),
        _ => None,
    }
}

fn is_type(dictionary: &PdfDictionary, expected: &[u8]) -> bool {
    matches!(dictionary.get(b"Type"), Some(PdfValue::Name(name)) if name.as_bytes() == expected)
}

fn is_number(value: &PdfValue) -> bool {
    matches!(value, PdfValue::Integer(_) | PdfValue::Number(_))
}

fn hash_object(object: &PdfObject, hasher: &mut CanonicalHasher) {
    match object {
        PdfObject::Value(value) => {
            hasher.byte(0);
            hash_value(value, hasher);
        }
        PdfObject::Stream { dictionary, data } => {
            hasher.byte(1);
            hash_dictionary(dictionary, hasher);
            hasher.bytes(data);
        }
        PdfObject::FormXObject {
            dictionary,
            data,
            bbox,
            matrix,
        } => {
            hasher.byte(3);
            hash_dictionary(dictionary, hasher);
            for value in bbox {
                hasher.i64(value.coefficient());
                hasher.byte(value.decimal_places());
            }
            hasher.byte(u8::from(matrix.is_some()));
            if let Some(matrix) = matrix {
                for value in matrix {
                    hasher.i64(value.coefficient());
                    hasher.byte(value.decimal_places());
                }
            }
            hasher.bytes(data);
        }
        PdfObject::Raw(data) => {
            hasher.byte(2);
            hasher.bytes(data);
        }
        PdfObject::Annotation(annotation) => {
            hasher.byte(3);
            for number in annotation.rect {
                hasher.i64(number.coefficient());
                hasher.byte(number.decimal_places());
            }
            hasher.byte(match annotation.subtype {
                None => 0,
                Some(PdfAnnotationType::Link) => 1,
            });
            hasher.bytes(&annotation.raw_entries);
            hash_annotation_action(annotation.action.as_ref(), hasher);
        }
        PdfObject::ImageXObject { image, data } => {
            hasher.byte(4);
            hasher.u32(image.width);
            hasher.u32(image.height);
            hasher.byte(image.bits_per_component);
            hasher.byte(match image.color_space {
                PdfImageColorSpace::DeviceGray => 0,
                PdfImageColorSpace::DeviceRgb => 1,
                PdfImageColorSpace::DeviceCmyk => 2,
            });
            match image.filter {
                PdfImageFilter::Dct => hasher.byte(0),
                PdfImageFilter::Flate => hasher.byte(1),
                PdfImageFilter::FlatePngPredictor { colors } => {
                    hasher.byte(2);
                    hasher.byte(colors);
                }
            }
            hasher.u32(image.soft_mask.map_or(0, PdfObjectId::get));
            hasher.bytes(data);
        }
        PdfObject::Destination(destination) | PdfObject::NamedDestination(destination) => {
            hasher.byte(if matches!(object, PdfObject::Destination(_)) {
                5
            } else {
                6
            });
            hasher.u32(destination.page.get());
            hash_destination_view(&destination.view, hasher);
        }
        PdfObject::DestinationNameTree(tree) => {
            hasher.byte(7);
            hasher.bool(tree.limits.is_some());
            if let Some((min, max)) = &tree.limits {
                hasher.bytes(min);
                hasher.bytes(max);
            }
            match &tree.children {
                PdfDestinationNameTreeChildren::Names(entries) => {
                    hasher.byte(0);
                    hasher.len(entries.len());
                    for (name, id) in entries {
                        hasher.bytes(name);
                        hasher.u32(id.get());
                    }
                }
                PdfDestinationNameTreeChildren::Kids(kids) => {
                    hasher.byte(1);
                    hasher.len(kids.len());
                    for id in kids {
                        hasher.u32(id.get());
                    }
                }
            }
        }
        PdfObject::Names(names) => {
            hasher.byte(8);
            hasher.u32(names.destinations.map_or(0, PdfObjectId::get));
            hasher.bytes(&names.raw_entries);
        }
        PdfObject::Action(action) => {
            hasher.byte(9);
            hash_annotation_action(Some(action), hasher);
        }
        PdfObject::PdfStringSyntax(value) => {
            hasher.byte(10);
            hasher.bytes(value);
        }
        PdfObject::Outline(outline) => {
            hasher.byte(11);
            hasher.u32(outline.first.get());
            hasher.u32(outline.last.get());
            hasher.i64(i64::from(outline.visible_count));
        }
        PdfObject::OutlineItem(item) => {
            hasher.byte(12);
            hasher.u32(item.title.get());
            hasher.u32(item.action.get());
            hasher.u32(item.parent.get());
            for id in [item.previous, item.next, item.first, item.last] {
                hasher.u32(id.map_or(0, PdfObjectId::get));
            }
            hasher.bool(item.count.is_some());
            if let Some(count) = item.count {
                hasher.i64(i64::from(count));
            }
            hasher.bytes(&item.raw_entries);
        }
    }
}

fn hash_destination_view(view: &PdfDestinationView, hasher: &mut CanonicalHasher) {
    let number = |value: &PdfNumber, hasher: &mut CanonicalHasher| {
        hasher.i64(value.coefficient());
        hasher.byte(value.decimal_places());
    };
    match view {
        PdfDestinationView::Xyz { left, top, zoom } => {
            hasher.byte(0);
            number(left, hasher);
            number(top, hasher);
            hasher.bool(zoom.is_some());
            if let Some(zoom) = zoom {
                number(zoom, hasher);
            }
        }
        PdfDestinationView::FitBoundingBoxHorizontal { top } => {
            hasher.byte(1);
            number(top, hasher);
        }
        PdfDestinationView::FitBoundingBoxVertical { left } => {
            hasher.byte(2);
            number(left, hasher);
        }
        PdfDestinationView::FitBoundingBox => hasher.byte(3),
        PdfDestinationView::FitHorizontal { top } => {
            hasher.byte(4);
            number(top, hasher);
        }
        PdfDestinationView::FitVertical { left } => {
            hasher.byte(5);
            number(left, hasher);
        }
        PdfDestinationView::FitRectangle {
            left,
            bottom,
            right,
            top,
        } => {
            hasher.byte(6);
            number(left, hasher);
            number(bottom, hasher);
            number(right, hasher);
            number(top, hasher);
        }
        PdfDestinationView::Fit => hasher.byte(7),
    }
}

fn hash_annotation_action(action: Option<&PdfAnnotationAction>, hasher: &mut CanonicalHasher) {
    let Some(action) = action else {
        hasher.byte(0);
        return;
    };
    match action {
        PdfAnnotationAction::UserEntries(entries) => {
            hasher.byte(1);
            hasher.bytes(entries);
        }
        PdfAnnotationAction::Destination(action) => {
            hasher.byte(2);
            hasher.byte(match action.kind {
                PdfDestinationActionKind::GoTo => 0,
                PdfDestinationActionKind::Thread => 1,
            });
            hasher.bool(action.file.is_some());
            if let Some(file) = &action.file {
                hasher.bytes(file);
            }
            match &action.target {
                PdfDestinationTarget::Reference(id) => {
                    hasher.byte(3);
                    hasher.u32(id.get());
                }
                PdfDestinationTarget::Page { page, view } => {
                    hasher.byte(0);
                    match page {
                        PdfDestinationPage::Internal(id) => {
                            hasher.byte(0);
                            hasher.u32(id.get());
                        }
                        PdfDestinationPage::External(number) => {
                            hasher.byte(1);
                            hasher.u32(*number);
                        }
                    }
                    hasher.bytes(view);
                }
                PdfDestinationTarget::Name(name) => {
                    hasher.byte(1);
                    hasher.bytes(name);
                }
                PdfDestinationTarget::Number(number) => {
                    hasher.byte(2);
                    hasher.u32(*number);
                }
            }
            match &action.structure {
                None => hasher.byte(0),
                Some(PdfDestinationStructure::Internal(id)) => {
                    hasher.byte(1);
                    hasher.u32(id.get());
                }
                Some(PdfDestinationStructure::External(value)) => {
                    hasher.byte(2);
                    hasher.bytes(value);
                }
            }
            hasher.byte(match action.new_window {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            });
        }
    }
}

fn hash_value(value: &PdfValue, hasher: &mut CanonicalHasher) {
    match value {
        PdfValue::Null => hasher.byte(0),
        PdfValue::Bool(value) => {
            hasher.byte(1);
            hasher.byte(u8::from(*value));
        }
        PdfValue::Integer(value) => {
            hasher.byte(2);
            hasher.i64(*value);
        }
        PdfValue::Number(value) => {
            hasher.byte(3);
            hasher.i64(value.coefficient());
            hasher.byte(value.decimal_places());
        }
        PdfValue::Name(name) => {
            hasher.byte(4);
            hasher.bytes(name.as_bytes());
        }
        PdfValue::String(value) => {
            hasher.byte(5);
            hasher.bytes(value);
        }
        PdfValue::Array(values) => {
            hasher.byte(6);
            hasher.len(values.len());
            for value in values {
                hash_value(value, hasher);
            }
        }
        PdfValue::Dictionary(dictionary) => {
            hasher.byte(7);
            hash_dictionary(dictionary, hasher);
        }
        PdfValue::Reference(id) => {
            hasher.byte(8);
            hasher.u32(id.get());
        }
    }
}

fn hash_dictionary(dictionary: &PdfDictionary, hasher: &mut CanonicalHasher) {
    hasher.len(dictionary.len());
    for (key, value) in dictionary.iter() {
        hasher.bytes(key.as_bytes());
        hash_value(value, hasher);
    }
    hasher.bytes(dictionary.raw_entries());
}

struct CanonicalHasher(Sha256);

impl CanonicalHasher {
    fn new() -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"umber-pdf-document\0");
        hasher.update([1]);
        Self(hasher)
    }

    fn byte(&mut self, value: u8) {
        self.0.update([value]);
    }

    fn bool(&mut self, value: bool) {
        self.byte(u8::from(value));
    }

    fn u32(&mut self, value: u32) {
        self.0.update(value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.0.update(value.to_le_bytes());
    }

    fn len(&mut self, value: usize) {
        self.0.update((value as u64).to_le_bytes());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.len(value.len());
        self.0.update(value);
    }

    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

#[cfg(test)]
mod tests;
