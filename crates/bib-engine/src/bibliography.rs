use std::collections::BTreeSet;
use std::sync::Arc;

use umber_vfs::{FileRequestBatch, VfsSnapshot, VirtualPath};

use crate::{
    BibAttempt, BibDiagnostic, BibDiagnosticCode, BibFailure, BibJob, BibResult, BibSession,
    BibSessionOptions, BibSeverity, BibSourceLocation, BibStats, GeneratedFile,
    ProcessedBibliography,
};

/// Semantic backend selected for a bibliography job.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BibliographyBackend {
    Biblatex,
    Classic,
}

/// A bibliography job with an explicit semantic backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographyJob {
    Biblatex(BibJob),
    Classic(ClassicBibJob),
}

impl BibliographyJob {
    #[must_use]
    pub const fn backend(&self) -> BibliographyBackend {
        match self {
            Self::Biblatex(_) => BibliographyBackend::Biblatex,
            Self::Classic(_) => BibliographyBackend::Classic,
        }
    }
}

impl From<BibJob> for BibliographyJob {
    fn from(job: BibJob) -> Self {
        Self::Biblatex(job)
    }
}

/// Bounded AUX-control resources accepted by a classic bibliography session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClassicBibLimits {
    pub aux_bytes: usize,
    pub aux_files: usize,
    pub aux_depth: usize,
}

impl Default for ClassicBibLimits {
    fn default() -> Self {
        Self {
            aux_bytes: 8 * 1024 * 1024,
            aux_files: 1_024,
            aux_depth: 64,
        }
    }
}

/// Options that affect classic control discovery and later execution phases.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicBibOptions {
    limits: ClassicBibLimits,
    cache_entries: usize,
}

impl Default for ClassicBibOptions {
    fn default() -> Self {
        Self {
            limits: ClassicBibLimits::default(),
            cache_entries: 32,
        }
    }
}

impl ClassicBibOptions {
    #[must_use]
    pub const fn limits(&self) -> ClassicBibLimits {
        self.limits
    }

    #[must_use]
    pub const fn with_limits(mut self, limits: ClassicBibLimits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub const fn with_cache_entries(mut self, entries: usize) -> Self {
        self.cache_entries = entries;
        self
    }

    #[must_use]
    pub const fn cache_entries(&self) -> usize {
        self.cache_entries
    }
}

/// A classic-BibTeX job rooted at a LaTeX AUX file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicBibJob {
    aux_path: VirtualPath,
    options: ClassicBibOptions,
}

impl ClassicBibJob {
    #[must_use]
    pub const fn new(aux_path: VirtualPath, options: ClassicBibOptions) -> Self {
        Self { aux_path, options }
    }

    #[must_use]
    pub const fn aux_path(&self) -> &VirtualPath {
        &self.aux_path
    }

    #[must_use]
    pub const fn options(&self) -> &ClassicBibOptions {
        &self.options
    }
}

/// A resumable bibliography attempt across either semantic backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographyAttempt {
    Finished(BibliographyResult),
    NeedResources(FileRequestBatch),
    Failed(BibliographyFailure),
}

impl From<BibAttempt> for BibliographyAttempt {
    fn from(attempt: BibAttempt) -> Self {
        match attempt {
            BibAttempt::Complete(result) => Self::Finished(result.into()),
            BibAttempt::NeedResources(resources) => Self::NeedResources(resources),
            BibAttempt::Failed(failure) => Self::Failed(BibliographyFailure::Biblatex(failure)),
        }
    }
}

/// The reference-program execution history of a finished bibliography run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BibliographyHistory {
    Spotless,
    Warning,
    Error,
    Fatal,
}

impl BibliographyHistory {
    /// Whether project orchestration may publish the result's generated files.
    #[must_use]
    pub const fn is_publishable(self) -> bool {
        !matches!(self, Self::Fatal)
    }

