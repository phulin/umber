//! Native host policy for driving one CLI compile through the resource loop.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::Instant;

use sha2::{Digest, Sha256};
use tex_fonts::AcceptedFontContainers;
use tex_state::{Universe, World};
use umber_distribution::{
    DependencyHint, FileKind as DistributionFileKind, FileRequestKey as DistributionFileRequestKey,
    ManifestShard, ShardFile, ShardedManifestRoot,
};
use umber_fetch::{
    FetchCancellation, FetchClient, FetchClientConfig, FetchFailure, FetchRequest,
    ManifestFetchError, ObjectCache, fetch_manifest_cancellable,
};

use crate::{
    AcceptedFinalization, CompileAttemptResult, CompileTelemetry, EngineMode, FileContentId,
    FileKind, FileRequest, MemoryRunOutput, NeedResources, OutputCapability, OutputCapabilitySet,
    ResolvedFile, ResolvedPkFont, ResourceRequest, ResourceResponse, SessionLimits, SessionOptions,
    SourcePatch, TexFontSearchPath, TexInputSearchPath, VirtualCompileSession,
};

pub const DEFAULT_DISTRIBUTION_URL: &str =
    "https://assets.umber.ink/texlive/texlive-20260301/manifest-v3.json";
pub const DEFAULT_DISTRIBUTION_SHA256: &str =
    "43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b";

const MAX_INDEX_SHARD_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct NativeRunOptions {
    pub input: PathBuf,
    pub format: Option<PathBuf>,
    pub initial_prefetch_keys: Vec<String>,
    pub engine: EngineMode,
    pub outputs: OutputCapabilitySet,
    pub html_asset_directory: Option<String>,
    pub distribution: Option<String>,
    pub distribution_sha256: Option<String>,
    pub offline: bool,
    pub expansion_fuel: Option<u64>,
}

#[derive(Debug)]
pub enum NativeRunError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Cache(String),
    ManifestFetch(String),
    ManifestDigestMismatch {
        expected: String,
        actual: String,
    },
    ManifestParse(String),
    ManifestTooLarge {
        label: String,
        limit: u64,
    },
    DistributionPinRequired(String),
    DistributionUnavailable(Vec<String>),
    Selection(String),
    Fetch(String),
    Compile(String),
    Format(String),
    Cancelled,
}

impl fmt::Display for NativeRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "failed to read {}: {source}", path.display()),
            Self::Cache(message) => write!(f, "distribution cache error: {message}"),
            Self::ManifestFetch(message) => write!(f, "distribution manifest error: {message}"),
            Self::ManifestDigestMismatch { expected, actual } => write!(
                f,
                "distribution manifest digest mismatch: expected {expected}, received {actual}"
            ),
            Self::ManifestParse(message) => write!(f, "invalid distribution manifest: {message}"),
            Self::ManifestTooLarge { label, limit } => {
                write!(f, "{label} exceeds the {limit}-byte limit")
            }
            Self::DistributionPinRequired(source) => write!(
                f,
                "distribution {source} requires --distribution-sha256 (or UMBER_DISTRIBUTION_SHA256)"
            ),
            Self::DistributionUnavailable(keys) => write!(
                f,
                "distribution unavailable for required request(s): {}",
                keys.join(", ")
            ),
            Self::Selection(message) => write!(f, "distribution selection error: {message}"),
            Self::Fetch(message) => f.write_str(message),
            Self::Compile(message) => f.write_str(message),
            Self::Format(message) => write!(f, "format resource error: {message}"),
            Self::Cancelled => f.write_str("distribution acquisition cancelled"),
        }
    }
}

impl Error for NativeRunError {}

pub fn run(options: &NativeRunOptions) -> Result<MemoryRunOutput, NativeRunError> {
    NativeCompileSession::new(options, &FetchCancellation::new())?
        .compile(&FetchCancellation::new())
}

pub struct NativeAcceptedRun {
    output: MemoryRunOutput,
    finalization: AcceptedFinalization,
    input_path_map: BTreeMap<PathBuf, PathBuf>,
    resolved_inputs: Vec<(PathBuf, usize)>,
    main_input: (PathBuf, usize),
    telemetry: CompileTelemetry,
    host_telemetry: NativeHostTelemetry,
    distribution: DistributionResolver,
    local: LocalResolver,
}

pub type NativeAcceptedParts = (
    MemoryRunOutput,
    AcceptedFinalization,
    BTreeMap<PathBuf, PathBuf>,
    Vec<(PathBuf, usize)>,
    (PathBuf, usize),
    CompileTelemetry,
    NativeHostTelemetry,
);

/// Mutually exclusive native host phases around the engine's typed resource loop.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeHostTelemetry {
    pub startup_time: Duration,
    pub compile_attempt_time: Duration,
    pub resolver_time: Duration,
    pub preload_time: Duration,
    pub provision_time: Duration,
    pub accepted_handoff_time: Duration,
    pub resolver: ResolverTelemetry,
}

/// Nested resolver phases and cache outcomes. Phase durations are mutually exclusive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResolverTelemetry {
    pub local_lookup_time: Duration,
    pub manifest_lookup_time: Duration,
    pub object_load_time: Duration,
    pub content_hash_time: Duration,
    pub response_build_time: Duration,
    pub local_lookups: u64,
    pub local_hits: u64,
    pub manifest_lookups: u64,
    pub manifest_cache_hits: u64,
    pub object_requests: u64,
    pub object_cache_hits: u64,
}

impl NativeAcceptedRun {
    #[must_use]
    pub fn pdf_draft_mode(&self) -> bool {
        self.finalization
            .stores
            .fixed_pdf_output_parameters()
            .is_some_and(|parameters| parameters.draft_mode > 0)
    }

    pub fn provide_pdf_font_programs(&mut self) -> Result<(), NativeRunError> {
        let cancellation = FetchCancellation::new();
        let distribution = &mut self.distribution;
        let local = &self.local;
        let virtual_names = self
            .finalization
            .virtual_font_resources
            .virtual_fonts
            .keys()
            .map(|name| name.as_bytes().to_vec())
            .collect();
        crate::provide_pdf_font_resources_excluding_at_dpi(
            &mut self.finalization.stores,
            crate::pdf_output::DEFAULT_PDF_PK_RESOLUTION,
            &virtual_names,
            |_stores, logical_name| {
                distribution
                    .resolve_generic_asset(local, logical_name, &cancellation)
                    .map_err(|error| error.to_string())
            },
        )
        .map_err(NativeRunError::Compile)
    }

    #[must_use]
    pub fn into_parts(self) -> NativeAcceptedParts {
        (
            self.output,
            self.finalization,
            self.input_path_map,
            self.resolved_inputs,
            self.main_input,
            self.telemetry,
            self.host_telemetry,
        )
    }
}

