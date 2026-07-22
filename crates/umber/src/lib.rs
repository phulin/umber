use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tex_exec::{
    CheckpointSink, ExecutionContext, ExecutionStats, Executor, FontResolver, PdfImageRequest,
    PdfImageResolver, try_execute_assignment,
};
use tex_expand::{InputResolver, get_x_token_with_context};
use tex_lex::{InputSource, InputStack, MemoryInput};
use tex_out::dvi::{DviError, DviPagePlan, DviStreamWriter};
use tex_state::env::banks::IntParam;
use tex_state::token::TracedTokenWord;
use tex_state::{
    CommittedArtifact, ContentHash, EffectPos, EffectRecord, ExpansionContext, PrintSink, Universe,
    WorldCommitMode, WorldError,
};

#[cfg(not(target_arch = "wasm32"))]
pub mod cli_resource;
mod input_observation;
mod input_search;
mod latex_project;
mod memory_output;
mod pdf_import;
mod pdf_output;
mod pdf_vf;
mod pdftex;
mod virtual_compile;

pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

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
pub use tex_fonts::{
    AcceptedFontContainers, FeatureSetting, FontContainer, FontFeaturePolicy, FontLanguage,
    FontLayoutPolicy, FontMappingFallbackPolicy, FontObjectIdentity, FontProgramIdentity,
    FontPurposes, FontRequest, FontRequestKey, LegacyFontMapping, OpenTypeTag, PdfPkFontRequest,
    ResolvedFont, VariationCoordinate, VariationInstance, VariationSelection, WritingDirection,
};
pub use tex_incr::{RenderedOutputId, ReuseMetrics, RevisionId, SameHistoryStop};
pub use tex_state::{InputDependency, InputDependencyAccess, InputDependencyOutcome};
pub use umber_vfs::FileContentId;
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
    artifact_cursor: usize,
    checkpoint_policy: CheckpointPolicy,
}

