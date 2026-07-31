//! Generic, guarded construction and loaded execution of generated formats.

use std::cell::Cell;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tex_command::{CommandObserver, FontResource, RegisteredSourceKind};
use tex_exec::{CanonicalResourceNeed, CheckpointSink};
use tex_state::{JobClock, Universe, World};
use umber_fetch::{
    FormatCacheClock, FormatCacheError, FormatCacheIdentity, FormatCacheStore, FormatEngineMode,
    FormatFingerprint, FormatFixtureIdentity, ValidatedFormatImage,
};

use crate::{
    CanonicalEngineSession, CanonicalResourceFulfillment, CanonicalResourceHost,
    CanonicalResourceOutcome, CanonicalResourceWorld, CanonicalSessionError, EngineMode, RunResult,
};

const IDENTITY_DOMAIN: &[u8] = b"umber.loaded-format-fixture.v1\0";
const PRODUCER_CONTRACT_VERSION: u32 = 1;
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

/// Complete host-independent recipe for one generated format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatRecipe {
    pub engine: EngineMode,
    pub format_name: String,
    pub construction_source_name: String,
    pub construction_source: Arc<[u8]>,
    pub resources: Vec<FormatResource>,
    pub distribution_identity: Arc<[u8]>,
    pub clock: JobClock,
    pub guards: FormatGenerationGuards,
}

impl FormatRecipe {
    /// Hermetic raw TeX82 image: primitives and INITEX state, without Plain.
    #[must_use]
    pub fn raw_tex82() -> Self {
        Self {
            engine: EngineMode::Tex82,
            format_name: "raw-tex82".into(),
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
}

impl FormatFixture {
    #[must_use]
    pub fn image(&self) -> &[u8] {
        self.image.as_bytes()
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

impl LoadedFormatFixture {
    pub fn run(
        mut self,
        source_name: &str,
        source: Arc<[u8]>,
        observer: &mut dyn CommandObserver,
    ) -> Result<LoadedFormatRun, FormatFixtureError> {
        let mut session =
            CanonicalEngineSession::new(&mut self.universe, self.recipe.engine.command_profile());
        session.set_preloaded_format(tex_exec::PreloadedFormat {
            name: self.recipe.format_name.clone(),
            year: self.recipe.clock.year,
            month: self.recipe.clock.month,
            day: self.recipe.clock.day,
        });
        session.set_fuel_limit(self.recipe.guards.command_fuel)?;
        session.register_retained_root(
            source_name,
            tex_command::SourceRegistration::new(RegisteredSourceKind::Generated, source)
                .with_name(format!("./{source_name}")),
        )?;
        let guards = GuardCheckpoints::new(self.recipe.guards)?;
        let mut checkpoints = &guards;
        let result = session.run_with_observer(
            &mut RecipeResourceHost::new(&self.recipe.resources),
            &mut checkpoints,
            observer,
        );
        let result = finish_guarded_run(result, &guards)?;
        Ok(LoadedFormatRun {
            result,
            universe: self.universe,
        })
    }
}

pub struct LoadedFormatRun {
    pub result: RunResult,
    pub universe: Universe,
}

/// Ensures one recipe image exists in the validated content-addressed cache.
pub fn ensure_format(
    cache: &FormatCacheStore,
    recipe: &FormatRecipe,
) -> Result<FormatFixture, FormatFixtureError> {
    let identity = recipe.identity()?;
    if let Some(image) = cache.load(&identity)? {
        return Ok(FormatFixture {
            recipe: recipe.clone(),
            image,
        });
    }
    let image = crate::format_worker::construct(recipe)?;
    cache.store(&identity, &image)?;
    let image = cache
        .load(&identity)?
        .ok_or(FormatFixtureError::PublishedEntryMissing)?;
    Ok(FormatFixture {
        recipe: recipe.clone(),
        image,
    })
}

pub(crate) fn construct_format_in_worker(
    recipe: &FormatRecipe,
) -> Result<Vec<u8>, FormatFixtureError> {
    recipe.guards.validate()?;
    let mut universe = Universe::with_world(World::memory_with_clock(recipe.clock));
    recipe.engine.prepare_initex(&mut universe);
    let mut session =
        CanonicalEngineSession::prepared_initex(&mut universe, recipe.engine.command_profile());
    session.set_fuel_limit(recipe.guards.command_fuel)?;
    session.register_authored_root(
        &recipe.construction_source_name,
        Arc::clone(&recipe.construction_source),
    )?;
    let guards = GuardCheckpoints::new(recipe.guards)?;
    let mut checkpoints = &guards;
    let result = session.run(
        &mut RecipeResourceHost::new(&recipe.resources),
        &mut checkpoints,
    );
    let result = finish_guarded_run(result, &guards)?;
    if !result.dumped_format {
        return Err(FormatFixtureError::ConstructionDidNotDump);
    }
    session
        .stores()
        .dump_format()
        .map_err(|error| FormatFixtureError::Format(error.to_string()))
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
    #[allow(
        clippy::disallowed_methods,
        reason = "native format-fixture host policy reads the process RSS counter"
    )]
    let statm = std::fs::read_to_string("/proc/self/statm")
        .map_err(|_| FormatFixtureError::ResidentSetUnsupported)?;
    let pages = statm
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(FormatFixtureError::ResidentSetUnsupported)?;
    Ok(pages.saturating_mul(4096))
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
    PublishedEntryMissing,
    Format(String),
    Cache(FormatCacheError),
    Session(Box<CanonicalSessionError>),
    Fuel(tex_command::CommandFuelLimitError),
    WorkerSpawn(String),
    WorkerProtocol(String),
    WorkerIdentityMismatch,
    WorkerCrashed(Option<i32>, String),
    Worker(String),
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
