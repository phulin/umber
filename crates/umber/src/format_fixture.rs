//! Generic, guarded construction and loaded execution of generated formats.

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
    FormatFingerprint, ValidatedFormatImage,
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
    pub build_configuration: Arc<[u8]>,
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
            build_configuration: Arc::from(&b"raw-tex82;canonical-session"[..]),
        }
    }

    pub fn identity(&self) -> Result<FormatCacheIdentity, FormatFixtureError> {
        self.guards.validate()?;
        let profile = self.engine.command_profile();
        let semantic = framed_hash(&[
            &tex_state::CHECKPOINT_STATE_HASH_SCHEMA_VERSION.to_le_bytes(),
            &COMMAND_OBSERVATION_SCHEMA_VERSION.to_le_bytes(),
            &profile.to_stable_bytes(),
            &profile.fingerprint().get().to_le_bytes(),
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
            self.format_name.as_bytes(),
        ]);
        Ok(FormatCacheIdentity::fixture(
            cache_mode(self.engine),
            FormatFingerprint::sha256(&self.distribution_identity),
            FormatFingerprint::new(closure),
            FormatFingerprint::new(source),
            FormatCacheClock {
                time: self.clock.time,
                second: self.clock.second,
                day: self.clock.day,
                month: self.clock.month,
                year: self.clock.year,
            },
            FormatFingerprint::sha256(&self.build_configuration),
            FormatFingerprint::new(semantic),
            FormatFingerprint::new(producer),
            FormatFingerprint::new(resources),
            FormatFingerprint::new(guards),
        ))
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
        session.set_fuel_limit(self.recipe.guards.command_fuel)?;
        session.register_authored_root(source_name, source)?;
        let started = Instant::now();
        let result = session.run_with_observer(
            &mut RecipeResourceHost::new(&self.recipe.resources),
            &mut NoCheckpoints,
            observer,
        )?;
        enforce_host_guards(started, self.recipe.guards)?;
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
    let image = construct_format(recipe)?;
    cache.store(&identity, &image)?;
    let image = cache
        .load(&identity)?
        .ok_or(FormatFixtureError::PublishedEntryMissing)?;
    Ok(FormatFixture {
        recipe: recipe.clone(),
        image,
    })
}

fn construct_format(recipe: &FormatRecipe) -> Result<Vec<u8>, FormatFixtureError> {
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
    let started = Instant::now();
    let result = session.run(
        &mut RecipeResourceHost::new(&recipe.resources),
        &mut NoCheckpoints,
    )?;
    enforce_host_guards(started, recipe.guards)?;
    if !result.dumped_format {
        return Err(FormatFixtureError::ConstructionDidNotDump);
    }
    session
        .stores()
        .dump_format()
        .map_err(|error| FormatFixtureError::Format(error.to_string()))
}

struct NoCheckpoints;

impl CheckpointSink for NoCheckpoints {
    fn wants_checkpoint(&self, _boundary: tex_exec::EngineBoundary) -> bool {
        false
    }

    fn checkpoint(&mut self, _checkpoint: tex_exec::EngineCheckpoint) {}
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
                            source_kind.clone(),
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

fn enforce_host_guards(
    started: Instant,
    guards: FormatGenerationGuards,
) -> Result<(), FormatFixtureError> {
    if started.elapsed() > guards.wall_time {
        return Err(FormatFixtureError::WallTimeExceeded);
    }
    if current_resident_bytes().is_some_and(|bytes| bytes > guards.resident_bytes) {
        return Err(FormatFixtureError::ResidentSetExceeded);
    }
    Ok(())
}

fn current_resident_bytes() -> Option<u64> {
    #[allow(
        clippy::disallowed_methods,
        reason = "native format-fixture host policy reads the process RSS counter"
    )]
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(pages.saturating_mul(4096))
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
    ConstructionDidNotDump,
    PublishedEntryMissing,
    Format(String),
    Cache(FormatCacheError),
    Session(CanonicalSessionError),
    Fuel(tex_command::CommandFuelLimitError),
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
        Self::Session(error)
    }
}

