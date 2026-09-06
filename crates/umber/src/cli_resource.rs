//! Native host policy for driving one CLI compile through the resource loop.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use tex_fonts::AcceptedFontContainers;
use tex_state::{FORMAT_SCHEMA_VERSION, World};
use umber_distribution::{
    FileKind as DistributionFileKind, FileRequestKey as DistributionFileRequestKey, LookupManifest,
    LookupOutcome, LookupRecord, LookupRole, NegativeScope, ObjectEntry, PrefetchIdentity,
    Readiness, ResolvedIdentity, ShardedManifestRoot, ValidatedPackedShard, shard_index_for_key,
};
use umber_fetch::{
    DistributionClient, DistributionClientError, FetchCancellation, FetchClientConfig,
    FetchFailure, FetchRequest, ManifestFetchError, ObjectCache,
};
use umber_hash::{AHash64, HashDomain};

use crate::input_search::{WorldSearchError, read_first_world_detailed};
use crate::prefetch::{
    PREFETCH_POLICY_VERSION, PrefetchDiagnosticDisposition, PrefetchPlanner, semantic_file_key,
};
use crate::{
    AcceptedFinalization, CompileAttemptResult, CompileError, CompileTelemetry, EngineMode,
    FileContentId, FileKind, FileRequest, FileRequestKey, MemoryRunOutput, NeedResources,
    OutputCapability, OutputCapabilitySet, ResolvedFile, ResolvedPkFont, ResourceRequest,
    ResourceResponse, SessionLimits, SessionOptions, SourcePatch, TexFontSearchPath,
    TexInputSearchPath, VirtualCompileSession,
};

pub const DEFAULT_DISTRIBUTION_URL: &str =
    "https://assets.umber.ink/texlive/texlive-20260301/manifest-v8.json";

const MAX_INDEX_SHARD_BYTES: u64 = 32 * 1024 * 1024;

fn resource_telemetry_enabled() -> bool {
    env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1")
}

#[derive(Clone, Debug)]
pub struct NativeRunOptions {
    pub input: PathBuf,
    pub format: Option<PathBuf>,
    pub initial_prefetch_keys: Vec<String>,
    pub engine: EngineMode,
    pub pdf_output_mode: Option<crate::PdfOutputMode>,
    pub outputs: OutputCapabilitySet,
    pub html_asset_directory: Option<String>,
    pub distribution: Option<String>,
    pub distribution_ahash64: Option<String>,
    pub offline: bool,
    pub expansion_fuel: Option<u64>,
    /// Explicit committed executor-step cap for this run, independent of
    /// expansion fuel.
    pub execution_steps: Option<u64>,
}

/// The two independent command-execution guards selected for one native run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativeEngineGuards {
    expansion_fuel: u64,
    execution_steps: u64,
}

#[derive(Debug)]
pub enum NativeRunError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Publish {
        path: PathBuf,
        source: tex_state::WorldError,
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
    DefaultDistributionUnpublished,
    DistributionUnavailable(Vec<String>),
    DistributionShardUnavailable {
        index: u32,
        digest: String,
        request_keys: Vec<String>,
        omitted_request_keys: usize,
        path: Option<PathBuf>,
    },
    Selection(String),
    Fetch(String),
    Compile(String),
    Diagnostic(Box<crate::CompileDiagnostic>),
    Format(String),
    Cancelled,
}

impl fmt::Display for NativeRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "failed to read {}: {source}", path.display()),
            Self::Publish { path, source } => {
                write!(f, "failed to publish {}: {source}", path.display())
            }
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
                "distribution {source} requires --distribution-ahash64 (or UMBER_DISTRIBUTION_AHASH64)"
            ),
            Self::DefaultDistributionUnpublished => f.write_str(
                "the default deterministic aHash64 distribution has not been published; pass --distribution and --distribution-ahash64 for a migrated local or hosted root",
            ),
            Self::DistributionUnavailable(keys) => write!(
                f,
                "distribution unavailable for required request(s): {}",
                keys.join(", ")
            ),
            Self::DistributionShardUnavailable {
                index,
                digest,
                request_keys,
                omitted_request_keys,
                path,
            } => {
                write!(
                    f,
                    "distribution shard unavailable: index={index} digest={digest} request_keys={}",
                    request_keys.join(", ")
                )?;
                if *omitted_request_keys > 0 {
                    write!(f, " (+{omitted_request_keys} more)")?;
                }
                if let Some(path) = path {
                    write!(f, " path={}", path.display())?;
                }
                Ok(())
            }
            Self::Selection(message) => write!(f, "distribution selection error: {message}"),
            Self::Fetch(message) => f.write_str(message),
            Self::Compile(message) => f.write_str(message),
            Self::Diagnostic(diagnostic) => f.write_str(&diagnostic.message),
            Self::Format(message) => write!(f, "format resource error: {message}"),
            Self::Cancelled => f.write_str("distribution acquisition cancelled"),
        }
    }
}

impl Error for NativeRunError {}

impl NativeRunError {
    #[must_use]
    pub fn diagnostic(&self) -> Option<&crate::CompileDiagnostic> {
        match self {
            Self::Diagnostic(diagnostic) => Some(diagnostic.as_ref()),
            _ => None,
        }
    }
}

pub fn run(options: &NativeRunOptions) -> Result<MemoryRunOutput, NativeRunError> {
    let owner = NativeDistributionOwner::from_environment(options)?;
    let store = tex_incr::new_reachability_store();
    NativeCompileSession::new_with_owners(options, &FetchCancellation::new(), &owner, &store)?
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

const MAX_RESOURCE_RESTART_BATCHES: u64 = 32;
const MAX_RESOURCE_BATCH_KEYS: usize = 64;

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
    /// Verified root/shard snapshots reused from the bounded owner.
    pub verified_manifest_hits: u64,
    /// Root or shard payloads read from local, persistent-cache, or transport bytes.
    pub manifest_reads: u64,
    /// Exact serialized root plus packed-shard bytes presented by those reads.
    pub manifest_read_bytes: u64,
    /// Strict root or shard parser invocations.
    pub manifest_parses: u64,
    /// Root or shard payload digest validations.
    pub manifest_validations: u64,
    /// Complete packed shards validated and retained.
    pub shard_loads: u64,
    /// Packed-shard lookup batches against already validated bytes.
    pub packed_selection_calls: u64,
    /// Canonical request keys probed by packed-shard lookup batches.
    pub packed_selection_keys: u64,
    /// Validated shard bytes presented to packed lookup batches, counted once per call.
    pub packed_selection_bytes: u64,
    /// Complete packed-shard structural validation attempts.
    pub packed_validation_calls: u64,
    /// Exact packed-shard bytes presented to structural validation.
    pub packed_validation_bytes: u64,
    /// Largest verified serialized root or packed shard read in one operation.
    pub manifest_parse_peak_bytes: u64,
    /// Packed shards currently retained by the verified owner.
    pub retained_manifest_shards: u64,
    /// Exact packed shard bytes retained by the verified owner.
    pub retained_manifest_bytes: u64,
    pub object_requests: u64,
    pub object_cache_hits: u64,
    /// Content-addressed object payload validations, excluding response IDs.
    pub object_hashes: u64,
    /// Accepted-run predictor and readiness metrics.  Persistent cache hits
    /// still count as prefetch bytes only when they cross the engine boundary.
    pub startup_prefetch_candidates: u64,
    pub literal_prefetch_hints: u64,
    pub package_group_candidates: u64,
    pub prefetch_bytes: u64,
    pub demand_bytes: u64,
    pub unused_prefetch_bytes: u64,
    pub ready_resources: u64,
    pub exists_not_ready_resources: u64,
    pub absent_resources: u64,
}

impl NativeAcceptedRun {
    #[must_use]
    pub fn pdf_draft_mode(&self) -> bool {
        self.finalization
            .completion
            .pdf()
            .and_then(tex_state::DetachedPdfCompletion::output_parameters)
            .is_some_and(|parameters| parameters.draft_mode > 0)
    }

