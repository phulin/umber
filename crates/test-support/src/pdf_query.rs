//! Bounded, focused PDF queries for host-side tests.
//!
//! Hayro's parsed document and borrowed objects are the sole object model. The
//! wrappers in this module are shallow handles; only stream bytes and decoded
//! content operations are materialized for callers that explicitly request
//! them.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow, bail};
use hayro_syntax::Pdf;
use hayro_syntax::content::UntypedIter;
use hayro_syntax::object::{Array, Dict, MaybeRef, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::{Page, Resources, Rotation};
use sha2::{Digest, Sha256};

/// Limits applied independently to each focused query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryLimits {
    pub max_depth: usize,
    pub max_objects: usize,
    pub max_values: usize,
    pub max_stream_bytes: usize,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_objects: 16_384,
            max_values: 262_144,
            max_stream_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A stable PDF indirect-object identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct QueryObjectId {
    pub number: i32,
    pub generation: i32,
}

impl QueryObjectId {
    #[must_use]
    pub const fn new(number: i32, generation: i32) -> Self {
        Self { number, generation }
    }
}

impl From<ObjectIdentifier> for QueryObjectId {
    fn from(value: ObjectIdentifier) -> Self {
        Self::new(value.obj_number, value.gen_number)
    }
}

impl From<QueryObjectId> for ObjectIdentifier {
    fn from(value: QueryObjectId) -> Self {
        Self::new(value.number, value.generation)
    }
}

/// A shallow borrowed PDF value. References are resolved at the point of use.
#[derive(Clone)]
pub struct QueryValue<'a> {
    xref: &'a hayro_syntax::xref::XRef,
    reference: Option<QueryObjectId>,
    object: Option<Object<'a>>,
}

impl<'a> QueryValue<'a> {
    fn from_maybe_ref(xref: &'a hayro_syntax::xref::XRef, value: MaybeRef<Object<'a>>) -> Self {
        match value {
            MaybeRef::NotRef(object) => Self {
                xref,
                reference: None,
                object: Some(object),
            },
            MaybeRef::Ref(reference) => {
                let id = QueryObjectId::new(reference.obj_number, reference.gen_number);
                Self {
                    xref,
                    reference: Some(id),
                    object: xref.get::<Object<'a>>(id.into()),
                }
            }
        }
    }

    fn from_object(xref: &'a hayro_syntax::xref::XRef, object: Object<'a>) -> Self {
        Self {
            xref,
            reference: None,
            object: Some(object),
        }
    }

    #[must_use]
    pub fn referenced_id(&self) -> Option<QueryObjectId> {
        self.reference
    }

    #[must_use]
    pub fn is_unresolved(&self) -> bool {
        self.reference.is_some() && self.object.is_none()
    }

    #[must_use]
    pub fn boolean(&self) -> Option<bool> {
        self.object.clone()?.into_bool()
    }

    #[must_use]
    pub fn number(&self) -> Option<f64> {
        Some(self.object.clone()?.into_number()?.as_f64())
    }

    #[must_use]
    pub fn string(&self) -> Option<hayro_syntax::object::String<'a>> {
        self.object.clone()?.into_string()
    }

    #[must_use]
    pub fn name(&self) -> Option<hayro_syntax::object::Name<'a>> {
        self.object.clone()?.into_name()
    }

    #[must_use]
    pub fn array(&self) -> Option<QueryArray<'a>> {
        Some(QueryArray {
            xref: self.xref,
            array: self.object.clone()?.into_array()?,
        })
    }

    #[must_use]
    pub fn as_dictionary(&self) -> Option<QueryDictionary<'a>> {
        match self.object.clone()? {
            Object::Dict(dictionary) => Some(QueryDictionary::new(self.xref, dictionary)),
            Object::Stream(stream) => Some(QueryDictionary::new(self.xref, stream.dict().clone())),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_stream(&self) -> Option<QueryStream<'a>> {
        self.object
            .clone()?
            .into_stream()
            .map(|stream| QueryStream::new(self.xref, stream))
    }

    #[must_use]
    pub(crate) fn object(&self) -> Option<Object<'a>> {
        self.object.clone()
    }
}

/// A shallow borrowed array.
#[derive(Clone)]
pub struct QueryArray<'a> {
    xref: &'a hayro_syntax::xref::XRef,
    array: Array<'a>,
}

