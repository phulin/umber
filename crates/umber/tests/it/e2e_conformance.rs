use std::env;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parity_harness::run_named_fixture_document;
use parity_harness::{
    ManifestBoundSource, TripObservers, TripTriageChannels, TripTriageInput, TripTriageSource,
    compare_dvi_files, write_trip_triage_artifact,
};
use sha2::{Digest, Sha256};
use test_support::dvi::normalized_dvi_for_comparison;
use tex_command::{CommandObserver, FontResource, RegisteredSourceKind};
use tex_command_stream::{LiveSessionOutcome, LiveSessionTranslator, LiveSource};
use tex_exec::{CanonicalResourceNeed, CheckpointSink, EngineBoundary, EngineCheckpoint};
use tex_oracle::{ObservationStream, SchemaVersion};
use tex_state::provenance::MacroInvocationProvenanceStats;
use tex_state::provenance::ProvenanceStats;
use tex_state::{EffectRecord, PrintSink};
use tex_state::{JobClock, Universe, World};

use umber::{
    CanonicalEngineSession, CanonicalResourceFulfillment, CanonicalResourceHost,
    CanonicalResourceOutcome, CanonicalResourceWorld, CanonicalSessionError, EngineMode,
    FormatGenerationGuards, FormatRecipe, FormatResource, LoadedFormatResource,
    dvi_from_page_plans, ensure_format,
};
use umber_fetch::FormatCacheStore;

#[path = "e2e_conformance/assets.rs"]
mod assets;

use assets::GateAssets;

fn target_dir(repo_root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map_or_else(
            || repo_root.join("target"),
            |path| {
                if path.is_absolute() {
                    path
                } else {
                    repo_root.join(path)
                }
            },
        )
}

struct InProcessRun {
    dvi: Option<Vec<u8>>,
    provenance: ProvenanceStats,
    macro_provenance: MacroInvocationProvenanceStats,
    terminal: Vec<u8>,
    log: Vec<u8>,
    capture: PhaseCapture,
}

enum PhaseCapture {
    Live(LiveCapture),
    Detached(tex_observe::DetachedEvidence),
}

impl PhaseCapture {
    fn streams(&self, oracle: &[u8]) -> tex_command_stream::LiveSessionStreams {
        match self {
            Self::Live(capture) => capture.streams(oracle),
            Self::Detached(evidence) => {
                let diagnostic = tex_observe::canonical_evidence_json_lines(
                    &evidence.semantic,
                    oracle,
                    SchemaVersion::V1,
                )
                .expect("construction semantic evidence encodes under oracle header");
                tex_command_stream::LiveSessionStreams {
                    diagnostic: diagnostic.clone(),
                    stable: diagnostic,
                }
            }
        }
    }

    fn geometry(&self, oracle: &[u8]) -> Vec<u8> {
        match self {
            Self::Live(capture) => capture.geometry(oracle),
            Self::Detached(evidence) => tex_observe::canonical_evidence_json_lines(
                &evidence.geometry,
                oracle,
                SchemaVersion::V2,
            )
            .expect("construction geometry evidence encodes under oracle header"),
        }
    }
}

struct LiveCapture {
    root: LiveSource,
    observations: Vec<tex_command::CommandObservation>,
    outcome: LiveSessionOutcome,
}

fn command_stream_for_fixture_phase(
    fixture_name: &str,
    phase: &str,
    streams: tex_command_stream::LiveSessionStreams,
) -> Vec<u8> {
    if fixture_name == "trip" && phase == "format-loaded" {
        streams.stable
    } else {
        streams.diagnostic
    }
}

impl LiveCapture {
    fn streams(&self, oracle: &[u8]) -> tex_command_stream::LiveSessionStreams {
        let header = ObservationStream::from_canonical_json_lines(oracle)
            .expect("oracle stream validates")
            .header;
        let mut translator =
            LiveSessionTranslator::for_root(SchemaVersion::V1, "terminal", self.root.clone());
        translator.translate_captured(self.observations.clone());
        translator
            .finish(header, self.outcome.clone())
            .expect("live observations translate")
    }

    fn geometry(&self, oracle: &[u8]) -> Vec<u8> {
        let mut observer = parity_harness::TripGeometryObserver::default();
        for observation in self.observations.iter().cloned() {
            observer.committed(observation);
        }
        observer
            .canonical_json_lines(oracle)
            .expect("geometry observations translate")
    }
}

fn transcript_channels(effects: &[EffectRecord]) -> (Vec<u8>, Vec<u8>) {
    let mut terminal = String::new();
    let mut log = String::new();
    for effect in effects {
        let EffectRecord::StreamWrite { sink, text } = effect else {
            continue;
        };
        match sink {
            PrintSink::Terminal => terminal.push_str(text),
            PrintSink::Log => log.push_str(text),
            PrintSink::TerminalAndLog => {
                terminal.push_str(text);
                log.push_str(text);
            }
            PrintSink::Stream(_) => {}
        }
    }
    (terminal.into_bytes(), log.into_bytes())
}

