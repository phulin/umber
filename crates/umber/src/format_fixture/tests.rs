use std::sync::atomic::{AtomicUsize, Ordering};
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

#[test]
fn dumped_format_identity_uses_the_construction_job_name_not_the_dump_selector() {
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.format_name = "selected-format".into();
    recipe.format_ident_name = "dump-job".into();
    assert_ne!(recipe.format_name, recipe.format_ident_name);
    assert_ne!(
        recipe.identity().expect("split format identity").key(),
        FormatRecipe::raw_tex82()
            .identity()
            .expect("raw format identity")
            .key()
    );
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

fn inspect_loaded<R>(
    loaded: LoadedFormatFixture,
    inspect: impl for<'id> FnOnce(&mut tex_state::Universe<tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    let LoadedFormatFixture {
        recipe,
        image,
        world,
        interaction_mode,
        error_context_widths,
        ..
    } = loaded;
    tex_state::with_materialized_format(
        crate::engine_interner_budget(),
        world,
        image.detached(),
        |universe| {
            recipe.engine.install_after_format(universe);
            if let Some(mode) = interaction_mode {
                universe.set_interaction_mode(mode);
            }
            if let Some(widths) = error_context_widths {
                universe.set_error_context_widths(widths);
            }
            inspect(universe)
        },
    )
    .expect("validated fixture materializes")
}

fn run_loaded_with_counts(
    loaded: LoadedFormatFixture,
    source_name: &str,
    source: Arc<[u8]>,
    count_registers: Vec<u16>,
    observer: &mut dyn CommandObserver,
) -> LoadedFormatRun {
    let guards = loaded.recipe.guards;
    let engine_binary = loaded.recipe.engine.binary_identity();
    loaded
        .run_configured(
            source_name,
            RegisteredSourceKind::Generated,
            source,
            &[],
            LoadedRunConfiguration {
                guards,
                engine_binary,
                startup_line: source_name.to_owned(),
                completion: tex_exec::RootCompletionPolicy::RequireTeXEnd,
                projection: LoadedFormatProjectionDemand {
                    count_registers,
                    ..LoadedFormatProjectionDemand::default()
                },
            },
            observer,
        )
        .expect("loaded run")
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
    recipe.hyphenation_exception_capacity += 1;
    mutations.push(recipe);
    let mut recipe = original.clone();
    recipe.format_name.push_str("-other");
    mutations.push(recipe);
    let mut recipe = original.clone();
    recipe.construction_source_name.push_str(".other");
    mutations.push(recipe);
    let mut recipe = original.clone();
    recipe.construction_source = b"\\relax\\dump\n".to_vec();
    mutations.push(recipe);
    let mut recipe = original.clone();
    recipe.resources.push(FormatResource::Input {
        logical_name: "fixture.tex".into(),
        source_kind: RegisteredSourceKind::Generated,
        bytes: b"\\relax".to_vec(),
    });
    mutations.push(recipe);
    for resource in [
        FormatResource::Input {
            logical_name: "other.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: b"\\relax".to_vec(),
        },
        FormatResource::Input {
            logical_name: "fixture.tex".into(),
            source_kind: RegisteredSourceKind::World,
            bytes: b"\\relax".to_vec(),
        },
        FormatResource::Input {
            logical_name: "fixture.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: b"\\end".to_vec(),
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
            bytes: b"a".to_vec(),
        },
        FormatResource::Tfm {
            logical_name: "b.tfm".into(),
            bytes: b"b".to_vec(),
        },
    ];
    let mut reversed = recipe.clone();
    reversed.resources.reverse();
    mutations.push(recipe);
    mutations.push(reversed);
    let mut recipe = original.clone();
    recipe.distribution_identity = b"other-distribution".to_vec();
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
fn producer_contract_seventeen_rejects_formats_without_hash_occupancy_observation() {
    let recipe = FormatRecipe::raw_tex82();
    let stale = producer_contract(16, &recipe.format_name, &recipe.format_ident_name);
    let current = producer_contract(
        PRODUCER_CONTRACT_VERSION,
        &recipe.format_name,
        &recipe.format_ident_name,
    );
    assert_eq!(PRODUCER_CONTRACT_VERSION, 17);
    assert_ne!(current, stale);
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
    assert_eq!(
        first.construction_evidence(),
        second.construction_evidence()
    );

    let loaded = first.load(test_world()).expect("fresh load");
    inspect_loaded(loaded, |universe| {
        assert!(universe.world().effect_records().is_empty());
        assert_eq!(universe.int_param(IntParam::YEAR), 2031);
        assert_eq!(
            universe.interaction_mode(),
            tex_state::InteractionMode::ErrorStop
        );
    });
}

#[test]
fn concurrent_cache_miss_spawns_exactly_one_real_construction_worker() {
    let cache_root = TempDir::new().expect("cache");
    let cache = Arc::new(FormatCacheStore::new(cache_root.path()));
    let recipe = Arc::new(FormatRecipe::raw_tex82());
    let spawns = Arc::new(AtomicUsize::new(0));
    let launcher =
        Arc::new(crate::umber_format_worker_launcher().with_spawn_counter(Arc::clone(&spawns)));
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for _ in 0..4 {
        let cache = Arc::clone(&cache);
        let recipe = Arc::clone(&recipe);
        let launcher = Arc::clone(&launcher);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            super::ensure_format(&cache, &recipe, &launcher).expect("concurrent ensure")
        }));
    }
    barrier.wait();
    let fixtures: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().expect("join"))
        .collect();
    assert_eq!(spawns.load(Ordering::SeqCst), 1);
    assert!(fixtures.windows(2).all(|pair| {
        pair[0].image() == pair[1].image()
            && pair[0].construction_evidence() == pair[1].construction_evidence()
    }));
}

