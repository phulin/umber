//! Checkpointed annotation and logical-link records.

use super::PdfActionSpec;
use crate::durable_arena::TokenListId;
use crate::scaled::Scaled;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PdfAnnotationDimensions {
    pub width: Option<Scaled>,
    pub height: Option<Scaled>,
    pub depth: Option<Scaled>,
}

impl PdfAnnotationDimensions {
    pub const RUNNING: Self = Self {
        width: None,
        height: None,
        depth: None,
    };
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfAnnotationData<G> {
    pub dimensions: PdfAnnotationDimensions,
    pub entries: TokenListId<G>,
}

impl<G> Copy for PdfAnnotationData<G> {}

impl<G> Clone for PdfAnnotationData<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfAnnotationRecord<G> {
    object: u32,
    data: Option<PdfAnnotationData<G>>,
}

impl<G> Copy for PdfAnnotationRecord<G> {}

impl<G> Clone for PdfAnnotationRecord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> PdfAnnotationRecord<G> {
    pub(super) const fn reserved(object: u32) -> Self {
        Self { object, data: None }
    }
    #[must_use]
    pub const fn object(&self) -> u32 {
        self.object
    }
    #[must_use]
    pub const fn data(&self) -> Option<PdfAnnotationData<G>> {
        self.data
    }
    pub(super) fn initialize(&mut self, data: PdfAnnotationData<G>) -> Result<(), ()> {
        if self.data.is_some() {
            return Err(());
        }
        self.data = Some(data);
        Ok(())
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfLinkRecord<G> {
    object: u32,
    dimensions: PdfAnnotationDimensions,
    attributes: TokenListId<G>,
    action: PdfActionSpec<G>,
}

impl<G> Copy for PdfLinkRecord<G> {}

impl<G> Clone for PdfLinkRecord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PdfOpenLink<G> {
    pub record: PdfLinkRecord<G>,
    pub nesting_depth: u32,
}

impl<G> Copy for PdfOpenLink<G> {}

impl<G> Clone for PdfOpenLink<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> PdfLinkRecord<G> {
    pub(super) const fn new(
        object: u32,
        dimensions: PdfAnnotationDimensions,
        attributes: TokenListId<G>,
        action: PdfActionSpec<G>,
    ) -> Self {
        Self {
            object,
            dimensions,
            attributes,
            action,
        }
    }
    #[must_use]
    pub const fn object(&self) -> u32 {
        self.object
    }
    #[must_use]
    pub const fn dimensions(&self) -> PdfAnnotationDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn attributes(&self) -> TokenListId<G> {
        self.attributes
    }
    #[must_use]
    pub const fn action(&self) -> PdfActionSpec<G> {
        self.action
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfAnnotationInitializeError(pub u32);

impl core::fmt::Display for PdfAnnotationInitializeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "PDF annotation object {} is unavailable", self.0)
    }
}

impl std::error::Error for PdfAnnotationInitializeError {}