#[test]
fn trip_observer_profile_selection_includes_fixture_and_phase_identity() {
    const DIAGNOSTIC_MANIFEST: &str =
        "1111111111111111111111111111111111111111111111111111111111111111";
    const STABLE_MANIFEST: &str =
        "2222222222222222222222222222222222222222222222222222222222222222";
    const DIAGNOSTIC: &[u8] = b"{\"schema\":1,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    const STABLE: &[u8] = b"{\"schema\":1,\"manifest\":\"2222222222222222222222222222222222222222222222222222222222222222\"}\n";

    for (fixture_name, phase, expected_manifest) in [
        ("trip", "format-loaded", STABLE_MANIFEST),
        ("trip", "initex", DIAGNOSTIC_MANIFEST),
        ("etrip", "initex", DIAGNOSTIC_MANIFEST),
        ("etrip", "format-loaded", DIAGNOSTIC_MANIFEST),
    ] {
        let selected = command_stream_for_fixture_phase(
            fixture_name,
            phase,
            tex_command_stream::LiveSessionStreams {
                diagnostic: DIAGNOSTIC.to_vec(),
                stable: STABLE.to_vec(),
            },
        );
        let stream = ObservationStream::from_canonical_json_lines(&selected)
            .expect("selected observer stream has a canonical schema/header");
        assert_eq!(stream.header.schema, SchemaVersion::V1.number());
        assert_eq!(stream.header.manifest, expected_manifest);
    }
}

struct NoCheckpoints;

impl CheckpointSink for NoCheckpoints {
    fn wants_checkpoint(&self, _boundary: EngineBoundary) -> bool {
        false
    }

    fn checkpoint(&mut self, _checkpoint: EngineCheckpoint) {}
}

/// Canonicalizes a staged fixture directory's job path and loads every file
/// it contains into a memory `World`, keyed by absolute path so both the
/// legacy and canonical in-process runners can address the same staged
/// inputs (source document, format source, hyphenation data, TFMs) the same
/// way `parity_harness::staged_source_dir` assembled them.
#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn staged_world(path: &Path) -> Result<(World, PathBuf), String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?;
    let mut world = World::memory_with_clock(JobClock {
        time: 13 * 60 + 36,
        second: 0,
        day: 9,
        month: 7,
        year: 2026,
    });
    for entry in fs::read_dir(parent)
        .map_err(|error| format!("read staged directory {}: {error}", parent.display()))?
    {
        let entry = entry.map_err(|error| format!("read staged directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?;
        if file_type.is_file() {
            let bytes = fs::read(entry.path())
                .map_err(|error| format!("read {}: {error}", entry.path().display()))?;
            world
                .set_memory_file(entry.path(), bytes)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok((world, path))
}

#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn run_file_in_process(
    path: &Path,
    format: Option<&[u8]>,
    engine: EngineMode,
) -> Result<InProcessRun, String> {
    let mut failure = None;
    let source_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    run_file_in_process_captured(path, &source_name, format, engine, &mut failure)
}

fn startup_input_name(canonical_source_name: &str) -> String {
    format!("./{canonical_source_name}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TripEngineProfile {
    Tex82,
    ETex,
}

impl TripEngineProfile {
    fn from_etex(etex: bool) -> Self {
        if etex { Self::ETex } else { Self::Tex82 }
    }

    fn recipe(self) -> FormatRecipe {
        match self {
            Self::Tex82 => FormatRecipe::raw_tex82(),
            Self::ETex => FormatRecipe::raw_etex26(),
        }
    }

    fn format_name(self) -> &'static str {
        match self {
            Self::Tex82 => "umber-tex82-oracle",
            Self::ETex => "umber-etex26-extended-oracle-clean",
        }
    }
}

fn trip_format_recipe(
    profile: TripEngineProfile,
    fixture_name: &str,
    source_name: &str,
    source: Arc<[u8]>,
    tripos: Arc<[u8]>,
    tfm: Arc<[u8]>,
) -> FormatRecipe {
    let mut recipe = profile.recipe();
    recipe.format_name = profile.format_name().into();
    recipe.construction_source_name = source_name.to_owned();
    recipe.construction_source = source;
    recipe.resources = vec![
        FormatResource::Input {
            logical_name: "tripos.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: tripos,
        },
        FormatResource::Tfm {
            logical_name: format!("{fixture_name}.tfm"),
            bytes: tfm,
        },
    ];
    recipe.distribution_identity = Arc::from(&b"pinned-trip-public-format-boundary-v1"[..]);
    recipe.clock = JobClock {
        time: 13 * 60 + 36,
        second: 0,
        day: 9,
        month: 7,
        year: 2026,
    };
    recipe.construction_interaction = tex_state::InteractionMode::Nonstop;
    recipe.construction_error_context_widths =
        tex_state::print::ErrorContextWidths::new(64, 32).expect("canonical TRIP context widths");
    recipe.guards = FormatGenerationGuards {
        command_fuel: tex_command::DEFAULT_COMMAND_FUEL_LIMIT,
        wall_time: Duration::from_secs(1_800),
        resident_bytes: 6 * 1024 * 1024 * 1024,
    };
    recipe
}

#[test]
fn canonical_source_identity_selects_startup_input_name_independently_of_staging() {
    assert_eq!(startup_input_name("etrip.tex"), "./etrip.tex");
    assert_eq!(
        startup_input_name("inputs/annual.report.tex"),
        "./inputs/annual.report.tex"
    );
}

#[test]
fn trip_and_etrip_recipes_select_typed_public_format_inputs() {
    for (profile, fixture_name, source_name, engine, format_name) in [
        (
            TripEngineProfile::Tex82,
            "trip",
            "trip.tex",
            EngineMode::Tex82,
            "umber-tex82-oracle",
        ),
        (
            TripEngineProfile::ETex,
            "etrip",
            "etrip.tex",
            EngineMode::ETex,
            "umber-etex26-extended-oracle-clean",
        ),
    ] {
        let recipe = trip_format_recipe(
            profile,
            fixture_name,
            source_name,
            Arc::from(&b"fixture source"[..]),
            Arc::from(&b"tripos"[..]),
            Arc::from(&b"tfm"[..]),
        );
        assert_eq!(recipe.engine, engine);
        assert_eq!(recipe.engine.command_profile(), engine.command_profile());
        assert_eq!(recipe.format_name, format_name);
        assert_eq!(recipe.construction_source_name, source_name);
        assert!(matches!(
            &recipe.resources[0],
            FormatResource::Input {
                logical_name,
                source_kind: RegisteredSourceKind::Generated,
                ..
            } if logical_name == "tripos.tex"
        ));
        assert!(matches!(
            &recipe.resources[1],
            FormatResource::Tfm { logical_name, .. }
                if logical_name == &format!("{fixture_name}.tfm")
        ));
        recipe.identity().expect("recipe identity");
    }
}

#[test]
fn two_phase_trip_helper_forbids_private_format_paths() {
    let source = include_str!("e2e_conformance.rs");
    let helper = source
        .rsplit_once("fn run_two_phase_fixture(")
        .expect("two-phase helper exists")
        .1
        .split_once("\n#[test]\n#[ignore = \"manual direct canonical TRIP parity")
        .expect("two-phase helper has a bounded source region")
        .0;
    for forbidden in [
        "run_file_in_process_captured",
        "Universe::from_format",
        ".dump_format(",
        "construct_format_in_worker",
    ] {
        assert!(
            !helper.contains(forbidden),
            "two-phase helper must not use private format path {forbidden}"
        );
    }
    for required in [
        "trip_format_recipe(",
        "ensure_format(",
        ".construction_evidence()",
        ".load(World::memory_with_clock(recipe.clock))",
        ".run(",
    ] {
        assert!(
            helper.contains(required),
            "two-phase helper must retain public boundary step {required}"
        );
    }
}

#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn run_file_in_process_captured(
    path: &Path,
    canonical_source_name: &str,
    format: Option<&[u8]>,
    engine: EngineMode,
    failure: &mut Option<LiveCapture>,
) -> Result<InProcessRun, String> {
    let (world, path) = staged_world(path)?;

    let mut stores = if let Some(format) = format {
        let mut stores = Universe::from_format(world, format).map_err(|error| error.to_string())?;
        engine.install_after_format(&mut stores);
        stores
    } else {
        let mut stores = Universe::with_world(world);
        engine.prepare_initex(&mut stores);
        stores
    };
    // Every live reference invocation used to build the four DVI and command
    // oracles passes `-interaction=nonstopmode`. Match that process option
    // before execution; it is initial session state, not TeX input and not a
    // value owned by the dumped format.
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    // The canonical TRIP environment's texmf.cnf selects 64/32 for TeX82
    // §79's process-level pseudoprint widths. This is driver configuration,
    // not fixture syntax or dumped-format state.
    stores.set_error_context_widths(
        tex_state::print::ErrorContextWidths::new(64, 32).expect("canonical TRIP context widths"),
    );
    let content = stores
        .world_mut()
        .read_file(&path)
        .map_err(|error| error.to_string())?;
    let root_bytes = content.shared_bytes();
    let base_dir = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?
        .to_owned();
    // TeX82 §§529, 537 derive the job name and print the file-opening frame
    // from the driver-selected startup name. The staged path may deliberately
    // differ (e-TRIP is locally renamed), so it is not an observable name.
    let startup_input_name = startup_input_name(canonical_source_name);
    let mut session = if format.is_some() {
        CanonicalEngineSession::new(&mut stores, engine.command_profile())
    } else {
        CanonicalEngineSession::prepared_initex(&mut stores, engine.command_profile())
    };
    assert_eq!(
        session.fuel_limit(),
        tex_command::DEFAULT_COMMAND_FUEL_LIMIT,
        "canonical e2e sessions must use the finite command-fuel default"
    );
    assert_ne!(
        session.fuel_limit(),
        u64::MAX,
        "canonical e2e sessions must never run with unbounded command fuel"
    );
    let root_source = session
        .register_world_root(&startup_input_name, content)
        .map_err(|error| error.to_string())?;
    let mut host = StagedDirResourceHost { base_dir };
    let mut observers = TripObservers::default();
    let run = match session.run_with_observer(&mut host, &mut NoCheckpoints, &mut observers) {
        Ok(run) => run,
        Err(error) => {
            let message = canonical_error_message(&session, &error);
            *failure = Some(LiveCapture {
                root: LiveSource {
                    name: canonical_source_name.to_owned(),
                    source: root_source,
                    bytes: root_bytes,
                },
                observations: mem::take(&mut observers).into_captured(),
                outcome: LiveSessionOutcome::Failed {
                    diagnostic: "canonical_session_error".into(),
                    detail: message.clone(),
                },
            });
            return Err(message);
        }
    };
    drop(session);
    for (index, committed) in run.committed_artifacts.iter().enumerate() {
        let page = tex_out::PageArtifact::from_bytes(committed.bytes())
            .map_err(|error| format!("decode page {} for HTML: {error}", index + 1))?;
        let positioned = tex_out::positioned::lower_page(&page, (index + 1) as u32)
            .map_err(|error| format!("lower page {} for HTML: {error}", index + 1))?;
        tex_out::dvi::coordinates::compare_page(&page, &positioned)
            .map_err(|error| format!("validate page {} HTML coordinates: {error}", index + 1))?;
    }
    let dvi = if run.artifacts.is_empty() {
        None
    } else {
        Some(dvi_from_page_plans(&run.dvi_pages).map_err(|error| error.to_string())?)
    };
    let _format = if run.dumped_format {
        let bytes = stores.dump_format().map_err(|error| error.to_string())?;
        let mut receipt = run
            .format_dump_receipt
            .clone()
            .ok_or_else(|| "dumped format is missing its engine receipt".to_owned())?;
        let displayed = format!("{}.fmt", receipt.format_ident.name);
        tex_exec::confirm_format_dump_publication(&mut stores, &mut receipt, &displayed);
        Some(bytes)
    } else {
        None
    };
    let provenance = stores.provenance_stats();
    let macro_provenance = stores.macro_invocation_provenance_stats();
    let (terminal, log) = transcript_channels(stores.world().effect_records());
    Ok(InProcessRun {
        dvi,
        provenance,
        macro_provenance,
        terminal: terminal.clone(),
        log: log.clone(),
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: canonical_source_name.to_owned(),
                source: root_source,
                bytes: root_bytes,
            },
            observations: observers.into_captured(),
            outcome: LiveSessionOutcome::Completed,
        }),
    })
}

