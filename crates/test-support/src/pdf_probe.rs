//! Bounded semantic inspection of PDF files for host-side tests.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow, bail};
use hayro_syntax::Pdf;
use hayro_syntax::content::UntypedIter;
use hayro_syntax::object::{Dict, MaybeRef, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::{Resources, Rotation};
use sha2::{Digest, Sha256};

/// Limits applied independently to each public projection operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeLimits {
    /// Maximum nesting depth of arrays, dictionaries, streams, and references.
    pub max_depth: usize,
    /// Maximum number of indirect-object resolutions.
    pub max_objects: usize,
    /// Maximum number of projected values and content instructions.
    pub max_values: usize,
    /// Maximum total number of raw and decoded stream bytes materialized.
    pub max_stream_bytes: usize,
}

impl Default for ProbeLimits {
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
pub struct ProbeObjectId {
    pub number: i32,
    pub generation: i32,
}

impl ProbeObjectId {
    #[must_use]
    pub const fn new(number: i32, generation: i32) -> Self {
        Self { number, generation }
    }
}

impl From<ObjectIdentifier> for ProbeObjectId {
    fn from(value: ObjectIdentifier) -> Self {
        Self::new(value.obj_number, value.gen_number)
    }
}

impl From<ProbeObjectId> for ObjectIdentifier {
    fn from(value: ProbeObjectId) -> Self {
        Self::new(value.number, value.generation)
    }
}

/// An owned, identity-preserving PDF value projection.
#[derive(Clone, Debug, PartialEq)]
pub enum ProbeValue {
    Null,
    Boolean(bool),
    Number(f64),
    String(Vec<u8>),
    Name(Vec<u8>),
    Array(Vec<Self>),
    Dictionary(ProbeDictionary),
    Stream(ProbeStream),
    /// A resolved indirect edge. The edge identity is retained alongside its target.
    Reference {
        id: ProbeObjectId,
        target: Box<Self>,
    },
    /// An edge to an object already active in the current traversal.
    BackReference(ProbeObjectId),
    /// A syntactically valid reference absent from the selected xref.
    UnresolvedReference(ProbeObjectId),
}

impl ProbeValue {
    #[must_use]
    pub fn referenced_id(&self) -> Option<ProbeObjectId> {
        match self {
            Self::Reference { id, .. }
            | Self::BackReference(id)
            | Self::UnresolvedReference(id) => Some(*id),
            _ => None,
        }
    }

    #[must_use]
    pub fn resolved(&self) -> &Self {
        match self {
            Self::Reference { target, .. } => target.resolved(),
            _ => self,
        }
    }

    #[must_use]
    pub fn as_dictionary(&self) -> Option<&ProbeDictionary> {
        match self.resolved() {
            Self::Dictionary(dictionary) => Some(dictionary),
            Self::Stream(stream) => Some(&stream.dictionary),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self.resolved() {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }
}

/// An owned dictionary whose indirect identity is retained when available.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeDictionary {
    pub id: Option<ProbeObjectId>,
    pub entries: BTreeMap<Vec<u8>, ProbeValue>,
}

impl ProbeDictionary {
    #[must_use]
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<&ProbeValue> {
        self.entries.get(key.as_ref())
    }
}

/// Raw and decoded views of a stream plus a complete-content fingerprint.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeStream {
    pub id: ProbeObjectId,
    pub dictionary: ProbeDictionary,
    pub raw: Vec<u8>,
    pub decoded: Vec<u8>,
    pub decoded_sha256: [u8; 32],
    pub operations: Vec<ProbeOperation>,
}

/// One decoded content-stream instruction.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeOperation {
    pub operands: Vec<ProbeValue>,
    pub operator: Vec<u8>,
}

/// A resource category with inheritance layers ordered from ancestor to child.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbeResources {
    pub categories: BTreeMap<Vec<u8>, Vec<ProbeDictionary>>,
}