    fn biblatex(diagnostics: impl Iterator<Item = BibSeverity>) -> Self {
        let mut history = Self::Spotless;
        for severity in diagnostics {
            match severity {
                BibSeverity::Error => return Self::Error,
                BibSeverity::Warning => history = Self::Warning,
                BibSeverity::Info => {}
            }
        }
        history
    }
}

/// A frozen processed document from its originating backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographyDocument {
    Biblatex(Arc<ProcessedBibliography>),
    Classic(Arc<ClassicBibliography>),
}

impl BibliographyDocument {
    #[must_use]
    pub const fn backend(&self) -> BibliographyBackend {
        match self {
            Self::Biblatex(_) => BibliographyBackend::Biblatex,
            Self::Classic(_) => BibliographyBackend::Classic,
        }
    }
}

/// Frozen classic-backend audit data.
///
/// Classic parsing and execution arrive in later phases. Keeping this value
/// detached from the phase-one session ensures those phases cannot expose VM
/// stacks or mutable symbol storage through the public facade.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClassicBibliography {
    aux_files: Arc<[VirtualPath]>,
    databases: Arc<[String]>,
    style: Option<String>,
}

impl ClassicBibliography {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            aux_files: Arc::new([]),
            databases: Arc::new([]),
            style: None,
        }
    }

    pub(crate) fn from_control(control: &crate::classic::ClassicControl) -> Self {
        Self {
            aux_files: control.aux_files().cloned().collect::<Vec<_>>().into(),
            databases: control
                .databases()
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .into(),
            style: control.style().map(str::to_owned),
        }
    }

    pub fn aux_files(&self) -> impl ExactSizeIterator<Item = &VirtualPath> {
        self.aux_files.iter()
    }

    pub fn databases(&self) -> impl ExactSizeIterator<Item = &str> {
        self.databases.iter().map(String::as_str)
    }

    #[must_use]
    pub fn style(&self) -> Option<&str> {
        self.style.as_deref()
    }
}

/// Backend-specific execution statistics retained by a bibliography result.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClassicBibliographyStats {
    _private: (),
}

/// Backend-specific statistics without flattening incompatible measurements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BibliographyStats {
    Biblatex(BibStats),
    Classic(ClassicBibliographyStats),
}

impl From<BibStats> for BibliographyStats {
    fn from(stats: BibStats) -> Self {
        Self::Biblatex(stats)
    }
}

/// A finished bibliography result, including detached fatal partial artifacts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibliographyResult {
    backend: BibliographyBackend,
    history: BibliographyHistory,
    document: BibliographyDocument,
    files: Arc<[GeneratedFile]>,
    partial_files: Arc<[GeneratedFile]>,
    diagnostics: Arc<[BibliographyDiagnostic]>,
    stats: BibliographyStats,
}

impl BibliographyResult {
    /// Creates a completed result while enforcing fatal-artifact publication
    /// policy and unique artifact paths.
    pub fn new(
        history: BibliographyHistory,
        document: BibliographyDocument,
        files: impl Into<Arc<[GeneratedFile]>>,
        partial_files: impl Into<Arc<[GeneratedFile]>>,
        diagnostics: impl Into<Arc<[BibliographyDiagnostic]>>,
        stats: BibliographyStats,
    ) -> Result<Self, BibliographyResultError> {
        let backend = document.backend();
        if stats.backend() != backend {
            return Err(BibliographyResultError::StatsBackendMismatch);
        }
        let files = files.into();
        let partial_files = partial_files.into();
        if !history.is_publishable() && !files.is_empty() {
            return Err(BibliographyResultError::FatalHistoryHasPublishedFiles);
        }
        if history != BibliographyHistory::Fatal && !partial_files.is_empty() {
            return Err(BibliographyResultError::PartialArtifactsRequireFatalHistory);
        }
        let mut paths = BTreeSet::new();
        for file in files.iter().chain(partial_files.iter()) {
            if !paths.insert(file.path().clone()) {
                return Err(BibliographyResultError::DuplicateArtifactPath(
                    file.path().clone(),
                ));
            }
        }
        Ok(Self {
            backend,
            history,
            document,
            files,
            partial_files,
            diagnostics: diagnostics.into(),
            stats,
        })
    }

