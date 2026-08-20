//! Canonical raw-object records within the checkpointed PDF ledger.

use crate::state_hash::{StateHashFragment, StateHasher};

use super::PdfTokenParameter;

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

/// Detached engine-side payload for an initialized `\pdfobj`.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfRawObjectData<G> {
    stream: bool,
    stream_attr: Option<PdfTokenParameter<G>>,
    file: bool,
    data: PdfTokenParameter<G>,
}

impl<G> Copy for PdfRawObjectData<G> {}

impl<G> Clone for PdfRawObjectData<G> {
    fn clone(&self) -> Self {
        *self
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

impl<G> Copy for PdfRawObjectRecord<G> {}

impl<G> Clone for PdfRawObjectRecord<G> {
    fn clone(&self) -> Self {
        *self
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
}

#[derive(Debug)]
struct PdfRawObjectState<G> {
    records: Vec<PdfRawObjectRecord<G>>,
    last_object: u32,
    fingerprint: StateHashFragment,
}

impl<G> Clone for PdfRawObjectState<G> {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            last_object: self.last_object,
            fingerprint: self.fingerprint,
        }
    }
}

/// Owned raw-object table copied into explicit PDF checkpoints.
#[derive(Debug)]
pub(crate) struct PdfRawObjects<G>(PdfRawObjectState<G>);

impl<G> Clone for PdfRawObjects<G> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<G> Default for PdfRawObjects<G> {
    fn default() -> Self {
        Self(PdfRawObjectState {
            records: Vec::new(),
            last_object: 0,
            fingerprint: StateHasher::new(PDF_RAW_OBJECT_DOMAIN).finish_fragment(),
        })
    }
}

impl<G> PdfRawObjects<G> {
    #[must_use]
    pub(crate) fn fingerprint(&self) -> StateHashFragment {
        self.0.fingerprint
    }

    #[must_use]
    pub(crate) fn last_object(&self) -> u32 {
        self.0.last_object
    }

    #[must_use]
    pub(crate) fn records(&self) -> &[PdfRawObjectRecord<G>] {
        &self.0.records
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
        let state = &mut self.0;
        debug_assert!(state.records.last().is_none_or(|record| record.id < id));
        state.records.push(PdfRawObjectRecord {
            id,
            data: None,
            immediate: false,
            referenced: false,
        });
        state.last_object = id.raw();
        state.fingerprint = fingerprint(state);
    }

    pub(crate) fn initialize(
        &mut self,
        id: PdfRawObjectId,
        data: PdfRawObjectData<G>,
        immediate: bool,
    ) -> Result<(), PdfRawObjectInitializeError> {
        let state = &mut self.0;
        let index = state
            .records
            .binary_search_by_key(&id, |record| record.id)
            .map_err(|_| PdfRawObjectInitializeError::NotFound(id))?;
        if state.records[index].data.is_some() {
            return Err(PdfRawObjectInitializeError::AlreadyInitialized(id));
        }
        state.records[index].data = Some(data);
        state.records[index].immediate = immediate;
        state.last_object = id.raw();
        state.fingerprint = fingerprint(state);
        Ok(())
    }

    pub(crate) fn reference(
        &mut self,
        id: PdfRawObjectId,
    ) -> Result<(), PdfRawObjectInitializeError> {
        let state = &mut self.0;
        let index = state
            .records
            .binary_search_by_key(&id, |record| record.id)
            .map_err(|_| PdfRawObjectInitializeError::NotFound(id))?;
        state.records[index].referenced = true;
        state.fingerprint = fingerprint(state);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfRawObjectInitializeError {
    NotFound(PdfRawObjectId),
    AlreadyInitialized(PdfRawObjectId),
}

fn fingerprint<G>(state: &PdfRawObjectState<G>) -> StateHashFragment {
    let mut hasher = StateHasher::new(PDF_RAW_OBJECT_DOMAIN);
    hasher.u32(state.last_object);
    hasher.usize(state.records.len());
    for record in &state.records {
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
    hasher.finish_fragment()
}