#[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
pub fn run_for_finalization(
    options: &NativeRunOptions,
) -> Result<NativeAcceptedRun, NativeRunError> {
    let cancellation = FetchCancellation::new();
    let mut session = NativeCompileSession::new(options, &cancellation)?;
    let output = session.compile(&cancellation)?;
    let accepted_handoff_started = Instant::now();
    let input_path_map = session.local.input_path_map();
    let resolved_inputs = session.local.resolved_inputs();
    let main_input = (options.input.clone(), session.source.len());
    let telemetry = session.session.compile_telemetry();
    let mut host_telemetry = session.host_telemetry;
    let NativeCompileSession {
        session,
        distribution,
        local,
        ..
    } = session;
    let mut finalization = session
        .into_accepted_finalization()
        .map_err(|error| NativeRunError::Compile(error.to_string()))?;
    finalization
        .stores
        .world_mut()
        .retarget_output_backend(&World::real())
        .map_err(|error| NativeRunError::Compile(error.to_string()))?;
    host_telemetry.accepted_handoff_time = accepted_handoff_started.elapsed();
    Ok(NativeAcceptedRun {
        output,
        finalization,
        input_path_map,
        resolved_inputs,
        main_input,
        telemetry,
        host_telemetry,
        distribution,
        local,
    })
}

/// Retained native resource and incremental compile state used by `run` and
/// long-lived watch sessions.
pub struct NativeCompileSession {
    session: VirtualCompileSession,
    distribution: DistributionResolver,
    local: LocalResolver,
    source: String,
    pending_source: Option<String>,
    host_telemetry: NativeHostTelemetry,
}

impl NativeCompileSession {
    pub fn new(
        options: &NativeRunOptions,
        cancellation: &FetchCancellation,
    ) -> Result<Self, NativeRunError> {
        let cache = ObjectCache::from_environment()
            .map_err(|error| NativeRunError::Cache(error.to_string()))?;
        Self::new_with_cache(options, cancellation, cache)
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn new_with_cache(
        options: &NativeRunOptions,
        cancellation: &FetchCancellation,
        cache: ObjectCache,
    ) -> Result<Self, NativeRunError> {
        let setup_started = std::time::Instant::now();
        let source_started = std::time::Instant::now();
        let main = read(&options.input)?;
        let source_read_ns = source_started.elapsed().as_nanos();
        let mut distribution = DistributionResolver::new(
            cache,
            options.distribution.clone(),
            options.distribution_sha256.clone(),
            options.offline,
        );
        let format_started = std::time::Instant::now();
        let (format, format_prefetch_hints) = match &options.format {
            Some(path) if path.exists() => (Some(read(path)?), Vec::new()),
            Some(path) => {
                let resolved = distribution.resolve_format(path, options.engine, cancellation)?;
                (Some(resolved.bytes), resolved.prefetch_hints)
            }
            None => (None, Vec::new()),
        };
        let format_read_ns = format_started.elapsed().as_nanos();
        let mut initial_prefetch_hints = options
            .initial_prefetch_keys
            .iter()
            .map(|key| {
                DistributionFileRequestKey::from_manifest_key(key)
                    .map_err(|error| NativeRunError::Selection(error.to_string()))
                    .and_then(distribution_request)
            })
            .collect::<Result<Vec<_>, _>>()?;
        initial_prefetch_hints.extend(format_prefetch_hints);
        let clock = World::real().job_clock();
        let name = options
            .input
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("main.tex");
        let job_name = options
            .input
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("texput")
            .to_owned();
        let engine_fuel = options
            .expansion_fuel
            .or(env::var("UMBER_ENGINE_FUEL")
                .ok()
                .map(|value| {
                    value.parse::<u64>().map_err(|_| {
                        NativeRunError::Selection(format!(
                            "UMBER_ENGINE_FUEL must be an unsigned integer: {value}"
                        ))
                    })
                })
                .transpose()?)
            .unwrap_or(SessionLimits::default().engine_fuel);
        let env_limit = |name: &'static str, default: u64| -> Result<u64, NativeRunError> {
            env::var(name).map_or(Ok(default), |value| {
                value.parse::<u64>().map_err(|_| {
                    NativeRunError::Selection(format!(
                        "{name} must be an unsigned integer: {value}"
                    ))
                })
            })
        };
        let defaults = SessionLimits::default();
        let engine_steps = env_limit("UMBER_ENGINE_STEPS", defaults.engine_steps)?;
        let input_frames = env_limit("UMBER_INPUT_FRAMES", defaults.input_frames)?;
        let journal_bytes = env_limit("UMBER_JOURNAL_BYTES", defaults.journal_bytes)?;
        let effects = env_limit("UMBER_EFFECTS", defaults.effects)?;
        let restore_started = std::time::Instant::now();
        let mut session = VirtualCompileSession::new(SessionOptions {
            main_path: format!("/job/{name}"),
            job_name: Some(job_name),
            format,
            initial_prefetch_hints: (!initial_prefetch_hints.is_empty())
                .then(|| initial_prefetch_hints.into_boxed_slice()),
            engine: options.engine,
            clock,
            limits: SessionLimits {
                attempts: SessionLimits::HARD_MAX.attempts,
                engine_fuel,
                engine_steps,
                input_frames,
                journal_bytes,
                effects,
                ..SessionLimits::default()
            },
            outputs: options.outputs,
            html_asset_mode: options.html_asset_directory.as_ref().map_or(
                tex_out::html::AssetMode::Embedded,
                |relative_directory| tex_out::html::AssetMode::Manifest {
                    relative_directory: relative_directory.clone(),
                },
            ),
            accepted_font_containers: if options.outputs.contains(OutputCapability::Html) {
                AcceptedFontContainers::WASM
            } else {
                AcceptedFontContainers::NATIVE_WITH_COLLECTIONS
            },
            font_layout_policy: if options.outputs.contains(OutputCapability::Html) {
                tex_fonts::FontLayoutPolicy::OpenTypePreferred
            } else {
                tex_fonts::FontLayoutPolicy::ClassicTfmExact
            },
            font_mapping_fallback: tex_fonts::FontMappingFallbackPolicy::ClassicTfmExact,
        })
        .map_err(|error| NativeRunError::Compile(error.to_string()))?;
        let format_restore_ns = restore_started.elapsed().as_nanos();
        session
            .add_user_file(name, main.clone())
            .map_err(|error| NativeRunError::Compile(error.to_string()))?;
        let local = LocalResolver::from_environment(&options.input);
        let source = match String::from_utf8(main) {
            Ok(source) => source,
            Err(error) => error.into_bytes().into_iter().map(char::from).collect(),
        };
        if env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
            eprintln!(
                "RESOURCE_STARTUP_TELEMETRY source_read_ns={} format_read_ns={} format_restore_ns={} setup_ns={}",
                source_read_ns,
                format_read_ns,
                format_restore_ns,
                setup_started.elapsed().as_nanos()
            );
        }
        let startup_time = setup_started.elapsed();
        Ok(Self {
            session,
            distribution,
            local,
            source,
            pending_source: None,
            host_telemetry: NativeHostTelemetry {
                startup_time,
                ..NativeHostTelemetry::default()
            },
        })
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    pub fn compile(
        &mut self,
        cancellation: &FetchCancellation,
    ) -> Result<MemoryRunOutput, NativeRunError> {
        loop {
            if cancellation.is_cancelled() {
                self.session.discard_suspended_candidate();
                return Err(NativeRunError::Cancelled);
            }
            let compile_attempt_started = Instant::now();
            let attempt = self.session.compile_attempt();
            self.host_telemetry.compile_attempt_time = self
                .host_telemetry
                .compile_attempt_time
                .saturating_add(compile_attempt_started.elapsed());
            match attempt {
                CompileAttemptResult::Complete(output) => {
                    if let Some(source) = self.pending_source.take() {
                        self.source = source;
                    }
                    return Ok(output);
                }
                CompileAttemptResult::Error(error) => {
                    return Err(NativeRunError::Compile(error.to_string()));
                }
                CompileAttemptResult::NeedResources(batch) => {
                    let resolver_started = Instant::now();
                    let resolved = match self.distribution.resolve_batch_with_prefetch(
                        &self.local,
                        &batch,
                        cancellation,
                        &mut self.host_telemetry.resolver,
                    ) {
                        Ok(resolved) => resolved,
                        Err(error) => {
                            self.session.discard_suspended_candidate();
                            return Err(error);
                        }
                    };
                    self.host_telemetry.resolver_time = self
                        .host_telemetry
                        .resolver_time
                        .saturating_add(resolver_started.elapsed());
                    if cancellation.is_cancelled() {
                        self.session.discard_suspended_candidate();
                        return Err(NativeRunError::Cancelled);
                    }
                    // Prefetch hints remain client-cache concerns; only requested
                    // resources cross the typed provisioning boundary.
                    let provision_started = Instant::now();
                    if let Err(error) = self.session.provide_resources(resolved.responses) {
                        self.session.discard_suspended_candidate();
                        return Err(NativeRunError::Compile(error.to_string()));
                    }
                    self.host_telemetry.provision_time = self
                        .host_telemetry
                        .provision_time
                        .saturating_add(provision_started.elapsed());
                }
            }
        }
    }

