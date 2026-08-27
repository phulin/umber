//! Checkpointed raw fragments for PDF document dictionaries and trailer data.

use crate::state_hash::{StateHashFragment, StateHasher};

use super::{PdfRows, PdfTokenParameter};

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
    pages: Option<u32>,
    names: Option<u32>,
    catalog: Option<u32>,
    info: Option<u32>,
}

impl PdfDocumentObjectIds {
    #[must_use]
    pub const fn pages(self) -> Option<u32> {
        self.pages
    }

    #[must_use]
    pub const fn names(self) -> Option<u32> {
        self.names
    }

    #[must_use]
    pub const fn catalog(self) -> Option<u32> {
        self.catalog
    }

    #[must_use]
    pub const fn info(self) -> Option<u32> {
        self.info
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

#[derive(Debug, Eq, Hash, PartialEq)]
struct PdfDocumentFragment<G> {
    kind: PdfDocumentFragmentKind,
    value: PdfTokenParameter<G>,
}

impl<G> Clone for PdfDocumentFragment<G> {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind,
            value: self.value.clone(),
        }
    }
}

/// Owned document fragments copied into explicit PDF checkpoints.
#[derive(Debug)]
pub(crate) struct PdfDocumentFragments<G> {
    fragments: PdfRows<PdfDocumentFragment<G>>,
    fingerprint: StateHashFragment,
}

impl<G> Default for PdfDocumentFragments<G> {
    fn default() -> Self {
        let fragments = PdfRows::default();
        Self {
            fingerprint: fingerprint(&fragments),
            fragments,
        }
    }
}

impl<G> PdfDocumentFragments<G> {
    pub(crate) fn begin_transaction(&mut self, len: usize) {
        self.fragments.begin_transaction(len);
    }

    pub(crate) fn reject_transaction(&mut self) {
        self.fragments.reject_transaction();
    }

    pub(crate) fn accept_transaction(&mut self) {
        self.fragments.accept_transaction();
    }

    pub(crate) fn len(&self) -> usize {
        self.fragments.len()
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        self.fragments.truncate(len);
        self.fingerprint = fingerprint(&self.fragments);
    }
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    #[must_use]
    pub(crate) const fn fingerprint(&self) -> StateHashFragment {
        self.fingerprint
    }

    pub(crate) fn set_fingerprint(&mut self, fingerprint: StateHashFragment) {
        self.fingerprint = fingerprint;
    }

    pub(crate) fn append(&mut self, kind: PdfDocumentFragmentKind, value: PdfTokenParameter<G>) {
        self.fragments.push(PdfDocumentFragment { kind, value });
        self.fingerprint = fingerprint(&self.fragments);
    }

    pub(crate) fn values(
        &self,
        kind: PdfDocumentFragmentKind,
    ) -> impl Iterator<Item = crate::TokenListId<G>> + '_ {
        self.fragments
            .iter()
            .filter(move |fragment| fragment.kind == kind)
            .map(|fragment| fragment.value.id())
    }
}

fn fingerprint<G>(fragments: &PdfRows<PdfDocumentFragment<G>>) -> StateHashFragment {
    let mut hasher = StateHasher::new(PDF_DOCUMENT_FRAGMENTS_DOMAIN);
    hasher.usize(fragments.len());
    for fragment in fragments.iter() {
        hasher.u8(fragment.kind.tag());
        hasher.bytes(&fragment.value.semantic_id.bytes());
    }
    hasher.finish_fragment()
}
