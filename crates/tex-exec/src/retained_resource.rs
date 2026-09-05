//! Host-neutral immutable resource protocol for retained canonical execution.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tex_command::{
    FileEnquiryRequest, FileEnquiryResource, FontLoadRequest, FontResource, PdfImageRequest,
    PdfImageResource, RegisteredSourceKind, SourceRegistration, SourceRole,
};
use tex_state::{
    FileContent, InputDependency, InputDependencyAccess, InputDependencyOutcome, InputReadState,
    SharedBytes, Universe, WorldError,
};

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

/// An owned, actionable failure while resolving a resource.
///
/// This is deliberately separate from [`ResourceOutcome::Declined`]. A
/// declined request is still pending and may be fulfilled by a later host
/// round; a failure is final for the current drive and must be surfaced to
/// the caller without being replayed or recorded as an absence. World-backed
/// failures retain the shared typed error, including its I/O classification
/// and path, while host adapters that cannot expose a typed error can retain
/// their rendered cause.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceFailure {
    World(WorldError),
    Message(String),
}

impl ResourceFailure {
    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    #[must_use]
    pub fn world(error: WorldError) -> Self {
        Self::World(error)
    }

    #[must_use]
    pub fn as_world_error(&self) -> Option<&WorldError> {
        match self {
            Self::World(error) => Some(error),
            Self::Message(_) => None,
        }
    }
}

impl From<WorldError> for ResourceFailure {
    fn from(error: WorldError) -> Self {
        Self::World(error)
    }
}

impl fmt::Display for ResourceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(error) => error.fmt(formatter),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ResourceFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::World(error) => Some(error),
            Self::Message(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ResourceOutcome {
    /// Immutable bytes or metadata are ready for replay into the engine.
    Fulfilled(ResourceFulfillment),
    /// The host authoritatively knows that this request is absent.
    Unavailable,
    /// The request is still pending and may be fulfilled by a later host round.
    Declined,
    /// Resolving the request failed; the cause must be surfaced immediately.
    Failed(ResourceFailure),
}

/// Semantic bookkeeping performed while a host resolves one immutable
/// resource. The host answer itself is cached outside rollback, while these
/// effects are replayed into the restored world on every full restart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceReplayEffect {
    InputDependency {
        path: PathBuf,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    },
}

impl ResourceReplayEffect {
    /// Detaches the semantic input fact carried by this retained answer so it
    /// can be installed in a capability binding and re-recorded on a cached
    /// hit after rollback.
    #[must_use]
    pub fn input_dependency(&self) -> InputDependency {
        match self {
            Self::InputDependency {
                path,
                outcome,
                access,
            } => InputDependency::new(path.clone(), *outcome, *access),
        }
    }
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
            .read_supplied_input_file(path, bytes.into())
    }
}

pub struct ResourceWorld<'a> {
    backend: &'a mut dyn ResourceWorldBackend,
    replay_effects: Vec<ResourceReplayEffect>,
}

impl<'a> ResourceWorld<'a> {
    #[must_use]
    pub fn new<G>(stores: &'a mut Universe<G>) -> Self {
        Self {
            backend: stores,
            replay_effects: Vec::new(),
        }
    }

    /// Borrows the candidate's private world for resource bookkeeping.
    pub fn with_input_read_state<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn InputReadState) -> T,
    ) -> T {
        let mut operation = Some(operation);
        let mut result = None;
        let replay_effects = &mut self.replay_effects;
        self.backend.with_input_read_state(&mut |input| {
            let mut recording = RecordingInputReadState {
                input,
                replay_effects,
            };
            result = Some(operation
                .take()
                .expect("resource input operation runs once")(
                &mut recording
            ));
        });
        result.expect("resource input operation ran")
    }

    /// Takes semantic effects recorded while the host answered this request.
    /// The resource host owns the returned list until the answer is either
    /// applied or discarded by its surrounding transaction.
    pub fn take_replay_effects(&mut self) -> Vec<ResourceReplayEffect> {
        std::mem::take(&mut self.replay_effects)
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

struct RecordingInputReadState<'a> {
    input: &'a mut dyn InputReadState,
    replay_effects: &'a mut Vec<ResourceReplayEffect>,
}

impl InputReadState for RecordingInputReadState<'_> {
    fn read_input_file(&mut self, path: &Path) -> Result<FileContent, WorldError> {
        self.input.read_input_file(path)
    }

    fn read_pending_output_file(&mut self, path: &Path) -> Result<Option<FileContent>, WorldError> {
        self.input.read_pending_output_file(path)
    }

    fn read_supplied_input_file(
        &mut self,
        path: &Path,
        bytes: SharedBytes,
    ) -> Result<FileContent, WorldError> {
        self.input.read_supplied_input_file(path, bytes)
    }

    fn record_input_dependency(
        &mut self,
        path: &Path,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    ) -> Result<(), WorldError> {
        self.input.record_input_dependency(path, outcome, access)?;
        self.replay_effects
            .push(ResourceReplayEffect::InputDependency {
                path: path.to_owned(),
                outcome,
                access,
            });
        Ok(())
    }
}

pub trait ResourceHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome;
}