    pub fn into_accepted_finalization(self) -> Result<AcceptedFinalization, NativeRunError> {
        self.session
            .into_accepted_finalization()
            .map_err(|error| NativeRunError::Compile(error.to_string()))
    }

    pub fn apply_source(
        &mut self,
        next_revision: tex_incr::RevisionId,
        next: &str,
    ) -> Result<(), NativeRunError> {
        let base_revision = self.session.revision().ok_or_else(|| {
            NativeRunError::Compile("the initial revision has not been accepted".into())
        })?;
        let expected_hash = self.session.content_hash().ok_or_else(|| {
            NativeRunError::Compile("the accepted source has no content hash".into())
        })?;
        let (range, replacement) = contiguous_edit(&self.source, next);
        self.session
            .apply_patch(SourcePatch {
                next_revision,
                base_revision,
                expected_hash,
                range,
                replacement,
            })
            .map_err(|error| NativeRunError::Compile(error.to_string()))?;
        self.pending_source = Some(next.to_owned());
        Ok(())
    }

    pub fn cancel_pending_revision(&mut self) -> bool {
        let cancelled = self.session.cancel_pending_patch();
        if cancelled {
            self.pending_source = None;
        }
        cancelled
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn reuse_metrics(&self) -> Option<tex_incr::ReuseMetrics> {
        self.session.reuse_metrics()
    }

    #[must_use]
    pub fn revision(&self) -> Option<tex_incr::RevisionId> {
        self.session.revision()
    }
}

fn contiguous_edit(old: &str, new: &str) -> (std::ops::Range<usize>, String) {
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    (
        prefix..old.len() - suffix,
        new[prefix..new.len() - suffix].to_owned(),
    )
}

struct LocalResolver {
    base: PathBuf,
    roots: Vec<PathBuf>,
    input: TexInputSearchPath,
    font: TexFontSearchPath,
    input_paths: RefCell<BTreeMap<PathBuf, PathBuf>>,
    resolved_inputs: RefCell<Vec<(PathBuf, usize)>>,
}

impl LocalResolver {
    fn from_environment(main: &Path) -> Self {
        let areas = |name| {
            env::var_os(name)
                .map(|value| {
                    env::split_paths(&value)
                        .filter(|path| !path.as_os_str().is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let base = main.parent().unwrap_or_else(|| Path::new(".")).to_owned();
        let input_areas = areas("TEXINPUTS");
        let font_areas = areas("TEXFONTS");
        let mut roots = vec![base.clone()];
        roots.extend(input_areas.iter().cloned());
        roots.extend(font_areas.iter().cloned());
        Self {
            base: base.clone(),
            roots,
            input: TexInputSearchPath::new(&base, input_areas),
            font: TexFontSearchPath::new(base, font_areas),
            input_paths: RefCell::new(BTreeMap::new()),
            resolved_inputs: RefCell::new(Vec::new()),
        }
    }

    fn resolve(&self, request: &FileRequest) -> Option<ResolvedFile> {
        if matches!(
            request.key().kind(),
            FileKind::BibAux | FileKind::ClassicBibData | FileKind::BibStyle
        ) {
            return self.resolve_classic_bibliography(request);
        }
        let mut world = World::real();
        let content = match request.key().kind() {
            FileKind::TexInput | FileKind::Image => self
                .input
                .read_from_world(&mut world, request.original_name()),
            FileKind::Tfm => self
                .font
                .read_from_world(&mut world, Path::new(request.original_name())),
            FileKind::GenericAsset
            | FileKind::VirtualFont
            | FileKind::PdfFontMap
            | FileKind::PdfEncoding
            | FileKind::PdfFontProgram => self
                .font
                .read_program_from_world(&mut world, Path::new(request.original_name())),
            _ => return None,
        }
        .ok()?;
        let bytes = content.bytes().to_vec();
        self.resolved_inputs
            .borrow_mut()
            .push((content.path().to_owned(), bytes.len()));
        let digest = FileContentId::for_bytes(&bytes);
        let virtual_path = self.virtual_path(request.key().kind(), content.path(), digest);
        self.input_paths
            .borrow_mut()
            .insert(virtual_path.clone(), content.path().to_owned());
        Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: virtual_path.to_string_lossy().into_owned(),
            expected_digest: Some(digest),
            bytes,
        })
    }

    fn resolve_font(
        &self,
        request: &tex_fonts::FontRequest,
    ) -> Result<Option<tex_fonts::ResolvedFont>, NativeRunError> {
        let _ = request;
        Ok(None)
    }

    fn resolve_pk_font(&self, request: &tex_fonts::PdfPkFontRequest) -> Option<ResolvedPkFont> {
        let name = String::from_utf8(request.logical_name()).ok()?;
        let mut world = World::real();
        let content = self
            .font
            .read_program_from_world(&mut world, Path::new(&name))
            .ok()?;
        let bytes = content.bytes().to_vec();
        self.resolved_inputs
            .borrow_mut()
            .push((content.path().to_owned(), bytes.len()));
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let virtual_path = self.virtual_path(
            FileKind::GenericAsset,
            content.path(),
            FileContentId::for_bytes(&bytes),
        );
        Some(ResolvedPkFont {
            request: request.clone(),
            virtual_path: virtual_path.to_string_lossy().into_owned(),
            bytes,
            expected_sha256: Some(digest),
        })
    }

    fn virtual_path(&self, kind: FileKind, path: &Path, digest: FileContentId) -> PathBuf {
        let relative = self
            .roots
            .iter()
            .filter_map(|root| path.strip_prefix(root).ok())
            .min_by_key(|path| path.components().count());
        relative.map_or_else(
            || PathBuf::from(format!("/texlive/local/{}/{digest}", kind.wire_name())),
            |relative| {
                Path::new("/texlive/local")
                    .join(kind.wire_name())
                    .join(relative)
            },
        )
    }

    fn resolve_classic_bibliography(&self, request: &FileRequest) -> Option<ResolvedFile> {
        let (variable, extension) = match request.key().kind() {
            FileKind::BibAux => ("TEXINPUTS", ".aux"),
            FileKind::ClassicBibData => ("BIBINPUTS", ".bib"),
            FileKind::BibStyle => ("BSTINPUTS", ".bst"),
            _ => return None,
        };
        let mut world = World::real();
        let content = read_classic_bib_resource(
            &mut world,
            &self.base,
            variable,
            request.original_name(),
            extension,
        )
        .ok()?;
        let path = content.path().to_owned();
        let bytes = content.bytes().to_vec();
        self.resolved_inputs
            .borrow_mut()
            .push((path.clone(), bytes.len()));
        let digest = FileContentId::for_bytes(&bytes);
        let virtual_path = self.virtual_path(request.key().kind(), &path, digest);
        self.input_paths
            .borrow_mut()
            .insert(virtual_path.clone(), path);
        Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: virtual_path.to_string_lossy().into_owned(),
            expected_digest: Some(digest),
            bytes,
        })
    }

