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

impl<G> Clone for PdfOutlineRecord<G> {
    fn clone(&self) -> Self {
        Self {
            action_object: self.action_object,
            item_object: self.item_object,
            title_object: self.title_object,
            attributes: self.attributes.clone(),
            action: self.action.clone(),
            count: self.count,
            title: self.title.clone(),
        }
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
    pub fn attributes(&self) -> TokenListId<G> {
        self.attributes.clone()
    }
    #[must_use]
    pub fn action(&self) -> PdfActionSpec<G> {
        self.action.clone()
    }
    #[must_use]
    pub const fn count(&self) -> i32 {
        self.count
    }
    #[must_use]
    pub fn title(&self) -> TokenListId<G> {
        self.title.clone()
    }
}
