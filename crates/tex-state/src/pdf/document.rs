//! Checkpointed raw fragments for PDF document dictionaries and trailer data.

use std::sync::Arc;

use crate::ids::TokenListId;
use crate::state_hash::StateHasher;

use super::PdfTokenParameter;

const PDF_DOCUMENT_FRAGMENTS_DOMAIN: u64 = 0x7064_665f_646f_6366;

/// A pdfTeX document-level token-list destination.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfDocumentFragmentKind {
    Info,
    Catalog,
    Names,
    Trailer,
    TrailerId,
}

/// Canonical ledger identities allocated for final document dictionaries.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PdfDocumentObjectIds {
    names: Option<u32>,
    info: Option<u32>,
}

impl PdfDocumentObjectIds {
    #[must_use]
    pub const fn names(self) -> Option<u32> {
        self.names
    }

    #[must_use]
    pub const fn info(self) -> Option<u32> {
        self.info
    }

    pub(crate) fn set_names(&mut self, id: u32) {
        self.names = Some(id);
    }

    pub(crate) fn set_info(&mut self, id: u32) {
        self.info = Some(id);
    }
}

impl PdfDocumentFragmentKind {
    const fn tag(self) -> u8 {
        match self {
            Self::Info => 0,
            Self::Catalog => 1,
            Self::Names => 2,
            Self::Trailer => 3,
            Self::TrailerId => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PdfDocumentFragment {
    kind: PdfDocumentFragmentKind,
    value: PdfTokenParameter,
}

/// Copy-on-write document fragments shared by PDF snapshots.
#[derive(Clone, Debug)]
pub(crate) struct PdfDocumentFragments {
    fragments: Arc<Vec<PdfDocumentFragment>>,
    fingerprint: u64,
}

impl Default for PdfDocumentFragments {
    fn default() -> Self {
        Self {
            fragments: Arc::new(Vec::new()),
            fingerprint: StateHasher::new(PDF_DOCUMENT_FRAGMENTS_DOMAIN).finish(),
        }
    }
}

impl PdfDocumentFragments {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    #[must_use]
    pub(crate) const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    pub(crate) fn append(&mut self, kind: PdfDocumentFragmentKind, value: PdfTokenParameter) {
        Arc::make_mut(&mut self.fragments).push(PdfDocumentFragment { kind, value });
        self.fingerprint = fingerprint(&self.fragments);
    }

    pub(crate) fn values(
        &self,
        kind: PdfDocumentFragmentKind,
    ) -> impl Iterator<Item = TokenListId> + '_ {
        self.fragments
            .iter()
            .filter(move |fragment| fragment.kind == kind)
            .map(|fragment| fragment.value.tokens)
    }
}

fn fingerprint(fragments: &[PdfDocumentFragment]) -> u64 {
    let mut hasher = StateHasher::new(PDF_DOCUMENT_FRAGMENTS_DOMAIN);
    hasher.usize(fragments.len());
    for fragment in fragments {
        hasher.u8(fragment.kind.tag());
        hasher.u64(fragment.value.semantic_id);
    }
    hasher.finish()
}
