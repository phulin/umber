//! Host-neutral immutable resource protocol for retained canonical execution.

use std::path::Path;
use std::sync::Arc;

use tex_command::{
    FileEnquiryRequest, FileEnquiryResource, FontLoadRequest, FontResource, PdfImageRequest,
    PdfImageResource, RegisteredSourceKind, SourceRegistration,
};
use tex_state::{FileContent, InputOpenState, InputReadState, Universe, WorldError};

use crate::CanonicalResourceNeed;

/// Returns the exact transient capability key for a canonical font request.
///
/// TeX TFM names receive §1257's default `.tfm` extension. Umber's explicit
/// `opentype:` namespace is already a complete typed resource name and must
/// never be rewritten as a TFM path.
#[must_use]
pub fn canonical_font_resource_path(name: &str) -> std::path::PathBuf {
    let mut path = std::path::PathBuf::from(name);
    if !name.starts_with("opentype:") && path.extension().is_none() {
        path.set_extension("tfm");
    }
    path
}

#[derive(Clone, Debug)]
pub enum CanonicalResourceFulfillment {
    Input {
        name: String,
        source: SourceRegistration,
    },
    /// Immutable bytes answering a non-opening pdfTeX file enquiry or
    /// `\openin` probe. This remains distinct from required input backing so
    /// a later opening read can upgrade host dependency accounting.
    InputProbe {
        request: FileEnquiryRequest,
        resource: FileEnquiryResource,
    },
    Font {
        request: FontLoadRequest,
        resource: Box<FontResource>,
    },
    PdfImage {
        request: PdfImageRequest,
        resource: Box<PdfImageResource>,
    },
}

#[derive(Clone, Debug)]
pub enum CanonicalResourceOutcome {
    Fulfilled(CanonicalResourceFulfillment),
    Unavailable,
    Declined,
}

impl CanonicalResourceFulfillment {
    #[must_use]
    pub fn input(name: impl Into<String>, kind: RegisteredSourceKind, bytes: Arc<[u8]>) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::new(kind, bytes),
        }
    }

    #[must_use]
    pub fn world_input(name: impl Into<String>, content: FileContent) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::world(content),
        }
    }

    #[must_use]
    pub fn world_input_probe(request: FileEnquiryRequest, content: FileContent) -> Self {
        Self::InputProbe {
            request,
            resource: FileEnquiryResource::world(content),
        }
    }
}

pub struct CanonicalResourceWorld<'a> {
    stores: &'a mut Universe,
}

impl<'a> CanonicalResourceWorld<'a> {
    #[must_use]
    pub fn new(stores: &'a mut Universe) -> Self {
        Self { stores }
    }

    /// Borrows the candidate's private world for resource bookkeeping.
    pub fn with_input_read_state<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn InputReadState) -> T,
    ) -> T {
        operation(&mut self.stores.input_open_context())
    }

    pub fn read_file(&mut self, path: impl AsRef<Path>) -> Result<FileContent, WorldError> {
        self.stores.world_mut().read_file(path)
    }

    pub fn register_selected_file(
        &mut self,
        path: impl AsRef<Path>,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        self.stores
            .input_open_context()
            .read_supplied_input_file(path.as_ref(), bytes)
    }
}

pub trait CanonicalResourceHost {
    fn fulfill(
        &mut self,
        world: &mut CanonicalResourceWorld<'_>,
        need: &CanonicalResourceNeed,
    ) -> CanonicalResourceOutcome;
}
