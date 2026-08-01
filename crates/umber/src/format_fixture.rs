//! Generic, guarded construction and loaded execution of generated formats.

use std::cell::Cell;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tex_command::{CommandObserver, FontResource, RegisteredSourceKind, SourceRegistration};
use tex_exec::{CanonicalResourceNeed, CheckpointSink};
use tex_observe::{
    DetachedEvidence, LiveSessionTranslator, LiveSource, decode_detached_evidence,
    encode_detached_evidence,
};
use tex_oracle::SchemaVersion;
use tex_state::{JobClock, Universe, World};
use umber_fetch::{
    FormatCacheClock, FormatCacheError, FormatCacheIdentity, FormatCacheStore, FormatEngineMode,
    FormatFingerprint, FormatFixtureIdentity, ValidatedFormatImage,
};

use crate::{
    CanonicalEngineSession, CanonicalResourceFulfillment, CanonicalResourceHost,
    CanonicalResourceOutcome, CanonicalResourceWorld, CanonicalSessionError, EngineMode, RunResult,
};

const IDENTITY_DOMAIN: &[u8] = b"umber.loaded-format-fixture.v2\0";
const PRODUCER_CONTRACT_VERSION: u32 = 2;
const COMMAND_OBSERVATION_SCHEMA_VERSION: u32 = 1;

/// Positive cumulative limits for one format construction or loaded job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatGenerationGuards {
    pub command_fuel: u64,
    pub wall_time: Duration,
    pub resident_bytes: u64,
}

impl FormatGenerationGuards {
    pub fn validate(self) -> Result<Self, FormatFixtureError> {
        if self.command_fuel == 0 || self.wall_time.is_zero() || self.resident_bytes == 0 {
            return Err(FormatFixtureError::UnboundedGuard);
        }
        Ok(self)
    }
}

/// One immutable member of a format recipe's typed resource closure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatResource {
    Input {
        logical_name: String,
        source_kind: RegisteredSourceKind,
        bytes: Arc<[u8]>,
    },
    Tfm {
        logical_name: String,
        bytes: Arc<[u8]>,
    },
}

/// One immutable job-local resource supplied after a format is loaded.
///
/// These resources belong to the execution episode, not to the format recipe
/// or its cache identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadedFormatResource {
    Input {
        logical_name: String,
        resolved_name: String,
        source_kind: RegisteredSourceKind,
        bytes: Arc<[u8]>,
    },
    Tfm {
        logical_name: String,
        bytes: Arc<[u8]>,
    },
}

/// Complete host-independent recipe for one generated format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatRecipe {
    pub engine: EngineMode,
    /// web2c's selected `dump_name`, used by §61's terminal banner.
    pub format_name: String,
    /// The dump job name embedded by TeX82 §1328 and restored for §536's log.
    pub format_ident_name: String,
    pub construction_source_name: String,
    pub construction_source: Arc<[u8]>,
    pub resources: Vec<FormatResource>,
    pub distribution_identity: Arc<[u8]>,
    pub clock: JobClock,
    /// Driver-selected interaction for the construction episode.
    pub construction_interaction: tex_state::InteractionMode,
    /// Driver-selected TeX82 §79 diagnostic widths for construction.
    pub construction_error_context_widths: tex_state::print::ErrorContextWidths,
    pub guards: FormatGenerationGuards,
}

impl FormatRecipe {
    /// Hermetic raw TeX82 image: primitives and INITEX state, without Plain.
    #[must_use]
    pub fn raw_tex82() -> Self {
        Self {
            engine: EngineMode::Tex82,
            format_name: "raw-tex82".into(),
            format_ident_name: "raw-tex82".into(),
            construction_source_name: "raw-tex82.ini".into(),
            construction_source: Arc::from(&b"\\dump\n"[..]),
            resources: Vec::new(),
            distribution_identity: Arc::from(&b"repository-raw-tex82-v1"[..]),
            clock: JobClock {
                time: 12 * 60,
                second: 0,
                day: 1,
                month: 3,
                year: 2026,
            },
            construction_interaction: tex_state::InteractionMode::ErrorStop,
            construction_error_context_widths: tex_state::print::ErrorContextWidths::default(),
            guards: FormatGenerationGuards {
                command_fuel: 100_000,
                wall_time: Duration::from_secs(10),
                resident_bytes: 512 * 1024 * 1024,
            },
        }
    }

