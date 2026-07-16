//! Canonical raw-object records within the checkpointed PDF ledger.

use std::sync::Arc;

use crate::ids::TokenListId;
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfRawObjectData {
    stream: bool,
    stream_attr: Option<PdfTokenParameter>,
    file: bool,
    data: PdfTokenParameter,
}

impl PdfRawObjectData {
    #[must_use]
    pub(crate) const fn new(
        stream: bool,
        stream_attr: Option<PdfTokenParameter>,
        file: bool,
        data: PdfTokenParameter,
    ) -> Self {
        Self {
            stream,
            stream_attr,
            file,
            data,
        }
    }

    #[must_use]
    pub const fn is_stream(self) -> bool {
        self.stream
    }

    #[must_use]
    pub const fn stream_attr(self) -> Option<TokenListId> {
        match self.stream_attr {
            Some(attr) => Some(attr.tokens),
            None => None,
        }
    }

    #[must_use]
    pub const fn is_file(self) -> bool {
        self.file
    }

    #[must_use]
    pub const fn data(self) -> TokenListId {
        self.data.tokens
    }
}

/// One reserved raw-object slot, initialized either now or by `useobjnum`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfRawObjectRecord {
    id: PdfRawObjectId,
    data: Option<PdfRawObjectData>,
    immediate: bool,
    referenced: bool,
}

impl PdfRawObjectRecord {
    #[must_use]
    pub const fn id(self) -> PdfRawObjectId {
        self.id
    }

    #[must_use]
    pub const fn data(self) -> Option<PdfRawObjectData> {
        self.data
    }

    #[must_use]
    pub const fn is_immediate(self) -> bool {
        self.immediate
    }

    #[must_use]
    pub const fn is_referenced(self) -> bool {
        self.referenced
    }
}

#[derive(Clone, Debug)]
struct PdfRawObjectState {
    records: Vec<PdfRawObjectRecord>,
    last_object: u32,
    fingerprint: StateHashFragment,
}

/// Copy-on-write raw-object table shared by PDF snapshots.
#[derive(Clone, Debug)]
pub(crate) struct PdfRawObjects(Arc<PdfRawObjectState>);

impl Default for PdfRawObjects {
    fn default() -> Self {
        Self(Arc::new(PdfRawObjectState {
            records: Vec::new(),
            last_object: 0,
            fingerprint: StateHasher::new(PDF_RAW_OBJECT_DOMAIN).finish_fragment(),
        }))
    }
}

impl PdfRawObjects {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.records.is_empty()
    }

    #[must_use]
    pub(crate) fn fingerprint(&self) -> StateHashFragment {
        self.0.fingerprint
    }

    #[must_use]
    pub(crate) fn last_object(&self) -> u32 {
        self.0.last_object
    }

    #[must_use]
    pub(crate) fn records(&self) -> &[PdfRawObjectRecord] {
        &self.0.records
    }

    #[must_use]
    pub(crate) fn record(&self, id: PdfRawObjectId) -> Option<PdfRawObjectRecord> {
        self.0
            .records
            .binary_search_by_key(&id, |record| record.id)
            .ok()
            .map(|index| self.0.records[index])
    }

    pub(crate) fn reserve(&mut self, id: PdfRawObjectId) {
        let state = Arc::make_mut(&mut self.0);
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
        data: PdfRawObjectData,
        immediate: bool,
    ) -> Result<(), PdfRawObjectInitializeError> {
        let state = Arc::make_mut(&mut self.0);
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
        let state = Arc::make_mut(&mut self.0);
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

fn fingerprint(state: &PdfRawObjectState) -> StateHashFragment {
    let mut hasher = StateHasher::new(PDF_RAW_OBJECT_DOMAIN);
    hasher.u32(state.last_object);
    hasher.usize(state.records.len());
    for record in &state.records {
        hasher.u32(record.id.raw());
        hasher.bool(record.data.is_some());
        if let Some(data) = record.data {
            hasher.bool(data.stream);
            hasher.bool(data.stream_attr.is_some());
            if let Some(attr) = data.stream_attr {
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
