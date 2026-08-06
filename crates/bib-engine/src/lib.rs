//! Public facade for Umber's in-process bibliography engine.
//!
//! This crate exposes detached jobs and results while its Biber semantic worker
//! remains private. These values contain no host policy or mutable global state.

use std::fmt;
use std::sync::Arc;

mod biber;
mod bibliography;
mod classic;
mod classic_command;
mod classic_execution;
mod classic_style;
mod command;
mod session;
mod tool;

pub use bib_input::{BibTexLimits, BibTexOptions, XmlLimits};
pub use bib_model::{
    BibConfiguration, BibConfigurationBuilder, BibDiagnostic, BibDiagnosticCode, BibSeverity,
    BibSourceLocation, COMPATIBILITY_VERSION, CompatibilityVersion, DataList, DataListId,
    DataListKind, DiagnosticError, Entry, EntryBuilder, EntryId, EntryType, Field, FieldId,
    FieldProvenance, FieldValue, FieldValueStage, GeneratedFile, Literal, Name, NameBuilder,
    NameList, NamePartValue, OptionId, OptionScope, OptionValue, OutputFormat, OutputNewline,
    OutputRequest, ProcessedBibliography, ProcessedBibliographyBuilder, ProcessedSection,
    ProcessedSectionBuilder, Range, RangeEndpoint, SectionId, SourceSpan, VirtualPath,
};
pub use bib_output::{
    BblOutputFailure, BblOutputFailureKind, BblSerializer, BibtexCase, BibtexMacro, BibtexOptions,
    BibtexOutputFailure, BibtexOutputFailureKind, BibtexSerializer, DotInclude, DotOptions,
    DotOutputFailure, DotOutputFailureKind, DotSerializer, OutputContext, OutputFailure,
    OutputFailureKind, OutputOptions, OutputPlan, OutputRouter,
};
pub use bib_unicode::{LegacyEncoding, RecodeSet, UnicodeData};
#[doc(hidden)]
pub use biber::sort::{
    DataListBuilder, PadDirection, SortComponent, SortDirection, SortField, SortOptions,
    SortTemplate,
};
pub use bibliography::{
    BibliographyAttempt, BibliographyBackend, BibliographyDiagnostic, BibliographyDiagnosticCode,
    BibliographyDocument, BibliographyFailure, BibliographyHistory, BibliographyInput,
    BibliographyJob, BibliographyResult, BibliographyResultError, BibliographySession,
    BibliographySourceLocation, BibliographyStats, ClassicBibCacheUsage, ClassicBibFailure,
    ClassicBibJob, ClassicBibLimits, ClassicBibOptions, ClassicBibSession, ClassicBibliography,
    ClassicBibliographyStats, ClassicDatabaseLimits, ClassicDatabaseOptions, ClassicDiagnosticCode,
    ClassicSourceLocation,
};
pub use classic::{
    BibliographyDetection, BibliographyDetector, BibliographyDetectorOptions, BibliographyMode,
    ClassicControl,
};
pub use classic_command::{ClassicBibCommand, ClassicBibCommandError, ClassicBibCommandOutput};
pub use command::{BibCommand, BibCommandError, BibCommandMode, BibCommandOutput, BibExitStatus};
pub use session::{BibInitFailure, BibSession, BibSessionOptions};
pub use tool::{SyntheticTool, ToolFailure, ToolFailureKind, ToolResult};
pub use umber_vfs::{
    FileKind, FileRequest, FileRequestBatch, FileRequestKey, ProjectWorkspace, ResolvedFile,
    VfsLimits, VfsSnapshot,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BibOptions {
    tool_mode: bool,
    outputs: Arc<[OutputRequest]>,
    output_options: OutputOptions,
    configuration: Option<VirtualPath>,
    schemas: Arc<[VirtualPath]>,
}

impl BibOptions {
    #[must_use]
    pub const fn tool_mode(&self) -> bool {
        self.tool_mode
    }
    pub fn outputs(&self) -> impl ExactSizeIterator<Item = &OutputRequest> {
        self.outputs.iter()
    }
    #[must_use]
    pub const fn output_options(&self) -> &OutputOptions {
        &self.output_options
    }
    #[must_use]
    pub const fn configuration(&self) -> Option<&VirtualPath> {
        self.configuration.as_ref()
    }
    pub fn schemas(&self) -> impl ExactSizeIterator<Item = &VirtualPath> {
        self.schemas.iter()
    }
}

#[derive(Clone, Debug, Default)]
pub struct BibOptionsBuilder {
    tool_mode: bool,
    outputs: Vec<OutputRequest>,
    output_options: OutputOptions,
    configuration: Option<VirtualPath>,
    schemas: Vec<VirtualPath>,
}

impl BibOptionsBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn tool_mode(&mut self, enabled: bool) -> &mut Self {
        self.tool_mode = enabled;
        self
    }
    pub fn output(&mut self, request: OutputRequest) -> Result<&mut Self, BibBuildError> {
        if self
            .outputs
            .iter()
            .any(|existing| existing.path() == request.path())
        {
            return Err(BibBuildError::DuplicateOutputPath(request.path().clone()));
        }
        self.outputs.push(request);
        Ok(self)
    }
    pub fn output_options(&mut self, options: OutputOptions) -> &mut Self {
        self.output_options = options;
        self
    }
    pub fn configuration(&mut self, path: VirtualPath) -> &mut Self {
        self.configuration = Some(path);
        self
    }
    pub fn configuration_path(&mut self, path: VirtualPath) -> &mut Self {
        self.configuration(path)
    }
    pub fn schema(&mut self, path: VirtualPath) -> Result<&mut Self, BibBuildError> {
        if self.schemas.contains(&path) {
            return Err(BibBuildError::DuplicateResourcePath(path));
        }
        self.schemas.push(path);
        Ok(self)
    }
    pub fn schema_path(&mut self, path: VirtualPath) -> Result<&mut Self, BibBuildError> {
        self.schema(path)
    }
    #[must_use]
    pub fn freeze(self) -> BibOptions {
        BibOptions {
            tool_mode: self.tool_mode,
            outputs: self.outputs.into(),
            output_options: self.output_options,
            configuration: self.configuration,
            schemas: self.schemas.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibJob {
    control_path: VirtualPath,
    options: BibOptions,
}

impl BibJob {
    #[must_use]
    pub const fn new(control_path: VirtualPath, options: BibOptions) -> Self {
        Self {
            control_path,
            options,
        }
    }
    #[must_use]
    pub const fn control_path(&self) -> &VirtualPath {
        &self.control_path
    }
    #[must_use]
    pub const fn options(&self) -> &BibOptions {
        &self.options
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BibStats {
    sections: usize,
    entries: usize,
    generated_files: usize,
    generated_bytes: usize,
}

impl BibStats {
    #[must_use]
    pub const fn sections(self) -> usize {
        self.sections
    }
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }
    #[must_use]
    pub const fn generated_files(self) -> usize {
        self.generated_files
    }
    #[must_use]
    pub const fn generated_bytes(self) -> usize {
        self.generated_bytes
    }
}

/// Compatibility name for the unified backend-aware result.
pub type BibResult = BibliographyResult;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibAttempt {
    Complete(BibliographyResult),
    NeedResources(FileRequestBatch),
    Failed(BibFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BibFailureKind {
    InvalidInvocation,
    IncompatibleVersion,
    MalformedInput,
    Validation,
    MissingResource,
    ResourceConflict,
    NoProgress,
    Semantic,
    Output,
    Limit,
    InternalInvariant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibFailure {
    kind: BibFailureKind,
    diagnostics: Arc<[BibDiagnostic]>,
}

impl BibFailure {
    #[must_use]
    pub fn new(kind: BibFailureKind, diagnostics: impl Into<Arc<[BibDiagnostic]>>) -> Self {
        Self {
            kind,
            diagnostics: diagnostics.into(),
        }
    }
    #[must_use]
    pub const fn kind(&self) -> BibFailureKind {
        self.kind
    }
    pub fn diagnostics(&self) -> impl ExactSizeIterator<Item = &BibDiagnostic> {
        self.diagnostics.iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibBuildError {
    DuplicateOutputPath(VirtualPath),
    DuplicateResourcePath(VirtualPath),
}

impl fmt::Display for BibBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for BibBuildError {}

/// Processes one attempt with default cold-session policy.
#[must_use]
pub fn process_once(job: &BibJob, snapshot: &umber_vfs::VfsSnapshot) -> BibAttempt {
    BibSession::default().process(job, snapshot)
}

/// Serializes one detached artifact from an immutable processed document.
pub fn serialize(
    document: &ProcessedBibliography,
    request: &OutputRequest,
) -> Result<GeneratedFile, OutputFailure> {
    OutputRouter::default().serialize(
        OutputContext::new(document, &UnicodeData::pinned()),
        request,
    )
}
