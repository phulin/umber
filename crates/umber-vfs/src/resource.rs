use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use tex_content::SharedBytes;

use crate::storage::{DistributionPath, JobPath, WorkspaceStorage};
use crate::{
    AdmissionError, FileContentId, FileOrigin, GeneratedTransaction, ResourceLifecycle,
    VfsLimitError, VfsLimitKind, VfsLimits, VfsSnapshot, VirtualFile, VirtualPath,
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
    /// A classic TeX virtual-font program probed for a TFM-backed font.
    VirtualFont = 13,
    /// A pdfTeX/dvips font-map file.
    PdfFontMap = 14,
    /// A PostScript encoding vector selected by a PDF font map.
    PdfEncoding = 15,
    /// An outline font program selected by a PDF font map.
    PdfFontProgram = 16,
}

impl FileKind {
    #[must_use]
    pub const fn domain(self) -> ResourceDomain {
        match self {
            Self::TexInput
            | Self::Tfm
            | Self::FormatImage
            | Self::Image
            | Self::VirtualFont
            | Self::PdfFontMap
            | Self::PdfEncoding
            | Self::PdfFontProgram => ResourceDomain::Tex,
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
            Self::VirtualFont => "vf",
            Self::PdfFontMap => "font-map",
            Self::PdfEncoding => "font-encoding",
            Self::PdfFontProgram => "font-program",
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
            "vf" => Some(Self::VirtualFont),
            "font-map" => Some(Self::PdfFontMap),
            "font-encoding" => Some(Self::PdfEncoding),
            "font-program" => Some(Self::PdfFontProgram),
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
            Self::VirtualFont => "virtual font",
            Self::PdfFontMap => "PDF font map",
            Self::PdfEncoding => "PDF font encoding",
            Self::PdfFontProgram => "PDF font program",
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
    pub bytes: SharedBytes,
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

/// Immutable typed resource bindings and their deterministic accounting.
#[derive(Clone, Debug, Default)]
pub struct ResourceLedger {
    lifecycle: ResourceLifecycle<FileRequestKey, VirtualPath>,
    user_bytes: usize,
    resolved_bytes: usize,
    required_at_batch_start: usize,
}

impl ResourceLedger {
    /// Returns the canonical path immutably selected for a typed request.
    #[must_use]
    pub fn resolved_path(&self, request: &FileRequestKey) -> Option<&VirtualPath> {
        self.lifecycle.admitted(request)
    }

    /// Reports an authoritative immutable negative binding.
    #[must_use]
    pub fn is_unavailable(&self, request: &FileRequestKey) -> bool {
        self.lifecycle.is_unavailable(request)
    }
}

/// One project-owned virtual filesystem, resource ledger, and build overlay.
///
/// Domain sessions receive snapshots or transaction views from this owner;
/// they never reconstruct storage or resource-binding indexes themselves.
#[derive(Clone, Debug)]
pub struct ProjectWorkspace {
    limits: VfsLimits,
    storage: WorkspaceStorage,
    ledger: ResourceLedger,
}

impl ProjectWorkspace {
    pub fn new(limits: VfsLimits) -> Result<Self, VfsLimitError> {
        Ok(Self {
            limits: limits.validate()?,
            storage: WorkspaceStorage::default(),
            ledger: ResourceLedger::default(),
        })
    }

    /// Returns the immutable typed binding and accounting view.
    #[must_use]
    pub const fn resource_ledger(&self) -> &ResourceLedger {
        &self.ledger
    }

    /// Registers or replaces an application-owned `/job` input atomically.
    pub fn register_user(
        &mut self,
        path: VirtualPath,
        bytes: Vec<u8>,
    ) -> Result<ProvisionOutcome, UserRegistrationError> {
        let path =
            JobPath::new(path).map_err(|path| UserRegistrationError::InvalidPath { path })?;
        self.limits.check(VfsLimitKind::OneFileBytes, bytes.len())?;
        let existing = self.storage.user().get(path.as_path());
        let next_files = self.storage.user().len() + usize::from(existing.is_none());
        self.limits.check(VfsLimitKind::UserFiles, next_files)?;
        let replaced = existing.map_or(0, |file| file.bytes().len());
        let next_bytes = self.limits.checked_replacement_total(
            VfsLimitKind::UserBytes,
            self.ledger.user_bytes,
            replaced,
            bytes.len(),
        )?;
        let bytes = SharedBytes::from(bytes);
        let outcome = if existing.is_some_and(|file| file.bytes() == bytes.as_ref()) {
            ProvisionOutcome::AlreadyPresent
        } else {
            ProvisionOutcome::Inserted
        };
        self.storage.replace_user(path, bytes);
        self.ledger.user_bytes = next_bytes;
        Ok(outcome)
    }

    /// Captures one immutable exact-lookup view of all registered inputs.
    #[must_use]
    pub fn snapshot(&self) -> VfsSnapshot {
        self.storage.snapshot()
    }

    /// Begins a generated-output build over the same layered storage that
    /// owns this provisioner's immutable inputs.
    pub fn begin_generated(&mut self) -> GeneratedTransaction<'_> {
        GeneratedTransaction::new(&mut self.storage, self.limits)
    }

    /// Borrows the resource ledger alongside the disjoint generated overlay.
    ///
    /// Compile resolvers use this narrow view instead of reconstructing
    /// request-to-path and unavailable indexes for every attempt.
    pub fn begin_generated_with_ledger(&mut self) -> (&ResourceLedger, GeneratedTransaction<'_>) {
        let ledger = &self.ledger;
        let generated = GeneratedTransaction::new(&mut self.storage, self.limits);
        (ledger, generated)
    }

    #[must_use]
    pub fn user_file_count(&self) -> usize {
        self.storage.user().len()
    }

    #[must_use]
    pub fn contains_user(&self, path: &VirtualPath) -> bool {
        self.storage.user().get(path).is_some()
    }

    /// Enumerates application-owned inputs in canonical path order.
    ///
    /// Session orchestrators use this view to retain the immutable user-input
    /// overlay independently from the separately edited root buffer.
    pub fn user_files(&self) -> impl Iterator<Item = &VirtualFile> {
        self.storage.user().files().map(|(_, file)| file)
    }

    #[must_use]
    pub const fn user_bytes(&self) -> usize {
        self.ledger.user_bytes
    }

    pub fn expect(&mut self, batch: &FileRequestBatch) {
        self.ledger.lifecycle.begin_batch(
            batch.required.iter().map(|request| request.key.clone()),
            batch.probes.iter().map(|request| request.key.clone()),
            batch
                .prefetch_hints
                .iter()
                .map(|request| request.key.clone()),
        );
        self.ledger.required_at_batch_start = self
            .ledger
            .lifecycle
            .outstanding()
            .filter(|(_, intent)| intent.is_blocking())
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
        if self.ledger.lifecycle.is_unavailable(&request) {
            return Ok(ProvisionOutcome::AlreadyPresent);
        }
        if self.ledger.lifecycle.admitted(&request).is_some() {
            return Err(ProvisionError::AvailabilityConflict { request });
        }
        self.limits.check(
            VfsLimitKind::ResolvedFiles,
            self.ledger.lifecycle.binding_count().saturating_add(1),
        )?;
        self.ledger
            .lifecycle
            .admit_unavailable(request)
            .map(|inserted| {
                if inserted {
                    ProvisionOutcome::Inserted
                } else {
                    ProvisionOutcome::AlreadyPresent
                }
            })
            .map_err(map_admission)
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
        if let Some(existing_path) = self.ledger.lifecycle.admitted(&response.request) {
            let existing = self
                .storage
                .resolved()
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
        if self.ledger.lifecycle.is_unavailable(&response.request) {
            return Err(ProvisionError::AvailabilityConflict {
                request: response.request,
            });
        }
        if require_expected {
            self.require_expected(&response.request)?;
        }
        self.limits.check(
            VfsLimitKind::ResolvedFiles,
            self.ledger.lifecycle.binding_count().saturating_add(1),
        )?;
        let shared = if let Some(existing) = self.storage.resolved().get(&path) {
            let FileOrigin::Resolved(existing_request) = existing.origin() else {
                unreachable!("resolved layer contains only resolved resources")
            };
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
                .ledger
                .resolved_bytes
                .checked_add(response.bytes.len())
                .ok_or(VfsLimitError::LimitExceeded {
                    kind: VfsLimitKind::ResolvedBytes,
                    limit: self.limits.resolved_bytes,
                    attempted: usize::MAX,
                })?;
            self.limits.check(VfsLimitKind::ResolvedBytes, attempted)?;
            self.ledger.resolved_bytes = attempted;
            response.bytes
        };
        if self.storage.resolved().get(&path).is_none() {
            self.storage.insert_resolved(
                DistributionPath::new(path.clone())
                    .expect("distribution canonicalization fixes the root"),
                shared.clone(),
                response.request.clone(),
            );
        }
        if require_expected {
            self.ledger
                .lifecycle
                .admit(response.request, path)
                .map_err(map_admission)?;
        } else {
            self.ledger
                .lifecycle
                .restore(response.request, path)
                .map_err(map_admission)?;
        }
        Ok(ProvisionOutcome::Inserted)
    }

    pub fn retry(&mut self) -> Result<(), RetryError> {
        let remaining = self
            .ledger
            .lifecycle
            .outstanding()
            .filter(|(_, intent)| intent.is_blocking())
            .count();
        if remaining == self.ledger.required_at_batch_start && remaining != 0 {
            return Err(RetryError::NoProgress);
        }
        self.ledger.required_at_batch_start = remaining;
        Ok(())
    }

    #[must_use]
    pub fn get(&self, key: &FileRequestKey) -> Option<&VirtualFile> {
        let path = self.ledger.lifecycle.admitted(key)?;
        self.storage.resolved().get(path)
    }

    #[must_use]
    pub fn is_unavailable(&self, key: &FileRequestKey) -> bool {
        self.ledger.lifecycle.is_unavailable(key)
    }

    pub fn files(&self) -> impl Iterator<Item = (&FileRequestKey, &VirtualFile)> {
        self.ledger.lifecycle.admitted_entries().map(|(key, path)| {
            let file = self
                .storage
                .resolved()
                .get(path)
                .expect("provisioned request paths remain registered");
            (key, file)
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ledger.lifecycle.binding_count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ledger.lifecycle.binding_count() == 0
    }

    #[must_use]
    pub const fn resolved_bytes(&self) -> usize {
        self.ledger.resolved_bytes
    }

    pub fn clear(&mut self) {
        self.ledger.lifecycle.clear();
        self.storage.clear_resolved();
        self.ledger.resolved_bytes = 0;
        self.ledger.required_at_batch_start = 0;
    }

    /// Cancels candidate-local response authorizations without changing
    /// immutable positive or negative session bindings.
    pub fn cancel_outstanding_resources(&mut self) {
        self.ledger.lifecycle.cancel_outstanding();
        self.ledger.required_at_batch_start = 0;
    }

    /// Drops accepted generated files while preserving immutable user and
    /// resolved-resource registrations.
    pub fn clear_generated_outputs(&mut self) {
        self.storage.clear_generated();
    }

    fn require_expected(&self, request: &FileRequestKey) -> Result<(), ProvisionError> {
        if self.ledger.lifecycle.is_outstanding(request) {
            return Ok(());
        }
        if let Some((expected, _)) = self.ledger.lifecycle.outstanding().find(|(expected, _)| {
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

fn map_admission(error: AdmissionError<FileRequestKey>) -> ProvisionError {
    match error {
        AdmissionError::Unexpected(request) | AdmissionError::NegativeHint(request) => {
            ProvisionError::UnexpectedRequest(request)
        }
        AdmissionError::AvailabilityConflict(request) => {
            ProvisionError::AvailabilityConflict { request }
        }
        AdmissionError::BindingConflict(_) => {
            unreachable!("file rebinding conflicts are diagnosed before lifecycle admission")
        }
    }
}

/// A deterministic failure while registering an application user file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserRegistrationError {
    Limit(VfsLimitError),
    InvalidPath { path: VirtualPath },
}

impl fmt::Display for UserRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(error) => error.fmt(f),
            Self::InvalidPath { path } => write!(f, "user path is outside /job: {path}"),
        }
    }
}

impl std::error::Error for UserRegistrationError {}

impl From<VfsLimitError> for UserRegistrationError {
    fn from(value: VfsLimitError) -> Self {
        Self::Limit(value)
    }
}
