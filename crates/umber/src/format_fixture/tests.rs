use std::sync::{Arc, Barrier};
use std::thread;

use tempfile::TempDir;
use tex_command::{CommandObservation, CommandObserver};
use tex_state::env::banks::IntParam;
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};

use super::*;

fn ensure_format(
    cache: &FormatCacheStore,
    recipe: &FormatRecipe,
) -> Result<FormatFixture, FormatFixtureError> {
    super::ensure_format(cache, recipe, &crate::umber_format_worker_launcher())
}

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
    for resource in [
        FormatResource::Input {
            logical_name: "other.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: Arc::from(&b"\\relax"[..]),
        },
        FormatResource::Input {
            logical_name: "fixture.tex".into(),
            source_kind: RegisteredSourceKind::World,
            bytes: Arc::from(&b"\\relax"[..]),
        },
        FormatResource::Input {
            logical_name: "fixture.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: Arc::from(&b"\\end"[..]),
        },
    ] {
        let mut recipe = original.clone();
        recipe.resources.push(resource);
        mutations.push(recipe);
    }
    let mut recipe = original.clone();
    recipe.resources = vec![
        FormatResource::Tfm {
            logical_name: "a.tfm".into(),
            bytes: Arc::from(&b"a"[..]),
        },
        FormatResource::Tfm {
            logical_name: "b.tfm".into(),
            bytes: Arc::from(&b"b"[..]),
        },
    ];
    let mut reversed = recipe.clone();
    reversed.resources.reverse();
    mutations.push(recipe);
    mutations.push(reversed);
    let mut recipe = original.clone();
    recipe.distribution_identity = Arc::from(&b"other-distribution"[..]);
    mutations.push(recipe);
    for mutate in [
        |clock: &mut JobClock| clock.time += 1,
        |clock: &mut JobClock| clock.second += 1,
        |clock: &mut JobClock| clock.day += 1,
        |clock: &mut JobClock| clock.month += 1,
        |clock: &mut JobClock| clock.year += 1,
    ] {
        let mut recipe = original.clone();
        mutate(&mut recipe.clock);
        mutations.push(recipe);
    }
    let mut recipe = original.clone();
    recipe.guards.command_fuel += 1;
    mutations.push(recipe);
    let mut recipe = original.clone();
    recipe.guards.wall_time += Duration::from_nanos(1);
    mutations.push(recipe);
    let mut recipe = original.clone();
    recipe.guards.resident_bytes += 1;
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
fn raw_etex_cache_reuse_reloads_exact_live_registry_into_fresh_runtime_state() {
    let cache_root = TempDir::new().expect("cache");
    let cache = FormatCacheStore::new(cache_root.path());
    let recipe = FormatRecipe::raw_etex26();
    let first = ensure_format(&cache, &recipe).expect("raw e-TeX construction");
    let second = ensure_format(&cache, &recipe).expect("raw e-TeX cache hit");
    assert_eq!(first.image(), second.image());

    let loaded = second.load(test_world()).expect("raw e-TeX load");
    assert_eq!(
        loaded.universe.primitive_meaning("unexpanded"),
        Some(Meaning::ExpandablePrimitive(
            ExpandablePrimitive::Unexpanded
        ))
    );
    assert_eq!(
        loaded.universe.primitive_meaning("showtokens"),
        Some(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::ShowTokens
        ))
    );
    assert_eq!(loaded.universe.primitive_meaning("pdfprimitive"), None);
    assert!(loaded.universe.world().effect_records().is_empty());
    assert!(loaded.universe.world().artifact_commits().is_empty());
    let provenance = loaded.universe.provenance_stats();
    assert_eq!(provenance.origin_records(), 0);
    assert_eq!(provenance.source_regions(), 0);
    assert_eq!(loaded.universe.int_param(IntParam::YEAR), 2031);
}

#[test]
fn construction_failure_publishes_no_entry() {
    let cache_root = TempDir::new().expect("cache");
    let cache = FormatCacheStore::new(cache_root.path());
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.construction_source = Arc::from(&b"\\end\n"[..]);
    assert!(matches!(
        ensure_format(&cache, &recipe),
        Err(FormatFixtureError::Worker(_))
    ));
    assert!(
        cache
            .load(&recipe.identity().expect("identity"))
            .expect("cache remains readable")
            .is_none()
    );
}

