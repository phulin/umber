//! Generation-typed PDF action specifications.

use crate::durable_arena::TokenListId;
use crate::state_hash::{StateHashFragment, StateHasher};

const PDF_ACTION_DOMAIN: u64 = 0x7064_665f_6163_746e;

#[derive(Debug, Eq, Hash, PartialEq)]
pub enum PdfActionIdentifier<G> {
    Name(TokenListId<G>),
    Number(u32),
    Raw(TokenListId<G>),
}

impl<G> Copy for PdfActionIdentifier<G> {}

impl<G> Clone for PdfActionIdentifier<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub enum PdfActionTarget<G> {
    Page { number: u32, view: TokenListId<G> },
    Destination(PdfActionIdentifier<G>),
}

impl<G> Copy for PdfActionTarget<G> {}

impl<G> Clone for PdfActionTarget<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfActionWindow {
    Unspecified,
    New,
    Same,
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfActionDestination<G> {
    pub file: Option<TokenListId<G>>,
    pub structure: Option<PdfActionIdentifier<G>>,
    pub target: PdfActionTarget<G>,
    pub window: PdfActionWindow,
}

impl<G> Copy for PdfActionDestination<G> {}

impl<G> Clone for PdfActionDestination<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub enum PdfActionSpec<G> {
    User(TokenListId<G>),
    GoTo(PdfActionDestination<G>),
    Thread(PdfActionDestination<G>),
}

impl<G> Copy for PdfActionSpec<G> {}

impl<G> Clone for PdfActionSpec<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> PdfActionSpec<G> {
    #[must_use]
    pub(crate) fn needs_target_object(&self) -> bool {
        matches!(
            self,
            Self::GoTo(PdfActionDestination { file: None, .. })
                | Self::Thread(PdfActionDestination { file: None, .. })
        )
    }

    #[must_use]
    pub(crate) fn needs_structure_object(&self) -> bool {
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
        &self,
        mut semantic_id: impl FnMut(TokenListId<G>) -> StateHashFragment,
    ) -> StateHashFragment {
        let mut hasher = StateHasher::new(PDF_ACTION_DOMAIN);
        match self {
            Self::User(tokens) => {
                hasher.u8(0);
                hash_tokens(*tokens, &mut hasher, &mut semantic_id);
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

fn hash_destination<G>(
    action: &PdfActionDestination<G>,
    hasher: &mut StateHasher,
    semantic_id: &mut impl FnMut(TokenListId<G>) -> StateHashFragment,
) {
    hasher.bool(action.file.is_some());
    if let Some(tokens) = action.file {
        hash_tokens(tokens, hasher, semantic_id);
    }
    hasher.bool(action.structure.is_some());
    if let Some(identifier) = action.structure {
        hash_identifier(identifier, hasher, semantic_id);
    }
    match action.target {
        PdfActionTarget::Page { number, view } => {
            hasher.u8(0);
            hasher.u32(number);
            hash_tokens(view, hasher, semantic_id);
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

fn hash_identifier<G>(
    identifier: PdfActionIdentifier<G>,
    hasher: &mut StateHasher,
    semantic_id: &mut impl FnMut(TokenListId<G>) -> StateHashFragment,
) {
    match identifier {
        PdfActionIdentifier::Name(tokens) => {
            hasher.u8(0);
            hash_tokens(tokens, hasher, semantic_id);
        }
        PdfActionIdentifier::Number(number) => {
            hasher.u8(1);
            hasher.u32(number);
        }
        PdfActionIdentifier::Raw(tokens) => {
            hasher.u8(2);
            hash_tokens(tokens, hasher, semantic_id);
        }
    }
}

fn hash_tokens<G>(
    tokens: TokenListId<G>,
    hasher: &mut StateHasher,
    semantic_id: &mut impl FnMut(TokenListId<G>) -> StateHashFragment,
) {
    hasher.bytes(&semantic_id(tokens).bytes());
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfActionRecord<G> {
    id: u32,
    spec: PdfActionSpec<G>,
    target_object: Option<u32>,
    structure_object: Option<u32>,
}

impl<G> Copy for PdfActionRecord<G> {}

impl<G> Clone for PdfActionRecord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> PdfActionRecord<G> {
    pub(crate) fn new(
        id: u32,
        spec: PdfActionSpec<G>,
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
    pub const fn id(&self) -> u32 {
        self.id
    }
    #[must_use]
    pub const fn spec(&self) -> PdfActionSpec<G> {
        self.spec
    }
    #[must_use]
    pub const fn target_object(&self) -> Option<u32> {
        self.target_object
    }
    #[must_use]
    pub const fn structure_object(&self) -> Option<u32> {
        self.structure_object
    }
}