    #[must_use]
    pub const fn backend(&self) -> BibliographyBackend {
        self.backend
    }

    #[must_use]
    pub const fn history(&self) -> BibliographyHistory {
        self.history
    }

    #[must_use]
    pub const fn document(&self) -> &BibliographyDocument {
        &self.document
    }

    /// Generated files eligible for project publication.
    pub fn files(&self) -> impl ExactSizeIterator<Item = &GeneratedFile> {
        self.files.iter()
    }

    /// Detached fatal artifacts, deliberately excluded from publication.
    pub fn partial_files(&self) -> impl ExactSizeIterator<Item = &GeneratedFile> {
        self.partial_files.iter()
    }

    pub fn diagnostics(&self) -> impl ExactSizeIterator<Item = &BibliographyDiagnostic> {
        self.diagnostics.iter()
    }

    #[must_use]
    pub const fn stats(&self) -> BibliographyStats {
        self.stats
    }

    #[must_use]
    pub const fn is_publishable(&self) -> bool {
        self.history.is_publishable()
    }
}

impl From<BibResult> for BibliographyResult {
    fn from(result: BibResult) -> Self {
        let BibResult {
            document,
            files,
            diagnostics,
            stats,
        } = result;
        let history =
            BibliographyHistory::biblatex(diagnostics.iter().map(BibDiagnostic::severity));
        let diagnostics = diagnostics
            .iter()
            .cloned()
            .map(BibliographyDiagnostic::from)
            .collect::<Vec<_>>();
        Self::new(
            history,
            BibliographyDocument::Biblatex(document),
            files,
            [],
            diagnostics,
            stats.into(),
        )
        .expect("a legacy biblatex result always has compatible publishable artifacts")
    }
}

/// A result-construction policy violation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographyResultError {
    FatalHistoryHasPublishedFiles,
    PartialArtifactsRequireFatalHistory,
    DuplicateArtifactPath(VirtualPath),
    StatsBackendMismatch,
}

impl std::fmt::Display for BibliographyResultError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for BibliographyResultError {}

impl BibliographyStats {
    #[must_use]
    pub const fn backend(self) -> BibliographyBackend {
        match self {
            Self::Biblatex(_) => BibliographyBackend::Biblatex,
            Self::Classic(_) => BibliographyBackend::Classic,
        }
    }
}

/// A backend-aware diagnostic retaining its backend-specific stable code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibliographyDiagnostic {
    backend: BibliographyBackend,
    severity: BibSeverity,
    code: BibliographyDiagnosticCode,
    message: String,
    source: Option<BibliographySourceLocation>,
}

impl BibliographyDiagnostic {
    #[must_use]
    pub fn new(
        severity: BibSeverity,
        code: BibliographyDiagnosticCode,
        message: impl Into<String>,
        source: Option<BibliographySourceLocation>,
    ) -> Self {
        Self {
            backend: code.backend(),
            severity,
            code,
            message: message.into(),
            source,
        }
    }

    #[must_use]
    pub const fn backend(&self) -> BibliographyBackend {
        self.backend
    }

    #[must_use]
    pub const fn severity(&self) -> BibSeverity {
        self.severity
    }

    #[must_use]
    pub const fn code(&self) -> &BibliographyDiagnosticCode {
        &self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn source(&self) -> Option<&BibliographySourceLocation> {
        self.source.as_ref()
    }
}

impl From<BibDiagnostic> for BibliographyDiagnostic {
    fn from(diagnostic: BibDiagnostic) -> Self {
        Self::new(
            diagnostic.severity(),
            BibliographyDiagnosticCode::Biblatex(diagnostic.code().clone()),
            diagnostic.message(),
            diagnostic
                .source()
                .cloned()
                .map(BibliographySourceLocation::Biblatex),
        )
    }
}

/// Stable diagnostic codes are scoped to their originating backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographyDiagnosticCode {
    Biblatex(BibDiagnosticCode),
    Classic(ClassicDiagnosticCode),
}

