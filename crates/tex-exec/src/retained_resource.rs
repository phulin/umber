//! Host-neutral immutable resource protocol for retained canonical execution.

use std::path::Path;
use std::sync::Arc;

use tex_command::{
    FileEnquiryRequest, FileEnquiryResource, FontLoadRequest, FontResource, PdfImageRequest,
    PdfImageResource, RegisteredSourceKind, SourceRegistration, SourceRole,
};
use tex_state::{FileContent, InputReadState, Universe, WorldError};

use crate::ResourceNeed;

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
pub enum ResourceFulfillment {
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
pub enum ResourceOutcome {
    Fulfilled(ResourceFulfillment),
    Unavailable,
    Declined,
}

impl ResourceFulfillment {
    #[must_use]
    pub fn input(name: impl Into<String>, kind: RegisteredSourceKind, bytes: Arc<[u8]>) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::new(kind, bytes),
        }
    }

    #[must_use]
    pub fn input_with_role(
        name: impl Into<String>,
        kind: RegisteredSourceKind,
        bytes: Arc<[u8]>,
        role: SourceRole,
    ) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::new(kind, bytes).with_role(role),
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
    pub fn world_input_with_role(
        name: impl Into<String>,
        content: FileContent,
        role: SourceRole,
    ) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::world(content).with_role(role),
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

trait ResourceWorldBackend {
    fn with_input_read_state(&mut self, operation: &mut dyn FnMut(&mut dyn InputReadState));
    fn read_file(&mut self, path: &Path) -> Result<FileContent, WorldError>;
    fn register_selected_file(
        &mut self,
        path: &Path,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError>;
}

impl<G> ResourceWorldBackend for Universe<G> {
    fn with_input_read_state(&mut self, operation: &mut dyn FnMut(&mut dyn InputReadState)) {
        operation(&mut self.input_open_context());
    }

    fn read_file(&mut self, path: &Path) -> Result<FileContent, WorldError> {
        self.world_mut().read_file(path)
    }

    fn register_selected_file(
        &mut self,
        path: &Path,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        self.input_open_context()
            .read_supplied_input_file(path, bytes)
    }
}

pub struct ResourceWorld<'a> {
    backend: &'a mut dyn ResourceWorldBackend,
}

impl<'a> ResourceWorld<'a> {
    #[must_use]
    pub fn new<G>(stores: &'a mut Universe<G>) -> Self {
        Self { backend: stores }
    }

    /// Borrows the candidate's private world for resource bookkeeping.
    pub fn with_input_read_state<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn InputReadState) -> T,
    ) -> T {
        let mut operation = Some(operation);
        let mut result = None;
        self.backend.with_input_read_state(&mut |input| {
            result = Some(operation
                .take()
                .expect("resource input operation runs once")(
                input
            ));
        });
        result.expect("resource input operation ran")
    }

    pub fn read_file(&mut self, path: impl AsRef<Path>) -> Result<FileContent, WorldError> {
        self.backend.read_file(path.as_ref())
    }

    pub fn register_selected_file(
        &mut self,
        path: impl AsRef<Path>,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        self.backend.register_selected_file(path.as_ref(), bytes)
    }
}

pub trait ResourceHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome;
}
