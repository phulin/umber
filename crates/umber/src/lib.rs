use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tex_command::{
    CommandProfile, FontResource, PdfImageRequest, PdfImageResource, RegisteredSourceKind,
    SourceRegistration, SourceRegistrationError,
};
use tex_exec::{
    CanonicalMainControl, CanonicalStepResult, CheckpointSink, EngineBoundary,
    ExecutionBudgetCounters, ExecutionContext, ExecutionStats, Executor, FontResolver,
    MainControlStep, PdfImageRequest as LegacyPdfImageRequest, PdfImageResolver,
    try_execute_assignment,
};
use tex_expand::{InputResolver, get_x_token_with_context};
use tex_lex::{InputSource, InputStack};
use tex_out::dvi::{DviError, DviPagePlan, DviStreamWriter};
use tex_state::env::banks::IntParam;
use tex_state::token::TracedTokenWord;
use tex_state::{
    CommittedArtifact, ContentHash, EffectPos, EffectRecord, ExpansionContext, FileContent,
    PrintSink, Universe, WorldCommitMode, WorldError,
};

mod canonical_session;
#[cfg(not(target_arch = "wasm32"))]
pub mod cli_resource;
mod editor_session;
mod fixed_point;
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
mod pdf_vf;
mod pdftex;
#[cfg(not(target_arch = "wasm32"))]
mod prepared_format;
mod tex_fixed_point;
mod virtual_compile;

pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub use canonical_session::{
    CanonicalEngineSession, CanonicalResourceFulfillment, CanonicalResourceHost,
    CanonicalResourceOutcome, CanonicalResourceWorld, CanonicalSessionError, CanonicalSessionState,
    CanonicalStartupInput, DEFAULT_CANONICAL_NO_PROGRESS_LIMIT,
};
pub use editor_session::{
    EditorCompileSession, EditorResourceError, EditorSessionOptions, EditorSessionStatus,
    EditorStabilizationAttempt,
};
pub use fixed_point::FixedPointLimits;
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
    PdfBuildError, pdf_from_committed_artifacts, pdf_from_committed_artifacts_at_dpi,
    pdf_from_committed_artifacts_with_virtual_fonts,
};
pub use pdftex::PDFTEX_PRIMITIVE_NAMES;
#[cfg(not(target_arch = "wasm32"))]
pub use prepared_format::{PreparedFormatJob, PreparedFormatProvider};
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
pub use tex_state::{InputDependency, InputDependencyAccess, InputDependencyOutcome};
pub use umber_vfs::FileContentId;

/// Complete immutable startup capability for one retained canonical run.
pub struct RetainedRootRequest {
    pub startup_name: String,
    pub invocation: String,
    pub profile: CommandProfile,
    pub source: SourceRegistration,
}

impl RetainedRootRequest {
    #[must_use]
    pub fn authored(
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
        }
    }
}

struct NoCanonicalCheckpoints;

impl CheckpointSink for NoCanonicalCheckpoints {
    fn wants_checkpoint(&self, _boundary: EngineBoundary) -> bool {
        false
    }

    fn checkpoint(&mut self, _checkpoint: tex_exec::EngineCheckpoint) {}
}

/// Runs one retained immutable root through canonical main control.
pub fn run_retained_root(
    stores: &mut Universe,
    request: RetainedRootRequest,
    host: &mut dyn CanonicalResourceHost,
) -> Result<RunResult, CanonicalSessionError> {
    let mut session = CanonicalEngineSession::new(stores, request.profile);
    session.register_retained_root_with_invocation(
        &request.startup_name,
        &request.invocation,
        request.source,
    )?;
    session.run(host, &mut NoCanonicalCheckpoints)
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
    CompileDiagnostic, CompileError, CompileSourceLocation, CompileTelemetry,
    CompositeResolverError, CompositeResourceResolver, DriverResourceClosure, EngineMode, FileKind,
    FileRequest, FileRequestKey, MissingOutputResource, NeedResources,
    OUTPUT_RESOURCE_PLAN_VERSION, OutputCapability, OutputCapabilitySet, OutputResourcePlan,
    PdfVirtualFontResources, PlannedResource, ProviderFailure, ProviderResponse,
    RenderedSourceLocation, RenderedSourceResult, RequestKeyError, ResolvedFile, ResolvedPkFont,
    ResourceClosureOwner, ResourceDomain, ResourcePlanError, ResourcePurpose, ResourceReason,
    ResourceRequest, ResourceRequestMode, ResourceResponse, RetentionMetrics, SessionLimits,
    SessionOptions, SourcePatch, TypedResourceProvider, VfsLimitError, VfsLimitKind, VfsLimits,
    VirtualCompileSession, VirtualPath, VirtualPathError,
};

/// The only checkpoint policy supported by composed engine sessions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointPolicy {
    NamedExecutorBoundaries,
}

/// Exclusive composition boundary for input, context, state, diagnostics, and artifacts.
pub struct EngineSession<'a, 'context> {
    input: &'a mut InputStack,
    stores: &'a mut Universe,
    context: ExecutionContext<'context>,
    /// The pending production command machine.  The selected run loop remains
    /// legacy until the cutover issue, but every new session owns canonical
    /// command state from startup so host-provided immutable resources can be
    /// registered without consulting `InputStack`.
    canonical: CanonicalMainControl,
    /// A root capability selects the canonical run loop.  Sessions without
    /// one retain the compatibility adapter while callers are migrated to
    /// register their retained root bytes at construction.
    canonical_root_registered: bool,
    artifact_cursor: usize,
    checkpoint_policy: CheckpointPolicy,
}

impl<'a, 'context> EngineSession<'a, 'context> {
    pub fn new(
        input: &'a mut InputStack,
        stores: &'a mut Universe,
        context: ExecutionContext<'context>,
    ) -> Self {
        // The selected engine profile/format has already installed meanings
        // in `stores`; constructing this bridge must not mutate that shared
        // state or re-register primitive identities.
        Self::with_command_profile(input, stores, context, CommandProfile::TEX82)
    }

    /// Constructs a session whose canonical processor is pinned to the same
    /// profile selected by fresh or format startup.
    pub fn with_command_profile(
        input: &'a mut InputStack,
        stores: &'a mut Universe,
        context: ExecutionContext<'context>,
        profile: CommandProfile,
    ) -> Self {
        let artifact_cursor = stores.world().artifact_commits().len();
        let canonical = CanonicalMainControl::with_profile(profile);
        Self {
            input,
            stores,
            context,
            canonical,
            canonical_root_registered: false,
            artifact_cursor,
            checkpoint_policy: CheckpointPolicy::NamedExecutorBoundaries,
        }
    }

    /// Returns the profile pinned into this session's canonical processor.
    #[must_use]
    pub fn canonical_command_profile(&self) -> CommandProfile {
        self.canonical.command_profile()
    }

    /// Publishes a canonical named boundary without deriving continuation
    /// state from the legacy input stack.
    pub fn capture_canonical_checkpoint(
        &mut self,
        boundary: tex_exec::EngineBoundary,
        budget_counters: tex_exec::ExecutionBudgetCounters,
    ) -> Result<tex_exec::EngineCheckpoint, tex_command::CommandSummaryError> {
        self.canonical
            .capture_checkpoint(boundary, self.stores, budget_counters)
    }

    /// Restores a canonical named boundary into this session's processor.
    pub fn restore_canonical_checkpoint(
        &mut self,
        checkpoint: &tex_exec::EngineCheckpoint,
    ) -> Result<(), tex_exec::CanonicalCheckpointRestoreError> {
        self.canonical.restore_checkpoint(checkpoint, self.stores)
    }

    #[must_use]
    pub const fn checkpoint_policy(&self) -> CheckpointPolicy {
        self.checkpoint_policy
    }

    #[must_use]
    pub fn stores(&self) -> &Universe {
        self.stores
    }

    pub fn stores_mut(&mut self) -> &mut Universe {
        self.stores
    }

    /// Registers the already-acquired immutable root for the canonical
    /// command machine and selects the job identity exposed by `\jobname`.
    ///
    /// This deliberately does not inspect the legacy `InputStack`: callers
    /// transfer the original registration selected by their World/editor
    /// policy, including any World input-record provenance.
    pub fn register_canonical_retained_root(
        &mut self,
        job_name: &str,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        self.canonical
            .capabilities_mut()
            .set_startup_job_name(job_name);
        let source = self.canonical.register_root_source(source)?;
        self.canonical_root_registered = true;
        Ok(source)
    }

    /// Registers a root selected through the active World without rebuilding
    /// its source identity or provenance from bytes.
    pub fn register_canonical_world_root(
        &mut self,
        job_name: &str,
        content: tex_state::FileContent,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        self.register_canonical_retained_root(job_name, SourceRegistration::world(content))
    }

    /// Registers a deliberately authored in-memory root.
    ///
    /// This byte helper is not suitable for a root selected through World;
    /// use [`Self::register_canonical_world_root`] in that case.
    pub fn register_canonical_authored_root(
        &mut self,
        job_name: &str,
        bytes: Arc<[u8]>,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        self.register_canonical_retained_root(
            job_name,
            SourceRegistration::new(RegisteredSourceKind::Generated, bytes),
        )
    }

    #[cfg(test)]
    fn register_canonical_root(
        &mut self,
        job_name: &str,
        kind: RegisteredSourceKind,
        bytes: Arc<[u8]>,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        assert_eq!(kind, RegisteredSourceKind::Generated);
        self.register_canonical_authored_root(job_name, bytes)
    }

    /// Makes a completed host input acquisition available to a future typed
    /// `\input` request.  Registration is capability-scoped; command state
    /// receives the bytes only after it has scanned that request.
    pub fn provide_canonical_retained_input(
        &mut self,
        name: impl Into<String>,
        source: SourceRegistration,
    ) {
        self.canonical
            .capabilities_mut()
            .register_input(name, source);
    }