    /// Hermetic raw e-TeX 2.6 image: TeX82 and e-TeX primitives, without Plain.
    #[must_use]
    pub fn raw_etex26() -> Self {
        Self {
            engine: EngineMode::ETex,
            format_name: "raw-etex26".into(),
            format_ident_name: "raw-etex26".into(),
            construction_source_name: "raw-etex26.ini".into(),
            construction_source: Arc::from(&b"\\dump\n"[..]),
            resources: Vec::new(),
            distribution_identity: Arc::from(&b"repository-raw-etex26-v1"[..]),
            clock: JobClock {
                time: 12 * 60,
                second: 0,
                day: 1,
                month: 3,
                year: 2026,
            },
            construction_interaction: tex_state::InteractionMode::ErrorStop,
            construction_error_context_widths: tex_state::print::ErrorContextWidths::default(),
            guards: FormatGenerationGuards {
                command_fuel: 100_000,
                wall_time: Duration::from_secs(10),
                resident_bytes: 512 * 1024 * 1024,
            },
        }
    }

    /// Hermetic production pdfTeX 1.40.27 image, without a macro format.
    ///
    /// Construction runs in pdfTeX INITEX mode and terminates only through
    /// this recipe-owned `\dump`.
    #[must_use]
    pub fn production_pdftex14027() -> Self {
        Self {
            engine: EngineMode::PdfTex,
            format_name: "production".into(),
            format_ident_name: "production".into(),
            construction_source_name: "production-pdftex14027.ini".into(),
            construction_source: Arc::from(&b"\\dump\n"[..]),
            resources: Vec::new(),
            distribution_identity: Arc::from(&b"repository-production-pdftex14027-v1"[..]),
            clock: JobClock {
                time: 12 * 60,
                second: 0,
                day: 1,
                month: 3,
                year: 2026,
            },
            construction_interaction: tex_state::InteractionMode::ErrorStop,
            construction_error_context_widths: tex_state::print::ErrorContextWidths::default(),
            guards: FormatGenerationGuards {
                command_fuel: 100_000,
                wall_time: Duration::from_secs(10),
                resident_bytes: 512 * 1024 * 1024,
            },
        }
    }

    pub fn identity(&self) -> Result<FormatCacheIdentity, FormatFixtureError> {
        self.guards.validate()?;
        let profile = self.engine.command_profile();
        let mut registry = Universe::with_world(World::memory_with_clock(self.clock));
        self.engine.prepare_initex(&mut registry);
        let registry_state = registry.snapshot().state_hash();
        let semantic = framed_hash(&[
            &tex_state::CHECKPOINT_STATE_HASH_SCHEMA_VERSION.to_le_bytes(),
            &COMMAND_OBSERVATION_SCHEMA_VERSION.to_le_bytes(),
            &profile.to_stable_bytes(),
            &profile.fingerprint().get().to_le_bytes(),
            &registry_state.to_le_bytes(),
            &tex_observe::EVIDENCE_CODEC_SCHEMA.to_le_bytes(),
            &(tex_observe::MAX_EVIDENCE_EVENTS_PER_STREAM as u64).to_le_bytes(),
            &(tex_observe::MAX_EVIDENCE_EVENT_BYTES as u64).to_le_bytes(),
            &(tex_observe::MAX_EVIDENCE_STRING_BYTES as u64).to_le_bytes(),
            &(tex_observe::MAX_EVIDENCE_NESTING_DEPTH as u64).to_le_bytes(),
            &(tex_observe::MAX_EVIDENCE_BYTES as u64).to_le_bytes(),
            &[interaction_tag(self.construction_interaction)],
            &(self.construction_error_context_widths.error_line() as u64).to_le_bytes(),
            &(self.construction_error_context_widths.half_error_line() as u64).to_le_bytes(),
        ]);
        let source = framed_hash(&[
            self.construction_source_name.as_bytes(),
            &self.construction_source,
        ]);
        let resources = resource_closure_hash(&self.resources);
        let closure = framed_hash(&[&source, &resources]);
        let guards = framed_hash(&[
            &self.guards.command_fuel.to_le_bytes(),
            &self.guards.wall_time.as_nanos().to_le_bytes(),
            &self.guards.resident_bytes.to_le_bytes(),
        ]);
        let producer = framed_hash(&[
            &PRODUCER_CONTRACT_VERSION.to_le_bytes(),
            env!("CARGO_PKG_VERSION").as_bytes(),
            build_feature_contract(),
            self.format_name.as_bytes(),
            self.format_ident_name.as_bytes(),
        ]);
        Ok(FormatCacheIdentity::fixture(FormatFixtureIdentity {
            engine_mode: cache_mode(self.engine),
            distribution_snapshot: FormatFingerprint::sha256(&self.distribution_identity),
            format_closure: FormatFingerprint::new(closure),
            source_lock: FormatFingerprint::new(source),
            job_clock: FormatCacheClock {
                time: self.clock.time,
                second: self.clock.second,
                day: self.clock.day,
                month: self.clock.month,
                year: self.clock.year,
            },
            build_configuration: FormatFingerprint::sha256(build_feature_contract()),
            semantic_contract: FormatFingerprint::new(semantic),
            producer_contract: FormatFingerprint::new(producer),
            resource_closure: FormatFingerprint::new(resources),
            generation_guards: FormatFingerprint::new(guards),
        }))
    }
}

