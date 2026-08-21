use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tex_command::{
    CommandProfile, FontResource, PdfImageResource, RegisteredSourceKind, SourceRegistration,
};
use tex_exec::{
    CheckpointSink, EngineBoundary, FontResolver, PdfImageRequest as OutputPdfImageRequest,
    PdfImageResolver,
};
use tex_out::dvi::{DviError, DviPagePlan, DviStreamWriter};
use tex_state::env::banks::IntParam;
use tex_state::{
    CommittedArtifact, ContentHash, EffectRecord, FileContent, GenerationBrand, InputResolver,
    PrintSink, ResourceLookup, ResourceResult, Universe, World, WorldError,
};

#[cfg(not(target_arch = "wasm32"))]
pub mod cli_resource;
#[cfg(not(target_arch = "wasm32"))]
mod distribution_verify;
mod editor_session;
mod engine_session;
mod fixed_point;
#[cfg(not(target_arch = "wasm32"))]
mod format_cache;
#[cfg(not(target_arch = "wasm32"))]
mod format_fixture;
#[cfg(not(target_arch = "wasm32"))]
mod format_worker;
mod input_observation;
mod input_search;
mod latex_project;
#[cfg(target_os = "linux")]
mod linux_rss;
mod memory_output;
mod pdf_import;
mod pdf_output;
mod pdftex;
#[cfg(not(target_arch = "wasm32"))]
mod prepared_format;
mod tex_fixed_point;
mod virtual_compile;

pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(not(target_arch = "wasm32"))]
pub use distribution_verify::{
    DistributionVerificationError, DistributionVerificationReport, verify_distribution,
};
pub use editor_session::{
    EditorCompileSession, EditorResourceError, EditorSessionOptions, EditorSessionStatus,
    EditorStabilizationAttempt,
};
pub use engine_session::{
    DEFAULT_NO_PROGRESS_LIMIT, EngineSession, ExpansionStats, SessionError, SessionState,
    StartupInput,
};
pub use fixed_point::FixedPointLimits;
#[cfg(not(target_arch = "wasm32"))]
pub use format_cache::{
    FormatCacheClock, FormatCacheError, FormatCacheIdentity, FormatCacheStore, FormatEngineMode,
    FormatFingerprint, FormatFixtureIdentity, ValidatedFormatEntry, ValidatedFormatImage,
};
#[cfg(not(target_arch = "wasm32"))]
pub use format_fixture::{
    FormatFixture, FormatFixtureError, FormatGenerationGuards, FormatRecipe, FormatResource,
    LoadedFormatFixture, LoadedFormatResource, LoadedFormatRun, ensure_format,
};
#[cfg(not(target_arch = "wasm32"))]
pub use format_worker::{FormatWorkerLauncher, dispatch_format_worker, run_format_worker};
pub use input_observation::{
    ACCEPTED_INPUT_OBSERVATION_SCHEMA_VERSION, AcceptedInputObservation,
    AcceptedInputObservationLedger, InputObservationNamespace, InputObservationOutcome,
    InputObservationOwner, InputObservationPhase, MAX_ACCEPTED_INPUT_OBSERVATIONS,
};
pub use input_search::{TexFontSearchPath, TexInputSearchPath};
pub use latex_project::{
    BibliographyProjectOptions, LatexProjectAttempt, LatexProjectError, LatexProjectLimits,
    LatexProjectOptions, LatexProjectOutput, LatexProjectSession, ProjectConvergenceFingerprint,
};
pub use memory_output::{
    MemoryOutputCollectionError, MemoryOutputFile, MemoryRunOutput, collect_final_memory_output,
    collect_final_memory_output_from_commits, collect_final_memory_output_from_plans,
};
pub use pdf_output::{
    PdfBuildError, pdf_finalization_input, pdf_finalization_input_with_raw_object_files,
    pdf_from_accepted_artifacts_with_virtual_fonts, pdf_from_completion_at_dpi,
};
#[cfg(not(target_arch = "wasm32"))]
pub use prepared_format::{PreparedFormatJob, PreparedFormatProvider};
pub use tex_exec::{ResourceFulfillment, ResourceHost, ResourceOutcome, ResourceWorld};
pub use tex_fixed_point::{
    TexFixedPointAttempt, TexFixedPointError, TexFixedPointOptions, TexFixedPointOutput,
    TexFixedPointSession,
};
pub use tex_fonts::{
    AcceptedFontContainers, FeatureSetting, FontContainer, FontFeaturePolicy, FontLanguage,
    FontLayoutPolicy, FontMappingFallbackPolicy, FontObjectIdentity, FontProgramIdentity,
    FontPurposes, FontRequest, FontRequestKey, LegacyFontMapping, OpenTypeTag, PdfPkFontRequest,
    ResolvedFont, VariationCoordinate, VariationInstance, VariationSelection, WritingDirection,
};
pub use tex_incr::{RenderedOutputId, ReuseMetrics, RevisionId, SameHistoryStop};
pub use tex_out::html::incremental::{
    PatchOp, PatchPlan, RenderBox, RenderDigest, RenderDirection, RenderDocument, RenderKey,
    RenderMathDrawing, RenderMathGlyph, RenderNode, RenderNodeValue, RenderPage, RenderPageHeader,
    RenderResource, RenderRevision, RenderRule, RenderSessionId, RenderSpecial,
    RenderSpecialAction, RenderText,
};
pub use tex_out::positioned::BoxKind;
pub use tex_state::{InputDependency, InputDependencyAccess, InputDependencyOutcome};
pub use umber_vfs::FileContentId;
pub use virtual_compile::RenderUpdate;

/// Immutable startup capability for one retained canonical run.
///
/// The completion policy distinguishes a complete TeX job, which must scan a
/// canonical terminator, from a host-owned fragment that intentionally stops
/// at its authored root boundary.
pub struct RetainedRootRequest {
    pub startup_name: String,
    pub invocation: String,
    pub profile: CommandProfile,
    pub source: SourceRegistration,
    pub completion: tex_exec::RootCompletionPolicy,
}

impl RetainedRootRequest {
    /// Constructs a complete authored TeX job.
    ///
    /// The source must reach `\end`, `\dump`, or a format-level equivalent.
    /// Root EOF without one follows TeX82's missing-`\end` handling.
    #[must_use]
    pub fn authored_job(
        startup_name: impl Into<String>,
        source: impl Into<Arc<[u8]>>,
        profile: CommandProfile,
    ) -> Self {
        let startup_name = startup_name.into();
        Self {
            invocation: startup_name.clone(),
            startup_name,
            profile,
            source: SourceRegistration::new(RegisteredSourceKind::Generated, source.into()),
            completion: tex_exec::RootCompletionPolicy::RequireTeXEnd,
        }
    }

    /// Constructs an authored fragment which stops at its root EOF without
    /// pretending that TeX scanned `\end` or running final cleanup.
    #[must_use]
    pub fn authored_fragment(
        startup_name: impl Into<String>,
        source: impl Into<Arc<[u8]>>,
        profile: CommandProfile,
    ) -> Self {
        let startup_name = startup_name.into();
        Self {
            invocation: startup_name.clone(),
            startup_name,
            profile,
            source: SourceRegistration::new(RegisteredSourceKind::Generated, source.into()),
            completion: tex_exec::RootCompletionPolicy::StopAtRootEof,
        }
    }

    /// Retains a root already selected through the active [`World`].
    #[must_use]
    pub fn file(
        startup_name: impl Into<String>,
        source: FileContent,
        profile: CommandProfile,
    ) -> Self {
        let startup_name = startup_name.into();
        Self {
            invocation: startup_name.clone(),
            startup_name,
            profile,
            source: SourceRegistration::world(source),
            completion: tex_exec::RootCompletionPolicy::RequireTeXEnd,
        }
    }
}

struct NoCheckpoints;

fn engine_interner_budget() -> tex_state::interner::InternerBudget {
    tex_state::interner::InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024)
        .expect("the Umber engine interner budget is valid")
}