/// One page in document order.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbePage {
    pub number: usize,
    pub id: ProbeObjectId,
    pub dictionary: ProbeDictionary,
    pub media_box: [f64; 4],
    pub crop_box: [f64; 4],
    pub rotation_degrees: i32,
    pub resources: ProbeResources,
    pub annotations: Vec<ProbeValue>,
    pub content: Option<ProbeContent>,
}

/// Decoded page content and its leniency guard.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeContent {
    pub decoded: Vec<u8>,
    pub decoded_sha256: [u8; 32],
    pub operations: Vec<ProbeOperation>,
}

/// Hayro-backed semantic access to a parsed PDF.
pub struct PdfProbe {
    pdf: Pdf,
    limits: ProbeLimits,
}

impl PdfProbe {
    /// Parse bytes and retain the independent Hayro document for bounded queries.
    pub fn new(bytes: impl AsRef<[u8]>, limits: ProbeLimits) -> Result<Self> {
        if limits.max_depth == 0
            || limits.max_objects == 0
            || limits.max_values == 0
            || limits.max_stream_bytes == 0
        {
            bail!("PDF probe limits must all be nonzero");
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
    pub fn root_id(&self) -> ProbeObjectId {
        self.pdf.xref().root_id().into()
    }

    /// Project the selected trailer, including xref-stream dictionaries.
    pub fn trailer(&self) -> Result<Option<ProbeDictionary>> {
        self.pdf
            .xref()
            .trailer()
            .map(|dictionary| self.project_dictionary(dictionary))
            .transpose()
    }

    pub fn root(&self) -> Result<ProbeDictionary> {
        self.dictionary(self.root_id())
            .context("PDF root is not a dictionary")
    }

    /// Resolve and project an arbitrary normal or object-stream object.
    pub fn object(&self, id: ProbeObjectId) -> Result<ProbeValue> {
        let object = self
            .pdf
            .xref()
            .get::<Object<'_>>(id.into())
            .with_context(|| format!("PDF object {} {} is missing", id.number, id.generation))?;
        let mut state = ProjectionState::new(self.limits);
        state.objects = 1;
        state.active.insert(id);
        project_object(self.pdf.xref(), object, 0, &mut state)
    }

    pub fn dictionary(&self, id: ProbeObjectId) -> Result<ProbeDictionary> {
        match self.object(id)?.resolved() {
            ProbeValue::Dictionary(dictionary) => Ok(dictionary.clone()),
            ProbeValue::Stream(stream) => Ok(stream.dictionary.clone()),
            _ => bail!(
                "PDF object {} {} is not a dictionary",
                id.number,
                id.generation
            ),
        }
    }

    /// Project pages in page-tree order, including inherited page attributes.
    pub fn pages(&self) -> Result<Vec<ProbePage>> {
        let mut state = ProjectionState::new(self.limits);
        self.pdf
            .pages()
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let id = page
                    .raw()
                    .obj_id()
                    .map(ProbeObjectId::from)
                    .context("ordered page has no indirect identity")?;
                state.bump_object()?;
                state.active.insert(id);
                let dictionary = project_dict(self.pdf.xref(), page.raw().clone(), 0, &mut state)?;
                state.active.remove(&id);
                let annotations = dictionary
                    .get(b"Annots")
                    .and_then(ProbeValue::as_array)
                    .map(<[ProbeValue]>::to_vec)
                    .unwrap_or_default();
                let resources = project_resources(self.pdf.xref(), page.resources(), &mut state)?;
                let content = page
                    .page_stream()
                    .map(|decoded| project_content(self.pdf.xref(), decoded, &mut state))
                    .transpose()?;
                let media_box = page.media_box();
                let crop_box = page.crop_box();
                Ok(ProbePage {
                    number: index + 1,
                    id,
                    dictionary,
                    media_box: [media_box.x0, media_box.y0, media_box.x1, media_box.y1],
                    crop_box: [crop_box.x0, crop_box.y0, crop_box.x1, crop_box.y1],
                    rotation_degrees: rotation_degrees(page.rotation()),
                    resources,
                    annotations,
                    content,
                })
            })
            .collect()
    }

