use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use tex_fonts::{
    AcceptedFontContainers, FontLayoutPolicy, FontLimits, FontMappingFallbackPolicy, FontPurposes,
    FontRequest, FontRequestKey, OpenTypeFont, PdfPkFontRequest, ResolvedFont,
};
use tex_out::html::incremental::{
    PatchLimits, PatchPlan, RenderDigest, RenderDocument, RenderLimits, RenderSessionId,
    build_render_document, plan_patch,
};
use tex_out::html::{HtmlFontAsset, HtmlFontAssets, HtmlFontKey};
use tex_state::{ContentHash, JobClock, Universe};

use crate::{
    MemoryOutputCollectionError, MemoryRunOutput, install_latex_format_primitives,
    install_pdflatex_format_primitives, install_pdftex_format_primitives,
    memory_output::publish_auxiliary_outputs, prepare_etex_run_stores, prepare_latex_run_stores,
    prepare_pdflatex_run_stores, prepare_pdftex_run_stores, prepare_run_stores,
};

mod path;
mod pdf_resources;
mod resolvers;
pub use pdf_resources::{CachedLocalTfm, CachedVirtualFont, PdfVirtualFontResources};
pub(crate) use pdf_resources::{detached_pk_request, resolved_font_map_lines};
pub(crate) use resolvers::parse_image;