#[test]
fn raw_etex_cache_reuse_reloads_exact_live_registry_into_fresh_runtime_state() {
    let cache_root = TempDir::new().expect("cache");
    let cache = FormatCacheStore::new(cache_root.path());
    let recipe = FormatRecipe::raw_etex26();
    let first = ensure_format(&cache, &recipe).expect("raw e-TeX construction");
    let second = ensure_format(&cache, &recipe).expect("raw e-TeX cache hit");
    assert_eq!(first.image(), second.image());
    assert_eq!(
        first.construction_evidence(),
        second.construction_evidence()
    );

    let loaded = second.load(test_world()).expect("raw e-TeX load");
    inspect_loaded(loaded, |universe| {
        assert_eq!(
            universe.primitive_meaning("unexpanded"),
            Some(Meaning::ExpandablePrimitive(
                ExpandablePrimitive::Unexpanded
            ))
        );
        assert_eq!(
            universe.primitive_meaning("showtokens"),
            Some(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::ShowTokens
            ))
        );
        assert_eq!(universe.primitive_meaning("pdfprimitive"), None);
        assert!(universe.world().effect_records().is_empty());
        assert!(universe.world().artifact_commits().is_empty());
        assert_eq!(universe.int_param(IntParam::YEAR), 2031);
    });
}

#[test]
fn production_pdftex_cache_reuse_reloads_exact_live_registry_into_fresh_runtime_state() {
    // pdfTeX 1.40.29 change file §8 installs the e-TeX extensions before the
    // pdfTeX additions. This checks one immutable loaded base and concrete
    // live witnesses from all three registry layers without admitting any
    // construction episode state.
    let cache_root = TempDir::new().expect("cache");
    let cache = FormatCacheStore::new(cache_root.path());
    let recipe = FormatRecipe::production_pdftex14029();
    let first = ensure_format(&cache, &recipe).expect("production pdfTeX construction");
    let second = ensure_format(&cache, &recipe).expect("production pdfTeX cache hit");
    assert_eq!(first.image(), second.image());
    assert_ne!(
        recipe.identity().expect("pdfTeX identity").key(),
        FormatRecipe::raw_tex82()
            .identity()
            .expect("TeX82 identity")
            .key()
    );
    assert_ne!(
        recipe.identity().expect("pdfTeX identity").key(),
        FormatRecipe::raw_etex26()
            .identity()
            .expect("e-TeX identity")
            .key()
    );

    let loaded = second.load(test_world()).expect("production pdfTeX load");
    inspect_loaded(loaded, |universe| {
        assert_eq!(
            universe.primitive_meaning("the"),
            Some(Meaning::ExpandablePrimitive(ExpandablePrimitive::The))
        );
        assert_eq!(
            universe.primitive_meaning("unexpanded"),
            Some(Meaning::ExpandablePrimitive(
                ExpandablePrimitive::Unexpanded
            ))
        );
        assert_eq!(
            universe.primitive_meaning("pdfprimitive"),
            Some(Meaning::ExpandablePrimitive(
                ExpandablePrimitive::PdfPrimitive
            ))
        );
        assert_eq!(
            universe.primitive_meaning("pdfsavepos"),
            Some(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::PdfSavePos
            ))
        );
        assert!(universe.world().effect_records().is_empty());
        assert!(universe.world().artifact_commits().is_empty());
        assert_eq!(universe.int_param(IntParam::PDF_OUTPUT), 0);
        assert_eq!(universe.fixed_pdf_output_parameters(), None);
        assert_eq!(universe.int_param(IntParam::YEAR), 2031);
        assert_eq!(
            universe.interaction_mode(),
            tex_state::InteractionMode::ErrorStop
        );
    });
}