/// Runs one engine episode inside a fresh generation brand.
///
/// The higher-ranked callback prevents runtime ids, checkpoints, or arena
/// borrows from escaping into host-owned state.
pub fn with_engine_universe<R>(
    use_universe: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> Result<R, tex_state::StateError> {
    tex_state::with_universe(engine_interner_budget(), use_universe)
}

/// Installs a host-owned world before entering one freshly branded episode.
pub fn with_engine_world<R>(
    world: World,
    use_universe: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> Result<R, tex_state::StateError> {
    with_engine_universe(|universe| {
        *universe.world_mut() = world;
        use_universe(universe)
    })
}

impl<G> CheckpointSink<G> for NoCheckpoints {
    fn wants_checkpoint(&self, _boundary: EngineBoundary) -> bool {
        false
    }

    fn checkpoint(&mut self, _checkpoint: tex_exec::EngineCheckpoint<G>) {}
}

/// Runs one retained immutable root through canonical main control.
pub fn run_retained_root<G>(
    stores: &mut Universe<G>,
    request: RetainedRootRequest,
    host: &mut dyn ResourceHost,
) -> Result<RunResult, SessionError> {
    let mut session = EngineSession::new(stores, request.profile);
    match request.completion {
        tex_exec::RootCompletionPolicy::RequireTeXEnd => {
            session.register_retained_root_with_invocation(
                &request.startup_name,
                &request.invocation,
                request.source,
            )?;
        }
        tex_exec::RootCompletionPolicy::StopAtRootEof => {
            session.register_retained_fragment_with_invocation(
                &request.startup_name,
                &request.invocation,
                request.source,
            )?;
        }
    }
    session.run(host, &mut NoCheckpoints)
}

/// Registers the one exact Cargo-test entry used when an authenticated format
/// worker re-executes the already-trusted current test image.
#[cfg(target_os = "linux")]
#[macro_export]
macro_rules! register_format_worker_test_bootstrap {
    () => {
        #[must_use]
        fn umber_format_worker_launcher() -> $crate::FormatWorkerLauncher {
            $crate::FormatWorkerLauncher::registered_libtest("umber_format_worker_bootstrap")
        }

        #[test]
        fn umber_format_worker_bootstrap() {
            $crate::run_format_worker_test_bootstrap();
        }
    };
}

/// Internal implementation for [`register_format_worker_test_bootstrap!`].
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn run_format_worker_test_bootstrap() {
    format_worker::run_test_bootstrap();
}

#[cfg(all(test, target_os = "linux"))]
register_format_worker_test_bootstrap!();

pub use virtual_compile::{
    AcceptedFinalization, CachedLocalTfm, CachedVirtualFont, CompileAttemptResult,
    CompileDiagnostic, CompileError, CompileSourceLocation, CompileTelemetry, EngineMode, FileKind,
    FileRequest, FileRequestKey, NeedResources, OutputCapability, OutputCapabilitySet,
    PdfFontClosureReceipt, PdfFontClosureReceiptEntry, PdfFontClosureResourceOutcome,
    PdfRawObjectFileReceipt, PdfRawObjectFileReceiptEntry, PdfRawObjectFileSource,
    PdfVirtualFontResources, RenderedSourceLocation, RenderedSourceResult, RequestKeyError,
    ResolvedFile, ResolvedPkFont, ResourceDomain, ResourceRequest, ResourceResponse,
    RetentionMetrics, SessionLimits, SessionOptions, SourcePatch, VfsLimitError, VfsLimitKind,
    VfsLimits, VirtualCompileSession, VirtualPath, VirtualPathError,
};

pub struct FileSessionResolvers {
    input: FileInputResolver,
    font: FileFontResolver,
    image: FileImageResolver,
}

impl FileSessionResolvers {
    #[must_use]
    pub fn from_environment(path: &Path) -> Self {
        let areas = |name| {
            std::env::var_os(name)
                .map(|value| {
                    std::env::split_paths(&value)
                        .filter(|path| !path.as_os_str().is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        Self::new(path, areas("TEXINPUTS"), areas("TEXFONTS"))
    }

    #[must_use]
    pub fn new(path: &Path, tex_input_areas: Vec<PathBuf>, tex_font_areas: Vec<PathBuf>) -> Self {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_owned();
        let input_search = TexInputSearchPath::new(&base_dir, tex_input_areas);
        Self {
            input: FileInputResolver(input_search.clone()),
            font: FileFontResolver(TexFontSearchPath::new(base_dir, tex_font_areas)),
            image: FileImageResolver(input_search),
        }
    }

    /// Borrows the input and font resolvers for an incremental editor session.
    pub fn resolvers(&mut self) -> (&mut dyn InputResolver, &mut dyn FontResolver) {
        (&mut self.input, &mut self.font)
    }
}

impl ResourceHost for FileSessionResolvers {
    fn fulfill(
        &mut self,
        world: &mut ResourceWorld<'_>,
        need: &tex_exec::ResourceNeed,
    ) -> ResourceOutcome {
        match need {
            tex_exec::ResourceNeed::Input { name, .. } => {
                if let Some(result) = self
                    .input
                    .0
                    .read_restricted_pipe_from_resource_world(world, name)
                {
                    return result.map_or(ResourceOutcome::Unavailable, |text| {
                        ResourceOutcome::Fulfilled(ResourceFulfillment::input(
                            name,
                            RegisteredSourceKind::Generated,
                            Arc::from(text.into_bytes()),
                        ))
                    });
                }
                self.input
                    .0
                    .read_from_resource_world(world, name)
                    .ok()
                    .map_or(ResourceOutcome::Unavailable, |content| {
                        ResourceOutcome::Fulfilled(ResourceFulfillment::world_input(name, content))
                    })
            }
            tex_exec::ResourceNeed::InputProbe { request } => self
                .input
                .0
                .read_from_resource_world(world, &request.name)
                .ok()
                .map_or(ResourceOutcome::Unavailable, |content| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::world_input_probe(
                        request.clone(),
                        content,
                    ))
                }),
            tex_exec::ResourceNeed::Font { request } => {
                let mut path = PathBuf::from(&request.name);
                if path.extension().is_none() {
                    path.set_extension("tfm");
                }
                ResourceOutcome::Fulfilled(
                    self.font
                        .0
                        .read_from_resource_world(world, &path)
                        .map_or_else(
                            |_| ResourceFulfillment::Font {
                                request: request.clone(),
                                resource: Box::new(FontResource::Unavailable),
                            },
                            |metrics| ResourceFulfillment::Font {
                                request: request.clone(),
                                resource: Box::new(FontResource::Tfm {
                                    metrics,
                                    opentype: None,
                                }),
                            },
                        ),
                )
            }
            tex_exec::ResourceNeed::PdfImage { request } => {
                let Ok(content) = self
                    .image
                    .0
                    .read_exact_from_resource_world(world, &request.name)
                else {
                    return ResourceOutcome::Unavailable;
                };
                let legacy = OutputPdfImageRequest {
                    name: request.name.clone(),
                    page: match &request.page {
                        tex_command::PdfImagePageSelection::Number(page) => {
                            tex_exec::PdfImagePageSelection::Number(
                                u32::try_from(*page).unwrap_or_default(),
                            )
                        }
                        tex_command::PdfImagePageSelection::Named(name) => {
                            tex_exec::PdfImagePageSelection::Named(name.clone())
                        }
                    },
                    color_space_object: request.color_space_object,
                    page_box: match request.page_box {
                        tex_command::PdfImagePageBox::Crop => tex_exec::PdfImagePageBox::Crop,
                        tex_command::PdfImagePageBox::Media => tex_exec::PdfImagePageBox::Media,
                        tex_command::PdfImagePageBox::Bleed => tex_exec::PdfImagePageBox::Bleed,
                        tex_command::PdfImagePageBox::Trim => tex_exec::PdfImagePageBox::Trim,
                        tex_command::PdfImagePageBox::Art => tex_exec::PdfImagePageBox::Art,
                    },
                    resolution: 0,
                };
                let resource = virtual_compile::parse_image(&content, &legacy)
                    .map(PdfImageResource::Available)
                    .unwrap_or_else(PdfImageResource::Invalid);
                ResourceOutcome::Fulfilled(ResourceFulfillment::PdfImage {
                    request: request.clone(),
                    resource: Box::new(resource),
                })
            }
        }
    }
}

struct FileInputResolver(TexInputSearchPath);

impl InputResolver for FileInputResolver {
    fn open_input(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        name: &str,
        _request_index: u64,
    ) -> ResourceResult<FileContent> {
        if let Some(output) = self.0.read_restricted_pipe(input, name) {
            return output.and_then(|text| {
                input
                    .read_supplied_input_file(Path::new(name), text.into_bytes().into())
                    .map(ResourceLookup::Available)
                    .map_err(|error| error.to_string())
            });
        }
        Ok(match self.0.read(input, name) {
            Ok(content) => ResourceLookup::Available(content),
            Err(_) => ResourceLookup::Unavailable,
        })
    }

    fn input_file_size(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        name: &str,
        _request_index: u64,
    ) -> ResourceResult<u64> {
        Ok(match self.0.read(input, name) {
            Ok(content) => {
                ResourceLookup::Available(u64::try_from(content.bytes().len()).unwrap_or(u64::MAX))
            }
            Err(_) => ResourceLookup::Unavailable,
        })
    }

    fn open_stream_input(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        name: &str,
        _request_index: u64,
    ) -> ResourceResult<tex_state::FileContent> {
        Ok(match self.0.read(input, name) {
            Ok(content) => ResourceLookup::Available(content),
            Err(_) => ResourceLookup::Unavailable,
        })
    }
}

struct FileFontResolver(TexFontSearchPath);

struct FileImageResolver(TexInputSearchPath);

impl PdfImageResolver for FileImageResolver {
    fn open_image(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        request: &OutputPdfImageRequest,
        _request_index: u64,
    ) -> tex_exec::ResourceResult<tex_state::PdfExternalImageSource> {
        let content = match self.0.read(input, &request.name) {
            Ok(content) => content,
            Err(_) => return Ok(tex_exec::ResourceLookup::Unavailable),
        };
        virtual_compile::parse_image(&content, request).map(tex_exec::ResourceLookup::Available)
    }
}

impl FontResolver for FileFontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        path: &Path,
        _request_index: u64,
    ) -> tex_exec::ResourceResult<tex_exec::FontSource> {
        Ok(match self.0.read(input, path) {
            Ok(metrics) => tex_exec::ResourceLookup::Available(tex_exec::FontSource::Tfm {
                metrics,
                opentype: None,
            }),
            Err(_) => tex_exec::ResourceLookup::Unavailable,
        })
    }
}

/// Result of running TeX through the batch executor.
#[derive(Debug)]
pub struct RunResult {
    pub terminal_text: String,
    /// TeX's process-level outcome after all engine finalization diagnostics.
    pub status: TexRunStatus,
    /// Ordered canonical execution modes, including the initial mode and each
    /// distinct mode reached after a committed main-control step.
    pub mode_transitions: Vec<tex_exec::Mode>,
    /// TeX82's defined fatal terminal state, distinct from runner failure.
    pub fatal: Option<tex_command::FatalError>,
    pub artifacts: Vec<ContentHash>,
    /// Precompiled page-local DVI bodies aligned with `artifacts`.
    pub dvi_pages: Vec<DviPagePlan>,
    /// Exact canonical bytes from this execution's successful shipout commits.
    pub committed_artifacts: Vec<CommittedArtifact>,
    /// Live effect suffix still pending in the World after this successful
    /// execution, in receipt order. Earlier commit boundaries may already have
    /// drained a prefix into the World's backing outputs.
    pub effects: Vec<EffectRecord>,
    /// Exact handle-free INITEX result, captured only after aggregate quiescence.
    pub format_dump: Option<tex_exec::DetachedFormatDump>,
}

/// TeX's completed process status, derived from the final §76 `history`.
///
/// Web2C maps `spotless` and `warning_issued` to a successful process and
/// maps `error_message_issued` and `fatal_error_stop` to an unsuccessful
/// process. Fatal completion remains separately available through
/// [`RunResult::fatal`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TexRunStatus {
    Success,
    CompletedWithErrors,
    Fatal,
}

impl TexRunStatus {
    #[must_use]
    pub const fn from_error_history(history: tex_state::print::ErrorHistory) -> Self {
        match history {
            tex_state::print::ErrorHistory::Spotless
            | tex_state::print::ErrorHistory::WarningIssued => Self::Success,
            tex_state::print::ErrorHistory::ErrorMessageIssued => Self::CompletedWithErrors,
            tex_state::print::ErrorHistory::FatalErrorStop => Self::Fatal,
        }
    }

    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }
}