fn run_plain_fixture_case(document: &str, gate: &GateAssets) {
    let fixture_name = gate.name;
    run_named_fixture_document(&gate.repo_root, document, &gate.oracle, |path| {
        let run = run_file_in_process(path, None, EngineMode::Tex82)?;
        let macro_stats = run.macro_provenance;
        let invocations = macro_stats.invocations();
        if invocations == 0 {
            return Err(format!("{document} executed no macro invocations"));
        }
        let macro_bytes = macro_stats.retained_bytes();
        let bytes_per_invocation = macro_stats.bytes_per_invocation();
        eprintln!(
            "{fixture_name} provenance: invocations={invocations} macro_retained_bytes={macro_bytes} bytes_per_invocation={bytes_per_invocation} total_retained_bytes={}",
            run.provenance.retained_bytes(),
        );
        if bytes_per_invocation > 64 {
            return Err(format!(
                "{document} macro provenance retained {bytes_per_invocation} bytes/invocation (budget: 64)"
            ));
        }
        run.dvi
            .ok_or_else(|| "Umber did not produce DVI".to_owned())
    })
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
fn e2e_conformance_story() {
    assets::with_gate("story", |gate| run_plain_fixture_case("story.tex", gate));
}

#[test]
fn e2e_conformance_gentle() {
    assets::with_gate("gentle", |gate| run_plain_fixture_case("gentle.tex", gate));
}

/// Adds an extension inferred from the resource kind (`.tex` for input
/// requests, `.tfm` for font requests) when a canonical resource need names a
/// file without one, mirroring the legacy `InProcessInputResolver`/
/// `InProcessFontResolver`'s own default-extension behavior above.
fn with_default_extension(name: &str, extension: &str) -> PathBuf {
    let mut path = PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension(extension);
    }
    path
}