impl<'a, 'context> EngineSession<'a, 'context> {
    pub fn new(
        input: &'a mut InputStack,
        stores: &'a mut Universe,
        context: ExecutionContext<'context>,
    ) -> Self {
        let artifact_cursor = stores.world().artifact_commits().len();
        Self {
            input,
            stores,
            context,
            artifact_cursor,
            checkpoint_policy: CheckpointPolicy::NamedExecutorBoundaries,
        }
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

    pub fn execute(&mut self) -> Result<RunResult, tex_exec::ExecError> {
        let artifact_start = self.artifact_cursor;
        let stats = Executor::new().run_with_context(self.input, self.stores, &mut self.context)?;
        Ok(self.finish_execution(artifact_start, stats))
    }

    /// Executes while publishing restartable state at named safe boundaries.
    pub fn execute_with_checkpoints<C: CheckpointSink>(
        &mut self,
        checkpoints: &mut C,
    ) -> Result<RunResult, tex_exec::ExecError> {
        let artifact_start = self.artifact_cursor;
        let stats = Executor::new().run_with_context_and_checkpoints(
            self.input,
            self.stores,
            &mut self.context,
            checkpoints,
        )?;
        Ok(self.finish_execution(artifact_start, stats))
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
            artifacts: stats.shipped_artifacts,
            dvi_pages: stats.dvi_pages,
            committed_artifacts: self.stores.world().committed_artifacts()
                [artifact_start..self.artifact_cursor]
                .to_vec(),
            dumped_format: stats.dumped_format,
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
        request: &PdfImageRequest,
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
    pub artifacts: Vec<ContentHash>,
    /// Precompiled page-local DVI bodies aligned with `artifacts`.
    pub dvi_pages: Vec<DviPagePlan>,
    /// Exact canonical bytes from this execution's successful shipout commits.
    pub committed_artifacts: Vec<CommittedArtifact>,
    pub dumped_format: bool,
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
}

impl PlannedFinalization {
    pub fn new(effect_pos: EffectPos, files: Vec<DriverFile>) -> Result<Self, FinalizationError> {
        let mut paths = BTreeSet::new();
        for file in &files {
            if !paths.insert(file.path.clone()) {
                return Err(FinalizationError::ConflictingDriverPath(file.path.clone()));
            }
        }
        Ok(Self { effect_pos, files })
    }

    pub fn commit_effects(
        self,
        stores: &mut Universe,
    ) -> Result<CommittedFinalization, FinalizationError> {
        if stores.world().commit_mode() == WorldCommitMode::Retained {
            debug_assert_eq!(self.effect_pos, stores.world().effect_pos());
            stores.export_retained_effects()?;
        } else {
            stores.commit_effects(self.effect_pos)?;
        }
        Ok(CommittedFinalization { files: self.files })
    }

    /// Explicit fixture policy: retain effect records and materialize nothing.
    pub fn discard_uncommitted(self) {}
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

/// Installs the primitive/state setup used by `umber run`.
pub fn prepare_run_stores(stores: &mut Universe) {
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    tex_expand::install_expandable_primitives(stores);
    tex_exec::install_unexpandable_primitives(stores);
    stores.intern("par");
}

/// Installs the primitive/state setup used by `umber run --etex`.
pub fn prepare_etex_run_stores(stores: &mut Universe) {
    prepare_run_stores(stores);
    tex_expand::install_etex_expandable_primitives(stores);
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
    tex_expand::register_expandable_primitives(stores);
    tex_expand::register_etex_expandable_primitives(stores);
    tex_exec::register_unexpandable_primitives(stores);
    tex_exec::register_etex_unexpandable_primitives(stores);
    pdftex::register_pdftex_layer(stores);
    stores.enable_pdf_output();
}

fn register_tex_format_primitives(stores: &mut Universe) {
    tex_expand::register_expandable_primitives(stores);
    tex_exec::register_unexpandable_primitives(stores);
}

fn register_etex_format_primitives(stores: &mut Universe) {
    register_tex_format_primitives(stores);
    tex_expand::register_etex_expandable_primitives(stores);
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
    use tex_state::World;
    use tex_state::env::banks::TokParam;
    use tex_state::ids::TokenListId;
    use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
    use tex_state::token::{Catcode, Token};

    #[test]
    fn latex_format_restores_frozen_base_primitives_without_rebinding_live_names() {
        let mut stores = Universe::with_world(World::memory());
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
            ("ifincsname", ExpandablePrimitive::IfInCsName),
            ("iffontchar", ExpandablePrimitive::IfFontChar),
        ] {
            let symbol = extended.intern(name);
            assert_eq!(
                extended.meaning(symbol),
                Meaning::ExpandablePrimitive(primitive)
            );
        }
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

/// Runs an already-open input stack through the same executor path as `umber run`.
pub fn run_input_with_context(
    input: &mut InputStack,
    stores: &mut Universe,
    context: ExecutionContext<'_>,
) -> Result<String, tex_exec::ExecError> {
    run_input_collecting_artifacts(input, stores, context).map(|result| result.terminal_text)
}

/// Runs input and returns the artifact ids emitted by `\shipout` in order.
pub fn run_input_collecting_artifacts(
    input: &mut InputStack,
    stores: &mut Universe,
    context: ExecutionContext<'_>,
) -> Result<RunResult, tex_exec::ExecError> {
    EngineSession::new(input, stores, context).execute()
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
) -> Result<String, tex_exec::ExecError> {
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut input_resolver = RejectingMemoryInputResolver;
    let mut font_resolver = DirectFontResolver;
    let context =
        ExecutionContext::with_resolvers("texput", &mut input_resolver, &mut font_resolver);
    run_input_with_context(&mut input, stores, context)
}

#[derive(Clone, Copy, Debug, Default)]
struct RejectingMemoryInputResolver;

impl InputResolver for RejectingMemoryInputResolver {
    fn open_input(
        &mut self,
        _input: &mut dyn tex_state::InputReadState,
        name: &str,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<Box<dyn InputSource>> {
        let _ = name;
        Ok(tex_expand::ResourceLookup::Unavailable)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DirectFontResolver;

impl FontResolver for DirectFontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        path: &Path,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<tex_exec::FontSource> {
        Ok(match input.read_input_file(path) {
            Ok(metrics) => tex_expand::ResourceLookup::Available(tex_exec::FontSource::Tfm {
                metrics,
                opentype: None,
            }),
            Err(_) => tex_expand::ResourceLookup::Unavailable,
        })
    }
}

fn uncommitted_terminal_text(stores: &Universe) -> String {
    let mut text = String::new();
    for record in stores.world().effect_records() {
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
    use super::{DriverFile, FinalizationError, PlannedFinalization};
    use std::path::PathBuf;
    use tex_state::{PrintSink, StreamSlot, Universe, World};

    #[test]
    #[allow(clippy::disallowed_methods)] // Verifies real host ordering at the World boundary.
    fn driver_materialization_follows_engine_effect_commit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let output = temp.path().join("shared.out");
        let mut stores = Universe::with_world(World::real());
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
        let mut stores = Universe::with_world(World::real());
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
    fn duplicate_driver_paths_are_rejected_before_finalization() {
        let stores = Universe::with_world(World::memory());
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
    fn fixture_policy_preserves_effects_without_materializing_files() {
        let mut stores = Universe::with_world(World::memory());
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