impl<'a> QueryArray<'a> {
    pub fn iter(&self) -> impl Iterator<Item = QueryValue<'a>> + '_ {
        self.array
            .raw_iter()
            .map(|value| QueryValue::from_maybe_ref(self.xref, value))
    }
}

/// A shallow borrowed dictionary with sorted Hayro entry iteration.
#[derive(Clone)]
pub struct QueryDictionary<'a> {
    xref: &'a hayro_syntax::xref::XRef,
    dictionary: Dict<'a>,
}

impl<'a> QueryDictionary<'a> {
    fn new(xref: &'a hayro_syntax::xref::XRef, dictionary: Dict<'a>) -> Self {
        Self { xref, dictionary }
    }

    #[must_use]
    pub fn id(&self) -> Option<QueryObjectId> {
        self.dictionary.obj_id().map(Into::into)
    }

    #[must_use]
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<QueryValue<'a>> {
        self.dictionary
            .get_raw::<Object<'a>>(key)
            .map(|value| QueryValue::from_maybe_ref(self.xref, value))
    }

    pub fn entries(&self) -> impl Iterator<Item = (Vec<u8>, QueryValue<'a>)> + '_ {
        self.dictionary.entries().map(|(key, value)| {
            (
                key.as_ref().to_vec(),
                QueryValue::from_maybe_ref(self.xref, value),
            )
        })
    }

    #[must_use]
    pub fn raw_entries_contain(&self, needle: &[u8]) -> bool {
        self.dictionary
            .data()
            .windows(needle.len())
            .any(|window| window == needle)
    }
}

/// Raw and decoded views of one selected stream.
pub struct QueryStream<'a> {
    pub id: QueryObjectId,
    pub dictionary: QueryDictionary<'a>,
    pub raw: Vec<u8>,
    pub decoded: Vec<u8>,
    pub decoded_sha256: [u8; 32],
    xref: &'a hayro_syntax::xref::XRef,
}

impl<'a> QueryStream<'a> {
    fn new(xref: &'a hayro_syntax::xref::XRef, stream: Stream<'a>) -> Self {
        let raw = stream.raw_data().into_owned();
        let decoded = stream
            .decoded()
            .map_or_else(|_| Vec::new(), |decoded| decoded.into_owned());
        Self {
            id: stream.obj_id().into(),
            dictionary: QueryDictionary::new(xref, stream.dict().clone()),
            raw,
            decoded_sha256: Sha256::digest(&decoded).into(),
            decoded,
            xref,
        }
    }

    pub fn operations(&self, limits: QueryLimits) -> Result<Vec<QueryOperation>> {
        let mut budget = QueryBudget::new(limits);
        budget.add_stream_bytes(self.decoded.len())?;
        project_operations(self.xref, &self.decoded, &mut budget)
    }
}

/// One decoded content-stream instruction.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryOperation {
    pub operands: Vec<QueryOperand>,
    pub operator: Vec<u8>,
}

/// An owned projection limited to content-stream operands.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryOperand {
    Null,
    Boolean(bool),
    Number(f64),
    String(Vec<u8>),
    Name(Vec<u8>),
    Array(Vec<Self>),
    Dictionary(BTreeMap<Vec<u8>, Self>),
}

/// A resource category with inheritance layers ordered ancestor to child.
pub struct QueryResources<'a> {
    pub categories: BTreeMap<Vec<u8>, Vec<QueryDictionary<'a>>>,
}

/// One page in document order. Its object values remain borrowed from Hayro.
pub struct QueryPage<'a> {
    pub number: usize,
    pub id: QueryObjectId,
    pub dictionary: QueryDictionary<'a>,
    pub media_box: [f64; 4],
    pub crop_box: [f64; 4],
    pub rotation_degrees: i32,
    pub resources: QueryResources<'a>,
    pub annotations: Vec<QueryValue<'a>>,
    pub content: Option<QueryContent>,
}

/// Decoded page content and operations requested as one focused result.
pub struct QueryContent {
    pub decoded: Vec<u8>,
    pub decoded_sha256: [u8; 32],
    pub operations: Vec<QueryOperation>,
}