/// A fully prepared downstream file that has not been materialized.
pub struct DriverFile {
    path: PathBuf,
    bytes: Vec<u8>,
}

impl DriverFile {
    #[must_use]
    pub fn new(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self { path, bytes }
    }
}

/// Finalization state before the engine's World effects have committed.
pub struct PlannedFinalization {
    publication: tex_exec::PreparedEnginePublication,
    files: Vec<DriverFile>,
}

/// A finalization effect commit that retained its downstream plan after a
/// retry-safe host failure.
pub enum FinalizationCommit {
    Committed(CommittedFinalization),
    Retry {
        plan: PlannedFinalization,
        failure: tex_exec::CompletionPublicationFailure,
    },
}

impl PlannedFinalization {
    pub fn new(
        publication: tex_exec::PreparedEnginePublication,
        files: Vec<DriverFile>,
    ) -> Result<Self, FinalizationError> {
        let mut paths = BTreeSet::new();
        for file in &files {
            if !paths.insert(lexically_normalize_path(&file.path)) {
                return Err(FinalizationError::ConflictingDriverPath(file.path.clone()));
            }
        }
        Ok(Self { publication, files })
    }

    #[must_use]
    pub fn pages(&self) -> &[tex_exec::DetachedPreparedPage] {
        self.publication.pages()
    }

    #[must_use]
    pub fn remaining_effects(&self) -> &[EffectRecord] {
        self.publication.remaining_effects()
    }

    pub fn retarget_stream_open(
        &mut self,
        failed: &tex_exec::CompletionPublicationFailure,
        replacement: &Path,
    ) -> Result<(), FinalizationError> {
        self.publication
            .retarget(failed, replacement.to_owned())
            .map_err(FinalizationError::Publication)
    }

    pub fn commit_effects(
        self,
        world: &mut World,
    ) -> Result<CommittedFinalization, FinalizationError> {
        match self.commit_effects_retryable(world)? {
            FinalizationCommit::Committed(committed) => Ok(committed),
            FinalizationCommit::Retry { failure, .. } => {
                Err(FinalizationError::RetryablePublication(failure))
            }
        }
    }

    /// Commits retained effects without consuming the downstream output plan
    /// when TeX82 §§1373--1375 permit retrying an unavailable stream open.
    ///
    /// `World` has already drained the successfully committed prefix in this
    /// case. Keeping this value lets the caller prompt and retarget the failed
    /// open, then resume the same pending suffix without rebuilding drivers or
    /// replaying engine effects.
    pub fn commit_effects_retryable(
        self,
        world: &mut World,
    ) -> Result<FinalizationCommit, FinalizationError> {
        match self.publication.publish(world)? {
            tex_exec::CompletionPublication::Committed(_) => {
                Ok(FinalizationCommit::Committed(CommittedFinalization {
                    files: self.files,
                }))
            }
            tex_exec::CompletionPublication::Retry { plan, failure } => {
                Ok(FinalizationCommit::Retry {
                    plan: Self {
                        publication: plan,
                        files: self.files,
                    },
                    failure,
                })
            }
        }
    }

    /// Explicit fixture policy: retain effect records and materialize nothing.
    pub fn discard_uncommitted(self) {}
}