/// Validated cached bytes paired with the exact recipe that selected them.
#[derive(Clone, Debug)]
pub struct FormatFixture {
    recipe: FormatRecipe,
    image: ValidatedFormatImage,
    evidence: DetachedEvidence,
}

impl FormatFixture {
    /// Engine/profile identity authenticated by this fixture's recipe.
    #[must_use]
    pub const fn engine_mode(&self) -> EngineMode {
        self.recipe.engine
    }

    #[must_use]
    pub fn image(&self) -> &[u8] {
        self.image.as_bytes()
    }

    /// Detached construction-only canonical semantic and geometry evidence.
    #[must_use]
    pub fn construction_evidence(&self) -> &DetachedEvidence {
        &self.evidence
    }

    pub fn load(&self, world: World) -> Result<LoadedFormatFixture, FormatFixtureError> {
        let mut universe = Universe::from_format(world, self.image.as_bytes())
            .map_err(|error| FormatFixtureError::Format(error.to_string()))?;
        self.recipe.engine.install_after_format(&mut universe);
        Ok(LoadedFormatFixture {
            recipe: self.recipe.clone(),
            universe,
        })
    }
}

/// Fresh post-load aggregate. It exposes execution, never format dumping.
pub struct LoadedFormatFixture {
    recipe: FormatRecipe,
    universe: Universe,
}

pub(crate) struct LoadedRunConfiguration {
    pub guards: FormatGenerationGuards,
    pub engine_binary: tex_exec::EngineBinaryIdentity,
}

impl LoadedFormatFixture {
    /// Selects the job-local TeX interaction mode after format loading.
    ///
    /// Interaction is runtime control state and is deliberately excluded from
    /// the frozen format image.
    pub fn set_interaction_mode(&mut self, mode: tex_state::InteractionMode) {
        self.universe.set_interaction_mode(mode);
    }

    /// Selects the job-local TeX error-context widths after format loading.
    ///
    /// These widths are process/driver configuration and are deliberately
    /// excluded from the immutable format image.
    pub fn set_error_context_widths(&mut self, widths: tex_state::print::ErrorContextWidths) {
        self.universe.set_error_context_widths(widths);
    }

    pub fn run(
        self,
        source_name: &str,
        source: Arc<[u8]>,
        resources: &[LoadedFormatResource],
        observer: &mut dyn CommandObserver,
    ) -> Result<LoadedFormatRun, FormatFixtureError> {
        let guards = self.recipe.guards;
        let engine_binary = self.recipe.engine.binary_identity();
        self.run_configured(
            source_name,
            RegisteredSourceKind::Generated,
            source,
            resources,
            LoadedRunConfiguration {
                guards,
                engine_binary,
            },
            observer,
        )
    }