#[test]
fn construction_failure_publishes_no_entry() {
    let cache_root = TempDir::new().expect("cache");
    let cache = FormatCacheStore::new(cache_root.path());
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.construction_source = b"\\end\n".to_vec();
    assert!(matches!(
        ensure_format(&cache, &recipe),
        Err(FormatFixtureError::Worker(_))
    ));
    assert!(
        cache
            .load_entry(&recipe.identity().expect("identity"), |bytes| {
                tex_oracle::decode_oracle_bundle(bytes).map(|_| ())
            })
            .expect("cache remains readable")
            .is_none()
    );
}

#[test]
fn construction_fuel_interrupts_a_cyclic_macro() {
    let cache_root = TempDir::new().expect("cache");
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.guards.command_fuel = 32;
    recipe.construction_source = b"\\def\\loop{\\loop}\\loop".to_vec();
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
    recipe.construction_source = b"\\end\n".to_vec();
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
    let run = run_loaded_with_counts(
        fixture.load(test_world()).expect("load"),
        "loaded-count-arithmetic.tex",
        Arc::from(&b"\\count0=7\\advance\\count0 by 5\\end\n"[..]),
        vec![0],
        &mut observations,
    );

    assert_eq!(run.projection.counts, [(0, 12)]);
    assert!(!observations.0.is_empty());
    assert!(run.result.format_dump.is_none());
}

#[test]
fn recipe_hyphenation_capacity_reaches_the_loaded_usage_report() {
    // TeX82 §§934/1308/1334 and Web2C `tex.ch` [51.1332]: the recipe's
    // process-selected `hyph_size` must reach INITEX, survive its real dump,
    // and remain the bound rendered by the loaded job.
    let cache_root = TempDir::new().expect("cache");
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.hyphenation_exception_capacity = 659;
    let fixture = ensure_format(&FormatCacheStore::new(cache_root.path()), &recipe)
        .expect("custom-capacity raw format");

    tex_state::with_materialized_format(
        crate::engine_interner_budget(),
        test_world(),
        fixture.image.detached(),
        |universe| {
            assert_eq!(
                universe
                    .command_context()
                    .expect("loaded context")
                    .detach_engine_usage_statistics()
                    .hyphenation_exception_capacity,
                659
            );
        },
    )
    .expect("custom-capacity format materializes");

    let guards = recipe.guards;
    let engine_binary = recipe.engine.binary_identity();
    let mut observations = Recorder::default();
    let run = fixture
        .load(test_world())
        .expect("load")
        .run_configured(
            "hyphen-capacity.tex",
            RegisteredSourceKind::Generated,
            Arc::from(&b"\\tracingstats=1 \\end\n"[..]),
            &[],
            LoadedRunConfiguration {
                guards,
                engine_binary,
                startup_line: "hyphen-capacity.tex".into(),
                completion: tex_exec::RootCompletionPolicy::RequireTeXEnd,
                projection: LoadedFormatProjectionDemand {
                    channels: true,
                    ..LoadedFormatProjectionDemand::default()
                },
            },
            &mut observations,
        )
        .expect("loaded job");
    let log = run.projection.channels.expect("channels requested").log;
    let log = String::from_utf8(log).expect("TeX log is UTF-8");
    assert!(
        log.contains("0 hyphenation exceptions out of 659"),
        "unexpected loaded log: {log:?}"
    );
}

