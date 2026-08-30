//! Native host policy for driving one CLI compile through the resource loop.

use std::cell::RefCell;
use std::collections::BTreeMap;
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
    FileKind as DistributionFileKind, FileRequestKey as DistributionFileRequestKey, ObjectEntry,
    ShardedManifestRoot, ValidatedPackedShard, shard_index_for_key,
};
use umber_fetch::{
    DistributionClient, DistributionClientError, FetchCancellation, FetchClientConfig,
    FetchFailure, FetchRequest, ManifestFetchError, ObjectCache,
};
use umber_hash::{AHash64, HashDomain};

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
    let output = match session.compile(&cancellation) {
        Ok(output) => output,
        Err(error) => {
            emit_failed_distribution_telemetry(session.host_telemetry.resolver);
            return Err(error);
        }
    };
    let accepted_handoff_started = Instant::now();
    let input_path_map = session.local.input_path_map();
    let resolved_inputs = session.local.resolved_inputs();
    let main_input = (options.input.clone(), session.source.len());
    let telemetry = session.session.compile_telemetry();
    let mut host_telemetry = session.host_telemetry;
    let NativeCompileSession { session, .. } = session;
    let finalization = session
        .into_accepted_finalization()
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
    source: String,
    pending_source: Option<String>,
    host_telemetry: NativeHostTelemetry,
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
        let initial_prefetch_hints = options
            .initial_prefetch_keys
            .iter()
            .map(|key| {
                DistributionFileRequestKey::from_manifest_key(key)
                    .map_err(|error| NativeRunError::Selection(error.to_string()))
                    .and_then(distribution_request)
            })
            .collect::<Result<Vec<_>, _>>()?;
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
            || env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1")
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
            #[cfg(test)]
            guards,
            distribution,
            local,
            source,
            pending_source: None,
            host_telemetry: NativeHostTelemetry {
                startup_time,
                resolver: resolver_telemetry,
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
                    return Err(match error {
                        CompileError::Diagnostic(diagnostic) => {
                            NativeRunError::Diagnostic(Box::new(diagnostic))
                        }
                        error => NativeRunError::Compile(error.to_string()),
                    });
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
}

fn selected_limit(
    explicit: Option<u64>,
    environment_name: &'static str,
    default: u64,
) -> Result<u64, NativeRunError> {
    let environment = env::var(environment_name).ok();
    selected_limit_value(explicit, environment.as_deref(), environment_name, default)
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
        let resolved_path = content.path().to_owned();
        let mut input_paths = self.input_paths.borrow_mut();
        input_paths.insert(virtual_path.clone(), resolved_path.clone());
        input_paths.insert(
            PathBuf::from(request.original_name()),
            resolved_path.clone(),
        );
        input_paths.insert(PathBuf::from(request.key().name()), resolved_path);
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
        let digest =
            umber_hash::AHash64::for_bytes(umber_hash::HashDomain::PkProgram, &bytes).to_le_bytes();
        let virtual_path = self.virtual_path(
            FileKind::GenericAsset,
            content.path(),
            FileContentId::for_bytes(&bytes),
        );
        Some(ResolvedPkFont {
            request: request.clone(),
            virtual_path: virtual_path.to_string_lossy().into_owned(),
            bytes,
            expected_ahash64: Some(digest),
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
    root: Arc<ShardedManifestRoot>,
    local_root: Option<PathBuf>,
    shards: BTreeMap<u32, Arc<ValidatedPackedShard>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SelectedDistributionRecord {
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
}

struct DistributionResolver {
    client: DistributionClient,
    source: Option<String>,
    expected: Option<String>,
    offline: bool,
    verified: Arc<Mutex<VerifiedDistributionState>>,
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
                                expected_ahash64: Some(
                                    umber_hash::AHash64::for_bytes(
                                        umber_hash::HashDomain::PkProgram,
                                        &file.bytes,
                                    )
                                    .to_le_bytes(),
                                ),
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
        let loaded = self.load(cancellation, telemetry)?;
        let root = &loaded.root;
        let shard_bits = root.shard_bits;
        let objects_base_url = root.objects_base_url.clone();
        telemetry.manifest_lookup_time = telemetry
            .manifest_lookup_time
            .saturating_add(manifest_started.elapsed());
        let mut original_files = BTreeMap::<String, Vec<FileRequestKey>>::new();
        for request in &unresolved {
            let Some(key) = distribution_file_key(request)? else {
                responses.push(ResourceResponse::FileUnavailable(request.key().clone()));
                continue;
            };
            original_files
                .entry(key.manifest_key().to_string())
                .or_default()
                .push(request.key().clone());
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
                .entry(
                    shard_index_for_key(&key, shard_bits)
                        .map_err(|error| NativeRunError::Selection(error.to_string()))?,
                )
                .or_default()
                .push(key);
        }
        let mut required = BTreeMap::<String, SelectedDistributionRecord>::new();
        let mut hints = BTreeMap::<String, SelectedDistributionRecord>::new();
        let mut fallback_files = BTreeMap::<String, Vec<FileRequestKey>>::new();
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
                if let Some(fallback) = appended_tex_distribution_key(&original)? {
                    fallback_files
                        .entry(fallback.manifest_key().to_string())
                        .or_default()
                        .push(original);
                } else {
                    responses.push(ResourceResponse::FileUnavailable(original));
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
            responses.extend(originals.into_iter().map(ResourceResponse::FileUnavailable));
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
                        if !required.contains_key(&key)
                            && let Some(Some(entry)) = selected.get(&key)
                        {
                            hints.insert(key, entry.clone());
                        }
                    }
                }
                Err(NativeRunError::Cancelled) => return Err(NativeRunError::Cancelled),
                Err(_) => {}
            }
        }
        let required_fetches = required
            .iter()
            .map(|(key, entry)| FetchRequest {
                request_key: key.clone(),
                object: entry.object.clone(),
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
            let Some(next_bytes) = hinted_bytes.checked_add(entry.object.bytes) else {
                break;
            };
            if next_files > limits.resolved_files || next_bytes > limits.cached_file_bytes as u64 {
                continue;
            }
            hinted_files = next_files;
            hinted_bytes = next_bytes;
            hint_fetches.push(FetchRequest {
                request_key: key.clone(),
                object: entry.object.clone(),
                max_bytes: limits.one_file_bytes as u64,
            });
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
                Err(_) if fetch_requests.len() > required_fetches.len() => self.fetch_objects(
                    &objects_base_url,
                    &required_fetches,
                    cancellation,
                    telemetry,
                )?,
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
            let keys = original_files
                .remove(&manifest_key)
                .expect("original file request");
            let hash_started = Instant::now();
            let expected_digest = FileContentId::for_bytes(&data);
            telemetry.content_hash_time = telemetry
                .content_hash_time
                .saturating_add(hash_started.elapsed());
            for key in keys {
                responses.push(ResourceResponse::File(ResolvedFile {
                    request: key,
                    expected_digest: Some(expected_digest),
                    virtual_path: entry.virtual_path.clone(),
                    bytes: data.clone(),
                }));
            }
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
                    }),
            )
        })
        .collect()
}

fn emit_failed_distribution_telemetry(telemetry: ResolverTelemetry) {
    if env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
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