fn lexically_normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized
                    .components()
                    .next_back()
                    .is_some_and(|last| matches!(last, Component::Normal(_)))
                {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

/// Finalization state that may materialize downstream files safely.
pub struct CommittedFinalization {
    files: Vec<DriverFile>,
}

impl CommittedFinalization {
    pub fn materialize(self, world: &mut World) -> Result<(), FinalizationError> {
        world.publish_files(
            self.files
                .into_iter()
                .map(|file| (file.path, file.bytes))
                .collect(),
        )?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum FinalizationError {
    ConflictingDriverPath(PathBuf),
    PreparedArtifact(String),
    Publication(tex_exec::EnginePublicationError),
    RetryablePublication(tex_exec::CompletionPublicationFailure),
    World(WorldError),
}

impl std::fmt::Display for FinalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConflictingDriverPath(path) => write!(
                f,
                "multiple downstream outputs resolve to {}",
                path.display()
            ),
            Self::PreparedArtifact(message) => {
                write!(f, "prepared page artifact finalization failed: {message}")
            }
            Self::Publication(error) => error.fmt(f),
            Self::RetryablePublication(failure) => write!(
                f,
                "retryable engine publication failed after {} effects: {}",
                failure.committed_prefix(),
                failure.message()
            ),
            Self::World(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for FinalizationError {}

impl From<WorldError> for FinalizationError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_exec::EnginePublicationError> for FinalizationError {
    fn from(value: tex_exec::EnginePublicationError) -> Self {
        Self::Publication(value)
    }
}

/// Installs the primitive/state setup used by an INITEX run.
///
/// TeX82 initializes only the category codes named in tex.web §232. In
/// particular, `{`, `}`, `$`, `&`, `#`, `^`, and `_` remain `other_char`
/// until the format source assigns them.
pub fn prepare_initex_stores<G>(stores: &mut Universe<G>) {
    stores
        .assign_int_param(
            IntParam::END_LINE_CHAR,
            13,
            tex_state::AssignmentScope::Global,
        )
        .expect("fresh end-line character assignment");
    tex_command::install_tex82_expandable_primitives(stores);
    tex_exec::install_unexpandable_primitives(stores);
    stores.intern("par");
}

/// Installs the primitive/state setup used by `umber run`.
///
/// `umber run` models a *format-loaded* engine -- the committed DVI corpora
/// are regenerated against a plain-format `pdftex`, not `pdftex -ini` (only
/// `tests/corpus/math`, whose sources carry their own `\catcode` preamble,
/// uses `--ini`). INITEX itself leaves `{ } $ & # ^ _` as `other_char`
/// (tex.web §232); the format assigns them, so Umber -- which has no dumped
/// plain format -- synthesizes that part of the format prelude here rather
/// than in [`Universe::new`]'s INITEX code tables.
pub fn prepare_run_stores<G>(stores: &mut Universe<G>) {
    prepare_initex_stores(stores);
    install_plain_catcodes(stores);
}

/// Installs the primitive/state setup used by `umber run --etex`.
pub fn prepare_etex_run_stores<G>(stores: &mut Universe<G>) {
    prepare_run_stores(stores);
    tex_command::install_etex_expandable_primitives(stores);
    tex_exec::install_etex_unexpandable_primitives(stores);
}

/// Installs the primitive/state setup used by `umber run --pdftex`.
pub fn prepare_pdftex_run_stores<G>(stores: &mut Universe<G>) {
    prepare_etex_run_stores(stores);
    pdftex::install_pdftex_layer(stores);
    stores.enable_pdf_output();
}

/// Restores driver-selected pdfTeX meanings after loading a format image.
pub fn install_pdftex_format_primitives<G>(stores: &mut Universe<G>) {
    tex_command::register_tex82_expandable_primitives(stores);
    tex_command::register_etex_expandable_primitives(stores);
    tex_exec::register_unexpandable_primitives(stores);
    tex_exec::register_etex_unexpandable_primitives(stores);
    pdftex::register_pdftex_layer(stores);
    stores.enable_pdf_output();
}

fn register_tex_format_primitives<G>(stores: &mut Universe<G>) {
    tex_command::register_tex82_expandable_primitives(stores);
    tex_exec::register_unexpandable_primitives(stores);
}

fn register_etex_format_primitives<G>(stores: &mut Universe<G>) {
    register_tex_format_primitives(stores);
    tex_command::register_etex_expandable_primitives(stores);
    tex_exec::register_etex_unexpandable_primitives(stores);
}

fn install_latex_compatibility_layer<G>(stores: &mut Universe<G>) {
    tex_command::install_latex_expandable_primitives(stores);
    let mut context = stores
        .command_context()
        .expect("fresh LaTeX compatibility admission");
    for character in ['{', '}', '$', '&', '#', '^', '_'] {
        context
            .assign_code(
                tex_state::CodeTableKind::Catcode,
                character,
                i64::from(tex_state::token::Catcode::Other as u8),
                tex_state::AssignmentScope::Global,
            )
            .expect("LaTeX compatibility catcode assignment");
    }
}

fn install_plain_catcodes<G>(stores: &mut Universe<G>) {
    let mut context = stores.command_context().expect("fresh plain admission");
    for (character, catcode) in [
        ('{', tex_state::token::Catcode::BeginGroup),
        ('}', tex_state::token::Catcode::EndGroup),
        ('$', tex_state::token::Catcode::MathShift),
        ('&', tex_state::token::Catcode::AlignmentTab),
        ('#', tex_state::token::Catcode::Parameter),
        ('^', tex_state::token::Catcode::Superscript),
        ('_', tex_state::token::Catcode::Subscript),
    ] {
        context
            .assign_code(
                tex_state::CodeTableKind::Catcode,
                character,
                i64::from(catcode as u8),
                tex_state::AssignmentScope::Global,
            )
            .expect("plain catcode assignment");
    }
}

/// Reconstructs the driver-selected LaTeX primitive registry after loading a format image.
pub fn install_latex_format_primitives<G>(stores: &mut Universe<G>) {
    register_etex_format_primitives(stores);
    tex_command::register_latex_expandable_primitives(stores);
}

/// Installs the primitive/state setup used by supported LaTeX-DVI runs.
///
/// This is an Umber extension layer over e-TeX. It intentionally does not
/// install pdfTeX identity or PDF-backend primitives.
pub fn prepare_latex_run_stores<G>(stores: &mut Universe<G>) {
    prepare_etex_run_stores(stores);
    install_latex_compatibility_layer(stores);
}

/// Installs the composed pdfTeX and LaTeX setup used by pdfLaTeX runs.
pub fn prepare_pdflatex_run_stores<G>(stores: &mut Universe<G>) {
    prepare_pdftex_run_stores(stores);
    install_latex_compatibility_layer(stores);
}

/// Reconstructs the composed pdfTeX and LaTeX primitive registry after format load.
pub fn install_pdflatex_format_primitives<G>(stores: &mut Universe<G>) {
    install_pdftex_format_primitives(stores);
    tex_command::register_latex_expandable_primitives(stores);
}

#[cfg(test)]
mod primitive_mode_tests {
    use super::*;
    use crate::EngineMode;
    use tex_state::World;
    use tex_state::env::banks::TokParam;
    use tex_state::ids::TokenListId;
    use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
    use tex_state::token::{Catcode, Token};

    #[test]
    fn composed_initex_setup_preserves_tex82_category_defaults() {
        for mode in [EngineMode::Tex82, EngineMode::ETex, EngineMode::PdfTex] {
            let mut stores = Universe::default();
            mode.prepare_initex(&mut stores);

            assert_eq!(stores.catcode('{'), Catcode::Other, "{}", mode.name());
            assert_eq!(stores.catcode('}'), Catcode::Other, "{}", mode.name());
            assert_eq!(stores.catcode('#'), Catcode::Other, "{}", mode.name());
            assert!(
                stores.primitive_token("catcode").is_some(),
                "{} INITEX primitives are installed",
                mode.name()
            );
        }
    }

    #[test]
    fn latex_format_restores_frozen_base_primitives_without_rebinding_live_names() {
        let mut stores = Universe::with_world(World::memory()).with_plain_catcodes();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi));

        install_latex_format_primitives(&mut stores);

        assert_eq!(
            stores.meaning(relax),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
            "format restoration must preserve the live meaning"
        );
        let frozen_relax = stores
            .primitive_token("relax")
            .expect("base primitive registry is reconstructed");
        assert_eq!(
            stores.frozen_primitive_meaning(frozen_relax),
            Some(Meaning::Relax)
        );
        assert!(stores.primitive_token("ifcsname").is_some());
    }

    #[test]
    fn protected_is_hidden_in_tex82_compatibility_mode() {
        let mut stores = Universe::default();
        prepare_run_stores(&mut stores);
        let protected = stores.intern("protected");
        assert_eq!(stores.meaning(protected), Meaning::Undefined);
        let readline = stores.intern("readline");
        assert_eq!(stores.meaning(readline), Meaning::Undefined);
        let everyeof = stores.intern("everyeof");
        assert_eq!(stores.meaning(everyeof), Meaning::Undefined);
        let errhelp = stores.intern("errhelp");
        assert_eq!(
            stores.meaning(errhelp),
            Meaning::TokParam(TokParam::ERR_HELP.raw())
        );
    }

    #[test]
    fn protected_is_installed_in_etex_extended_mode() {
        let mut stores = Universe::default();
        prepare_etex_run_stores(&mut stores);
        let protected = stores.intern("protected");
        assert_eq!(
            stores.meaning(protected),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Protected)
        );
        let readline = stores.intern("readline");
        assert_eq!(
            stores.meaning(readline),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ReadLine)
        );
        let everyeof = stores.intern("everyeof");
        assert_eq!(
            stores.meaning(everyeof),
            Meaning::TokParam(TokParam::EVERY_EOF.raw())
        );
        let errhelp = stores.intern("errhelp");
        assert_eq!(
            stores.meaning(errhelp),
            Meaning::TokParam(TokParam::ERR_HELP.raw())
        );
        assert_ne!(stores.meaning(errhelp), stores.meaning(everyeof));
    }

    #[test]
    fn errhelp_and_everyeof_assign_group_snapshot_hash_and_format_independently() {
        let mut stores = Universe::default();
        prepare_etex_run_stores(&mut stores);
        let output = run_memory_with_stores(
            concat!(
                "\\errhelp{help-outer}\\everyeof{eof-outer}",
                "{\\errhelp{help-inner}\\everyeof{eof-inner}",
                "\\message{local=[\\the\\errhelp]/[\\the\\everyeof]}}",
                "\\message{restored=[\\the\\errhelp]/[\\the\\everyeof]}",
                "{\\globaldefs=1\\errhelp{help-global}\\everyeof{eof-global}}",
                "\\end",
            ),
            &mut stores,
        )
        .expect("independent token parameters execute");
        assert!(
            output.contains("local=[help-inner]/[eof-inner]"),
            "{output}"
        );
        assert!(
            output.contains("restored=[help-outer]/[eof-outer]"),
            "{output}"
        );
        assert_eq!(token_list_text(&stores, TokParam::ERR_HELP), "help-global");
        assert_eq!(token_list_text(&stores, TokParam::EVERY_EOF), "eof-global");

        let committed = stores.snapshot();
        let changed_help = stores.intern_token_list(&[Token::Char {
            ch: 'H',
            cat: Catcode::Other,
        }]);
        let changed_eof = stores.intern_token_list(&[Token::Char {
            ch: 'E',
            cat: Catcode::Other,
        }]);
        stores.set_tok_param(TokParam::ERR_HELP, changed_help);
        stores.set_tok_param(TokParam::EVERY_EOF, changed_eof);
        assert_ne!(stores.snapshot().state_hash(), committed.state_hash());

        stores.rollback(&committed);
        assert_eq!(stores.snapshot().state_hash(), committed.state_hash());
        assert_eq!(token_list_text(&stores, TokParam::ERR_HELP), "help-global");
        assert_eq!(token_list_text(&stores, TokParam::EVERY_EOF), "eof-global");

        let mut format_stores = Universe::default();
        prepare_etex_run_stores(&mut format_stores);
        let format_help = intern_text(&mut format_stores, "help-format");
        let format_eof = intern_text(&mut format_stores, "eof-format");
        format_stores.set_tok_param_global(TokParam::ERR_HELP, format_help);
        format_stores.set_tok_param_global(TokParam::EVERY_EOF, format_eof);
        let format = format_stores.dump_format().expect("token parameter format");
        let loaded = Universe::from_format(World::default(), &format).expect("load format");
        assert_eq!(loaded.dump_format().expect("redump format"), format);
        assert_eq!(token_list_text(&loaded, TokParam::ERR_HELP), "help-format");
        assert_eq!(token_list_text(&loaded, TokParam::EVERY_EOF), "eof-format");
    }

    fn intern_text(stores: &mut Universe, text: &str) -> TokenListId {
        let tokens = text
            .chars()
            .map(|ch| Token::Char {
                ch,
                cat: Catcode::Other,
            })
            .collect::<Vec<_>>();
        stores.intern_token_list(&tokens)
    }

    fn token_list_text(stores: &Universe, parameter: TokParam) -> String {
        let id: TokenListId = stores.tok_param(parameter);
        stores
            .tokens(id)
            .iter()
            .filter_map(|token| match token {
                Token::Char { ch, .. } => Some(*ch),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn latex_extensions_are_isolated_from_plain_etex_mode() {
        let mut etex = Universe::default();
        prepare_etex_run_stores(&mut etex);
        let expanded = etex.intern("expanded");
        assert_eq!(etex.meaning(expanded), Meaning::Undefined);
        let strcmp = etex.intern("strcmp");
        assert_eq!(etex.meaning(strcmp), Meaning::Undefined);

        let mut latex = Universe::default();
        prepare_latex_run_stores(&mut latex);
        let expanded = latex.intern("expanded");
        assert_eq!(
            latex.meaning(expanded),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded)
        );
        let strcmp = latex.intern("strcmp");
        assert_eq!(
            latex.meaning(strcmp),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::StringCompare)
        );
        let ifincsname = latex.intern("ifincsname");
        assert_eq!(
            latex.meaning(ifincsname),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfInCsName)
        );
        assert_eq!(latex.catcode('{'), Catcode::Other);
        assert_eq!(latex.catcode('#'), Catcode::Other);
        assert_eq!(latex.catcode('A'), Catcode::Letter);
        assert_eq!(latex.catcode('\\'), Catcode::Escape);
        let pdftex_version = latex.intern("pdftexversion");
        assert_eq!(latex.meaning(pdftex_version), Meaning::Undefined);

        let mut latex_initex = Universe::default();
        EngineMode::Latex.prepare_initex(&mut latex_initex);
        let pdftex_version = latex_initex.intern("pdftexversion");
        assert_eq!(latex_initex.meaning(pdftex_version), Meaning::Undefined);
    }

    #[test]
    fn pdflatex_composes_pdftex_and_latex_layers() {
        let mut stores = Universe::default();
        prepare_pdflatex_run_stores(&mut stores);

        let pdfoutput = stores.intern("pdfoutput");
        assert_eq!(
            stores.meaning(pdfoutput),
            Meaning::IntParam(IntParam::PDF_OUTPUT.raw())
        );
        let strcmp = stores.intern("strcmp");
        assert_eq!(
            stores.meaning(strcmp),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::StringCompare)
        );
        assert_eq!(stores.catcode('{'), Catcode::Other);
        assert_eq!(stores.catcode('#'), Catcode::Other);
        assert!(stores.pdf_output_enabled());
    }

    #[test]
    fn format_startup_reconstructs_each_engine_primitive_registry_without_overwriting_meanings() {
        for (mode, primitive) in [
            (EngineMode::Tex82, "relax"),
            (EngineMode::ETex, "unless"),
            (EngineMode::PdfTex, "pdfprimitive"),
            (EngineMode::Latex, "strcmp"),
            (EngineMode::PdfLatex, "strcmp"),
        ] {
            let mut source = Universe::default();
            mode.prepare_fresh(&mut source);
            let original = source
                .primitive_meaning(primitive)
                .unwrap_or_else(|| panic!("{} must register {primitive}", mode.name()));
            let symbol = source.intern(primitive);
            source.set_meaning(symbol, Meaning::Undefined);
            let format = source
                .dump_format()
                .expect("dump shadowed primitive format");

            let mut loaded =
                Universe::from_format(World::default(), &format).expect("load engine format");
            assert_eq!(loaded.primitive_meaning(primitive), None);
            mode.install_after_format(&mut loaded);

            let symbol = loaded.intern(primitive);
            assert_eq!(
                loaded.meaning(symbol),
                Meaning::Undefined,
                "{}",
                mode.name()
            );
            assert_eq!(
                loaded.primitive_meaning(primitive),
                Some(original),
                "{}",
                mode.name()
            );
            let frozen = loaded
                .primitive_token(primitive)
                .expect("primitive token is reconstructed");
            assert_eq!(loaded.frozen_primitive_meaning(frozen), Some(original));
        }
    }

    #[test]
    fn etex_expandable_primitives_follow_driver_mode() {
        let mut compatibility = Universe::default();
        prepare_run_stores(&mut compatibility);
        let unexpanded = compatibility.intern("unexpanded");
        let detokenize = compatibility.intern("detokenize");
        let unless = compatibility.intern("unless");
        let scantokens = compatibility.intern("scantokens");
        let etex_version = compatibility.intern("eTeXversion");
        let etex_revision = compatibility.intern("eTeXrevision");
        let ifdefined = compatibility.intern("ifdefined");
        let ifcsname = compatibility.intern("ifcsname");
        let currentgrouplevel = compatibility.intern("currentgrouplevel");
        let currentgrouptype = compatibility.intern("currentgrouptype");
        let currentiflevel = compatibility.intern("currentiflevel");
        let currentiftype = compatibility.intern("currentiftype");
        let currentifbranch = compatibility.intern("currentifbranch");
        let lastnodetype = compatibility.intern("lastnodetype");
        let iffontchar = compatibility.intern("iffontchar");
        let fontcharwd = compatibility.intern("fontcharwd");
        let fontcharht = compatibility.intern("fontcharht");
        let fontchardp = compatibility.intern("fontchardp");
        let fontcharic = compatibility.intern("fontcharic");
        let interactionmode = compatibility.intern("interactionmode");
        let tracingscantokens = compatibility.intern("tracingscantokens");
        let numexpr = compatibility.intern("numexpr");
        let dimexpr = compatibility.intern("dimexpr");
        let glueexpr = compatibility.intern("glueexpr");
        let muexpr = compatibility.intern("muexpr");
        let gluestretch = compatibility.intern("gluestretch");
        let glueshrink = compatibility.intern("glueshrink");
        let gluestretchorder = compatibility.intern("gluestretchorder");
        let glueshrinkorder = compatibility.intern("glueshrinkorder");
        let gluetomu = compatibility.intern("gluetomu");
        let mutoglue = compatibility.intern("mutoglue");
        let showtokens = compatibility.intern("showtokens");
        let showgroups = compatibility.intern("showgroups");
        let showifs = compatibility.intern("showifs");
        let tex_xet_state = compatibility.intern("TeXXeTstate");
        let predisplaydirection = compatibility.intern("predisplaydirection");
        assert_eq!(compatibility.meaning(unexpanded), Meaning::Undefined);
        assert_eq!(compatibility.meaning(detokenize), Meaning::Undefined);
        assert_eq!(compatibility.meaning(unless), Meaning::Undefined);
        assert_eq!(compatibility.meaning(scantokens), Meaning::Undefined);
        for symbol in [
            etex_version,
            etex_revision,
            ifdefined,
            ifcsname,
            currentgrouplevel,
            currentgrouptype,
            currentiflevel,
            currentiftype,
            currentifbranch,
            lastnodetype,
            iffontchar,
            fontcharwd,
            fontcharht,
            fontchardp,
            fontcharic,
            interactionmode,
            tracingscantokens,
            numexpr,
            dimexpr,
            glueexpr,
            muexpr,
            gluestretch,
            glueshrink,
            gluestretchorder,
            glueshrinkorder,
            gluetomu,
            mutoglue,
            showtokens,
            showgroups,
            showifs,
            tex_xet_state,
            predisplaydirection,
        ] {
            assert_eq!(compatibility.meaning(symbol), Meaning::Undefined);
        }
        let wvo_primitives = [
            "marks",
            "topmarks",
            "firstmarks",
            "botmarks",
            "splitfirstmarks",
            "splitbotmarks",
            "pagediscards",
            "splitdiscards",
            "clubpenalties",
            "widowpenalties",
            "displaywidowpenalties",
            "interlinepenalties",
            "parshapelength",
            "parshapeindent",
            "parshapedimen",
            "lastlinefit",
            "savinghyphcodes",
            "savingvdiscards",
        ];
        for name in wvo_primitives {
            let symbol = compatibility.intern(name);
            assert_eq!(compatibility.meaning(symbol), Meaning::Undefined, "{name}");
        }

        let mut extended = Universe::default();
        prepare_etex_run_stores(&mut extended);
        for name in wvo_primitives {
            let symbol = extended.intern(name);
            assert_ne!(extended.meaning(symbol), Meaning::Undefined, "{name}");
        }
        let unexpanded = extended.intern("unexpanded");
        let detokenize = extended.intern("detokenize");
        let unless = extended.intern("unless");
        let scantokens = extended.intern("scantokens");
        assert_eq!(
            extended.meaning(unexpanded),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded)
        );
        assert_eq!(
            extended.meaning(detokenize),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize)
        );
        assert_eq!(
            extended.meaning(unless),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unless)
        );
        assert_eq!(
            extended.meaning(scantokens),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Scantokens)
        );
        let version = extended.intern("eTeXversion");
        assert_eq!(
            extended.meaning(version),
            Meaning::InternalInteger(tex_state::meaning::InternalInteger::ETeXVersion)
        );
        for (name, value) in [
            (
                "currentgrouplevel",
                tex_state::meaning::InternalInteger::CurrentGroupLevel,
            ),
            (
                "currentgrouptype",
                tex_state::meaning::InternalInteger::CurrentGroupType,
            ),
            (
                "currentiflevel",
                tex_state::meaning::InternalInteger::CurrentIfLevel,
            ),
            (
                "currentiftype",
                tex_state::meaning::InternalInteger::CurrentIfType,
            ),
            (
                "currentifbranch",
                tex_state::meaning::InternalInteger::CurrentIfBranch,
            ),
            (
                "lastnodetype",
                tex_state::meaning::InternalInteger::LastNodeType,
            ),
        ] {
            let symbol = extended.intern(name);
            assert_eq!(extended.meaning(symbol), Meaning::InternalInteger(value));
        }
        for (name, primitive) in [
            ("eTeXrevision", ExpandablePrimitive::ETeXRevision),
            ("ifdefined", ExpandablePrimitive::IfDefined),
            ("ifcsname", ExpandablePrimitive::IfCsName),
            ("iffontchar", ExpandablePrimitive::IfFontChar),
        ] {
            let symbol = extended.intern(name);
            assert_eq!(
                extended.meaning(symbol),
                Meaning::ExpandablePrimitive(primitive)
            );
        }
        let ifincsname = extended.intern("ifincsname");
        assert_eq!(extended.meaning(ifincsname), Meaning::Undefined);
        for (name, primitive) in [
            ("fontcharwd", UnexpandablePrimitive::FontCharWd),
            ("fontcharht", UnexpandablePrimitive::FontCharHt),
            ("fontchardp", UnexpandablePrimitive::FontCharDp),
            ("fontcharic", UnexpandablePrimitive::FontCharIc),
            ("numexpr", UnexpandablePrimitive::NumExpr),
            ("dimexpr", UnexpandablePrimitive::DimExpr),
            ("glueexpr", UnexpandablePrimitive::GlueExpr),
            ("muexpr", UnexpandablePrimitive::MuExpr),
            ("gluestretch", UnexpandablePrimitive::GlueStretch),
            ("glueshrink", UnexpandablePrimitive::GlueShrink),
            ("gluestretchorder", UnexpandablePrimitive::GlueStretchOrder),
            ("glueshrinkorder", UnexpandablePrimitive::GlueShrinkOrder),
            ("gluetomu", UnexpandablePrimitive::GlueToMu),
            ("mutoglue", UnexpandablePrimitive::MuToGlue),
            ("showtokens", UnexpandablePrimitive::ShowTokens),
            ("showgroups", UnexpandablePrimitive::ShowGroups),
            ("showifs", UnexpandablePrimitive::ShowIfs),
            ("interactionmode", UnexpandablePrimitive::InteractionMode),
            ("beginL", UnexpandablePrimitive::BeginL),
            ("endL", UnexpandablePrimitive::EndL),
            ("beginR", UnexpandablePrimitive::BeginR),
            ("endR", UnexpandablePrimitive::EndR),
            ("middle", UnexpandablePrimitive::Middle),
        ] {
            let symbol = extended.intern(name);
            assert_eq!(
                extended.meaning(symbol),
                Meaning::UnexpandablePrimitive(primitive)
            );
        }
        let tracingscantokens = extended.intern("tracingscantokens");
        assert_eq!(
            extended.meaning(tracingscantokens),
            Meaning::IntParam(tex_state::env::banks::IntParam::TRACING_SCAN_TOKENS.raw())
        );
        for (name, parameter) in [
            (
                "TeXXeTstate",
                tex_state::env::banks::IntParam::TEX_XET_STATE,
            ),
            (
                "predisplaydirection",
                tex_state::env::banks::IntParam::PRE_DISPLAY_DIRECTION,
            ),
            (
                "tracingassigns",
                tex_state::env::banks::IntParam::TRACING_ASSIGNS,
            ),
            (
                "tracinggroups",
                tex_state::env::banks::IntParam::TRACING_GROUPS,
            ),
            ("tracingifs", tex_state::env::banks::IntParam::TRACING_IFS),
            (
                "tracingnesting",
                tex_state::env::banks::IntParam::TRACING_NESTING,
            ),
        ] {
            let symbol = extended.intern(name);
            assert_eq!(extended.meaning(symbol), Meaning::IntParam(parameter.raw()));
        }
    }
}