    fn input_path_map(&self) -> BTreeMap<PathBuf, PathBuf> {
        self.input_paths.borrow().clone()
    }

    fn resolved_inputs(&self) -> Vec<(PathBuf, usize)> {
        self.resolved_inputs.borrow().clone()
    }
}

fn read_classic_bib_resource(
    world: &mut World,
    base: &Path,
    variable: &str,
    original: &str,
    extension: &str,
) -> Result<tex_state::FileContent, String> {
    let name = Path::new(original);
    let mut candidates = Vec::new();
    if name.is_absolute() {
        candidates.push(name.to_owned());
    } else {
        candidates.push(base.join(name));
        if let Some(areas) = env::var_os(variable) {
            candidates.extend(
                env::split_paths(&areas)
                    .filter(|area| !area.as_os_str().is_empty())
                    .map(|area| area.join(name)),
            );
        }
    }
    for mut candidate in candidates {
        if candidate.extension().is_none() {
            candidate.set_extension(extension.trim_start_matches('.'));
        }
        if let Ok(content) = world.read_file(&candidate) {
            return Ok(content);
        }
    }
    Err(format!("{original} was not found in {variable}"))
}

#[derive(Clone)]
struct LoadedDistribution {
    root: ShardedManifestRoot,
    local_root: Option<PathBuf>,
    shards: BTreeMap<u32, ManifestShard>,
}

struct ResolvedFormat {
    bytes: Vec<u8>,
    prefetch_hints: Vec<ResourceRequest>,
}

struct ResolvedDistributionBatch {
    responses: Vec<ResourceResponse>,
}

struct DistributionResolver {
    cache: ObjectCache,
    source: Option<String>,
    expected: Option<String>,
    offline: bool,
    loaded: Option<LoadedDistribution>,
}

impl DistributionResolver {
    fn new(
        cache: ObjectCache,
        source: Option<String>,
        expected: Option<String>,
        offline: bool,
    ) -> Self {
        Self {
            cache,
            source,
            expected,
            offline,
            loaded: None,
        }
    }