    pub(crate) fn run_configured(
        mut self,
        source_name: &str,
        source_kind: RegisteredSourceKind,
        source: Arc<[u8]>,
        resources: &[LoadedFormatResource],
        config: LoadedRunConfiguration,
        observer: &mut dyn CommandObserver,
    ) -> Result<LoadedFormatRun, FormatFixtureError> {
        let guards = config.guards.validate()?;
        let mut session =
            CanonicalEngineSession::new(&mut self.universe, self.recipe.engine.command_profile());
        session.set_preloaded_format(tex_exec::PreloadedFormat {
            dump_name: self.recipe.format_name.clone(),
            format_name: self.recipe.format_ident_name.clone(),
            year: self.recipe.clock.year,
            month: self.recipe.clock.month,
            day: self.recipe.clock.day,
        });
        session.set_engine_binary(config.engine_binary);
        session.set_fuel_limit(guards.command_fuel)?;
        let root_source = session.register_retained_root(
            source_name,
            tex_command::SourceRegistration::new(source_kind, source)
                .with_name(format!("./{source_name}")),
        )?;
        let checkpoints = GuardCheckpoints::new(guards)?;
        let mut checkpoint_sink = &checkpoints;
        let result = session.run_with_observer(
            &mut LoadedResourceHost::new(resources, &self.recipe.resources),
            &mut checkpoint_sink,
            observer,
        );
        let result = finish_guarded_run(result, &checkpoints)?;
        Ok(LoadedFormatRun {
            result,
            universe: self.universe,
            root_source,
        })
    }
}

struct LoadedResourceHost<'a> {
    job_resources: &'a [LoadedFormatResource],
    format_resources: &'a [FormatResource],
}

impl<'a> LoadedResourceHost<'a> {
    fn new(
        job_resources: &'a [LoadedFormatResource],
        format_resources: &'a [FormatResource],
    ) -> Self {
        Self {
            job_resources,
            format_resources,
        }
    }
}

impl CanonicalResourceHost for LoadedResourceHost<'_> {
    fn fulfill(
        &mut self,
        world: &mut CanonicalResourceWorld<'_>,
        need: &CanonicalResourceNeed,
    ) -> CanonicalResourceOutcome {
        match need {
            CanonicalResourceNeed::Input { name } => self
                .job_resources
                .iter()
                .find_map(|resource| match resource {
                    LoadedFormatResource::Input {
                        logical_name,
                        resolved_name,
                        source_kind,
                        bytes,
                    } if logical_name == name => Some(CanonicalResourceOutcome::Fulfilled(
                        CanonicalResourceFulfillment::Input {
                            name: logical_name.clone(),
                            source: SourceRegistration::new(*source_kind, Arc::clone(bytes))
                                .with_name(resolved_name.clone()),
                        },
                    )),
                    _ => None,
                })
                .or_else(|| {
                    self.format_resources
                        .iter()
                        .find_map(|resource| match resource {
                            FormatResource::Input {
                                logical_name,
                                source_kind,
                                bytes,
                            } if logical_name == name => Some(CanonicalResourceOutcome::Fulfilled(
                                CanonicalResourceFulfillment::Input {
                                    name: logical_name.clone(),
                                    source: SourceRegistration::new(
                                        *source_kind,
                                        Arc::clone(bytes),
                                    )
                                    .with_name(format!("./{logical_name}")),
                                },
                            )),
                            _ => None,
                        })
                })
                .unwrap_or(CanonicalResourceOutcome::Unavailable),
            CanonicalResourceNeed::Font { request } => self
                .job_resources
                .iter()
                .find_map(|resource| match resource {
                    LoadedFormatResource::Tfm {
                        logical_name,
                        bytes,
                    } if Path::new(logical_name).file_stem()
                        == Some(std::ffi::OsStr::new(&request.name)) =>
                    {
                        let content = world
                            .register_selected_file(logical_name, Arc::clone(bytes))
                            .ok()?;
                        Some(CanonicalResourceOutcome::Fulfilled(
                            CanonicalResourceFulfillment::Font {
                                request: request.clone(),
                                resource: Box::new(FontResource::Tfm {
                                    metrics: content,
                                    opentype: None,
                                }),
                            },
                        ))
                    }
                    _ => None,
                })
                .or_else(|| {
                    self.format_resources
                        .iter()
                        .find_map(|resource| match resource {
                            FormatResource::Tfm {
                                logical_name,
                                bytes,
                            } if Path::new(logical_name).file_stem()
                                == Some(std::ffi::OsStr::new(&request.name)) =>
                            {
                                let content = world
                                    .register_selected_file(logical_name, Arc::clone(bytes))
                                    .ok()?;
                                Some(CanonicalResourceOutcome::Fulfilled(
                                    CanonicalResourceFulfillment::Font {
                                        request: request.clone(),
                                        resource: Box::new(FontResource::Tfm {
                                            metrics: content,
                                            opentype: None,
                                        }),
                                    },
                                ))
                            }
                            _ => None,
                        })
                })
                .unwrap_or(CanonicalResourceOutcome::Unavailable),
            CanonicalResourceNeed::PdfImage { .. } => CanonicalResourceOutcome::Unavailable,
        }
    }
}

