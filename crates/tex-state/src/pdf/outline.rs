use super::PdfActionSpec;
use crate::durable_arena::TokenListId;

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfOutlineRecord<G> {
    action_object: u32,
    item_object: u32,
    title_object: u32,
    attributes: TokenListId<G>,
    action: PdfActionSpec<G>,
    count: i32,
    title: TokenListId<G>,
}

impl<G> Copy for PdfOutlineRecord<G> {}

impl<G> Clone for PdfOutlineRecord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> PdfOutlineRecord<G> {
    pub(super) const fn new(
        action_object: u32,
        item_object: u32,
        title_object: u32,
        attributes: TokenListId<G>,
        action: PdfActionSpec<G>,
        count: i32,
        title: TokenListId<G>,
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
    pub const fn attributes(&self) -> TokenListId<G> {
        self.attributes
    }
    #[must_use]
    pub const fn action(&self) -> PdfActionSpec<G> {
        self.action
    }
    #[must_use]
    pub const fn count(&self) -> i32 {
        self.count
    }
    #[must_use]
    pub const fn title(&self) -> TokenListId<G> {
        self.title
    }
}