#[test]
fn construction_fuel_interrupts_a_cyclic_macro() {
    let cache_root = TempDir::new().expect("cache");
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.guards.command_fuel = 32;
    recipe.construction_source = Arc::from(&b"\\def\\loop{\\loop}\\loop"[..]);
    assert!(matches!(
        ensure_format(&FormatCacheStore::new(cache_root.path()), &recipe),
        Err(FormatFixtureError::Worker(_))
    ));
}

#[test]
fn construction_wall_guard_interrupts_during_execution() {
    let cache_root = TempDir::new().expect("cache");
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.guards.wall_time = Duration::from_nanos(1);
    assert!(matches!(
        ensure_format(&FormatCacheStore::new(cache_root.path()), &recipe),
        Err(FormatFixtureError::WallTimeExceeded)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn construction_rss_guard_interrupts_without_allocating() {
    let cache_root = TempDir::new().expect("cache");
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.construction_source = Arc::from(&b"\\def\\loop{\\loop}\\loop"[..]);
    recipe.guards.resident_bytes = 1;
    let result = ensure_format(&FormatCacheStore::new(cache_root.path()), &recipe);
    assert!(
        matches!(result, Err(FormatFixtureError::ResidentSetExceeded)),
        "unexpected low-RSS construction result: {result:?}"
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
            &[],
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
            &[],
            &mut loaded_observations,
        )
        .expect("loaded run");

    let (fresh, fresh_observations) =
        run_explicit_fresh_compatibility(&recipe, "equivalence.tex", source);
    assert_eq!(loaded.universe.count(0), fresh.count(0));
    assert_eq!(loaded_observations.0, fresh_observations);
}

#[test]
fn raw_etex_fresh_and_loaded_match_extension_state_and_observations() {
    // e-TeX manual §3.6 makes \tracingassigns extension-owned mutable state;
    // matching it and the canonical observations exercises both restored
    // unexpandable assignment and expandable \numexpr meanings.
    let source = Arc::from(&b"\\tracingassigns=7\\count0=\\numexpr 2+3\\relax\\end\n"[..]);
    let recipe = FormatRecipe::raw_etex26();
    let cache_root = TempDir::new().expect("cache");
    let fixture =
        ensure_format(&FormatCacheStore::new(cache_root.path()), &recipe).expect("raw e-TeX");
    let mut loaded_observations = Recorder::default();
    let loaded = fixture
        .load(test_world())
        .expect("load")
        .run(
            "etex-equivalence.tex",
            Arc::clone(&source),
            &[],
            &mut loaded_observations,
        )
        .expect("loaded e-TeX run");

    let (fresh, fresh_observations) =
        run_explicit_fresh_compatibility(&recipe, "etex-equivalence.tex", source);
    assert_eq!(loaded.universe.count(0), 5);
    assert_eq!(loaded.universe.count(0), fresh.count(0));
    assert_eq!(
        loaded.universe.int_param(IntParam::TRACING_ASSIGNS),
        fresh.int_param(IntParam::TRACING_ASSIGNS)
    );
    assert_eq!(loaded_observations.0, fresh_observations);
}

#[test]
fn loaded_page_job_reports_exact_serialized_dvi_length() {
    let recipe = FormatRecipe::raw_tex82();
    let cache_root = TempDir::new().expect("cache");
    let fixture =
        ensure_format(&FormatCacheStore::new(cache_root.path()), &recipe).expect("raw format");
    let mut observations = Recorder::default();
    let run = fixture
        .load(test_world())
        .expect("load")
        .run(
            "page.tex",
            Arc::from(&br"\catcode`\{=1 \catcode`\}=2 \shipout\hbox{}\end"[..]),
            &[],
            &mut observations,
        )
        .expect("loaded page run");

    let dvi = crate::dvi_from_page_plans(&run.result.dvi_pages).expect("DVI serializes");
    assert_eq!(run.result.dvi_pages.len(), 1);
    assert!(
        run.result.terminal_text.contains(&format!(
            "Output written on page.dvi (1 page, {} bytes).",
            dvi.len()
        )),
        "TeX82 §642 reports the serialized DVI length rather than a placeholder"
    );
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