pub struct LoadedFormatRun {
    pub result: RunResult,
    pub universe: Universe,
    /// Provenance identity assigned to this job's retained root source.
    pub root_source: tex_state::SourceId,
}

/// Ensures one recipe image exists in the validated content-addressed cache.
pub fn ensure_format(
    cache: &FormatCacheStore,
    recipe: &FormatRecipe,
    launcher: &crate::FormatWorkerLauncher,
) -> Result<FormatFixture, FormatFixtureError> {
    let identity = recipe.identity()?;
    let entry = cache.ensure_entry::<FormatFixtureError>(
        &identity,
        |bytes| decode_detached_evidence(bytes).map(|_| ()),
        || {
            let result = crate::format_worker::construct(Some(launcher), recipe)?;
            Ok((result.image, result.evidence))
        },
    )?;
    Ok(FormatFixture {
        recipe: recipe.clone(),
        image: entry.image().clone(),
        evidence: decode_detached_evidence(entry.evidence())
            .map_err(FormatFixtureError::Evidence)?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConstructionResult {
    pub image: Vec<u8>,
    pub evidence: Vec<u8>,
}

pub(crate) fn construct_format_in_worker(
    recipe: &FormatRecipe,
) -> Result<ConstructionResult, FormatFixtureError> {
    recipe.guards.validate()?;
    let mut universe = Universe::with_world(World::memory_with_clock(recipe.clock));
    recipe.engine.prepare_initex(&mut universe);
    universe.set_interaction_mode(recipe.construction_interaction);
    universe.set_error_context_widths(recipe.construction_error_context_widths);
    let mut session =
        CanonicalEngineSession::prepared_initex(&mut universe, recipe.engine.command_profile());
    session.set_fuel_limit(recipe.guards.command_fuel)?;
    let root = session.register_authored_root(
        &recipe.construction_source_name,
        Arc::clone(&recipe.construction_source),
    )?;
    let mut observer = LiveSessionTranslator::for_root(
        SchemaVersion::V2,
        "terminal",
        LiveSource {
            name: recipe.construction_source_name.clone(),
            source: root,
            bytes: Arc::clone(&recipe.construction_source),
        },
    );
    let guards = GuardCheckpoints::new(recipe.guards)?;
    let mut checkpoints = &guards;
    let result = session.run_with_observer(
        &mut RecipeResourceHost::new(&recipe.resources),
        &mut checkpoints,
        &mut observer,
    );
    let result = finish_guarded_run(result, &guards)?;
    if !result.dumped_format {
        return Err(FormatFixtureError::ConstructionDidNotDump);
    }
    let image = session
        .stores()
        .dump_format()
        .map_err(|error| FormatFixtureError::Format(error.to_string()))?;
    let evidence = encode_detached_evidence(&observer.finalize_detached_evidence())
        .map_err(FormatFixtureError::Evidence)?;
    Ok(ConstructionResult { image, evidence })
}

#[cfg(test)]
struct NoCheckpoints;
#[cfg(test)]
impl CheckpointSink for NoCheckpoints {
    fn wants_checkpoint(&self, _boundary: tex_exec::EngineBoundary) -> bool {
        false
    }

    fn checkpoint(&mut self, _checkpoint: tex_exec::EngineCheckpoint) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuardFailure {
    WallTime,
    ResidentSet,
}

struct GuardCheckpoints {
    started: Instant,
    guards: FormatGenerationGuards,
    failure: Cell<Option<GuardFailure>>,
}

impl GuardCheckpoints {
    #[allow(
        clippy::disallowed_methods,
        reason = "native format-fixture guard measures host wall time independently of TeX's fixed job clock"
    )]
    fn new(guards: FormatGenerationGuards) -> Result<Self, FormatFixtureError> {
        current_resident_bytes()?;
        Ok(Self {
            started: Instant::now(),
            guards,
            failure: Cell::new(None),
        })
    }
}

impl CheckpointSink for &GuardCheckpoints {
    fn wants_checkpoint(&self, _boundary: tex_exec::EngineBoundary) -> bool {
        false
    }

    fn stop_requested(&self) -> bool {
        let failure = if self.started.elapsed() > self.guards.wall_time {
            Some(GuardFailure::WallTime)
        } else {
            match current_resident_bytes() {
                Ok(bytes) if bytes > self.guards.resident_bytes => Some(GuardFailure::ResidentSet),
                Ok(_) => None,
                Err(_) => Some(GuardFailure::ResidentSet),
            }
        };
        self.failure.set(failure);
        failure.is_some()
    }

    fn checkpoint(&mut self, _checkpoint: tex_exec::EngineCheckpoint) {}
}

fn finish_guarded_run<T>(
    result: Result<T, CanonicalSessionError>,
    guards: &GuardCheckpoints,
) -> Result<T, FormatFixtureError> {
    match (result, guards.failure.get()) {
        (Err(CanonicalSessionError::CooperativeStopRequested), Some(GuardFailure::WallTime)) => {
            Err(FormatFixtureError::WallTimeExceeded)
        }
        (Err(CanonicalSessionError::CooperativeStopRequested), Some(GuardFailure::ResidentSet)) => {
            Err(FormatFixtureError::ResidentSetExceeded)
        }
        (result, _) => result.map_err(FormatFixtureError::from),
    }
}

struct RecipeResourceHost<'a> {
    resources: &'a [FormatResource],
}

impl<'a> RecipeResourceHost<'a> {
    fn new(resources: &'a [FormatResource]) -> Self {
        Self { resources }
    }
}

impl CanonicalResourceHost for RecipeResourceHost<'_> {
    fn fulfill(
        &mut self,
        world: &mut CanonicalResourceWorld<'_>,
        need: &CanonicalResourceNeed,
    ) -> CanonicalResourceOutcome {
        match need {
            CanonicalResourceNeed::Input { name } => self
                .resources
                .iter()
                .find_map(|resource| match resource {
                    FormatResource::Input {
                        logical_name,
                        source_kind,
                        bytes,
                    } if logical_name == name => Some(CanonicalResourceOutcome::Fulfilled(
                        CanonicalResourceFulfillment::input(
                            logical_name,
                            *source_kind,
                            Arc::clone(bytes),
                        ),
                    )),
                    _ => None,
                })
                .unwrap_or(CanonicalResourceOutcome::Unavailable),
            CanonicalResourceNeed::Font { request } => self
                .resources
                .iter()
                .find_map(|resource| match resource {
                    FormatResource::Tfm {
                        logical_name,
                        bytes,
                    } if Path::new(logical_name).file_stem()
                        == Some(std::ffi::OsStr::new(&request.name)) =>
                    {
                        let content = world
                            .register_selected_file(logical_name, Arc::clone(bytes))
                            .ok()?;
                        Some(CanonicalResourceOutcome::Fulfilled(
                            CanonicalResourceFulfillment::Font {
                                request: request.clone(),
                                resource: Box::new(FontResource::Tfm {
                                    metrics: content,
                                    opentype: None,
                                }),
                            },
                        ))
                    }
                    _ => None,
                })
                .unwrap_or(CanonicalResourceOutcome::Unavailable),
            CanonicalResourceNeed::PdfImage { .. } => CanonicalResourceOutcome::Unavailable,
        }
    }
}