    /// Publishes the accepted engine-owned PDF classic-font closure as a
    /// deterministic, identity-pinned receipt. Resolved rows are accepted
    /// directly by `scripts/provision.py materialize --keys-from`; unavailable
    /// probes seed canonical shard-absence checks without selecting payloads.
    pub fn write_pdf_font_closure_receipt(&self, path: &Path) -> Result<(), NativeRunError> {
        let bytes = pdf_font_closure_receipt_bytes(&self.finalization.pdf_font_closure_receipt)?;
        World::real()
            .publish_files(vec![(path.to_owned(), bytes)])
            .map_err(|source| NativeRunError::Publish {
                path: path.to_owned(),
                source,
            })
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

fn pdf_font_closure_receipt_bytes(
    receipt: &crate::PdfFontClosureReceipt,
) -> Result<Vec<u8>, NativeRunError> {
    let mut output = b"umber-pdf-font-closure-v1\n".to_vec();
    for entry in &receipt.entries {
        let (semantic_kind, request_name, manifest_key, outcome) = match entry {
            crate::PdfFontClosureReceiptEntry::File { request, outcome } => {
                let logical = distribution_file_key(&FileRequest::new(
                    request.clone(),
                    request.name().to_owned(),
                ))?
                .ok_or_else(|| {
                    NativeRunError::Selection(format!(
                        "PDF font closure request {} has no distribution key",
                        request.name()
                    ))
                })?;
                (
                    request.kind().wire_name(),
                    request.name().to_owned(),
                    logical.manifest_key().to_string(),
                    outcome,
                )
            }
            crate::PdfFontClosureReceiptEntry::PkFont { request, outcome } => {
                let request_name = std::str::from_utf8(&request.logical_name())
                    .map_err(|_| {
                        NativeRunError::Selection(
                            "PDF PK font closure name is not valid UTF-8".to_owned(),
                        )
                    })?
                    .to_owned();
                let logical =
                    DistributionFileRequestKey::new(DistributionFileKind::Tex, &request_name)
                        .map_err(|error| NativeRunError::Selection(error.to_string()))?;
                (
                    "pk-font",
                    request_name,
                    logical.manifest_key().to_string(),
                    outcome,
                )
            }
        };
        for field in [semantic_kind, request_name.as_str(), manifest_key.as_str()] {
            validate_receipt_field(field)?;
        }
        match outcome {
            crate::PdfFontClosureResourceOutcome::Resolved {
                virtual_path,
                bytes,
                ahash64,
            } => {
                validate_receipt_field(virtual_path)?;
                output.extend_from_slice(
                    format!(
                        "resolved\t{semantic_kind}\t{request_name}\t{manifest_key}\t{virtual_path}\t{bytes}\t{}\n",
                        encode_hex(ahash64)
                    )
                    .as_bytes(),
                );
            }
            crate::PdfFontClosureResourceOutcome::Unavailable => {
                output.extend_from_slice(
                    format!("unavailable\t{semantic_kind}\t{request_name}\t{manifest_key}\n")
                        .as_bytes(),
                );
            }
        }
    }
    Ok(output)
}

fn validate_receipt_field(field: &str) -> Result<(), NativeRunError> {
    if field.contains(['\t', '\n', '\r']) {
        Err(NativeRunError::Selection(
            "PDF font closure receipt field contains a TSV delimiter".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
pub fn run_for_finalization(
    options: &NativeRunOptions,
) -> Result<NativeAcceptedRun, NativeRunError> {
    let cancellation = FetchCancellation::new();
    let owner = NativeDistributionOwner::from_environment(options)?;
    let store = tex_incr::new_reachability_store();
    let mut session =
        NativeCompileSession::new_with_owners(options, &cancellation, &owner, &store)?;
    let output = match session.compile_without_publication(&cancellation) {
        Ok(output) => output,
        Err(error) => {
            emit_failed_distribution_telemetry(session.host_telemetry.resolver);
            session.emit_resource_boundary_summary();
            return Err(error);
        }
    };
    session.emit_resource_boundary_summary();
    let accepted_handoff_started = Instant::now();
    let input_path_map = session.local.input_path_map();
    let resolved_inputs = session.local.resolved_inputs();
    let main_input = (options.input.clone(), session.source.len());
    let telemetry = session.session.compile_telemetry();
    let mut host_telemetry = session.host_telemetry;
    let finalization = session
        .session
        .into_accepted_finalization()
        .map_err(|error| NativeRunError::Compile(error.to_string()))?;
    // Publish only after the accepted output has crossed the finalization
    // handoff.  A cancelled or failed discovery therefore remains a hint.
    session.distribution.publish_lookup_manifest()?;
    host_telemetry.accepted_handoff_time = accepted_handoff_started.elapsed();
    Ok(NativeAcceptedRun {
        output,
        finalization,
        input_path_map,
        resolved_inputs,
        main_input,
        telemetry,
        host_telemetry,
    })
}

/// Retained native resource and incremental compile state used by `run` and
/// long-lived watch sessions.
pub struct NativeCompileSession<'owner> {
    session: VirtualCompileSession<'owner>,
    #[cfg(test)]
    guards: NativeEngineGuards,
    distribution: DistributionResolver,
    local: LocalResolver,
    prefetch: PrefetchPlanner,
    source: String,
    pending_source: Option<String>,
    host_telemetry: NativeHostTelemetry,
    resource_restart_batches: u64,
    resource_restart_batch_drops: u64,
}

impl<'owner> NativeCompileSession<'owner> {
    /// Starts a fresh engine session while reusing the owner's verified
    /// immutable distribution root and compact selected-shard evidence.
    pub fn new_with_owners(
        options: &NativeRunOptions,
        cancellation: &FetchCancellation,
        owner: &NativeDistributionOwner,
        reachability_store: &'owner tex_state::ReachabilityStore,
    ) -> Result<Self, NativeRunError> {
        Self::new_with_resolver(
            options,
            cancellation,
            owner.resolver(options)?,
            reachability_store,
        )
    }

    #[cfg(test)]
    fn new_with_distribution_owner(
        options: &NativeRunOptions,
        cancellation: &FetchCancellation,
        owner: &NativeDistributionOwner,
    ) -> Result<NativeCompileSession<'static>, NativeRunError> {
        let store = Box::leak(Box::new(tex_incr::new_reachability_store()));
        NativeCompileSession::new_with_owners(options, cancellation, owner, store)
    }

    #[cfg(test)]
    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn new_with_cache(
        options: &NativeRunOptions,
        cancellation: &FetchCancellation,
        cache: ObjectCache,
    ) -> Result<Self, NativeRunError> {
        let owner = Box::leak(Box::new(NativeDistributionOwner::with_cache(
            options, cache,
        )));
        Self::new_with_distribution_owner(options, cancellation, owner)
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn new_with_resolver(
        options: &NativeRunOptions,
        cancellation: &FetchCancellation,
        mut distribution: DistributionResolver,
        reachability_store: &'owner tex_state::ReachabilityStore,
    ) -> Result<Self, NativeRunError> {
        let setup_started = std::time::Instant::now();
        let source_started = std::time::Instant::now();
        let main = read(&options.input)?;
        let source = match String::from_utf8(main.clone()) {
            Ok(source) => source,
            Err(error) => error.into_bytes().into_iter().map(char::from).collect(),
        };
        let local = LocalResolver::from_environment(&options.input);
        distribution
            .set_project_revision(AHash64::for_bytes(HashDomain::DistributionContent, &main).hex());
        let source_read_ns = source_started.elapsed().as_nanos();
        let mut resolver_telemetry = ResolverTelemetry::default();
        let format_started = std::time::Instant::now();
        let format = match &options.format {
            Some(path) if path.exists() => Some(read(path)?),
            Some(path) => {
                let resolved = distribution.resolve_format(
                    path,
                    options.engine,
                    cancellation,
                    &mut resolver_telemetry,
                )?;
                Some(resolved.bytes)
            }
            None => None,
        };
        let format_read_ns = format_started.elapsed().as_nanos();
        let distribution_root = distribution.prefetch_root_identity()?;
        let prefetch_identity = native_prefetch_identity_with_search_policy(
            options,
            format.as_deref(),
            distribution_root.as_deref(),
            &local.search_policy_identity(),
        );
        let prior_manifest = if distribution_root.is_some() {
            distribution.load_lookup_manifest(&prefetch_identity)?
        } else {
            None
        };
        let resource_telemetry_enabled = resource_telemetry_enabled();
        let mut planner = PrefetchPlanner::with_prior(
            prefetch_identity.clone(),
            umber_distribution::PrefetchBudget::default(),
            prior_manifest,
        );
        if resource_telemetry_enabled {
            planner.enable_diagnostics_with_sink(emit_planner_diagnostic);
        }
        let initial_prefetch_hints = options
            .initial_prefetch_keys
            .iter()
            .map(|key| {
                DistributionFileRequestKey::from_manifest_key(key)
                    .map_err(|error| NativeRunError::Selection(error.to_string()))
                    .and_then(distribution_request)
            })
            .collect::<Result<Vec<_>, _>>()?;
        planner.enqueue_escalation(initial_prefetch_hints.iter().filter_map(
            |request| match request {
                ResourceRequest::File(request) => Some(request.clone()),
                ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
            },
        ));
        let mut initial_prefetch_hints = initial_prefetch_hints;
        initial_prefetch_hints.extend(planner.startup_hints(&source));
        let planner_metrics = planner.metrics();
        resolver_telemetry.startup_prefetch_candidates = planner_metrics.startup_candidates;
        resolver_telemetry.literal_prefetch_hints = planner_metrics.literal_hints;
        if distribution_root.is_some() {
            distribution.set_lookup_manifest(LookupManifest::new(prefetch_identity));
        }
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
        let defaults = SessionLimits::default();
        let engine_fuel = selected_limit(
            options.expansion_fuel,
            "UMBER_ENGINE_FUEL",
            defaults.engine_fuel,
        )?;
        let engine_steps = selected_limit(
            options.execution_steps,
            "UMBER_ENGINE_STEPS",
            defaults.engine_steps,
        )?;
        let guards = NativeEngineGuards {
            expansion_fuel: engine_fuel,
            execution_steps: engine_steps,
        };
        if options.expansion_fuel.is_some()
            || options.execution_steps.is_some()
            || resource_telemetry_enabled
        {
            eprintln!(
                "RUN_GUARDS expansion_fuel_cap={} execution_steps_cap={}",
                guards.expansion_fuel, guards.execution_steps
            );
        }
        let input_frames = selected_limit(None, "UMBER_INPUT_FRAMES", defaults.input_frames)?;
        let journal_bytes = selected_limit(None, "UMBER_JOURNAL_BYTES", defaults.journal_bytes)?;
        let effects = selected_limit(None, "UMBER_EFFECTS", defaults.effects)?;
        let restore_started = std::time::Instant::now();
        let mut session = VirtualCompileSession::new_with_store(
            reachability_store,
            SessionOptions {
                main_path: format!("/job/{name}"),
                job_name: Some(job_name),
                authored_root_name: None,
                format,
                initial_prefetch_hints: (!initial_prefetch_hints.is_empty())
                    .then(|| initial_prefetch_hints.into_boxed_slice()),
                engine: options.engine,
                pdf_output_mode: options.pdf_output_mode,
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
            },
        )
        .map_err(|error| NativeRunError::Compile(error.to_string()))?;
        let format_restore_ns = restore_started.elapsed().as_nanos();
        session
            .add_user_file(name, main.clone())
            .map_err(|error| NativeRunError::Compile(error.to_string()))?;
        if resource_telemetry_enabled {
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
            #[cfg(test)]
            guards,
            distribution,
            local,
            prefetch: planner,
            source,
            pending_source: None,
            host_telemetry: NativeHostTelemetry {
                startup_time,
                resolver: resolver_telemetry,
                ..NativeHostTelemetry::default()
            },
            resource_restart_batches: 0,
            resource_restart_batch_drops: 0,
        })
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    pub fn compile(
        &mut self,
        cancellation: &FetchCancellation,
    ) -> Result<MemoryRunOutput, NativeRunError> {
        self.compile_internal(cancellation, true)
    }

    fn compile_without_publication(
        &mut self,
        cancellation: &FetchCancellation,
    ) -> Result<MemoryRunOutput, NativeRunError> {
        self.compile_internal(cancellation, false)
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn compile_internal(
        &mut self,
        cancellation: &FetchCancellation,
        publish_manifest: bool,
    ) -> Result<MemoryRunOutput, NativeRunError> {
        loop {
            if cancellation.is_cancelled() {
                self.session.discard_suspended_candidate();
                self.distribution.reset_lookup_manifest();
                return Err(NativeRunError::Cancelled);
            }
            let compile_attempt_started = Instant::now();
            let execution_before = self.session.compile_telemetry().execution;
            let attempt = self.session.compile_attempt();
            self.host_telemetry.compile_attempt_time = self
                .host_telemetry
                .compile_attempt_time
                .saturating_add(compile_attempt_started.elapsed());
            match attempt {
                CompileAttemptResult::Complete(output) => {
                    self.account_prefetch_usage();
                    if publish_manifest {
                        self.distribution.publish_lookup_manifest()?;
                    }
                    if let Some(source) = self.pending_source.take() {
                        self.source = source;
                    }
                    return Ok(output);
                }
                CompileAttemptResult::Error(error) => {
                    self.distribution.reset_lookup_manifest();
                    return Err(match error {
                        CompileError::Diagnostic(diagnostic) => {
                            NativeRunError::Diagnostic(Box::new(diagnostic))
                        }
                        error => NativeRunError::Compile(error.to_string()),
                    });
                }
                CompileAttemptResult::NeedResources(mut batch) => {
                    // A source/context reset may have queued fresh literal
                    // seeds after the one-shot engine startup list was used.
                    // Merge them at the same preflight seam so they are
                    // admitted before the next engine retry.
                    batch.prefetch_hints.extend(self.prefetch.drain_followups());
                    self.record_resource_restart_batch(&batch, execution_before);
                    if let Some((region, discarded_work)) = self.session.resource_replay_context() {
                        let required_keys = batch
                            .required
                            .iter()
                            .chain(&batch.probes)
                            .filter_map(|request| match request {
                                ResourceRequest::File(request) => Some(request.key().clone()),
                                ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                            })
                            .collect::<Vec<_>>();
                        for request in &required_keys {
                            if let Some(escalation) =
                                self.prefetch.note_replay(&region, request, discarded_work)
                            {
                                let dependencies = self
                                    .prefetch
                                    .escalation_dependencies(request, escalation.tier);
                                self.prefetch.enqueue_escalation_with_priority(
                                    dependencies,
                                    escalation.discarded_work_delta,
                                );
                            }
                        }
                    }
                    let resolver_started = Instant::now();
                    let mut note_catalog = |keys: &[FileRequestKey]| {
                        self.session.note_resource_exists(keys.iter().cloned());
                    };
                    let resolved = match self.distribution.resolve_batch_with_catalog(
                        &self.local,
                        &batch,
                        cancellation,
                        &mut self.host_telemetry.resolver,
                        &mut self.prefetch,
                        &mut note_catalog,
                    ) {
                        Ok(resolved) => resolved,
                        Err(error) => {
                            self.session.discard_suspended_candidate();
                            self.distribution.reset_lookup_manifest();
                            return Err(error);
                        }
                    };
                    self.host_telemetry.resolver_time = self
                        .host_telemetry
                        .resolver_time
                        .saturating_add(resolver_started.elapsed());
                    if cancellation.is_cancelled() {
                        self.session.discard_suspended_candidate();
                        self.distribution.reset_lookup_manifest();
                        return Err(NativeRunError::Cancelled);
                    }
                    let generated_transaction = match self.session.generated_transaction_identity()
                    {
                        Ok(identity) => identity,
                        Err(error) => {
                            self.session.discard_suspended_candidate();
                            self.distribution.reset_lookup_manifest();
                            return Err(NativeRunError::Compile(error.to_string()));
                        }
                    };
                    self.distribution
                        .record_generated_misses(&batch, &generated_transaction);
                    // Prefetch hints are admitted through the same typed VFS
                    // transaction as demanded resources.  A host cache hit is
                    // not engine readiness until this boundary has accepted
                    // the payload and its request metadata.
                    let provision_started = Instant::now();
                    self.session
                        .note_resource_exists(resolved.catalog_exists.clone());
                    self.session
                        .authorize_prefetch_files(resolved.prefetch_requests.clone());
                    if let Err(error) = self.session.provide_resources(resolved.responses.clone()) {
                        self.session.discard_suspended_candidate();
                        self.distribution.reset_lookup_manifest();
                        return Err(NativeRunError::Compile(error.to_string()));
                    }
                    self.distribution
                        .note_engine_admitted(&mut self.host_telemetry.resolver, &resolved);
                    for (request, file) in &resolved.admitted_files {
                        let dependencies =
                            self.distribution.dependencies_for([request.key().clone()]);
                        self.prefetch.admit_file_with_metadata(
                            request,
                            &file.virtual_path,
                            file.bytes.as_ref(),
                            dependencies,
                        );
                    }
                    self.drain_prefetch_closure(cancellation)?;
                    self.host_telemetry.provision_time = self
                        .host_telemetry
                        .provision_time
                        .saturating_add(provision_started.elapsed());
                }
            }
        }
    }

    fn drain_prefetch_closure(
        &mut self,
        cancellation: &FetchCancellation,
    ) -> Result<(), NativeRunError> {
        loop {
            check_cancelled(cancellation)?;
            let hints = self.prefetch.drain_followups();
            if hints.is_empty() {
                return Ok(());
            }
            let mut note_catalog = |keys: &[FileRequestKey]| {
                self.session.note_resource_exists(keys.iter().cloned());
            };
            let resolved = self.distribution.resolve_batch_with_catalog(
                &self.local,
                &NeedResources {
                    required: Vec::new(),
                    probes: Vec::new(),
                    prefetch_hints: hints,
                },
                cancellation,
                &mut self.host_telemetry.resolver,
                &mut self.prefetch,
                &mut note_catalog,
            )?;
            self.session
                .note_resource_exists(resolved.catalog_exists.clone());
            self.session
                .authorize_prefetch_files(resolved.prefetch_requests.clone());
            // Even an empty speculative response is acknowledged once.  This
            // is what lets a false-positive hint terminate without becoming a
            // startup retry loop; it never claims that the payload is ready.
            self.session
                .provide_resources(resolved.responses.clone())
                .map_err(|error| NativeRunError::Compile(error.to_string()))?;
            self.distribution
                .note_engine_admitted(&mut self.host_telemetry.resolver, &resolved);
            for (request, file) in &resolved.admitted_files {
                let dependencies = self.distribution.dependencies_for([request.key().clone()]);
                self.prefetch.admit_file_with_metadata(
                    request,
                    &file.virtual_path,
                    file.bytes.as_ref(),
                    dependencies,
                );
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
        self.distribution.reset_lookup_manifest();
        self.distribution.set_project_revision(
            AHash64::for_bytes(HashDomain::DistributionContent, next.as_bytes()).hex(),
        );
        self.prefetch.reset_for_context(next);
        self.pending_source = Some(next.to_owned());
        Ok(())
    }

    pub fn cancel_pending_revision(&mut self) -> bool {
        let cancelled = self.session.cancel_pending_patch();
        if cancelled {
            self.pending_source = None;
            self.distribution.reset_lookup_manifest();
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
    pub const fn host_telemetry(&self) -> NativeHostTelemetry {
        self.host_telemetry
    }

    #[cfg(test)]
    #[must_use]
    const fn engine_guards(&self) -> NativeEngineGuards {
        self.guards
    }

    #[must_use]
    pub fn revision(&self) -> Option<tex_incr::RevisionId> {
        self.session.revision()
    }

    fn account_prefetch_usage(&mut self) {
        let paths = self
            .session
            .accepted_input_dependencies()
            .map(|dependency| dependency.path().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        self.distribution.account_prefetch_paths(paths);
        self.host_telemetry.resolver.unused_prefetch_bytes =
            self.distribution.unused_prefetch_bytes();
    }

    fn record_resource_restart_batch(
        &mut self,
        batch: &NeedResources,
        execution_before: tex_exec::ExecutionTelemetry,
    ) {
        if !resource_telemetry_enabled() {
            return;
        }
        self.resource_restart_batches = self.resource_restart_batches.saturating_add(1);
        let index = self.resource_restart_batches;
        if index > MAX_RESOURCE_RESTART_BATCHES {
            self.resource_restart_batch_drops = self.resource_restart_batch_drops.saturating_add(1);
            return;
        }
        let execution_after = self.session.compile_telemetry().execution;
        let engine_telemetry_available = execution_after.cold_starts > 0;
        let (required, required_omitted) = diagnostic_request_keys(&batch.required);
        let (probes, probe_omitted) = diagnostic_request_keys(&batch.probes);
        let (prefetch_hints, hint_omitted) = diagnostic_request_keys(&batch.prefetch_hints);
        let checkpoint = self
            .session
            .resource_replay_context()
            .map(|(region, _)| region);
        let attempt_fuel = engine_telemetry_available.then(|| {
            execution_after
                .cumulative_fuel
                .saturating_sub(execution_before.cumulative_fuel)
        });
        let cumulative_fuel = engine_telemetry_available.then_some(execution_after.cumulative_fuel);
        let cumulative_discarded_fuel =
            engine_telemetry_available.then_some(execution_after.discarded_fuel);
        eprintln!(
            "RESOURCE_RESTART_BATCH index={} checkpoint={} resource_restarts={} attempt_fuel={} cumulative_fuel={} cumulative_discarded_fuel={} omitted_keys={} required={} probes={} prefetch_hints={}",
            index,
            checkpoint
                .as_deref()
                .map_or_else(|| "none".to_owned(), escape_telemetry_field),
            execution_after.resource_restarts,
            optional_u64(attempt_fuel),
            optional_u64(cumulative_fuel),
            optional_u64(cumulative_discarded_fuel),
            required_omitted
                .saturating_add(probe_omitted)
                .saturating_add(hint_omitted),
            required.join(","),
            probes.join(","),
            prefetch_hints.join(","),
        );
    }

    fn emit_resource_boundary_summary(&self) {
        if !resource_telemetry_enabled() {
            return;
        }
        let (planner_decisions, dropped_planner_decisions) =
            self.prefetch.diagnostic_counts().unwrap_or_default();
        eprintln!(
            "RESOURCE_BOUNDARY_SUMMARY observed_restart_batches={} retained_restart_batches={} dropped_restart_batches={} planner_decisions={} dropped_planner_decisions={}",
            self.resource_restart_batches,
            self.resource_restart_batches
                .min(MAX_RESOURCE_RESTART_BATCHES),
            self.resource_restart_batch_drops,
            planner_decisions,
            dropped_planner_decisions,
        );
    }
}

fn selected_limit(
    explicit: Option<u64>,
    environment_name: &'static str,
    default: u64,
) -> Result<u64, NativeRunError> {
    let environment = env::var(environment_name).ok();
    selected_limit_value(explicit, environment.as_deref(), environment_name, default)
}

#[cfg(test)]
fn native_prefetch_identity(
    options: &NativeRunOptions,
    format_bytes: Option<&[u8]>,
    distribution_root: Option<&str>,
) -> PrefetchIdentity {
    let local = LocalResolver::from_environment(&options.input);
    native_prefetch_identity_with_search_policy(
        options,
        format_bytes,
        distribution_root,
        &local.search_policy_identity(),
    )
}

fn native_prefetch_identity_with_search_policy(
    options: &NativeRunOptions,
    format_bytes: Option<&[u8]>,
    distribution_root: Option<&str>,
    search_policy: &str,
) -> PrefetchIdentity {
    let format = format_bytes.map_or_else(
        || "none".to_owned(),
        |bytes| {
            format!(
                "content:{}:schema={FORMAT_SCHEMA_VERSION}",
                AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex()
            )
        },
    );
    let distribution =
        distribution_root.map_or_else(|| "unavailable".to_owned(), |root| format!("root:{root}"));
    PrefetchIdentity::new(
        options.engine.name(),
        format,
        format!(
            "schema={FORMAT_SCHEMA_VERSION};pdf={:?};outputs={:?};offline={};engine={}",
            options.pdf_output_mode,
            options.outputs,
            options.offline,
            options.engine.name(),
        ),
        distribution,
        format!("{PREFETCH_POLICY_VERSION};{search_policy}"),
    )
    .expect("native prefetch identity fields are bounded by CLI options")
}

impl LocalResolver {
    /// Fingerprints the concrete ordered host search inputs used by this
    /// resolver.  A predictor may be reused across source edits, but not when
    /// the principal input area or any configured search area changes.
    fn search_policy_identity(&self) -> String {
        format!(
            "providers=project/generated/local/distribution;precedence=v1;base={};roots={};TEXINPUTS={};TEXFONTS={};BIBINPUTS={};BSTINPUTS={}",
            native_path_identity(&self.base),
            native_paths_identity(&self.roots),
            native_paths_identity(&self.input_areas),
            native_paths_identity(&self.font_areas),
            native_paths_identity(&self.bib_areas),
            native_paths_identity(&self.bst_areas),
        )
    }
}

fn native_path_identity(path: &Path) -> String {
    AHash64::for_bytes(
        HashDomain::DistributionTree,
        path.to_string_lossy().as_bytes(),
    )
    .hex()
}

fn native_paths_identity(paths: &[PathBuf]) -> String {
    let mut encoded = Vec::new();
    for path in paths {
        encoded.extend_from_slice(path.to_string_lossy().as_bytes());
        encoded.push(0);
    }
    AHash64::for_bytes(HashDomain::DistributionTree, &encoded).hex()
}

fn selected_limit_value(
    explicit: Option<u64>,
    environment: Option<&str>,
    environment_name: &'static str,
    default: u64,
) -> Result<u64, NativeRunError> {
    if let Some(explicit) = explicit {
        return Ok(explicit);
    }
    environment.map_or(Ok(default), |value| {
        value.parse::<u64>().map_err(|_| {
            NativeRunError::Selection(format!(
                "{environment_name} must be an unsigned integer: {value}"
            ))
        })
    })
}

fn diagnostic_request_keys(requests: &[ResourceRequest]) -> (Vec<String>, u64) {
    let mut keys = Vec::with_capacity(requests.len().min(MAX_RESOURCE_BATCH_KEYS));
    let mut omitted = 0_u64;
    for request in requests {
        if keys.len() < MAX_RESOURCE_BATCH_KEYS {
            keys.push(diagnostic_request_key(request));
        } else {
            omitted = omitted.saturating_add(1);
        }
    }
    (keys, omitted)
}

fn diagnostic_request_key(request: &ResourceRequest) -> String {
    match request {
        ResourceRequest::File(request) => format!(
            "file:{}:{}:{}",
            request.key().domain().wire_name(),
            request.key().kind().wire_name(),
            escape_telemetry_field(request.key().name()),
        ),
        ResourceRequest::Font(request) => {
            format!(
                "font:{}",
                escape_telemetry_field(&format!("{:?}", request.key))
            )
        }
        ResourceRequest::PkFont(request) => format!(
            "pk-font:{}",
            escape_telemetry_field(&format!("{:?}", request))
        ),
    }
}

fn escape_telemetry_field(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push_str(&format!("{byte:02x}"));
        }
    }
    output
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
    input_areas: Vec<PathBuf>,
    font_areas: Vec<PathBuf>,
    bib_areas: Vec<PathBuf>,
    bst_areas: Vec<PathBuf>,
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
        let bib_areas = areas("BIBINPUTS");
        let bst_areas = areas("BSTINPUTS");
        let mut roots = vec![base.clone()];
        roots.extend(input_areas.iter().cloned());
        roots.extend(font_areas.iter().cloned());
        Self {
            base: base.clone(),
            roots,
            input_areas: input_areas.clone(),
            font_areas: font_areas.clone(),
            bib_areas,
            bst_areas,
            input: TexInputSearchPath::new(&base, input_areas),
            font: TexFontSearchPath::new(base, font_areas),
            input_paths: RefCell::new(BTreeMap::new()),
            resolved_inputs: RefCell::new(Vec::new()),
        }
    }

    fn resolve(&self, request: &FileRequest) -> Result<Option<ResolvedFile>, NativeRunError> {
        self.resolve_with_name(request, request.original_name())
    }

    /// Resolves a speculative candidate by its canonical typed name while
    /// retaining the request's original spelling for the eventual engine
    /// lookup and input-path accounting. Literal class/package hints add
    /// their kind-specific suffix to the typed key, but TeX's generic input
    /// search path cannot infer that suffix from the original spelling.
    fn resolve_prefetch(
        &self,
        request: &FileRequest,
    ) -> Result<Option<ResolvedFile>, NativeRunError> {
        let lookup_name = match request.key().kind() {
            FileKind::TexInput | FileKind::Image => request.key().name(),
            _ => request.original_name(),
        };
        self.resolve_with_name(request, lookup_name)
    }

    fn resolve_with_name(
        &self,
        request: &FileRequest,
        lookup_name: &str,
    ) -> Result<Option<ResolvedFile>, NativeRunError> {
        if matches!(
            request.key().kind(),
            FileKind::BibAux | FileKind::ClassicBibData | FileKind::BibStyle
        ) {
            return self.resolve_classic_bibliography(request);
        }
        let mut world = World::real();
        let read = match request.key().kind() {
            FileKind::TexInput | FileKind::Image => {
                self.input.read_from_world_detailed(&mut world, lookup_name)
            }
            FileKind::Tfm => self
                .font
                .read_from_world_detailed(&mut world, Path::new(lookup_name)),
            FileKind::GenericAsset
            | FileKind::VirtualFont
            | FileKind::PdfFontMap
            | FileKind::PdfEncoding
            | FileKind::PdfFontProgram => self
                .font
                .read_program_from_world_detailed(&mut world, Path::new(lookup_name)),
            _ => return Ok(None),
        };
        let content = match read {
            Ok(content) => content,
            Err(error) if error.is_authoritative_not_found() => return Ok(None),
            Err(error) => return Err(local_world_error(error)),
        };
        let bytes = content.shared_bytes();
        self.resolved_inputs
            .borrow_mut()
            .push((content.path().to_owned(), bytes.len()));
        let digest = FileContentId::for_bytes(&bytes);
        let virtual_path = self.virtual_path(request.key().kind(), content.path(), digest);
        let resolved_path = content.path().to_owned();
        let mut input_paths = self.input_paths.borrow_mut();
        input_paths.insert(virtual_path.clone(), resolved_path.clone());
        input_paths.insert(
            PathBuf::from(request.original_name()),
            resolved_path.clone(),
        );
        input_paths.insert(PathBuf::from(request.key().name()), resolved_path);
        Ok(Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: virtual_path.to_string_lossy().into_owned(),
            expected_digest: Some(digest),
            bytes,
        }))
    }

    fn resolve_font(
        &self,
        request: &tex_fonts::FontRequest,
    ) -> Result<Option<tex_fonts::ResolvedFont>, NativeRunError> {
        let _ = request;
        Ok(None)
    }

    fn resolve_pk_font(
        &self,
        request: &tex_fonts::PdfPkFontRequest,
    ) -> Result<Option<ResolvedPkFont>, NativeRunError> {
        let Ok(name) = String::from_utf8(request.logical_name()) else {
            return Ok(None);
        };
        let mut world = World::real();
        let content = match self
            .font
            .read_program_from_world_detailed(&mut world, Path::new(&name))
        {
            Ok(content) => content,
            Err(error) if error.is_authoritative_not_found() => return Ok(None),
            Err(error) => return Err(local_world_error(error)),
        };
        let bytes = content.bytes().to_vec();
        self.resolved_inputs
            .borrow_mut()
            .push((content.path().to_owned(), bytes.len()));
        let digest =
            umber_hash::AHash64::for_bytes(umber_hash::HashDomain::PkProgram, &bytes).to_le_bytes();
        let virtual_path = self.virtual_path(
            FileKind::GenericAsset,
            content.path(),
            FileContentId::for_bytes(&bytes),
        );
        Ok(Some(ResolvedPkFont {
            request: request.clone(),
            virtual_path: virtual_path.to_string_lossy().into_owned(),
            bytes,
            expected_ahash64: Some(digest),
        }))
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

    fn resolve_classic_bibliography(
        &self,
        request: &FileRequest,
    ) -> Result<Option<ResolvedFile>, NativeRunError> {
        let (areas, extension) = match request.key().kind() {
            FileKind::BibAux => (&self.input_areas, ".aux"),
            FileKind::ClassicBibData => (&self.bib_areas, ".bib"),
            FileKind::BibStyle => (&self.bst_areas, ".bst"),
            _ => return Ok(None),
        };
        let mut world = World::real();
        let content = match read_classic_bib_resource(
            &mut world,
            &self.base,
            areas,
            request.original_name(),
            extension,
        ) {
            Ok(content) => content,
            Err(error) if error.is_authoritative_not_found() => return Ok(None),
            Err(error) => return Err(local_world_error(error)),
        };
        let path = content.path().to_owned();
        let bytes = content.shared_bytes();
        self.resolved_inputs
            .borrow_mut()
            .push((path.clone(), bytes.len()));
        let digest = FileContentId::for_bytes(&bytes);
        let virtual_path = self.virtual_path(request.key().kind(), &path, digest);
        self.input_paths
            .borrow_mut()
            .insert(virtual_path.clone(), path);
        Ok(Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: virtual_path.to_string_lossy().into_owned(),
            expected_digest: Some(digest),
            bytes,
        }))
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
    areas: &[PathBuf],
    original: &str,
    extension: &str,
) -> Result<tex_state::FileContent, WorldSearchError> {
    let name = Path::new(original);
    let mut candidates = Vec::new();
    let mut failures = Vec::new();
    if name.is_absolute() {
        candidates.push(name.to_owned());
    } else {
        candidates.push(base.join(name));
        candidates.extend(areas.iter().map(|area| area.join(name)));
    }
    for mut candidate in candidates {
        if candidate.extension().is_none() {
            candidate.set_extension(extension.trim_start_matches('.'));
        }
        // Keep the classic bibliography extension and area ordering in one
        // host search, while retaining typed I/O failures for LocalResolver.
        match read_first_world_detailed(world, vec![candidate]) {
            Ok(content) => return Ok(content),
            Err(error) => failures.extend(error.failures),
        }
    }
    Err(WorldSearchError { failures })
}

fn local_world_error(error: WorldSearchError) -> NativeRunError {
    let (path, kind, source) = error
        .first_non_not_found()
        .expect("a non-not-found search error has one host failure");
    NativeRunError::Io {
        path: path.to_owned(),
        source: std::io::Error::new(kind, source.to_string()),
    }
}

#[derive(Clone)]
struct LoadedDistribution {
    root: Arc<ShardedManifestRoot>,
    local_root: Option<PathBuf>,
    shards: BTreeMap<u32, Arc<ValidatedPackedShard>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SelectedDistributionRecord {
    virtual_path: String,
    object: ObjectEntry,
    dependencies: Vec<SelectedDistributionDependency>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SelectedDistributionDependency {
    key: String,
    virtual_path: String,
    object: ObjectEntry,
}

impl LoadedDistribution {
    fn record_retention(&self, telemetry: &mut ResolverTelemetry) {
        telemetry.retained_manifest_shards = self.shards.len() as u64;
        telemetry.retained_manifest_bytes = self
            .shards
            .values()
            .map(|shard| shard.bytes().len() as u64)
            .sum();
    }
}

#[derive(Default)]
struct VerifiedDistributionState {
    loaded: Option<LoadedDistribution>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DistributionOwnerIdentity {
    source: Option<String>,
    expected: Option<String>,
    offline: bool,
}

impl DistributionOwnerIdentity {
    fn from_options(options: &NativeRunOptions) -> Self {
        Self {
            source: options.distribution.clone(),
            expected: options.distribution_ahash64.clone(),
            offline: options.offline,
        }
    }
}

/// Explicitly bounded owner for one immutable distribution identity.
///
/// Cloned compile sessions share the verified root plus every touched validated
/// packed shard. Lookups probe those immutable bytes directly. Object bytes still pass through
/// the content-addressed store on every session, so its ordinary corruption
/// detection and offline/source-selection behavior remain in force. Dropping this
/// owner drops the reusable manifest state.
pub struct NativeDistributionOwner {
    cache: ObjectCache,
    identity: DistributionOwnerIdentity,
    verified: Arc<Mutex<VerifiedDistributionState>>,
}

impl NativeDistributionOwner {
    pub fn from_environment(options: &NativeRunOptions) -> Result<Self, NativeRunError> {
        let cache = ObjectCache::from_environment()
            .map_err(|error| NativeRunError::Cache(error.to_string()))?;
        Ok(Self::with_cache(options, cache))
    }

    #[must_use]
    pub fn with_cache(options: &NativeRunOptions, cache: ObjectCache) -> Self {
        Self {
            cache,
            identity: DistributionOwnerIdentity::from_options(options),
            verified: Arc::new(Mutex::new(VerifiedDistributionState::default())),
        }
    }

    fn resolver(&self, options: &NativeRunOptions) -> Result<DistributionResolver, NativeRunError> {
        let identity = DistributionOwnerIdentity::from_options(options);
        if identity != self.identity {
            return Err(NativeRunError::Selection(
                "distribution owner identity does not match the compile options".to_owned(),
            ));
        }
        Ok(DistributionResolver::with_verified_state(
            self.cache.clone(),
            identity.source,
            identity.expected,
            identity.offline,
            Arc::clone(&self.verified),
        ))
    }
}

struct ResolvedFormat {
    bytes: Vec<u8>,
}

struct ResolvedDistributionBatch {
    responses: Vec<ResourceResponse>,
    /// File payloads paired with the original request spelling.  The native
    /// host calls the planner with these only after `provide_resources` has
    /// committed the bytes to the engine VFS.
    admitted_files: Vec<(FileRequest, ResolvedFile)>,
    /// File requests that were discovered inside an authenticated package
    /// closure rather than in the engine's original hint batch.
    prefetch_requests: Vec<FileRequest>,
    /// Catalog-positive requests whose payloads are about to be admitted.
    catalog_exists: Vec<FileRequestKey>,
}

struct DistributionResolver {
    client: DistributionClient,
    source: Option<String>,
    expected: Option<String>,
    offline: bool,
    verified: Arc<Mutex<VerifiedDistributionState>>,
    lookup_manifest: Option<LookupManifest>,
    project_revision: Option<String>,
    prefetch_admitted: BTreeMap<FileRequestKey, (u64, String)>,
    prefetch_counted_paths: BTreeSet<String>,
    prefetch_used: BTreeSet<FileRequestKey>,
    readiness: BTreeMap<FileRequestKey, Readiness>,
    known_dependencies: BTreeMap<FileRequestKey, Vec<FileRequest>>,
}

fn admitted_files_for(
    responses: &[ResourceResponse],
    requests: impl IntoIterator<Item = FileRequest>,
) -> Vec<(FileRequest, ResolvedFile)> {
    let mut by_key = BTreeMap::new();
    for request in requests {
        by_key.entry(request.key().clone()).or_insert(request);
    }
    responses
        .iter()
        .filter_map(|response| {
            let ResourceResponse::File(file) = response else {
                return None;
            };
            by_key
                .get(&file.request)
                .cloned()
                .map(|request| (request, file.clone()))
        })
        .collect()
}

impl DistributionResolver {
    #[cfg(test)]
    fn new(
        cache: ObjectCache,
        source: Option<String>,
        expected: Option<String>,
        offline: bool,
    ) -> Self {
        Self::with_verified_state(
            cache,
            source,
            expected,
            offline,
            Arc::new(Mutex::new(VerifiedDistributionState::default())),
        )
    }

    fn with_verified_state(
        cache: ObjectCache,
        source: Option<String>,
        expected: Option<String>,
        offline: bool,
        verified: Arc<Mutex<VerifiedDistributionState>>,
    ) -> Self {
        Self {
            client: DistributionClient::new(cache, FetchClientConfig::default()),
            source,
            expected,
            offline,
            verified,
            lookup_manifest: None,
            project_revision: None,
            prefetch_admitted: BTreeMap::new(),
            prefetch_counted_paths: BTreeSet::new(),
            prefetch_used: BTreeSet::new(),
            readiness: BTreeMap::new(),
            known_dependencies: BTreeMap::new(),
        }
    }

    fn set_lookup_manifest(&mut self, manifest: LookupManifest) {
        self.lookup_manifest = Some(manifest);
    }

    fn reset_lookup_manifest(&mut self) {
        self.prefetch_admitted.clear();
        self.prefetch_counted_paths.clear();
        self.prefetch_used.clear();
        self.known_dependencies.clear();
        let Some(current) = &self.lookup_manifest else {
            return;
        };
        self.lookup_manifest = Some(LookupManifest::new(current.identity().clone()));
    }

    fn set_project_revision(&mut self, revision: String) {
        self.project_revision = Some(revision);
    }

    fn note_prefetch_admitted(&mut self, request: &FileRequest, file: &ResolvedFile) {
        if self.prefetch_admitted.contains_key(request.key()) {
            return;
        }
        self.prefetch_admitted.insert(
            request.key().clone(),
            (file.bytes.len() as u64, file.virtual_path.clone()),
        );
    }

    fn admit_local_prefetch(
        &mut self,
        telemetry: &mut ResolverTelemetry,
        request: FileRequest,
        file: ResolvedFile,
        responses: &mut Vec<ResourceResponse>,
        prefetch_requests: &mut Vec<FileRequest>,
    ) {
        self.record_request_readiness(telemetry, request.key(), Readiness::ExistsNotReady);
        self.record_file_resolved(&request, LookupRole::Hint, "local", &file, None);
        self.note_prefetch_admitted(&request, &file);
        prefetch_requests.push(request);
        responses.push(ResourceResponse::File(file));
    }

    fn note_engine_admitted(
        &mut self,
        telemetry: &mut ResolverTelemetry,
        batch: &ResolvedDistributionBatch,
    ) {
        for (request, file) in &batch.admitted_files {
            self.record_request_readiness(telemetry, request.key(), Readiness::Ready);
            if let Some((bytes, virtual_path)) = self.prefetch_admitted.get(request.key())
                && self.prefetch_counted_paths.insert(virtual_path.clone())
            {
                debug_assert_eq!(*bytes, file.bytes.len() as u64);
                telemetry.prefetch_bytes = telemetry.prefetch_bytes.saturating_add(*bytes);
            }
        }
    }

    fn dependencies_for(
        &self,
        requests: impl IntoIterator<Item = FileRequestKey>,
    ) -> Vec<FileRequest> {
        self.dependencies_for_at_tier(requests, 1)
    }

    fn dependencies_for_at_tier(
        &self,
        requests: impl IntoIterator<Item = FileRequestKey>,
        tier: u8,
    ) -> Vec<FileRequest> {
        let mut output = Vec::new();
        let mut frontier = requests.into_iter().collect::<Vec<_>>();
        let mut seen = frontier.iter().cloned().collect::<BTreeSet<_>>();
        let max_depth = usize::from(tier.max(1)).min(3);
        for _ in 0..max_depth {
            let mut next = Vec::new();
            for request in frontier.drain(..) {
                for dependency in self.known_dependencies.get(&request).into_iter().flatten() {
                    if seen.insert(dependency.key().clone()) {
                        next.push(dependency.key().clone());
                        output.push(dependency.clone());
                    }
                }
            }
            frontier = next;
        }
        output
    }

    fn unused_prefetch_bytes(&self) -> u64 {
        let mut paths = BTreeSet::new();
        self.prefetch_admitted
            .iter()
            .filter_map(|(key, (bytes, path))| {
                (!self.prefetch_used.contains(key) && paths.insert(path.clone())).then_some(*bytes)
            })
            .sum()
    }

    fn account_prefetch_paths(&mut self, paths: impl IntoIterator<Item = String>) {
        for path in paths {
            for (key, (_, virtual_path)) in &self.prefetch_admitted {
                if virtual_path == &path {
                    self.prefetch_used.insert(key.clone());
                }
            }
        }
    }

    /// Returns the authenticated identity of the root used for predictive
    /// history.  An unpinned remote cannot safely namespace a persisted
    /// history, so it deliberately returns `None`.
    fn prefetch_root_identity(&self) -> Result<Option<String>, NativeRunError> {
        if let Some(expected) = &self.expected {
            return Ok(Some(expected.clone()));
        }
        let Some(source) = &self.source else {
            return Ok(None);
        };
        if source.contains("://") {
            return Ok(None);
        }
        let path = PathBuf::from(source);
        let manifest = if path.is_dir() {
            let schema_nine = path.join("manifest-v9.json");
            let schema_eight = path.join("manifest-v8.json");
            if schema_nine.exists() {
                schema_nine
            } else if schema_eight.exists() {
                schema_eight
            } else {
                path.join("manifest.json")
            }
        } else {
            path
        };
        if !manifest.exists() {
            return Ok(None);
        }
        let bytes = read_bounded(
            &manifest,
            MAX_INDEX_SHARD_BYTES,
            "distribution root manifest",
        )?;
        Ok(Some(
            AHash64::for_bytes(HashDomain::DistributionContent, &bytes).hex(),
        ))
    }

    fn load_lookup_manifest(
        &self,
        identity: &PrefetchIdentity,
    ) -> Result<Option<LookupManifest>, NativeRunError> {
        let Some(bytes) = self
            .client
            .store()
            .load_named("prefetch", &identity.canonical_key(), 4 * 1024 * 1024)
            .map_err(|error| NativeRunError::Cache(error.to_string()))?
        else {
            return Ok(None);
        };
        Ok(LookupManifest::decode(&bytes)
            .ok()
            .filter(|manifest| manifest.identity() == identity))
    }

    fn publish_lookup_manifest(&self) -> Result<(), NativeRunError> {
        let Some(manifest) = &self.lookup_manifest else {
            return Ok(());
        };
        self.client
            .store()
            .store_named(
                "prefetch",
                &manifest.identity().canonical_key(),
                4 * 1024 * 1024,
                &manifest.encode(),
            )
            .map_err(|error| NativeRunError::Cache(error.to_string()))
    }

    fn record_file_resolved(
        &mut self,
        request: &FileRequest,
        role: LookupRole,
        search_context: &str,
        file: &ResolvedFile,
        object: Option<&ObjectEntry>,
    ) {
        let Some(manifest_key) = distribution_file_key(request)
            .ok()
            .flatten()
            .map(|key| key.manifest_key().to_string())
        else {
            return;
        };
        let digest = AHash64::for_bytes(HashDomain::DistributionContent, &file.bytes).hex();
        let object_name = object.map_or_else(|| "local".to_owned(), |entry| entry.object.clone());
        let Ok(identity) = ResolvedIdentity::new(
            manifest_key.clone(),
            Some(file.virtual_path.clone()),
            object_name,
            digest,
            file.bytes.len() as u64,
        ) else {
            return;
        };
        if let Some(manifest) = &mut self.lookup_manifest
            && let Ok(record) = LookupRecord::new(
                request.original_name(),
                manifest_key,
                request.key().kind().wire_name(),
                search_context,
                role,
                LookupOutcome::Resolved(identity),
            )
        {
            let _ = manifest.record(record);
        }
    }

    fn project_negative_scope(&self) -> NegativeScope {
        NegativeScope::Project {
            revision: self
                .project_revision
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
        }
    }

    fn distribution_negative_scope(&self) -> NegativeScope {
        let root = self
            .expected
            .clone()
            .or_else(|| {
                self.lookup_manifest.as_ref().map(|manifest| {
                    manifest
                        .identity()
                        .distribution
                        .strip_prefix("root:")
                        .unwrap_or(&manifest.identity().distribution)
                        .to_owned()
                })
            })
            .unwrap_or_else(|| "unknown".to_owned());
        NegativeScope::Distribution { root }
    }

    fn record_file_absent(
        &mut self,
        request: &FileRequest,
        role: LookupRole,
        search_context: &str,
        scope: NegativeScope,
    ) {
        let Some(manifest_key) = distribution_file_key(request)
            .ok()
            .flatten()
            .map(|key| key.manifest_key().to_string())
        else {
            return;
        };
        if let Some(manifest) = &mut self.lookup_manifest
            && let Ok(record) = LookupRecord::new(
                request.original_name(),
                manifest_key,
                request.key().kind().wire_name(),
                search_context,
                role,
                LookupOutcome::Absent(scope),
            )
        {
            let _ = manifest.record(record);
        }
    }

    fn record_generated_misses(&mut self, batch: &NeedResources, transaction: &str) {
        for request in batch.required.iter().chain(&batch.probes) {
            let ResourceRequest::File(request) = request else {
                continue;
            };
            self.record_file_absent(
                request,
                lookup_role(batch, request, false),
                "generated",
                NegativeScope::Generated {
                    transaction: transaction.to_owned(),
                },
            );
        }
    }

    fn record_request_readiness(
        &mut self,
        telemetry: &mut ResolverTelemetry,
        request: &FileRequestKey,
        readiness: umber_distribution::Readiness,
    ) {
        let previous = self.readiness.insert(request.clone(), readiness);
        if previous == Some(readiness) {
            return;
        }
        if let Some(previous) = previous {
            self.adjust_readiness_metric(telemetry, previous, -1);
        }
        self.adjust_readiness_metric(telemetry, readiness, 1);
    }

    fn adjust_readiness_metric(
        &self,
        telemetry: &mut ResolverTelemetry,
        readiness: Readiness,
        delta: i64,
    ) {
        match readiness {
            Readiness::Ready => {
                telemetry.ready_resources = telemetry.ready_resources.saturating_add_signed(delta);
            }
            Readiness::ExistsNotReady => {
                telemetry.exists_not_ready_resources = telemetry
                    .exists_not_ready_resources
                    .saturating_add_signed(delta);
            }
            Readiness::Absent => {
                telemetry.absent_resources =
                    telemetry.absent_resources.saturating_add_signed(delta);
            }
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

    #[cfg(test)]
    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn resolve_batch_with_prefetch(
        &mut self,
        local: &LocalResolver,
        batch: &NeedResources,
        cancellation: &FetchCancellation,
        telemetry: &mut ResolverTelemetry,
    ) -> Result<ResolvedDistributionBatch, NativeRunError> {
        let identity = PrefetchIdentity::new("test", "test", "test", "test", "test")
            .map_err(|error| NativeRunError::Selection(error.to_string()))?;
        let mut planner =
            PrefetchPlanner::new(identity, umber_distribution::PrefetchBudget::default());
        self.resolve_batch_with_catalog(
            local,
            batch,
            cancellation,
            telemetry,
            &mut planner,
            &mut |_| {},
        )
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn resolve_batch_with_catalog(
        &mut self,
        local: &LocalResolver,
        batch: &NeedResources,
        cancellation: &FetchCancellation,
        telemetry: &mut ResolverTelemetry,
        planner: &mut PrefetchPlanner,
        note_catalog: &mut impl FnMut(&[FileRequestKey]),
    ) -> Result<ResolvedDistributionBatch, NativeRunError> {
        check_cancelled(cancellation)?;
        let mut responses = Vec::new();
        let mut prefetch_requests = Vec::new();
        let mut local_prefetch = Vec::<(FileRequest, ResolvedFile)>::new();
        let mut catalog_exists = BTreeSet::new();
        let mut unresolved = Vec::new();
        for request in batch.required.iter().chain(&batch.probes) {
            match request {
                ResourceRequest::File(request) => {
                    let started = Instant::now();
                    telemetry.local_lookups = telemetry.local_lookups.saturating_add(1);
                    let resolved = local.resolve(request)?;
                    telemetry.local_lookup_time = telemetry
                        .local_lookup_time
                        .saturating_add(started.elapsed());
                    if let Some(file) = resolved {
                        telemetry.local_hits = telemetry.local_hits.saturating_add(1);
                        telemetry.demand_bytes = telemetry
                            .demand_bytes
                            .saturating_add(file.bytes.len() as u64);
                        self.record_request_readiness(
                            telemetry,
                            request.key(),
                            Readiness::ExistsNotReady,
                        );
                        self.record_file_resolved(
                            request,
                            lookup_role(batch, request, false),
                            "local",
                            &file,
                            None,
                        );
                        responses.push(ResourceResponse::File(file));
                    } else {
                        self.record_file_absent(
                            request,
                            lookup_role(batch, request, false),
                            "local",
                            self.project_negative_scope(),
                        );
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
                    let resolved = local.resolve_pk_font(request)?;
                    telemetry.local_lookup_time = telemetry
                        .local_lookup_time
                        .saturating_add(started.elapsed());
                    if let Some(font) = resolved {
                        telemetry.local_hits = telemetry.local_hits.saturating_add(1);
                        responses.push(ResourceResponse::PkFont(font));
                    } else {
                        let file = self.resolve_generic_file_with_planner(
                            local,
                            &request.logical_name(),
                            cancellation,
                            planner,
                        );
                        match file {
                            Ok(file) => responses.push(ResourceResponse::PkFont(ResolvedPkFont {
                                request: request.clone(),
                                virtual_path: file.virtual_path,
                                expected_ahash64: Some(
                                    umber_hash::AHash64::for_bytes(
                                        umber_hash::HashDomain::PkProgram,
                                        &file.bytes,
                                    )
                                    .to_le_bytes(),
                                ),
                                bytes: file.bytes.to_vec(),
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
            let resolved = local.resolve_prefetch(request)?;
            telemetry.local_lookup_time = telemetry
                .local_lookup_time
                .saturating_add(started.elapsed());
            if let Some(file) = resolved {
                telemetry.local_hits = telemetry.local_hits.saturating_add(1);
                local_prefetch.push((request.clone(), file));
            } else {
                unresolved_hints.push(request.clone());
            }
        }
        if unresolved.is_empty() && unresolved_hints.is_empty() {
            let selected = planner.select_prefetch_group(
                [],
                local_prefetch
                    .iter()
                    .map(|(request, file)| prefetch_candidate_for_file(request, file))
                    .collect::<Result<Vec<_>, _>>()?,
            );
            let selected_keys = selected
                .hints
                .into_iter()
                .filter_map(|candidate| candidate.file_key.map(|key| key.identity()))
                .collect::<BTreeSet<_>>();
            for (request, file) in local_prefetch {
                if semantic_file_key(request.key())
                    .is_some_and(|key| selected_keys.contains(&key.identity()))
                {
                    self.admit_local_prefetch(
                        telemetry,
                        request,
                        file,
                        &mut responses,
                        &mut prefetch_requests,
                    );
                }
            }
            return Ok(ResolvedDistributionBatch {
                admitted_files: admitted_files_for(
                    &responses,
                    batch
                        .required
                        .iter()
                        .chain(&batch.probes)
                        .chain(&batch.prefetch_hints)
                        .filter_map(|request| match request {
                            ResourceRequest::File(request) => Some(request.clone()),
                            ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                        }),
                ),
                responses,
                prefetch_requests,
                catalog_exists: catalog_exists.into_iter().collect(),
            });
        }
        let manifest_started = Instant::now();
        telemetry.manifest_lookups = telemetry.manifest_lookups.saturating_add(1);
        let loaded = self.load(cancellation, telemetry)?;
        let root = &loaded.root;
        let shard_bits = root.shard_bits;
        let objects_base_url = root.objects_base_url.clone();
        telemetry.manifest_lookup_time = telemetry
            .manifest_lookup_time
            .saturating_add(manifest_started.elapsed());
        let mut original_files = BTreeMap::<String, Vec<FileRequest>>::new();
        for request in &unresolved {
            let Some(key) = distribution_file_key(request)? else {
                responses.push(ResourceResponse::FileUnavailable(request.key().clone()));
                continue;
            };
            original_files
                .entry(key.manifest_key().to_string())
                .or_default()
                .push(request.clone());
        }
        let mut keys_by_shard = BTreeMap::<u32, Vec<String>>::new();
        for key in original_files.keys() {
            keys_by_shard
                .entry(
                    shard_index_for_key(key, shard_bits)
                        .map_err(|error| NativeRunError::Selection(error.to_string()))?,
                )
                .or_default()
                .push(key.clone());
        }
        let mut hinted_keys = BTreeMap::<u32, Vec<String>>::new();
        let mut original_hints = BTreeMap::<String, Vec<FileRequest>>::new();
        for request in &unresolved_hints {
            let Some(key) = distribution_file_key(request)? else {
                planner.note_catalog_result(request, false);
                continue;
            };
            let key = key.manifest_key().to_string();
            original_hints
                .entry(key.clone())
                .or_default()
                .push(request.clone());
            hinted_keys
                .entry(
                    shard_index_for_key(&key, shard_bits)
                        .map_err(|error| NativeRunError::Selection(error.to_string()))?,
                )
                .or_default()
                .push(key);
        }
        for keys in hinted_keys.values_mut() {
            keys.sort_unstable();
            keys.dedup();
        }
        let mut required = BTreeMap::<String, SelectedDistributionRecord>::new();
        let mut hints = BTreeMap::<String, SelectedDistributionRecord>::new();
        let mut fallback_files = BTreeMap::<String, Vec<FileRequest>>::new();
        let exact_misses = self.select_required_manifest_files(
            keys_by_shard,
            cancellation,
            telemetry,
            &mut required,
        )?;
        for key in exact_misses {
            let originals = original_files
                .remove(&key)
                .expect("requested key has an original file request");
            for original in originals {
                if let Some(fallback) = appended_tex_distribution_key(original.key())? {
                    fallback_files
                        .entry(fallback.manifest_key().to_string())
                        .or_default()
                        .push(original);
                } else {
                    planner.note_catalog_result(&original, false);
                    self.record_file_absent(
                        &original,
                        lookup_role(batch, &original, false),
                        "distribution",
                        self.distribution_negative_scope(),
                    );
                    self.record_request_readiness(telemetry, original.key(), Readiness::Absent);
                    responses.push(ResourceResponse::FileUnavailable(original.key().clone()));
                }
            }
        }
        let mut fallback_keys_by_shard = BTreeMap::<u32, Vec<String>>::new();
        for key in fallback_files.keys() {
            fallback_keys_by_shard
                .entry(
                    shard_index_for_key(key, shard_bits)
                        .map_err(|error| NativeRunError::Selection(error.to_string()))?,
                )
                .or_default()
                .push(key.clone());
        }
        let fallback_misses = self.select_required_manifest_files(
            fallback_keys_by_shard,
            cancellation,
            telemetry,
            &mut required,
        )?;
        for key in fallback_misses {
            let originals = fallback_files
                .remove(&key)
                .expect("fallback key has an original file request");
            for original in originals {
                planner.note_catalog_result(&original, false);
                self.record_file_absent(
                    &original,
                    lookup_role(batch, &original, false),
                    "distribution",
                    self.distribution_negative_scope(),
                );
                self.record_request_readiness(telemetry, original.key(), Readiness::Absent);
                responses.push(ResourceResponse::FileUnavailable(original.key().clone()));
            }
        }
        for (manifest_key, originals) in fallback_files {
            original_hints.remove(&manifest_key);
            original_files
                .entry(manifest_key)
                .or_default()
                .extend(originals);
        }
        for (index, keys) in hinted_keys {
            let manifest_started = Instant::now();
            telemetry.manifest_lookups = telemetry.manifest_lookups.saturating_add(1);
            match self.select_manifest_files(index, &keys, cancellation, telemetry) {
                Ok(selected) => {
                    telemetry.manifest_lookup_time = telemetry
                        .manifest_lookup_time
                        .saturating_add(manifest_started.elapsed());
                    for key in keys {
                        let present = selected.get(&key).is_some_and(Option::is_some);
                        if let Some(requests) = original_hints.get(&key) {
                            for request in requests {
                                planner.note_catalog_result(request, present);
                            }
                        }
                        if let Some(Some(entry)) = selected.get(&key) {
                            hints.insert(key.clone(), entry.clone());
                            telemetry.package_group_candidates =
                                telemetry.package_group_candidates.saturating_add(1);
                        }
                    }
                }
                Err(NativeRunError::Cancelled) => return Err(NativeRunError::Cancelled),
                Err(_) => {}
            }
        }
        // Retain the authenticated dependency metadata for later replay
        // escalation.  This map is scheduling evidence only; it never makes
        // a request engine-readable without a later payload admission.
        for (manifest_key, entry) in required.iter().chain(&hints) {
            let parent_requests = original_files
                .get(manifest_key)
                .cloned()
                .or_else(|| original_hints.get(manifest_key).cloned());
            let Some(parent_requests) = parent_requests else {
                continue;
            };
            for parent in &parent_requests {
                let dependencies = self
                    .known_dependencies
                    .entry(parent.key().clone())
                    .or_default();
                for dependency in &entry.dependencies {
                    let Ok(distribution_key) =
                        DistributionFileRequestKey::from_manifest_key(&dependency.key)
                    else {
                        continue;
                    };
                    let Ok(ResourceRequest::File(request)) = distribution_request(distribution_key)
                    else {
                        continue;
                    };
                    if !dependencies
                        .iter()
                        .any(|known| known.key() == request.key())
                    {
                        dependencies.push(request);
                    }
                }
            }
        }

        // Inline TLPDB-derived dependencies are already authenticated by the
        // packed shard.  Reuse their metadata directly; do not fetch an
        // arbitrary locality shard just to discover package peers.
        let dependency_sources = if batch.prefetch_hints.is_empty() {
            Vec::new()
        } else {
            required
                .values()
                .chain(hints.values())
                .flat_map(|entry| entry.dependencies.iter())
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut dependency_keys = BTreeSet::new();
        for dependency in dependency_sources {
            if required.contains_key(&dependency.key) || dependency_keys.contains(&dependency.key) {
                continue;
            }
            let request = match DistributionFileRequestKey::from_manifest_key(&dependency.key)
                .map_err(|_| ())
                .and_then(|key| distribution_request(key).map_err(|_| ()))
            {
                Ok(ResourceRequest::File(request)) => request,
                Ok(ResourceRequest::Font(_) | ResourceRequest::PkFont(_)) | Err(_) => continue,
            };
            if hints.contains_key(&dependency.key) {
                // An explicit hint already owns the response path.  It still
                // remains a prefetch candidate, but is authorized by the
                // engine's ordinary hint batch.
                dependency_keys.insert(dependency.key.clone());
                continue;
            }
            let started = Instant::now();
            telemetry.local_lookups = telemetry.local_lookups.saturating_add(1);
            let resolved = local.resolve_prefetch(&request)?;
            telemetry.local_lookup_time = telemetry
                .local_lookup_time
                .saturating_add(started.elapsed());
            if let Some(file) = resolved {
                telemetry.local_hits = telemetry.local_hits.saturating_add(1);
                local_prefetch.push((request.clone(), file));
                dependency_keys.insert(dependency.key.clone());
            } else {
                hints.insert(
                    dependency.key.clone(),
                    SelectedDistributionRecord {
                        virtual_path: dependency.virtual_path.clone(),
                        object: dependency.object.clone(),
                        dependencies: Vec::new(),
                    },
                );
                original_hints
                    .entry(dependency.key.clone())
                    .or_default()
                    .push(request);
                if let Some(request) = original_hints
                    .get(&dependency.key)
                    .and_then(|requests| requests.last())
                {
                    planner.note_catalog_result(request, true);
                }
                dependency_keys.insert(dependency.key.clone());
                telemetry.package_group_candidates =
                    telemetry.package_group_candidates.saturating_add(1);
            }
        }
        for (manifest_key, requests) in &original_files {
            if required.contains_key(manifest_key) {
                for request in requests {
                    planner.note_catalog_result(request, true);
                    catalog_exists.insert(request.key().clone());
                    self.record_request_readiness(
                        telemetry,
                        request.key(),
                        Readiness::ExistsNotReady,
                    );
                }
            }
        }
        for (manifest_key, requests) in &original_hints {
            if hints.contains_key(manifest_key) {
                for request in requests {
                    catalog_exists.insert(request.key().clone());
                    self.record_request_readiness(
                        telemetry,
                        request.key(),
                        Readiness::ExistsNotReady,
                    );
                }
            }
        }
        let catalog_exists_keys = catalog_exists.iter().cloned().collect::<Vec<_>>();
        note_catalog(&catalog_exists_keys);
        let required_fetches = required
            .iter()
            .map(|(key, entry)| FetchRequest {
                request_key: key.clone(),
                object: entry.object.clone(),
                max_bytes: crate::SessionLimits::default().one_file_bytes as u64,
            })
            .collect::<Vec<_>>();
        let limits = crate::SessionLimits::default();
        let mut hint_fetches = Vec::new();
        let mut hint_candidates = local_prefetch
            .iter()
            .map(|(request, file)| prefetch_candidate_for_file(request, file))
            .collect::<Result<Vec<_>, _>>()?;
        hint_candidates.extend(
            hints
                .iter()
                .flat_map(|(key, entry)| {
                    original_hints
                        .get(key)
                        .into_iter()
                        .flatten()
                        .filter_map(move |request| {
                            Some(umber_distribution::PrefetchCandidate {
                                key: key.clone(),
                                object: entry.object.clone(),
                                class: prefetch_class_for_request(request, key),
                                required: false,
                                file_key: semantic_file_key(request.key()),
                            })
                        })
                })
                .collect::<Vec<_>>(),
        );
        let selected_hints = planner.select_prefetch_group(
            required.iter().flat_map(|(key, entry)| {
                original_files
                    .get(key)
                    .into_iter()
                    .flatten()
                    .map(move |request| umber_distribution::PrefetchCandidate {
                        key: key.clone(),
                        object: entry.object.clone(),
                        class: prefetch_class_for_request(request, key),
                        required: true,
                        file_key: semantic_file_key(request.key()),
                    })
            }),
            hint_candidates,
        );
        let selected_semantic_keys = selected_hints
            .hints
            .into_iter()
            .filter_map(|candidate| candidate.file_key.map(|key| key.identity()))
            .collect::<BTreeSet<_>>();
        for (request, file) in local_prefetch {
            if semantic_file_key(request.key())
                .is_some_and(|key| selected_semantic_keys.contains(&key.identity()))
            {
                self.admit_local_prefetch(
                    telemetry,
                    request,
                    file,
                    &mut responses,
                    &mut prefetch_requests,
                );
            }
        }
        for (key, entry) in &hints {
            if required.contains_key(key) {
                // The demanded fetch below already acquires this shared
                // payload; keep the semantic hint response but avoid a
                // second transport request.
                continue;
            }
            if original_hints.get(key).is_some_and(|requests| {
                requests.iter().any(|request| {
                    semantic_file_key(request.key()).is_some_and(|file_key| {
                        selected_semantic_keys.contains(&file_key.identity())
                    })
                })
            }) {
                hint_fetches.push(FetchRequest {
                    request_key: key.clone(),
                    object: entry.object.clone(),
                    max_bytes: limits.one_file_bytes as u64,
                });
            }
        }
        let mut fetch_requests = required_fetches.clone();
        fetch_requests.extend(hint_fetches);
        telemetry.object_requests = telemetry
            .object_requests
            .saturating_add(fetch_requests.len() as u64);
        let object_started = Instant::now();
        let fetched =
            match self.fetch_objects(&objects_base_url, &fetch_requests, cancellation, telemetry) {
                Ok(fetched) => fetched,
                Err(NativeRunError::Cancelled) => return Err(NativeRunError::Cancelled),
                Err(_)
                    if !required_fetches.is_empty()
                        && fetch_requests.len() > required_fetches.len() =>
                {
                    self.fetch_objects(
                        &objects_base_url,
                        &required_fetches,
                        cancellation,
                        telemetry,
                    )?
                }
                Err(_) if required_fetches.is_empty() => {
                    // A speculative hint is advisory.  Transport or object
                    // validation failure must not become semantic absence or
                    // abort a demanded request in the same run.
                    return Ok(ResolvedDistributionBatch {
                        admitted_files: admitted_files_for(
                            &responses,
                            batch
                                .required
                                .iter()
                                .chain(&batch.probes)
                                .chain(&batch.prefetch_hints)
                                .filter_map(|request| match request {
                                    ResourceRequest::File(request) => Some(request.clone()),
                                    ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                                })
                                .chain(prefetch_requests.iter().cloned()),
                        ),
                        responses,
                        prefetch_requests,
                        catalog_exists: catalog_exists.into_iter().collect(),
                    });
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
        let bytes = fetched
            .into_iter()
            .map(|(key, bytes, _)| (key, tex_state::SharedBytes::from(bytes)))
            .collect::<BTreeMap<_, _>>();
        let response_started = Instant::now();
        let hash_before = telemetry.content_hash_time;
        for (manifest_key, entry) in required {
            let data = bytes.get(&manifest_key).expect("fetched required object");
            let keys = original_files
                .remove(&manifest_key)
                .expect("original file request");
            let hash_started = Instant::now();
            let expected_digest = FileContentId::for_bytes(data);
            telemetry.content_hash_time = telemetry
                .content_hash_time
                .saturating_add(hash_started.elapsed());
            for key in keys {
                let request = key.clone();
                let file = ResolvedFile {
                    request: key.key().clone(),
                    expected_digest: Some(expected_digest),
                    virtual_path: entry.virtual_path.clone(),
                    bytes: data.clone(),
                };
                telemetry.demand_bytes = telemetry.demand_bytes.saturating_add(data.len() as u64);
                self.record_request_readiness(telemetry, request.key(), Readiness::ExistsNotReady);
                self.record_file_resolved(
                    &request,
                    lookup_role(batch, &request, false),
                    "distribution",
                    &file,
                    Some(&entry.object),
                );
                responses.push(ResourceResponse::File(file));
            }
        }
        for (manifest_key, keys) in original_hints {
            let Some(data) = bytes.get(&manifest_key) else {
                continue;
            };
            let entry = hints
                .get(&manifest_key)
                .expect("fetched closure hint has manifest metadata");
            let hash_started = Instant::now();
            let expected_digest = FileContentId::for_bytes(data);
            telemetry.content_hash_time = telemetry
                .content_hash_time
                .saturating_add(hash_started.elapsed());
            for key in keys {
                let request = key.clone();
                let file = ResolvedFile {
                    request: key.key().clone(),
                    expected_digest: Some(expected_digest),
                    virtual_path: entry.virtual_path.clone(),
                    bytes: data.clone(),
                };
                self.record_request_readiness(telemetry, request.key(), Readiness::ExistsNotReady);
                self.record_file_resolved(
                    &request,
                    LookupRole::Hint,
                    "distribution",
                    &file,
                    Some(&entry.object),
                );
                self.note_prefetch_admitted(&request, &file);
                // Every speculative response is authorized through the same VFS
                // seam. Dependency-discovered files and explicit hints therefore
                // share one closure queue and one admission callback.
                prefetch_requests.push(key.clone());
                responses.push(ResourceResponse::File(file));
            }
        }
        drop(hints);
        telemetry.response_build_time = telemetry.response_build_time.saturating_add(
            response_started
                .elapsed()
                .saturating_sub(telemetry.content_hash_time.saturating_sub(hash_before)),
        );
        Ok(ResolvedDistributionBatch {
            admitted_files: admitted_files_for(
                &responses,
                batch
                    .required
                    .iter()
                    .chain(&batch.probes)
                    .chain(&batch.prefetch_hints)
                    .filter_map(|request| match request {
                        ResourceRequest::File(request) => Some(request.clone()),
                        ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                    })
                    .chain(prefetch_requests.iter().cloned()),
            ),
            responses,
            prefetch_requests,
            catalog_exists: catalog_exists.into_iter().collect(),
        })
    }

    #[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
    fn select_required_manifest_files(
        &mut self,
        keys_by_shard: BTreeMap<u32, Vec<String>>,
        cancellation: &FetchCancellation,
        telemetry: &mut ResolverTelemetry,
        required: &mut BTreeMap<String, SelectedDistributionRecord>,
    ) -> Result<Vec<String>, NativeRunError> {
        let mut misses = Vec::new();
        for (index, keys) in keys_by_shard {
            let manifest_started = Instant::now();
            telemetry.manifest_lookups = telemetry.manifest_lookups.saturating_add(1);
            let selected = self.select_manifest_files(index, &keys, cancellation, telemetry)?;
            telemetry.manifest_lookup_time = telemetry
                .manifest_lookup_time
                .saturating_add(manifest_started.elapsed());
            for key in keys {
                match selected
                    .get(&key)
                    .expect("verified selection covers every requested key")
                {
                    Some(entry) => {
                        required.insert(key, entry.clone());
                    }
                    None => misses.push(key),
                }
            }
        }
        Ok(misses)
    }

    #[cfg(test)]
    fn resolve_generic_file(
        &mut self,
        local: &LocalResolver,
        logical_name: &[u8],
        cancellation: &FetchCancellation,
    ) -> Result<ResolvedFile, NativeRunError> {
        let identity = PrefetchIdentity::new("test", "test", "test", "test", "test")
            .map_err(|error| NativeRunError::Selection(error.to_string()))?;
        let mut planner =
            PrefetchPlanner::new(identity, umber_distribution::PrefetchBudget::default());
        self.resolve_generic_file_with_planner(local, logical_name, cancellation, &mut planner)
    }

    fn resolve_generic_file_with_planner(
        &mut self,
        local: &LocalResolver,
        logical_name: &[u8],
        cancellation: &FetchCancellation,
        planner: &mut PrefetchPlanner,
    ) -> Result<ResolvedFile, NativeRunError> {
        let name = std::str::from_utf8(logical_name).map_err(|_| {
            NativeRunError::Selection("PDF resource name is not valid UTF-8".to_owned())
        })?;
        let key = crate::FileRequestKey::new(FileKind::GenericAsset, name)
            .map_err(|error| NativeRunError::Selection(error.to_string()))?;
        let resolved = self.resolve_batch_with_catalog(
            local,
            &NeedResources {
                required: vec![ResourceRequest::File(FileRequest::new(key.clone(), name))],
                probes: Vec::new(),
                prefetch_hints: Vec::new(),
            },
            cancellation,
            &mut ResolverTelemetry::default(),
            planner,
            &mut |_| {},
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
        telemetry: &mut ResolverTelemetry,
    ) -> Result<ResolvedFormat, NativeRunError> {
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| NativeRunError::Format("format name is not valid UTF-8".into()))?;
        telemetry.manifest_lookups = telemetry.manifest_lookups.saturating_add(1);
        let loaded = self.load(cancellation, telemetry)?;
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
        if entry.format_schema != FORMAT_SCHEMA_VERSION {
            return Err(NativeRunError::Format(format!(
                "format {name} uses schema {}; this runtime requires schema {}",
                entry.format_schema, FORMAT_SCHEMA_VERSION
            )));
        }
        if entry.engine != engine.name() && entry.engine != "umber" {
            return Err(NativeRunError::Format(format!(
                "format {name} targets {}, not {}",
                entry.engine,
                engine.name()
            )));
        }
        telemetry.object_requests = telemetry.object_requests.saturating_add(1);
        if let Some(bytes) = self
            .client
            .store()
            .load_object(&entry.ahash64, entry.bytes)
            .map_err(|error| NativeRunError::Cache(error.to_string()))?
        {
            telemetry.object_cache_hits = telemetry.object_cache_hits.saturating_add(1);
            telemetry.object_hashes = telemetry.object_hashes.saturating_add(1);
            return Ok(ResolvedFormat { bytes });
        }
        let object = umber_distribution::ObjectEntry {
            object: entry.object,
            ahash64: entry.ahash64,
            bytes: entry.bytes,
        };
        if let Some(root) = &loaded.local_root {
            let bytes = read(&local_object_path(root, &object.object))?;
            check_cancelled(cancellation)?;
            self.client
                .store()
                .store_object(&object.ahash64, object.bytes, &bytes)
                .map_err(|error| NativeRunError::Cache(error.to_string()))?;
            telemetry.object_hashes = telemetry.object_hashes.saturating_add(1);
            eprintln!("umber: acquired 1 distribution resource(s)");
            return Ok(ResolvedFormat { bytes });
        }
        if self.offline {
            return Err(NativeRunError::DistributionUnavailable(vec![format!(
                "format:{name}"
            )]));
        }
        let request = FetchRequest {
            request_key: format!("format:{name}"),
            object,
            max_bytes: crate::SessionLimits::FORMAT_IMAGE_BYTES as u64,
        };
        let object = self
            .client
            .acquire_batch(&loaded.root.objects_base_url, &[request], cancellation)
            .map_err(map_fetch_error)?
            .pop()
            .expect("one format result");
        telemetry.object_hashes = telemetry.object_hashes.saturating_add(1);
        if object.cache_hit {
            telemetry.object_cache_hits = telemetry.object_cache_hits.saturating_add(1);
        }
        if !object.cache_hit {
            eprintln!("umber: acquired 1 distribution resource(s)");
        }
        Ok(ResolvedFormat {
            bytes: object.bytes,
        })
    }

    fn fetch_objects(
        &self,
        objects_base_url: &str,
        requests: &[FetchRequest],
        cancellation: &FetchCancellation,
        telemetry: &mut ResolverTelemetry,
    ) -> Result<Vec<(String, Vec<u8>, bool)>, NativeRunError> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let local_root = self
            .verified
            .lock()
            .map_err(|_| NativeRunError::Cache("verified distribution owner poisoned".into()))?
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.local_root.clone());
        let mut found = Vec::new();
        let mut remaining = Vec::new();
        for request in requests {
            check_cancelled(cancellation)?;
            match self
                .client
                .store()
                .load_object(&request.object.ahash64, request.object.bytes)
            {
                Ok(Some(bytes)) => {
                    telemetry.object_hashes = telemetry.object_hashes.saturating_add(1);
                    found.push((request.request_key.clone(), bytes, true));
                }
                Ok(None) => remaining.push(request.clone()),
                Err(error) => return Err(NativeRunError::Cache(error.to_string())),
            }
        }
        if remaining.is_empty() {
            return Ok(found);
        }
        if let Some(local_root) = local_root {
            for request in remaining {
                check_cancelled(cancellation)?;
                let bytes = read(&local_object_path(&local_root, &request.object.object))?;
                check_cancelled(cancellation)?;
                self.client
                    .store()
                    .store_object(&request.object.ahash64, request.object.bytes, &bytes)
                    .map_err(|error| NativeRunError::Cache(error.to_string()))?;
                telemetry.object_hashes = telemetry.object_hashes.saturating_add(1);
                found.push((request.request_key, bytes, false));
            }
            return Ok(found);
        }
        if self.offline {
            return Err(NativeRunError::DistributionUnavailable(
                remaining
                    .into_iter()
                    .map(|request| request.request_key)
                    .collect(),
            ));
        }
        let objects = self
            .client
            .acquire_batch(objects_base_url, &remaining, cancellation)
            .map_err(map_fetch_error)?;
        telemetry.object_hashes = telemetry.object_hashes.saturating_add(objects.len() as u64);
        found.extend(
            objects
                .into_iter()
                .map(|object| (object.request_key, object.bytes, object.cache_hit)),
        );
        Ok(found)
    }

    fn select_manifest_files(
        &mut self,
        index: u32,
        request_keys: &[String],
        cancellation: &FetchCancellation,
        telemetry: &mut ResolverTelemetry,
    ) -> Result<BTreeMap<String, Option<SelectedDistributionRecord>>, NativeRunError> {
        check_cancelled(cancellation)?;
        let loaded = self.load(cancellation, telemetry)?;
        let verified = Arc::clone(&self.verified);
        let mut state = verified
            .lock()
            .map_err(|_| NativeRunError::Cache("verified distribution owner poisoned".into()))?;
        let shared = state.loaded.as_mut().expect("root loaded before shard");
        if shared.shards.contains_key(&index) {
            telemetry.verified_manifest_hits = telemetry.verified_manifest_hits.saturating_add(1);
            shared.record_retention(telemetry);
            return Ok(selected_records(
                &shared.shards[&index],
                request_keys,
                telemetry,
            ));
        }
        let local_root = loaded.local_root.clone();
        let digest = loaded
            .root
            .shard_digest(index)
            .expect("canonical shard index is bounded by shardBits")
            .to_owned();
        let bytes = if let Some(bytes) = self
            .client
            .load_manifest(&digest)
            .map_err(|error| NativeRunError::Cache(error.to_string()))?
        {
            telemetry.manifest_cache_hits = telemetry.manifest_cache_hits.saturating_add(1);
            bytes
        } else {
            let bytes = if let Some(local_root) = &local_root {
                let path = local_object_path(local_root, &format!("ahash64-v1-{digest}"));
                let bytes =
                    match read_bounded(&path, MAX_INDEX_SHARD_BYTES, "distribution index shard") {
                        Ok(bytes) => bytes,
                        Err(NativeRunError::Io { source, .. })
                            if source.kind() == std::io::ErrorKind::NotFound =>
                        {
                            return Err(shard_unavailable_error(
                                index,
                                &digest,
                                request_keys,
                                Some(path),
                            ));
                        }
                        Err(error) => return Err(error),
                    };
                verify_manifest_digest(&bytes, &digest)?;
                bytes
            } else if self.offline {
                return Err(shard_unavailable_error(index, &digest, request_keys, None));
            } else {
                let url = format!("{}ahash64-v1-{digest}", loaded.root.objects_base_url);
                self.client
                    .acquire_manifest(&url, &digest, cancellation)
                    .map_err(map_distribution_client_error)?
                    .bytes
            };
            check_cancelled(cancellation)?;
            self.client
                .store()
                .store_manifest(&digest, &bytes)
                .map_err(|error| NativeRunError::Cache(error.to_string()))?;
            bytes
        };
        telemetry.manifest_reads = telemetry.manifest_reads.saturating_add(1);
        telemetry.manifest_read_bytes = telemetry
            .manifest_read_bytes
            .saturating_add(bytes.len() as u64);
        telemetry.manifest_validations = telemetry.manifest_validations.saturating_add(1);
        telemetry.manifest_parse_peak_bytes =
            telemetry.manifest_parse_peak_bytes.max(bytes.len() as u64);
        check_cancelled(cancellation)?;
        telemetry.packed_validation_calls = telemetry.packed_validation_calls.saturating_add(1);
        telemetry.packed_validation_bytes = telemetry
            .packed_validation_bytes
            .saturating_add(bytes.len() as u64);
        let shard = ValidatedPackedShard::new(bytes, &loaded.root, index)
            .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
        telemetry.shard_loads = telemetry.shard_loads.saturating_add(1);
        shared.shards.insert(index, Arc::new(shard));
        let selected = selected_records(&shared.shards[&index], request_keys, telemetry);
        shared.record_retention(telemetry);
        Ok(selected)
    }

    fn load(
        &mut self,
        cancellation: &FetchCancellation,
        telemetry: &mut ResolverTelemetry,
    ) -> Result<LoadedDistribution, NativeRunError> {
        check_cancelled(cancellation)?;
        let verified = Arc::clone(&self.verified);
        let mut state = verified
            .lock()
            .map_err(|_| NativeRunError::Cache("verified distribution owner poisoned".into()))?;
        if let Some(loaded) = &state.loaded {
            telemetry.verified_manifest_hits = telemetry.verified_manifest_hits.saturating_add(1);
            return Ok(loaded.clone());
        }
        {
            if self.source.is_none() && self.expected.is_none() {
                return Err(NativeRunError::DefaultDistributionUnpublished);
            }
            let source = self
                .source
                .clone()
                .unwrap_or_else(|| DEFAULT_DISTRIBUTION_URL.to_owned());
            let explicit = self.source.is_some();
            let path = PathBuf::from(&source);
            let local_path = if path.is_dir() {
                let schema_nine = path.join("manifest-v9.json");
                let schema_eight = path.join("manifest-v8.json");
                if schema_nine.exists() {
                    schema_nine
                } else if schema_eight.exists() {
                    schema_eight
                } else {
                    path.join("manifest.json")
                }
            } else {
                path.clone()
            };
            let is_local = local_path.exists() || (!source.contains("://") && explicit);
            let expected = self.expected.clone();
            let (manifest_bytes, local_root) = if is_local {
                let bytes = read_bounded(
                    &local_path,
                    MAX_INDEX_SHARD_BYTES,
                    "distribution root manifest",
                )?;
                if let Some(expected) = &expected {
                    verify_manifest_digest(&bytes, expected)?;
                    telemetry.manifest_validations =
                        telemetry.manifest_validations.saturating_add(1);
                }
                (bytes, local_path.parent().map(Path::to_owned))
            } else {
                let expected = expected
                    .ok_or_else(|| NativeRunError::DistributionPinRequired(source.clone()))?;
                let bytes = if let Some(bytes) = self
                    .client
                    .load_manifest(&expected)
                    .map_err(|error| NativeRunError::Cache(error.to_string()))?
                {
                    telemetry.manifest_cache_hits = telemetry.manifest_cache_hits.saturating_add(1);
                    bytes
                } else {
                    if self.offline {
                        return Err(NativeRunError::DistributionUnavailable(vec![
                            "manifest".into(),
                        ]));
                    }
                    self.client
                        .acquire_manifest(&source, &expected, cancellation)
                        .map_err(map_distribution_client_error)?
                        .bytes
                };
                telemetry.manifest_validations = telemetry.manifest_validations.saturating_add(1);
                (bytes, None)
            };
            telemetry.manifest_reads = telemetry.manifest_reads.saturating_add(1);
            telemetry.manifest_read_bytes = telemetry
                .manifest_read_bytes
                .saturating_add(manifest_bytes.len() as u64);
            telemetry.manifest_parse_peak_bytes = telemetry
                .manifest_parse_peak_bytes
                .max(manifest_bytes.len() as u64);
            let text = std::str::from_utf8(&manifest_bytes)
                .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
            let root = ShardedManifestRoot::parse(text)
                .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
            telemetry.manifest_parses = telemetry.manifest_parses.saturating_add(1);
            state.loaded = Some(LoadedDistribution {
                root: Arc::new(root),
                local_root,
                shards: BTreeMap::new(),
            });
        }
        Ok(state.loaded.as_ref().expect("distribution loaded").clone())
    }
}

fn selected_records(
    shard: &ValidatedPackedShard,
    request_keys: &[String],
    telemetry: &mut ResolverTelemetry,
) -> BTreeMap<String, Option<SelectedDistributionRecord>> {
    telemetry.packed_selection_calls = telemetry.packed_selection_calls.saturating_add(1);
    telemetry.packed_selection_keys = telemetry
        .packed_selection_keys
        .saturating_add(request_keys.len() as u64);
    telemetry.packed_selection_bytes = telemetry
        .packed_selection_bytes
        .saturating_add(shard.bytes().len() as u64);
    request_keys
        .iter()
        .map(|key| {
            (
                key.clone(),
                shard
                    .lookup(key)
                    .and_then(|record| record.file())
                    .map(|file| SelectedDistributionRecord {
                        virtual_path: file.virtual_path().to_owned(),
                        object: file.object(),
                        dependencies: file
                            .dependencies()
                            .map(|dependency| SelectedDistributionDependency {
                                key: dependency.key().to_owned(),
                                virtual_path: dependency.virtual_path().to_owned(),
                                object: dependency.object(),
                            })
                            .collect(),
                    }),
            )
        })
        .collect()
}

fn emit_failed_distribution_telemetry(telemetry: ResolverTelemetry) {
    if resource_telemetry_enabled() {
        eprintln!(
            "DISTRIBUTION_MANIFEST_TELEMETRY manifest_reads={} manifest_read_bytes={} manifest_parses={} manifest_validations={} shard_loads={} packed_selection_calls={} packed_selection_keys={} packed_selection_bytes={} packed_validation_calls={} packed_validation_bytes={} manifest_parse_peak_bytes={} retained_manifest_shards={} retained_manifest_bytes={}",
            telemetry.manifest_reads,
            telemetry.manifest_read_bytes,
            telemetry.manifest_parses,
            telemetry.manifest_validations,
            telemetry.shard_loads,
            telemetry.packed_selection_calls,
            telemetry.packed_selection_keys,
            telemetry.packed_selection_bytes,
            telemetry.packed_validation_calls,
            telemetry.packed_validation_bytes,
            telemetry.manifest_parse_peak_bytes,
            telemetry.retained_manifest_shards,
            telemetry.retained_manifest_bytes,
        );
    }
}

fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "na".to_owned(), |value| value.to_string())
}

fn emit_planner_diagnostic(
    index: u64,
    key: &umber_distribution::PrefetchFileKey,
    disposition: PrefetchDiagnosticDisposition,
) {
    eprintln!(
        "RESOURCE_PLANNER_DECISION index={} key={} disposition={}",
        index,
        escape_telemetry_field(&format!(
            "{}:{}:{}",
            key.domain, key.kind, key.normalized_name
        )),
        planner_disposition_name(disposition),
    );
}

const fn planner_disposition_name(disposition: PrefetchDiagnosticDisposition) -> &'static str {
    match disposition {
        PrefetchDiagnosticDisposition::HintSelected => "hint_selected",
        PrefetchDiagnosticDisposition::CandidateSelected => "candidate_selected",
        PrefetchDiagnosticDisposition::CatalogPresent => "catalog_present",
        PrefetchDiagnosticDisposition::CatalogAbsent => "catalog_absent",
        PrefetchDiagnosticDisposition::ContentAdmitted => "content_admitted",
        PrefetchDiagnosticDisposition::AlreadyResident => "already_resident",
        PrefetchDiagnosticDisposition::RuntimeTextScanned => "runtime_text_scanned",
        PrefetchDiagnosticDisposition::RuntimeTextScanSkipped => "runtime_text_scan_skipped",
        PrefetchDiagnosticDisposition::BudgetSkipped => "budget_skipped",
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
        FileKind::Image
        | FileKind::VirtualFont
        | FileKind::PdfFontMap
        | FileKind::PdfEncoding
        | FileKind::PdfFontProgram => DistributionFileKind::Tex,
        _ => return Ok(None),
    };
    DistributionFileRequestKey::new(kind, request.key().name())
        .map(Some)
        .map_err(|error| NativeRunError::Selection(error.to_string()))
}

fn prefetch_candidate_for_file(
    request: &FileRequest,
    file: &ResolvedFile,
) -> Result<umber_distribution::PrefetchCandidate, NativeRunError> {
    let key = distribution_file_key(request)?.ok_or_else(|| {
        NativeRunError::Selection(format!(
            "prefetch request kind {} has no distribution catalogue key",
            request.key().kind().wire_name()
        ))
    })?;
    let manifest_key = key.manifest_key().to_string();
    Ok(umber_distribution::PrefetchCandidate {
        key: manifest_key.clone(),
        object: ObjectEntry {
            object: "local".to_owned(),
            ahash64: AHash64::for_bytes(HashDomain::DistributionContent, &file.bytes).hex(),
            bytes: file.bytes.len() as u64,
        },
        class: prefetch_class_for_request(request, &manifest_key),
        required: false,
        file_key: semantic_file_key(request.key()),
    })
}

fn prefetch_class_for_request(
    request: &FileRequest,
    transport_key: &str,
) -> umber_distribution::PrefetchClass {
    if request.key().kind() == FileKind::Image {
        umber_distribution::PrefetchClass::Image
    } else {
        umber_distribution::PrefetchClass::for_key(transport_key)
    }
}

fn lookup_role(batch: &NeedResources, request: &FileRequest, hint: bool) -> LookupRole {
    if hint {
        return LookupRole::Hint;
    }
    if batch
        .required
        .iter()
        .any(|candidate| matches!(candidate, ResourceRequest::File(value) if value == request))
    {
        LookupRole::Required
    } else {
        LookupRole::Probe
    }
}

/// Web2C `tex.ch` [29.537] asks Kpathsea to try both an input name as written
/// and the same name with `.tex` appended, even when the written name already
/// has an extension. The local resolver performs the same ordered fallback;
/// this returns only the second candidate for a remote manifest miss.
fn appended_tex_distribution_key(
    request: &FileRequestKey,
) -> Result<Option<DistributionFileRequestKey>, NativeRunError> {
    if request.kind() != FileKind::TexInput {
        return Ok(None);
    }
    let path = Path::new(request.name());
    if path.extension().is_none_or(|extension| extension == "tex") {
        return Ok(None);
    }
    let mut name = path.as_os_str().to_os_string();
    name.push(".tex");
    let name = name
        .to_str()
        .ok_or_else(|| NativeRunError::Selection("TeX input name is not valid UTF-8".into()))?;
    DistributionFileRequestKey::new(DistributionFileKind::Tex, name)
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

fn check_cancelled(cancellation: &FetchCancellation) -> Result<(), NativeRunError> {
    if cancellation.is_cancelled() {
        Err(NativeRunError::Cancelled)
    } else {
        Ok(())
    }
}

fn local_object_path(root: &Path, object: &str) -> PathBuf {
    let objects = root.join("objects").join(object);
    if objects.exists() {
        objects
    } else {
        root.join(object)
    }
}

fn shard_unavailable_error(
    index: u32,
    digest: &str,
    request_keys: &[String],
    path: Option<PathBuf>,
) -> NativeRunError {
    const MAX_DIAGNOSTIC_KEYS: usize = 4;

    NativeRunError::DistributionShardUnavailable {
        index,
        digest: digest.to_owned(),
        request_keys: request_keys
            .iter()
            .take(MAX_DIAGNOSTIC_KEYS)
            .cloned()
            .collect(),
        omitted_request_keys: request_keys.len().saturating_sub(MAX_DIAGNOSTIC_KEYS),
        path,
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

fn map_distribution_client_error(error: DistributionClientError) -> NativeRunError {
    match error {
        DistributionClientError::Manifest(ManifestFetchError::Cancelled) => {
            NativeRunError::Cancelled
        }
        DistributionClientError::Manifest(error) => {
            NativeRunError::ManifestFetch(error.to_string())
        }
        DistributionClientError::Cache(error) => NativeRunError::Cache(error.to_string()),
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
    AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex()
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
