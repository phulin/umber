use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{collections::BTreeMap, mem};

use parity_harness::run_named_fixture_document;
use parity_harness::{
    ManifestBoundSource, TripObservers, TripTriageChannels, TripTriageInput, TripTriageSource,
    compare_dvi_files, write_trip_triage_artifact,
};
use sha2::{Digest, Sha256};
use test_support::dvi::normalized_dvi_for_comparison;
use tex_command::CommandObserver;
use tex_command::FontResource;
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
    dvi_from_page_plans,
};

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
    format: Option<Vec<u8>>,
    provenance: ProvenanceStats,
    macro_provenance: MacroInvocationProvenanceStats,
    terminal: Vec<u8>,
    log: Vec<u8>,
    capture: LiveCapture,
}

struct LiveCapture {
    root: LiveSource,
    registered_inputs: BTreeMap<String, Arc<[u8]>>,
    observations: Vec<tex_command::CommandObservation>,
    outcome: LiveSessionOutcome,
    terminal: Vec<u8>,
    log: Vec<u8>,
}

impl LiveCapture {
    fn streams(&self, oracle: &[u8]) -> tex_command_stream::LiveSessionStreams {
        let header = ObservationStream::from_canonical_json_lines(oracle)
            .expect("oracle stream validates")
            .header;
        let mut translator = LiveSessionTranslator::for_root(
            SchemaVersion::V1,
            "terminal",
            self.root.clone(),
            self.registered_inputs.clone(),
        );
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

#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn run_file_in_process_captured(
    path: &Path,
    _canonical_source_name: &str,
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
    let content = stores
        .world_mut()
        .read_file(&path)
        .map_err(|error| error.to_string())?;
    let root_bytes = content.shared_bytes();
    let base_dir = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?
        .to_owned();
    let job_name = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("texput");
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
        .register_world_root(job_name, content)
        .map_err(|error| error.to_string())?;
    let registered_inputs = fs::read_dir(&base_dir)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path() != path && entry.path().extension().is_some_and(|ext| ext == "tex")
        })
        .filter_map(|entry| {
            let name = entry.path().file_stem()?.to_str()?.to_owned();
            let bytes = fs::read(entry.path()).ok()?;
            Some((name, Arc::from(bytes)))
        })
        .collect::<BTreeMap<_, _>>();
    let mut host = StagedDirResourceHost { base_dir };
    let mut observers = TripObservers::default();
    let run = match session.run_with_observer(&mut host, &mut NoCheckpoints, &mut observers) {
        Ok(run) => run,
        Err(error) => {
            let message = canonical_error_message(&session, &error);
            let (terminal, log) = transcript_channels(session.stores().world().effect_records());
            *failure = Some(LiveCapture {
                root: LiveSource {
                    name: _canonical_source_name.to_owned(),
                    source: root_source,
                    bytes: root_bytes,
                },
                registered_inputs,
                observations: mem::take(&mut observers).into_captured(),
                outcome: LiveSessionOutcome::Failed {
                    diagnostic: "canonical_session_error".into(),
                    detail: message.clone(),
                },
                terminal,
                log,
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
    let format = if run.dumped_format {
        Some(stores.dump_format().map_err(|error| error.to_string())?)
    } else {
        None
    };
    let provenance = stores.provenance_stats();
    let macro_provenance = stores.macro_invocation_provenance_stats();
    let (terminal, log) = transcript_channels(&run.effects);
    Ok(InProcessRun {
        dvi,
        format,
        provenance,
        macro_provenance,
        terminal: terminal.clone(),
        log: log.clone(),
        capture: LiveCapture {
            root: LiveSource {
                name: _canonical_source_name.to_owned(),
                source: root_source,
                bytes: root_bytes,
            },
            registered_inputs,
            observations: observers.into_captured(),
            outcome: LiveSessionOutcome::Completed,
            terminal: terminal.clone(),
            log: log.clone(),
        },
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
    let setup = test_support::dvi::DviCaseSetup::new("dvi", "ligature_group_boundaries");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("dvi")
            .join("ligature_group_boundaries.expected.dvi"),
    )
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_rule_space_factor_reset_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("dvi", "rule_space_factor_reset");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("dvi")
            .join("rule_space_factor_reset.expected.dvi"),
    )
    .expect("reference DVI");
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
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("alignment_leading_tabskip.expected.dvi"),
    )
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
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("rule_character_order.expected.dvi"),
    )
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
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("relax_ligature_boundary.expected.dvi"),
    )
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
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("display_eqnos.expected.dvi"),
    )
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
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("mathopen_boxed_delimiter.expected.dvi"),
    )
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
    dvi_pair: Option<(&[u8], &[u8])>,
) {
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
    let actual_command = run.capture.streams(&expected_command).diagnostic;
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
    write_trip_triage_artifact(
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
                transcript: &expected_terminal,
                log: &expected_log,
                dvi: dvi_pair.map(|(expected, _)| expected),
            },
            actual: TripTriageChannels {
                initialization_events: actual_initialization.as_deref(),
                command_events: Some(&actual_command),
                geometry_events: Some(&actual_geometry),
                transcript: &run.terminal,
                log: &run.log,
                dvi: dvi_pair.map(|(_, actual)| actual),
            },
        },
    )
    .expect("write bounded TRIP triage artifact");
}

