//! Typed PDF action specifications shared by catalog, link, and outline users.

use crate::ids::TokenListId;
use crate::state_hash::{StateHashFragment, StateHasher};

const PDF_ACTION_DOMAIN: u64 = 0x7064_665f_6163_746e;

/// A positive numeric identifier or an expanded PDF string token list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfActionIdentifier {
    Name(TokenListId),
    Number(u32),
    Raw(TokenListId),
}

/// The destination selected by a GoTo or Thread action.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfActionTarget {
    Page { number: u32, view: TokenListId },
    Destination(PdfActionIdentifier),
}

/// pdfTeX's tri-state remote-window preference.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfActionWindow {
    Unspecified,
    New,
    Same,
}

/// One fully scanned non-user PDF action.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfActionDestination {
    pub file: Option<TokenListId>,
    pub structure: Option<PdfActionIdentifier>,
    pub target: PdfActionTarget,
    pub window: PdfActionWindow,
}

/// Shared engine-side representation of pdfTeX's action specification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfActionSpec {
    User(TokenListId),
    GoTo(PdfActionDestination),
    Thread(PdfActionDestination),
}

impl PdfActionSpec {
    #[must_use]
    pub(crate) const fn needs_target_object(self) -> bool {
        matches!(
            self,
            Self::GoTo(PdfActionDestination { file: None, .. })
                | Self::Thread(PdfActionDestination { file: None, .. })
        )
    }

    #[must_use]
    pub(crate) const fn needs_structure_object(self) -> bool {
        matches!(
            self,
            Self::GoTo(PdfActionDestination {
                file: None,
                structure: Some(_),
                ..
            })
        )
    }

    pub(crate) fn fingerprint(
        self,
        mut semantic_id: impl FnMut(TokenListId) -> StateHashFragment,
    ) -> StateHashFragment {
        let mut hasher = StateHasher::new(PDF_ACTION_DOMAIN);
        match self {
            Self::User(tokens) => {
                hasher.u8(0);
                hasher.bytes(&semantic_id(tokens).bytes());
            }
            Self::GoTo(action) => {
                hasher.u8(1);
                hash_destination(action, &mut hasher, &mut semantic_id);
            }
            Self::Thread(action) => {
                hasher.u8(2);
                hash_destination(action, &mut hasher, &mut semantic_id);
            }
        }
        hasher.finish_fragment()
    }
}

fn hash_destination(
    action: PdfActionDestination,
    hasher: &mut StateHasher,
    semantic_id: &mut impl FnMut(TokenListId) -> StateHashFragment,
) {
    hash_optional_tokens(action.file, hasher, semantic_id);
    hasher.bool(action.structure.is_some());
    if let Some(identifier) = action.structure {
        hash_identifier(identifier, hasher, semantic_id);
    }
    match action.target {
        PdfActionTarget::Page { number, view } => {
            hasher.u8(0);
            hasher.u32(number);
            hasher.bytes(&semantic_id(view).bytes());
        }
        PdfActionTarget::Destination(identifier) => {
            hasher.u8(1);
            hash_identifier(identifier, hasher, semantic_id);
        }
    }
    hasher.u8(match action.window {
        PdfActionWindow::Unspecified => 0,
        PdfActionWindow::New => 1,
        PdfActionWindow::Same => 2,
    });
}

fn hash_identifier(
    identifier: PdfActionIdentifier,
    hasher: &mut StateHasher,
    semantic_id: &mut impl FnMut(TokenListId) -> StateHashFragment,
) {
    match identifier {
        PdfActionIdentifier::Name(tokens) => {
            hasher.u8(0);
            hasher.bytes(&semantic_id(tokens).bytes());
        }
        PdfActionIdentifier::Number(number) => {
            hasher.u8(1);
            hasher.u32(number);
        }
        PdfActionIdentifier::Raw(tokens) => {
            hasher.u8(2);
            hasher.bytes(&semantic_id(tokens).bytes());
        }
    }
}

fn hash_optional_tokens(
    tokens: Option<TokenListId>,
    hasher: &mut StateHasher,
    semantic_id: &mut impl FnMut(TokenListId) -> StateHashFragment,
) {
    hasher.bool(tokens.is_some());
    if let Some(tokens) = tokens {
        hasher.bytes(&semantic_id(tokens).bytes());
    }
}

/// The indirect action object retained by the catalog.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfActionRecord {
    id: u32,
    spec: PdfActionSpec,
    target_object: Option<u32>,
    structure_object: Option<u32>,
}

impl PdfActionRecord {
    pub(crate) const fn new(
        id: u32,
        spec: PdfActionSpec,
        target_object: Option<u32>,
        structure_object: Option<u32>,
    ) -> Self {
        Self {
            id,
            spec,
            target_object,
            structure_object,
        }
    }

    #[must_use]
    pub const fn id(self) -> u32 {
        self.id
    }

    #[must_use]
    pub const fn spec(self) -> PdfActionSpec {
        self.spec
    }

    #[must_use]
    pub const fn target_object(self) -> Option<u32> {
        self.target_object
    }

    #[must_use]
    pub const fn structure_object(self) -> Option<u32> {
        self.structure_object
    }
}
