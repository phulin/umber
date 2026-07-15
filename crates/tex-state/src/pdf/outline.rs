use crate::ids::TokenListId;

use super::PdfActionSpec;

/// One immediately allocated pdfTeX outline entry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfOutlineRecord {
    action_object: u32,
    item_object: u32,
    title_object: u32,
    attributes: TokenListId,
    action: PdfActionSpec,
    count: i32,
    title: TokenListId,
}

impl PdfOutlineRecord {
    pub(super) const fn new(
        action_object: u32,
        item_object: u32,
        title_object: u32,
        attributes: TokenListId,
        action: PdfActionSpec,
        count: i32,
        title: TokenListId,
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
    pub const fn action_object(self) -> u32 {
        self.action_object
    }
    #[must_use]
    pub const fn item_object(self) -> u32 {
        self.item_object
    }
    #[must_use]
    pub const fn title_object(self) -> u32 {
        self.title_object
    }
    #[must_use]
    pub const fn attributes(self) -> TokenListId {
        self.attributes
    }
    #[must_use]
    pub const fn action(self) -> PdfActionSpec {
        self.action
    }
    #[must_use]
    pub const fn count(self) -> i32 {
        self.count
    }
    #[must_use]
    pub const fn title(self) -> TokenListId {
        self.title
    }
}