/// Resolves canonical resource suspensions (`\input`, font loads) directly
/// against the same staged fixture directory the legacy runner reads,
/// keeping the two engines' host-side fixture wiring identical so a DVI
/// difference between them reflects only engine behavior.
struct StagedDirResourceHost {
    base_dir: PathBuf,
}

impl CanonicalResourceHost for StagedDirResourceHost {
    fn fulfill(
        &mut self,
        world: &mut CanonicalResourceWorld<'_>,
        need: &CanonicalResourceNeed,
    ) -> CanonicalResourceOutcome {
        match need {
            CanonicalResourceNeed::Input { name } => {
                let path = with_default_extension(name, "tex");
                world.read_file(self.base_dir.join(path)).ok().map_or(
                    CanonicalResourceOutcome::Unavailable,
                    |content| {
                        CanonicalResourceOutcome::Fulfilled(
                            CanonicalResourceFulfillment::world_input(name, content),
                        )
                    },
                )
            }
            CanonicalResourceNeed::Font { request } => {
                let path = with_default_extension(&request.name, "tfm");
                CanonicalResourceOutcome::Fulfilled(
                    world.read_file(self.base_dir.join(path)).map_or_else(
                        |_| CanonicalResourceFulfillment::Font {
                            request: request.clone(),
                            resource: Box::new(FontResource::Unavailable),
                        },
                        |metrics| CanonicalResourceFulfillment::Font {
                            request: request.clone(),
                            resource: Box::new(FontResource::Tfm {
                                metrics,
                                opentype: None,
                            }),
                        },
                    ),
                )
            }
            CanonicalResourceNeed::PdfImage { .. } => CanonicalResourceOutcome::Unavailable,
        }
    }
}

/// Renders a canonical session failure the same way `first_failure_locator.rs`
/// does: an execution error gets its provenance-resolved TeX source context,
/// while every other `CanonicalSessionError` variant already carries enough
/// context through its own `Display` impl.
fn canonical_error_message(
    session: &CanonicalEngineSession<'_>,
    error: &CanonicalSessionError,
) -> String {
    match error {
        CanonicalSessionError::Execution(exec_error) => {
            exec_error.format_with_provenance(session.stores())
        }
        other => other.to_string(),
    }
}