#[cfg(target_os = "linux")]
fn current_resident_bytes() -> Result<u64, FormatFixtureError> {
    crate::linux_rss::resident_bytes(Path::new("/proc/self/statm"))
        .ok_or(FormatFixtureError::ResidentSetUnsupported)
}

#[cfg(not(target_os = "linux"))]
fn current_resident_bytes() -> Result<u64, FormatFixtureError> {
    Err(FormatFixtureError::ResidentSetUnsupported)
}

fn resource_closure_hash(resources: &[FormatResource]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(IDENTITY_DOMAIN);
    hasher.update((resources.len() as u64).to_le_bytes());
    for resource in resources {
        match resource {
            FormatResource::Input {
                logical_name,
                source_kind,
                bytes,
            } => {
                hasher.update([1, source_kind_tag(*source_kind)]);
                hash_field(&mut hasher, logical_name.as_bytes());
                hash_field(&mut hasher, bytes);
            }
            FormatResource::Tfm {
                logical_name,
                bytes,
            } => {
                hasher.update([2]);
                hash_field(&mut hasher, logical_name.as_bytes());
                hash_field(&mut hasher, bytes);
            }
        }
    }
    hasher.finalize().into()
}

const fn source_kind_tag(kind: RegisteredSourceKind) -> u8 {
    match kind {
        RegisteredSourceKind::World => 1,
        RegisteredSourceKind::Generated => 2,
        RegisteredSourceKind::EditorFragment => 3,
        RegisteredSourceKind::ReadLine => 4,
    }
}