    /// Registers deliberately authored in-memory input bytes. World-selected
    /// inputs must use [`Self::provide_canonical_world_input`].
    pub fn provide_canonical_authored_input(&mut self, name: impl Into<String>, bytes: Arc<[u8]>) {
        self.provide_canonical_retained_input(
            name,
            SourceRegistration::new(RegisteredSourceKind::Generated, bytes),
        );
    }

    #[cfg(test)]
    fn provide_canonical_input(
        &mut self,
        name: impl Into<String>,
        kind: RegisteredSourceKind,
        bytes: Arc<[u8]>,
    ) {
        assert_eq!(kind, RegisteredSourceKind::Generated);
        self.provide_canonical_authored_input(name, bytes);
    }

    /// Convenience adapter for a `World` input that has already been
    /// selected and recorded by the existing resolver policy.
    pub fn provide_canonical_world_input(
        &mut self,
        name: impl Into<String>,
        content: tex_state::FileContent,
    ) {
        self.provide_canonical_retained_input(name, SourceRegistration::world(content));
    }

    /// Registers a host-acquired immutable font resource for a suspended
    /// canonical `\font` request. `path` is the canonical request path (for
    /// example `cmr10.tfm`), never a retained host handle.
    pub fn provide_canonical_font(&mut self, path: impl Into<PathBuf>, resource: FontResource) {
        self.canonical
            .capabilities_mut()
            .register_font(path, resource);
    }

    /// Registers the complete host result of an exact canonical
    /// `\\pdfximage` request.  This is deliberately the same retained-byte
    /// contract used by direct callers and all higher-level session drivers.
    pub fn provide_canonical_pdf_image(
        &mut self,
        request: PdfImageRequest,
        resource: PdfImageResource,
    ) {
        self.canonical
            .capabilities_mut()
            .register_pdf_image(request, resource);
    }

    /// Advances one aggregate canonical operation and exposes a typed
    /// resource suspension to every direct caller. Supplying immutable bytes
    /// through the corresponding `provide_canonical_*` method then retries
    /// the same bounded TeX82 operation from its rollback boundary.
    pub fn advance_canonical(&mut self) -> Result<CanonicalStepResult, tex_exec::ExecError> {
        self.canonical.advance(self.stores)
    }

    /// Advances exactly one canonical main-control operation.  Effects and
    /// artifacts are still committed through `World`, so callers can retain
    /// the ordinary executor transaction boundary around this typed step.
    pub fn step_canonical(&mut self) -> Result<MainControlStep, tex_exec::ExecError> {
        self.canonical.step(self.stores)
    }

    /// Borrows the canonical driver for typed lifecycle operations such as
    /// alignment requests.  It exposes no legacy source-consumption API.
    #[must_use]
    pub fn canonical_main_control_mut(&mut self) -> &mut CanonicalMainControl {
        &mut self.canonical
    }

    pub fn execute(&mut self) -> Result<RunResult, tex_exec::ExecError> {
        if self.canonical_root_registered {
            return self.execute_canonical(None);
        }
        let artifact_start = self.artifact_cursor;
        let stats = Executor::new().run_with_context(self.input, self.stores, &mut self.context)?;
        Ok(self.finish_execution(artifact_start, stats))
    }

    /// Executes while publishing restartable state at named safe boundaries.
    pub fn execute_with_checkpoints<C: CheckpointSink>(
        &mut self,
        checkpoints: &mut C,
    ) -> Result<RunResult, tex_exec::ExecError> {
        if self.canonical_root_registered {
            return self.execute_canonical(Some(checkpoints));
        }
        let artifact_start = self.artifact_cursor;
        let stats = Executor::new().run_with_context_and_checkpoints(
            self.input,
            self.stores,
            &mut self.context,
            checkpoints,
        )?;
        Ok(self.finish_execution(artifact_start, stats))
    }

    /// Runs a source capability through the single canonical TeX82 command
    /// machine.  The legacy `InputStack` is deliberately not observed here:
    /// it remains only for compatibility sessions that have not yet supplied
    /// retained root bytes to the host bridge.
    fn execute_canonical(
        &mut self,
        checkpoints: Option<&mut dyn CheckpointSink>,
    ) -> Result<RunResult, tex_exec::ExecError> {
        let artifact_start = self.artifact_cursor;
        let mut committed_steps = 0_u64;
        let mut mode_transitions = vec![self.canonical.current_mode()];
        if let Some(sink) = checkpoints
            && sink.wants_checkpoint(EngineBoundary::JobStart)
        {
            let checkpoint = self
                .capture_canonical_checkpoint(
                    EngineBoundary::JobStart,
                    ExecutionBudgetCounters {
                        committed_steps,
                        cumulative_fuel: 0,
                    },
                )
                .map_err(|_| tex_exec::ExecError::MissingToken {
                    context: "canonical checkpoint",
                })?;
            sink.checkpoint(checkpoint);
        }
        while let MainControlStep::Continue = self.step_canonical()? {
            committed_steps = committed_steps.saturating_add(1);
            let mode = self.canonical.current_mode();
            if mode_transitions.last() != Some(&mode) {
                mode_transitions.push(mode);
            }
        }
        let committed = self.stores.world().artifact_commits();
        let receipts = self.canonical.take_prepared_dvi_pages();
        let committed_artifacts = self.stores.world().committed_artifacts();
        let run_artifacts = &committed[artifact_start..];
        let run_committed = &committed_artifacts[artifact_start..];
        let emits_dvi = !self
            .canonical
            .command_profile()
            .capabilities()
            .supports_pdftex();
        if (emits_dvi && receipts.len() != run_artifacts.len())
            || receipts
                .iter()
                .zip(run_artifacts)
                .any(|(receipt, hash)| receipt.hash() != *hash)
        {
            return Err(tex_exec::ExecError::InvalidShipoutArtifact(
                "canonical DVI receipts are not aligned with committed artifacts".into(),
            ));
        }
        self.artifact_cursor = committed.len();
        Ok(RunResult {
            terminal_text: uncommitted_terminal_text(self.stores),
            mode_transitions,
            fatal: self.canonical.fatal_error(),
            artifacts: run_artifacts.to_vec(),
            dvi_pages: receipts
                .into_iter()
                .map(tex_exec::PreparedDviPage::into_plan)
                .collect(),
            committed_artifacts: run_committed.to_vec(),
            effects: self.stores.world().effect_records().to_vec(),
            dumped_format: self.canonical.dumped_format(),
            format_dump_receipt: self.canonical.format_dump_receipt().cloned(),
        })
    }

    fn finish_execution(&mut self, artifact_start: usize, stats: ExecutionStats) -> RunResult {
        let committed = self.stores.world().artifact_commits();
        debug_assert_eq!(
            &committed[self.artifact_cursor..],
            stats.shipped_artifacts.as_slice()
        );
        self.artifact_cursor = committed.len();
        RunResult {
            terminal_text: uncommitted_terminal_text(self.stores),
            mode_transitions: Vec::new(),
            fatal: None,
            artifacts: stats.shipped_artifacts,
            dvi_pages: stats.dvi_pages,
            committed_artifacts: self.stores.world().committed_artifacts()
                [artifact_start..self.artifact_cursor]
                .to_vec(),
            effects: self.stores.world().effect_records().to_vec(),
            dumped_format: stats.dumped_format,
            format_dump_receipt: stats.format_dump_receipt,
        }
    }

    pub fn next_expanded_token(
        &mut self,
    ) -> Result<Option<TracedTokenWord>, tex_expand::ExpandError> {
        let mut expansion = ExpansionContext::new(self.stores);
        get_x_token_with_context(self.input, &mut expansion, &mut self.context)
    }

    pub fn try_execute_assignment(
        &mut self,
        token: TracedTokenWord,
    ) -> Result<bool, tex_exec::ExecError> {
        try_execute_assignment(token, self.input, self.stores, &mut self.context)
    }

    pub fn publish_input_summary(&mut self) {
        let summary = self.input.publication_summary(self.stores);
        self.stores.set_input_summary(summary);
    }
}

/// Shared file search and job identity policy for run-like commands.
pub struct FileSessionResolvers {
    input: FileInputResolver,
    font: FileFontResolver,
    image: FileImageResolver,
    job_name: String,
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
        let job_name = path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("texput")
            .to_owned();
        let input_search = TexInputSearchPath::new(&base_dir, tex_input_areas);
        Self {
            input: FileInputResolver(input_search.clone()),
            font: FileFontResolver(TexFontSearchPath::new(base_dir, tex_font_areas)),
            image: FileImageResolver(input_search),
            job_name,
        }
    }

    pub fn context(&mut self) -> ExecutionContext<'_> {
        ExecutionContext::with_resource_resolvers(
            &self.job_name,
            &mut self.input,
            &mut self.font,
            &mut self.image,
        )
    }

    /// Acquires every mapline-selected font program and encoding through the
    /// driver's configured font search path. PDF finalization remains
    /// host-neutral and consumes only validated resources in engine state.
    pub fn provide_pdf_font_programs(&self, stores: &mut Universe) -> Result<(), String> {
        self.provide_pdf_font_programs_at_dpi(stores, pdf_output::DEFAULT_PDF_PK_RESOLUTION)
    }

    /// Variant used by hosts that configure a non-default bitmap device DPI.
    pub fn provide_pdf_font_programs_at_dpi(
        &self,
        stores: &mut Universe,
        driver_dpi: i32,
    ) -> Result<(), String> {
        provide_pdf_font_resources_at_dpi(stores, driver_dpi, |stores, name| {
            let logical_name = String::from_utf8_lossy(name);
            self.font
                .0
                .read_program_from_world(stores.world_mut(), Path::new(logical_name.as_ref()))
                .map(|content| content.bytes().to_vec())
        })
    }

    /// Borrows the input and font resolvers for an incremental editor session.
    pub fn resolvers(&mut self) -> (&mut dyn InputResolver, &mut dyn FontResolver) {
        (&mut self.input, &mut self.font)
    }
}

