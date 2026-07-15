//! Checkpointed annotation and logical-link object records.

use crate::ids::TokenListId;
use crate::scaled::Scaled;

use super::PdfActionSpec;

/// pdfTeX rule dimensions; `None` retains the running sentinel until shipout.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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

/// Initialized contents of one general `\pdfannot` object.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfAnnotationData {
    pub dimensions: PdfAnnotationDimensions,
    pub entries: TokenListId,
}

/// One annotation reservation. `data == None` is `reserveobjnum` state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfAnnotationRecord {
    object: u32,
    data: Option<PdfAnnotationData>,
}

impl PdfAnnotationRecord {
    pub(super) const fn reserved(object: u32) -> Self {
        Self { object, data: None }
    }

    #[must_use]
    pub const fn object(self) -> u32 {
        self.object
    }

    #[must_use]
    pub const fn data(self) -> Option<PdfAnnotationData> {
        self.data
    }

    pub(super) fn initialize(&mut self, data: PdfAnnotationData) -> Result<(), ()> {
        if self.data.is_some() {
            return Err(());
        }
        self.data = Some(data);
        Ok(())
    }
}

/// One logical link created by `\pdfstartlink`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfLinkRecord {
    object: u32,
    dimensions: PdfAnnotationDimensions,
    attributes: TokenListId,
    action: PdfActionSpec,
}

/// One currently open logical link and the mode-nest depth where it started.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfOpenLink {
    pub record: PdfLinkRecord,
    pub nesting_depth: u32,
}

impl PdfLinkRecord {
    pub(super) const fn new(
        object: u32,
        dimensions: PdfAnnotationDimensions,
        attributes: TokenListId,
        action: PdfActionSpec,
    ) -> Self {
        Self {
            object,
            dimensions,
            attributes,
            action,
        }
    }

    #[must_use]
    pub const fn object(self) -> u32 {
        self.object
    }

    #[must_use]
    pub const fn dimensions(self) -> PdfAnnotationDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn attributes(self) -> TokenListId {
        self.attributes
    }

    #[must_use]
    pub const fn action(self) -> PdfActionSpec {
        self.action
    }
}

/// A `useobjnum` target is absent, already initialized, or not an annotation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfAnnotationInitializeError(pub u32);

impl std::fmt::Display for PdfAnnotationInitializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PDF annotation object {} is unavailable", self.0)
    }
}

impl std::error::Error for PdfAnnotationInitializeError {}
