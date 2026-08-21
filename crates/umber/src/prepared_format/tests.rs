#![allow(
    clippy::disallowed_methods,
    reason = "provider recovery tests deliberately corrupt an isolated native cache"
)]

use std::fs;
use std::process::Command;
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
        engine_binary: tex_exec::EngineBinaryIdentity::Tex82,
        backend: OutputCapability::Dvi,
        clock: clock(2031),
        interaction: InteractionMode::ErrorStop,
        error_context_widths: tex_state::print::ErrorContextWidths::default(),
        provenance_demand: tex_state::ProvenanceDemand::DIAGNOSTICS,
        guards: guards(),
        startup_line: "provider-job.tex".into(),
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

    let entry = fs::read_dir(cache.path().join("blobs-v1"))
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

#[cfg(unix)]
#[test]
fn competing_provider_processes_construct_once() {
    let cache = TempDir::new().expect("cache");
    let markers = TempDir::new().expect("construction markers");
    let executable = std::env::current_exe().expect("current test executable");
    let mut children = Vec::new();
    for _ in 0..6 {
        children.push(
            Command::new(&executable)
                .args([
                    "--ignored",
                    "--exact",
                    "prepared_format::tests::process_prepared_format_worker",
                ])
                .env("UMBER_PREPARED_FORMAT_CACHE_ROOT", cache.path())
                .env("UMBER_PREPARED_FORMAT_MARKER_ROOT", markers.path())
                .spawn()
                .expect("spawn provider process"),
        );
    }
    for mut child in children {
        assert!(child.wait().expect("wait for provider process").success());
    }

    assert_eq!(
        fs::read_dir(markers.path())
            .expect("construction markers")
            .count(),
        1,
        "exactly one process may launch format construction"
    );
    provider(&cache)
        .prepare(&FormatRecipe::raw_tex82())
        .expect("published entry remains authenticated");
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
        Err(FormatFixtureError::ProviderProfileMismatch {
            expected: EngineMode::Tex82,
            actual: EngineMode::ETex,
        })
    ));

    let mut recorder = Recorder::default();
    let mut request = job(b"\\end\n", &mut recorder);
    request.engine_binary = tex_exec::EngineBinaryIdentity::Etex26;
    request.engine = EngineMode::PdfTex;
    assert!(matches!(
        provider.run(&fixture, request),
        Err(FormatFixtureError::ProviderProfileMismatch { .. })
    ));

    let pdf_fixture = provider
        .prepare(&FormatRecipe::production_pdftex14029())
        .expect("prepare pdfTeX");
    let mut recorder = Recorder::default();
    let mut request = job(b"\\end\n", &mut recorder);
    request.engine = EngineMode::PdfTex;
    request.engine_binary = tex_exec::EngineBinaryIdentity::Etex26;
    assert!(matches!(
        provider.run(&pdf_fixture, request),
        Err(FormatFixtureError::ProviderBinaryMismatch {
            engine: EngineMode::PdfTex,
            binary: tex_exec::EngineBinaryIdentity::Etex26,
        })
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
    let mut first_job = job(
        b"\\count0=123\\read0 to \\terminalcommand \\terminalcommand \\end\n",
        &mut first_observer,
    );
    first_job.clock = clock(2041);
    first_job.terminal_input.push("\\count0=321".into());
    let first = provider.run(&fixture, first_job).expect("first loaded job");
    assert_eq!(first.universe.count(0), 321);
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

#[test]
fn loaded_job_reopens_authenticated_resources_after_job_precedence() {
    let cache = TempDir::new().expect("cache");
    let provider = provider(&cache);
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.resources.push(crate::FormatResource::Tfm {
        logical_name: "cmr10.tfm".into(),
        bytes: Arc::from(&include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm")[..]),
    });
    recipe.resources.push(crate::FormatResource::Input {
        logical_name: "shared.tex".into(),
        source_kind: RegisteredSourceKind::World,
        bytes: Arc::from(&b"\\count0=1\n"[..]),
    });
    let fixture = provider
        .prepare(&recipe)
        .expect("prepare with font closure");
    let mut recorder = Recorder::default();
    let mut request = job(
        b"\\catcode`\\{=1 \\catcode`\\}=2 \\input shared \\font\\tenrm=cmr10 \\shipout\\hbox{\\tenrm X}\\end\n",
        &mut recorder,
    );
    request.resources.push(LoadedFormatResource::Input {
        logical_name: "shared.tex".into(),
        resolved_name: "./job/shared.tex".into(),
        source_kind: RegisteredSourceKind::World,
        bytes: Arc::from(&b"\\count0=2\n"[..]),
    });
    let run = provider
        .run(&fixture, request)
        .expect("loaded job reopens construction font");

    assert!(!run.result.dvi_pages.is_empty());
    assert_eq!(run.universe.count(0), 2);
    assert!(
        run.universe
            .world()
            .input_records()
            .iter()
            .any(|record| record.path() == std::path::Path::new("cmr10.tfm"))
    );
}

#[test]
fn loaded_job_applies_explicit_provenance_demand_after_format_restore() {
    let cache = TempDir::new().expect("cache");
    let provider = provider(&cache);
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.resources.push(crate::FormatResource::Tfm {
        logical_name: "cmr10.tfm".into(),
        bytes: Arc::from(&include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm")[..]),
    });
    let fixture = provider
        .prepare(&recipe)
        .expect("prepare with font closure");
    let prepared_bytes = fixture.image().to_vec();
    let source =
        b"\\catcode`\\{=1 \\catcode`\\}=2 \\font\\tenrm=cmr10 \\def\\x{X}\\shipout\\hbox{\\tenrm \\x}\\end\n";

    let mut diagnostics_observer = Recorder::default();
    let diagnostics = provider
        .run(&fixture, job(source, &mut diagnostics_observer))
        .expect("diagnostics-only loaded job");
    let diagnostics_frames = diagnostics.universe.macro_invocation_origins_for_testing();
    assert_eq!(diagnostics_frames.len(), 1);
    let diagnostics_artifact = diagnostics
        .universe
        .world()
        .committed_artifacts()
        .first()
        .expect("diagnostics-only shipped artifact");
    assert_eq!(
        diagnostics_artifact.render_node_count(),
        0,
        "without a rendered-source consumer the artifact retains no cold sidecar"
    );

    let mut rendered_observer = Recorder::default();
    let mut rendered_job = job(source, &mut rendered_observer);
    rendered_job.provenance_demand = tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE;
    let rendered = provider
        .run(&fixture, rendered_job)
        .expect("rendered-source loaded job");
    assert!(
        rendered
            .universe
            .macro_invocation_provenance_stats()
            .invocations()
            > 0,
        "the loaded job archives its producing macro frame"
    );
    let rendered_artifact = rendered
        .universe
        .world()
        .committed_artifacts()
        .first()
        .expect("rendered-source shipped artifact");
    assert!(rendered_artifact.render_node_count() > 0);
    assert!(
        (0..rendered_artifact.render_node_count()).any(|node| matches!(
            rendered_artifact.render_origin(node, 0),
            tex_state::ArtifactOrigin::Detached(_)
        )),
        "rendered material owns an exact cold provenance sidecar"
    );
    assert_eq!(
        fixture.image(),
        prepared_bytes,
        "job-local provenance demand must not mutate prepared-format bytes"
    );
}

#[test]
fn loaded_job_does_not_reopen_wrong_typed_recipe_resource() {
    let cache = TempDir::new().expect("cache");
    let provider = provider(&cache);
    let mut recipe = FormatRecipe::raw_tex82();
    recipe.resources.push(crate::FormatResource::Input {
        logical_name: "cmr10.tfm".into(),
        source_kind: RegisteredSourceKind::World,
        bytes: Arc::from(&include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm")[..]),
    });
    let fixture = provider.prepare(&recipe).expect("prepare typed closure");
    let mut recorder = Recorder::default();
    let run = provider
        .run(&fixture, job(b"\\font\\tenrm=cmr10 \\end\n", &mut recorder))
        .expect("missing font remains a bounded TeX job outcome");

    assert!(run.universe.world().input_records().is_empty());
}

#[cfg(unix)]
#[test]
#[ignore = "subprocess-only helper"]
fn process_prepared_format_worker() {
    let (Some(cache_root), Some(marker_root)) = (
        std::env::var_os("UMBER_PREPARED_FORMAT_CACHE_ROOT"),
        std::env::var_os("UMBER_PREPARED_FORMAT_MARKER_ROOT"),
    ) else {
        return;
    };
    let spawns = Arc::new(AtomicUsize::new(0));
    PreparedFormatProvider::with_store(
        FormatCacheStore::new(cache_root),
        launcher().with_spawn_counter(Arc::clone(&spawns)),
    )
    .prepare(&FormatRecipe::raw_tex82())
    .expect("process provider preparation");
    if spawns.load(Ordering::SeqCst) == 1 {
        fs::write(
            std::path::PathBuf::from(marker_root).join(std::process::id().to_string()),
            b"constructed",
        )
        .expect("record construction winner");
    }
}
