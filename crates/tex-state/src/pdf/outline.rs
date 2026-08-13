use crate::token_store::TokenListRef;

use super::PdfActionSpec;

/// One immediately allocated pdfTeX outline entry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfOutlineRecord {
    action_object: u32,
    item_object: u32,
    title_object: u32,
    attributes: TokenListRef,
    action: PdfActionSpec,
    count: i32,
    title: TokenListRef,
}

impl PdfOutlineRecord {
    pub(super) fn new(
        action_object: u32,
        item_object: u32,
        title_object: u32,
        attributes: TokenListRef,
        action: PdfActionSpec,
        count: i32,
        title: TokenListRef,
    ) -> Self {
        Self {
            action_object,
            item_object,
            title_object,
            attributes,
            action,
            count,
            title,
        }
    }

    #[must_use]
    pub const fn action_object(&self) -> u32 {
        self.action_object
    }
    #[must_use]
    pub const fn item_object(&self) -> u32 {
        self.item_object
    }
    #[must_use]
    pub const fn title_object(&self) -> u32 {
        self.title_object
    }
    #[must_use]
    pub fn attributes(&self) -> crate::ids::TokenListId {
        self.attributes.id()
    }
    #[must_use]
    pub fn action(&self) -> PdfActionSpec {
        self.action.clone()
    }
    #[must_use]
    pub const fn count(&self) -> i32 {
        self.count
    }
    #[must_use]
    pub fn title(&self) -> crate::ids::TokenListId {
        self.title.id()
    }
}