/// Hayro-backed semantic access to a parsed PDF.
pub struct PdfQuery {
    pdf: Pdf,
    limits: QueryLimits,
}

impl PdfQuery {
    pub fn new(bytes: impl AsRef<[u8]>, limits: QueryLimits) -> Result<Self> {
        if limits.max_depth == 0
            || limits.max_objects == 0
            || limits.max_values == 0
            || limits.max_stream_bytes == 0
        {
            bail!("PDF query limits must all be nonzero");
        }
        let pdf = Pdf::new(bytes.as_ref().to_vec())
            .map_err(|error| anyhow!("failed to parse PDF: {error:?}"))?;
        Ok(Self { pdf, limits })
    }

    #[must_use]
    pub fn version(&self) -> (u8, u8) {
        use hayro_syntax::PdfVersion::*;
        match self.pdf.version() {
            Pdf10 => (1, 0),
            Pdf11 => (1, 1),
            Pdf12 => (1, 2),
            Pdf13 => (1, 3),
            Pdf14 => (1, 4),
            Pdf15 => (1, 5),
            Pdf16 => (1, 6),
            Pdf17 => (1, 7),
            Pdf20 => (2, 0),
        }
    }

    #[must_use]
    pub fn root_id(&self) -> QueryObjectId {
        self.pdf.xref().root_id().into()
    }

    pub fn trailer(&self) -> Result<Option<QueryDictionary<'_>>> {
        Ok(self
            .pdf
            .xref()
            .trailer()
            .map(|dictionary| QueryDictionary::new(self.pdf.xref(), dictionary)))
    }

    pub fn root(&self) -> Result<QueryDictionary<'_>> {
        self.dictionary(self.root_id())
            .context("PDF root is not a dictionary")
    }

    pub fn object(&self, id: QueryObjectId) -> Result<QueryValue<'_>> {
        let object = self
            .pdf
            .xref()
            .get::<Object<'_>>(id.into())
            .with_context(|| format!("PDF object {} {} is missing", id.number, id.generation))?;
        Ok(QueryValue::from_object(self.pdf.xref(), object))
    }

    /// Walk one object without retaining it, enforcing every configured budget.
    pub fn validate_object(&self, id: QueryObjectId) -> Result<()> {
        let object = self
            .pdf
            .xref()
            .get::<Object<'_>>(id.into())
            .with_context(|| format!("PDF object {} {} is missing", id.number, id.generation))?;
        let mut budget = QueryBudget::new(self.limits);
        budget.bump_object()?;
        validate_object(
            self.pdf.xref(),
            object,
            0,
            &mut BTreeSet::from([id]),
            &mut budget,
        )
    }

    pub fn dictionary(&self, id: QueryObjectId) -> Result<QueryDictionary<'_>> {
        self.object(id)?.as_dictionary().with_context(|| {
            format!(
                "PDF object {} {} is not a dictionary",
                id.number, id.generation
            )
        })
    }

    pub fn stream(&self, id: QueryObjectId) -> Result<QueryStream<'_>> {
        let stream = self.object(id)?.as_stream().with_context(|| {
            format!("PDF object {} {} is not a stream", id.number, id.generation)
        })?;
        let mut budget = QueryBudget::new(self.limits);
        budget.bump_object()?;
        budget.add_stream_bytes(stream.raw.len().saturating_add(stream.decoded.len()))?;
        Ok(stream)
    }

    /// Return pages in page-tree order with inherited geometry and resources.
    pub fn pages(&self) -> Result<Vec<QueryPage<'_>>> {
        let mut budget = QueryBudget::new(self.limits);
        self.pdf
            .pages()
            .iter()
            .enumerate()
            .map(|(index, page)| project_page(self.pdf.xref(), page, index, &mut budget))
            .collect()
    }
}

fn validate_maybe_ref(
    xref: &hayro_syntax::xref::XRef,
    value: MaybeRef<Object<'_>>,
    depth: usize,
    active: &mut BTreeSet<QueryObjectId>,
    budget: &mut QueryBudget,
) -> Result<()> {
    budget.check_depth(depth)?;
    match value {
        MaybeRef::NotRef(object) => validate_object(xref, object, depth, active, budget),
        MaybeRef::Ref(reference) => {
            budget.bump_value()?;
            let id = QueryObjectId::new(reference.obj_number, reference.gen_number);
            if active.contains(&id) {
                return Ok(());
            }
            budget.bump_object()?;
            let Some(object) = xref.get::<Object<'_>>(id.into()) else {
                return Ok(());
            };
            active.insert(id);
            let result = validate_object(xref, object, depth + 1, active, budget);
            active.remove(&id);
            result
        }
    }
}