    #[cfg(test)]
    fn resolve_batch(
        &mut self,
        local: &LocalResolver,
        batch: &NeedResources,
        cancellation: &FetchCancellation,
    ) -> Result<Vec<ResourceResponse>, NativeRunError> {
        self.resolve_batch_with_prefetch(
            local,
            batch,
            cancellation,
            &mut ResolverTelemetry::default(),
        )
        .map(|resolved| resolved.responses)
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn resolve_batch_with_prefetch(
        &mut self,
        local: &LocalResolver,
        batch: &NeedResources,
        cancellation: &FetchCancellation,
        telemetry: &mut ResolverTelemetry,
    ) -> Result<ResolvedDistributionBatch, NativeRunError> {
        check_cancelled(cancellation)?;
        let mut responses = Vec::new();
        let mut unresolved = Vec::new();
        for request in batch.required.iter().chain(&batch.probes) {
            match request {
                ResourceRequest::File(request) => {
                    let started = Instant::now();
                    telemetry.local_lookups = telemetry.local_lookups.saturating_add(1);
                    let resolved = local.resolve(request);
                    telemetry.local_lookup_time = telemetry
                        .local_lookup_time
                        .saturating_add(started.elapsed());
                    if let Some(file) = resolved {
                        telemetry.local_hits = telemetry.local_hits.saturating_add(1);
                        responses.push(ResourceResponse::File(file));
                    } else {
                        unresolved.push(request.clone());
                    }
                }
                ResourceRequest::Font(request) => {
                    responses.push(local.resolve_font(request)?.map_or_else(
                        || ResourceResponse::FontUnavailable(request.key.clone()),
                        ResourceResponse::Font,
                    ));
                }
                ResourceRequest::PkFont(request) => {
                    let started = Instant::now();
                    telemetry.local_lookups = telemetry.local_lookups.saturating_add(1);
                    let resolved = local.resolve_pk_font(request);
                    telemetry.local_lookup_time = telemetry
                        .local_lookup_time
                        .saturating_add(started.elapsed());
                    if let Some(font) = resolved {
                        telemetry.local_hits = telemetry.local_hits.saturating_add(1);
                        responses.push(ResourceResponse::PkFont(font));
                    } else {
                        let file =
                            self.resolve_generic_file(local, &request.logical_name(), cancellation);
                        match file {
                            Ok(file) => responses.push(ResourceResponse::PkFont(ResolvedPkFont {
                                request: request.clone(),
                                virtual_path: file.virtual_path,
                                expected_sha256: Some(Sha256::digest(&file.bytes).into()),
                                bytes: file.bytes,
                            })),
                            Err(NativeRunError::DistributionUnavailable(_)) => {
                                responses.push(ResourceResponse::PkFontUnavailable(request.clone()))
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
            }
        }
        let mut unresolved_hints = Vec::new();
        for request in &batch.prefetch_hints {
            let ResourceRequest::File(request) = request else {
                continue;
            };
            let started = Instant::now();
            telemetry.local_lookups = telemetry.local_lookups.saturating_add(1);
            let resolved = local.resolve(request);
            telemetry.local_lookup_time = telemetry
                .local_lookup_time
                .saturating_add(started.elapsed());
            if let Some(file) = resolved {
                telemetry.local_hits = telemetry.local_hits.saturating_add(1);
                responses.push(ResourceResponse::File(file));
            } else {
                unresolved_hints.push(request.clone());
            }
        }
        if unresolved.is_empty() && unresolved_hints.is_empty() {
            return Ok(ResolvedDistributionBatch { responses });
        }
        let manifest_started = Instant::now();
        telemetry.manifest_lookups = telemetry.manifest_lookups.saturating_add(1);
        if self.loaded.is_some() {
            telemetry.manifest_cache_hits = telemetry.manifest_cache_hits.saturating_add(1);
        }
        let root = &self.load(cancellation)?.root;
        let shard_bits = root.shard_bits;
        let objects_base_url = root.objects_base_url.clone();
        telemetry.manifest_lookup_time = telemetry
            .manifest_lookup_time
            .saturating_add(manifest_started.elapsed());
        let mut original_files = BTreeMap::new();
        for request in &unresolved {
            let Some(key) = distribution_file_key(request)? else {
                responses.push(ResourceResponse::FileUnavailable(request.key().clone()));
                continue;
            };
            original_files.insert(key.manifest_key().to_string(), request.key().clone());
        }
        let mut keys_by_shard = BTreeMap::<u32, Vec<String>>::new();
        for key in original_files.keys() {
            keys_by_shard
                .entry(shard_index(key, shard_bits))
                .or_default()
                .push(key.clone());
        }
        let mut hinted_keys = BTreeMap::<u32, Vec<String>>::new();
        let mut original_hints = BTreeMap::new();
        for request in &unresolved_hints {
            let Some(key) = distribution_file_key(request)? else {
                continue;
            };
            let key = key.manifest_key().to_string();
            if original_files.contains_key(&key) {
                continue;
            }
            original_hints.insert(key.clone(), request.key().clone());
            hinted_keys
                .entry(shard_index(&key, shard_bits))
                .or_default()
                .push(key);
        }
        let mut required = BTreeMap::<String, ShardFile>::new();
        let mut hints = BTreeMap::<String, DependencyHint>::new();
        for (index, keys) in keys_by_shard {
            let manifest_started = Instant::now();
            telemetry.manifest_lookups = telemetry.manifest_lookups.saturating_add(1);
            if self
                .loaded
                .as_ref()
                .is_some_and(|loaded| loaded.shards.contains_key(&index))
            {
                telemetry.manifest_cache_hits = telemetry.manifest_cache_hits.saturating_add(1);
            }
            let shard = self.load_shard(index, cancellation)?;
            telemetry.manifest_lookup_time = telemetry
                .manifest_lookup_time
                .saturating_add(manifest_started.elapsed());
            for key in keys {
                let Some(entry) = shard.files.get(&key) else {
                    let original = original_files
                        .remove(&key)
                        .expect("requested key has an original file request");
                    responses.push(ResourceResponse::FileUnavailable(original));
                    continue;
                };
                required.insert(key.clone(), entry.clone());
                for dependency in &entry.dependencies {
                    hints
                        .entry(dependency.key.clone())
                        .or_insert_with(|| dependency.clone());
                }
            }
            if let Some(keys) = hinted_keys.remove(&index) {
                collect_closure_hints(shard, keys, &required, &mut hints);
            }
        }
        for (index, keys) in hinted_keys {
            let manifest_started = Instant::now();
            telemetry.manifest_lookups = telemetry.manifest_lookups.saturating_add(1);
            if self
                .loaded
                .as_ref()
                .is_some_and(|loaded| loaded.shards.contains_key(&index))
            {
                telemetry.manifest_cache_hits = telemetry.manifest_cache_hits.saturating_add(1);
            }
            match self.load_shard(index, cancellation) {
                Ok(shard) => {
                    telemetry.manifest_lookup_time = telemetry
                        .manifest_lookup_time
                        .saturating_add(manifest_started.elapsed());
                    collect_closure_hints(shard, keys, &required, &mut hints);
                }
                Err(NativeRunError::Cancelled) => return Err(NativeRunError::Cancelled),
                Err(_) => {}
            }
        }
        let mut locally_shadowed_hints = BTreeSet::new();
        for manifest_key in hints.keys() {
            let distribution_key = DistributionFileRequestKey::from_manifest_key(manifest_key)
                .map_err(|error| NativeRunError::Selection(error.to_string()))?;
            let ResourceRequest::File(request) = distribution_request(distribution_key)? else {
                continue;
            };
            let started = Instant::now();
            telemetry.local_lookups = telemetry.local_lookups.saturating_add(1);
            let resolved = local.resolve(&request);
            telemetry.local_lookup_time = telemetry
                .local_lookup_time
                .saturating_add(started.elapsed());
            if resolved.is_some() {
                telemetry.local_hits = telemetry.local_hits.saturating_add(1);
                locally_shadowed_hints.insert(manifest_key.clone());
            }
        }
        hints.retain(|key, _| !locally_shadowed_hints.contains(key));
        let required_fetches = required
            .iter()
            .map(|(key, entry)| FetchRequest {
                request_key: key.clone(),
                object: entry.object_entry(),
                max_bytes: crate::SessionLimits::default().one_file_bytes as u64,
            })
            .collect::<Vec<_>>();
        let limits = crate::SessionLimits::default();
        let mut hinted_files = required_fetches.len();
        let mut hinted_bytes = required_fetches
            .iter()
            .map(|request| request.object.bytes)
            .sum::<u64>();
        let mut hint_fetches = Vec::new();
        for (key, entry) in hints.iter().filter(|(key, _)| !required.contains_key(*key)) {
            let Some(next_files) = hinted_files.checked_add(1) else {
                break;
            };
            let Some(next_bytes) = hinted_bytes.checked_add(entry.bytes) else {
                break;
            };
            if next_files > limits.resolved_files || next_bytes > limits.cached_file_bytes as u64 {
                continue;
            }
            hinted_files = next_files;
            hinted_bytes = next_bytes;
            hint_fetches.push(FetchRequest {
                request_key: key.clone(),
                object: entry.object_entry(),
                max_bytes: limits.one_file_bytes as u64,
            });
        }
        let mut fetch_requests = required_fetches.clone();
        fetch_requests.extend(hint_fetches);
        telemetry.object_requests = telemetry
            .object_requests
            .saturating_add(fetch_requests.len() as u64);
        let object_started = Instant::now();
        let fetched = match self.fetch_objects(&objects_base_url, &fetch_requests, cancellation) {
            Ok(fetched) => fetched,
            Err(NativeRunError::Cancelled) => return Err(NativeRunError::Cancelled),
            Err(_) if fetch_requests.len() > required_fetches.len() => {
                self.fetch_objects(&objects_base_url, &required_fetches, cancellation)?
            }
            Err(error) => return Err(error),
        };
        telemetry.object_load_time = telemetry
            .object_load_time
            .saturating_add(object_started.elapsed());
        telemetry.object_cache_hits = telemetry.object_cache_hits.saturating_add(
            fetched
                .iter()
                .filter(|(_, _, cache_hit)| *cache_hit)
                .count() as u64,
        );
        if fetched.iter().any(|(_, _, cache_hit)| !cache_hit) {
            eprintln!("umber: acquired {} distribution resource(s)", fetched.len());
        }
        let mut bytes = fetched
            .into_iter()
            .map(|(key, bytes, _)| (key, bytes))
            .collect::<BTreeMap<_, _>>();
        let response_started = Instant::now();
        let hash_before = telemetry.content_hash_time;
        for (manifest_key, entry) in required {
            let data = bytes
                .remove(&manifest_key)
                .expect("fetched required object");
            let key = original_files
                .remove(&manifest_key)
                .expect("original file request");
            let hash_started = Instant::now();
            let expected_digest = FileContentId::for_bytes(&data);
            telemetry.content_hash_time = telemetry
                .content_hash_time
                .saturating_add(hash_started.elapsed());
            responses.push(ResourceResponse::File(ResolvedFile {
                request: key,
                expected_digest: Some(expected_digest),
                virtual_path: entry.virtual_path,
                bytes: data,
            }));
        }
        for (manifest_key, key) in original_hints {
            let Some(data) = bytes.remove(&manifest_key) else {
                continue;
            };
            let entry = hints
                .get(&manifest_key)
                .expect("fetched closure hint has manifest metadata");
            let hash_started = Instant::now();
            let expected_digest = FileContentId::for_bytes(&data);
            telemetry.content_hash_time = telemetry
                .content_hash_time
                .saturating_add(hash_started.elapsed());
            responses.push(ResourceResponse::File(ResolvedFile {
                request: key,
                expected_digest: Some(expected_digest),
                virtual_path: entry.virtual_path.clone(),
                bytes: data,
            }));
        }
        drop(hints);
        telemetry.response_build_time = telemetry.response_build_time.saturating_add(
            response_started
                .elapsed()
                .saturating_sub(telemetry.content_hash_time.saturating_sub(hash_before)),
        );
        Ok(ResolvedDistributionBatch { responses })
    }

    fn resolve_generic_asset(
        &mut self,
        local: &LocalResolver,
        logical_name: &[u8],
        cancellation: &FetchCancellation,
    ) -> Result<Vec<u8>, NativeRunError> {
        self.resolve_generic_file(local, logical_name, cancellation)
            .map(|file| file.bytes)
    }

    fn resolve_generic_file(
        &mut self,
        local: &LocalResolver,
        logical_name: &[u8],
        cancellation: &FetchCancellation,
    ) -> Result<ResolvedFile, NativeRunError> {
        let name = std::str::from_utf8(logical_name).map_err(|_| {
            NativeRunError::Selection("PDF resource name is not valid UTF-8".to_owned())
        })?;
        let key = crate::FileRequestKey::new(FileKind::GenericAsset, name)
            .map_err(|error| NativeRunError::Selection(error.to_string()))?;
        let resolved = self.resolve_batch_with_prefetch(
            local,
            &NeedResources {
                required: vec![ResourceRequest::File(FileRequest::new(key.clone(), name))],
                probes: Vec::new(),
                prefetch_hints: Vec::new(),
            },
            cancellation,
            &mut ResolverTelemetry::default(),
        )?;
        for response in resolved.responses {
            match response {
                ResourceResponse::File(file) if file.request == key => return Ok(file),
                ResourceResponse::FileUnavailable(unavailable) if unavailable == key => {
                    return Err(NativeRunError::DistributionUnavailable(vec![format!(
                        "tex:{name}"
                    )]));
                }
                _ => {}
            }
        }
        Err(NativeRunError::DistributionUnavailable(vec![format!(
            "tex:{name}"
        )]))
    }

    fn resolve_format(
        &mut self,
        path: &Path,
        engine: EngineMode,
        cancellation: &FetchCancellation,
    ) -> Result<ResolvedFormat, NativeRunError> {
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| NativeRunError::Format("format name is not valid UTF-8".into()))?;
        let loaded = self.load(cancellation)?.clone();
        let entry = loaded
            .root
            .formats
            .get(name)
            .ok_or_else(|| NativeRunError::Format(format!("manifest has no format named {name}")))?
            .clone();
        if entry.engine_version != crate::PACKAGE_VERSION {
            return Err(NativeRunError::Format(format!(
                "format {name} requires Umber {}, this runtime is {}",
                entry.engine_version,
                crate::PACKAGE_VERSION
            )));
        }
        if entry.format_schema != Universe::FORMAT_SCHEMA_VERSION {
            return Err(NativeRunError::Format(format!(
                "format {name} uses schema {}; this runtime requires schema {}",
                entry.format_schema,
                Universe::FORMAT_SCHEMA_VERSION
            )));
        }
        if entry.engine != engine.name() && entry.engine != "umber" {
            return Err(NativeRunError::Format(format!(
                "format {name} targets {}, not {}",
                entry.engine,
                engine.name()
            )));
        }
        let prefetch_hints = entry
            .input_closure
            .as_ref()
            .map(|closure| {
                closure
                    .keys
                    .iter()
                    .map(|key| {
                        let key = DistributionFileRequestKey::from_manifest_key(key)
                            .map_err(|error| NativeRunError::Selection(error.to_string()))?;
                        distribution_request(key)
                    })
                    .collect::<Result<Vec<_>, NativeRunError>>()
            })
            .transpose()?
            .unwrap_or_default();
        if let Some(bytes) = self
            .cache
            .load_object(&entry.sha256, entry.bytes)
            .map_err(|error| NativeRunError::Cache(error.to_string()))?
        {
            return Ok(ResolvedFormat {
                bytes,
                prefetch_hints,
            });
        }
        if self.offline {
            return Err(NativeRunError::DistributionUnavailable(vec![format!(
                "format:{name}"
            )]));
        }
        let object = umber_distribution::ObjectEntry {
            object: entry.object,
            sha256: entry.sha256,
            bytes: entry.bytes,
        };
        if let Some(root) = &loaded.local_root {
            let bytes = read(&local_object_path(root, &object.object))?;
            check_cancelled(cancellation)?;
            self.cache
                .store_object(&object.sha256, object.bytes, &bytes)
                .map_err(|error| NativeRunError::Cache(error.to_string()))?;
            eprintln!("umber: acquired 1 distribution resource(s)");
            return Ok(ResolvedFormat {
                bytes,
                prefetch_hints,
            });
        }
        let request = FetchRequest {
            request_key: format!("format:{name}"),
            object,
            max_bytes: crate::SessionLimits::FORMAT_IMAGE_BYTES as u64,
        };
        let object = FetchClient::new(FetchClientConfig::default())
            .fetch_batch_cancellable(
                &self.cache,
                &loaded.root.objects_base_url,
                &[request],
                cancellation,
            )
            .map_err(map_fetch_error)?
            .pop()
            .expect("one format result");
        if !object.cache_hit {
            eprintln!("umber: acquired 1 distribution resource(s)");
        }
        Ok(ResolvedFormat {
            bytes: object.bytes,
            prefetch_hints,
        })
    }

    fn fetch_objects(
        &self,
        objects_base_url: &str,
        requests: &[FetchRequest],
        cancellation: &FetchCancellation,
    ) -> Result<Vec<(String, Vec<u8>, bool)>, NativeRunError> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        if self.offline {
            let mut found = Vec::new();
            let mut missing = Vec::new();
            for request in requests {
                check_cancelled(cancellation)?;
                match self
                    .cache
                    .load_object(&request.object.sha256, request.object.bytes)
                {
                    Ok(Some(bytes)) => found.push((request.request_key.clone(), bytes, true)),
                    Ok(None) => missing.push(request.request_key.clone()),
                    Err(error) => return Err(NativeRunError::Cache(error.to_string())),
                }
            }
            if !missing.is_empty() {
                return Err(NativeRunError::DistributionUnavailable(missing));
            }
            return Ok(found);
        }
        if let Some(local_root) = self
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.local_root.as_ref())
        {
            let mut found = Vec::new();
            for request in requests {
                check_cancelled(cancellation)?;
                let bytes = read(&local_object_path(local_root, &request.object.object))?;
                check_cancelled(cancellation)?;
                self.cache
                    .store_object(&request.object.sha256, request.object.bytes, &bytes)
                    .map_err(|error| NativeRunError::Cache(error.to_string()))?;
                found.push((request.request_key.clone(), bytes, false));
            }
            return Ok(found);
        }
        let client = FetchClient::new(FetchClientConfig::default());
        client
            .fetch_batch_cancellable(&self.cache, objects_base_url, requests, cancellation)
            .map_err(map_fetch_error)
            .map(|objects| {
                objects
                    .into_iter()
                    .map(|object| (object.request_key, object.bytes, object.cache_hit))
                    .collect()
            })
    }

    fn load_shard(
        &mut self,
        index: u32,
        cancellation: &FetchCancellation,
    ) -> Result<&ManifestShard, NativeRunError> {
        check_cancelled(cancellation)?;
        let loaded = self.load(cancellation)?;
        if loaded.shards.contains_key(&index) {
            return Ok(self
                .loaded
                .as_ref()
                .expect("root loaded before shard")
                .shards
                .get(&index)
                .expect("checked shard remains loaded"));
        }
        let shard_bits = loaded.root.shard_bits;
        let local_root = loaded.local_root.clone();
        let digest = loaded
            .root
            .shard_digest(index)
            .expect("canonical shard index is bounded by shardBits")
            .to_owned();
        let bytes = if let Some(bytes) = self
            .cache
            .load_manifest(&digest)
            .map_err(|error| NativeRunError::Cache(error.to_string()))?
        {
            bytes
        } else {
            let bytes = if let Some(local_root) = &local_root {
                let path = local_object_path(local_root, &format!("sha256-{digest}"));
                let bytes = read_bounded(&path, MAX_INDEX_SHARD_BYTES, "distribution index shard")?;
                verify_manifest_digest(&bytes, &digest)?;
                bytes
            } else if self.offline {
                return Err(NativeRunError::DistributionUnavailable(vec![format!(
                    "shard:{index}"
                )]));
            } else {
                let url = format!(
                    "{}sha256-{digest}",
                    self.loaded
                        .as_ref()
                        .expect("root loaded before shard")
                        .root
                        .objects_base_url
                );
                fetch_manifest_cancellable(&url, &digest, Duration::from_secs(30), cancellation)
                    .map_err(|error| match error {
                        ManifestFetchError::Cancelled => NativeRunError::Cancelled,
                        error => NativeRunError::ManifestFetch(error.to_string()),
                    })?
            };
            check_cancelled(cancellation)?;
            self.cache
                .store_manifest(&digest, &bytes)
                .map_err(|error| NativeRunError::Cache(error.to_string()))?;
            bytes
        };
        check_cancelled(cancellation)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
        let shard = ManifestShard::parse(text)
            .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
        shard
            .validate_identity(
                &self.loaded.as_ref().expect("root loaded before shard").root,
                index,
            )
            .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
        for key in shard.files.keys() {
            if shard_index(key, shard_bits) != index {
                return Err(NativeRunError::ManifestParse(format!(
                    "lookup key {key} is not in its canonical shard"
                )));
            }
        }
        self.loaded
            .as_mut()
            .expect("root loaded before shard")
            .shards
            .insert(index, shard.clone());
        Ok(self
            .loaded
            .as_ref()
            .expect("root loaded before shard")
            .shards
            .get(&index)
            .expect("inserted shard remains loaded"))
    }

    fn load(
        &mut self,
        cancellation: &FetchCancellation,
    ) -> Result<&LoadedDistribution, NativeRunError> {
        check_cancelled(cancellation)?;
        if self.loaded.is_none() {
            let source = self
                .source
                .clone()
                .unwrap_or_else(|| DEFAULT_DISTRIBUTION_URL.to_owned());
            let explicit = self.source.is_some();
            let path = PathBuf::from(&source);
            let local_path = if path.is_dir() {
                let schema_three = path.join("manifest-v3.json");
                let schema_two = path.join("manifest-v2.json");
                if schema_three.exists() {
                    schema_three
                } else if schema_two.exists() {
                    schema_two
                } else {
                    path.join("manifest.json")
                }
            } else {
                path.clone()
            };
            let is_local = local_path.exists() || (!source.contains("://") && explicit);
            let expected = self
                .expected
                .clone()
                .or_else(|| (!explicit).then(|| DEFAULT_DISTRIBUTION_SHA256.to_owned()));
            let (manifest_bytes, local_root) = if is_local {
                let bytes = read_bounded(
                    &local_path,
                    MAX_INDEX_SHARD_BYTES,
                    "distribution root manifest",
                )?;
                if let Some(expected) = &expected {
                    verify_manifest_digest(&bytes, expected)?;
                }
                (bytes, local_path.parent().map(Path::to_owned))
            } else {
                let expected = expected
                    .ok_or_else(|| NativeRunError::DistributionPinRequired(source.clone()))?;
                let bytes = if let Some(bytes) = self
                    .cache
                    .load_manifest(&expected)
                    .map_err(|error| NativeRunError::Cache(error.to_string()))?
                {
                    bytes
                } else {
                    if self.offline {
                        return Err(NativeRunError::DistributionUnavailable(vec![
                            "manifest".into(),
                        ]));
                    }
                    let bytes = fetch_manifest_cancellable(
                        &source,
                        &expected,
                        Duration::from_secs(30),
                        cancellation,
                    )
                    .map_err(|error| match error {
                        ManifestFetchError::Cancelled => NativeRunError::Cancelled,
                        error => NativeRunError::ManifestFetch(error.to_string()),
                    })?;
                    check_cancelled(cancellation)?;
                    self.cache
                        .store_manifest(&expected, &bytes)
                        .map_err(|error| NativeRunError::Cache(error.to_string()))?;
                    bytes
                };
                (bytes, None)
            };
            let text = std::str::from_utf8(&manifest_bytes)
                .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
            let root = ShardedManifestRoot::parse(text)
                .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
            self.loaded = Some(LoadedDistribution {
                root,
                local_root,
                shards: BTreeMap::new(),
            });
        }
        Ok(self.loaded.as_ref().expect("distribution loaded"))
    }
}

fn distribution_file_key(
    request: &FileRequest,
) -> Result<Option<DistributionFileRequestKey>, NativeRunError> {
    let kind = match request.key().kind() {
        FileKind::TexInput => DistributionFileKind::Tex,
        FileKind::Tfm => DistributionFileKind::Tfm,
        FileKind::BibAux => DistributionFileKind::BibAux,
        FileKind::ClassicBibData => DistributionFileKind::ClassicBib,
        FileKind::BibStyle => DistributionFileKind::BibStyle,
        FileKind::GenericAsset => DistributionFileKind::Tex,
        FileKind::VirtualFont
        | FileKind::PdfFontMap
        | FileKind::PdfEncoding
        | FileKind::PdfFontProgram => DistributionFileKind::Tex,
        _ => return Ok(None),
    };
    DistributionFileRequestKey::new(kind, request.key().name())
        .map(Some)
        .map_err(|error| NativeRunError::Selection(error.to_string()))
}

fn distribution_request(
    request: DistributionFileRequestKey,
) -> Result<ResourceRequest, NativeRunError> {
    let kind = match request.kind() {
        DistributionFileKind::Tex => FileKind::TexInput,
        DistributionFileKind::Tfm => FileKind::Tfm,
        DistributionFileKind::BibAux => FileKind::BibAux,
        DistributionFileKind::ClassicBib => FileKind::ClassicBibData,
        DistributionFileKind::BibStyle => FileKind::BibStyle,
    };
    let name = request.normalized_name();
    let key = crate::FileRequestKey::new(kind, name)
        .map_err(|error| NativeRunError::Selection(error.to_string()))?;
    Ok(ResourceRequest::File(FileRequest::new(key, name)))
}

fn collect_closure_hints(
    shard: &ManifestShard,
    keys: Vec<String>,
    required: &BTreeMap<String, ShardFile>,
    hints: &mut BTreeMap<String, DependencyHint>,
) {
    for key in keys {
        let Some(entry) = shard.files.get(&key) else {
            continue;
        };
        if !required.contains_key(&key) {
            hints.entry(key.clone()).or_insert_with(|| DependencyHint {
                key: key.clone(),
                virtual_path: entry.virtual_path.clone(),
                object: entry.object.clone(),
                sha256: entry.sha256.clone(),
                bytes: entry.bytes,
            });
        }
        for dependency in &entry.dependencies {
            if !required.contains_key(&dependency.key) {
                hints
                    .entry(dependency.key.clone())
                    .or_insert_with(|| dependency.clone());
            }
        }
    }
}

fn check_cancelled(cancellation: &FetchCancellation) -> Result<(), NativeRunError> {
    if cancellation.is_cancelled() {
        Err(NativeRunError::Cancelled)
    } else {
        Ok(())
    }
}

fn shard_index(key: &str, shard_bits: u8) -> u32 {
    if shard_bits == 0 {
        return 0;
    }
    let digest = Sha256::digest(key.as_bytes());
    let prefix = u16::from_be_bytes([digest[0], digest[1]]);
    u32::from(prefix >> (16 - shard_bits))
}

fn local_object_path(root: &Path, object: &str) -> PathBuf {
    let objects = root.join("objects").join(object);
    if objects.exists() {
        objects
    } else {
        root.join(object)
    }
}

fn read_bounded(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>, NativeRunError> {
    let metadata = fs::metadata(path).map_err(|source| NativeRunError::Io {
        path: path.to_owned(),
        source,
    })?;
    if metadata.len() > limit {
        return Err(NativeRunError::ManifestTooLarge {
            label: label.to_owned(),
            limit,
        });
    }
    let bytes = read(path)?;
    if bytes.len() as u64 > limit {
        return Err(NativeRunError::ManifestTooLarge {
            label: label.to_owned(),
            limit,
        });
    }
    Ok(bytes)
}

fn map_fetch_error(error: umber_fetch::BatchFetchError) -> NativeRunError {
    if error
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.failure == FetchFailure::Cancelled)
    {
        NativeRunError::Cancelled
    } else {
        NativeRunError::Fetch(
            error
                .diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

fn verify_manifest_digest(bytes: &[u8], expected: &str) -> Result<(), NativeRunError> {
    let actual = hex_digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(NativeRunError::ManifestDigestMismatch {
            expected: expected.to_owned(),
            actual,
        })
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[allow(
    clippy::disallowed_methods,
    reason = "this module is the native CLI host I/O boundary"
)]
fn read(path: &Path) -> Result<Vec<u8>, NativeRunError> {
    fs::read(path).map_err(|source| NativeRunError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests;