#[test]
fn loaded_driver_configuration_is_job_local() {
    let cache_root = TempDir::new().expect("cache");
    let fixture = ensure_format(
        &FormatCacheStore::new(cache_root.path()),
        &FormatRecipe::raw_tex82(),
    )
    .expect("raw format");
    let widths = tex_state::print::ErrorContextWidths::new(64, 32).expect("valid widths");
    let mut loaded = fixture.load(test_world()).expect("load");
    loaded.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    loaded.set_error_context_widths(widths);
    let mut observations = Recorder::default();
    let run = loaded
        .run(
            "configured.tex",
            Arc::from(&b"\\end\n"[..]),
            &[],
            &mut observations,
        )
        .expect("configured loaded run");
    assert!(run.result.format_dump.is_none());
}

#[test]
fn complete_channel_projection_materializes_the_terminal_suffix_once() {
    crate::with_engine_world(World::memory(), |universe| {
        universe
            .world_mut()
            .publish_detached_effect_records(&[tex_state::EffectRecord::StreamWrite {
                sink: tex_state::PrintSink::TerminalAndLog,
                text: "committed".into(),
            }])
            .expect("committed prefix publishes");
        universe
            .world_mut()
            .begin_terminal_publication(tex_state::TerminalPublicationPhase::Notices);
        universe
            .world_mut()
            .write_text(tex_state::PrintSink::TerminalAndLog, "-terminal");
        universe.world_mut().commit_terminal_publication();

        let projection = capture_loaded_projection(
            universe,
            &LoadedFormatProjectionDemand {
                channels: true,
                ..LoadedFormatProjectionDemand::default()
            },
            tex_exec::RootCompletionPolicy::RequireTeXEnd,
        )
        .expect("complete projection");
        let channels = projection.channels.expect("channels requested");
        assert_eq!(channels.terminal, b"committed-terminal");
        assert_eq!(channels.log, b"committed-terminal");
        assert!(channels.pending_effects.is_empty());
    })
    .expect("fresh universe");
}

#[test]
fn explicit_fresh_seam_matches_loaded_semantic_state() {
    let source = Arc::from(&b"\\count0=7\\advance\\count0 by 5\\end\n"[..]);
    let recipe = FormatRecipe::raw_tex82();
    let cache_root = TempDir::new().expect("cache");
    let fixture =
        ensure_format(&FormatCacheStore::new(cache_root.path()), &recipe).expect("raw format");
    let mut loaded_observations = Recorder::default();
    let loaded = run_loaded_with_counts(
        fixture.load(test_world()).expect("load"),
        "equivalence.tex",
        Arc::clone(&source),
        vec![0],
        &mut loaded_observations,
    );

    let (fresh, fresh_observations) =
        run_explicit_fresh_compatibility(&recipe, "equivalence.tex", source);
    assert_eq!(loaded.projection.counts[0].1, fresh.0);
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
    let loaded = run_loaded_with_counts(
        fixture.load(test_world()).expect("load"),
        "etex-equivalence.tex",
        Arc::clone(&source),
        vec![0],
        &mut loaded_observations,
    );

    let (fresh, fresh_observations) =
        run_explicit_fresh_compatibility(&recipe, "etex-equivalence.tex", source);
    assert_eq!(loaded.projection.counts, [(0, 5)]);
    assert_eq!(loaded.projection.counts[0].1, fresh.0);
    assert_eq!(fresh.1, 7);
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
) -> ((i32, i32), Vec<CommandObservation>) {
    crate::with_engine_world(test_world(), |universe| {
        recipe.engine.prepare_initex(universe);
        let mut session = EngineSession::prepared_initex(universe, recipe.engine.command_profile());
        session
            .set_fuel_limit(recipe.guards.command_fuel)
            .expect("finite fresh fuel");
        session
            .register_authored_job(source_name, source)
            .expect("fresh root");
        let mut recorder = Recorder::default();
        let result = session
            .run_with_observer(
                &mut RecipeResourceHost::new(&recipe.resources),
                &mut NoCheckpoints,
                &mut recorder,
            )
            .expect("fresh compatibility run");
        assert!(result.format_dump.is_none());
        (
            (
                session.stores().count(0).expect("count register"),
                session.stores().int_param(IntParam::TRACING_ASSIGNS),
            ),
            recorder.0,
        )
    })
    .expect("fresh universe")
}