    fn project_dictionary(&self, dictionary: Dict<'_>) -> Result<ProbeDictionary> {
        let mut state = ProjectionState::new(self.limits);
        if let Some(id) = dictionary.obj_id().map(ProbeObjectId::from) {
            state.bump_object()?;
            state.active.insert(id);
        }
        project_dict(self.pdf.xref(), dictionary, 0, &mut state)
    }
}

fn rotation_degrees(rotation: Rotation) -> i32 {
    match rotation {
        Rotation::None => 0,
        Rotation::Horizontal => 90,
        Rotation::Flipped => 180,
        Rotation::FlippedHorizontal => 270,
    }
}

struct ProjectionState {
    limits: ProbeLimits,
    objects: usize,
    values: usize,
    stream_bytes: usize,
    active: BTreeSet<ProbeObjectId>,
}

impl ProjectionState {
    fn new(limits: ProbeLimits) -> Self {
        Self {
            limits,
            objects: 0,
            values: 0,
            stream_bytes: 0,
            active: BTreeSet::new(),
        }
    }

    fn check_depth(&self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth {
            bail!(
                "PDF probe depth budget exceeded ({})",
                self.limits.max_depth
            );
        }
        Ok(())
    }

    fn bump_object(&mut self) -> Result<()> {
        self.objects = self.objects.saturating_add(1);
        if self.objects > self.limits.max_objects {
            bail!(
                "PDF probe object budget exceeded ({})",
                self.limits.max_objects
            );
        }
        Ok(())
    }

    fn bump_value(&mut self) -> Result<()> {
        self.values = self.values.saturating_add(1);
        if self.values > self.limits.max_values {
            bail!(
                "PDF probe value budget exceeded ({})",
                self.limits.max_values
            );
        }
        Ok(())
    }

    fn add_stream_bytes(&mut self, count: usize) -> Result<()> {
        self.stream_bytes = self.stream_bytes.saturating_add(count);
        if self.stream_bytes > self.limits.max_stream_bytes {
            bail!(
                "PDF probe stream budget exceeded ({})",
                self.limits.max_stream_bytes
            );
        }
        Ok(())
    }
}

fn project_maybe_ref(
    xref: &hayro_syntax::xref::XRef,
    value: MaybeRef<Object<'_>>,
    depth: usize,
    state: &mut ProjectionState,
) -> Result<ProbeValue> {
    state.check_depth(depth)?;
    match value {
        MaybeRef::NotRef(object) => project_object(xref, object, depth, state),
        MaybeRef::Ref(reference) => {
            state.bump_value()?;
            let id = ProbeObjectId::new(reference.obj_number, reference.gen_number);
            if state.active.contains(&id) {
                return Ok(ProbeValue::BackReference(id));
            }
            state.bump_object()?;
            let Some(object) = xref.get::<Object<'_>>(id.into()) else {
                return Ok(ProbeValue::UnresolvedReference(id));
            };
            state.active.insert(id);
            let target = project_object(xref, object, depth + 1, state)?;
            state.active.remove(&id);
            Ok(ProbeValue::Reference {
                id,
                target: Box::new(target),
            })
        }
    }
}

fn project_object(
    xref: &hayro_syntax::xref::XRef,
    object: Object<'_>,
    depth: usize,
    state: &mut ProjectionState,
) -> Result<ProbeValue> {
    state.check_depth(depth)?;
    state.bump_value()?;
    match object {
        Object::Null(_) => Ok(ProbeValue::Null),
        Object::Boolean(value) => Ok(ProbeValue::Boolean(value)),
        Object::Number(value) => Ok(ProbeValue::Number(value.as_f64())),
        Object::String(value) => Ok(ProbeValue::String(value.as_bytes().to_vec())),
        Object::Name(value) => Ok(ProbeValue::Name(value.as_ref().to_vec())),
        Object::Array(array) => array
            .raw_iter()
            .map(|value| project_maybe_ref(xref, value, depth + 1, state))
            .collect::<Result<Vec<_>>>()
            .map(ProbeValue::Array),
        Object::Dict(dictionary) => {
            project_dict(xref, dictionary, depth + 1, state).map(ProbeValue::Dictionary)
        }
        Object::Stream(stream) => {
            project_stream(xref, stream, depth + 1, state).map(ProbeValue::Stream)
        }
    }
}

