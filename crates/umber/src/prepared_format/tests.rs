#![allow(
    clippy::disallowed_methods,
    reason = "provider recovery tests deliberately corrupt an isolated native cache"
)]

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use tex_command::{CommandObservation, CommandObserver};

use super::*;

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn launcher() -> FormatWorkerLauncher {
    crate::umber_format_worker_launcher()
}

fn provider(cache: &TempDir) -> PreparedFormatProvider {
    PreparedFormatProvider::with_store(FormatCacheStore::new(cache.path()), launcher())
}

fn clock(year: i32) -> JobClock {
    JobClock {
        time: 7 * 60,
        second: 8,
        day: 9,
        month: 10,
        year,
    }
}

fn guards() -> FormatGenerationGuards {
    FormatGenerationGuards {
        command_fuel: 100_000,
        wall_time: Duration::from_secs(10),
        resident_bytes: 512 * 1024 * 1024,
    }
}

fn job<'a>(source: &'static [u8], observer: &'a mut dyn CommandObserver) -> PreparedFormatJob<'a> {
    PreparedFormatJob {
        engine: EngineMode::Tex82,
        backend: OutputCapability::Dvi,
        clock: clock(2031),
        interaction: InteractionMode::ErrorStop,
        error_context_widths: tex_state::print::ErrorContextWidths::default(),
        guards: guards(),
        source_name: "provider-job.tex".into(),
        source_kind: RegisteredSourceKind::World,
        source: Arc::from(source),
        resources: Vec::new(),
        terminal_input: Vec::new(),
        observer,
    }
}

#[test]
fn independent_providers_share_authenticated_warm_entry_offline() {
    let cache = TempDir::new().expect("cache");
    let recipe = FormatRecipe::raw_tex82();
    let spawns = Arc::new(AtomicUsize::new(0));
    let counted = launcher().with_spawn_counter(Arc::clone(&spawns));
    let first =
        PreparedFormatProvider::with_store(FormatCacheStore::new(cache.path()), counted.clone())
            .prepare(&recipe)
            .expect("cold preparation from complete recipe closure");
    let second = PreparedFormatProvider::with_store(FormatCacheStore::new(cache.path()), counted)
        .prepare(&recipe)
        .expect("independent warm preparation");

    assert_eq!(spawns.load(Ordering::SeqCst), 1);
    assert_eq!(first.image(), second.image());
    assert_eq!(
        first.construction_evidence(),
        second.construction_evidence()
    );
}

#[test]
fn concurrent_providers_construct_once_and_recover_corruption() {
    let cache = TempDir::new().expect("cache");
    let recipe = Arc::new(FormatRecipe::raw_tex82());
    let spawns = Arc::new(AtomicUsize::new(0));
    let launcher = launcher().with_spawn_counter(Arc::clone(&spawns));
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for _ in 0..4 {
        let provider = PreparedFormatProvider::with_store(
            FormatCacheStore::new(cache.path()),
            launcher.clone(),
        );
        let recipe = Arc::clone(&recipe);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            provider.prepare(&recipe).expect("concurrent prepare")
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

    let entry = fs::read_dir(cache.path().join("formats-v2"))
        .expect("cache namespace")
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().starts_with("sha256-"))
        .expect("published entry");
    fs::write(entry.path(), b"corrupt provider entry").expect("corrupt entry");
    let recovered =
        PreparedFormatProvider::with_store(FormatCacheStore::new(cache.path()), launcher)
            .prepare(&recipe)
            .expect("quarantine and recover");
    assert_eq!(spawns.load(Ordering::SeqCst), 2);
    assert_eq!(recovered.image(), fixtures[0].image());
    assert_eq!(
        recovered.construction_evidence(),
        fixtures[0].construction_evidence()
    );
}

#[test]
fn provider_fails_closed_for_cache_profile_backend_and_guards() {
    let unavailable = TempDir::new().expect("parent");
    let file_root = unavailable.path().join("not-a-directory");
    fs::write(&file_root, b"occupied").expect("blocking file");
    let error = PreparedFormatProvider::with_store(FormatCacheStore::new(file_root), launcher())
        .prepare(&FormatRecipe::raw_tex82())
        .expect_err("unavailable cache must fail");
    assert!(matches!(error, FormatFixtureError::Cache(_)));

    let cache = TempDir::new().expect("cache");
    let provider = provider(&cache);
    let fixture = provider
        .prepare(&FormatRecipe::raw_tex82())
        .expect("prepare");
    let mut recorder = Recorder::default();
    let mut request = job(b"\\end\n", &mut recorder);
    request.engine = EngineMode::ETex;
    assert!(matches!(
        provider.run(&fixture, request),
        Err(FormatFixtureError::ProviderProfileMismatch { .. })
    ));

    let mut recorder = Recorder::default();
    let mut request = job(b"\\end\n", &mut recorder);
    request.backend = OutputCapability::Pdf;
    assert!(matches!(
        provider.run(&fixture, request),
        Err(FormatFixtureError::ProviderBackendMismatch { .. })
    ));

    let mut recorder = Recorder::default();
    let mut request = job(b"\\end\n", &mut recorder);
    request.guards.command_fuel = 0;
    assert!(matches!(
        provider.run(&fixture, request),
        Err(FormatFixtureError::UnboundedGuard)
    ));

    let mut recipe = FormatRecipe::raw_tex82();
    recipe.guards.wall_time = Duration::ZERO;
    assert!(matches!(
        provider.prepare(&recipe),
        Err(FormatFixtureError::UnboundedGuard)
    ));
}

#[test]
fn every_loaded_job_has_fresh_clock_terminal_and_mutable_state() {
    let cache = TempDir::new().expect("cache");
    let provider = provider(&cache);
    let fixture = provider
        .prepare(&FormatRecipe::raw_tex82())
        .expect("prepare");

    let mut first_observer = Recorder::default();
    let mut first_job = job(b"\\count0=123\\end\n", &mut first_observer);
    first_job.clock = clock(2041);
    first_job.terminal_input.push("unused first input".into());
    let first = provider.run(&fixture, first_job).expect("first loaded job");
    assert_eq!(first.universe.count(0), 123);
    assert_eq!(first.universe.world().job_clock().year, 2041);

    let mut second_observer = Recorder::default();
    let mut second_job = job(b"\\end\n", &mut second_observer);
    second_job.clock = clock(2042);
    let second = provider
        .run(&fixture, second_job)
        .expect("second loaded job");
    assert_eq!(second.universe.count(0), 0);
    assert_eq!(second.universe.world().job_clock().year, 2042);
    assert!(!first_observer.0.is_empty());
    assert!(!second_observer.0.is_empty());
}