/// Runs one retained root through canonical main control.
pub fn run_input_with_context<G>(
    stores: &mut Universe<G>,
    request: RetainedRootRequest,
    host: &mut dyn ResourceHost,
) -> Result<String, SessionError> {
    run_input_collecting_artifacts(stores, request, host).map(|result| result.terminal_text)
}

/// Runs one retained root with the explicitly selected command profile.
pub fn run_input_with_context_and_profile<G>(
    stores: &mut Universe<G>,
    request: RetainedRootRequest,
    host: &mut dyn ResourceHost,
    profile: CommandProfile,
) -> Result<String, SessionError> {
    run_input_collecting_artifacts_with_profile(stores, request, host, profile)
        .map(|result| result.terminal_text)
}

/// Runs input and returns the artifact ids emitted by `\shipout` in order.
pub fn run_input_collecting_artifacts<G>(
    stores: &mut Universe<G>,
    request: RetainedRootRequest,
    host: &mut dyn ResourceHost,
) -> Result<RunResult, SessionError> {
    run_retained_root(stores, request, host)
}

/// Runs input under an explicitly selected command profile and returns its artifacts.
///
/// Primitive/state preparation and command-profile selection are separate host
/// responsibilities. In particular, a pdfTeX store must be paired with
/// [`CommandProfile::PDFTEX14029`] so shipout finalizes PDF-only deferred nodes as
/// PDF rather than applying the exact DVI-mode rejection.
pub fn run_input_collecting_artifacts_with_profile<G>(
    stores: &mut Universe<G>,
    mut request: RetainedRootRequest,
    host: &mut dyn ResourceHost,
    profile: CommandProfile,
) -> Result<RunResult, SessionError> {
    request.profile = profile;
    run_retained_root(stores, request, host)
}