impl BibliographyDiagnosticCode {
    #[must_use]
    pub const fn backend(&self) -> BibliographyBackend {
        match self {
            Self::Biblatex(_) => BibliographyBackend::Biblatex,
            Self::Classic(_) => BibliographyBackend::Classic,
        }
    }
}

/// A classic-backend stable diagnostic code.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClassicDiagnosticCode(String);

impl ClassicDiagnosticCode {
    pub fn new(value: impl Into<String>) -> Result<Self, crate::DiagnosticError> {
        BibDiagnosticCode::new(value).map(|code| Self(code.as_str().to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A backend-specific diagnostic source location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographySourceLocation {
    Biblatex(BibSourceLocation),
    Classic(ClassicSourceLocation),
}

/// Source location reserved for classic AUX, database, and style diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicSourceLocation {
    path: VirtualPath,
    byte_offset: u64,
    line: Option<u32>,
}

impl ClassicSourceLocation {
    #[must_use]
    pub const fn new(path: VirtualPath, byte_offset: u64, line: Option<u32>) -> Self {
        Self {
            path,
            byte_offset,
            line,
        }
    }

    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }

    #[must_use]
    pub const fn byte_offset(&self) -> u64 {
        self.byte_offset
    }

    #[must_use]
    pub const fn line(&self) -> Option<u32> {
        self.line
    }
}

/// Infrastructure failures, distinct from finished reference histories.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographyFailure {
    Biblatex(BibFailure),
    Classic(ClassicBibFailure),
    BackendMismatch {
        session: BibliographyBackend,
        job: BibliographyBackend,
    },
}

/// Failure categories reserved for classic backend infrastructure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassicBibFailure {
    InvalidInvocation,
    ResourceConflict,
    NoProgress,
    InternalInvariant,
    MalformedInput,
    Limit,
    IncompleteControl,
    AmbiguousProtocol,
}

/// An explicitly dispatched bibliography session.
#[derive(Debug)]
pub enum BibliographySession {
    Biblatex(Box<BibSession>),
    Classic(ClassicBibSession),
}

impl BibliographySession {
    pub fn biblatex(options: BibSessionOptions) -> Result<Self, crate::BibInitFailure> {
        BibSession::new(options).map(|session| Self::Biblatex(Box::new(session)))
    }

    #[must_use]
    pub fn classic() -> Self {
        Self::Classic(ClassicBibSession::new())
    }

    pub fn process(
        &mut self,
        job: &BibliographyJob,
        snapshot: &VfsSnapshot,
    ) -> BibliographyAttempt {
        match (self, job) {
            (Self::Biblatex(session), BibliographyJob::Biblatex(job)) => {
                session.process(job, snapshot).into()
            }
            (Self::Classic(session), BibliographyJob::Classic(job)) => {
                session.process(job, snapshot)
            }
            (Self::Biblatex(_), job) => {
                BibliographyAttempt::Failed(BibliographyFailure::BackendMismatch {
                    session: BibliographyBackend::Biblatex,
                    job: job.backend(),
                })
            }
            (Self::Classic(_), job) => {
                BibliographyAttempt::Failed(BibliographyFailure::BackendMismatch {
                    session: BibliographyBackend::Classic,
                    job: job.backend(),
                })
            }
        }
    }
}

/// Phase-one no-op classic session.
///
/// It exists so callers can exercise explicit backend dispatch and neutral
/// result plumbing before AUX parsing and classic resource requests land.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClassicBibSession {
    control: crate::classic::ClassicControlSession,
}

impl ClassicBibSession {
    #[must_use]
    pub fn new() -> Self {
        Self {
            control: crate::classic::ClassicControlSession::new(),
        }
    }

    #[must_use]
    pub fn process(&mut self, job: &ClassicBibJob, snapshot: &VfsSnapshot) -> BibliographyAttempt {
        self.control.process(job, snapshot)
    }
}