impl CanonicalResourceHost for FileSessionResolvers {
    fn fulfill(
        &mut self,
        world: &mut CanonicalResourceWorld<'_>,
        need: &tex_exec::CanonicalResourceNeed,
    ) -> CanonicalResourceOutcome {
        match need {
            tex_exec::CanonicalResourceNeed::Input { name } => {
                if let Some(result) = self
                    .input
                    .0
                    .read_restricted_pipe_from_canonical_world(world, name)
                {
                    return result.map_or(CanonicalResourceOutcome::Unavailable, |text| {
                        CanonicalResourceOutcome::Fulfilled(CanonicalResourceFulfillment::input(
                            name,
                            RegisteredSourceKind::Generated,
                            Arc::from(text.into_bytes()),
                        ))
                    });
                }
                self.input
                    .0
                    .read_from_canonical_world(world, name)
                    .ok()
                    .map_or(CanonicalResourceOutcome::Unavailable, |content| {
                        CanonicalResourceOutcome::Fulfilled(
                            CanonicalResourceFulfillment::world_input(name, content),
                        )
                    })
            }
            tex_exec::CanonicalResourceNeed::Font { request } => {
                let mut path = PathBuf::from(&request.name);
                if path.extension().is_none() {
                    path.set_extension("tfm");
                }
                CanonicalResourceOutcome::Fulfilled(
                    self.font
                        .0
                        .read_from_canonical_world(world, &path)
                        .map_or_else(
                            |_| CanonicalResourceFulfillment::Font {
                                request: request.clone(),
                                resource: Box::new(FontResource::Unavailable),
                            },
                            |metrics| CanonicalResourceFulfillment::Font {
                                request: request.clone(),
                                resource: Box::new(FontResource::Tfm {
                                    metrics,
                                    opentype: None,
                                }),
                            },
                        ),
                )
            }
            tex_exec::CanonicalResourceNeed::PdfImage { request } => {
                let Ok(content) = self
                    .image
                    .0
                    .read_exact_from_canonical_world(world, &request.name)
                else {
                    return CanonicalResourceOutcome::Unavailable;
                };
                let legacy = LegacyPdfImageRequest {
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
                CanonicalResourceOutcome::Fulfilled(CanonicalResourceFulfillment::PdfImage {
                    request: request.clone(),
                    resource: Box::new(resource),
                })
            }
        }
    }
}

pub(crate) fn provide_pdf_font_resources_at_dpi(
    stores: &mut Universe,
    driver_dpi: i32,
    acquire: impl FnMut(&mut Universe, &[u8]) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    provide_pdf_font_resources_excluding_at_dpi(stores, driver_dpi, &BTreeSet::new(), acquire)
}

pub(crate) fn provide_pdf_font_resources_excluding_at_dpi(
    stores: &mut Universe,
    driver_dpi: i32,
    excluded_names: &BTreeSet<Vec<u8>>,
    mut acquire: impl FnMut(&mut Universe, &[u8]) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let used_names = stores
        .pdf_font_resources()
        .filter_map(|resource| {
            let name = stores.font(resource.font()).name().as_bytes().to_vec();
            (!excluded_names.contains(&name)).then_some(name)
        })
        .collect::<BTreeSet<_>>();
    if used_names.is_empty() {
        return Ok(());
    }
    let explicitly_requests_default = stores.pdf_font_maps().any(|operation| {
        matches!(
            operation,
            tex_state::PdfFontMapOperation::File(file)
                if file.logical_name == b"pdftex.map"
        )
    });
    let mut implicit_default = false;
    for name in stores.pdf_font_map_file_requests() {
        if stores.has_pdf_font_map_file(&name) {
            continue;
        }
        if name == b"pdftex.map" && !explicitly_requests_default {
            implicit_default = true;
            continue;
        }
        let bytes = acquire(stores, &name)?;
        stores
            .provide_pdf_font_map_file(name, &bytes)
            .map_err(|error| error.to_string())?;
    }
    let mapped_names = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .map(|entry| entry.tex_name)
        .collect::<BTreeSet<_>>();
    let covered_names = mapped_names
        .into_iter()
        .chain(stores.authoritative_pdf_font_map_names())
        .collect::<BTreeSet<_>>();
    if implicit_default && !used_names.is_subset(&covered_names) {
        let name = b"pdftex.map".to_vec();
        let bytes = acquire(stores, &name)?;
        stores
            .provide_pdf_font_map_file(name, &bytes)
            .map_err(|error| error.to_string())?;
    }
    let encodings = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .filter(|entry| used_names.contains(&entry.tex_name))
        .flat_map(|entry| entry.encoding_files)
        .collect::<std::collections::BTreeSet<_>>();
    for name in encodings {
        if stores.pdf_encoding(&name).is_some() {
            continue;
        }
        let bytes = acquire(stores, &name)?;
        stores
            .provide_pdf_encoding(name, &bytes)
            .map_err(|error| error.to_string())?;
    }
    let names = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .filter(|entry| used_names.contains(&entry.tex_name))
        .filter_map(|entry| entry.font_file)
        .collect::<std::collections::BTreeSet<_>>();
    for name in names {
        let is_truetype = pdf_output::is_pdf_sfnt_program(&name);
        if (is_truetype && stores.pdf_truetype_program(&name).is_some())
            || (!is_truetype && stores.pdf_type1_program(&name).is_some())
        {
            continue;
        }
        let bytes = acquire(stores, &name)?;
        if is_truetype {
            stores
                .provide_pdf_truetype_program(name, &bytes)
                .map_err(|error| error.to_string())?;
        } else {
            stores
                .provide_pdf_type1_program(name, &bytes)
                .map_err(|error| error.to_string())?;
        }
    }
    let mapped_names = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .filter(|entry| used_names.contains(&entry.tex_name))
        .map(|entry| entry.tex_name)
        .collect::<BTreeSet<_>>();
    let requests = stores
        .pdf_font_resources()
        .filter_map(|resource| {
            let font = stores.font(resource.font());
            (used_names.contains(font.name().as_bytes())
                && !mapped_names.contains(font.name().as_bytes()))
            .then(|| pdf_output::pk_font_request(stores, resource.font(), driver_dpi))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    for request in requests {
        if stores.pdf_pk_font(&request).is_some() {
            continue;
        }
        let logical_name = request.logical_name();
        let bytes = acquire(stores, &logical_name)?;
        stores
            .provide_pdf_pk_font(request, &bytes)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod pdf_font_resources_tests;

struct FileInputResolver(TexInputSearchPath);

impl InputResolver for FileInputResolver {
    fn open_input(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        name: &str,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<Box<dyn InputSource>> {
        if let Some(output) = self.0.read_restricted_pipe(input, name) {
            return output.map(|text| {
                tex_expand::ResourceLookup::Available(
                    Box::new(tex_lex::WorldInput::generated(text)) as Box<dyn InputSource>,
                )
            });
        }
        Ok(match self.0.read(input, name) {
            Ok(content) => tex_expand::ResourceLookup::Available(Box::new(
                tex_lex::WorldInput::from_content(content),
            )
                as Box<dyn InputSource>),
            Err(_) => tex_expand::ResourceLookup::Unavailable,
        })
    }

    fn input_file_size(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        name: &str,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<u64> {
        Ok(match self.0.read(input, name) {
            Ok(content) => tex_expand::ResourceLookup::Available(
                u64::try_from(content.bytes().len()).unwrap_or(u64::MAX),
            ),
            Err(_) => tex_expand::ResourceLookup::Unavailable,
        })
    }

    fn open_stream_input(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        name: &str,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<tex_state::FileContent> {
        Ok(match self.0.read(input, name) {
            Ok(content) => tex_expand::ResourceLookup::Available(content),
            Err(_) => tex_expand::ResourceLookup::Unavailable,
        })
    }
}

struct FileFontResolver(TexFontSearchPath);

struct FileImageResolver(TexInputSearchPath);

impl PdfImageResolver for FileImageResolver {
    fn open_image(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        request: &LegacyPdfImageRequest,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<tex_state::PdfExternalImageSource> {
        let content = match self.0.read(input, &request.name) {
            Ok(content) => content,
            Err(_) => return Ok(tex_expand::ResourceLookup::Unavailable),
        };
        virtual_compile::parse_image(&content, request).map(tex_expand::ResourceLookup::Available)
    }
}

impl FontResolver for FileFontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        path: &Path,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<tex_exec::FontSource> {
        Ok(match self.0.read(input, path) {
            Ok(metrics) => tex_expand::ResourceLookup::Available(tex_exec::FontSource::Tfm {
                metrics,
                opentype: None,
            }),
            Err(_) => tex_expand::ResourceLookup::Unavailable,
        })
    }
}

/// Result of running TeX through the batch executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunResult {
    pub terminal_text: String,
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
    pub dumped_format: bool,
    pub format_dump_receipt: Option<tex_exec::FormatDumpReceipt>,
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
    effect_pos: EffectPos,
    files: Vec<DriverFile>,
    prepared_pages: Option<tex_state::PreparedPageSuffix>,
}

/// A finalization effect commit that retained its downstream plan after a
/// retry-safe host failure.
pub enum FinalizationCommit {
    Committed(CommittedFinalization),
    Retry {
        plan: PlannedFinalization,
        error: WorldError,
    },
}

impl PlannedFinalization {
    pub fn new(effect_pos: EffectPos, files: Vec<DriverFile>) -> Result<Self, FinalizationError> {
        let mut paths = BTreeSet::new();
        for file in &files {
            if !paths.insert(lexically_normalize_path(&file.path)) {
                return Err(FinalizationError::ConflictingDriverPath(file.path.clone()));
            }
        }
        Ok(Self {
            effect_pos,
            files,
            prepared_pages: None,
        })
    }

    #[must_use]
    pub fn with_prepared_pages(mut self, pages: Option<tex_state::PreparedPageSuffix>) -> Self {
        self.prepared_pages = pages;
        self
    }

    pub fn retarget_stream_open(
        &mut self,
        stores: &mut Universe,
        failed: &tex_state::StreamOpenFailure,
        replacement: &Path,
    ) -> Result<(), FinalizationError> {
        let Some(mut pages) = self.prepared_pages.clone() else {
            return Err(FinalizationError::PreparedArtifact(
                "the failed stream open has no prepared page suffix".to_owned(),
            ));
        };
        let failed_path = failed.path().to_string_lossy();
        let replacement_text = replacement.to_string_lossy();
        pages
            .effects()
            .iter()
            .position(|(position, effect)| {
                *position == failed.position()
                    && matches!(
                        effect,
                        tex_state::EffectRecord::StreamOpen { slot, target }
                            if *slot == failed.slot() && target.path() == failed.path()
                    )
            })
            .ok_or_else(|| {
                FinalizationError::PreparedArtifact(
                    "the failed stream-open identity is absent or stale".to_owned(),
                )
            })?;
        let mut retargeted = 0usize;
        for artifact in pages.artifacts_mut() {
            let target_page_index =
                artifact
                    .open_out_occurrences()
                    .iter()
                    .find_map(|(page_index, position)| {
                        (*position == failed.position()).then_some(*page_index)
                    });
            let Some(page_index) = target_page_index else {
                continue;
            };
            let mut page = tex_out::PageArtifact::from_bytes(artifact.bytes())
                .map_err(|error| FinalizationError::PreparedArtifact(error.to_string()))?;
            if !page.retarget_open_out_at(
                page_index,
                failed.slot().raw(),
                &failed_path,
                &replacement_text,
            ) {
                return Err(FinalizationError::PreparedArtifact(
                    "the exact prepared page effect does not validate against the failed open"
                        .to_owned(),
                ));
            }
            let bytes = page
                .to_bytes()
                .map_err(|error| FinalizationError::PreparedArtifact(error.to_string()))?;
            *artifact = artifact.clone().with_prepared_bytes(bytes);
            retargeted += 1;
        }
        if retargeted == 0 {
            return Err(FinalizationError::PreparedArtifact(
                "no prepared artifact contains the failed stream-open occurrence".to_owned(),
            ));
        }
        stores
            .world_mut()
            .retarget_pending_stream_open(failed, replacement)?;
        self.prepared_pages = Some(pages);
        Ok(())
    }

    pub fn commit_effects(
        self,
        stores: &mut Universe,
    ) -> Result<CommittedFinalization, FinalizationError> {
        match self.commit_effects_retryable(stores)? {
            FinalizationCommit::Committed(committed) => Ok(committed),
            FinalizationCommit::Retry { error, .. } => Err(error.into()),
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
        mut self,
        stores: &mut Universe,
    ) -> Result<FinalizationCommit, FinalizationError> {
        let result = if stores.world().commit_mode() == WorldCommitMode::Retained {
            // §530's retry report is itself selector-routed output appended
            // while this plan is suspended.
            self.effect_pos = stores.world().effect_pos();
            stores.export_retained_effects()
        } else {
            stores.commit_effects(self.effect_pos)
        };
        if let Err(error) = result {
            if error.stream_open_unavailable().is_some()
                && error.retry_safety() == tex_state::EffectRetrySafety::Safe
            {
                return Ok(FinalizationCommit::Retry { plan: self, error });
            }
            return Err(error.into());
        }
        if let Some(pages) = self.prepared_pages.take() {
            stores.publish_page_suffix(pages)?;
        }
        Ok(FinalizationCommit::Committed(CommittedFinalization {
            files: self.files,
        }))
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
    pub fn materialize(self, stores: &mut Universe) -> Result<(), FinalizationError> {
        stores.world_mut().publish_files(
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

/// Installs the primitive/state setup used by an INITEX run.
///
/// TeX82 initializes only the category codes named in tex.web §232. In
/// particular, `{`, `}`, `$`, `&`, `#`, `^`, and `_` remain `other_char`
/// until the format source assigns them.
pub fn prepare_initex_stores(stores: &mut Universe) {
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
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
pub fn prepare_run_stores(stores: &mut Universe) {
    prepare_initex_stores(stores);
    stores.install_plain_catcodes();
}

/// Installs the primitive/state setup used by `umber run --etex`.
pub fn prepare_etex_run_stores(stores: &mut Universe) {
    prepare_run_stores(stores);
    tex_command::install_etex_expandable_primitives(stores);
    tex_exec::install_etex_unexpandable_primitives(stores);
}

/// Installs the primitive/state setup used by `umber run --pdftex`.
pub fn prepare_pdftex_run_stores(stores: &mut Universe) {
    prepare_etex_run_stores(stores);
    pdftex::install_pdftex_layer(stores);
    pdftex::initialize_pdftex_parameter_defaults(stores);
    stores.enable_pdf_output();
}

/// Restores driver-selected pdfTeX meanings after loading a format image.
pub fn install_pdftex_format_primitives(stores: &mut Universe) {
    tex_command::register_tex82_expandable_primitives(stores);
    tex_command::register_etex_expandable_primitives(stores);
    tex_exec::register_unexpandable_primitives(stores);
    tex_exec::register_etex_unexpandable_primitives(stores);
    pdftex::register_pdftex_layer(stores);
    stores.enable_pdf_output();
}

fn register_tex_format_primitives(stores: &mut Universe) {
    tex_command::register_tex82_expandable_primitives(stores);
    tex_exec::register_unexpandable_primitives(stores);
}

fn register_etex_format_primitives(stores: &mut Universe) {
    register_tex_format_primitives(stores);
    tex_command::register_etex_expandable_primitives(stores);
    tex_exec::register_etex_unexpandable_primitives(stores);
}

fn install_latex_compatibility_layer(stores: &mut Universe) {
    tex_expand::install_latex_expandable_primitives(stores);
    for ch in ['{', '}', '$', '&', '#', '^', '_'] {
        stores.set_catcode(ch, tex_state::token::Catcode::Other);
    }
}

/// Reconstructs the driver-selected LaTeX primitive registry after loading a format image.
pub fn install_latex_format_primitives(stores: &mut Universe) {
    register_etex_format_primitives(stores);
    tex_expand::register_latex_expandable_primitives(stores);
}

/// Installs the primitive/state setup used by supported LaTeX-DVI runs.
///
/// This is an Umber extension layer over e-TeX. It intentionally does not
/// install pdfTeX identity or PDF-backend primitives.
pub fn prepare_latex_run_stores(stores: &mut Universe) {
    prepare_etex_run_stores(stores);
    install_latex_compatibility_layer(stores);
}

/// Installs the composed pdfTeX and LaTeX setup used by pdfLaTeX runs.
pub fn prepare_pdflatex_run_stores(stores: &mut Universe) {
    prepare_pdftex_run_stores(stores);
    install_latex_compatibility_layer(stores);
}

/// Reconstructs the composed pdfTeX and LaTeX primitive registry after format load.
pub fn install_pdflatex_format_primitives(stores: &mut Universe) {
    install_pdftex_format_primitives(stores);
    tex_expand::register_latex_expandable_primitives(stores);
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
        assert_eq!(latex.catcode('{'), Catcode::Other);
        assert_eq!(latex.catcode('#'), Catcode::Other);
        assert_eq!(latex.catcode('A'), Catcode::Letter);
        assert_eq!(latex.catcode('\\'), Catcode::Escape);
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
pub fn run_input_with_context(
    stores: &mut Universe,
    request: RetainedRootRequest,
    host: &mut dyn CanonicalResourceHost,
) -> Result<String, CanonicalSessionError> {
    run_input_collecting_artifacts(stores, request, host).map(|result| result.terminal_text)
}

/// Runs one retained root with the explicitly selected command profile.
pub fn run_input_with_context_and_profile(
    stores: &mut Universe,
    request: RetainedRootRequest,
    host: &mut dyn CanonicalResourceHost,
    profile: CommandProfile,
) -> Result<String, CanonicalSessionError> {
    run_input_collecting_artifacts_with_profile(stores, request, host, profile)
        .map(|result| result.terminal_text)
}

/// Runs input and returns the artifact ids emitted by `\shipout` in order.
pub fn run_input_collecting_artifacts(
    stores: &mut Universe,
    request: RetainedRootRequest,
    host: &mut dyn CanonicalResourceHost,
) -> Result<RunResult, CanonicalSessionError> {
    run_retained_root(stores, request, host)
}

/// Runs input under an explicitly selected command profile and returns its artifacts.
///
/// Primitive/state preparation and command-profile selection are separate host
/// responsibilities. In particular, a pdfTeX store must be paired with
/// [`CommandProfile::PDFTEX14027`] so shipout finalizes PDF-only deferred nodes as
/// PDF rather than applying the exact DVI-mode rejection.
pub fn run_input_collecting_artifacts_with_profile(
    stores: &mut Universe,
    mut request: RetainedRootRequest,
    host: &mut dyn CanonicalResourceHost,
    profile: CommandProfile,
) -> Result<RunResult, CanonicalSessionError> {
    request.profile = profile;
    run_retained_root(stores, request, host)
}

/// Reads committed page artifacts from `World` and writes a complete DVI file.
pub fn dvi_from_artifacts(
    stores: &Universe,
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
pub fn write_dvi_from_artifacts<W: std::io::Write>(
    stores: &Universe,
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
pub fn html_from_artifacts<R: tex_out::html::HtmlFontAssets>(
    stores: &Universe,
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
pub fn run_memory_with_stores(
    source: &str,
    stores: &mut Universe,
) -> Result<String, CanonicalSessionError> {
    run_memory_with_stores_and_profile(source, stores, CommandProfile::TEX82, true)
}

/// Runs in-memory input with an explicit command profile and output backend.
pub fn run_memory_with_stores_and_profile(
    source: &str,
    stores: &mut Universe,
    profile: CommandProfile,
    emit_dvi: bool,
) -> Result<String, CanonicalSessionError> {
    let _ = emit_dvi;
    let mut host = FileSessionResolvers::new(Path::new("texput.tex"), Vec::new(), Vec::new());
    let mut session = CanonicalEngineSession::new(stores, profile);
    session.project_terminal_text_to_root_body();
    session.register_retained_root_with_invocation(
        "texput",
        "texput",
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source.as_bytes()),
        ),
    )?;
    session
        .run(&mut host, &mut NoCanonicalCheckpoints)
        .map(|result| result.terminal_text)
}

fn uncommitted_terminal_text(stores: &Universe) -> String {
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
        DriverFile, EngineSession, FinalizationCommit, FinalizationError, PlannedFinalization,
        dvi_from_committed_artifacts, dvi_from_page_plans, prepare_pdftex_run_stores,
        run_input_collecting_artifacts_with_profile, terminal_text_from_effects,
        uncommitted_terminal_text,
    };
    use crate::FileSessionResolvers;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tex_command::{CommandProfile, PdfImageResource, RegisteredSourceKind};
    use tex_exec::{
        CanonicalResourceNeed, CanonicalStepResult, ExecutionContext, MainControlStep,
        install_unexpandable_primitives,
    };
    use tex_expand::install_expandable_primitives;
    use tex_lex::{InputStack, MemoryInput};
    use tex_state::{PrintSink, StreamSlot, Universe, World};

    const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

    #[test]
    fn public_file_root_retains_world_identity_across_typed_input_retry() {
        let mut stores = Universe::new_with_plain_catcodes();
        tex_expand::install_expandable_primitives(&mut stores);
        tex_exec::install_unexpandable_primitives(&mut stores);
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
                super::RetainedRootRequest::authored(
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
            super::RetainedRootRequest::authored(
                "profile-boundary",
                source.as_bytes(),
                CommandProfile::PDFTEX14027,
            ),
            &mut host,
            CommandProfile::PDFTEX14027,
        )
        .expect("pdfTeX profile accepts deferred PDF nodes in PDF mode");
        assert_eq!(result.committed_artifacts.len(), 1);
    }

    #[test]
    fn canonical_bridge_registers_only_acquired_root_and_nested_sources() {
        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        stores
            .world_mut()
            .set_memory_file(
                "selected/report.tex",
                b"\\message{\\jobname}\\input child \\end",
            )
            .expect("World root is seeded");
        let root = stores
            .world_mut()
            .read_file("selected/report.tex")
            .expect("World selects root");
        let root_hash = root.hash();
        let mut legacy_input = InputStack::new(MemoryInput::new("legacy input is not consumed"));
        let mut session = EngineSession::new(
            &mut legacy_input,
            &mut stores,
            ExecutionContext::new("ignored-by-canonical-bridge"),
        );
        session
            .register_canonical_world_root("inputs/report.tex", root)
            .expect("root registers");
        session.provide_canonical_input(
            "child.tex",
            RegisteredSourceKind::Generated,
            Arc::from(&b"\\message{nested}"[..]),
        );

        assert_eq!(
            session.step_canonical().expect("root message"),
            MainControlStep::Continue
        );
        assert_eq!(
            session.step_canonical().expect("nested message"),
            MainControlStep::Continue
        );
        // TeX82 §343 retires the nested source and resumes the parent inside
        // command delivery. Each source also contributes its normalized line
        // ending, so the number of following main-control operations is not a
        // bridge contract. `execute` owns the canonical loop through its
        // command-delivered `MainControlStep::End` instead.
        let result = session.execute().expect("canonical root terminates");
        // TeX82 §1280's separating space between two `\message` texts, plus
        // §537/§362's parens bracketing the nested `\input child`, named as
        // opened (`child.tex`) the way §537's `a_make_name_string` does.
        assert_eq!(result.terminal_text, "report (child.tex nested)");
        assert!(matches!(
            session.stores().world().input_records(),
            [record] if record.hash() == root_hash
                && record.path() == std::path::Path::new("selected/report.tex")
        ));
        assert_eq!(
            uncommitted_terminal_text(session.stores()),
            "report (child.tex nested)"
        );
    }

    #[test]
    fn registered_root_executes_through_the_canonical_session_path() {
        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        let mut legacy_input = InputStack::new(MemoryInput::new("\\message{legacy}"));
        let mut session = EngineSession::new(
            &mut legacy_input,
            &mut stores,
            ExecutionContext::new("canonical"),
        );
        session
            .register_canonical_root(
                "canonical.tex",
                RegisteredSourceKind::Generated,
                Arc::from(&b"\\count0=17\\message{canonical}\\end"[..]),
            )
            .expect("root registers");

        let result = session.execute().expect("canonical run completes");

        assert_eq!(result.terminal_text, "canonical");
        assert_eq!(session.stores().count(0), 17);
    }

    #[test]
    fn retained_memory_roots_project_local_text_and_keep_effect_cursors_rollbackable() {
        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);

        let run = |source: &'static [u8], stores: &mut Universe| {
            let mut host =
                FileSessionResolvers::new(Path::new("texput.tex"), Vec::new(), Vec::new());
            let mut session = crate::CanonicalEngineSession::new(stores, CommandProfile::TEX82);
            session.project_terminal_text_to_root_body();
            session
                .register_retained_root_with_invocation(
                    "texput",
                    "texput",
                    tex_command::SourceRegistration::new(RegisteredSourceKind::Generated, source),
                )
                .expect("root registers");
            session
                .run(&mut host, &mut super::NoCanonicalCheckpoints)
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
    fn canonical_explicit_shipout_publishes_aligned_prepared_dvi_receipt() {
        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::new(
            &mut input,
            &mut stores,
            ExecutionContext::new("canonical-shipout"),
        );
        session
            .register_canonical_root(
                "shipout.tex",
                RegisteredSourceKind::Generated,
                Arc::from(&b"\\shipout\\vbox{}\\end"[..]),
            )
            .expect("root registers");

        let result = session.execute().expect("canonical shipout completes");

        assert_eq!(result.artifacts.len(), 1);
        assert_eq!(result.dvi_pages.len(), result.artifacts.len());
        assert_eq!(
            dvi_from_page_plans(&result.dvi_pages).expect("prepared plans assemble"),
            dvi_from_committed_artifacts(&result.committed_artifacts)
                .expect("committed artifact reference assembles"),
        );
    }

    #[test]
    fn canonical_effect_free_shipout_memo_republishes_one_aligned_receipt() {
        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        stores.enable_pure_memo(tex_state::PureMemoConfig::default());
        stores.enable_shipout_memo();
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::new(
            &mut input,
            &mut stores,
            ExecutionContext::new("canonical-memo-shipout"),
        );
        session
            .register_canonical_root(
                "memo.tex",
                RegisteredSourceKind::Generated,
                Arc::from(
                    &b"\\setbox0=\\hbox{\\vrule width1pt height1pt}\\shipout\\copy0\\shipout\\copy0\\end"[..],
                ),
            )
            .expect("root registers");

        let result = session.execute().expect("canonical memo run completes");

        assert_eq!(result.artifacts.len(), 2);
        assert_eq!(result.dvi_pages.len(), result.artifacts.len());
        assert!(session.stores().pure_memo_stats().shipout_hits >= 1);
        assert!(result.committed_artifacts.iter().all(|artifact| {
            tex_out::PageArtifact::from_bytes(artifact.bytes())
                .expect("memoized artifact parses")
                .effects
                .is_empty()
        }));
        assert!(session.stores().world().effect_records().is_empty());
    }

    #[test]
    fn canonical_default_page_shipout_publishes_an_aligned_receipt() {
        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::new(
            &mut input,
            &mut stores,
            ExecutionContext::new("canonical-default-shipout"),
        );
        session
            .register_canonical_root(
                "default.tex",
                RegisteredSourceKind::Generated,
                Arc::from(
                    &b"\\topskip=0pt\\setbox0=\\hbox{\\vrule width1pt height1pt}\\copy0\\penalty-10000\\end"[..],
                ),
            )
            .expect("root registers");

        let result = session
            .execute()
            .expect("canonical default shipout completes");

        assert_eq!(result.artifacts.len(), 1);
        assert_eq!(result.dvi_pages.len(), result.artifacts.len());
    }

    #[test]
    fn canonical_special_is_deferred_until_shipout() {
        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::new(
            &mut input,
            &mut stores,
            ExecutionContext::new("canonical-whatsits"),
        );
        session
            .register_canonical_root(
                "whatsits.tex",
                RegisteredSourceKind::Generated,
                Arc::from(&b"\\shipout\\hbox{\\special{one}}\\end"[..]),
            )
            .expect("root registers");

        let result = session.execute().expect("canonical whatsits complete");

        // §638's `[0]` progress marker did print, but `result.terminal_text`
        // (`uncommitted_terminal_text`) reads only the live, not-yet-
        // materialized effect suffix, and `shipout_replay_box` commits the
        // marker immediately after printing it (see its doc comment); it is
        // visible in `stores.world().memory_terminal_output()` instead.
        assert_eq!(result.terminal_text, "");
        let page = tex_out::PageArtifact::from_bytes(result.committed_artifacts[0].bytes())
            .expect("committed artifact parses");
        assert!(matches!(
            page.effects.as_slice(),
            [tex_out::PageEffect::Special { payload, .. }] if payload == b"one"
        ));
    }

    #[test]
    fn canonical_stream_effects_and_page_effects_commit_in_tex_order() {
        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::new(
            &mut input,
            &mut stores,
            ExecutionContext::new("canonical-effects"),
        );
        session
            .register_canonical_root(
                "effects.tex",
                RegisteredSourceKind::Generated,
                Arc::from(
                    &b"\\immediate\\openout2=ordered.aux \
                       \\immediate\\write2{before} \
                       \\shipout\\hbox{\\write2{during}\\special{after-write}} \
                       \\immediate\\closeout2 \\end"[..],
                ),
            )
            .expect("root registers");

        let result = session.execute().expect("canonical effects complete");

        assert_eq!(
            session.stores().world().memory_output("ordered.aux"),
            Some(&b"before\nduring\n"[..])
        );
        // §638's `[0]` progress marker prints right after the `\shipout`
        // and is immediately committed (`shipout_replay_box`'s doc comment),
        // so only the later, still-uncommitted `\closeout2` remains live.
        assert!(matches!(
            session.stores().world().effect_records(),
            [tex_state::EffectRecord::StreamClose { slot }]
                if *slot == StreamSlot::new(2)
        ));
        let page = tex_out::PageArtifact::from_bytes(result.committed_artifacts[0].bytes())
            .expect("committed artifact parses");
        assert!(matches!(
            page.effects.as_slice(),
            [
                tex_out::PageEffect::OpenOut { stream: 2, path },
                tex_out::PageEffect::Write { text: before, .. },
                tex_out::PageEffect::Write { text: during, .. },
                tex_out::PageEffect::Special { payload, .. },
            ] if path == "ordered.aux"
                && before == "before\n"
                && during == "during\n"
                && payload == b"after-write"
        ));
    }

    #[test]
    fn canonical_pdf_whatsits_keep_explicit_shipout_order() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_pdftex_run_stores(&mut stores);
        stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::with_command_profile(
            &mut input,
            &mut stores,
            ExecutionContext::new("canonical-pdf-effects").with_dvi_output(false),
            tex_command::CommandProfile::PDFTEX14027,
        );
        session
            .register_canonical_root(
                "pdf-effects.tex",
                RegisteredSourceKind::Generated,
                Arc::from(
                    &b"\\shipout\\hbox{\
                       \\pdfliteral direct{A}\
                       \\pdfsave\\pdfrestore\
                       \\pdfdest name{target}fit\
                       \\special{B}\
                       \\pdfannot width1pt{/Subtype/Text}\
                       \\pdfstartlink width1pt goto name{target}\\pdfendlink}\\end"[..],
                ),
            )
            .expect("root registers");

        let result = session.execute().expect("canonical PDF effects complete");
        let page = tex_out::PageArtifact::from_bytes(result.committed_artifacts[0].bytes())
            .expect("committed artifact parses");

        assert!(matches!(
            page.effects.as_slice(),
            [
                tex_out::PageEffect::PdfLiteral { payload: literal, .. },
                tex_out::PageEffect::PdfSave,
                tex_out::PageEffect::PdfRestore,
                tex_out::PageEffect::PdfDestination(destination),
                tex_out::PageEffect::Special { payload: special, .. },
                tex_out::PageEffect::PdfAnnotation(
                    tex_out::PdfAnnotationEffect::Annotation { .. },
                ),
                tex_out::PageEffect::PdfAnnotation(
                    tex_out::PdfAnnotationEffect::LinkStart { .. },
                ),
                tex_out::PageEffect::PdfAnnotation(
                    tex_out::PdfAnnotationEffect::LinkEnd { .. },
                ),
            ] if literal == b"A"
                && destination.identifier
                    == tex_out::PdfDestinationIdentifier::Name(b"target".to_vec())
                && special == b"B"
        ));
    }

    #[test]
    fn canonical_pdf_whatsits_survive_default_and_output_routine_shipout() {
        let run = |source: &'static [u8]| {
            let mut stores = Universe::new_with_plain_catcodes();
            crate::prepare_pdftex_run_stores(&mut stores);
            stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
            let mut input = InputStack::new(MemoryInput::new("legacy input"));
            let mut session = EngineSession::with_command_profile(
                &mut input,
                &mut stores,
                ExecutionContext::new("canonical-pdf-page-builder").with_dvi_output(false),
                tex_command::CommandProfile::PDFTEX14027,
            );
            session
                .register_canonical_root(
                    "page-builder.tex",
                    RegisteredSourceKind::Generated,
                    Arc::from(source),
                )
                .expect("root registers");
            let result = session.execute().expect("page builder ships PDF whatsit");
            result
                .committed_artifacts
                .iter()
                .map(|artifact| {
                    tex_out::PageArtifact::from_bytes(artifact.bytes())
                        .expect("committed artifact parses")
                })
                .collect::<Vec<_>>()
        };

        let default_pages = run(
            b"\\topskip=0pt\\setbox0=\\hbox{\\vrule width1pt height1pt\\pdfliteral direct{default}}\
              \\copy0\\penalty-10000\\end",
        );
        assert_eq!(default_pages.len(), 1);
        assert!(matches!(
            default_pages[0].effects.as_slice(),
            [tex_out::PageEffect::PdfLiteral { payload, .. }] if payload == b"default"
        ));

        let output_pages = run(b"\\output={\\shipout\\box255}\\topskip=0pt\
              \\setbox0=\\hbox{\\vrule width1pt height1pt\\pdfliteral direct{routine}}\
              \\copy0\\penalty-10000\\end");
        let output_effects = output_pages
            .iter()
            .flat_map(|page| page.effects.iter())
            .collect::<Vec<_>>();
        assert_eq!(output_effects.len(), 1, "effects: {output_effects:#?}");
        assert!(
            matches!(
                output_effects[0],
                tex_out::PageEffect::PdfLiteral { payload, .. } if payload == b"routine"
            ),
            "effects: {output_effects:#?}"
        );
    }

    #[test]
    fn canonical_pdf_resource_retry_matches_no_failure_effect_sequence() {
        let source = &b"\\immediate\\write17{once} \\pdfximage {image.png} \
                         \\shipout\\hbox{\\pdfrefximage1}\\end"[..];
        let image = tex_state::PdfExternalImageSource {
            identity: tex_state::ContentHash::from_bytes(b"canonical retry image"),
            metadata: tex_state::PdfExternalImageMetadata::Raster(
                tex_state::PdfRasterImageMetadata {
                    format: tex_state::PdfRasterFormat::Png,
                    width: 1,
                    height: 1,
                    bits_per_component: 8,
                    color_space: tex_state::PdfRasterColorSpace::Gray,
                    alpha: false,
                    png_color_type: Some(0),
                },
            ),
            natural_width: tex_state::scaled::Scaled::from_raw(tex_state::scaled::Scaled::UNITY),
            natural_height: tex_state::scaled::Scaled::from_raw(tex_state::scaled::Scaled::UNITY),
            bytes: Arc::from(&b"retry image bytes"[..]),
        };

        let mut retry_stores = Universe::new_with_plain_catcodes();
        crate::prepare_pdftex_run_stores(&mut retry_stores);
        retry_stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
        let mut retry_input = InputStack::new(MemoryInput::new("legacy input"));
        let mut retry = EngineSession::with_command_profile(
            &mut retry_input,
            &mut retry_stores,
            ExecutionContext::new("canonical-pdf-retry").with_dvi_output(false),
            tex_command::CommandProfile::PDFTEX14027,
        );
        retry
            .register_canonical_root(
                "retry.tex",
                RegisteredSourceKind::Generated,
                Arc::from(source),
            )
            .expect("retry root registers");
        assert!(matches!(
            retry.advance_canonical().expect("immediate write"),
            CanonicalStepResult::Progress(MainControlStep::Continue)
        ));
        let mut request = None;
        for _ in 0..64 {
            match retry.advance_canonical().expect("image suspension") {
                CanonicalStepResult::Progress(MainControlStep::Continue) => {}
                CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage {
                    request: requested,
                }) => {
                    request = Some(requested);
                    break;
                }
                other => panic!("expected image suspension, got {other:?}"),
            }
        }
        let request = request.expect("image request appears within the bounded retry window");
        assert_eq!(retry.stores().world().effect_records().len(), 1);
        assert!(retry.stores().world().artifact_commits().is_empty());
        retry.provide_canonical_pdf_image(
            request.clone(),
            PdfImageResource::Available(image.clone()),
        );
        let retried = retry.execute().expect("retried execution succeeds");

        let mut fresh_stores = Universe::new_with_plain_catcodes();
        crate::prepare_pdftex_run_stores(&mut fresh_stores);
        fresh_stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
        let mut fresh_input = InputStack::new(MemoryInput::new("legacy input"));
        let mut fresh = EngineSession::with_command_profile(
            &mut fresh_input,
            &mut fresh_stores,
            ExecutionContext::new("canonical-pdf-fresh").with_dvi_output(false),
            tex_command::CommandProfile::PDFTEX14027,
        );
        fresh
            .register_canonical_root(
                "fresh.tex",
                RegisteredSourceKind::Generated,
                Arc::from(source),
            )
            .expect("fresh root registers");
        fresh.provide_canonical_pdf_image(request, PdfImageResource::Available(image));
        let no_failure = fresh.execute().expect("no-failure execution succeeds");

        assert_eq!(
            retried.committed_artifacts[0].bytes(),
            no_failure.committed_artifacts[0].bytes()
        );
        let page = tex_out::PageArtifact::from_bytes(retried.committed_artifacts[0].bytes())
            .expect("retried artifact parses");
        assert!(matches!(
            page.effects.as_slice(),
            [
                tex_out::PageEffect::Write {
                    sink: tex_out::EffectSink::TerminalAndLog,
                    text,
                },
                tex_out::PageEffect::PdfRefXImage { object: 1, .. },
            ] if text == "once\n"
        ));
        // §638's `[0]` progress marker for the one shipped page, committed
        // immediately after it prints (`shipout_replay_box`'s doc comment).
        assert_eq!(
            retry.stores().world().memory_terminal_output(),
            Some(&b"once\n[0]"[..])
        );
        assert_eq!(
            fresh.stores().world().memory_terminal_output(),
            Some(&b"once\n[0]"[..])
        );
    }

    #[test]
    fn fresh_and_format_sessions_pin_the_same_command_profile() {
        use crate::EngineMode;

        for mode in [EngineMode::Tex82, EngineMode::ETex, EngineMode::PdfTex] {
            let mut fresh = Universe::new_with_plain_catcodes();
            mode.prepare_fresh(&mut fresh);
            let mut fresh_input = InputStack::new(MemoryInput::new(""));
            let fresh_session = EngineSession::with_command_profile(
                &mut fresh_input,
                &mut fresh,
                ExecutionContext::new("fresh"),
                mode.command_profile(),
            );
            let fresh_profile = fresh_session.canonical_command_profile();
            let format = fresh_session.stores().dump_format().expect("format dumps");
            drop(fresh_session);
            let mut loaded =
                Universe::from_format(World::default(), &format).expect("format loads");
            mode.install_after_format(&mut loaded);
            let mut loaded_input = InputStack::new(MemoryInput::new(""));
            let loaded_session = EngineSession::with_command_profile(
                &mut loaded_input,
                &mut loaded,
                ExecutionContext::new("loaded"),
                mode.command_profile(),
            );
            assert_eq!(
                fresh_profile,
                loaded_session.canonical_command_profile(),
                "{}",
                mode.name()
            );
        }
    }

    #[test]
    fn canonical_fresh_and_format_loaded_sessions_execute_the_same_root() {
        use crate::EngineMode;

        for mode in [EngineMode::Tex82, EngineMode::ETex, EngineMode::PdfTex] {
            let root = b"\\message{canonical format}\\end";

            let mut fresh = Universe::new_with_plain_catcodes();
            mode.prepare_fresh(&mut fresh);
            fresh
                .world_mut()
                .set_memory_file("selected/job.tex", root)
                .expect("fresh World root is seeded");
            let fresh_root = fresh
                .world_mut()
                .read_file("selected/job.tex")
                .expect("fresh World selects root");
            let mut fresh_input = InputStack::new(MemoryInput::new("legacy fresh input"));
            let mut fresh_session = EngineSession::with_command_profile(
                &mut fresh_input,
                &mut fresh,
                ExecutionContext::new("fresh"),
                mode.command_profile(),
            );
            fresh_session
                .register_canonical_world_root("job.tex", fresh_root)
                .expect("fresh canonical root registers");
            let fresh_run = fresh_session.execute().expect("fresh canonical run");
            let format = fresh_session.stores().dump_format().expect("format dumps");
            drop(fresh_session);

            let mut loaded =
                Universe::from_format(World::default(), &format).expect("format loads");
            mode.install_after_format(&mut loaded);
            loaded
                .world_mut()
                .set_memory_file("selected/job.tex", root)
                .expect("format-loaded World root is seeded");
            let loaded_root = loaded
                .world_mut()
                .read_file("selected/job.tex")
                .expect("format-loaded World selects root");
            let mut loaded_input = InputStack::new(MemoryInput::new("legacy loaded input"));
            let mut loaded_session = EngineSession::with_command_profile(
                &mut loaded_input,
                &mut loaded,
                ExecutionContext::new("loaded"),
                mode.command_profile(),
            );
            loaded_session
                .register_canonical_world_root("job.tex", loaded_root)
                .expect("format-loaded canonical root registers");
            let loaded_run = loaded_session
                .execute()
                .expect("format-loaded canonical run");

            assert_eq!(fresh_run, loaded_run, "{}", mode.name());
        }
    }

    #[test]
    fn etex_format_loaded_session_executes_unexpanded_with_finite_fuel() {
        use crate::EngineMode;

        let mode = EngineMode::ETex;
        let mut initex = Universe::new_with_plain_catcodes();
        mode.prepare_initex(&mut initex);
        let format = initex.dump_format().expect("e-TeX format dumps");
        let mut loaded =
            Universe::from_format(World::default(), &format).expect("e-TeX format loads");
        mode.install_after_format(&mut loaded);
        loaded
            .world_mut()
            .set_memory_file(
                "selected/job.tex",
                b"\\edef\\holder{\\unexpanded{\\iftrue}}\\end",
            )
            .expect("format-loaded root is seeded");
        let root = loaded
            .world_mut()
            .read_file("selected/job.tex")
            .expect("format-loaded root is selected");
        let mut legacy_input = InputStack::new(MemoryInput::new("legacy loaded input"));
        let mut session = EngineSession::with_command_profile(
            &mut legacy_input,
            &mut loaded,
            ExecutionContext::new("loaded"),
            mode.command_profile(),
        );
        session
            .canonical_main_control_mut()
            .set_fuel_limit(1_000)
            .expect("finite command fuel is valid");
        session
            .register_canonical_world_root("job.tex", root)
            .expect("format-loaded canonical root registers");
        session
            .execute()
            .expect("format-loaded unexpanded execution completes");
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
            let mut session =
                crate::CanonicalEngineSession::new(&mut stores, mode.command_profile());
            session
                .register_world_root("job", selected_root)
                .expect("selected root registers unchanged");
            let mut host =
                crate::FileSessionResolvers::new(Path::new("/project/job.tex"), vec![], vec![]);
            let run = session
                .run(&mut host, &mut Vec::new())
                .expect("typed retries complete");

            assert!(!run.dumped_format);
            assert!(session.stores().pdf_last_external_image().is_some());
            let records = session.stores().world().input_records();
            assert_eq!(records.len(), 3);
            assert_eq!(records[0].hash(), root_hash);
            assert_eq!(records[1].path(), Path::new("/project/cmr10.tfm"));
            assert_eq!(records[2].path(), Path::new("/project/image.png"));
        }
    }

    #[test]
    fn canonical_session_checkpoint_uses_command_summary_not_legacy_input() {
        use tex_exec::{EngineBoundary, ExecutionBudgetCounters};

        let mut stores = Universe::new_with_plain_catcodes();
        install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::new(&mut input, &mut stores, ExecutionContext::new("job"));
        session
            .register_canonical_root(
                "job.tex",
                RegisteredSourceKind::Generated,
                Arc::from(&b"x"[..]),
            )
            .expect("root registers");
        let checkpoint = session
            .capture_canonical_checkpoint(
                EngineBoundary::JobStart,
                ExecutionBudgetCounters::default(),
            )
            .expect("quiescent command checkpoint");
        assert!(checkpoint.command_summary().is_some());
        assert!(checkpoint.input_summary().is_empty());
        session
            .restore_canonical_checkpoint(&checkpoint)
            .expect("canonical checkpoint restores");
    }

    #[test]
    fn canonical_pdfximage_suspends_then_retries_with_the_exact_request() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_pdftex_run_stores(&mut stores);
        stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
        stores.set_int_param_global(
            tex_state::env::banks::IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX,
            4,
        );
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::with_command_profile(
            &mut input,
            &mut stores,
            ExecutionContext::new("pdf-image"),
            tex_command::CommandProfile::PDFTEX14027,
        );
        session
            .register_canonical_root(
                "job.tex",
                RegisteredSourceKind::Generated,
                Arc::from(&b"\\pdfximage width 10pt height 20pt depth 3pt named {chapter} colorspace -7 mediabox {image.pdf}\\pdfrefximage1\\end"[..]),
            )
            .expect("root registers");

        let request = match session.advance_canonical().expect("image scan") {
            CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { request }) => request,
            other => panic!("expected image suspension, got {other:?}"),
        };
        assert_eq!(request.name, "image.pdf");
        assert_eq!(
            request.page,
            tex_command::PdfImagePageSelection::Named(b"chapter".to_vec())
        );
        assert_eq!(request.color_space_object, -7);
        assert_eq!(
            request.width.expect("width scanned").raw(),
            10 * tex_state::scaled::Scaled::UNITY
        );
        assert_eq!(
            request.height.expect("height scanned").raw(),
            20 * tex_state::scaled::Scaled::UNITY
        );
        assert_eq!(
            request.depth.expect("depth scanned").raw(),
            3 * tex_state::scaled::Scaled::UNITY
        );
        assert_eq!(request.page_box, tex_command::PdfImagePageBox::Trim);
        assert!(session.stores().pdf_external_images().is_empty());
        assert_eq!(
            session
                .stores()
                .int_param(tex_state::env::banks::IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
            4,
            "resource suspension rolls back the obsolete compatibility transition"
        );
        assert_eq!(
            session
                .stores()
                .int_param(tex_state::env::banks::IntParam::PDF_FORCE_PAGE_BOX),
            0
        );

        let source = tex_state::PdfExternalImageSource {
            identity: tex_state::ContentHash::from_bytes(b"canonical image"),
            metadata: tex_state::PdfExternalImageMetadata::Raster(
                tex_state::PdfRasterImageMetadata {
                    format: tex_state::PdfRasterFormat::Png,
                    width: 1,
                    height: 1,
                    bits_per_component: 8,
                    color_space: tex_state::PdfRasterColorSpace::Gray,
                    alpha: false,
                    png_color_type: Some(0),
                },
            ),
            natural_width: tex_state::scaled::Scaled::from_raw(tex_state::scaled::Scaled::UNITY),
            natural_height: tex_state::scaled::Scaled::from_raw(tex_state::scaled::Scaled::UNITY),
            bytes: Arc::from(&b"image bytes"[..]),
        };
        session.provide_canonical_pdf_image(request.clone(), PdfImageResource::Available(source));
        assert!(matches!(
            session
                .advance_canonical()
                .expect("fulfilled image retries"),
            CanonicalStepResult::Progress(MainControlStep::Continue)
        ));
        assert_eq!(
            session
                .stores()
                .int_param(tex_state::env::banks::IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
            0
        );
        assert_eq!(
            session
                .stores()
                .int_param(tex_state::env::banks::IntParam::PDF_FORCE_PAGE_BOX),
            4
        );
        let obsolete_warnings = session
            .stores()
            .world()
            .effect_records()
            .iter()
            .filter(|effect| {
                matches!(effect, tex_state::EffectRecord::StreamWrite { text, .. }
                    if text.contains("\\pdfoptionalwaysusepdfpagebox is obsolete"))
            })
            .count();
        assert_eq!(obsolete_warnings, 1, "successful retry warns exactly once");
        let image = session
            .stores()
            .pdf_last_external_image()
            .expect("image allocated");
        assert_eq!(
            image.dimensions().width.raw(),
            10 * tex_state::scaled::Scaled::UNITY
        );
        assert_eq!(
            image.dimensions().height.raw(),
            20 * tex_state::scaled::Scaled::UNITY
        );
        assert_eq!(
            image.dimensions().depth.raw(),
            3 * tex_state::scaled::Scaled::UNITY
        );
        assert_eq!(image.color_space_object(), -7);
        assert!(matches!(
            session.advance_canonical().expect("reference image"),
            CanonicalStepResult::Progress(MainControlStep::Continue)
        ));
    }

    #[test]
    fn canonical_pdfximage_authoritative_absence_is_a_pdftex_diagnostic() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_pdftex_run_stores(&mut stores);
        stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::with_command_profile(
            &mut input,
            &mut stores,
            ExecutionContext::new("pdf-image"),
            tex_command::CommandProfile::PDFTEX14027,
        );
        session
            .register_canonical_root(
                "job.tex",
                RegisteredSourceKind::Generated,
                Arc::from(&b"\\pdfximage {absent.png}"[..]),
            )
            .expect("root registers");
        let request = match session.advance_canonical().expect("image scan") {
            CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { request }) => request,
            other => panic!("expected image suspension, got {other:?}"),
        };
        session.provide_canonical_pdf_image(request, PdfImageResource::Unavailable);
        let error = session.advance_canonical().expect_err("absence is final");
        assert!(matches!(
            error,
            tex_exec::ExecError::PdfImageOpen { ref name, ref message }
                if name == "absent.png" && message == "image is unavailable"
        ));
        assert!(session.stores().pdf_external_images().is_empty());
    }

    #[test]
    fn canonical_pdfximage_request_uses_live_pagebox_configuration() {
        let request_for = |page_box, force_page_box, source: &[u8]| {
            let mut stores = Universe::new_with_plain_catcodes();
            crate::prepare_pdftex_run_stores(&mut stores);
            stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
            stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_PAGE_BOX, page_box);
            stores.set_int_param_global(
                tex_state::env::banks::IntParam::PDF_FORCE_PAGE_BOX,
                force_page_box,
            );
            let mut input = InputStack::new(MemoryInput::new("legacy input"));
            let mut session = EngineSession::with_command_profile(
                &mut input,
                &mut stores,
                ExecutionContext::new("pdf-image"),
                tex_command::CommandProfile::PDFTEX14027,
            );
            session
                .register_canonical_root(
                    "job.tex",
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(source),
                )
                .expect("root registers");
            match session.advance_canonical().expect("image scan") {
                CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { request }) => {
                    request
                }
                other => panic!("expected image suspension, got {other:?}"),
            }
        };

        assert_eq!(
            request_for(1, 0, b"\\pdfximage {image.pdf}").page_box,
            tex_command::PdfImagePageBox::Media,
            "the live pdfpagebox default is part of the host identity"
        );
        assert_eq!(
            request_for(2, 5, b"\\pdfximage mediabox {image.pdf}").page_box,
            tex_command::PdfImagePageBox::Art,
            "pdfforcepagebox overrides an explicit selector before acquisition"
        );
        assert_eq!(
            request_for(4, 0, b"\\pdfximage mediabox {image.pdf}").page_box,
            tex_command::PdfImagePageBox::Media,
            "modern pdfpagebox does not override an explicit selector"
        );
    }

    #[test]
    fn canonical_pdfximage_rejects_dvi_mode_before_resource_acquisition() {
        let mut stores = Universe::new_with_plain_catcodes();
        crate::prepare_pdftex_run_stores(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("legacy input"));
        let mut session = EngineSession::with_command_profile(
            &mut input,
            &mut stores,
            ExecutionContext::new("pdf-image"),
            tex_command::CommandProfile::PDFTEX14027,
        );
        session
            .register_canonical_root(
                "job.tex",
                RegisteredSourceKind::Generated,
                Arc::from(&b"\\pdfximage {unavailable.png}"[..]),
            )
            .expect("root registers");
        assert!(matches!(
            session.advance_canonical(),
            Err(tex_exec::ExecError::PdfExtensionInDviMode("pdfximage"))
        ));
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // Verifies real host ordering at the World boundary.
    fn driver_materialization_follows_engine_effect_commit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let output = temp.path().join("shared.out");
        let mut stores = Universe::with_world(World::real()).with_plain_catcodes();
        let slot = StreamSlot::new(1);
        stores.world_mut().open_out(slot, &output);
        stores
            .world_mut()
            .write_text(PrintSink::Stream(slot), "engine");
        let plan = PlannedFinalization::new(
            stores.world().effect_pos(),
            vec![DriverFile::new(output.clone(), b"driver".to_vec())],
        )
        .expect("paths are distinct");

        plan.commit_effects(&mut stores)
            .expect("effects commit")
            .materialize(&mut stores)
            .expect("driver materializes");

        assert_eq!(std::fs::read(output).expect("read output"), b"driver");
    }

    #[test]
    fn failed_effect_commit_cannot_materialize_driver_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut stores = Universe::with_world(World::real()).with_plain_catcodes();
        let slot = StreamSlot::new(1);
        stores.world_mut().open_out(slot, temp.path());
        stores
            .world_mut()
            .write_text(PrintSink::Stream(slot), "cannot write a directory");
        let driver_path = temp.path().join("driver.dvi");
        let plan = PlannedFinalization::new(
            stores.world().effect_pos(),
            vec![DriverFile::new(driver_path.clone(), b"driver".to_vec())],
        )
        .expect("paths are distinct");

        assert!(plan.commit_effects(&mut stores).is_err());
        assert!(!driver_path.exists());
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // Verifies retry ordering against the real backend.
    fn retryable_finalization_keeps_plan_and_does_not_replay_committed_prefix() {
        let temp = tempfile::tempdir().expect("temp dir");
        let prefix_path = temp.path().join("prefix.out");
        let replacement_path = temp.path().join("replacement.out");
        let driver_path = temp.path().join("driver.dvi");
        let mut stores = Universe::with_world(World::real()).with_plain_catcodes();
        let prefix_slot = StreamSlot::new(1);
        let retry_slot = StreamSlot::new(2);
        stores.world_mut().open_out(prefix_slot, &prefix_path);
        stores
            .world_mut()
            .write_text(PrintSink::Stream(prefix_slot), "once");
        stores.world_mut().open_out(retry_slot, temp.path());
        stores
            .world_mut()
            .write_text(PrintSink::Stream(retry_slot), "suffix");
        let plan = PlannedFinalization::new(
            stores.world().effect_pos(),
            vec![DriverFile::new(driver_path.clone(), b"driver".to_vec())],
        )
        .expect("plan");

        let FinalizationCommit::Retry { plan, error } = plan
            .commit_effects_retryable(&mut stores)
            .expect("retry-safe failure is retained")
        else {
            panic!("directory open must suspend finalization");
        };
        let failed = error
            .stream_open_unavailable()
            .expect("typed unavailable open")
            .clone();
        assert_eq!(failed.path(), temp.path());
        assert_eq!(
            std::fs::read(&prefix_path).expect("committed prefix"),
            b"once"
        );
        assert!(!driver_path.exists());

        stores
            .world_mut()
            .retarget_pending_stream_open(&failed, &replacement_path)
            .expect("retarget pending open");
        let FinalizationCommit::Committed(committed) = plan
            .commit_effects_retryable(&mut stores)
            .expect("replacement commits")
        else {
            panic!("replacement must finish the retained plan");
        };
        committed
            .materialize(&mut stores)
            .expect("driver materializes");

        assert_eq!(std::fs::read(prefix_path).expect("prefix remains"), b"once");
        assert_eq!(
            std::fs::read(replacement_path).expect("suffix commits"),
            b"suffix"
        );
        assert_eq!(std::fs::read(driver_path).expect("driver"), b"driver");
    }

    #[test]
    fn duplicate_driver_paths_are_rejected_before_finalization() {
        let stores = Universe::with_world(World::memory()).with_plain_catcodes();
        let result = PlannedFinalization::new(
            stores.world().effect_pos(),
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
        let stores = Universe::with_world(World::memory()).with_plain_catcodes();
        let result = PlannedFinalization::new(
            stores.world().effect_pos(),
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
            stores.world().effect_pos(),
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
        let mut stores = Universe::with_world(World::memory()).with_plain_catcodes();
        stores
            .world_mut()
            .write_text(PrintSink::Terminal, "fixture");
        let plan = PlannedFinalization::new(
            stores.world().effect_pos(),
            vec![DriverFile::new(PathBuf::from("fixture.dvi"), vec![1])],
        )
        .expect("path is unique");

        plan.discard_uncommitted();

        assert_eq!(stores.world().effect_records().len(), 1);
        assert_eq!(stores.world().memory_output("fixture.dvi"), None);
    }
}