/// Reads committed page artifacts from `World` and writes a complete DVI file.
pub fn dvi_from_artifacts<G>(
    stores: &Universe<G>,
    artifacts: &[ContentHash],
) -> Result<Vec<u8>, DviBuildError> {
    write_dvi_from_artifacts(stores, artifacts, Vec::new())
}

/// Writes a complete DVI file directly from in-process shipout commit receipts.
///
/// Unlike [`dvi_from_artifacts`], this does not reread or rehash the durable
/// content-addressed store. Parsing and validation remain identical.
pub fn dvi_from_committed_artifacts(
    artifacts: &[CommittedArtifact],
) -> Result<Vec<u8>, DviBuildError> {
    write_dvi_from_committed_artifacts(artifacts, Vec::new())
}

/// Assembles DVI from page-local bodies compiled before shipout commit.
pub fn dvi_from_page_plans(plans: &[DviPagePlan]) -> Result<Vec<u8>, DviBuildError> {
    write_dvi_from_page_plans(plans, Vec::new())
}

pub fn write_dvi_from_page_plans<W: std::io::Write>(
    plans: &[DviPagePlan],
    sink: W,
) -> Result<W, DviBuildError> {
    let mut writer = DviStreamWriter::new(sink);
    for plan in plans {
        writer.write_page_plan(plan)?;
    }
    Ok(writer.finish()?)
}

pub fn write_dvi_from_committed_artifacts<W: std::io::Write>(
    artifacts: &[CommittedArtifact],
    sink: W,
) -> Result<W, DviBuildError> {
    let mut writer = DviStreamWriter::new(sink);
    for committed in artifacts {
        let plan = DviPagePlan::compile_v10(committed.bytes())?;
        writer.write_page_plan(&plan)?;
    }
    Ok(writer.finish()?)
}

/// Decodes, validates, emits, and drops each artifact before loading the next.
pub fn write_dvi_from_artifacts<G, W: std::io::Write>(
    stores: &Universe<G>,
    artifacts: &[ContentHash],
    sink: W,
) -> Result<W, DviBuildError> {
    let mut writer = DviStreamWriter::new(sink);
    for &hash in artifacts {
        let bytes = stores
            .world()
            .read_artifact(hash)?
            .ok_or(DviBuildError::MissingArtifact(hash))?;
        let plan = DviPagePlan::compile_v10(&bytes)?;
        writer.write_page_plan(&plan)?;
    }
    Ok(writer.finish()?)
}

/// Writes standalone HTML directly from successful in-process shipout receipts.
///
/// Font acquisition is an explicit downstream capability and never reaches
/// back into live engine state.
pub fn html_from_committed_artifacts<R: tex_out::html::HtmlFontAssets>(
    artifacts: &[CommittedArtifact],
    assets: &R,
    options: &tex_out::html::HtmlOptions,
) -> Result<tex_out::html::HtmlOutput, HtmlBuildError> {
    let pages = artifacts
        .iter()
        .map(|artifact| tex_out::PageArtifact::from_bytes(artifact.bytes()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tex_out::html::write_html(&pages, assets, options)?)
}

/// Replays durable artifacts through the HTML driver one page at a time.
pub fn html_from_artifacts<G, R: tex_out::html::HtmlFontAssets>(
    stores: &Universe<G>,
    artifacts: &[ContentHash],
    assets: &R,
    options: &tex_out::html::HtmlOptions,
) -> Result<tex_out::html::HtmlOutput, HtmlBuildError> {
    let mut pages = Vec::with_capacity(artifacts.len());
    for &hash in artifacts {
        let bytes = stores
            .world()
            .read_artifact(hash)?
            .ok_or(HtmlBuildError::MissingArtifact(hash))?;
        pages.push(tex_out::PageArtifact::from_bytes(&bytes)?);
    }
    Ok(tex_out::html::write_html(&pages, assets, options)?)
}

/// Runs in-memory TeX through the `umber run` executor setup.
pub fn run_memory_with_stores<G>(
    source: &str,
    stores: &mut Universe<G>,
) -> Result<String, SessionError> {
    run_memory_collecting_artifacts(source, stores).map(|result| result.terminal_text)
}

/// Runs in-memory input with an explicit command profile and output backend.
pub fn run_memory_with_stores_and_profile<G>(
    source: &str,
    stores: &mut Universe<G>,
    profile: CommandProfile,
    emit_dvi: bool,
) -> Result<String, SessionError> {
    run_memory_collecting_artifacts_with_profile(source, stores, profile, emit_dvi)
        .map(|result| result.terminal_text)
}

/// Runs in-memory TeX and preserves its completed status and artifacts.
pub fn run_memory_collecting_artifacts<G>(
    source: &str,
    stores: &mut Universe<G>,
) -> Result<RunResult, SessionError> {
    run_memory_collecting_artifacts_with_profile(source, stores, CommandProfile::TEX82, true)
}

/// Runs in-memory input with an explicit profile while preserving its status.
pub fn run_memory_collecting_artifacts_with_profile<G>(
    source: &str,
    stores: &mut Universe<G>,
    profile: CommandProfile,
    emit_dvi: bool,
) -> Result<RunResult, SessionError> {
    let _ = emit_dvi;
    let mut host = FileSessionResolvers::new(Path::new("texput.tex"), Vec::new(), Vec::new());
    let mut session = EngineSession::new(stores, profile);
    session.project_terminal_text_to_root_body();
    session.register_retained_fragment_with_invocation(
        "texput",
        "texput",
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source.as_bytes()),
        ),
    )?;
    session.run(&mut host, &mut NoCheckpoints)
}

fn uncommitted_terminal_text<G>(stores: &Universe<G>) -> String {
    terminal_text_from_effects(stores.world().effect_records())
}

fn terminal_text_from_effects(records: &[EffectRecord]) -> String {
    let mut text = String::new();
    for record in records {
        let EffectRecord::StreamWrite { sink, text: chunk } = record else {
            continue;
        };
        match sink {
            PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log => {
                text.push_str(chunk);
            }
            PrintSink::Stream(_) => {}
        }
    }
    text
}

#[derive(Debug)]
pub enum DviBuildError {
    MissingArtifact(ContentHash),
    World(WorldError),
    Parse(tex_out::ParseError),
    Dvi(DviError),
}

