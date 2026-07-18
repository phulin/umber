use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use crate::{
    BuildPlan, BuildTransaction, FileContentId, FileOrigin, ImmutableBindingError, LayerKind,
    LayeredFileStorage, VfsLimitError, VfsLimitKind, VfsLimits, VfsSnapshot, VirtualFile,
    VirtualPath,
};

#[cfg(test)]
mod tests;

/// Semantic subsystem that issued a logical file request.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ResourceDomain {
    Tex = 1,
    Bibliography = 2,
    Generic = 3,
}

impl ResourceDomain {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Tex => "tex",
            Self::Bibliography => "bibliography",
            Self::Generic => "generic",
        }
    }

    #[must_use]
    pub fn from_wire_name(value: &str) -> Option<Self> {
        match value {
            "tex" => Some(Self::Tex),
            "bibliography" => Some(Self::Bibliography),
            "generic" => Some(Self::Generic),
            _ => None,
        }
    }
}

/// Semantic kind of a host-provisioned immutable file.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum FileKind {
    TexInput = 1,
    Tfm = 2,
    FormatImage = 3,
    BibControl = 4,
    BibData = 5,
    BibConfiguration = 6,
    XmlSchema = 7,
    GenericAsset = 8,
    Image = 9,
    /// A root or recursively included classic BibTeX auxiliary file.
    BibAux = 10,
    /// A classic BibTeX datasource, kept distinct from Biber's input boundary.
    ClassicBibData = 11,
    /// An executable classic BibTeX style program.
    BibStyle = 12,
}

impl FileKind {
    #[must_use]
    pub const fn domain(self) -> ResourceDomain {
        match self {
            Self::TexInput | Self::Tfm | Self::FormatImage | Self::Image => ResourceDomain::Tex,
            Self::BibControl
            | Self::BibData
            | Self::BibConfiguration
            | Self::XmlSchema
            | Self::BibAux
            | Self::ClassicBibData
            | Self::BibStyle => ResourceDomain::Bibliography,
            Self::GenericAsset => ResourceDomain::Generic,
        }
    }

    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::TexInput => "tex",
            Self::Tfm => "tfm",
            Self::FormatImage => "format",
            Self::BibControl => "bib-control",
            Self::BibData => "bib-data",
            Self::BibConfiguration => "bib-configuration",
            Self::XmlSchema => "xml-schema",
            Self::GenericAsset => "asset",
            Self::Image => "image",
            Self::BibAux => "bib-aux",
            Self::ClassicBibData => "classic-bib-data",
            Self::BibStyle => "bib-style",
        }
    }

    #[must_use]
    pub fn from_wire_name(value: &str) -> Option<Self> {
        match value {
            "tex" => Some(Self::TexInput),
            "tfm" => Some(Self::Tfm),
            "format" => Some(Self::FormatImage),
            "bib-control" => Some(Self::BibControl),
            "bib-data" => Some(Self::BibData),
            "bib-configuration" => Some(Self::BibConfiguration),
            "xml-schema" => Some(Self::XmlSchema),
            "asset" => Some(Self::GenericAsset),
            "image" => Some(Self::Image),
            "bib-aux" => Some(Self::BibAux),
            "classic-bib-data" => Some(Self::ClassicBibData),
            "bib-style" => Some(Self::BibStyle),
            _ => None,
        }
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TexInput => "TeX input",
            Self::Tfm => "TFM",
            Self::FormatImage => "format image",
            Self::BibControl => "bibliography control",
            Self::BibData => "bibliography data",
            Self::BibConfiguration => "bibliography configuration",
            Self::XmlSchema => "XML schema",
            Self::GenericAsset => "generic asset",
            Self::Image => "image",
            Self::BibAux => "classic bibliography auxiliary",
            Self::ClassicBibData => "classic bibliography data",
            Self::BibStyle => "classic bibliography style",
        })
    }
}

/// Complete typed identity of one logical file request.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileRequestKey {
    domain: ResourceDomain,
    kind: FileKind,
    normalized_name: String,
}

impl FileRequestKey {
    /// Constructs a key in the natural domain for `kind`.
    pub fn new(kind: FileKind, name: &str) -> Result<Self, RequestKeyError> {
        Self::for_domain(kind.domain(), kind, name)
    }