fn validate_object(
    xref: &hayro_syntax::xref::XRef,
    object: Object<'_>,
    depth: usize,
    active: &mut BTreeSet<QueryObjectId>,
    budget: &mut QueryBudget,
) -> Result<()> {
    budget.check_depth(depth)?;
    budget.bump_value()?;
    match object {
        Object::Array(array) => {
            for value in array.raw_iter() {
                validate_maybe_ref(xref, value, depth + 1, active, budget)?;
            }
        }
        Object::Dict(dictionary) => {
            for (_, value) in dictionary.entries() {
                validate_maybe_ref(xref, value, depth + 1, active, budget)?;
            }
        }
        Object::Stream(stream) => {
            let raw = stream.raw_data();
            let decoded = stream.decoded().unwrap_or_default();
            budget.add_stream_bytes(raw.len().saturating_add(decoded.len()))?;
            for (_, value) in stream.dict().entries() {
                validate_maybe_ref(xref, value, depth + 1, active, budget)?;
            }
            project_operations(xref, &decoded, budget)?;
        }
        _ => {}
    }
    Ok(())
}

fn project_page<'a>(
    xref: &'a hayro_syntax::xref::XRef,
    page: &'a Page<'a>,
    index: usize,
    budget: &mut QueryBudget,
) -> Result<QueryPage<'a>> {
    budget.bump_object()?;
    let id = page
        .raw()
        .obj_id()
        .map(QueryObjectId::from)
        .context("ordered page has no indirect identity")?;
    let dictionary = QueryDictionary::new(xref, page.raw().clone());
    let annotations = dictionary
        .get(b"Annots")
        .and_then(|value| value.array())
        .map(|array| array.iter().collect())
        .unwrap_or_default();
    let resources = project_resources(xref, page.resources());
    let content = page
        .page_stream()
        .map(|decoded| {
            budget.add_stream_bytes(decoded.len())?;
            Ok::<_, anyhow::Error>(QueryContent {
                decoded: decoded.to_vec(),
                decoded_sha256: Sha256::digest(decoded).into(),
                operations: project_operations(xref, decoded, budget)?,
            })
        })
        .transpose()?;
    let media_box = page.media_box();
    let crop_box = page.crop_box();
    Ok(QueryPage {
        number: index + 1,
        id,
        dictionary,
        media_box: [media_box.x0, media_box.y0, media_box.x1, media_box.y1],
        crop_box: [crop_box.x0, crop_box.y0, crop_box.x1, crop_box.y1],
        rotation_degrees: match page.rotation() {
            Rotation::None => 0,
            Rotation::Horizontal => 90,
            Rotation::Flipped => 180,
            Rotation::FlippedHorizontal => 270,
        },
        resources,
        annotations,
        content,
    })
}

fn project_resources<'a>(
    xref: &'a hayro_syntax::xref::XRef,
    resources: &'a Resources<'a>,
) -> QueryResources<'a> {
    let mut chain = Vec::new();
    let mut current = Some(resources);
    while let Some(layer) = current {
        chain.push(layer);
        current = layer.parent();
    }
    chain.reverse();
    let mut categories: BTreeMap<Vec<u8>, Vec<QueryDictionary<'a>>> = BTreeMap::new();
    for layer in chain {
        for (name, dictionary) in [
            (b"ExtGState".as_slice(), &layer.ext_g_states),
            (b"Font".as_slice(), &layer.fonts),
            (b"Properties".as_slice(), &layer.properties),
            (b"ColorSpace".as_slice(), &layer.color_spaces),
            (b"XObject".as_slice(), &layer.x_objects),
            (b"Pattern".as_slice(), &layer.patterns),
            (b"Shading".as_slice(), &layer.shadings),
        ] {
            if !dictionary.is_empty() {
                categories
                    .entry(name.to_vec())
                    .or_default()
                    .push(QueryDictionary::new(xref, dictionary.clone()));
            }
        }
    }
    QueryResources { categories }
}