/// Drives the same staged fixture job through `CanonicalEngineSession`
/// (the canonical `tex-command` architecture, not the legacy
/// `EngineSession`/`ExecutionContext` path above) and returns its assembled
/// DVI bytes. This is the production migration path's equivalent of
/// `run_file_in_process`, sharing the same staged directory contract so the
/// canonical Story and Gentle regression gates below are real byte-exact
/// checks on the `umber2-johp` canonical/reference DVI milestones.
#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn run_file_in_process_canonical(path: &Path) -> Result<Vec<u8>, String> {
    let (world, path) = staged_world(path)?;
    let base_dir = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?
        .to_owned();
    let job_name = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("texput")
        .to_owned();
    let job_bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;

    let mut stores = Universe::with_world(world);
    // These staged gates bootstrap plain.tex from source, so this phase is
    // INITEX rather than a cold job loaded from an already-built format.
    let mut session = CanonicalEngineSession::tex82_initex(&mut stores);
    assert_eq!(
        session.fuel_limit(),
        tex_command::DEFAULT_COMMAND_FUEL_LIMIT,
        "canonical e2e sessions must use the finite command-fuel default"
    );
    assert_ne!(
        session.fuel_limit(),
        u64::MAX,
        "canonical e2e sessions must never run with unbounded command fuel"
    );
    session
        .register_authored_root(&job_name, Arc::from(job_bytes))
        .map_err(|error| format!("register canonical root {job_name}: {error}"))?;

    let mut host = StagedDirResourceHost { base_dir };
    let mut checkpoints: Vec<EngineCheckpoint> = Vec::new();
    let run = session
        .run(&mut host, &mut checkpoints)
        .map_err(|error| canonical_error_message(&session, &error))?;
    if run.dvi_pages.is_empty() {
        return Err("canonical Umber run did not produce DVI".to_owned());
    }
    dvi_from_page_plans(&run.dvi_pages).map_err(|error| error.to_string())
}

fn run_plain_fixture_case_canonical(document: &str, gate: &GateAssets) {
    run_named_fixture_document(
        &gate.repo_root,
        document,
        &gate.oracle,
        run_file_in_process_canonical,
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

/// Protects `umber2-johp`'s first canonical/reference byte-identical DVI
/// milestone (commit 5eed4dc3): the canonical `tex-command`
/// architecture's DVI for `story.tex` must remain byte-identical to real
/// pdfTeX's output after only the same preamble-comment normalization the
/// legacy `e2e_conformance_story` test above already tolerates. Kept
/// alongside (not replacing) the legacy test while the `umber2-johp`
/// migration is in progress; both reach the same registered `story` gate
/// through `assets::with_gate`, so neither can skip silently.
#[test]
fn e2e_conformance_story_canonical() {
    assets::with_gate("story", |gate| {
        run_plain_fixture_case_canonical("story.tex", gate);
    });
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_ligature_group_boundaries_match_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("canonical-dvi", "ligature_group_boundaries");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected =
        test_support::read_binary_fixture("canonical-dvi", "ligature_group_boundaries", "dvi");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_rule_space_factor_reset_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("canonical-dvi", "rule_space_factor_reset");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected =
        test_support::read_binary_fixture("canonical-dvi", "rule_space_factor_reset", "dvi");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_alignment_leading_tabskip_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "alignment_leading_tabskip");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path(
        "math",
        "alignment_leading_tabskip",
        "dvi",
    ))
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_rule_follows_pending_characters_in_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "rule_character_order");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path(
        "math",
        "rule_character_order",
        "dvi",
    ))
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_relax_breaks_ligatures_in_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "relax_ligature_boundary");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path(
        "math",
        "relax_ligature_boundary",
        "dvi",
    ))
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_display_equation_number_preserves_formula_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "display_eqnos");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path("math", "display_eqnos", "dvi"))
        .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_math_group_singleton_ord_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "mathopen_boxed_delimiter");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path(
        "math",
        "mathopen_boxed_delimiter",
        "dvi",
    ))
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

/// Pins the canonical engine's Gentle DVI to the real-pdfTeX oracle. The
/// shared conformance comparator permits only the variable preamble comment;
/// every remaining byte, including list-setting geometry, must match.
#[test]
fn e2e_conformance_gentle_canonical() {
    assets::with_gate("gentle", |gate| {
        run_plain_fixture_case_canonical("gentle.tex", gate);
    });
}

#[allow(clippy::disallowed_methods)] // Host-side fixture staging and artifact comparison.
fn compare_trip_phase(
    root: &Path,
    fixture_name: &str,
    phase: &str,
    run: &InProcessRun,
    expected_identity: &str,
    actual_identity: &str,
    comparison: PhaseComparison<'_>,
) {
    let PhaseComparison {
        dvi_pair,
        contract: phase_contract,
    } = comparison;
    let oracle_root = target_dir(root).join("trip-oracles").join(fixture_name);
    let artifact_root = target_dir(root)
        .join("conformance-artifacts")
        .join(fixture_name);
    fs::create_dir_all(&artifact_root).expect("create event artifact directory");
    let expected_command =
        fs::read(oracle_root.join(format!("{phase}-command.jsonl"))).expect("command oracle");
    let expected_geometry =
        fs::read(oracle_root.join(format!("{phase}-geometry.jsonl"))).expect("geometry oracle");
    let expected_terminal =
        fs::read(oracle_root.join(format!("{phase}-terminal.txt"))).expect("terminal oracle");
    let expected_log = fs::read(oracle_root.join(format!("{phase}.log"))).expect("log oracle");
    let expected_initialization = (phase == "format-loaded").then(|| {
        fs::read(oracle_root.join("initex-command.jsonl")).expect("INITEX command oracle")
    });
    let actual_initialization = (phase == "format-loaded").then(|| {
        fs::read(artifact_root.join("initex-command.jsonl")).expect("INITEX command artifact")
    });
    let actual_command = command_stream_for_fixture_phase(
        fixture_name,
        phase,
        run.capture.streams(&expected_command),
    );
    let actual_geometry = run.capture.geometry(&expected_geometry);
    fs::write(
        artifact_root.join(format!("{phase}-command.jsonl")),
        &actual_command,
    )
    .expect("write command events");
    fs::write(
        artifact_root.join(format!("{phase}-geometry.jsonl")),
        &actual_geometry,
    )
    .expect("write geometry events");
    let label = format!("{fixture_name}-{phase}");
    let (expected_terminal, actual_terminal) =
        phase_contract.text_channel(&expected_terminal, &run.terminal);
    let (expected_log, actual_log) = phase_contract.text_channel(&expected_log, &run.log);
    let verdict = write_trip_triage_artifact(
        &target_dir(root).join("conformance-triage"),
        TripTriageInput {
            label: &label,
            phase,
            expected_source: TripTriageSource {
                name: &format!("target/trip-oracles/{fixture_name}/{phase}"),
                identity: expected_identity,
            },
            actual_source: TripTriageSource {
                name: "umber in-process canonical run",
                identity: actual_identity,
            },
            expected: TripTriageChannels {
                initialization_events: expected_initialization.as_deref(),
                command_events: Some(&expected_command),
                geometry_events: Some(&expected_geometry),
                transcript: expected_terminal,
                log: expected_log,
                dvi: dvi_pair.map(|(expected, _)| expected),
            },
            actual: TripTriageChannels {
                initialization_events: actual_initialization.as_deref(),
                command_events: Some(&actual_command),
                geometry_events: Some(&actual_geometry),
                transcript: actual_terminal,
                log: actual_log,
                dvi: dvi_pair.map(|(_, actual)| actual),
            },
        },
    )
    .expect("write bounded TRIP triage artifact");
    assert_trip_channels_match(&verdict);
}