use path::{RequestedFile, user_path_for_key};
use resolvers::{FontResolutionPolicy, VirtualRunResolvers};
use umber_vfs::{
    AdmissionError, FileContentId, FileOrigin, FileRequestBatch, ProjectWorkspace, ProvisionError,
    ProvisionOutcome, ResourceLifecycle, TransactionError, UserRegistrationError, VirtualRoot,
};
pub use umber_vfs::{
    FileKind, FileRequest, FileRequestKey, RequestKeyError, ResolvedFile, ResourceDomain,
    VfsLimitError, VfsLimitKind, VfsLimits, VirtualPath, VirtualPathError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceRequest {
    File(FileRequest),
    Font(FontRequest),
    PkFont(PdfPkFontRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedPkFont {
    pub request: PdfPkFontRequest,
    pub virtual_path: String,
    pub bytes: Vec<u8>,
    pub expected_sha256: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceResponse {
    File(ResolvedFile),
    FileUnavailable(FileRequestKey),
    Font(ResolvedFont),
    FontUnavailable(FontRequestKey),
    PkFont(ResolvedPkFont),
    PkFontUnavailable(PdfPkFontRequest),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NeedResources {
    pub required: Vec<ResourceRequest>,
    pub probes: Vec<ResourceRequest>,
    pub prefetch_hints: Vec<ResourceRequest>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionLimits {
    /// Engine executions allowed without an intervening accepted resource
    /// binding. Resource and byte limits separately bound progressing retries.
    pub attempts: u32,
    pub user_files: usize,
    pub resolved_files: usize,
    pub one_file_bytes: usize,
    pub cached_file_bytes: usize,
    pub user_source_bytes: usize,
    pub output_bytes: usize,
    /// Monotonic expansion work allowed for one logical engine revision.
    pub engine_fuel: u64,
    /// Committed owned executor steps allowed for one logical revision.
    pub engine_steps: u64,
    /// Maximum simultaneously live lexer/input frames.
    pub input_frames: u64,
    /// Maximum live environment-journal bytes.
    pub journal_bytes: u64,
    /// Maximum pending virtualized effects.
    pub effects: u64,
}

impl SessionLimits {
    /// Maximum serialized format-image size accepted by every frontend.
    ///
    /// Format images are engine snapshots, not VFS files: a production LaTeX
    /// image can legitimately exceed the per-file resource ceiling while it
    /// remains bounded by the same ceiling as generated engine output.
    pub const FORMAT_IMAGE_BYTES: usize = 256 * 1024 * 1024;

    pub const HARD_MAX: Self = Self {
        attempts: 128,
        user_files: VfsLimits::HARD_MAX.user_files,
        resolved_files: VfsLimits::HARD_MAX.resolved_files,
        one_file_bytes: VfsLimits::HARD_MAX.one_file_bytes,
        cached_file_bytes: VfsLimits::HARD_MAX.resolved_bytes,
        user_source_bytes: VfsLimits::HARD_MAX.user_bytes,
        output_bytes: 256 * 1024 * 1024,
        engine_fuel: 1_000_000_000,
        engine_steps: 100_000_000,
        input_frames: 1_000_000,
        journal_bytes: 1024 * 1024 * 1024,
        effects: 10_000_000,
    };

    fn validate(self) -> Result<Self, CompileError> {
        self.vfs_limits().validate().map_err(map_vfs_limit)?;
        for (resource, attempted, hard) in [
            (
                "compile attempts",
                self.attempts as usize,
                Self::HARD_MAX.attempts as usize,
            ),
            (
                "returned output bytes",
                self.output_bytes,
                Self::HARD_MAX.output_bytes,
            ),
        ] {
            if attempted > hard {
                return Err(CompileError::HardLimitExceeded {
                    resource,
                    hard,
                    attempted,
                });
            }
        }
        for (resource, attempted, hard) in [
            ("engine fuel", self.engine_fuel, Self::HARD_MAX.engine_fuel),
            (
                "engine steps",
                self.engine_steps,
                Self::HARD_MAX.engine_steps,
            ),
            (
                "input frames",
                self.input_frames,
                Self::HARD_MAX.input_frames,
            ),
            (
                "journal bytes",
                self.journal_bytes,
                Self::HARD_MAX.journal_bytes,
            ),
            ("effects", self.effects, Self::HARD_MAX.effects),
        ] {
            if attempted > hard {
                return Err(CompileError::HardLimitExceeded {
                    resource,
                    hard: usize::try_from(hard).unwrap_or(usize::MAX),
                    attempted: usize::try_from(attempted).unwrap_or(usize::MAX),
                });
            }
        }
        Ok(self)
    }

    const fn vfs_limits(self) -> VfsLimits {
        VfsLimits {
            user_files: self.user_files,
            resolved_files: self.resolved_files,
            stage_files: VfsLimits::HARD_MAX.stage_files,
            generated_files: VfsLimits::HARD_MAX.generated_files,
            one_file_bytes: self.one_file_bytes,
            user_bytes: self.user_source_bytes,
            resolved_bytes: self.cached_file_bytes,
            stage_bytes: self.output_bytes,
            generated_bytes: self.output_bytes,
        }
    }
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            attempts: 32,
            user_files: 512,
            resolved_files: 512,
            one_file_bytes: 96 * 1024 * 1024,
            cached_file_bytes: 64 * 1024 * 1024,
            user_source_bytes: 16 * 1024 * 1024,
            output_bytes: 64 * 1024 * 1024,
            engine_fuel: 100_000_000,
            engine_steps: 10_000_000,
            input_frames: 100_000,
            journal_bytes: 256 * 1024 * 1024,
            effects: 1_000_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionOptions {
    pub main_path: String,
    pub job_name: Option<String>,
    /// Optional authored startup identity, distinct from the VFS provenance
    /// path. This selects the same canonical root framing as
    /// [`crate::RetainedRootRequest::authored`].
    pub authored_root_name: Option<String>,
    pub format: Option<Vec<u8>>,
    /// Validated, transport-only requests likely to be needed by the compile.
    /// They are emitted once, with the first required resource batch.
    pub initial_prefetch_hints: Option<Box<[ResourceRequest]>>,
    pub engine: EngineMode,
    pub clock: JobClock,
    pub limits: SessionLimits,
    /// Downstream products requested independently from engine compatibility.
    pub outputs: OutputCapabilitySet,
    /// HTML asset publication policy fixed before execution.
    pub html_asset_mode: tex_out::html::AssetMode,
    /// Font containers the host can provide. Browser sessions use WOFF2.
    pub accepted_font_containers: AcceptedFontContainers,
    /// Versioned authority for unprefixed TFM-style font selections.
    pub font_layout_policy: FontLayoutPolicy,
    /// Explicit missing-mapping behavior under `OpenTypePreferred`.
    pub font_mapping_fallback: FontMappingFallbackPolicy,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            main_path: "/job/main.tex".to_owned(),
            job_name: None,
            authored_root_name: None,
            format: None,
            initial_prefetch_hints: None,
            engine: EngineMode::Tex82,
            clock: JobClock::DEFAULT,
            limits: SessionLimits::default(),
            outputs: OutputCapabilitySet::DVI,
            html_asset_mode: tex_out::html::AssetMode::Embedded,
            accepted_font_containers: AcceptedFontContainers::WASM,
            font_layout_policy: FontLayoutPolicy::OpenTypePreferred,
            font_mapping_fallback: FontMappingFallbackPolicy::ClassicTfmExact,
        }
    }
}

/// One downstream product selected independently from [`EngineMode`].
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OutputCapability {
    Dvi,
    Pdf,
    Html,
}

/// A nonempty set of downstream products fixed before execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputCapabilitySet(u8);

impl OutputCapabilitySet {
    const DVI_BIT: u8 = 1;
    const PDF_BIT: u8 = 2;
    const HTML_BIT: u8 = 4;

    pub const DVI: Self = Self(Self::DVI_BIT);
    pub const PDF: Self = Self(Self::PDF_BIT);
    pub const HTML: Self = Self(Self::HTML_BIT);

    #[must_use]
    pub const fn new(capability: OutputCapability) -> Self {
        Self(capability.bit())
    }

    #[must_use]
    pub const fn with(self, capability: OutputCapability) -> Self {
        Self(self.0 | capability.bit())
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn contains(self, capability: OutputCapability) -> bool {
        self.0 & capability.bit() != 0
    }

    pub fn iter(self) -> impl Iterator<Item = OutputCapability> {
        [
            OutputCapability::Dvi,
            OutputCapability::Pdf,
            OutputCapability::Html,
        ]
        .into_iter()
        .filter(move |capability| self.contains(*capability))
    }
}

impl Default for OutputCapabilitySet {
    fn default() -> Self {
        Self::DVI
    }
}

impl OutputCapability {
    const fn bit(self) -> u8 {
        match self {
            Self::Dvi => OutputCapabilitySet::DVI_BIT,
            Self::Pdf => OutputCapabilitySet::PDF_BIT,
            Self::Html => OutputCapabilitySet::HTML_BIT,
        }
    }
}

/// The engine compatibility contract selected for a composed session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EngineMode {
    #[default]
    Tex82,
    ETex,
    PdfTex,
    Latex,
    PdfLatex,
}

impl EngineMode {
    #[must_use]
    pub const fn binary_identity(self) -> tex_exec::EngineBinaryIdentity {
        match self {
            Self::Tex82 => tex_exec::EngineBinaryIdentity::Tex82,
            Self::ETex => tex_exec::EngineBinaryIdentity::Etex26,
            Self::PdfTex | Self::Latex | Self::PdfLatex => {
                tex_exec::EngineBinaryIdentity::Pdftex14029
            }
        }
    }

    /// The canonical command profile shared by fresh and format-loaded runs.
    #[must_use]
    pub const fn command_profile(self) -> tex_command::CommandProfile {
        match self {
            Self::Tex82 => tex_command::CommandProfile::TEX82,
            Self::ETex | Self::Latex => tex_command::CommandProfile::ETEX26,
            Self::PdfTex | Self::PdfLatex => tex_command::CommandProfile::PDFTEX14029,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tex82 => "tex82",
            Self::ETex => "etex",
            Self::PdfTex => "pdftex",
            Self::Latex => "latex",
            Self::PdfLatex => "pdflatex",
        }
    }

    #[must_use]
    pub const fn version(self) -> &'static str {
        match self {
            Self::Tex82 => "3.141592653",
            Self::ETex => "2.6",
            Self::PdfTex => "1.40.29",
            Self::Latex => "1",
            Self::PdfLatex => "1.40.29",
        }
    }

    /// Installs the primitive and state layers selected for a fresh run.
    ///
    /// This is the non-interactive host: an editor or browser session has no
    /// terminal at all. tex.web §75 starts a job in `error_stop_mode`, and
    /// §82 enters §83's dialog on that alone; §71 answers a terminal at end
    /// of file with `fatal_error`, so leaving the default in place would end
    /// a virtual compile at its first recoverable error. Selecting
    /// `\nonstopmode` is what a host with no terminal passes on the command
    /// line, and is the same thing this crate's `-interaction=nonstopmode`
    /// reference captures run under.
    pub fn prepare_fresh<G>(self, stores: &mut Universe<G>) {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        match self {
            Self::Tex82 => prepare_run_stores(stores),
            Self::ETex => prepare_etex_run_stores(stores),
            Self::PdfTex => prepare_pdftex_run_stores(stores),
            Self::Latex => prepare_latex_run_stores(stores),
            Self::PdfLatex => prepare_pdflatex_run_stores(stores),
        }
    }

    /// Installs the primitive and state layers selected for an INITEX run.
    ///
    /// Unlike [`Self::prepare_fresh`], this preserves TeX82's initial
    /// category-code table while the format source is tokenized.
    pub fn prepare_initex<G>(self, stores: &mut Universe<G>) {
        // Same non-interactive host as [`Self::prepare_fresh`].
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        crate::prepare_initex_stores(stores);
        match self {
            Self::Tex82 => {}
            Self::ETex => {
                tex_command::install_etex_expandable_primitives(stores);
                tex_exec::install_etex_unexpandable_primitives(stores);
            }
            Self::PdfTex => {
                tex_command::install_etex_expandable_primitives(stores);
                tex_exec::install_etex_unexpandable_primitives(stores);
                crate::pdftex::install_pdftex_layer(stores);
                stores.enable_pdf_output();
            }
            Self::Latex => {
                tex_command::install_etex_expandable_primitives(stores);
                tex_exec::install_etex_unexpandable_primitives(stores);
                crate::install_latex_compatibility_layer(stores);
            }
            Self::PdfLatex => {
                tex_command::install_etex_expandable_primitives(stores);
                tex_exec::install_etex_unexpandable_primitives(stores);
                crate::pdftex::install_pdftex_layer(stores);
                stores.enable_pdf_output();
                crate::install_latex_compatibility_layer(stores);
            }
        }
    }

    /// Restores driver-owned primitive implementations after a format load.
    pub fn install_after_format<G>(self, stores: &mut Universe<G>) {
        match self {
            Self::Tex82 => {
                tex_command::register_tex82_expandable_primitives(stores);
                tex_exec::register_unexpandable_primitives(stores);
            }
            Self::ETex => {
                tex_command::register_tex82_expandable_primitives(stores);
                tex_command::register_etex_expandable_primitives(stores);
                tex_exec::register_unexpandable_primitives(stores);
                tex_exec::register_etex_unexpandable_primitives(stores);
            }
            Self::PdfTex => install_pdftex_format_primitives(stores),
            Self::Latex => install_latex_format_primitives(stores),
            Self::PdfLatex => install_pdflatex_format_primitives(stores),
        }
    }

    /// Whether this compatibility contract uses LaTeX's byte-oriented UTF-8 input layer.
    #[must_use]
    pub const fn uses_latex_input(self) -> bool {
        matches!(self, Self::Latex | Self::PdfLatex)
    }

    /// Whether this compatibility contract can publish PDF output.
    #[must_use]
    pub const fn supports_pdf_output(self) -> bool {
        matches!(self, Self::PdfTex | Self::PdfLatex)
    }
}

/// One atomic root-buffer replacement for a persistent compile session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcePatch {
    pub next_revision: tex_incr::RevisionId,
    pub base_revision: tex_incr::RevisionId,
    pub expected_hash: ContentHash,
    pub range: std::ops::Range<usize>,
    pub replacement: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileSourceLocation {
    pub file: String,
    pub byte_start: u64,
    pub byte_end: u64,
    pub line: u32,
    pub column: u32,
    /// Owned physical line text detached before the engine generation retires.
    pub excerpt: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileDiagnostic {
    pub message: String,
    pub location: Option<CompileSourceLocation>,
    pub context: Option<Box<tex_exec::FrozenDiagnosticContext>>,
}

/// Live retained-memory charges for one accepted compile session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetentionMetrics {
    pub checkpoint_root_bytes: usize,
    pub diagnostic_bytes: usize,
    pub output_bytes: usize,
    pub resource_bytes: usize,
    pub protected_overage_bytes: usize,
}

/// Monotonic engine and host-wait telemetry for the active logical revision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompileTelemetry {
    pub execution: tex_exec::ExecutionTelemetry,
    pub resource_wait_time: Duration,
    pub request_extraction_time: Duration,
    pub candidate_restore_time: Duration,
    pub resolver_index_time: Duration,
    pub vfs_stage_time: Duration,
}

/// One rendered text unit resolved against the accepted editor revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedSourceLocation {
    pub revision: tex_incr::RevisionId,
    pub path: String,
    pub start: u64,
    pub end: u64,
    pub line: u32,
    pub column: u32,
}

/// Revision-checked result of a rendered-source query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderedSourceResult {
    Current(RenderedSourceLocation),
    Deleted {
        minted_revision: u64,
    },
    StaleRevision {
        accepted: tex_incr::RevisionId,
    },
    OutputMismatch {
        accepted: tex_incr::RenderedOutputId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompileAttemptResult {
    NeedResources(NeedResources),
    Complete(MemoryRunOutput),
    Error(CompileError),
}

/// Accepted one-shot engine state handed from the resource session to a
/// client-owned downstream finalizer. Effects remain uncommitted.
pub struct AcceptedFinalization {
    pub completion: tex_exec::DetachedEngineCompletion,
    pub format_dump: Option<tex_exec::DetachedFormatDump>,
    pub expansion_stats: tex_incr::ExpansionStats,
    pub virtual_font_resources: PdfVirtualFontResources,
    pub pdf_font_closure_receipt: PdfFontClosureReceipt,
    pub pdf_raw_object_file_receipt: PdfRawObjectFileReceipt,
}

/// Identity-pinned file payloads owned by accepted raw PDF objects.
///
/// The object number and exact source spelling bind each immutable VFS payload
/// to the engine ledger that requested it. Detached finalization therefore
/// never reconstructs a host filename or performs a second read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PdfRawObjectFileReceipt {
    pub entries: BTreeMap<u32, PdfRawObjectFileReceiptEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfRawObjectFileReceiptEntry {
    pub source_name: String,
    pub source: PdfRawObjectFileSource,
    pub virtual_path: String,
    pub content_id: FileContentId,
    /// Exact accepted payload, owned independently of the VFS snapshot.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfRawObjectFileSource {
    User(VirtualPath),
    Resolved(FileRequestKey),
}

/// Complete typed PDF classic-font request closure retained by an accepted
/// session. Resolved entries carry immutable payload evidence; authoritative
/// VF absence remains part of the receipt without becoming a materialization
/// key.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PdfFontClosureReceipt {
    pub entries: Vec<PdfFontClosureReceiptEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfFontClosureReceiptEntry {
    File {
        request: FileRequestKey,
        outcome: PdfFontClosureResourceOutcome,
    },
    PkFont {
        request: PdfPkFontRequest,
        outcome: PdfFontClosureResourceOutcome,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfFontClosureResourceOutcome {
    Resolved {
        virtual_path: String,
        bytes: usize,
        sha256: [u8; 32],
    },
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompileError {
    InvalidVirtualPath {
        path: String,
        message: String,
    },
    InvalidRequestedPath {
        name: String,
        message: String,
    },
    UnavailableAbsoluteUserFile(String),
    MissingMainFile(String),
    HardLimitExceeded {
        resource: &'static str,
        hard: usize,
        attempted: usize,
    },
    LimitExceeded {
        resource: &'static str,
        limit: usize,
        attempted: usize,
    },
    AttemptLimit {
        limit: u32,
    },
    NoProgress,
    ConflictingResolvedBinding(String),
    ConflictingHtmlFontBinding {
        name: String,
        expected_tfm_identity: [u8; 32],
        conflicting_tfm_identity: [u8; 32],
    },
    UnexpectedResourceResponse(String),
    FileProvision(ProvisionError),
    Font(String),
    DistributionPathCollision(String),
    Format(String),
    Diagnostic(CompileDiagnostic),
    World(String),
    Output(String),
    Html(String),
    OutputCapability {
        capability: OutputCapability,
        message: String,
    },
    Incremental(String),
    SessionAlreadyStarted,
    PatchAlreadyPending,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVirtualPath { path, message } => {
                write!(f, "invalid virtual path {path:?}: {message}")
            }
            Self::InvalidRequestedPath { name, message } => {
                write!(f, "invalid requested path {name:?}: {message}")
            }
            Self::UnavailableAbsoluteUserFile(path) => {
                write!(f, "absolute user file {path} is unavailable")
            }
            Self::MissingMainFile(path) => write!(f, "main file {path} was not provided"),
            Self::HardLimitExceeded {
                resource,
                hard,
                attempted,
            } => write!(
                f,
                "{resource} setting {attempted} exceeds hard ceiling {hard}"
            ),
            Self::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(
                f,
                "{resource} requires {attempted}, exceeding limit {limit}"
            ),
            Self::AttemptLimit { limit } => write!(f, "compile attempt limit {limit} reached"),
            Self::NoProgress => f.write_str("retry made no progress on requested files"),
            Self::ConflictingResolvedBinding(name) => {
                write!(
                    f,
                    "resolved request {name} was rebound to different content"
                )
            }
            Self::ConflictingHtmlFontBinding {
                name,
                expected_tfm_identity,
                conflicting_tfm_identity,
            } => write!(
                f,
                "HTML font {name} has conflicting TFM identities {} and {}",
                hex_sha256(*expected_tfm_identity),
                hex_sha256(*conflicting_tfm_identity),
            ),
            Self::UnexpectedResourceResponse(name) => {
                write!(f, "resource response {name} was not requested")
            }
            Self::FileProvision(error) => error.fmt(f),
            Self::Font(message) => write!(f, "font resource rejected: {message}"),
            Self::DistributionPathCollision(path) => {
                write!(
                    f,
                    "distribution path {path} is already bound to another request"
                )
            }
            Self::Format(message) => write!(f, "format image rejected: {message}"),
            Self::Diagnostic(diagnostic) => f.write_str(&diagnostic.message),
            Self::OutputCapability {
                capability,
                message,
            } => write!(f, "{capability:?} output finalization failed: {message}"),
            Self::World(message)
            | Self::Output(message)
            | Self::Html(message)
            | Self::Incremental(message) => f.write_str(message),
            Self::SessionAlreadyStarted => {
                f.write_str("user files cannot change after the first revision is accepted")
            }
            Self::PatchAlreadyPending => f.write_str("a source patch is already pending"),
        }
    }
}

impl std::error::Error for CompileError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FontResponseFingerprint {
    container: tex_fonts::FontContainer,
    object: tex_fonts::FontObjectIdentity,
    declared_object: Option<tex_fonts::FontObjectIdentity>,
    declared_program: Option<tex_fonts::FontProgramIdentity>,
    provenance: Option<String>,
    legacy_mapping: Option<tex_fonts::LegacyFontMapping>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ResourceRequestKey {
    File(FileRequestKey),
    Font(FontRequestKey),
    PkFont(PdfPkFontRequest),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum NonFileResourceKey {
    Font(FontRequestKey),
    PkFont(PdfPkFontRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NonFileAdmission {
    Font(FontResponseFingerprint),
    PkFont { path: String, sha256: [u8; 32] },
}

pub struct VirtualCompileSession {
    main_path: VirtualPath,
    job_name: String,
    authored_root_name: Option<String>,
    format: Option<Vec<u8>>,
    initial_prefetch_hints: Option<Box<[ResourceRequest]>>,
    engine: EngineMode,
    clock: JobClock,
    limits: SessionLimits,
    workspace: ProjectWorkspace,
    font_cached_bytes: usize,
    attempts: u32,
    attempts_without_progress: u32,
    awaiting: Option<BTreeSet<ResourceRequestKey>>,
    non_file_admission: ResourceLifecycle<NonFileResourceKey, NonFileAdmission>,
    font_requests: BTreeMap<FontRequestKey, FontRequest>,
    resolved_fonts: BTreeMap<FontRequestKey, OpenTypeFont>,
    resolved_pk_fonts: BTreeMap<PdfPkFontRequest, ResolvedPkFont>,
    accepted_font_containers: AcceptedFontContainers,
    font_layout_policy: FontLayoutPolicy,
    font_mapping_fallback: FontMappingFallbackPolicy,
    outputs: OutputCapabilitySet,
    html_asset_mode: tex_out::html::AssetMode,
    incremental: Option<Box<tex_incr::Session>>,
    accepted_engine_output: Option<Box<tex_incr::AcceptedOutput>>,
    accepted_output: Option<MemoryRunOutput>,
    accepted_render_document: Option<RenderDocument>,
    pending_render_update: Option<RenderUpdate>,
    pending_patch: Option<(tex_incr::RevisionId, tex_incr::Edit)>,
    candidate: Option<Box<RetainedCandidate>>,
    response_generation: u64,
    virtual_font_resources: PdfVirtualFontResources,
    pdf_font_closure_requests: BTreeSet<ResourceRequestKey>,
    last_reuse: Option<tex_incr::ReuseMetrics>,
    last_stabilization_required: bool,
    initial_revision: tex_incr::RevisionId,
    execution_telemetry: tex_exec::ExecutionTelemetry,
    resource_wait_time: Duration,
    request_extraction_time: Duration,
    candidate_restore_time: Duration,
    resolver_index_time: Duration,
    vfs_stage_time: Duration,
    #[cfg(not(target_arch = "wasm32"))]
    resource_wait_started: Option<Instant>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderUpdate {
    Snapshot(RenderDocument),
    Patch(PatchPlan),
}

impl RenderUpdate {
    #[must_use]
    pub fn target_revision(&self) -> u64 {
        match self {
            Self::Snapshot(document) => document.revision.revision,
            Self::Patch(patch) => patch.target_revision,
        }
    }

    #[must_use]
    pub fn target_digest(&self) -> RenderDigest {
        match self {
            Self::Snapshot(document) => document.revision.digest,
            Self::Patch(patch) => patch.after_digest,
        }
    }
}

enum CandidateExecution {
    Initial {
        session: Box<tex_incr::Session>,
        accepted: Result<Box<tex_incr::AcceptedOutput>, tex_incr::SessionError>,
    },
    Transaction(Result<Box<tex_incr::RevisionTransaction>, tex_incr::SessionError>),
}

enum RetainedExecution {
    Initial {
        session: Box<tex_incr::Session>,
        candidate: tex_incr::RevisionCandidate,
    },
    Pending(tex_incr::RevisionCandidate),
}

struct RetainedCandidate {
    workspace: ProjectWorkspace,
    execution: RetainedExecution,
    response_generation: u64,
    suspension_serial: u64,
}

impl RetainedCandidate {
    fn engine_retention(&self) -> tex_incr::RetentionMetrics {
        match &self.execution {
            RetainedExecution::Initial { candidate, .. }
            | RetainedExecution::Pending(candidate) => candidate.retention_metrics(),
        }
    }
}

impl RetainedExecution {
    fn resolve_frozen_diagnostic_primary(
        &self,
        frozen: &tex_exec::FrozenDiagnosticOrigin,
    ) -> Option<CompileSourceLocation> {
        let resolved = match self {
            RetainedExecution::Initial { candidate, .. }
            | RetainedExecution::Pending(candidate) => {
                candidate.resolve_frozen_diagnostic_primary(frozen)
            }
        }?;
        Some(CompileSourceLocation {
            file: resolved.path,
            byte_start: resolved.start,
            byte_end: resolved.end,
            line: resolved.line,
            column: resolved.column,
            excerpt: resolved.excerpt,
        })
    }
}

impl CompileDiagnostic {
    fn from_session_error(
        error: &tex_incr::SessionError,
        candidate: Option<&RetainedExecution>,
    ) -> Self {
        let location = error
            .frozen_diagnostic_origin()
            .and_then(|frozen| candidate?.resolve_frozen_diagnostic_primary(frozen));
        Self {
            message: error.to_string(),
            location,
            context: error.frozen_diagnostic_context().cloned().map(Box::new),
        }
    }
}

enum PreparedExecution {
    Initial {
        session: Box<tex_incr::Session>,
        accepted: Box<tex_incr::AcceptedOutput>,
    },
    Transaction(Box<tex_incr::RevisionTransaction>),
}

impl CandidateExecution {
    fn into_prepared(self) -> Result<PreparedExecution, tex_incr::SessionError> {
        match self {
            Self::Initial { session, accepted } => Ok(PreparedExecution::Initial {
                session,
                accepted: accepted?,
            }),
            Self::Transaction(transaction) => Ok(PreparedExecution::Transaction(transaction?)),
        }
    }
}

impl PreparedExecution {
    fn revision(&self) -> tex_incr::RevisionId {
        match self {
            Self::Initial { accepted, .. } => accepted.revision,
            Self::Transaction(transaction) => transaction.revision(),
        }
    }

    fn reuse(&self) -> tex_incr::ReuseMetrics {
        match self {
            Self::Initial { accepted, .. } => accepted.reuse,
            Self::Transaction(transaction) => transaction.reuse(),
        }
    }

    fn completion(&self) -> &tex_exec::DetachedEngineCompletion {
        match self {
            Self::Initial { accepted, .. } => accepted.completion(),
            Self::Transaction(transaction) => transaction.completion(),
        }
    }

    fn pages(&self) -> &[tex_exec::DetachedPreparedPage] {
        self.completion().pages()
    }

    fn dvi_bytes(&self) -> Result<Vec<u8>, tex_out::dvi::DviError> {
        match self {
            Self::Initial { accepted, .. } => accepted.dvi_bytes(),
            Self::Transaction(transaction) => transaction.dvi_bytes(),
        }
    }
}

impl VirtualCompileSession {
    fn execution_budgets(&self) -> tex_exec::ExecutionBudgets {
        tex_exec::ExecutionBudgets {
            steps: self.limits.engine_steps,
            input_frames: self.limits.input_frames,
            journal_bytes: self.limits.journal_bytes,
            effects: self.limits.effects,
        }
    }

    pub fn new(options: SessionOptions) -> Result<Self, CompileError> {
        Self::new_at_revision(options, tex_incr::RevisionId::new(1))
    }

    pub(crate) fn new_at_revision(
        options: SessionOptions,
        initial_revision: tex_incr::RevisionId,
    ) -> Result<Self, CompileError> {
        let limits = options.limits.validate()?;
        if options.outputs.contains(OutputCapability::Pdf) && !options.engine.supports_pdf_output()
        {
            return Err(CompileError::OutputCapability {
                capability: OutputCapability::Pdf,
                message: format!(
                    "engine {} does not provide pdfTeX PDF semantics",
                    options.engine.name()
                ),
            });
        }
        let main_path = VirtualPath::user(&options.main_path).map_err(|error| {
            CompileError::InvalidVirtualPath {
                path: options.main_path.clone(),
                message: error.to_string(),
            }
        })?;
        if let Some(format) = &options.format {
            check_format_image_bytes(format.len())?;
        }
        let job_name = options.job_name.unwrap_or_else(|| {
            Path::new(main_path.as_str())
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("texput")
                .to_owned()
        });
        let mut initial_prefetch_hints = options
            .initial_prefetch_hints
            .map_or_else(Vec::new, |hints| hints.into_vec());
        initial_prefetch_hints.sort_by_key(resource_sort_key);
        initial_prefetch_hints.dedup();
        check_limit(
            "initial prefetch hints",
            initial_prefetch_hints.len(),
            limits.resolved_files,
        )?;
        Ok(Self {
            main_path,
            job_name,
            authored_root_name: options.authored_root_name,
            format: options.format,
            initial_prefetch_hints: (!initial_prefetch_hints.is_empty())
                .then(|| initial_prefetch_hints.into_boxed_slice()),
            engine: options.engine,
            clock: options.clock,
            limits,
            workspace: ProjectWorkspace::new(limits.vfs_limits()).map_err(map_vfs_limit)?,
            font_cached_bytes: 0,
            attempts: 0,
            attempts_without_progress: 0,
            awaiting: None,
            non_file_admission: ResourceLifecycle::default(),
            font_requests: BTreeMap::new(),
            resolved_fonts: BTreeMap::new(),
            resolved_pk_fonts: BTreeMap::new(),
            accepted_font_containers: options.accepted_font_containers,
            font_layout_policy: options.font_layout_policy,
            font_mapping_fallback: options.font_mapping_fallback,
            outputs: options.outputs,
            html_asset_mode: options.html_asset_mode,
            incremental: None,
            accepted_engine_output: None,
            accepted_output: None,
            accepted_render_document: None,
            pending_render_update: None,
            pending_patch: None,
            candidate: None,
            response_generation: 0,
            virtual_font_resources: PdfVirtualFontResources::default(),
            pdf_font_closure_requests: BTreeSet::new(),
            last_reuse: None,
            last_stabilization_required: false,
            initial_revision,
            execution_telemetry: tex_exec::ExecutionTelemetry::default(),
            resource_wait_time: Duration::ZERO,
            request_extraction_time: Duration::ZERO,
            candidate_restore_time: Duration::ZERO,
            resolver_index_time: Duration::ZERO,
            vfs_stage_time: Duration::ZERO,
            #[cfg(not(target_arch = "wasm32"))]
            resource_wait_started: None,
        })
    }

    pub(crate) fn session_options(&self) -> SessionOptions {
        SessionOptions {
            main_path: self.main_path.to_string(),
            job_name: Some(self.job_name.clone()),
            authored_root_name: self.authored_root_name.clone(),
            format: self.format.clone(),
            initial_prefetch_hints: self.initial_prefetch_hints.clone(),
            engine: self.engine,
            clock: self.clock,
            limits: self.limits,
            outputs: self.outputs,
            html_asset_mode: self.html_asset_mode.clone(),
            accepted_font_containers: self.accepted_font_containers,
            font_layout_policy: self.font_layout_policy,
            font_mapping_fallback: self.font_mapping_fallback,
        }
    }

    pub(crate) fn workspace(&self) -> &ProjectWorkspace {
        &self.workspace
    }

    /// Consumes a completed session and transfers its accepted engine state
    /// to a one-shot client finalizer. Pending and failed revisions never
    /// cross this boundary.
    pub fn into_accepted_finalization(self) -> Result<AcceptedFinalization, CompileError> {
        if self.pending_patch.is_some()
            || self.candidate.is_some()
            || self.accepted_output.is_none()
        {
            return Err(CompileError::Incremental(
                "the session has no completed accepted output to finalize".to_owned(),
            ));
        }
        let pdf_font_closure_receipt = self.pdf_font_closure_receipt()?;
        let session = self.incremental.ok_or_else(|| {
            CompileError::Incremental("the accepted incremental session is missing".to_owned())
        })?;
        let expansion_stats = session.accepted_expansion_stats();
        let accepted = self.accepted_engine_output.ok_or_else(|| {
            CompileError::Incremental("the accepted detached completion is missing".to_owned())
        })?;
        let pdf_raw_object_file_receipt =
            pdf_raw_object_file_receipt(accepted.pdf(), &self.workspace)?;
        let (completion, format_dump) = accepted.into_terminal();
        Ok(AcceptedFinalization {
            completion,
            format_dump,
            expansion_stats,
            virtual_font_resources: self.virtual_font_resources,
            pdf_font_closure_receipt,
            pdf_raw_object_file_receipt,
        })
    }

    fn pdf_font_closure_receipt(&self) -> Result<PdfFontClosureReceipt, CompileError> {
        let mut entries = Vec::with_capacity(self.pdf_font_closure_requests.len());
        for request in &self.pdf_font_closure_requests {
            let entry = match request {
                ResourceRequestKey::File(request) => {
                    let outcome = if let Some(file) = self.workspace.get(request) {
                        PdfFontClosureResourceOutcome::Resolved {
                            virtual_path: file.path().as_str().to_owned(),
                            bytes: file.bytes().len(),
                            sha256: Sha256::digest(file.bytes()).into(),
                        }
                    } else if self.workspace.is_unavailable(request) {
                        PdfFontClosureResourceOutcome::Unavailable
                    } else {
                        return Err(CompileError::Incremental(format!(
                            "accepted PDF font closure has an unbound file request {}",
                            request.name()
                        )));
                    };
                    PdfFontClosureReceiptEntry::File {
                        request: request.clone(),
                        outcome,
                    }
                }
                ResourceRequestKey::PkFont(request) => {
                    let outcome = if let Some(font) = self.resolved_pk_fonts.get(request) {
                        PdfFontClosureResourceOutcome::Resolved {
                            virtual_path: font.virtual_path.clone(),
                            bytes: font.bytes.len(),
                            sha256: Sha256::digest(&font.bytes).into(),
                        }
                    } else if self.unavailable_pk_font_keys().contains(request) {
                        PdfFontClosureResourceOutcome::Unavailable
                    } else {
                        return Err(CompileError::Incremental(format!(
                            "accepted PDF font closure has an unbound PK request {}",
                            String::from_utf8_lossy(&request.logical_name())
                        )));
                    };
                    PdfFontClosureReceiptEntry::PkFont {
                        request: request.clone(),
                        outcome,
                    }
                }
                ResourceRequestKey::Font(_) => continue,
            };
            entries.push(entry);
        }
        Ok(PdfFontClosureReceipt { entries })
    }

    pub fn add_user_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), CompileError> {
        if self.accepted_output.is_some() {
            return Err(CompileError::SessionAlreadyStarted);
        }
        let path = VirtualPath::user(path).map_err(|error| CompileError::InvalidVirtualPath {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        self.workspace
            .register_user(path.clone(), bytes.clone())
            .map_err(map_user_registration)?;
        if let Some(session) = &mut self.incremental {
            session
                .register_input_file(path.as_path(), bytes)
                .map_err(|error| CompileError::Incremental(error.to_string()))?;
        }
        Ok(())
    }

    pub fn apply_patch(&mut self, patch: SourcePatch) -> Result<(), CompileError> {
        if self.pending_patch.is_some() {
            return Err(CompileError::PatchAlreadyPending);
        }
        let session = self.incremental.as_ref().ok_or_else(|| {
            CompileError::Incremental("the initial revision has not been accepted".to_owned())
        })?;
        let edit = tex_incr::Edit {
            base_revision: patch.base_revision,
            expected_hash: patch.expected_hash,
            range: patch.range,
            replacement: patch.replacement,
        };
        session
            .validate_edit(patch.next_revision, &edit)
            .map_err(|error| CompileError::Incremental(error.to_string()))?;
        self.pending_patch = Some((patch.next_revision, edit));
        self.candidate = None;
        self.awaiting = None;
        self.attempts = 0;
        self.attempts_without_progress = 0;
        Ok(())
    }

    /// Discards an unaccepted editor revision while retaining the last
    /// accepted revision and all immutable resource bindings.
    pub fn cancel_pending_patch(&mut self) -> bool {
        let cancelled = self.pending_patch.take().is_some();
        if cancelled {
            self.candidate = None;
            self.awaiting = None;
            self.attempts = 0;
            self.attempts_without_progress = 0;
        }
        cancelled
    }

    /// Drops the currently executing candidate without changing the requested
    /// revision. Hosts use this when an in-flight operation is cancelled; a
    /// later attempt starts that revision again from a fresh candidate.
    pub fn discard_suspended_candidate(&mut self) -> bool {
        let discarded = self.candidate.take().is_some();
        if discarded {
            self.awaiting = None;
            self.attempts_without_progress = 0;
        }
        discarded
    }

    #[must_use]
    pub fn revision(&self) -> Option<tex_incr::RevisionId> {
        self.incremental
            .as_ref()
            .filter(|_| self.accepted_output.is_some() || self.pending_patch.is_some())
            .map(|session| session.revision())
    }

    #[must_use]
    pub fn content_hash(&self) -> Option<ContentHash> {
        self.revision().and_then(|_| {
            self.incremental
                .as_ref()
                .map(|session| session.content_hash())
        })
    }

    #[must_use]
    pub fn rendered_output_id(&self) -> Option<tex_incr::RenderedOutputId> {
        self.revision()
            .and_then(|_| self.incremental.as_ref().map(|session| session.output_id()))
    }

    #[must_use]
    pub const fn render_update(&self) -> Option<&RenderUpdate> {
        self.pending_render_update.as_ref()
    }

    /// Acknowledges exactly the delivered render target. Stale or conflicting
    /// acknowledgements never retire the pending resource/revision base.
    pub fn acknowledge_render_update(
        &mut self,
        revision: u64,
        digest: RenderDigest,
    ) -> Result<(), CompileError> {
        let Some(update) = &self.pending_render_update else {
            return Err(CompileError::Incremental(
                "there is no pending HTML render update".to_owned(),
            ));
        };
        if update.target_revision() != revision || update.target_digest() != digest {
            return Err(CompileError::Incremental(
                "HTML render acknowledgement does not match the delivered target".to_owned(),
            ));
        }
        self.pending_render_update = None;
        Ok(())
    }

    /// Returns a bounded full snapshot for explicit protocol recovery.
    #[must_use]
    pub fn render_resync(&self) -> Option<RenderUpdate> {
        self.accepted_render_document
            .as_ref()
            .cloned()
            .map(RenderUpdate::Snapshot)
    }

    /// Resolves one HTML page/event/unit against the currently accepted output.
    pub fn rendered_source_location(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
        output_id: tex_incr::RenderedOutputId,
        revision: tex_incr::RevisionId,
    ) -> Result<Option<RenderedSourceResult>, CompileError> {
        if self.accepted_output.is_none() || self.pending_patch.is_some() {
            return Ok(None);
        }
        let Some(session) = self.incremental.as_ref() else {
            return Ok(None);
        };
        let accepted = self
            .accepted_engine_output
            .as_ref()
            .expect("accepted output checked above");
        session
            .rendered_source_location(accepted, page, event, unit, output_id, revision)
            .map(|location| {
                location.map(|result| match result {
                    tex_incr::RenderedSourceResult::Current(location) => {
                        RenderedSourceResult::Current(RenderedSourceLocation {
                            revision,
                            path: location.path,
                            start: location.start,
                            end: location.end,
                            line: location.line,
                            column: location.column,
                        })
                    }
                    tex_incr::RenderedSourceResult::Deleted { minted_revision } => {
                        RenderedSourceResult::Deleted { minted_revision }
                    }
                    tex_incr::RenderedSourceResult::StaleRevision { accepted } => {
                        RenderedSourceResult::StaleRevision { accepted }
                    }
                    tex_incr::RenderedSourceResult::OutputMismatch { accepted } => {
                        RenderedSourceResult::OutputMismatch { accepted }
                    }
                })
            })
            .map_err(|error| CompileError::Incremental(error.to_string()))
    }

    #[must_use]
    pub const fn reuse_metrics(&self) -> Option<tex_incr::ReuseMetrics> {
        self.last_reuse
    }

    #[must_use]
    pub const fn stabilization_required(&self) -> bool {
        self.last_stabilization_required
    }

    pub(crate) fn mark_stable(&mut self) {
        self.last_stabilization_required = false;
    }

    pub(crate) fn accepted_generated_fingerprint(
        &self,
    ) -> Result<Vec<(VirtualPath, ContentHash)>, CompileError> {
        generated_fingerprint(&self.workspace)
    }

    #[must_use]
    pub fn retention_metrics(&self) -> Option<RetentionMetrics> {
        let accepted = self
            .incremental
            .as_ref()
            .and_then(|session| session.retention_metrics());
        let candidate = self
            .candidate
            .as_ref()
            .map(|candidate| candidate.engine_retention());
        if accepted.is_none() && candidate.is_none() {
            return None;
        }
        let accepted = accepted.unwrap_or_default();
        let candidate = candidate.unwrap_or_default();
        let vfs = self
            .candidate
            .as_ref()
            .map_or_else(
                || self.workspace.snapshot(),
                |candidate| candidate.workspace.snapshot(),
            )
            .retention();
        let returned_output = self
            .accepted_output
            .as_ref()
            .map_or(0, memory_run_output_bytes);
        Some(RetentionMetrics {
            checkpoint_root_bytes: accepted
                .checkpoint_root_bytes
                .saturating_add(candidate.checkpoint_root_bytes),
            diagnostic_bytes: accepted
                .diagnostic_bytes
                .saturating_add(candidate.diagnostic_bytes),
            output_bytes: accepted
                .output_bytes
                .saturating_add(candidate.output_bytes)
                .saturating_add(returned_output)
                .saturating_add(vfs.generated_bytes),
            resource_bytes: vfs.input_bytes,
            protected_overage_bytes: accepted
                .protected_overage_bytes
                .saturating_add(candidate.protected_overage_bytes),
        })
    }

    /// Enumerates the semantic input dependencies of the accepted revision.
    /// Suspended and discarded candidate observations are never visible here.
    pub fn accepted_input_dependencies(&self) -> impl Iterator<Item = &tex_state::InputDependency> {
        self.incremental
            .iter()
            .filter(|_| self.accepted_output.is_some())
            .flat_map(|session| session.accepted_input_dependencies())
    }

    /// Returns the versioned public projection of the accepted revision's
    /// semantic input dependencies. Candidate-only observations are excluded.
    #[must_use]
    pub fn accepted_input_observations(&self) -> Option<crate::AcceptedInputObservationLedger> {
        let revision = self.revision()?;
        let dependencies = self.accepted_input_dependency_values();
        let observations = crate::input_observation::tex_observations(
            dependencies.into_iter(),
            &self.workspace.snapshot(),
            revision,
            None,
        );
        Some(crate::AcceptedInputObservationLedger::new(
            revision,
            observations,
        ))
    }

    pub(crate) fn accepted_input_dependency_values(
        &self,
    ) -> Vec<(
        VirtualPath,
        tex_state::InputDependencyOutcome,
        tex_state::InputDependencyAccess,
    )> {
        let mut dependencies = self
            .accepted_input_dependencies()
            .filter_map(|dependency| {
                crate::input_observation::virtual_path(dependency.path())
                    .map(|path| (path, (dependency.outcome(), dependency.access())))
            })
            .collect::<BTreeMap<_, _>>();
        if self.accepted_output.is_some()
            && let Ok(Some(root)) = self.workspace.snapshot().get(&self.main_path)
        {
            dependencies.insert(
                self.main_path.clone(),
                (
                    tex_state::InputDependencyOutcome::Present(ContentHash::from_bytes(
                        root.bytes(),
                    )),
                    tex_state::InputDependencyAccess::RequiredRead,
                ),
            );
        }
        dependencies
            .into_iter()
            .map(|(path, (outcome, access))| (path, outcome, access))
            .collect()
    }

    pub(crate) fn restore_cached_file(
        &mut self,
        request: FileRequestKey,
        virtual_path: &str,
        bytes: Vec<u8>,
    ) -> Result<(), CompileError> {
        let key = ResourceRequestKey::File(request.clone());
        let was_bound = self.resource_is_bound(&key);
        self.provide_file_inner(
            ResolvedFile {
                request,
                virtual_path: virtual_path.to_owned(),
                bytes,
                expected_digest: None,
            },
            false,
            true,
        )?;
        if self
            .awaiting
            .as_ref()
            .is_some_and(|awaiting| awaiting.contains(&key))
            && !was_bound
            && self.resource_is_bound(&key)
        {
            self.response_generation = self.response_generation.saturating_add(1);
            self.finish_resource_wait();
        }
        self.refresh_candidate_files()
    }

    #[cfg(test)]
    pub(crate) fn provide_resolved_file(
        &mut self,
        request: FileRequestKey,
        virtual_path: &str,
        bytes: Vec<u8>,
    ) -> Result<(), CompileError> {
        self.restore_cached_file(request, virtual_path, bytes)
    }

    pub fn provide_resources(
        &mut self,
        responses: Vec<ResourceResponse>,
    ) -> Result<(), CompileError> {
        let awaited_before = self.awaiting.as_ref().map(|awaiting| {
            awaiting
                .iter()
                .filter(|key| self.resource_is_bound(key))
                .count()
        });
        let mut staged_workspace = self.workspace.clone();
        let mut staged_fonts = self.resolved_fonts.clone();
        let mut staged_admission = self.non_file_admission.clone();
        let mut staged_pk_fonts = self.resolved_pk_fonts.clone();
        let original_workspace = std::mem::replace(&mut self.workspace, staged_workspace);
        let original_fonts = std::mem::replace(&mut self.resolved_fonts, staged_fonts);
        let original_admission = std::mem::replace(&mut self.non_file_admission, staged_admission);
        let original_pk_fonts = std::mem::replace(&mut self.resolved_pk_fonts, staged_pk_fonts);
        let original_font_cached_bytes = self.font_cached_bytes;
        let result = responses
            .into_iter()
            .try_for_each(|response| match response {
                ResourceResponse::File(file) => self.provide_file_inner(file, true, false),
                ResourceResponse::FileUnavailable(request) => self
                    .workspace
                    .provision_unavailable(request)
                    .map(|_| ())
                    .map_err(map_provision),
                ResourceResponse::Font(font) => self.provide_resolved_font_inner(font),
                ResourceResponse::FontUnavailable(request) => {
                    self.provide_unavailable_font(request)
                }
                ResourceResponse::PkFont(font) => self.provide_resolved_pk_font_inner(font),
                ResourceResponse::PkFontUnavailable(request) => {
                    self.provide_unavailable_pk_font(request)
                }
            });
        if result.is_err() {
            staged_workspace = std::mem::replace(&mut self.workspace, original_workspace);
            staged_fonts = std::mem::replace(&mut self.resolved_fonts, original_fonts);
            staged_admission = std::mem::replace(&mut self.non_file_admission, original_admission);
            staged_pk_fonts = std::mem::replace(&mut self.resolved_pk_fonts, original_pk_fonts);
            drop((
                staged_workspace,
                staged_fonts,
                staged_admission,
                staged_pk_fonts,
            ));
            self.font_cached_bytes = original_font_cached_bytes;
        } else {
            if let Some(session) = &mut self.incremental {
                for (request, file) in self.workspace.files() {
                    if original_workspace.get(request).is_none() {
                        session
                            .register_input_file(file.path().as_path(), file.bytes().to_vec())
                            .map_err(|error| CompileError::Incremental(error.to_string()))?;
                    }
                }
            }
            let awaited_after = self.awaiting.as_ref().map(|awaiting| {
                awaiting
                    .iter()
                    .filter(|key| self.resource_is_bound(key))
                    .count()
            });
            if awaited_before
                .zip(awaited_after)
                .is_some_and(|(before, after)| after > before)
            {
                self.response_generation = self.response_generation.saturating_add(1);
                self.finish_resource_wait();
            }
            self.refresh_candidate_files()?;
        }
        result
    }

    fn refresh_candidate_files(&mut self) -> Result<(), CompileError> {
        if self.candidate.is_none() {
            return Ok(());
        }
        let mut refreshed = self.workspace.clone();
        if let (Some((_, edit)), Some(session)) = (&self.pending_patch, self.incremental.as_ref()) {
            let mut source = session.source().to_owned();
            source.replace_range(edit.range.clone(), &edit.replacement);
            let source = session.source_file_bytes(&source);
            refreshed
                .register_user(self.main_path.clone(), source)
                .map_err(map_user_registration)?;
        }
        let candidate = self
            .candidate
            .as_mut()
            .expect("candidate presence was checked");
        if let RetainedExecution::Initial { session, .. } = &mut candidate.execution {
            register_incremental_inputs(session, &refreshed, &self.main_path)?;
        }
        candidate.workspace = refreshed;
        Ok(())
    }

    fn provide_resolved_font_inner(&mut self, response: ResolvedFont) -> Result<(), CompileError> {
        let key = response.request.clone();
        let request = self.font_requests.get(&key).ok_or_else(|| {
            CompileError::UnexpectedResourceResponse(key.logical_name().to_owned())
        })?;
        check_limit(
            "one font resource bytes",
            response.bytes.len(),
            self.limits.one_file_bytes,
        )?;
        let fingerprint = FontResponseFingerprint {
            container: response.container,
            object: tex_fonts::FontObjectIdentity::for_bytes(&response.bytes),
            declared_object: response.declared_object_sha256,
            declared_program: response.declared_program_identity,
            provenance: response.provenance.clone(),
            legacy_mapping: response.legacy_mapping.clone(),
        };
        if let Some(NonFileAdmission::Font(existing)) = self
            .non_file_admission
            .admitted(&NonFileResourceKey::Font(key.clone()))
        {
            if existing == &fingerprint {
                return Ok(());
            }
            return Err(CompileError::ConflictingResolvedBinding(
                key.logical_name().to_owned(),
            ));
        }
        let shared = self
            .resolved_fonts
            .values()
            .find(|font| font.object_identity == fingerprint.object);
        let shares_object = shared.is_some();
        let font = OpenTypeFont::parse_reusing(request, response, FontLimits::default(), shared)
            .map_err(|error| CompileError::Font(error.to_string()))?;
        if self.outputs.contains(OutputCapability::Html)
            && fingerprint.provenance.as_deref().is_none_or(str::is_empty)
        {
            return Err(CompileError::Font(format!(
                "font {} has no embedding provenance",
                key.logical_name()
            )));
        }
        if let Some(mapping) = &fingerprint.legacy_mapping {
            if self.outputs.contains(OutputCapability::Html) && !mapping.embeddable {
                return Err(CompileError::Font(format!(
                    "font {} is not licensed for embedding",
                    key.logical_name()
                )));
            }
            tex_fonts::LegacyEncodingMap::new(mapping.encoding.clone())
                .map_err(|message| CompileError::Font(message.to_owned()))?;
            for text in mapping.encoding.iter().flatten() {
                if text.chars().any(|scalar| font.cmap.glyph(scalar).is_none()) {
                    return Err(CompileError::Font(format!(
                        "font {} mapping contains a scalar absent from its cmap",
                        key.logical_name()
                    )));
                }
            }
        }
        let transport_bytes = if shares_object {
            0
        } else {
            font.transport_bytes.len()
        };
        let metadata_bytes = fingerprint
            .provenance
            .as_ref()
            .map_or(0, String::len)
            .checked_add(fingerprint.legacy_mapping.as_ref().map_or(0, |mapping| {
                32usize.saturating_add(1).saturating_add(
                    mapping
                        .encoding
                        .iter()
                        .flatten()
                        .map(String::len)
                        .sum::<usize>(),
                )
            }))
            .ok_or(CompileError::LimitExceeded {
                resource: "cached resource bytes",
                limit: self.limits.cached_file_bytes,
                attempted: usize::MAX,
            })?;
        let additional_bytes =
            transport_bytes
                .checked_add(metadata_bytes)
                .ok_or(CompileError::LimitExceeded {
                    resource: "cached resource bytes",
                    limit: self.limits.cached_file_bytes,
                    attempted: usize::MAX,
                })?;
        let attempted = self
            .cached_file_bytes()
            .checked_add(additional_bytes)
            .ok_or(CompileError::LimitExceeded {
                resource: "cached resource bytes",
                limit: self.limits.cached_file_bytes,
                attempted: usize::MAX,
            })?;
        check_limit(
            "cached resource bytes",
            attempted,
            self.limits.cached_file_bytes,
        )?;
        self.non_file_admission
            .admit(
                NonFileResourceKey::Font(key.clone()),
                NonFileAdmission::Font(fingerprint.clone()),
            )
            .map_err(|error| map_non_file_admission(error, key.logical_name()))?;
        self.resolved_fonts.insert(key.clone(), font);
        self.font_cached_bytes = self
            .font_cached_bytes
            .checked_add(additional_bytes)
            .expect("combined cache limit checked overflow");
        Ok(())
    }

    fn provide_unavailable_font(&mut self, key: FontRequestKey) -> Result<(), CompileError> {
        let name = key.logical_name().to_owned();
        self.non_file_admission
            .admit_unavailable(NonFileResourceKey::Font(key))
            .map(|_| ())
            .map_err(|error| map_non_file_admission(error, &name))
    }

    fn provide_resolved_pk_font_inner(
        &mut self,
        response: ResolvedPkFont,
    ) -> Result<(), CompileError> {
        VirtualPath::distribution(&response.virtual_path).map_err(|error| {
            CompileError::InvalidVirtualPath {
                path: response.virtual_path.clone(),
                message: error.to_string(),
            }
        })?;
        check_limit(
            "one PK font resource bytes",
            response.bytes.len(),
            self.limits.one_file_bytes,
        )?;
        let digest: [u8; 32] = Sha256::digest(&response.bytes).into();
        if response
            .expected_sha256
            .is_some_and(|expected| expected != digest)
        {
            return Err(CompileError::Font(format!(
                "PK font {} content digest does not match",
                String::from_utf8_lossy(&response.request.logical_name())
            )));
        }
        tex_fonts::PdfPkFont::parse(&response.bytes)
            .map_err(|error| CompileError::Font(error.to_string()))?;
        if let Some(existing) = self.resolved_pk_fonts.get(&response.request) {
            if existing == &response {
                return Ok(());
            }
            return Err(CompileError::ConflictingResolvedBinding(format!(
                "PK font {}",
                String::from_utf8_lossy(&response.request.logical_name())
            )));
        }
        let attempted = self
            .cached_file_bytes()
            .checked_add(response.bytes.len())
            .ok_or(CompileError::LimitExceeded {
                resource: "cached resource bytes",
                limit: self.limits.cached_file_bytes,
                attempted: usize::MAX,
            })?;
        check_limit(
            "cached resource bytes",
            attempted,
            self.limits.cached_file_bytes,
        )?;
        self.font_cached_bytes = self
            .font_cached_bytes
            .checked_add(response.bytes.len())
            .expect("combined cache limit checked overflow");
        let request = response.request.clone();
        let name = format!(
            "PK font {}",
            String::from_utf8_lossy(&request.logical_name())
        );
        self.non_file_admission
            .admit(
                NonFileResourceKey::PkFont(request.clone()),
                NonFileAdmission::PkFont {
                    path: response.virtual_path.clone(),
                    sha256: digest,
                },
            )
            .map_err(|error| map_non_file_admission(error, &name))?;
        self.resolved_pk_fonts.insert(request, response);
        Ok(())
    }

    fn provide_unavailable_pk_font(
        &mut self,
        request: PdfPkFontRequest,
    ) -> Result<(), CompileError> {
        let name = format!(
            "PK font {}",
            String::from_utf8_lossy(&request.logical_name())
        );
        self.non_file_admission
            .admit_unavailable(NonFileResourceKey::PkFont(request))
            .map(|_| ())
            .map_err(|error| map_non_file_admission(error, &name))
    }

    fn provide_file_inner(
        &mut self,
        response: ResolvedFile,
        require_expected: bool,
        register_incremental: bool,
    ) -> Result<(), CompileError> {
        let request = response.request.clone();
        let mut staged = self.workspace.clone();
        let outcome = if require_expected {
            staged.provision(response)
        } else {
            staged.preload(response)
        }
        .map_err(map_provision)?;
        let attempted = staged
            .resolved_bytes()
            .checked_add(self.font_cached_bytes)
            .ok_or(CompileError::LimitExceeded {
                resource: "cached resource bytes",
                limit: self.limits.cached_file_bytes,
                attempted: usize::MAX,
            })?;
        check_limit(
            "cached resource bytes",
            attempted,
            self.limits.cached_file_bytes,
        )?;
        self.workspace = staged;
        if register_incremental
            && outcome == ProvisionOutcome::Inserted
            && let (Some(session), Some(file)) =
                (&mut self.incremental, self.workspace.get(&request))
        {
            session
                .register_input_file(file.path().as_path(), file.bytes().to_vec())
                .map_err(|error| CompileError::Incremental(error.to_string()))?;
        }
        Ok(())
    }

    pub fn compile_attempt(&mut self) -> CompileAttemptResult {
        if self.pending_patch.is_none()
            && let Some(output) = &self.accepted_output
        {
            return CompileAttemptResult::Complete(output.clone());
        }
        if let Some(awaiting) = &self.awaiting {
            let progressed = self.candidate.as_ref().map_or_else(
                || awaiting.iter().any(|key| self.resource_is_bound(key)),
                |candidate| self.response_generation > candidate.response_generation,
            );
            if !progressed {
                self.candidate = None;
                if self.accepted_output.is_some() {
                    self.pending_patch = None;
                }
                return CompileAttemptResult::Error(CompileError::NoProgress);
            }
            self.attempts_without_progress = 0;
        }
        if self.attempts_without_progress >= self.limits.attempts {
            self.candidate = None;
            if self.accepted_output.is_some() {
                self.pending_patch = None;
            }
            return CompileAttemptResult::Error(CompileError::AttemptLimit {
                limit: self.limits.attempts,
            });
        }
        self.awaiting = None;
        self.attempts += 1;
        self.attempts_without_progress += 1;
        match self.run_attempt() {
            Ok(result) => result,
            Err(error) => {
                self.candidate = None;
                if self.accepted_output.is_some() {
                    self.pending_patch = None;
                }
                CompileAttemptResult::Error(error)
            }
        }
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn run_attempt(&mut self) -> Result<CompileAttemptResult, CompileError> {
        #[cfg(not(target_arch = "wasm32"))]
        let candidate_restore_started = Instant::now();
        let existing_candidate = self.candidate.take();
        let mut pending_workspace = existing_candidate.as_ref().map_or_else(
            || self.workspace.clone(),
            |candidate| candidate.workspace.clone(),
        );
        if existing_candidate.is_none()
            && let (Some(session), Some((_, edit))) =
                (self.incremental.as_ref(), self.pending_patch.as_ref())
        {
            let mut source = session.source().to_owned();
            source.replace_range(edit.range.clone(), &edit.replacement);
            pending_workspace
                .register_user(self.main_path.clone(), session.source_file_bytes(&source))
                .map_err(map_user_registration)?;
        }

        #[cfg(not(target_arch = "wasm32"))]
        let resolver_index_started = Instant::now();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.resolver_index_time = self
                .resolver_index_time
                .saturating_add(resolver_index_started.elapsed());
        }
        #[cfg(not(target_arch = "wasm32"))]
        let vfs_stage_started = Instant::now();
        let candidate_workspace = pending_workspace.clone();
        let (resource_ledger, mut generated_transaction) =
            pending_workspace.begin_generated_with_ledger();
        let snapshot = generated_transaction.snapshot();

        let mut retained = if let Some(candidate) = existing_candidate {
            candidate
        } else if self.incremental.is_none() {
            let source = snapshot
                .get(&self.main_path)
                .map_err(|error| CompileError::World(error.to_string()))?
                .ok_or_else(|| CompileError::MissingMainFile(self.main_path.to_string()))?;
            let source = source.bytes().to_vec();
            let format_image = if let Some(format) = &self.format {
                let image = tex_state::DetachedFormatImage::try_from_bytes(format.clone())
                    .map_err(|error| CompileError::Format(error.to_string()))?;
                Some(image)
            } else {
                None
            };
            let session = Box::new({
                let mut session = tex_incr::Session::start_with_source_bytes(
                    (),
                    &self.job_name,
                    self.main_path.as_str(),
                    self.initial_revision,
                    source,
                    self.limits.cached_file_bytes,
                )
                .map_err(|error| CompileError::Incremental(error.to_string()))?;
                session.set_job_clock(self.clock);
                session.set_command_profile(self.engine.command_profile(), self.format.is_none());
                if let Some(image) = format_image {
                    session.set_format_image(image);
                }
                session.set_utf8_input_as_bytes(self.engine.uses_latex_input());
                session.set_dvi_output(self.outputs.contains(OutputCapability::Dvi));
                session.set_render_cache_budget(self.limits.output_bytes);
                let startup_name = self
                    .main_path
                    .as_str()
                    .rsplit('/')
                    .next()
                    .expect("a canonical virtual path has a final component");
                session.set_root_source_framing_name(format!("./{startup_name}"));
                if self.authored_root_name.is_some() {
                    session
                        .set_root_source_framing(tex_command::SourceFramingPolicy::ExternallyOwned);
                }
                session
            });
            let mut session = session;
            register_incremental_inputs(&mut session, &candidate_workspace, &self.main_path)?;
            let mut candidate = session
                .start_cold_candidate()
                .map_err(|error| CompileError::Incremental(error.to_string()))?;
            candidate.set_cumulative_fuel_limit(self.limits.engine_fuel);
            candidate.set_execution_budgets(self.execution_budgets());
            candidate.set_provenance_config(
                tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
                tex_state::ProvenanceBudgets::default(),
            );
            Box::new(RetainedCandidate {
                workspace: candidate_workspace.clone(),
                execution: RetainedExecution::Initial { session, candidate },
                response_generation: self.response_generation,
                suspension_serial: 0,
            })
        } else if let Some(session) = self.incremental.as_ref() {
            let (next_revision, edit) = self
                .pending_patch
                .as_ref()
                .expect("accepted sessions execute only pending patches");
            let dependencies_match = accepted_dependencies_match_snapshot(
                session.accepted_input_dependencies(),
                &snapshot,
                &self.main_path,
            )?;
            let mut candidate = if dependencies_match {
                session.start_advance_candidate(*next_revision, edit.clone())
            } else {
                session.start_advance_candidate_from_job_start(*next_revision, edit.clone())
            }
            .map_err(|error| CompileError::Incremental(error.to_string()))?;
            candidate.set_cumulative_fuel_limit(self.limits.engine_fuel);
            candidate.set_execution_budgets(self.execution_budgets());
            candidate.set_provenance_config(
                tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
                tex_state::ProvenanceBudgets::default(),
            );
            Box::new(RetainedCandidate {
                workspace: candidate_workspace,
                execution: RetainedExecution::Pending(candidate),
                response_generation: self.response_generation,
                suspension_serial: 0,
            })
        } else {
            unreachable!("candidate creation covers initial and accepted sessions")
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.candidate_restore_time = self
                .candidate_restore_time
                .saturating_add(candidate_restore_started.elapsed());
        }
        let unavailable_fonts = self.unavailable_font_keys();
        let font_responses = self.font_response_fingerprints();
        let mut resolvers = VirtualRunResolvers::new(
            &snapshot,
            resource_ledger,
            &self.resolved_fonts,
            &unavailable_fonts,
            FontResolutionPolicy {
                accepted_containers: self.accepted_font_containers,
                layout: self.font_layout_policy,
                fallback: self.font_mapping_fallback,
                font_responses: &font_responses,
            },
        );
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.vfs_stage_time = self
                .vfs_stage_time
                .saturating_add(vfs_stage_started.elapsed());
        }
        let cancellation = tex_exec::Cancellation::new();
        let drive = match &mut retained.execution {
            RetainedExecution::Initial { candidate, .. }
            | RetainedExecution::Pending(candidate) => {
                candidate.drive_with_resource_resolvers(&mut resolvers, &cancellation)
            }
        };
        self.execution_telemetry = match &retained.execution {
            RetainedExecution::Initial { candidate, .. }
            | RetainedExecution::Pending(candidate) => candidate.execution_telemetry(),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let request_extraction_started = Instant::now();
        let (file_misses, file_probes, font_misses, fatal) = resolvers.finish();

        if !file_misses.is_empty() || !file_probes.is_empty() || !font_misses.is_empty() {
            let _suspension = match &drive {
                Ok(tex_incr::RevisionCandidateResult::AwaitingResources(suspension)) => suspension,
                Ok(tex_incr::RevisionCandidateResult::Complete) => {
                    return Err(CompileError::NoProgress);
                }
                Err(error) => {
                    return Err(CompileError::Diagnostic(
                        CompileDiagnostic::from_session_error(error, Some(&retained.execution)),
                    ));
                }
            };
            generated_transaction.discard();
            for request in &font_misses {
                self.font_requests
                    .entry(request.key.clone())
                    .or_insert_with(|| request.clone());
            }
            let mut required = file_misses
                .into_iter()
                .map(ResourceRequest::File)
                .chain(font_misses.into_iter().map(ResourceRequest::Font))
                .collect::<Vec<_>>();
            required.sort_by_key(resource_sort_key);
            required.dedup();
            let required_keys = required
                .iter()
                .map(resource_request_key)
                .collect::<BTreeSet<_>>();
            let mut probes = file_probes
                .into_iter()
                .map(ResourceRequest::File)
                .filter(|request| !required_keys.contains(&resource_request_key(request)))
                .collect::<Vec<_>>();
            probes.sort_by_key(resource_sort_key);
            probes.dedup();
            let awaiting = required
                .iter()
                .chain(&probes)
                .map(resource_request_key)
                .collect::<BTreeSet<_>>();
            if awaiting.iter().any(|key| self.resource_is_bound(key)) {
                return Err(CompileError::NoProgress);
            }
            self.awaiting = Some(awaiting);
            let prefetch_hints = if let Some(hints) = self.initial_prefetch_hints.take() {
                let required_keys = required
                    .iter()
                    .chain(&probes)
                    .map(|request| match request {
                        ResourceRequest::File(request) => {
                            ResourceRequestKey::File(request.key().clone())
                        }
                        ResourceRequest::Font(request) => {
                            ResourceRequestKey::Font(request.key.clone())
                        }
                        ResourceRequest::PkFont(request) => {
                            ResourceRequestKey::PkFont(request.clone())
                        }
                    })
                    .collect::<BTreeSet<_>>();
                hints
                    .into_vec()
                    .into_iter()
                    .filter(|request| {
                        let key = match request {
                            ResourceRequest::File(request) => {
                                if self.workspace.get(request.key()).is_some()
                                    || self.workspace.is_unavailable(request.key())
                                    || user_path_for_key(request.key())
                                        .is_ok_and(|path| self.workspace.contains_user(&path))
                                {
                                    return false;
                                }
                                ResourceRequestKey::File(request.key().clone())
                            }
                            ResourceRequest::Font(request) => {
                                if self.resolved_fonts.contains_key(&request.key)
                                    || self.font_is_unavailable(&request.key)
                                {
                                    return false;
                                }
                                ResourceRequestKey::Font(request.key.clone())
                            }
                            ResourceRequest::PkFont(request) => {
                                if self.resolved_pk_fonts.contains_key(request)
                                    || self.pk_font_is_unavailable(request)
                                {
                                    return false;
                                }
                                ResourceRequestKey::PkFont(request.clone())
                            }
                        };
                        !required_keys.contains(&key)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            check_resource_batch_limit(
                &required,
                &probes,
                &prefetch_hints,
                self.limits.resolved_files,
            )?;
            self.workspace.expect(&FileRequestBatch::with_probes(
                required.iter().filter_map(|request| match request {
                    ResourceRequest::File(request) => Some(request.clone()),
                    ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                }),
                probes.iter().filter_map(|request| match request {
                    ResourceRequest::File(request) => Some(request.clone()),
                    ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                }),
                prefetch_hints.iter().filter_map(|request| match request {
                    ResourceRequest::File(request) => Some(request.clone()),
                    ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                }),
            ));
            self.begin_non_file_batch(&required, &probes, &prefetch_hints);
            retained.workspace = pending_workspace;
            retained.response_generation = self.response_generation;
            retained.suspension_serial = match &retained.execution {
                RetainedExecution::Initial { candidate, .. }
                | RetainedExecution::Pending(candidate) => candidate.suspension_serial(),
            };
            self.candidate = Some(retained);
            self.start_resource_wait();
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.request_extraction_time = self
                    .request_extraction_time
                    .saturating_add(request_extraction_started.elapsed());
            }
            return Ok(CompileAttemptResult::NeedResources(NeedResources {
                required,
                probes,
                prefetch_hints,
            }));
        }
        if let Some(fatal) = fatal {
            generated_transaction.discard();
            return Err(fatal);
        }
        match drive {
            Ok(tex_incr::RevisionCandidateResult::Complete) => {}
            Ok(tex_incr::RevisionCandidateResult::AwaitingResources(_)) => {
                generated_transaction.discard();
                return Err(CompileError::NoProgress);
            }
            Err(error) => {
                let diagnostic =
                    CompileDiagnostic::from_session_error(&error, Some(&retained.execution));
                generated_transaction.discard();
                return Err(CompileError::Diagnostic(diagnostic));
            }
        }
        if self.outputs.contains(OutputCapability::Pdf) {
            #[cfg(not(target_arch = "wasm32"))]
            let pdf_request_extraction_started = Instant::now();
            let completion = match &retained.execution {
                RetainedExecution::Initial { candidate, .. }
                | RetainedExecution::Pending(candidate) => candidate
                    .completion_resource_discovery()
                    .expect("a completed drive exposes detached resource discovery"),
            };
            let unavailable_pk_fonts = self.unavailable_pk_font_keys();
            let raw_object_files = discover_pdf_raw_object_files(completion.pdf(), &self.workspace)
                .map_err(|message| CompileError::OutputCapability {
                    capability: OutputCapability::Pdf,
                    message,
                })?;
            let mut discovery = pdf_resources::discover(
                completion,
                &self.workspace,
                &mut self.virtual_font_resources,
                &self.resolved_pk_fonts,
                &unavailable_pk_fonts,
            )
            .map_err(|message| CompileError::OutputCapability {
                capability: OutputCapability::Pdf,
                message,
            })?;
            discovery
                .required
                .extend(raw_object_files.into_iter().map(ResourceRequest::File));
            discovery.required.sort_by_key(resource_sort_key);
            discovery
                .required
                .dedup_by(|left, right| resource_request_key(left) == resource_request_key(right));
            self.pdf_font_closure_requests.extend(
                discovery
                    .observed_files
                    .iter()
                    .map(|request| ResourceRequestKey::File(request.key().clone())),
            );
            self.pdf_font_closure_requests.extend(
                discovery
                    .observed_pk_fonts
                    .iter()
                    .cloned()
                    .map(ResourceRequestKey::PkFont),
            );
            if !discovery.required.is_empty() || !discovery.probes.is_empty() {
                generated_transaction.discard();
                let required = discovery.required;
                let probes = discovery.probes;
                check_resource_batch_limit(&required, &probes, &[], self.limits.resolved_files)?;
                let awaiting = required
                    .iter()
                    .chain(&probes)
                    .map(resource_request_key)
                    .collect::<BTreeSet<_>>();
                if awaiting.iter().any(|key| self.resource_is_bound(key)) {
                    return Err(CompileError::NoProgress);
                }
                self.awaiting = Some(awaiting);
                self.workspace.expect(&FileRequestBatch::with_probes(
                    required.iter().filter_map(|request| match request {
                        ResourceRequest::File(request) => Some(request.clone()),
                        ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                    }),
                    probes.iter().filter_map(|request| match request {
                        ResourceRequest::File(request) => Some(request.clone()),
                        ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                    }),
                    std::iter::empty(),
                ));
                self.begin_non_file_batch(&required, &probes, &[]);
                retained.workspace = pending_workspace;
                retained.response_generation = self.response_generation;
                retained.suspension_serial = retained.suspension_serial.saturating_add(1);
                self.candidate = Some(retained);
                self.start_resource_wait();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.request_extraction_time = self
                        .request_extraction_time
                        .saturating_add(pdf_request_extraction_started.elapsed());
                }
                return Ok(CompileAttemptResult::NeedResources(NeedResources {
                    required,
                    probes,
                    prefetch_hints: Vec::new(),
                }));
            }
        }
        if self.outputs.contains(OutputCapability::Html) {
            #[cfg(not(target_arch = "wasm32"))]
            let html_request_extraction_started = Instant::now();
            let pages = match &retained.execution {
                RetainedExecution::Initial { candidate, .. }
                | RetainedExecution::Pending(candidate) => candidate
                    .completion_resource_discovery()
                    .expect("a completed drive exposes detached resource discovery")
                    .completion()
                    .pages(),
            };
            let unavailable_fonts = self.unavailable_font_keys();
            let required = discover_html_paint_resources(
                pages.iter().map(tex_exec::DetachedPreparedPage::artifact),
                &self.resolved_fonts,
                &unavailable_fonts,
                self.accepted_font_containers,
            )?;
            if !required.is_empty() {
                generated_transaction.discard();
                for request in &required {
                    let ResourceRequest::Font(request) = request else {
                        unreachable!("HTML paint discovery emits only font resources")
                    };
                    self.font_requests
                        .entry(request.key.clone())
                        .or_insert_with(|| request.clone());
                }
                check_resource_batch_limit(&required, &[], &[], self.limits.resolved_files)?;
                self.awaiting = Some(required.iter().map(resource_request_key).collect());
                self.begin_non_file_batch(&required, &[], &[]);
                retained.workspace = pending_workspace;
                retained.response_generation = self.response_generation;
                retained.suspension_serial = retained.suspension_serial.saturating_add(1);
                self.candidate = Some(retained);
                self.start_resource_wait();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.request_extraction_time = self
                        .request_extraction_time
                        .saturating_add(html_request_extraction_started.elapsed());
                }
                return Ok(CompileAttemptResult::NeedResources(NeedResources {
                    required,
                    probes: Vec::new(),
                    prefetch_hints: Vec::new(),
                }));
            }
        }
        let execution = match retained.execution {
            RetainedExecution::Initial {
                mut session,
                candidate,
            } => {
                let accepted = session.accept_cold_candidate(candidate).map(Box::new);
                CandidateExecution::Initial { session, accepted }
            }
            RetainedExecution::Pending(candidate) => CandidateExecution::Transaction(
                self.incremental
                    .as_mut()
                    .expect("a pending candidate requires an accepted incremental session")
                    .prepare_revision_candidate(candidate)
                    .map(Box::new),
            ),
        }
        .into_prepared()
        .map_err(|error| {
            CompileError::Diagnostic(CompileDiagnostic::from_session_error(&error, None))
        })?;
        let mut accepted_world = tex_state::World::memory();
        accepted_world
            .publish_detached_effect_records(execution.completion().effects())
            .map_err(|error| CompileError::Output(format!("{error:?}")))?;
        let terminal = accepted_world
            .memory_terminal_output()
            .ok_or_else(|| CompileError::Output("accepted output is not memory-backed".to_owned()))?
            .to_vec();
        let log = accepted_world
            .memory_log_output()
            .ok_or_else(|| CompileError::Output("accepted output is not memory-backed".to_owned()))?
            .to_vec();
        let files = publish_auxiliary_outputs(&accepted_world, &mut generated_transaction)
            .map_err(map_memory_output)?;
        let dvi = if !self.outputs.contains(OutputCapability::Dvi) || execution.pages().is_empty() {
            Vec::new()
        } else {
            execution
                .dvi_bytes()
                .map_err(|error| CompileError::OutputCapability {
                    capability: OutputCapability::Dvi,
                    message: error.to_string(),
                })?
        };
        let mut output = MemoryRunOutput {
            outputs: self.outputs,
            terminal,
            log,
            dvi,
            html: None,
            html_assets: Vec::new(),
            files,
        };
        let existing = output
            .terminal
            .len()
            .saturating_add(output.log.len())
            .saturating_add(output.dvi.len())
            .saturating_add(
                output
                    .files
                    .iter()
                    .map(|file| file.bytes.len())
                    .sum::<usize>(),
            );
        let remaining = self.limits.output_bytes.saturating_sub(existing);
        let mut next_render_document = None;
        let html = if self.outputs.contains(OutputCapability::Html) {
            let output_id = match &execution {
                PreparedExecution::Initial { session, .. } => session.output_id(),
                PreparedExecution::Transaction(_) => self
                    .incremental
                    .as_ref()
                    .expect("a prepared patch has an accepted incremental session")
                    .output_id(),
            };
            let font_responses = self.font_response_fingerprints();
            let assets = SessionFontResolver {
                resolved: &self.resolved_fonts,
                responses: &font_responses,
            };
            let html_options = tex_out::html::HtmlOptions {
                asset_mode: self.html_asset_mode.clone(),
                revision: execution.revision().raw(),
                output_id,
                max_html_bytes: remaining,
                max_total_asset_bytes: remaining,
                max_asset_bytes: remaining,
                ..tex_out::html::HtmlOptions::default()
            };
            let pages = execution
                .pages()
                .iter()
                .map(|page| tex_out::PageArtifact::from_bytes(page.artifact().bytes()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| CompileError::OutputCapability {
                    capability: OutputCapability::Html,
                    message: error.to_string(),
                })?;
            let render_document = build_render_document(
                &pages,
                &assets,
                &html_options,
                RenderSessionId::from_bytes(output_id.as_bytes()),
                execution.revision().raw(),
                self.accepted_render_document
                    .as_ref()
                    .map(|document| &document.revision),
                RenderLimits {
                    max_pages: html_options.max_pages,
                    max_nodes: html_options.max_positioned_events,
                    max_resources: 65_536,
                    max_resource_bytes: remaining,
                },
            )
            .map_err(|error| CompileError::OutputCapability {
                capability: OutputCapability::Html,
                message: error.to_string(),
            })?;
            let html = tex_out::html::write_render_document(&render_document, &html_options)
                .map_err(|error| CompileError::OutputCapability {
                    capability: OutputCapability::Html,
                    message: error.to_string(),
                })?;
            next_render_document = Some(render_document);
            Some(html)
        } else {
            None
        };
        if let Some(html) = html {
            let attempted = existing.saturating_add(html.html.len()).saturating_add(
                html.assets
                    .iter()
                    .map(|asset| asset.bytes.len())
                    .sum::<usize>(),
            );
            check_limit("returned output bytes", attempted, self.limits.output_bytes)?;
            output.html = Some(html.html);
            output.html_assets = html
                .assets
                .into_iter()
                .map(|asset| crate::MemoryOutputFile {
                    path: asset.path.into(),
                    bytes: asset.bytes,
                })
                .collect();
        }
        check_limit("returned output bytes", existing, self.limits.output_bytes)?;
        generated_transaction.accept().map_err(map_transaction)?;
        let previous_generated = generated_fingerprint(&self.workspace)?;
        let next_generated = generated_fingerprint(&pending_workspace)?;
        let reuse = execution.reuse();
        let accepted_engine_output = match execution {
            PreparedExecution::Initial { session, accepted } => {
                self.incremental = Some(session);
                accepted
            }
            PreparedExecution::Transaction(transaction) => Box::new(
                self.incremental
                    .as_mut()
                    .expect("a prepared patch has an accepted incremental session")
                    .accept_revision(*transaction)
                    .map_err(|error| CompileError::Incremental(error.to_string()))?,
            ),
        };
        self.accepted_engine_output = Some(accepted_engine_output);
        self.workspace = pending_workspace;
        self.pending_patch = None;
        self.last_reuse = Some(reuse);
        self.last_stabilization_required = previous_generated != next_generated;
        self.accepted_output = Some(output.clone());
        if let Some(target) = next_render_document {
            let update = if self.pending_render_update.is_some() {
                // The consumer missed an acknowledgement. Coalesce to the
                // newest accepted target through the explicit snapshot path.
                RenderUpdate::Snapshot(target.clone())
            } else if let Some(base) = &self.accepted_render_document {
                let patch = plan_patch(&base.revision, &target.revision, PatchLimits::default())
                    .map_err(|error| CompileError::OutputCapability {
                        capability: OutputCapability::Html,
                        message: error.to_string(),
                    })?;
                RenderUpdate::Patch(patch)
            } else {
                RenderUpdate::Snapshot(target.clone())
            };
            self.accepted_render_document = Some(target);
            self.pending_render_update = Some(update);
        }
        Ok(CompileAttemptResult::Complete(output))
    }

    fn resource_is_bound(&self, key: &ResourceRequestKey) -> bool {
        match key {
            ResourceRequestKey::File(key) => {
                self.workspace.get(key).is_some()
                    || self.workspace.is_unavailable(key)
                    || user_path_for_key(key).is_ok_and(|path| self.workspace.contains_user(&path))
            }
            ResourceRequestKey::Font(key) => self
                .non_file_admission
                .is_bound(&NonFileResourceKey::Font(key.clone())),
            ResourceRequestKey::PkFont(key) => self
                .non_file_admission
                .is_bound(&NonFileResourceKey::PkFont(key.clone())),
        }
    }

    fn font_is_unavailable(&self, key: &FontRequestKey) -> bool {
        self.non_file_admission
            .is_unavailable(&NonFileResourceKey::Font(key.clone()))
    }

    fn pk_font_is_unavailable(&self, key: &PdfPkFontRequest) -> bool {
        self.non_file_admission
            .is_unavailable(&NonFileResourceKey::PkFont(key.clone()))
    }

    fn unavailable_font_keys(&self) -> BTreeSet<FontRequestKey> {
        self.non_file_admission
            .unavailable_keys()
            .filter_map(|key| match key {
                NonFileResourceKey::Font(key) => Some(key.clone()),
                NonFileResourceKey::PkFont(_) => None,
            })
            .collect()
    }

    fn font_response_fingerprints(&self) -> BTreeMap<FontRequestKey, FontResponseFingerprint> {
        self.non_file_admission
            .admitted_entries()
            .filter_map(|(key, binding)| match (key, binding) {
                (NonFileResourceKey::Font(key), NonFileAdmission::Font(fingerprint)) => {
                    Some((key.clone(), fingerprint.clone()))
                }
                (NonFileResourceKey::PkFont(_), NonFileAdmission::PkFont { .. }) => None,
                _ => unreachable!("admission key and binding kinds remain aligned"),
            })
            .collect()
    }

    fn unavailable_pk_font_keys(&self) -> BTreeSet<PdfPkFontRequest> {
        self.non_file_admission
            .unavailable_keys()
            .filter_map(|key| match key {
                NonFileResourceKey::Font(_) => None,
                NonFileResourceKey::PkFont(key) => Some(key.clone()),
            })
            .collect()
    }

    fn begin_non_file_batch(
        &mut self,
        required: &[ResourceRequest],
        probes: &[ResourceRequest],
        hints: &[ResourceRequest],
    ) {
        self.non_file_admission.begin_batch(
            required.iter().filter_map(non_file_resource_key),
            probes.iter().filter_map(non_file_resource_key),
            hints.iter().filter_map(non_file_resource_key),
        );
    }

    pub fn clear_distribution_cache(&mut self) -> Result<(), CompileError> {
        if let Some(session) = &self.incremental {
            let latest = session.source().as_bytes().to_vec();
            self.workspace
                .register_user(self.main_path.clone(), latest)
                .map_err(map_user_registration)?;
        }
        self.workspace.clear();
        self.workspace.clear_generated_outputs();
        self.resolved_fonts.clear();
        self.resolved_pk_fonts.clear();
        self.non_file_admission.clear();
        self.font_requests.clear();
        self.font_cached_bytes = 0;
        self.awaiting = None;
        self.attempts_without_progress = 0;
        self.incremental = None;
        self.accepted_output = None;
        self.pending_patch = None;
        self.candidate = None;
        self.response_generation = 0;
        self.last_reuse = None;
        Ok(())
    }

    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    #[must_use]
    pub const fn compile_telemetry(&self) -> CompileTelemetry {
        CompileTelemetry {
            execution: self.execution_telemetry,
            resource_wait_time: self.resource_wait_time,
            request_extraction_time: self.request_extraction_time,
            candidate_restore_time: self.candidate_restore_time,
            resolver_index_time: self.resolver_index_time,
            vfs_stage_time: self.vfs_stage_time,
        }
    }

    fn finish_resource_wait(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(started) = self.resource_wait_started.take() {
            self.resource_wait_time = self.resource_wait_time.saturating_add(started.elapsed());
        }
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn start_resource_wait(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.resource_wait_started = Some(Instant::now());
        }
    }

    #[cfg(test)]
    pub(crate) const fn attempt_limit(&self) -> u32 {
        self.limits.attempts
    }

    #[must_use]
    pub fn resolved_file_count(&self) -> usize {
        self.workspace.len()
    }

    #[must_use]
    pub fn cached_file_bytes(&self) -> usize {
        self.workspace
            .resolved_bytes()
            .saturating_add(self.font_cached_bytes)
    }
}

fn register_incremental_inputs(
    session: &mut tex_incr::Session,
    workspace: &ProjectWorkspace,
    main_path: &VirtualPath,
) -> Result<(), CompileError> {
    for file in workspace
        .user_files()
        .filter(|file| file.path() != main_path)
        .chain(workspace.files().map(|(_, file)| file))
    {
        session
            .register_input_file(file.path().as_path(), file.bytes().to_vec())
            .map_err(|error| CompileError::Incremental(error.to_string()))?;
    }
    Ok(())
}

fn resource_request_key(request: &ResourceRequest) -> ResourceRequestKey {
    match request {
        ResourceRequest::File(request) => ResourceRequestKey::File(request.key().clone()),
        ResourceRequest::Font(request) => ResourceRequestKey::Font(request.key.clone()),
        ResourceRequest::PkFont(request) => ResourceRequestKey::PkFont(request.clone()),
    }
}

fn non_file_resource_key(request: &ResourceRequest) -> Option<NonFileResourceKey> {
    match request {
        ResourceRequest::File(_) => None,
        ResourceRequest::Font(request) => Some(NonFileResourceKey::Font(request.key.clone())),
        ResourceRequest::PkFont(request) => Some(NonFileResourceKey::PkFont(request.clone())),
    }
}

fn resource_sort_key(request: &ResourceRequest) -> (u8, String) {
    match request {
        ResourceRequest::File(request) => (
            0,
            format!("{:?}:{}", request.key().kind(), request.key().name()),
        ),
        ResourceRequest::Font(request) => (1, request.key.logical_name().to_owned()),
        ResourceRequest::PkFont(request) => (
            2,
            format!(
                "{}:{}:{}",
                String::from_utf8_lossy(request.tex_name()),
                request.dpi(),
                String::from_utf8_lossy(request.mode())
            ),
        ),
    }
}

fn check_resource_batch_limit(
    required: &[ResourceRequest],
    probes: &[ResourceRequest],
    prefetch_hints: &[ResourceRequest],
    limit: usize,
) -> Result<(), CompileError> {
    let unique = required
        .iter()
        .chain(probes)
        .chain(prefetch_hints)
        .map(resource_request_key)
        .collect::<BTreeSet<_>>();
    check_limit("resource request batch", unique.len(), limit)
}

struct SessionFontResolver<'a> {
    resolved: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
    responses: &'a BTreeMap<FontRequestKey, FontResponseFingerprint>,
}

impl<'a> SessionFontResolver<'a> {
    fn realized(
        &'a self,
        font: &tex_out::FontResource,
    ) -> Option<(&'a FontRequestKey, &'a OpenTypeFont)> {
        if let Some(binding) = &font.opentype {
            return self.resolved.iter().find(|(key, supplied)| {
                key.logical_name() == font.name
                    && supplied.identity == binding.program_identity
                    && supplied.object_identity == binding.object_identity
                    && supplied.instance_identity(font.at_size) == binding.instance_identity
            });
        }
        self.resolved.iter().find(|(key, _)| {
            self.responses
                .get(*key)
                .and_then(|response| response.legacy_mapping.as_ref())
                .is_some_and(|mapping| {
                    key.logical_name() == font.name
                        && mapping.tfm_sha256 == font.tfm_content_hash.bytes()
                })
        })
    }
}

impl HtmlFontAssets for SessionFontResolver<'_> {
    fn font_asset(&self, font: &tex_out::FontResource) -> Result<HtmlFontAsset, String> {
        if let Some(binding) = &font.opentype {
            let (key, supplied) = self.realized(font).ok_or_else(|| {
                format!(
                    "retained OpenType resource for artifact font {} is unavailable or mismatched",
                    font.name
                )
            })?;
            if binding.container != tex_fonts::FontContainer::Woff2
                || supplied.container != tex_fonts::FontContainer::Woff2
            {
                return Err(format!(
                    "HTML reuse for retained {:?} font {} is not supported",
                    supplied.container, font.name
                ));
            }
            let response = self.responses.get(key).ok_or_else(|| {
                format!(
                    "retained response metadata for {} is unavailable",
                    font.name
                )
            })?;
            let provenance = response.provenance.clone().ok_or_else(|| {
                format!("retained font {} has no embedding provenance", font.name)
            })?;
            let mapped_bundle = response.legacy_mapping.as_ref();
            let mut encoding = if let Some(bundle) = mapped_bundle {
                if bundle.tfm_sha256 != font.tfm_content_hash.bytes() {
                    return Err(format!(
                        "retained mapping for {} has the wrong TFM identity",
                        font.name
                    ));
                }
                bundle.encoding.clone()
            } else {
                vec![None; 256]
            };
            if mapped_bundle.is_none() {
                for (code, mapped) in encoding.iter_mut().enumerate() {
                    if let Some(scalar) = char::from_u32(code as u32)
                        && supplied.cmap.glyph(scalar).is_some()
                    {
                        *mapped = Some(scalar.to_string());
                    }
                }
            }
            for text in encoding.iter().flatten() {
                if text
                    .chars()
                    .any(|scalar| supplied.cmap.glyph(scalar).is_none())
                {
                    return Err(format!(
                        "mapped bundle for {} contains Unicode text absent from the retained cmap",
                        font.name
                    ));
                }
            }
            return Ok(HtmlFontAsset {
                key: HtmlFontKey::from(font),
                woff2: supplied.transport_bytes.to_vec(),
                sha256: supplied.object_identity.bytes(),
                encoding,
                provenance,
                embeddable: mapped_bundle.is_none_or(|mapping| mapping.embeddable),
            });
        }
        let (key, supplied, mapping) = self
            .resolved
            .iter()
            .find_map(|(key, supplied)| {
                let mapping = self.responses.get(key)?.legacy_mapping.as_ref()?;
                (key.logical_name() == font.name
                    && mapping.tfm_sha256 == font.tfm_content_hash.bytes())
                .then_some((key, supplied, mapping))
            })
            .ok_or_else(|| {
                format!(
                    "unsupported HTML legacy mapping for classic TFM font {} ({})",
                    font.name,
                    font.tfm_content_hash.hex()
                )
            })?;
        if supplied.container != tex_fonts::FontContainer::Woff2 {
            return Err(format!(
                "HTML reuse for retained {:?} font {} is not supported",
                supplied.container, font.name
            ));
        }
        let response = self.responses.get(key).expect("selected response exists");
        let provenance = response
            .provenance
            .clone()
            .ok_or_else(|| format!("retained font {} has no embedding provenance", font.name))?;
        for text in mapping.encoding.iter().flatten() {
            if text
                .chars()
                .any(|scalar| supplied.cmap.glyph(scalar).is_none())
            {
                return Err(format!(
                    "mapped bundle for {} contains Unicode text absent from the retained cmap",
                    font.name
                ));
            }
        }
        Ok(HtmlFontAsset {
            key: HtmlFontKey::from(font),
            woff2: supplied.transport_bytes.to_vec(),
            sha256: supplied.object_identity.bytes(),
            encoding: mapping.encoding.clone(),
            provenance,
            embeddable: mapping.embeddable,
        })
    }

    fn realized_opentype(&self, font: &tex_out::FontResource) -> Option<&OpenTypeFont> {
        self.realized(font).map(|(_, font)| font)
    }
}

fn discover_html_paint_resources<'a>(
    artifacts: impl IntoIterator<Item = &'a tex_state::CommittedArtifact>,
    resolved: &BTreeMap<FontRequestKey, OpenTypeFont>,
    unavailable: &BTreeSet<FontRequestKey>,
    accepted_containers: AcceptedFontContainers,
) -> Result<Vec<ResourceRequest>, CompileError> {
    let mut classic_fonts = BTreeMap::<FontRequestKey, (String, [u8; 32])>::new();
    for artifact in artifacts {
        let page = tex_out::PageArtifact::from_bytes(artifact.bytes()).map_err(|error| {
            CompileError::OutputCapability {
                capability: OutputCapability::Html,
                message: error.to_string(),
            }
        })?;
        for font in &page.fonts {
            if font.opentype.is_some() {
                continue;
            }
            let key = FontRequestKey::new(
                &font.name,
                0,
                tex_fonts::VariationSelection::default(),
                tex_fonts::FontFeaturePolicy::default(),
            )
            .map_err(|error| CompileError::OutputCapability {
                capability: OutputCapability::Html,
                message: format!(
                    "invalid classic font paint request for {}: {error}",
                    font.name
                ),
            })?;
            register_classic_html_paint_font(
                &mut classic_fonts,
                key,
                &font.name,
                font.tfm_content_hash.bytes(),
            )?;
        }
    }
    let mut required = Vec::new();
    for (key, (name, tfm_sha256)) in classic_fonts {
        if let Some(font) = resolved.get(&key) {
            let _ = font;
            continue;
        }
        if unavailable.contains(&key) {
            return Err(CompileError::OutputCapability {
                capability: OutputCapability::Html,
                message: format!(
                    "unsupported HTML legacy mapping for classic TFM font {name} ({})",
                    hex_sha256(tfm_sha256)
                ),
            });
        }
        required.push(ResourceRequest::Font(FontRequest {
            key,
            accepted_containers,
            purposes: FontPurposes::HTML,
        }));
    }
    required.sort_by_key(resource_sort_key);
    Ok(required)
}

enum PdfRawObjectFileLookup {
    Bound(PdfRawObjectFileReceiptEntry),
    Missing(FileRequest),
    Unavailable,
}

fn discover_pdf_raw_object_files(
    pdf: Option<&tex_state::DetachedPdfCompletion>,
    workspace: &ProjectWorkspace,
) -> Result<Vec<FileRequest>, String> {
    let mut required = BTreeMap::new();
    for need in pdf.into_iter().flat_map(|pdf| pdf.raw_object_file_needs()) {
        let name = std::str::from_utf8(&need.source_name).map_err(|_| {
            format!(
                "PDF stream object {} has a non-UTF-8 file name",
                need.object
            )
        })?;
        match lookup_pdf_raw_object_file(workspace, name)? {
            PdfRawObjectFileLookup::Bound(_) => {}
            PdfRawObjectFileLookup::Missing(request) => {
                required.entry(request.key().clone()).or_insert(request);
            }
            PdfRawObjectFileLookup::Unavailable => {
                return Err(format!(
                    "PDF stream object {} file {name:?} is unavailable",
                    need.object
                ));
            }
        }
    }
    Ok(required.into_values().collect())
}

fn pdf_raw_object_file_receipt(
    pdf: Option<&tex_state::DetachedPdfCompletion>,
    workspace: &ProjectWorkspace,
) -> Result<PdfRawObjectFileReceipt, CompileError> {
    let mut entries = BTreeMap::new();
    for need in pdf.into_iter().flat_map(|pdf| pdf.raw_object_file_needs()) {
        let name =
            std::str::from_utf8(&need.source_name).map_err(|_| CompileError::OutputCapability {
                capability: OutputCapability::Pdf,
                message: format!(
                    "PDF stream object {} has a non-UTF-8 file name",
                    need.object
                ),
            })?;
        let PdfRawObjectFileLookup::Bound(entry) = lookup_pdf_raw_object_file(workspace, name)
            .map_err(|message| CompileError::OutputCapability {
                capability: OutputCapability::Pdf,
                message,
            })?
        else {
            return Err(CompileError::Incremental(format!(
                "accepted PDF stream object {} has no bound file payload",
                need.object
            )));
        };
        entries.insert(need.object, entry);
    }
    Ok(PdfRawObjectFileReceipt { entries })
}

fn lookup_pdf_raw_object_file(
    workspace: &ProjectWorkspace,
    name: &str,
) -> Result<PdfRawObjectFileLookup, String> {
    let requested = RequestedFile::parse(FileKind::TexInput, name)
        .map_err(|error| format!("invalid PDF stream file name {name:?}: {error}"))?;
    let snapshot = workspace.snapshot();
    let entry = match requested {
        RequestedFile::UserOnly(path) => {
            let Some(file) = snapshot.get(&path).map_err(|error| error.to_string())? else {
                return Ok(PdfRawObjectFileLookup::Unavailable);
            };
            PdfRawObjectFileReceiptEntry {
                source_name: name.to_owned(),
                source: PdfRawObjectFileSource::User(path),
                virtual_path: file.path().as_str().to_owned(),
                content_id: file.content_id(),
                bytes: file.bytes().to_vec(),
            }
        }
        RequestedFile::Remote { user_path, key } => {
            if let Some(path) = user_path
                && let Some(file) = snapshot.get(&path).map_err(|error| error.to_string())?
            {
                return Ok(PdfRawObjectFileLookup::Bound(
                    PdfRawObjectFileReceiptEntry {
                        source_name: name.to_owned(),
                        source: PdfRawObjectFileSource::User(path),
                        virtual_path: file.path().as_str().to_owned(),
                        content_id: file.content_id(),
                        bytes: file.bytes().to_vec(),
                    },
                ));
            }
            if let Some(file) = workspace.get(&key) {
                PdfRawObjectFileReceiptEntry {
                    source_name: name.to_owned(),
                    source: PdfRawObjectFileSource::Resolved(key),
                    virtual_path: file.path().as_str().to_owned(),
                    content_id: file.content_id(),
                    bytes: file.bytes().to_vec(),
                }
            } else if workspace.is_unavailable(&key) {
                return Ok(PdfRawObjectFileLookup::Unavailable);
            } else {
                return Ok(PdfRawObjectFileLookup::Missing(FileRequest::new(key, name)));
            }
        }
    };
    Ok(PdfRawObjectFileLookup::Bound(entry))
}

fn register_classic_html_paint_font(
    fonts: &mut BTreeMap<FontRequestKey, (String, [u8; 32])>,
    key: FontRequestKey,
    name: &str,
    identity: [u8; 32],
) -> Result<(), CompileError> {
    match fonts.entry(key) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert((name.to_owned(), identity));
            Ok(())
        }
        std::collections::btree_map::Entry::Occupied(entry) if entry.get().1 != identity => {
            Err(CompileError::ConflictingHtmlFontBinding {
                name: name.to_owned(),
                expected_tfm_identity: entry.get().1,
                conflicting_tfm_identity: identity,
            })
        }
        std::collections::btree_map::Entry::Occupied(_) => Ok(()),
    }
}

fn hex_sha256(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn check_limit(resource: &'static str, attempted: usize, limit: usize) -> Result<(), CompileError> {
    if attempted > limit {
        return Err(CompileError::LimitExceeded {
            resource,
            limit,
            attempted,
        });
    }
    Ok(())
}

fn check_format_image_bytes(attempted: usize) -> Result<(), CompileError> {
    check_limit(
        "format image bytes",
        attempted,
        SessionLimits::FORMAT_IMAGE_BYTES,
    )
}

fn accepted_dependencies_match_snapshot<'a>(
    dependencies: impl Iterator<Item = &'a tex_state::InputDependency>,
    snapshot: &umber_vfs::VfsSnapshot,
    main_path: &VirtualPath,
) -> Result<bool, CompileError> {
    for dependency in dependencies {
        let Some(path) = crate::input_observation::virtual_path(dependency.path()) else {
            return Ok(false);
        };
        if &path == main_path {
            continue;
        }
        let file = snapshot
            .get(&path)
            .map_err(|error| CompileError::World(error.to_string()))?;
        let matches = match (dependency.outcome(), file) {
            (tex_state::InputDependencyOutcome::Missing, None) => true,
            (tex_state::InputDependencyOutcome::Present(expected), Some(file)) => {
                ContentHash::from_bytes(file.bytes()) == expected
            }
            (tex_state::InputDependencyOutcome::Missing, Some(_))
            | (tex_state::InputDependencyOutcome::Present(_), None) => false,
        };
        if !matches {
            return Ok(false);
        }
    }
    Ok(true)
}

fn generated_fingerprint(
    files: &ProjectWorkspace,
) -> Result<Vec<(VirtualPath, ContentHash)>, CompileError> {
    let snapshot = files.snapshot();
    let path_limit = VfsLimits::HARD_MAX
        .user_files
        .saturating_add(VfsLimits::HARD_MAX.generated_files);
    let paths = snapshot
        .list_root(VirtualRoot::Job, path_limit)
        .map_err(|error| CompileError::Output(error.to_string()))?;
    let mut generated = Vec::new();
    for path in paths {
        let file = snapshot
            .get(&path)
            .map_err(|error| CompileError::Output(error.to_string()))?
            .expect("a listed VFS path resolves");
        if matches!(file.origin(), FileOrigin::Generated) {
            generated.push((path, ContentHash::from_bytes(file.bytes())));
        }
    }
    Ok(generated)
}

fn memory_run_output_bytes(output: &MemoryRunOutput) -> usize {
    output
        .terminal
        .len()
        .saturating_add(output.log.len())
        .saturating_add(output.dvi.len())
        .saturating_add(output.html.as_ref().map_or(0, Vec::len))
        .saturating_add(
            output
                .html_assets
                .iter()
                .chain(&output.files)
                .map(|file| file.bytes.len())
                .sum::<usize>(),
        )
}

fn map_user_registration(error: UserRegistrationError) -> CompileError {
    match error {
        UserRegistrationError::Limit(error) => map_vfs_limit(error),
        UserRegistrationError::InvalidPath { path } => {
            CompileError::World(format!("user path is outside /job: {path}"))
        }
    }
}

fn map_vfs_limit(error: VfsLimitError) -> CompileError {
    match error {
        VfsLimitError::HardLimitExceeded {
            kind,
            hard,
            attempted,
        } => CompileError::HardLimitExceeded {
            resource: kind.description(),
            hard,
            attempted,
        },
        VfsLimitError::LimitExceeded {
            kind,
            limit,
            attempted,
        } => CompileError::LimitExceeded {
            resource: kind.description(),
            limit,
            attempted,
        },
    }
}

fn map_transaction(error: TransactionError) -> CompileError {
    match error {
        TransactionError::Limit(error) => map_vfs_limit(error),
        error => CompileError::Output(error.to_string()),
    }
}

fn map_memory_output(error: MemoryOutputCollectionError) -> CompileError {
    match error {
        MemoryOutputCollectionError::Transaction(error) => map_transaction(error),
        error => CompileError::Output(error.to_string()),
    }
}

fn map_provision(error: ProvisionError) -> CompileError {
    match error {
        ProvisionError::Limit(error) => map_vfs_limit(error),
        ProvisionError::Conflict { request, .. } => {
            CompileError::ConflictingResolvedBinding(request.name().to_owned())
        }
        ProvisionError::AvailabilityConflict { request } => {
            CompileError::ConflictingResolvedBinding(request.name().to_owned())
        }
        ProvisionError::PathConflict { path, .. } => {
            CompileError::DistributionPathCollision(path.to_string())
        }
        ProvisionError::UnexpectedRequest(request) => {
            CompileError::UnexpectedResourceResponse(request.name().to_owned())
        }
        ProvisionError::InvalidPath { path, message, .. } => CompileError::InvalidVirtualPath {
            path,
            message: message.to_owned(),
        },
        error @ (ProvisionError::KindMismatch { .. } | ProvisionError::DigestMismatch { .. }) => {
            CompileError::FileProvision(error)
        }
    }
}

fn map_non_file_admission(error: AdmissionError<NonFileResourceKey>, name: &str) -> CompileError {
    match error {
        AdmissionError::Unexpected(_) | AdmissionError::NegativeHint(_) => {
            CompileError::UnexpectedResourceResponse(name.to_owned())
        }
        AdmissionError::AvailabilityConflict(_) | AdmissionError::BindingConflict(_) => {
            CompileError::ConflictingResolvedBinding(name.to_owned())
        }
    }
}

#[cfg(test)]
mod tests;