    /// Constructs a domain-qualified key, rejecting cross-domain kinds.
    pub fn for_domain(
        domain: ResourceDomain,
        kind: FileKind,
        name: &str,
    ) -> Result<Self, RequestKeyError> {
        if domain != kind.domain() {
            return Err(RequestKeyError::KindMismatch { domain, kind });
        }
        if name.starts_with('/') {
            return Err(RequestKeyError::InvalidName {
                name: name.to_owned(),
                message: "resource request names must be relative",
            });
        }
        let path = VirtualPath::user(name).map_err(|error| RequestKeyError::InvalidName {
            name: name.to_owned(),
            message: error.message(),
        })?;
        Ok(Self {
            domain,
            kind,
            normalized_name: path
                .as_str()
                .strip_prefix("/job/")
                .expect("user paths have the /job root")
                .to_owned(),
        })
    }

    #[must_use]
    pub const fn domain(&self) -> ResourceDomain {
        self.domain
    }

    #[must_use]
    pub const fn kind(&self) -> FileKind {
        self.kind
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.normalized_name
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestKeyError {
    InvalidName {
        name: String,
        message: &'static str,
    },
    KindMismatch {
        domain: ResourceDomain,
        kind: FileKind,
    },
}

impl fmt::Display for RequestKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName { message, .. } => f.write_str(message),
            Self::KindMismatch { domain, kind } => {
                write!(f, "file kind {kind} does not belong to {domain:?}")
            }
        }
    }
}

impl std::error::Error for RequestKeyError {}

/// One logical request plus its spelling at the requesting subsystem boundary.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FileRequest {
    key: FileRequestKey,
    original_name: String,
}

impl FileRequest {
    #[must_use]
    pub fn new(key: FileRequestKey, original_name: impl Into<String>) -> Self {
        Self {
            key,
            original_name: original_name.into(),
        }
    }

    #[must_use]
    pub const fn key(&self) -> &FileRequestKey {
        &self.key
    }

    #[must_use]
    pub fn original_name(&self) -> &str {
        &self.original_name
    }
}

/// A deterministically ordered, deduplicated file-only request batch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileRequestBatch {
    pub required: Vec<FileRequest>,
    pub probes: Vec<FileRequest>,
    pub prefetch_hints: Vec<FileRequest>,
}

impl FileRequestBatch {
    #[must_use]
    pub fn new(
        required: impl IntoIterator<Item = FileRequest>,
        prefetch_hints: impl IntoIterator<Item = FileRequest>,
    ) -> Self {
        Self::with_probes(required, [], prefetch_hints)
    }

    #[must_use]
    pub fn with_probes(
        required: impl IntoIterator<Item = FileRequest>,
        probes: impl IntoIterator<Item = FileRequest>,
        prefetch_hints: impl IntoIterator<Item = FileRequest>,
    ) -> Self {
        let required = canonical_requests(required);
        let required_keys = required
            .iter()
            .map(|request| request.key.clone())
            .collect::<BTreeSet<_>>();
        let probes = canonical_requests(probes)
            .into_iter()
            .filter(|request| !required_keys.contains(request.key()))
            .collect::<Vec<_>>();
        let blocking_keys = required
            .iter()
            .chain(&probes)
            .map(|request| request.key.clone())
            .collect::<BTreeSet<_>>();
        let prefetch_hints = canonical_requests(prefetch_hints)
            .into_iter()
            .filter(|request| !blocking_keys.contains(request.key()))
            .collect();
        Self {
            required,
            probes,
            prefetch_hints,
        }
    }

    fn blocking_keys(&self) -> BTreeSet<FileRequestKey> {
        self.required
            .iter()
            .chain(&self.probes)
            .map(|request| request.key.clone())
            .collect()
    }

    fn all_keys(&self) -> BTreeSet<FileRequestKey> {
        self.required
            .iter()
            .chain(&self.probes)
            .chain(&self.prefetch_hints)
            .map(|request| request.key.clone())
            .collect()
    }
}

fn canonical_requests(requests: impl IntoIterator<Item = FileRequest>) -> Vec<FileRequest> {
    let mut by_key = BTreeMap::new();
    for request in requests {
        by_key
            .entry(request.key.clone())
            .and_modify(|existing: &mut FileRequest| {
                if request.original_name < existing.original_name {
                    *existing = request.clone();
                }
            })
            .or_insert(request);
    }
    by_key.into_values().collect()
}

