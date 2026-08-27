//! Canonical raw-object records within the checkpointed PDF ledger.

use crate::state_hash::{StateHashFragment, StateHasher};

use super::{PdfRows, PdfTokenParameter};

const PDF_RAW_OBJECT_DOMAIN: u64 = 0x7064_665f_7261_776f;

/// Typed identity assigned to a raw object by the one PDF allocation ledger.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfRawObjectId(u32);

impl PdfRawObjectId {
    #[must_use]
    pub(crate) const fn from_allocated(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Generation-local engine payload for an initialized `\pdfobj`.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfRawObjectData<G> {
    stream: bool,
    stream_attr: Option<PdfTokenParameter<G>>,
    file: bool,
    data: PdfTokenParameter<G>,
}

impl<G> Clone for PdfRawObjectData<G> {
    fn clone(&self) -> Self {
        Self {
            stream: self.stream,
            stream_attr: self.stream_attr.clone(),
            file: self.file,
            data: self.data.clone(),
        }
    }
}

impl<G> PdfRawObjectData<G> {
    #[must_use]
    pub(crate) fn new(
        stream: bool,
        stream_attr: Option<PdfTokenParameter<G>>,
        file: bool,
        data: PdfTokenParameter<G>,
    ) -> Self {
        Self {
            stream,
            stream_attr,
            file,
            data,
        }
    }

    #[must_use]
    pub const fn is_stream(&self) -> bool {
        self.stream
    }

    #[must_use]
    pub fn stream_attr(&self) -> Option<crate::TokenListId<G>> {
        self.stream_attr.as_ref().map(PdfTokenParameter::id)
    }

    #[must_use]
    pub const fn is_file(&self) -> bool {
        self.file
    }

    #[must_use]
    pub fn data(&self) -> crate::TokenListId<G> {
        self.data.id()
    }
}

/// One reserved raw-object slot, initialized either now or by `useobjnum`.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfRawObjectRecord<G> {
    id: PdfRawObjectId,
    data: Option<PdfRawObjectData<G>>,
    immediate: bool,
    referenced: bool,
}

impl<G> Clone for PdfRawObjectRecord<G> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            data: self.data.clone(),
            immediate: self.immediate,
            referenced: self.referenced,
        }
    }
}

impl<G> PdfRawObjectRecord<G> {
    #[must_use]
    pub const fn id(&self) -> PdfRawObjectId {
        self.id
    }

    #[must_use]
    pub fn data(&self) -> Option<PdfRawObjectData<G>> {
        self.data.clone()
    }

    #[must_use]
    pub const fn is_immediate(&self) -> bool {
        self.immediate
    }

    #[must_use]
    pub const fn is_referenced(&self) -> bool {
        self.referenced
    }

    pub(super) fn initialize_version(
        mut self,
        data: PdfRawObjectData<G>,
        immediate: bool,
    ) -> Result<Self, PdfRawObjectInitializeError> {
        if self.data.is_some() {
            return Err(PdfRawObjectInitializeError::AlreadyInitialized(self.id));
        }
        self.data = Some(data);
        self.immediate = immediate;
        Ok(self)
    }

    pub(super) fn reference_version(mut self) -> Self {
        self.referenced = true;
        self
    }
}

#[derive(Debug)]
struct PdfRawObjectState<G> {
    records: PdfRows<PdfRawObjectRecord<G>>,
    last_object: u32,
    fingerprint: StateHashFragment,
}

/// Owned raw-object table copied into explicit PDF checkpoints.
#[derive(Debug)]
pub(crate) struct PdfRawObjects<G>(PdfRawObjectState<G>);

impl<G> Default for PdfRawObjects<G> {
    fn default() -> Self {
        Self(PdfRawObjectState {
            records: PdfRows::default(),
            last_object: 0,
            fingerprint: StateHasher::new(PDF_RAW_OBJECT_DOMAIN).finish_fragment(),
        })
    }
}

impl<G> PdfRawObjects<G> {
    pub(crate) fn begin_transaction(&mut self, len: usize) {
        self.0.records.begin_transaction(len);
    }

    pub(crate) fn reject_transaction(&mut self) {
        self.0.records.reject_transaction();
    }

    pub(crate) fn accept_transaction(&mut self) {
        self.0.records.accept_transaction();
    }

    pub(crate) fn len(&self) -> usize {
        self.0.records.len()
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        self.0.records.truncate(len);
    }

    pub(crate) fn set_fingerprint(&mut self, fingerprint: StateHashFragment) {
        self.0.fingerprint = fingerprint;
    }

    pub(crate) fn append_version_fingerprint(
        &mut self,
        operation: u8,
        record: &PdfRawObjectRecord<G>,
    ) {
        let mut hasher = StateHasher::new(PDF_RAW_OBJECT_DOMAIN);
        self.0.fingerprint.apply(&mut hasher);
        hasher.u8(operation);
        hasher.u32(self.0.last_object);
        hash_record(&mut hasher, record);
        self.0.fingerprint = hasher.finish_fragment();
    }
    #[must_use]
    pub(crate) fn fingerprint(&self) -> StateHashFragment {
        self.0.fingerprint
    }

    #[must_use]
    pub(crate) fn last_object(&self) -> u32 {
        self.0.last_object
    }

    pub(crate) fn set_last_object(&mut self, object: u32) {
        self.0.last_object = object;
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &PdfRawObjectRecord<G>> {
        self.0.records.iter()
    }

    #[must_use]
    pub(crate) fn record(&self, id: PdfRawObjectId) -> Option<PdfRawObjectRecord<G>> {
        self.0
            .records
            .binary_search_by_key(&id, |record| record.id)
            .ok()
            .map(|index| self.0.records[index].clone())
    }

    pub(crate) fn reserve(&mut self, id: PdfRawObjectId) {
        debug_assert!(self.0.records.last().is_none_or(|record| record.id < id));
        let record = PdfRawObjectRecord {
            id,
            data: None,
            immediate: false,
            referenced: false,
        };
        self.0.records.push(record.clone());
        self.0.last_object = id.raw();
        self.append_version_fingerprint(0, &record);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfRawObjectInitializeError {
    NotFound(PdfRawObjectId),
    AlreadyInitialized(PdfRawObjectId),
}

fn hash_record<G>(hasher: &mut StateHasher, record: &PdfRawObjectRecord<G>) {
    hasher.u32(record.id.raw());
    hasher.bool(record.data.is_some());
    if let Some(data) = &record.data {
        hasher.bool(data.stream);
        hasher.bool(data.stream_attr.is_some());
        if let Some(attr) = &data.stream_attr {
            hasher.bytes(&attr.semantic_id.bytes());
        }
        hasher.bool(data.file);
        hasher.bytes(&data.data.semantic_id.bytes());
    }
    hasher.bool(record.immediate);
    hasher.bool(record.referenced);
}