impl From<tex_command::CommandFuelLimitError> for FormatFixtureError {
    fn from(error: tex_command::CommandFuelLimitError) -> Self {
        Self::Fuel(error)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use tempfile::TempDir;
    use tex_command::{CommandObservation, CommandObserver};
    use tex_state::env::banks::IntParam;

    use super::*;

    #[derive(Default)]
    struct Recorder(Vec<CommandObservation>);

    impl CommandObserver for Recorder {
        fn committed(&mut self, observation: CommandObservation) {
            self.0.push(observation);
        }
    }

    fn test_world() -> World {
        World::memory_with_clock(JobClock {
            time: 7 * 60,
            second: 8,
            day: 9,
            month: 10,
            year: 2031,
        })
    }

    #[test]
    fn recipe_identity_invalidates_every_fixture_input_class() {
        let original = FormatRecipe::raw_tex82();
        let original_key = original.identity().expect("identity").key();
        let mut mutations = Vec::new();

        let mut recipe = original.clone();
        recipe.engine = EngineMode::ETex;
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.format_name.push_str("-other");
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.construction_source_name.push_str(".other");
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.construction_source = Arc::from(&b"\\relax\\dump\n"[..]);
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.resources.push(FormatResource::Input {
            logical_name: "fixture.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: Arc::from(&b"\\relax"[..]),
        });
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.distribution_identity = Arc::from(&b"other-distribution"[..]);
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.clock.second += 1;
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.guards.command_fuel += 1;
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.guards.wall_time += Duration::from_nanos(1);
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.guards.resident_bytes += 1;
        mutations.push(recipe);
        let mut recipe = original.clone();
        recipe.build_configuration = Arc::from(&b"other-build"[..]);
        mutations.push(recipe);

        for mutation in mutations {
            assert_ne!(
                mutation.identity().expect("mutated identity").key(),
                original_key
            );
        }
    }

    #[test]
    fn independent_raw_builds_are_byte_identical_and_cache_reload_is_fresh() {
        let first_cache = TempDir::new().expect("first cache");
        let second_cache = TempDir::new().expect("second cache");
        let recipe = FormatRecipe::raw_tex82();
        let first = ensure_format(&FormatCacheStore::new(first_cache.path()), &recipe)
            .expect("first construction");
        let second = ensure_format(&FormatCacheStore::new(second_cache.path()), &recipe)
            .expect("second construction");
        assert_eq!(first.image(), second.image());

        let loaded = first.load(test_world()).expect("fresh load");
        assert!(loaded.universe.world().effect_records().is_empty());
        let provenance = loaded.universe.provenance_stats();
        assert_eq!(provenance.origin_records(), 0);
        assert_eq!(provenance.origin_list_entries(), 0);
        assert_eq!(provenance.source_regions(), 0);
        assert_eq!(provenance.generated_source_backings(), 0);
        assert_eq!(provenance.source_map_bytes(), 0);
        assert_eq!(loaded.universe.int_param(IntParam::YEAR), 2031);
        assert_eq!(
            loaded.universe.interaction_mode(),
            tex_state::InteractionMode::ErrorStop
        );
    }

    #[test]
    fn construction_failure_publishes_no_entry() {
        let cache_root = TempDir::new().expect("cache");
        let cache = FormatCacheStore::new(cache_root.path());
        let mut recipe = FormatRecipe::raw_tex82();
        recipe.construction_source = Arc::from(&b"\\end\n"[..]);
        assert!(matches!(
            ensure_format(&cache, &recipe),
            Err(FormatFixtureError::ConstructionDidNotDump)
        ));
        assert!(
            cache
                .load(&recipe.identity().expect("identity"))
                .expect("cache remains readable")
                .is_none()
        );
    }

    #[test]
    fn concurrent_ensure_deduplicates_without_clobbering() {
        let cache_root = TempDir::new().expect("cache");
        let cache = Arc::new(FormatCacheStore::new(cache_root.path()));
        let recipe = Arc::new(FormatRecipe::raw_tex82());
        let barrier = Arc::new(Barrier::new(5));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let cache = Arc::clone(&cache);
            let recipe = Arc::clone(&recipe);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                ensure_format(&cache, &recipe)
                    .expect("concurrent ensure")
                    .image()
                    .to_vec()
            }));
        }
        barrier.wait();
        let images: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker"))
            .collect();
        assert!(images.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn representative_command_semantic_case_runs_loaded() {
        let cache_root = TempDir::new().expect("cache");
        let fixture = ensure_format(
            &FormatCacheStore::new(cache_root.path()),
            &FormatRecipe::raw_tex82(),
        )
        .expect("raw format");
        let mut observations = Recorder::default();
        let run = fixture
            .load(test_world())
            .expect("load")
            .run(
                "loaded-count-arithmetic.tex",
                Arc::from(&b"\\count0=7\\advance\\count0 by 5\\end\n"[..]),
                &mut observations,
            )
            .expect("loaded run");

        assert_eq!(run.universe.count(0), 12);
        assert!(!observations.0.is_empty());
        assert!(!run.result.dumped_format);
    }

    #[test]
    fn explicit_fresh_seam_matches_loaded_semantic_state() {
        let source = Arc::from(&b"\\count0=7\\advance\\count0 by 5\\end\n"[..]);
        let recipe = FormatRecipe::raw_tex82();
        let cache_root = TempDir::new().expect("cache");
        let fixture =
            ensure_format(&FormatCacheStore::new(cache_root.path()), &recipe).expect("raw format");
        let mut loaded_observations = Recorder::default();
        let loaded = fixture
            .load(test_world())
            .expect("load")
            .run(
                "equivalence.tex",
                Arc::clone(&source),
                &mut loaded_observations,
            )
            .expect("loaded run");

        let (fresh, fresh_observations) =
            run_explicit_fresh_compatibility(&recipe, "equivalence.tex", source);
        assert_eq!(loaded.universe.count(0), fresh.count(0));
        assert_eq!(loaded_observations.0, fresh_observations);
    }

    fn run_explicit_fresh_compatibility(
        recipe: &FormatRecipe,
        source_name: &str,
        source: Arc<[u8]>,
    ) -> (Universe, Vec<CommandObservation>) {
        let mut universe = Universe::with_world(test_world());
        recipe.engine.prepare_initex(&mut universe);
        let mut session =
            CanonicalEngineSession::prepared_initex(&mut universe, recipe.engine.command_profile());
        session
            .set_fuel_limit(recipe.guards.command_fuel)
            .expect("finite fresh fuel");
        session
            .register_authored_root(source_name, source)
            .expect("fresh root");
        let mut recorder = Recorder::default();
        let result = session
            .run_with_observer(
                &mut RecipeResourceHost::new(&recipe.resources),
                &mut NoCheckpoints,
                &mut recorder,
            )
            .expect("fresh compatibility run");
        assert!(!result.dumped_format);
        (universe, recorder.0)
    }
}