const fn interaction_tag(mode: tex_state::InteractionMode) -> u8 {
    match mode {
        tex_state::InteractionMode::Batch => 0,
        tex_state::InteractionMode::Nonstop => 1,
        tex_state::InteractionMode::Scroll => 2,
        tex_state::InteractionMode::ErrorStop => 3,
    }
}

fn framed_hash(fields: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(IDENTITY_DOMAIN);
    hasher.update((fields.len() as u64).to_le_bytes());
    for field in fields {
        hash_field(&mut hasher, field);
    }
    hasher.finalize().into()
}

const fn build_feature_contract() -> &'static [u8] {
    if cfg!(feature = "shadow") {
        b"umber-format-producer-v1;shadow"
    } else {
        b"umber-format-producer-v1;default"
    }
}

fn hash_field(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_le_bytes());
    hasher.update(field);
}

const fn cache_mode(engine: EngineMode) -> FormatEngineMode {
    match engine {
        EngineMode::Tex82 => FormatEngineMode::Tex82,
        EngineMode::ETex => FormatEngineMode::ETex,
        EngineMode::PdfTex => FormatEngineMode::PdfTex,
        EngineMode::Latex => FormatEngineMode::Latex,
        EngineMode::PdfLatex => FormatEngineMode::PdfLatex,
    }
}

#[derive(Debug)]
pub enum FormatFixtureError {
    UnboundedGuard,
    WallTimeExceeded,
    ResidentSetExceeded,
    ResidentSetUnsupported,
    ConstructionDidNotDump,
    Format(String),
    Evidence(String),
    Cache(FormatCacheError),
    Session(Box<CanonicalSessionError>),
    Fuel(tex_command::CommandFuelLimitError),
    WorkerSpawn(String),
    WorkerBootstrapUnregistered,
    WorkerProtocol(String),
    WorkerIdentityMismatch,
    WorkerCrashed(Option<i32>, String),
    Worker(String),
    ProviderProfileMismatch {
        expected: EngineMode,
        actual: EngineMode,
    },
    ProviderBinaryMismatch {
        engine: EngineMode,
        binary: tex_exec::EngineBinaryIdentity,
    },
    ProviderBackendMismatch {
        engine: EngineMode,
        backend: crate::OutputCapability,
    },
    World(String),
}

impl fmt::Display for FormatFixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for FormatFixtureError {}

impl From<FormatCacheError> for FormatFixtureError {
    fn from(error: FormatCacheError) -> Self {
        Self::Cache(error)
    }
}

impl From<CanonicalSessionError> for FormatFixtureError {
    fn from(error: CanonicalSessionError) -> Self {
        Self::Session(Box::new(error))
    }
}

impl From<tex_command::CommandFuelLimitError> for FormatFixtureError {
    fn from(error: tex_command::CommandFuelLimitError) -> Self {
        Self::Fuel(error)
    }
}

#[cfg(test)]
mod tests;