#[allow(clippy::disallowed_methods)] // Host-side oracle and triage artifact boundary.
fn compare_trip_failure(
    root: &Path,
    fixture_name: &str,
    phase: &str,
    capture: &LiveCapture,
    error: &str,
) {
    let oracle_root = target_dir(root).join("trip-oracles").join(fixture_name);
    let expected_command =
        fs::read(oracle_root.join(format!("{phase}-command.jsonl"))).expect("command oracle");
    let expected_geometry =
        fs::read(oracle_root.join(format!("{phase}-geometry.jsonl"))).expect("geometry oracle");
    let actual_command = capture.streams(&expected_command).diagnostic;
    let mut geometry = parity_harness::TripGeometryObserver::default();
    for observation in capture.observations.iter().cloned() {
        geometry.committed(observation);
    }
    let actual_geometry = (geometry.event_count() != 0).then(|| {
        geometry
            .canonical_json_lines(&expected_geometry)
            .expect("geometry")
    });
    let expected_terminal =
        fs::read(oracle_root.join(format!("{phase}-terminal.txt"))).expect("terminal oracle");
    let expected_log = fs::read(oracle_root.join(format!("{phase}.log"))).expect("log oracle");
    let artifact_root = target_dir(root)
        .join("conformance-artifacts")
        .join(fixture_name);
    fs::create_dir_all(&artifact_root).expect("create event artifact directory");
    fs::write(
        artifact_root.join(format!("{phase}-command.jsonl")),
        &actual_command,
    )
    .expect("write failed command stream");
    if let Some(geometry) = &actual_geometry {
        fs::write(
            artifact_root.join(format!("{phase}-geometry.jsonl")),
            geometry,
        )
        .expect("write failed geometry stream");
    }
    let label = format!("{fixture_name}-{phase}");
    let identity = format!("failed:sha256:{:x}", Sha256::digest(error.as_bytes()));
    write_trip_triage_artifact(
        &target_dir(root).join("conformance-triage"),
        TripTriageInput {
            label: &label,
            phase,
            expected_source: TripTriageSource {
                name: &format!("target/trip-oracles/{fixture_name}/{phase}"),
                identity: "pinned-reference",
            },
            actual_source: TripTriageSource {
                name: "umber failed canonical run",
                identity: &identity,
            },
            expected: TripTriageChannels {
                initialization_events: None,
                command_events: Some(&expected_command),
                geometry_events: Some(&expected_geometry),
                transcript: &expected_terminal,
                log: &expected_log,
                dvi: None,
            },
            actual: TripTriageChannels {
                initialization_events: None,
                command_events: Some(&actual_command),
                geometry_events: actual_geometry.as_deref(),
                transcript: &capture.terminal,
                log: &capture.log,
                dvi: None,
            },
        },
    )
    .expect("write failed-run triage");
}

#[allow(clippy::disallowed_methods)] // Host-side fixture staging and artifact comparison.
fn run_two_phase_fixture(source_name: &str, local_name: &str, etex: bool, gate: &GateAssets) {
    let root = &gate.repo_root;
    let fixture_name = gate.name;
    let fixture = &gate.oracle;
    let source = root.join("third_party/trip").join(source_name);

    let temp = tempfile::tempdir().expect("create two-phase conformance directory");
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
    let input = temp.path().join(source_identity.staged_name());
    fs::write(&input, source_bytes).expect("stage conformance source");
    fs::copy(
        root.join("third_party/trip/trip.tfm"),
        temp.path().join(format!("{fixture_name}.tfm")),
    )
    .expect("stage conformance TFM");
    fs::copy(
        root.join("third_party/trip/tripos.tex"),
        temp.path().join("tripos.tex"),
    )
    .expect("stage shared TRIP input");

    let engine = if etex {
        EngineMode::ETex
    } else {
        EngineMode::Tex82
    };
    let mut failure = None;
    let mut initial = run_file_in_process_captured(
        &input,
        source_identity.canonical_name(),
        None,
        engine,
        &mut failure,
    )
    .unwrap_or_else(|error| {
        compare_trip_failure(
            root,
            fixture_name,
            "initex",
            failure.as_ref().expect("failed capture"),
            &error,
        );
        panic!("{fixture_name} format creation failed: {error}")
    });
    let format = initial.format.clone();
    let initex_identity = format
        .as_deref()
        .map(|bytes| format!("sha256:{:x}", Sha256::digest(bytes)))
        .unwrap_or_else(|| "absent".to_owned());
    if format.is_none() {
        initial.capture.outcome = LiveSessionOutcome::Failed {
            diagnostic: "missing_format_dump".into(),
            detail: format!("{fixture_name} did not dump a format"),
        };
    }
    compare_trip_phase(
        root,
        fixture_name,
        "initex",
        &initial,
        &initex_identity,
        &initex_identity,
        None,
    );
    let format = format.unwrap_or_else(|| panic!("{fixture_name} did not dump a format"));
    let mut failure = None;
    let loaded = run_file_in_process_captured(
        &input,
        source_identity.canonical_name(),
        Some(&format),
        engine,
        &mut failure,
    )
    .unwrap_or_else(|error| {
        compare_trip_failure(
            root,
            fixture_name,
            "format-loaded",
            failure.as_ref().expect("failed capture"),
            &error,
        );
        panic!("{fixture_name} format-loaded run failed: {error}")
    });
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
        Some((&expected_dvi, &actual_dvi)),
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
#[ignore = "manual full-document TRIP parity; run through scripts/trip.sh"]
fn e2e_conformance_trip() {
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