impl std::fmt::Display for DviBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingArtifact(hash) => {
                write!(f, "shipped page artifact {} is missing", hash.hex())
            }
            Self::World(err) => write!(f, "{err}"),
            Self::Parse(err) => write!(f, "{err}"),
            Self::Dvi(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for DviBuildError {}

impl From<WorldError> for DviBuildError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_out::ParseError> for DviBuildError {
    fn from(value: tex_out::ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<DviError> for DviBuildError {
    fn from(value: DviError) -> Self {
        Self::Dvi(value)
    }
}

#[derive(Debug)]
pub enum HtmlBuildError {
    MissingArtifact(ContentHash),
    World(WorldError),
    Parse(tex_out::ParseError),
    Html(tex_out::html::HtmlError),
}

impl std::fmt::Display for HtmlBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingArtifact(hash) => {
                write!(f, "shipped page artifact {} is missing", hash.hex())
            }
            Self::World(error) => error.fmt(f),
            Self::Parse(error) => error.fmt(f),
            Self::Html(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for HtmlBuildError {}

impl From<WorldError> for HtmlBuildError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_out::ParseError> for HtmlBuildError {
    fn from(value: tex_out::ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<tex_out::html::HtmlError> for HtmlBuildError {
    fn from(value: tex_out::html::HtmlError) -> Self {
        Self::Html(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DriverFile, FinalizationCommit, FinalizationError, PlannedFinalization, TexRunStatus,
        prepare_pdftex_run_stores, run_input_collecting_artifacts_with_profile,
        terminal_text_from_effects,
    };
    use crate::FileSessionResolvers;
    use std::path::{Path, PathBuf};
    use tex_command::{CommandProfile, RegisteredSourceKind};
    use tex_state::{PrintSink, StreamSlot, Universe, World};

    const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

    fn publication(source: &str) -> tex_exec::PreparedEnginePublication {
        let mut session = crate::VirtualCompileSession::new(crate::SessionOptions::default())
            .expect("finalization test session");
        session
            .add_user_file("main.tex", source.as_bytes().to_vec())
            .expect("finalization test source");
        assert!(matches!(
            session.compile_attempt(),
            crate::CompileAttemptResult::Complete(_)
        ));
        session
            .into_accepted_finalization()
            .expect("accepted finalization")
            .completion
            .into_publication()
            .expect("prepared engine publication")
    }

    #[test]
    fn tex_run_status_preserves_web2c_history_threshold() {
        use tex_state::print::ErrorHistory;

        assert_eq!(
            TexRunStatus::from_error_history(ErrorHistory::Spotless),
            TexRunStatus::Success
        );
        assert_eq!(
            TexRunStatus::from_error_history(ErrorHistory::WarningIssued),
            TexRunStatus::Success
        );
        assert_eq!(
            TexRunStatus::from_error_history(ErrorHistory::ErrorMessageIssued),
            TexRunStatus::CompletedWithErrors
        );
        assert_eq!(
            TexRunStatus::from_error_history(ErrorHistory::FatalErrorStop),
            TexRunStatus::Fatal
        );
    }

    #[test]
    fn tracingcommands_display_end_probe_reports_restored_mode() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);
        crate::run_memory_with_stores(
            "\\nonstopmode\\tracingcommands=2\\tracingonline=1\\noindent$$\\vtop{\\noindent$$Aa$\\ifvmode$\\fi}\\hss\\end",
            &mut stores,
        )
        .expect("run completes");
        let output = String::from_utf8_lossy(
            stores
                .world()
                .memory_log_output()
                .expect("memory-backed log"),
        )
        .into_owned();
        assert!(
            output.contains("{math shift character $}\n! Math formula deleted:")
                && output.contains("{internal vertical mode: \\ifvmode}\n{true}"),
            "{output}"
        );
    }

    #[test]
    fn display_end_probe_mode_conditionals_share_the_restored_mode() {
        // TeX82 §§1185/1194/1197: `fin_mlist` restores internal vertical
        // mode before the expanded second-dollar probe evaluates these
        // conditionals. Close the enclosing display too, so unrelated final
        // cleanup recovery cannot print the branch markers as source context.
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);
        crate::run_memory_with_stores(
            "\\nonstopmode\\tracingcommands=2\\tracingonline=1\\noindent$$\\vtop{\\noindent$$$\\ifvmode\\else\\errmessage{ifvmode stale}\\fi\\ifhmode\\errmessage{ifhmode stale}\\fi\\ifmmode\\errmessage{ifmmode stale}\\fi$}\\hss$$\\end",
            &mut stores,
        )
        .expect("run completes");
        let output = String::from_utf8_lossy(
            stores
                .world()
                .memory_log_output()
                .expect("memory-backed log"),
        );
        assert!(
            output.contains("{internal vertical mode: \\ifvmode}\n{true}")
                && output.contains("{\\ifhmode}\n{false}")
                && output.contains("{\\ifmmode}\n{false}"),
            "{output}"
        );
        for rejected_diagnostic in [
            "! ifvmode stale.",
            "! ifhmode stale.",
            "! ifmmode stale.",
            "! Missing $ inserted.",
            "! Display math should end with $$.",
        ] {
            assert!(!output.contains(rejected_diagnostic), "{output}");
        }
    }

    #[test]
    fn tracingcommands_same_mode_conditional_omits_mode_prefix() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);
        crate::run_memory_with_stores(
            "\\nonstopmode\\tracingcommands=2\\tracingonline=1\\ifvmode\\fi\\noindent$$\\vtop{\\noindent$$Aa$\\ifvmode$\\fi}\\hss\\end",
            &mut stores,
        )
        .expect("run completes");
        let output = String::from_utf8_lossy(
            stores
                .world()
                .memory_log_output()
                .expect("memory-backed log"),
        );
        assert!(
            output.contains("{vertical mode: \\tracingonline}\n{\\ifvmode}\n{true}"),
            "{output}"
        );
        assert!(!output.contains("{vertical mode: \\ifvmode}"), "{output}");
    }

    #[test]
    fn halign_preamble_span_expansion_traces_the_pushed_mode() {
        // TeX82 §§299, 367, 759, and 774: this is TRIP's `#\span\iftrue`
        // sequence. The packing scan has already completed when the preamble
        // processor expands `\iftrue`, so that distinct episode must publish
        // the internal-vertical mode pushed by `init_align`.
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);
        crate::run_memory_with_stores(
            "\\nonstopmode\\tracingcommands=2\\tracingonline=1\\halign{&#\\span\\iftrue\\relax\\span\\else\\span\\fi\\span&#\\cr a&b\\cr}\\end",
            &mut stores,
        )
        .expect("run completes");
        let output = String::from_utf8_lossy(
            stores
                .world()
                .memory_log_output()
                .expect("memory-backed log"),
        );

        assert!(
            output.contains("{internal vertical mode: \\iftrue}\n{true}"),
            "{output}"
        );
    }

    #[test]
    fn halign_first_entry_body_handoff_stays_inside_main_loop_trace() {
        // TeX82 §§789, 1034, and 1038: the u-template's final `A` enters
        // main_loop. Its bare lookahead fetches the adjacent first body `A`
        // without returning to §1030's traced reswitch boundary.
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed cmr10");
        crate::run_memory_with_stores(
            "\\nonstopmode\\font\\tracefont=cmr10 \\tracefont\\tracingcommands=2\\tracingonline=1\\halign{A#\\cr A\\cr}\\end",
            &mut stores,
        )
        .expect("run completes");
        let output = String::from_utf8_lossy(
            stores
                .world()
                .memory_log_output()
                .expect("memory-backed log"),
        );
        assert_eq!(output.matches("the letter A}").count(), 1, "{output}");
        assert!(
            output.contains(
                "{restricted horizontal mode: the letter A}\n{end of alignment template}"
            ),
            "{output}"
        );
    }

    #[test]
    fn halign_entry_trace_preserves_genuinely_separate_repeated_characters() {
        // A non-character returns to §1030's big_switch. The following `A`
        // is therefore a genuinely distinct traced reswitch, even though it
        // has the same token value as the template and first body tokens.
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("seed cmr10");
        crate::run_memory_with_stores(
            "\\nonstopmode\\font\\tracefont=cmr10 \\tracefont\\tracingcommands=2\\tracingonline=1\\halign{A#\\cr A\\kern0pt A\\cr}\\end",
            &mut stores,
        )
        .expect("run completes");
        let output = String::from_utf8_lossy(
            stores
                .world()
                .memory_log_output()
                .expect("memory-backed log"),
        );
        assert_eq!(output.matches("the letter A}").count(), 2, "{output}");
        assert!(
            output.contains("{restricted horizontal mode: the letter A}\n{\\kern}\n{the letter A}"),
            "{output}"
        );
    }

    #[test]
    fn valign_packing_scan_preserves_same_horizontal_mode_capability() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);
        crate::run_memory_with_stores(
            "\\nonstopmode\\tracingcommands=2\\tracingonline=1\\hbox{\\valign to \\ifhmode13pt\\else\\errmessage{valign mode stale}26pt\\fi{#\\cr y\\cr}}\\end",
            &mut stores,
        )
        .expect("run completes");
        let output = String::from_utf8_lossy(
            stores
                .world()
                .memory_log_output()
                .expect("memory-backed log"),
        );

        assert!(
            output.contains("{restricted horizontal mode: \\valign}\n{\\ifhmode}\n{true}"),
            "{output}"
        );
        assert!(!output.contains("valign mode stale"), "{output}");
    }

    #[test]
    fn public_file_root_retains_world_identity_across_typed_input_retry() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);
        stores
            .world_mut()
            .set_memory_file("/project/root.tex", b"\\message{root}\\input child \\end")
            .expect("root is staged");
        stores
            .world_mut()
            .set_memory_file("/project/child.tex", b"\\message{child}")
            .expect("child is staged");
        let root = stores
            .world_mut()
            .read_file("/project/root.tex")
            .expect("root is selected");
        let root_hash = root.hash();
        let mut host = super::FileSessionResolvers::new(
            Path::new("/project/root.tex"),
            Vec::new(),
            Vec::new(),
        );

        let result = super::run_input_collecting_artifacts(
            &mut stores,
            super::RetainedRootRequest::file("root", root, CommandProfile::TEX82),
            &mut host,
        )
        .expect("file-root run completes");

        assert!(result.terminal_text.contains("root"));
        assert!(result.terminal_text.contains("child"));
        assert!(matches!(
            stores.world().input_records(),
            [root_record, child_record]
                if root_record.hash() == root_hash
                    && root_record.path() == Path::new("/project/root.tex")
                    && child_record.path() == Path::new("/project/child.tex")
        ));
    }

    #[test]
    fn deferred_pdf_nodes_follow_the_explicit_session_profile() {
        let source = "\\pdfoutput=1\\shipout\\hbox{\\pdfliteral{q}}\\end";
        for profile in [CommandProfile::TEX82, CommandProfile::ETEX26] {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            let mut host = super::FileSessionResolvers::new(
                Path::new("profile-boundary.tex"),
                Vec::new(),
                Vec::new(),
            );
            let error = run_input_collecting_artifacts_with_profile(
                &mut stores,
                super::RetainedRootRequest::authored_job(
                    "profile-boundary",
                    source.as_bytes(),
                    profile,
                ),
                &mut host,
                profile,
            )
            .expect_err("TeX and e-TeX profiles must traverse deferred nodes in DVI mode");
            assert_eq!(
                error.to_string(),
                "pdfTeX error (ext4): \\pdfliteral used while \\pdfoutput is not set."
            );
        }

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let mut host = super::FileSessionResolvers::new(
            Path::new("profile-boundary.tex"),
            Vec::new(),
            Vec::new(),
        );
        let result = run_input_collecting_artifacts_with_profile(
            &mut stores,
            super::RetainedRootRequest::authored_job(
                "profile-boundary",
                source.as_bytes(),
                CommandProfile::PDFTEX14029,
            ),
            &mut host,
            CommandProfile::PDFTEX14029,
        )
        .expect("pdfTeX profile accepts deferred PDF nodes in PDF mode");
        assert_eq!(result.committed_artifacts.len(), 1);
    }

    #[test]
    fn retained_memory_roots_project_local_text_and_keep_effect_cursors_rollbackable() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_run_stores(&mut stores);

        let run = |source: &'static [u8], stores: &mut Universe| {
            let mut host =
                FileSessionResolvers::new(Path::new("texput.tex"), Vec::new(), Vec::new());
            let mut session = crate::EngineSession::new(stores, CommandProfile::TEX82);
            session.project_terminal_text_to_root_body();
            session
                .register_retained_fragment_with_invocation(
                    "texput",
                    "texput",
                    tex_command::SourceRegistration::new(RegisteredSourceKind::Generated, source),
                )
                .expect("root registers");
            session
                .run(&mut host, &mut super::NoCheckpoints)
                .expect("retained root completes")
        };

        let first = run(b"\\count0=7\\message{first}\\end", &mut stores);
        assert_eq!(first.terminal_text, " first");
        assert_eq!(
            terminal_text_from_effects(&first.effects),
            "(texput first )"
        );
        let after_first = stores.snapshot();

        let second = run(
            b"\\advance\\count0 by1\\message{second=\\the\\count0}\\end",
            &mut stores,
        );
        assert_eq!(second.terminal_text, " second=8");
        assert_eq!(
            terminal_text_from_effects(&second.effects),
            "(texput second=8 )"
        );
        assert!(!terminal_text_from_effects(&second.effects).contains("first"));
        let second_hash = stores.snapshot().state_hash();

        stores.rollback(&after_first);
        assert_eq!(stores.count(0), 7);
        let replay = run(
            b"\\advance\\count0 by1\\message{second=\\the\\count0}\\end",
            &mut stores,
        );
        assert_eq!(replay.terminal_text, second.terminal_text);
        assert_eq!(stores.snapshot().state_hash(), second_hash);
    }

    #[test]
    fn direct_file_host_retries_world_font_and_image_in_fresh_and_format_sessions() {
        use crate::EngineMode;

        let mode = EngineMode::PdfTex;
        let mut format_stores = Universe::new_with_plain_catcodes();
        mode.prepare_fresh(&mut format_stores);
        let format = format_stores.dump_format().expect("base format dumps");
        let mut png = vec![0_u8; 29];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&1_u32.to_be_bytes());
        png[20..24].copy_from_slice(&1_u32.to_be_bytes());
        png[24] = 8;
        png[25] = 0;
        let root = b"\\font\\tenrm=cmr10 \\tenrm A\\pdfoutput=1\\pdfximage {image.png}\\end";

        for loaded in [false, true] {
            let mut world = World::memory();
            world
                .set_memory_file("/project/job.tex", root)
                .expect("root is seeded");
            world
                .set_memory_file("/project/cmr10.tfm", CMR10)
                .expect("font is seeded");
            world
                .set_memory_file("/project/image.png", png.clone())
                .expect("image is seeded");
            let mut stores = if loaded {
                let mut stores = Universe::from_format(world, &format).expect("format restores");
                mode.install_after_format(&mut stores);
                stores
            } else {
                let mut stores = Universe::with_world(world);
                mode.prepare_fresh(&mut stores);
                stores
            };
            let selected_root = stores
                .world_mut()
                .read_file("/project/job.tex")
                .expect("World selects the root");
            let root_hash = selected_root.hash();
            let mut session = crate::EngineSession::new(&mut stores, mode.command_profile());
            session
                .register_world_root("job", selected_root)
                .expect("selected root registers unchanged");
            let mut host =
                crate::FileSessionResolvers::new(Path::new("/project/job.tex"), vec![], vec![]);
            let run = session
                .run(&mut host, &mut Vec::new())
                .expect("typed retries complete");

            assert!(run.format_dump.is_none());
            assert!(session.stores().pdf_last_external_image().is_some());
            let records = session.stores().world().input_records();
            assert_eq!(records.len(), 3);
            assert_eq!(records[0].hash(), root_hash);
            assert_eq!(records[1].path(), Path::new("/project/cmr10.tfm"));
            assert_eq!(records[2].path(), Path::new("/project/image.png"));
        }
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // Verifies real host ordering at the World boundary.
    fn driver_materialization_follows_engine_effect_commit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let output = temp.path().join("shared.out");
        let publication = publication(&format!(
            "\\openout1={} \\write1{{engine}}\\closeout1\\end",
            output.display()
        ));
        let plan = PlannedFinalization::new(
            publication,
            vec![DriverFile::new(output.clone(), b"driver".to_vec())],
        )
        .expect("paths are distinct");
        let mut world = World::real();

        plan.commit_effects(&mut world)
            .expect("effects commit")
            .materialize(&mut world)
            .expect("driver materializes");

        assert_eq!(std::fs::read(output).expect("read output"), b"driver");
    }

    #[test]
    fn failed_effect_commit_cannot_materialize_driver_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let publication = publication(&format!(
            "\\openout1={} \\write1{{cannot write a directory}}\\end",
            temp.path().display()
        ));
        let driver_path = temp.path().join("driver.dvi");
        let plan = PlannedFinalization::new(
            publication,
            vec![DriverFile::new(driver_path.clone(), b"driver".to_vec())],
        )
        .expect("paths are distinct");
        let mut world = World::real();

        assert!(plan.commit_effects(&mut world).is_err());
        assert!(!driver_path.exists());
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // Verifies retry ordering against the real backend.
    fn retryable_finalization_keeps_plan_and_does_not_replay_committed_prefix() {
        let temp = tempfile::tempdir().expect("temp dir");
        let prefix_path = temp.path().join("prefix.out");
        let replacement_path = temp.path().join("replacement.out");
        let driver_path = temp.path().join("driver.dvi");
        let publication = publication(&format!(
            "\\openout1={} \\write1{{once}}\\closeout1 \\openout2={} \\write2{{suffix}}\\end",
            prefix_path.display(),
            temp.path().display()
        ));
        let plan = PlannedFinalization::new(
            publication,
            vec![DriverFile::new(driver_path.clone(), b"driver".to_vec())],
        )
        .expect("plan");
        let mut world = World::real();

        let FinalizationCommit::Retry { mut plan, failure } = plan
            .commit_effects_retryable(&mut world)
            .expect("retry-safe failure is retained")
        else {
            panic!("directory open must suspend finalization");
        };
        assert_eq!(failure.path(), Some(temp.path()));
        assert_eq!(
            std::fs::read(&prefix_path).expect("committed prefix"),
            b"once\n"
        );
        assert!(!driver_path.exists());

        plan.retarget_stream_open(&failure, &replacement_path)
            .expect("retarget pending open");
        let FinalizationCommit::Committed(committed) = plan
            .commit_effects_retryable(&mut world)
            .expect("replacement commits")
        else {
            panic!("replacement must finish the retained plan");
        };
        committed
            .materialize(&mut world)
            .expect("driver materializes");

        assert_eq!(
            std::fs::read(prefix_path).expect("prefix remains"),
            b"once\n"
        );
        assert_eq!(
            std::fs::read(replacement_path).expect("suffix commits"),
            b"suffix\n"
        );
        assert_eq!(std::fs::read(driver_path).expect("driver"), b"driver");
    }

    #[test]
    fn duplicate_driver_paths_are_rejected_before_finalization() {
        let result = PlannedFinalization::new(
            publication("\\end"),
            vec![
                DriverFile::new(PathBuf::from("same.out"), vec![1]),
                DriverFile::new(PathBuf::from("same.out"), vec![2]),
            ],
        );
        assert!(matches!(
            result,
            Err(FinalizationError::ConflictingDriverPath(path)) if path == std::path::Path::new("same.out")
        ));
    }

    #[test]
    fn lexically_aliased_driver_paths_are_rejected_before_finalization() {
        let result = PlannedFinalization::new(
            publication("\\end"),
            vec![
                DriverFile::new(PathBuf::from("out"), vec![1]),
                DriverFile::new(PathBuf::from("./out"), vec![2]),
            ],
        );
        assert!(matches!(
            result,
            Err(FinalizationError::ConflictingDriverPath(path)) if path == std::path::Path::new("./out")
        ));

        let result = PlannedFinalization::new(
            publication("\\end"),
            vec![
                DriverFile::new(PathBuf::from("build/out"), vec![1]),
                DriverFile::new(PathBuf::from("build/tmp/../out"), vec![2]),
            ],
        );
        assert!(matches!(
            result,
            Err(FinalizationError::ConflictingDriverPath(path)) if path == std::path::Path::new("build/tmp/../out")
        ));
    }

    #[test]
    fn fixture_policy_preserves_effects_without_materializing_files() {
        let publication = publication("\\message{fixture}\\end");
        let plan = PlannedFinalization::new(
            publication,
            vec![DriverFile::new(PathBuf::from("fixture.dvi"), vec![1])],
        )
        .expect("path is unique");

        plan.discard_uncommitted();

        let world = World::memory();
        assert_eq!(world.memory_terminal_output(), Some([].as_slice()));
        assert_eq!(world.memory_output("fixture.dvi"), None);
    }
}