fn project_operations(
    xref: &hayro_syntax::xref::XRef,
    decoded: &[u8],
    budget: &mut QueryBudget,
) -> Result<Vec<QueryOperation>> {
    let mut iterator = UntypedIter::new(decoded);
    let mut operations = Vec::new();
    while let Some(instruction) = iterator.next() {
        budget.bump_value()?;
        let operands = instruction
            .operands()
            .map(|operand| project_operand(xref, operand.clone(), 0, budget))
            .collect::<Result<_>>()?;
        operations.push(QueryOperation {
            operands,
            operator: instruction.operator.as_ref().to_vec(),
        });
    }
    Ok(operations)
}

fn project_operand(
    xref: &hayro_syntax::xref::XRef,
    object: Object<'_>,
    depth: usize,
    budget: &mut QueryBudget,
) -> Result<QueryOperand> {
    budget.check_depth(depth)?;
    budget.bump_value()?;
    Ok(match object {
        Object::Null(_) => QueryOperand::Null,
        Object::Boolean(value) => QueryOperand::Boolean(value),
        Object::Number(value) => QueryOperand::Number(value.as_f64()),
        Object::String(value) => QueryOperand::String(value.as_bytes().to_vec()),
        Object::Name(value) => QueryOperand::Name(value.as_ref().to_vec()),
        Object::Array(array) => QueryOperand::Array(
            array
                .raw_iter()
                .map(|value| match value {
                    MaybeRef::NotRef(value) => project_operand(xref, value, depth + 1, budget),
                    MaybeRef::Ref(reference) => {
                        budget.bump_object()?;
                        let id = ObjectIdentifier::new(reference.obj_number, reference.gen_number);
                        let value = xref
                            .get::<Object<'_>>(id)
                            .context("unresolved indirect reference in content-stream operand")?;
                        project_operand(xref, value, depth + 1, budget)
                    }
                })
                .collect::<Result<_>>()?,
        ),
        Object::Dict(dictionary) => QueryOperand::Dictionary(
            dictionary
                .entries()
                .map(|(key, value)| {
                    let value = match value {
                        MaybeRef::NotRef(value) => project_operand(xref, value, depth + 1, budget)?,
                        MaybeRef::Ref(reference) => {
                            budget.bump_object()?;
                            let id =
                                ObjectIdentifier::new(reference.obj_number, reference.gen_number);
                            project_operand(
                                xref,
                                xref.get::<Object<'_>>(id).context(
                                    "unresolved indirect reference in content-stream operand",
                                )?,
                                depth + 1,
                                budget,
                            )?
                        }
                    };
                    Ok((key.as_ref().to_vec(), value))
                })
                .collect::<Result<_>>()?,
        ),
        Object::Stream(_) => bail!("content-stream operand cannot be a stream"),
    })
}

pub(crate) struct QueryBudget {
    limits: QueryLimits,
    objects: usize,
    values: usize,
    stream_bytes: usize,
}

impl QueryBudget {
    pub(crate) fn new(limits: QueryLimits) -> Self {
        Self {
            limits,
            objects: 0,
            values: 0,
            stream_bytes: 0,
        }
    }

    pub(crate) fn check_depth(&self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth {
            bail!(
                "PDF query depth budget exceeded ({})",
                self.limits.max_depth
            );
        }
        Ok(())
    }

    pub(crate) fn bump_object(&mut self) -> Result<()> {
        self.objects = self.objects.saturating_add(1);
        if self.objects > self.limits.max_objects {
            bail!(
                "PDF query object budget exceeded ({})",
                self.limits.max_objects
            );
        }
        Ok(())
    }

    pub(crate) fn bump_value(&mut self) -> Result<()> {
        self.values = self.values.saturating_add(1);
        if self.values > self.limits.max_values {
            bail!(
                "PDF query value budget exceeded ({})",
                self.limits.max_values
            );
        }
        Ok(())
    }

    pub(crate) fn add_stream_bytes(&mut self, count: usize) -> Result<()> {
        self.stream_bytes = self.stream_bytes.saturating_add(count);
        if self.stream_bytes > self.limits.max_stream_bytes {
            bail!(
                "PDF query stream budget exceeded ({})",
                self.limits.max_stream_bytes
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