fn project_dict(
    xref: &hayro_syntax::xref::XRef,
    dictionary: Dict<'_>,
    depth: usize,
    state: &mut ProjectionState,
) -> Result<ProbeDictionary> {
    state.check_depth(depth)?;
    let id = dictionary.obj_id().map(ProbeObjectId::from);
    let entries = dictionary
        .entries()
        .map(|(key, value)| {
            Ok((
                key.as_ref().to_vec(),
                project_maybe_ref(xref, value, depth + 1, state)?,
            ))
        })
        .collect::<Result<_>>()?;
    Ok(ProbeDictionary { id, entries })
}

fn project_stream(
    xref: &hayro_syntax::xref::XRef,
    stream: Stream<'_>,
    depth: usize,
    state: &mut ProjectionState,
) -> Result<ProbeStream> {
    state.check_depth(depth)?;
    let raw = stream.raw_data().into_owned();
    let decoded = stream
        .decoded()
        .map_err(|error| anyhow!("failed to decode PDF stream: {error:?}"))?
        .into_owned();
    state.add_stream_bytes(raw.len().saturating_add(decoded.len()))?;
    let operations = project_operations(xref, &decoded, state)?;
    Ok(ProbeStream {
        id: stream.obj_id().into(),
        dictionary: project_dict(xref, stream.dict().clone(), depth + 1, state)?,
        raw,
        decoded_sha256: Sha256::digest(&decoded).into(),
        decoded,
        operations,
    })
}

fn project_content(
    xref: &hayro_syntax::xref::XRef,
    decoded: &[u8],
    state: &mut ProjectionState,
) -> Result<ProbeContent> {
    state.add_stream_bytes(decoded.len())?;
    Ok(ProbeContent {
        decoded: decoded.to_vec(),
        decoded_sha256: Sha256::digest(decoded).into(),
        operations: project_operations(xref, decoded, state)?,
    })
}

fn project_operations(
    xref: &hayro_syntax::xref::XRef,
    decoded: &[u8],
    state: &mut ProjectionState,
) -> Result<Vec<ProbeOperation>> {
    let mut iterator = UntypedIter::new(decoded);
    let mut operations = Vec::new();
    while let Some(instruction) = iterator.next() {
        state.bump_value()?;
        let operands = instruction
            .operands()
            .map(|operand| project_object(xref, operand.clone(), 0, state))
            .collect::<Result<_>>()?;
        operations.push(ProbeOperation {
            operands,
            operator: instruction.operator.as_ref().to_vec(),
        });
    }
    Ok(operations)
}

fn project_resources(
    xref: &hayro_syntax::xref::XRef,
    resources: &Resources<'_>,
    state: &mut ProjectionState,
) -> Result<ProbeResources> {
    let mut chain = Vec::new();
    let mut current = Some(resources);
    while let Some(layer) = current {
        chain.push(layer);
        current = layer.parent();
    }
    chain.reverse();

    let mut categories: BTreeMap<Vec<u8>, Vec<ProbeDictionary>> = BTreeMap::new();
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
            if dictionary.keys().next().is_some() {
                categories
                    .entry(name.to_vec())
                    .or_default()
                    .push(project_dict(xref, dictionary.clone(), 0, state)?);
            }
        }
    }
    Ok(ProbeResources { categories })
}

#[cfg(test)]
mod tests;