/// Phase-level parity policy for canonical format fixtures.
///
/// A recipe construction that successfully publishes through its own `\dump`
/// compares structured semantic channels, but its allocator, string-pool, and
/// serialization diagnostics are deliberately outside output parity. Every
/// other phase retains byte-exact terminal and log comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhaseParityContract {
    DumpConstruction,
    OutputProducing,
}

struct PhaseComparison<'a> {
    dvi_pair: Option<(&'a [u8], &'a [u8])>,
    contract: PhaseParityContract,
}

impl PhaseParityContract {
    fn text_channel<'a>(self, expected: &'a [u8], actual: &'a [u8]) -> (&'a [u8], &'a [u8]) {
        match self {
            Self::DumpConstruction => (&[], &[]),
            Self::OutputProducing => (expected, actual),
        }
    }
}

#[test]
fn dump_construction_excludes_only_textual_diagnostics() {
    let expected = b"reference allocator diagnostics";
    let actual = b"implementation-owned serialization diagnostics";
    assert_eq!(
        PhaseParityContract::DumpConstruction.text_channel(expected, actual),
        (&[][..], &[][..])
    );
}

#[test]
fn loaded_and_ordinary_phases_retain_exact_text_channels() {
    let expected = b"reference output";
    let actual = b"mutated output";
    assert_eq!(
        PhaseParityContract::OutputProducing.text_channel(expected, actual),
        (&expected[..], &actual[..])
    );
    assert_ne!(
        expected.as_slice(),
        actual.as_slice(),
        "negative control must remain divergent"
    );
}

#[allow(clippy::disallowed_methods)] // Failure reporting reads the bounded triage artifact.
fn assert_trip_channels_match(verdict: &parity_harness::TripTriageVerdict) {
    if verdict.gating_mismatch {
        let path = verdict
            .artifact
            .as_ref()
            .expect("a gating mismatch writes a bounded report");
        let content = fs::read_to_string(path)
            .unwrap_or_else(|error| format!("unable to read triage report: {error}"));
        panic!(
            "TRIP compared-channel mismatch; report: {}\n{}",
            path.display(),
            content
        );
    }
}

