//! Detached deterministic PDF document graph.
//!
//! This module owns structural PDF state and semantic identity. Final PDF byte
//! serialization is deliberately separate and must use the `pdf_writer` crate.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

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

/// Encodes filled rule rectangles exclusively through `pdf_writer`.
#[must_use]
pub fn filled_rectangle_content(rectangles: &[PdfContentRectangle]) -> Vec<u8> {
    let mut content = pdf_writer::Content::new();
    content.save_state();
    for rectangle in rectangles {
        content
            .rect(rectangle.x, rectangle.y, rectangle.width, rectangle.height)
            .fill_nonzero();
    }
    content.restore_state();
    content.finish().to_vec()
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
    Stream {
        dictionary: PdfDictionary,
        data: Vec<u8>,
    },
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
    /// Optional document-information dictionary registered in the file trailer.
    pub info: Option<PdfObjectId>,
    pub objects: Vec<PdfIndirectObject>,
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
        self.0.info
    }

    pub fn objects(&self) -> impl ExactSizeIterator<Item = &PdfIndirectObject> {
        self.0.objects.iter()
    }

    /// Hashes a versioned canonical structural encoding, independent of input order.
    #[must_use]
    pub fn semantic_hash(&self) -> PdfDocumentHash {
        let mut hasher = CanonicalHasher::new();
        hasher.byte(self.version().major());
        hasher.byte(self.version().minor());
        hasher.u32(self.catalog().get());
        hasher.byte(u8::from(self.info().is_some()));
        if let Some(info) = self.info() {
            hasher.u32(info.get());
        }
        hasher.len(self.0.objects.len());
        for indirect in &self.0.objects {
            hasher.u32(indirect.id.get());
            hash_object(&indirect.object, &mut hasher);
        }
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
    CatalogTypeMissing(PdfObjectId),
    CatalogPagesMissing(PdfObjectId),
    InfoNotDictionary(PdfObjectId),
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
    if let Some(info) = document.info {
        if !ids.contains(&info) {
            return Err(PdfModelError::MissingObject(info));
        }
        if object_dictionary(document, info).is_none() {
            return Err(PdfModelError::InfoNotDictionary(info));
        }
    }

    let mut value_count = 0_usize;
    let mut stream_bytes = 0_usize;
    for indirect in &document.objects {
        if let PdfObject::Stream { dictionary, data } = &indirect.object {
            if dictionary.get(b"Length").is_some() {
                return Err(PdfModelError::ReservedStreamLength(indirect.id));
            }
            stream_bytes = stream_bytes.saturating_add(data.len());
            if stream_bytes > limits.max_stream_bytes {
                return Err(PdfModelError::TooManyStreamBytes {
                    actual: stream_bytes,
                    limit: limits.max_stream_bytes,
                });
            }
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
        PdfObject::Stream { dictionary, .. } => {
            stack.extend(dictionary.iter().map(|(_, value)| (value, 1)))
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
    for kid in kids {
        let page_id =
            reference_value(Some(kid)).ok_or(PdfModelError::PagesKidsInvalid(pages_id))?;
        validate_page(document, page_id, pages_id)?;
    }
    Ok(())
}

fn validate_page(
    document: &UnvalidatedPdfDocument,
    page_id: PdfObjectId,
    pages_id: PdfObjectId,
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
