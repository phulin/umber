//! Public facade for Umber's in-process bibliography engine.
//!
//! This crate exposes detached jobs and results while semantic worker crates
//! remain implementation boundaries. Resource-session execution is added by a
//! later bibliography issue; these values already contain no host policy or
//! mutable global state.

use std::fmt;
use std::sync::Arc;

mod session;
mod tool;

pub use bib_input::{BibTexLimits, BibTexOptions, XmlLimits};
pub use bib_model::{
    BibConfigurationBuilder, BibDiagnostic, BibDiagnosticCode, BibSeverity, BibSourceLocation,
    COMPATIBILITY_VERSION, CompatibilityVersion, DataList, DataListId, DataListKind, Entry,
    EntryBuilder, EntryId, EntryType, Field, FieldId, FieldProvenance, FieldValue, FieldValueStage,
    GeneratedFile, Literal, NameBuilder, NameList, NamePartValue, OutputFormat, OutputNewline,
    OutputRequest, ProcessedBibliography, ProcessedBibliographyBuilder, ProcessedSection,
    ProcessedSectionBuilder, SectionId, SourceSpan, VirtualPath,
};
pub use bib_output::{
    BblOutputFailure, BblOutputFailureKind, BblSerializer, BibtexCase, BibtexMacro, BibtexOptions,
    BibtexOutputFailure, BibtexOutputFailureKind, BibtexSerializer, DotInclude, DotOptions,
    DotOutputFailure, DotOutputFailureKind, DotSerializer, OutputContext, OutputFailure,
    OutputFailureKind, OutputOptions, OutputRouter, Serializer,
};
pub use bib_unicode::{LegacyEncoding, RecodeSet, UnicodeData};
pub use session::{BibInitFailure, BibSession, BibSessionOptions};
pub use tool::{SyntheticTool, ToolFailure, ToolFailureKind, ToolResult};
pub use umber_vfs::{FileKind, FileRequest, FileRequestBatch, FileRequestKey, VfsSnapshot};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BibOptions {
    tool_mode: bool,
    outputs: Arc<[OutputRequest]>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibResult {
    document: Arc<ProcessedBibliography>,
    files: Arc<[GeneratedFile]>,
    diagnostics: Arc<[BibDiagnostic]>,
    stats: BibStats,
}

impl BibResult {
    #[must_use]
    pub const fn document(&self) -> &Arc<ProcessedBibliography> {
        &self.document
    }
    pub fn files(&self) -> impl ExactSizeIterator<Item = &GeneratedFile> {
        self.files.iter()
    }
    pub fn diagnostics(&self) -> impl ExactSizeIterator<Item = &BibDiagnostic> {
        self.diagnostics.iter()
    }
    #[must_use]
    pub const fn stats(&self) -> BibStats {
        self.stats
    }
}

#[derive(Clone, Debug)]
pub struct BibResultBuilder {
    document: Arc<ProcessedBibliography>,
    files: Vec<GeneratedFile>,
    diagnostics: Vec<BibDiagnostic>,
}

impl BibResultBuilder {
    #[must_use]
    pub fn new(document: Arc<ProcessedBibliography>) -> Self {
        Self {
            document,
            files: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
    pub fn file(&mut self, file: GeneratedFile) -> Result<&mut Self, BibBuildError> {
        if self
            .files
            .iter()
            .any(|existing| existing.path() == file.path())
        {
            return Err(BibBuildError::DuplicateOutputPath(file.path().clone()));
        }
        self.files.push(file);
        Ok(self)
    }
    pub fn diagnostic(&mut self, diagnostic: BibDiagnostic) -> &mut Self {
        self.diagnostics.push(diagnostic);
        self
    }
    #[must_use]
    pub fn files_len(&self) -> usize {
        self.files.len()
    }
    #[must_use]
    pub fn freeze(self) -> BibResult {
        let stats = BibStats {
            sections: self.document.sections().len(),
            entries: self
                .document
                .sections()
                .map(|section| section.entries().len())
                .sum(),
            generated_files: self.files.len(),
            generated_bytes: self.files.iter().map(|file| file.bytes().len()).sum(),
        };
        BibResult {
            document: self.document,
            files: self.files.into(),
            diagnostics: self.diagnostics.into(),
            stats,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibAttempt {
    Complete(BibResult),
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