#[test]
fn trip_channel_mismatch_controls_fail_at_the_caller_boundary() {
    fn mismatch_panics(
        root: &Path,
        expected: TripTriageChannels<'_>,
        actual: TripTriageChannels<'_>,
    ) -> String {
        let verdict = write_trip_triage_artifact(
            root,
            TripTriageInput {
                label: "negative-control",
                phase: "bounded",
                expected_source: TripTriageSource {
                    name: "expected",
                    identity: "expected-id",
                },
                actual_source: TripTriageSource {
                    name: "actual",
                    identity: "actual-id",
                },
                expected,
                actual,
            },
        )
        .expect("write negative-control report");
        let panic = std::panic::catch_unwind(|| assert_trip_channels_match(&verdict))
            .expect_err("channel mismatch must fail");
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                panic
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_owned())
            })
            .expect("panic carries a string")
    }

    let temp = tempfile::tempdir().expect("negative-control directory");
    let base = TripTriageChannels {
        initialization_events: None,
        command_events: None,
        geometry_events: None,
        transcript: b"transcript",
        log: b"log",
        dvi: None,
    };
    for (channel, expected, actual) in [
        (
            "command_events",
            TripTriageChannels {
                command_events: Some(b""),
                ..base
            },
            base,
        ),
        (
            "transcript",
            base,
            TripTriageChannels {
                transcript: b"mutated transcript",
                ..base
            },
        ),
        (
            "log",
            base,
            TripTriageChannels {
                log: b"mutated log",
                ..base
            },
        ),
    ] {
        let message = mismatch_panics(temp.path(), expected, actual);
        assert!(
            message.contains("TRIP compared-channel mismatch"),
            "{message}"
        );
        assert!(message.contains("report:"), "{message}");
        assert!(
            message.contains(&format!("earliest.channel: {channel}")),
            "{message}"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // Reads the bounded host-side triage artifact.
fn trip_geometry_only_mismatch_is_reported_but_non_gating() {
    let temp = tempfile::tempdir().expect("geometry control directory");
    let base = TripTriageChannels {
        initialization_events: None,
        command_events: None,
        geometry_events: None,
        transcript: b"transcript",
        log: b"log",
        dvi: None,
    };
    let verdict = write_trip_triage_artifact(
        temp.path(),
        TripTriageInput {
            label: "geometry-advisory-control",
            phase: "bounded",
            expected_source: TripTriageSource {
                name: "expected",
                identity: "expected-id",
            },
            actual_source: TripTriageSource {
                name: "actual",
                identity: "actual-id",
            },
            expected: TripTriageChannels {
                geometry_events: Some(b""),
                ..base
            },
            actual: base,
        },
    )
    .expect("write advisory report");
    assert!(!verdict.gating_mismatch);
    assert!(verdict.advisory_geometry_mismatch);
    let report = fs::read_to_string(verdict.artifact.as_ref().expect("advisory report path"))
        .expect("read advisory report");
    assert!(
        report.contains("status: advisory-geometry-mismatch"),
        "{report}"
    );
    assert!(
        report.contains("geometry.policy: advisory-non-gating"),
        "{report}"
    );
    assert_trip_channels_match(&verdict);
}

fn assert_format_image_contract(format: &[u8], engine: EngineMode) {
    let mut host_world = World::memory();
    host_world
        .set_memory_file("host-only-capability.tex", b"host-only".to_vec())
        .expect("stage host-only capability");
    let mut loaded =
        Universe::from_format(host_world, format).expect("load format into supplied host world");
    let pristine =
        Universe::from_format(World::memory(), format).expect("load format into pristine world");

    assert_eq!(
        loaded.dump_format().expect("redump loaded format"),
        pristine.dump_format().expect("redump pristine format"),
        "host capabilities must not enter the format image"
    );
    assert_eq!(
        loaded.provenance_stats(),
        pristine.provenance_stats(),
        "diagnostic provenance must not survive format loading"
    );
    assert_eq!(
        loaded.macro_invocation_provenance_stats().invocations(),
        0,
        "macro invocation provenance must not survive format loading"
    );
    assert!(
        loaded.world().effect_records().is_empty(),
        "host effects must not survive format loading"
    );
    assert_eq!(
        loaded.env_journal_bytes(),
        pristine.env_journal_bytes(),
        "format loading must reconstruct only the schema's baseline environment journal"
    );

    let before_runtime_state = loaded.dump_format().expect("format before runtime state");
    let _checkpoint = loaded.snapshot();
    loaded.testing_clear_state_hash_caches();
    assert_eq!(
        loaded.dump_format().expect("format after runtime state"),
        before_runtime_state,
        "checkpoints and runtime caches must not enter the format image"
    );

    let relax = loaded.intern("relax");
    let live_relax = loaded.meaning(relax);
    assert_eq!(
        loaded.primitive_meaning("relax"),
        None,
        "primitive registry is runtime state and must not be serialized"
    );
    let mut fresh = Universe::new();
    engine.prepare_initex(&mut fresh);
    let expected_relax = fresh
        .primitive_meaning("relax")
        .expect("fresh engine registers relax");
    engine.install_after_format(&mut loaded);
    assert_eq!(
        loaded.meaning(relax),
        live_relax,
        "registry reconstruction must preserve the format's live meaning"
    );
    assert_eq!(
        loaded.primitive_meaning("relax"),
        Some(expected_relax),
        "format loading must reconstruct the selected engine registry"
    );
    let frozen_relax = loaded
        .primitive_token("relax")
        .expect("reconstructed primitive token");
    assert_eq!(
        loaded.frozen_primitive_meaning(frozen_relax),
        Some(expected_relax),
        "reconstructed frozen meaning must match fresh engine setup"
    );
}

#[test]
fn format_image_contract_excludes_runtime_state_and_rebuilds_registry() {
    let mut source = Universe::new();
    EngineMode::Tex82.prepare_initex(&mut source);
    source
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "host effect excluded");
    let _checkpoint = source.snapshot();
    source.testing_clear_state_hash_caches();
    let format = source.dump_format().expect("bounded format image");

    assert_format_image_contract(&format, EngineMode::Tex82);
}

#[allow(clippy::disallowed_methods)] // Host-side fixture staging and artifact comparison.
fn run_two_phase_fixture(source_name: &str, local_name: &str, etex: bool, gate: &GateAssets) {
    let root = &gate.repo_root;
    let fixture_name = gate.name;
    let fixture = &gate.oracle;
    let source = root.join("third_party/trip").join(source_name);

    let source_bytes = fs::read(&source).expect("read conformance source");
    let source_bytes = if etex {
        let source = String::from_utf8(source_bytes).expect("e-TRIP source is UTF-8");
        format!(
            "%% Local e-TeX 2.6 compatibility adaptation; the official etrip.tex remains unchanged.\n%% Renamed and modified as required by the e-TeX distribution terms.\n{}",
            source.replace("\\def\\etripversion{2.0}", "\\def\\etripversion{2.6}")
        )
        .into_bytes()
    } else {
        source_bytes
    };
    let source_identity = ManifestBoundSource::new(source_name, local_name, &source_bytes);
    let source_bytes: Arc<[u8]> = Arc::from(source_bytes);
    let tripos: Arc<[u8]> = Arc::from(
        fs::read(root.join("third_party/trip/tripos.tex")).expect("read shared TRIP input"),
    );
    let tfm: Arc<[u8]> =
        Arc::from(fs::read(root.join("third_party/trip/trip.tfm")).expect("read conformance TFM"));
    let profile = TripEngineProfile::from_etex(etex);
    let recipe = trip_format_recipe(
        profile,
        fixture_name,
        source_identity.canonical_name(),
        Arc::clone(&source_bytes),
        Arc::clone(&tripos),
        Arc::clone(&tfm),
    );
    let engine = recipe.engine;
    let cache_root = tempfile::tempdir().expect("create authenticated format cache");
    let cache = FormatCacheStore::new(cache_root.path());
    let first = ensure_format(&cache, &recipe, &super::umber_format_worker_launcher())
        .unwrap_or_else(|error| panic!("{fixture_name} format creation failed: {error}"));
    let second = ensure_format(&cache, &recipe, &super::umber_format_worker_launcher())
        .unwrap_or_else(|error| panic!("{fixture_name} format cache hit failed: {error}"));
    assert_eq!(first.image(), second.image(), "cache hit image changed");
    assert_eq!(
        first.construction_evidence(),
        second.construction_evidence(),
        "cache hit construction evidence changed"
    );
    let format = second.image().to_vec();
    let initex_identity = format!("sha256:{:x}", Sha256::digest(&format));
    let initial = InProcessRun {
        dvi: None,
        provenance: ProvenanceStats::default(),
        macro_provenance: MacroInvocationProvenanceStats::default(),
        terminal: Vec::new(),
        log: Vec::new(),
        capture: PhaseCapture::Detached(second.construction_evidence().clone()),
    };
    compare_trip_phase(
        root,
        fixture_name,
        "initex",
        &initial,
        &initex_identity,
        &initex_identity,
        PhaseComparison {
            dvi_pair: None,
            contract: PhaseParityContract::DumpConstruction,
        },
    );
    assert_format_image_contract(&format, engine);
    let mut loaded_fixture = second
        .load(World::memory_with_clock(recipe.clock))
        .expect("load authenticated format into a fresh job world");
    loaded_fixture.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    loaded_fixture.set_error_context_widths(
        tex_state::print::ErrorContextWidths::new(64, 32).expect("canonical TRIP context widths"),
    );
    let resources = vec![
        LoadedFormatResource::Input {
            logical_name: "tripos.tex".into(),
            resolved_name: "./tripos.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: tripos,
        },
        LoadedFormatResource::Tfm {
            logical_name: format!("{fixture_name}.tfm"),
            bytes: tfm,
        },
    ];
    let mut observers = TripObservers::default();
    let loaded_run = loaded_fixture
        .run(
            source_identity.canonical_name(),
            Arc::clone(&source_bytes),
            &resources,
            &mut observers,
        )
        .unwrap_or_else(|error| panic!("{fixture_name} format-loaded run failed: {error}"));
    let dvi = (!loaded_run.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded_run.result.dvi_pages))
        .transpose()
        .expect("serialize loaded DVI");
    let (terminal, log) = transcript_channels(&loaded_run.result.effects);
    assert!(
        !terminal
            .windows(b"Beginning to dump on file".len())
            .any(|window| window == b"Beginning to dump on file")
            && !log
                .windows(b"Beginning to dump on file".len())
                .any(|window| window == b"Beginning to dump on file"),
        "construction-only dump diagnostics entered loaded output"
    );
    let loaded = InProcessRun {
        dvi,
        provenance: loaded_run.universe.provenance_stats(),
        macro_provenance: loaded_run.universe.macro_invocation_provenance_stats(),
        terminal: terminal.clone(),
        log: log.clone(),
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: source_identity.canonical_name().to_owned(),
                source: loaded_run.root_source,
                bytes: source_bytes,
            },
            observations: observers.into_captured(),
            outcome: LiveSessionOutcome::Completed,
        }),
    };
    let dvi = loaded
        .dvi
        .clone()
        .unwrap_or_else(|| panic!("{fixture_name} did not produce DVI"));
    let actual = target_dir(root)
        .join("conformance-artifacts")
        .join(format!("{fixture_name}.dvi"));
    fs::create_dir_all(actual.parent().expect("artifact parent"))
        .expect("create conformance artifact directory");
    fs::write(&actual, dvi).expect("write conformance artifact");
    let expected_dvi = fs::read(fixture).expect("read conformance DVI oracle");
    let actual_dvi = fs::read(&actual).expect("read conformance DVI artifact");
    let expected_normalized =
        normalized_dvi_for_comparison(&expected_dvi).expect("normalize conformance DVI oracle");
    let expected_identity = format!("sha256:{:x}", Sha256::digest(&expected_normalized));
    let actual_identity = format!("sha256:{:x}", Sha256::digest(&format));
    compare_trip_phase(
        root,
        fixture_name,
        "format-loaded",
        &loaded,
        &expected_identity,
        &actual_identity,
        PhaseComparison {
            dvi_pair: Some((&expected_dvi, &actual_dvi)),
            contract: PhaseParityContract::OutputProducing,
        },
    );
    compare_dvi_files(
        fixture,
        &actual,
        &target_dir(root).join("conformance-triage"),
        fixture_name,
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
#[ignore = "manual direct canonical TRIP parity; run through scripts/trip.sh"]
fn e2e_conformance_trip_canonical() {
    assets::with_gate("trip", |gate| {
        run_two_phase_fixture("trip.tex", "trip.tex", false, gate);
    });
}

#[test]
#[ignore = "manual full-document e-TRIP parity; run through scripts/trip.sh"]
fn e2e_conformance_etrip() {
    assets::with_gate("etrip", |gate| {
        run_two_phase_fixture("etrip.tex", "etrip-local.tex", true, gate);
    });
}