/// One host response before generic VFS registration validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedFile {
    pub request: FileRequestKey,
    pub virtual_path: String,
    pub bytes: Vec<u8>,
    pub expected_digest: Option<FileContentId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProvisionOutcome {
    Inserted,
    AlreadyPresent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProvisionError {
    UnexpectedRequest(FileRequestKey),
    KindMismatch {
        expected: FileRequestKey,
        actual: FileRequestKey,
    },
    InvalidPath {
        request: FileRequestKey,
        path: String,
        message: &'static str,
    },
    DigestMismatch {
        request: FileRequestKey,
        expected: FileContentId,
        actual: FileContentId,
    },
    Conflict {
        request: Box<FileRequestKey>,
        existing_path: Box<VirtualPath>,
        incoming_path: Box<VirtualPath>,
        existing: FileContentId,
        incoming: FileContentId,
    },
    AvailabilityConflict {
        request: FileRequestKey,
    },
    PathConflict {
        path: Box<VirtualPath>,
        existing_request: Box<FileRequestKey>,
        incoming_request: Box<FileRequestKey>,
        existing: FileContentId,
        incoming: FileContentId,
    },
    Limit(VfsLimitError),
}

impl fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedRequest(request) => {
                write!(f, "resource response {} was not requested", request.name())
            }
            Self::KindMismatch { expected, actual } => write!(
                f,
                "resource response kind {:?} does not match requested kind {:?} for {}",
                actual.kind,
                expected.kind,
                actual.name()
            ),
            Self::InvalidPath { path, message, .. } => {
                write!(f, "invalid resolved path {path:?}: {message}")
            }
            Self::DigestMismatch {
                request,
                expected,
                actual,
            } => write!(
                f,
                "resolved file digest for {} does not match: {actual} != {expected}",
                request.name()
            ),
            Self::Conflict { request, .. } => write!(
                f,
                "resolved request {} was rebound to different content",
                request.name()
            ),
            Self::AvailabilityConflict { request } => write!(
                f,
                "resolved request {} was rebound between available and unavailable",
                request.name()
            ),
            Self::PathConflict { path, .. } => write!(
                f,
                "distribution path {path} is already bound to different content"
            ),
            Self::Limit(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ProvisionError {}

impl From<VfsLimitError> for ProvisionError {
    fn from(value: VfsLimitError) -> Self {
        Self::Limit(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryError {
    NoProgress,
}

impl fmt::Display for RetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("retry made no progress on required files")
    }
}

impl std::error::Error for RetryError {}

/// Generic typed request registration and immutable provisioning state.
#[derive(Clone, Debug)]
pub struct FileProvisioner {
    limits: VfsLimits,
    storage: LayeredFileStorage,
    files: BTreeMap<FileRequestKey, VirtualPath>,
    unavailable: BTreeSet<FileRequestKey>,
    paths: BTreeMap<VirtualPath, FileRequestKey>,
    user_bytes: usize,
    resolved_bytes: usize,
    expected: BTreeSet<FileRequestKey>,
    required: BTreeSet<FileRequestKey>,
    required_at_batch_start: usize,
}

impl FileProvisioner {
    pub fn new(limits: VfsLimits) -> Result<Self, VfsLimitError> {
        Ok(Self {
            limits: limits.validate()?,
            storage: LayeredFileStorage::new(),
            files: BTreeMap::new(),
            unavailable: BTreeSet::new(),
            paths: BTreeMap::new(),
            user_bytes: 0,
            resolved_bytes: 0,
            expected: BTreeSet::new(),
            required: BTreeSet::new(),
            required_at_batch_start: 0,
        })
    }

    /// Registers or replaces an application-owned `/job` input atomically.
    pub fn register_user(
        &mut self,
        path: VirtualPath,
        bytes: Vec<u8>,
    ) -> Result<ProvisionOutcome, UserRegistrationError> {
        self.limits.check(VfsLimitKind::OneFileBytes, bytes.len())?;
        let existing = self.storage.layer(LayerKind::User).get(&path);
        let next_files =
            self.storage.layer(LayerKind::User).len() + usize::from(existing.is_none());
        self.limits.check(VfsLimitKind::UserFiles, next_files)?;
        let replaced = existing.map_or(0, |file| file.bytes().len());
        let next_bytes = self.limits.checked_replacement_total(
            VfsLimitKind::UserBytes,
            self.user_bytes,
            replaced,
            bytes.len(),
        )?;
        let incoming = VirtualFile::new(path, bytes, FileOrigin::User);
        let outcome = if existing.is_some_and(|file| file == &incoming) {
            ProvisionOutcome::AlreadyPresent
        } else {
            ProvisionOutcome::Inserted
        };
        self.storage.replace_user(incoming)?;
        self.user_bytes = next_bytes;
        Ok(outcome)
    }

    /// Captures one immutable exact-lookup view of all registered inputs.
    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        self.storage.snapshot()
    }

    /// Begins a generated-output build over the same layered storage that
    /// owns this provisioner's immutable inputs.
    pub fn begin_build(&mut self, plan: BuildPlan) -> BuildTransaction<'_> {
        BuildTransaction::new(&mut self.storage, self.limits, plan)
    }

    /// Enumerates typed resolved-resource path bindings in request order.
    pub fn resolved_paths(&self) -> impl Iterator<Item = (&FileRequestKey, &VirtualPath)> {
        self.files.iter()
    }

    /// Enumerates request keys authoritatively bound as absent.
    pub fn unavailable_keys(&self) -> impl Iterator<Item = &FileRequestKey> {
        self.unavailable.iter()
    }

    #[must_use]
    pub fn user_file_count(&self) -> usize {
        self.storage.layer(LayerKind::User).len()
    }

    #[must_use]
    pub fn contains_user(&self, path: &VirtualPath) -> bool {
        self.storage.layer(LayerKind::User).get(path).is_some()
    }

    #[must_use]
    pub const fn user_bytes(&self) -> usize {
        self.user_bytes
    }

    pub fn expect(&mut self, batch: &FileRequestBatch) {
        self.expected = batch.all_keys();
        self.required = batch.blocking_keys();
        self.required_at_batch_start = self
            .required
            .iter()
            .filter(|key| !self.files.contains_key(*key))
            .filter(|key| !self.unavailable.contains(*key))
            .count();
    }

    /// Provisions a response for an outstanding request.
    pub fn provision(
        &mut self,
        response: ResolvedFile,
    ) -> Result<ProvisionOutcome, ProvisionError> {
        self.provision_inner(response, true)
    }

    /// Binds an outstanding request to an immutable absent marker.
    pub fn provision_unavailable(
        &mut self,
        request: FileRequestKey,
    ) -> Result<ProvisionOutcome, ProvisionError> {
        if self.unavailable.contains(&request) {
            return Ok(ProvisionOutcome::AlreadyPresent);
        }
        if self.files.contains_key(&request) {
            return Err(ProvisionError::AvailabilityConflict { request });
        }
        if !self.required.contains(&request) {
            return Err(ProvisionError::UnexpectedRequest(request));
        }
        self.limits.check(
            VfsLimitKind::ResolvedFiles,
            self.files
                .len()
                .saturating_add(self.unavailable.len())
                .saturating_add(1),
        )?;
        self.unavailable.insert(request);
        Ok(ProvisionOutcome::Inserted)
    }

    /// Preserves the explicit native preload API while applying all generic checks.
    pub fn preload(&mut self, response: ResolvedFile) -> Result<ProvisionOutcome, ProvisionError> {
        self.provision_inner(response, false)
    }

    /// Atomically provisions a partial or complete response batch.
    pub fn provision_batch(
        &mut self,
        responses: impl IntoIterator<Item = ResolvedFile>,
    ) -> Result<Vec<ProvisionOutcome>, ProvisionError> {
        let mut staged = self.clone();
        let outcomes = responses
            .into_iter()
            .map(|response| staged.provision(response))
            .collect::<Result<Vec<_>, _>>()?;
        *self = staged;
        Ok(outcomes)
    }

    fn provision_inner(
        &mut self,
        response: ResolvedFile,
        require_expected: bool,
    ) -> Result<ProvisionOutcome, ProvisionError> {
        let path = VirtualPath::distribution(&response.virtual_path).map_err(|error| {
            ProvisionError::InvalidPath {
                request: response.request.clone(),
                path: response.virtual_path.clone(),
                message: error.message(),
            }
        })?;
        self.limits
            .check(VfsLimitKind::OneFileBytes, response.bytes.len())?;
        let content_id = FileContentId::for_bytes(&response.bytes);
        if let Some(expected) = response.expected_digest
            && expected != content_id
        {
            return Err(ProvisionError::DigestMismatch {
                request: response.request,
                expected,
                actual: content_id,
            });
        }
        if let Some(existing_path) = self.files.get(&response.request) {
            let existing = self
                .storage
                .layer(LayerKind::ResolvedResource)
                .get(existing_path)
                .expect("provisioned request paths remain registered");
            if existing_path == &path && existing.content_id() == content_id {
                return Ok(ProvisionOutcome::AlreadyPresent);
            }
            return Err(ProvisionError::Conflict {
                request: Box::new(response.request),
                existing_path: Box::new(existing_path.clone()),
                incoming_path: Box::new(path),
                existing: existing.content_id(),
                incoming: content_id,
            });
        }
        if self.unavailable.contains(&response.request) {
            return Err(ProvisionError::AvailabilityConflict {
                request: response.request,
            });
        }
        if require_expected {
            self.require_expected(&response.request)?;
        }
        self.limits.check(
            VfsLimitKind::ResolvedFiles,
            self.files
                .len()
                .saturating_add(self.unavailable.len())
                .saturating_add(1),
        )?;
        let shared = if let Some(existing_request) = self.paths.get(&path) {
            let existing = self
                .storage
                .layer(LayerKind::ResolvedResource)
                .get(&path)
                .expect("provisioned paths remain registered");
            let existing_id = existing.content_id();
            if existing_id != content_id {
                return Err(ProvisionError::PathConflict {
                    path: Box::new(path),
                    existing_request: Box::new(existing_request.clone()),
                    incoming_request: Box::new(response.request),
                    existing: existing_id,
                    incoming: content_id,
                });
            }
            existing.shared_bytes()
        } else {
            let attempted = self
                .resolved_bytes
                .checked_add(response.bytes.len())
                .ok_or(VfsLimitError::LimitExceeded {
                    kind: VfsLimitKind::ResolvedBytes,
                    limit: self.limits.resolved_bytes,
                    attempted: usize::MAX,
                })?;
            self.limits.check(VfsLimitKind::ResolvedBytes, attempted)?;
            self.resolved_bytes = attempted;
            Arc::from(response.bytes)
        };
        if !self.paths.contains_key(&path) {
            self.storage
                .insert(
                    LayerKind::ResolvedResource,
                    VirtualFile::new(
                        path.clone(),
                        Arc::clone(&shared),
                        FileOrigin::Resolved(response.request.clone()),
                    ),
                )
                .expect("new resolved paths satisfy layer ownership");
            self.paths.insert(path.clone(), response.request.clone());
        }
        self.files.insert(response.request, path);
        Ok(ProvisionOutcome::Inserted)
    }

    pub fn retry(&mut self) -> Result<(), RetryError> {
        let remaining = self
            .required
            .iter()
            .filter(|key| !self.files.contains_key(*key))
            .filter(|key| !self.unavailable.contains(*key))
            .count();
        if remaining == self.required_at_batch_start && remaining != 0 {
            return Err(RetryError::NoProgress);
        }
        self.required_at_batch_start = remaining;
        Ok(())
    }

    #[must_use]
    pub fn get(&self, key: &FileRequestKey) -> Option<&VirtualFile> {
        let path = self.files.get(key)?;
        self.storage.layer(LayerKind::ResolvedResource).get(path)
    }

    #[must_use]
    pub fn is_unavailable(&self, key: &FileRequestKey) -> bool {
        self.unavailable.contains(key)
    }

    pub fn files(&self) -> impl Iterator<Item = (&FileRequestKey, &VirtualFile)> {
        self.files.iter().map(|(key, path)| {
            let file = self
                .storage
                .layer(LayerKind::ResolvedResource)
                .get(path)
                .expect("provisioned request paths remain registered");
            (key, file)
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len() + self.unavailable.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.unavailable.is_empty()
    }

    #[must_use]
    pub const fn resolved_bytes(&self) -> usize {
        self.resolved_bytes
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.unavailable.clear();
        self.paths.clear();
        self.storage.clear_layer(LayerKind::ResolvedResource);
        self.resolved_bytes = 0;
        self.expected.clear();
        self.required.clear();
        self.required_at_batch_start = 0;
    }

    /// Drops accepted generated files while preserving immutable user and
    /// resolved-resource registrations.
    pub fn clear_generated_outputs(&mut self) {
        self.storage.clear_layer(LayerKind::AcceptedGenerated);
        self.storage.clear_layer(LayerKind::PendingGenerated);
    }

    fn require_expected(&self, request: &FileRequestKey) -> Result<(), ProvisionError> {
        if self.expected.contains(request) {
            return Ok(());
        }
        if let Some(expected) = self.expected.iter().find(|expected| {
            expected.domain == request.domain && expected.normalized_name == request.normalized_name
        }) {
            return Err(ProvisionError::KindMismatch {
                expected: expected.clone(),
                actual: request.clone(),
            });
        }
        Err(ProvisionError::UnexpectedRequest(request.clone()))
    }
}

/// A deterministic failure while registering an application user file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserRegistrationError {
    Limit(VfsLimitError),
    Storage(ImmutableBindingError),
}

impl fmt::Display for UserRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(error) => error.fmt(f),
            Self::Storage(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for UserRegistrationError {}

impl From<VfsLimitError> for UserRegistrationError {
    fn from(value: VfsLimitError) -> Self {
        Self::Limit(value)
    }
}

impl From<ImmutableBindingError> for UserRegistrationError {
    fn from(value: ImmutableBindingError) -> Self {
        Self::Storage(value)
    }
}
