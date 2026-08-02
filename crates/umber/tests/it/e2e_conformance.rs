use std::env;
use std::fs;
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
use tex_command::{CommandObserver, RegisteredSourceKind};
use tex_command_stream::{LiveSessionOutcome, LiveSessionTranslator, LiveSource};
use tex_oracle::{ObservationStream, SchemaVersion};
use tex_state::provenance::MacroInvocationProvenanceStats;
use tex_state::provenance::ProvenanceStats;
use tex_state::{EffectRecord, PrintSink};
use tex_state::{JobClock, Universe, World};

use umber::{
    EngineMode, FormatGenerationGuards, FormatRecipe, FormatResource, LoadedFormatResource,
    OutputCapability, PreparedFormatJob, PreparedFormatProvider, dvi_from_page_plans,
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
    fn command(&self, fixture_name: &str, phase: &str, oracle: &[u8]) -> Vec<u8> {
        if fixture_name == "trip"
            && phase == "format-loaded"
            && let Self::Live(capture) = self
        {
            let mut observer = parity_harness::TripProfileObserver::default();
            for observation in capture.observations.iter().cloned() {
                observer.committed(observation);
            }
            return observer
                .canonical_json_lines(oracle)
                .expect("TRIP profile observations translate");
        }
        command_stream_for_fixture_phase(fixture_name, phase, self.streams(oracle))
    }

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

fn transcript_channels(stores: &Universe, effects: &[EffectRecord]) -> (Vec<u8>, Vec<u8>) {
    // A shipout commits and drains the live effect prefix into the memory
    // backend. `RunResult::effects` consequently contains only the suffix
    // after the last commit. TeX82 §§61, 536, 638, and 1333 still define one
    // ordered terminal/log episode, so parity evidence must join the already
    // committed prefix to that pending suffix rather than treating the suffix
    // as the whole transcript.
    let terminal = stores
        .world()
        .memory_terminal_output()
        .expect("prepared-format jobs use a memory World")
        .to_vec();
    let log = stores
        .world()
        .memory_log_output()
        .expect("prepared-format jobs use a memory World")
        .to_vec();
    append_transcript_suffix(terminal, log, effects)
}

fn append_transcript_suffix(
    mut terminal: Vec<u8>,
    mut log: Vec<u8>,
    effects: &[EffectRecord],
) -> (Vec<u8>, Vec<u8>) {
    for effect in effects {
        let EffectRecord::StreamWrite { sink, text } = effect else {
            continue;
        };
        match sink {
            PrintSink::Terminal => terminal.extend_from_slice(text.as_bytes()),
            PrintSink::Log => log.extend_from_slice(text.as_bytes()),
            PrintSink::TerminalAndLog => {
                terminal.extend_from_slice(text.as_bytes());
                log.extend_from_slice(text.as_bytes());
            }
            PrintSink::Stream(_) => {}
        }
    }
    (terminal, log)
}

#[test]
fn transcript_capture_preserves_multiple_committed_prefixes_exactly_once() {
    let mut stores = Universe::with_world(World::memory());
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "shared-prefix:");
    stores
        .commit_effects(stores.world().effect_pos())
        .expect("commit shared prefix");
    stores
        .world_mut()
        .write_text(PrintSink::Terminal, "terminal-prefix:");
    stores.world_mut().write_text(PrintSink::Log, "log-prefix:");
    stores
        .commit_effects(stores.world().effect_pos())
        .expect("commit per-channel prefixes");
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "shared-tail");
    stores
        .world_mut()
        .write_text(PrintSink::Terminal, "-terminal");
    stores.world_mut().write_text(PrintSink::Log, "-log");

    let effects = stores.world().effect_records().to_vec();
    let (terminal, log) = transcript_channels(&stores, &effects);

    assert_eq!(
        terminal,
        b"shared-prefix:terminal-prefix:shared-tail-terminal"
    );
    assert_eq!(log, b"shared-prefix:log-prefix:shared-tail-log");
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

fn startup_input_name(canonical_source_name: &str) -> String {
    format!("./{canonical_source_name}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TripEngineProfile {
    Tex82,
    ETex,
}

impl TripEngineProfile {
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
    // TeX82 §1328 persists the dump job name independently of web2c §61's
    // selected `dump_name` used by the terminal banner.
    recipe.format_ident_name = fixture_name.to_owned();
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
    recipe.construction_error_context_widths = tex_state::print::ErrorContextWidths::new(64, 32)
        .and_then(|widths| widths.with_max_print_line(72))
        .expect("canonical TRIP print widths");
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
    let source: Arc<[u8]> = Arc::from(&b"fixture source"[..]);
    let tripos: Arc<[u8]> = Arc::from(&b"tripos"[..]);
    let tfm: Arc<[u8]> = Arc::from(&b"tfm"[..]);
    let mut identities = Vec::new();
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
            Arc::clone(&source),
            Arc::clone(&tripos),
            Arc::clone(&tfm),
        );
        assert_eq!(recipe.engine, engine);
        assert_eq!(recipe.engine.command_profile(), engine.command_profile());
        assert_eq!(recipe.format_name, format_name);
        assert_eq!(recipe.format_ident_name, fixture_name);
        assert_eq!(recipe.construction_source_name, source_name);
        assert_eq!(recipe.construction_source, source);
        assert_eq!(
            recipe.resources,
            vec![
                FormatResource::Input {
                    logical_name: "tripos.tex".into(),
                    source_kind: RegisteredSourceKind::Generated,
                    bytes: Arc::clone(&tripos),
                },
                FormatResource::Tfm {
                    logical_name: format!("{fixture_name}.tfm"),
                    bytes: Arc::clone(&tfm),
                },
            ]
        );
        assert_eq!(
            recipe.distribution_identity.as_ref(),
            b"pinned-trip-public-format-boundary-v1"
        );
        assert_eq!(
            recipe.clock,
            JobClock {
                time: 13 * 60 + 36,
                second: 0,
                day: 9,
                month: 7,
                year: 2026,
            }
        );
        assert_eq!(
            recipe.construction_interaction,
            tex_state::InteractionMode::Nonstop
        );
        assert_eq!(recipe.construction_error_context_widths.error_line(), 64);
        assert_eq!(
            recipe.construction_error_context_widths.half_error_line(),
            32
        );
        assert_eq!(
            recipe.guards,
            FormatGenerationGuards {
                command_fuel: tex_command::DEFAULT_COMMAND_FUEL_LIMIT,
                wall_time: Duration::from_secs(1_800),
                resident_bytes: 6 * 1024 * 1024 * 1024,
            }
        );
        let identity = recipe.identity().expect("recipe identity");
        assert_eq!(
            identity.key(),
            recipe.identity().expect("stable recipe identity").key()
        );
        identities.push(identity.key());
    }
    assert_ne!(
        identities[0], identities[1],
        "TeX82 and e-TeX choices must select disjoint cache identities"
    );
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
        concat!("run_file_in_process_", "captured"),
        concat!("tempfile::", "tempdir"),
        concat!("FormatCacheStore::", "new"),
        concat!("ensure_", "format("),
        ".load(",
        concat!("Universe::from_", "format"),
        concat!(".dump_", "format("),
        concat!("construct_format_", "in_worker"),
        concat!("run_format_", "worker"),
        concat!("CanonicalEngineSession::", "tex82_initex"),
        concat!("Once", "Lock"),
        concat!("Temp", "Dir"),
    ] {
        assert!(
            !helper.contains(forbidden),
            "two-phase helper must not use private format path {forbidden}"
        );
    }
    for required in [
        "trip_format_recipe(",
        "PreparedFormatProvider::from_environment(",
        ".prepare(&recipe)",
        ".construction_evidence()",
        "PreparedFormatJob {",
        "let loaded_run = provider",
        ".run(",
    ] {
        assert!(
            helper.contains(required),
            "two-phase helper must retain public boundary step {required}"
        );
    }
    for (caller, profile) in [
        ("e2e_conformance_trip_canonical", "TripEngineProfile::Tex82"),
        ("e2e_conformance_etrip", "TripEngineProfile::ETex"),
    ] {
        let body = source
            .split_once(&format!("\nfn {caller}()"))
            .unwrap_or_else(|| panic!("full-pipeline caller exists: {caller}"))
            .1
            .split_once("\n}")
            .unwrap_or_else(|| panic!("bounded full-pipeline caller: {caller}"))
            .0;
        assert_eq!(body.matches("run_two_phase_fixture").count(), 1);
        assert!(body.contains(profile), "{caller} selects {profile}");
    }
}

#[test]
fn trip_profiles_reuse_authenticated_provider_entries_and_fresh_jobs() {
    let cache = tempfile::tempdir().expect("scoped provider cache");
    let launcher = super::umber_format_worker_launcher();
    let mut identities = Vec::new();

    for (profile, fixture_name) in [
        (TripEngineProfile::Tex82, "trip"),
        (TripEngineProfile::ETex, "etrip"),
    ] {
        let tripos: Arc<[u8]> = Arc::from(&b"complete input closure"[..]);
        let tfm: Arc<[u8]> = Arc::from(&b"complete TFM closure"[..]);
        let recipe = trip_format_recipe(
            profile,
            fixture_name,
            &format!("{fixture_name}.tex"),
            Arc::from(&b"\\dump\n"[..]),
            Arc::clone(&tripos),
            Arc::clone(&tfm),
        );
        identities.push(recipe.identity().expect("recipe identity").key());
        let first_provider = PreparedFormatProvider::with_store(
            FormatCacheStore::new(cache.path()),
            launcher.clone(),
        );
        let first = first_provider
            .prepare(&recipe)
            .expect("cold profile preparation");
        let second_provider = PreparedFormatProvider::with_store(
            FormatCacheStore::new(cache.path()),
            launcher.clone(),
        );
        let second = second_provider
            .prepare(&recipe)
            .expect("independent warm profile preparation");
        assert_eq!(first.image(), second.image());
        assert_eq!(
            first.construction_evidence(),
            second.construction_evidence(),
            "warm entry must retain authenticated construction evidence"
        );

        for (assignment, expected) in [("\\count0=41\\end\n", 41), ("\\end\n", 0)] {
            let mut observer = TripObservers::default();
            let run = second_provider
                .run(
                    &second,
                    PreparedFormatJob {
                        engine: recipe.engine,
                        engine_binary: recipe.engine.binary_identity(),
                        backend: OutputCapability::Dvi,
                        clock: recipe.clock,
                        interaction: tex_state::InteractionMode::Nonstop,
                        error_context_widths: recipe.construction_error_context_widths,
                        guards: recipe.guards,
                        startup_line: format!("{fixture_name}-provider-control.tex"),
                        source_name: format!("{fixture_name}-provider-control.tex"),
                        source_kind: RegisteredSourceKind::Generated,
                        source: Arc::from(assignment.as_bytes()),
                        resources: vec![
                            LoadedFormatResource::Input {
                                logical_name: "tripos.tex".into(),
                                resolved_name: "./tripos.tex".into(),
                                source_kind: RegisteredSourceKind::Generated,
                                bytes: Arc::clone(&tripos),
                            },
                            LoadedFormatResource::Tfm {
                                logical_name: format!("{fixture_name}.tfm"),
                                bytes: Arc::clone(&tfm),
                            },
                        ],
                        terminal_input: Vec::new(),
                        observer: &mut observer,
                    },
                )
                .expect("fresh loaded provider job");
            assert_eq!(run.universe.count(0), expected);
            assert!(!observer.into_captured().is_empty());
        }
    }

    assert_ne!(identities[0], identities[1]);
    assert_eq!(
        fs::read_dir(cache.path().join("formats-v2"))
            .expect("provider cache namespace")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("sha256-"))
            .count(),
        2,
        "one authenticated entry must be published for each profile identity"
    );
}

const PLAIN_CLOCK: JobClock = JobClock {
    time: 13 * 60 + 36,
    second: 0,
    day: 9,
    month: 7,
    year: 2026,
};

fn plain_guards() -> FormatGenerationGuards {
    FormatGenerationGuards {
        command_fuel: tex_command::DEFAULT_COMMAND_FUEL_LIMIT,
        wall_time: Duration::from_secs(1_800),
        resident_bytes: 6 * 1024 * 1024 * 1024,
    }
}

#[allow(clippy::disallowed_methods)] // Reads only repository-pinned construction inputs.
fn plain_format_recipe(repo_root: &Path) -> Result<FormatRecipe, String> {
    let read = |relative: &str| {
        fs::read(repo_root.join(relative))
            .map(Arc::<[u8]>::from)
            .map_err(|error| format!("read pinned Plain construction resource {relative}: {error}"))
    };
    let mut resources = vec![
        FormatResource::Input {
            logical_name: "plain.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: read("third_party/corpus/plain.tex")?,
        },
        FormatResource::Input {
            logical_name: "hyphen.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: read("third_party/hyphen/hyphen.tex")?,
        },
    ];
    for name in parity_harness::PLAIN_PRELOAD_FONTS {
        resources.push(FormatResource::Tfm {
            logical_name: format!("{name}.tfm"),
            bytes: read(&format!("third_party/fonts/{name}.tfm"))?,
        });
    }
    Ok(FormatRecipe {
        engine: EngineMode::Tex82,
        format_name: "repository-plain-tex82".into(),
        format_ident_name: "repository-plain-tex82".into(),
        construction_source_name: "repository-plain-tex82.ini".into(),
        construction_source: Arc::from(&b"\\input plain.tex\n\\dump\n"[..]),
        resources,
        distribution_identity: Arc::from(&b"repository-pinned-plain-tex82-v1"[..]),
        clock: PLAIN_CLOCK,
        construction_interaction: tex_state::InteractionMode::Nonstop,
        construction_error_context_widths: tex_state::print::ErrorContextWidths::new(64, 32)
            .expect("canonical Plain context widths"),
        guards: plain_guards(),
    })
}

struct PlainJobInput {
    source_name: String,
    source: Arc<[u8]>,
    resources: Vec<LoadedFormatResource>,
}

#[allow(clippy::disallowed_methods)] // Acquires one isolated staged job into typed values.
fn plain_job_input(path: &Path) -> Result<PlainJobInput, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?;
    let source_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| format!("input name is not UTF-8: {}", path.display()))?
        .to_owned();
    let source = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let source = source
        .strip_prefix(b"\\input plain.tex\n")
        .unwrap_or(&source)
        .to_vec();
    let mut entries = fs::read_dir(parent)
        .map_err(|error| format!("read staged directory {}: {error}", parent.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read staged directory entry: {error}"))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut resources = Vec::new();
    for entry in entries {
        if !entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?
            .is_file()
            || entry.path() == path
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "plain.tex" || name == "hyphen.tex" {
            continue;
        }
        let bytes = Arc::from(
            fs::read(entry.path())
                .map_err(|error| format!("read {}: {error}", entry.path().display()))?,
        );
        if let Some(font) = name.strip_suffix(".tfm") {
            if !parity_harness::PLAIN_PRELOAD_FONTS.contains(&font) {
                resources.push(LoadedFormatResource::Tfm {
                    logical_name: name,
                    bytes,
                });
            }
        } else {
            resources.push(LoadedFormatResource::Input {
                logical_name: name.clone(),
                resolved_name: format!("./{name}"),
                source_kind: RegisteredSourceKind::Generated,
                bytes,
            });
        }
    }
    Ok(PlainJobInput {
        source_name,
        source: Arc::from(source),
        resources,
    })
}

fn run_file_with_plain_format(path: &Path) -> Result<InProcessRun, String> {
    let repo_root = test_support::repository_root();
    let recipe = plain_format_recipe(&repo_root)?;
    let provider = PreparedFormatProvider::from_environment(super::umber_format_worker_launcher())
        .map_err(|error| format!("Plain persistent format provider failed: {error}"))?;
    let prepared = provider
        .prepare(&recipe)
        .map_err(|error| format!("Plain format preparation failed: {error}"))?;
    let input = plain_job_input(path)?;
    let source_name = input.source_name.clone();
    let source = Arc::clone(&input.source);
    let mut observers = TripObservers::default();
    let loaded = provider
        .run(
            &prepared,
            PreparedFormatJob {
                engine: EngineMode::Tex82,
                engine_binary: tex_exec::EngineBinaryIdentity::Tex82,
                backend: OutputCapability::Dvi,
                clock: PLAIN_CLOCK,
                interaction: tex_state::InteractionMode::Nonstop,
                error_context_widths: tex_state::print::ErrorContextWidths::new(64, 32)
                    .expect("canonical Plain context widths"),
                guards: plain_guards(),
                startup_line: source_name.clone(),
                source_name: source_name.clone(),
                source_kind: RegisteredSourceKind::Generated,
                source: Arc::clone(&source),
                resources: input.resources,
                terminal_input: Vec::new(),
                observer: &mut observers,
            },
        )
        .map_err(|error| format!("Plain loaded job failed: {error}"))?;
    let dvi = (!loaded.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded.result.dvi_pages))
        .transpose()
        .map_err(|error| error.to_string())?;
    let (terminal, log) = transcript_channels(&loaded.universe, &loaded.result.effects);
    Ok(InProcessRun {
        dvi,
        provenance: loaded.universe.provenance_stats(),
        macro_provenance: loaded.universe.macro_invocation_provenance_stats(),
        terminal,
        log,
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: source_name,
                source: loaded.root_source,
                bytes: source,
            },
            observations: observers.into_captured(),
            outcome: LiveSessionOutcome::Completed,
        }),
    })
}

#[allow(clippy::disallowed_methods)] // Acquires one isolated staged raw TeX82 job.
fn run_file_with_raw_tex82_format(path: &Path) -> Result<InProcessRun, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?;
    let source_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| format!("input name is not UTF-8: {}", path.display()))?
        .to_owned();
    let source: Arc<[u8]> = fs::read(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?
        .into();
    let mut entries = fs::read_dir(parent)
        .map_err(|error| format!("read staged directory {}: {error}", parent.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read staged directory entry: {error}"))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut resources = Vec::new();
    for entry in entries {
        if !entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?
            .is_file()
            || entry.path() == path
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let bytes: Arc<[u8]> = fs::read(entry.path())
            .map_err(|error| format!("read {}: {error}", entry.path().display()))?
            .into();
        if name.ends_with(".tfm") {
            resources.push(LoadedFormatResource::Tfm {
                logical_name: name,
                bytes,
            });
        } else if name.ends_with(".tex") || name.ends_with(".inc") {
            resources.push(LoadedFormatResource::Input {
                logical_name: name.clone(),
                resolved_name: format!("./{name}"),
                source_kind: RegisteredSourceKind::Generated,
                bytes,
            });
        }
    }
    let recipe = FormatRecipe::raw_tex82();
    let provider = PreparedFormatProvider::from_environment(super::umber_format_worker_launcher())
        .map_err(|error| format!("raw TeX82 persistent format provider failed: {error}"))?;
    let prepared = provider
        .prepare(&recipe)
        .map_err(|error| format!("raw TeX82 format preparation failed: {error}"))?;
    let mut observers = TripObservers::default();
    let loaded = provider
        .run(
            &prepared,
            PreparedFormatJob {
                engine: EngineMode::Tex82,
                engine_binary: tex_exec::EngineBinaryIdentity::Tex82,
                backend: OutputCapability::Dvi,
                clock: PLAIN_CLOCK,
                interaction: tex_state::InteractionMode::Nonstop,
                error_context_widths: tex_state::print::ErrorContextWidths::new(64, 32)
                    .expect("canonical raw TeX82 context widths"),
                guards: plain_guards(),
                startup_line: source_name.clone(),
                source_name: source_name.clone(),
                source_kind: RegisteredSourceKind::Generated,
                source: Arc::clone(&source),
                resources,
                terminal_input: Vec::new(),
                observer: &mut observers,
            },
        )
        .map_err(|error| format!("raw TeX82 loaded job failed: {error}"))?;
    let dvi = (!loaded.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded.result.dvi_pages))
        .transpose()
        .map_err(|error| error.to_string())?;
    let (terminal, log) = transcript_channels(&loaded.universe, &loaded.result.effects);
    Ok(InProcessRun {
        dvi,
        provenance: loaded.universe.provenance_stats(),
        macro_provenance: loaded.universe.macro_invocation_provenance_stats(),
        terminal,
        log,
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: source_name,
                source: loaded.root_source,
                bytes: source,
            },
            observations: observers.into_captured(),
            outcome: LiveSessionOutcome::Completed,
        }),
    })
}

#[test]
#[allow(clippy::disallowed_methods)] // Verifies repository-pinned fixture bytes.
fn plain_recipe_has_exact_pinned_ordered_closure_and_stable_identity() {
    let repo_root = test_support::repository_root();
    let first = plain_format_recipe(&repo_root).expect("complete Plain recipe");
    let second = plain_format_recipe(&repo_root).expect("repeat complete Plain recipe");
    assert_eq!(first.engine, EngineMode::Tex82);
    assert_eq!(first.format_name, "repository-plain-tex82");
    assert_eq!(first.construction_source_name, "repository-plain-tex82.ini");
    assert_eq!(
        first.construction_source.as_ref(),
        b"\\input plain.tex\n\\dump\n"
    );
    assert!(
        !fs::read(repo_root.join("third_party/corpus/plain.tex"))
            .expect("pinned plain.tex")
            .windows(b"\\dump".len())
            .any(|window| window == b"\\dump"),
        "the recipe-owned dump must be the only Plain construction dump"
    );
    assert_eq!(first.clock, PLAIN_CLOCK);
    assert_eq!(
        first.construction_interaction,
        tex_state::InteractionMode::Nonstop
    );
    assert_eq!(first.construction_error_context_widths.error_line(), 64);
    assert_eq!(
        first.construction_error_context_widths.half_error_line(),
        32
    );
    assert_eq!(first.guards, plain_guards());
    assert_eq!(
        first.resources.len(),
        2 + parity_harness::PLAIN_PRELOAD_FONTS.len()
    );
    assert!(matches!(
        &first.resources[0],
        FormatResource::Input { logical_name, bytes, .. }
            if logical_name == "plain.tex"
                && bytes.as_ref() == fs::read(repo_root.join("third_party/corpus/plain.tex"))
                    .expect("pinned plain.tex")
    ));
    assert!(matches!(
        &first.resources[1],
        FormatResource::Input { logical_name, bytes, .. }
            if logical_name == "hyphen.tex"
                && bytes.as_ref() == fs::read(repo_root.join("third_party/hyphen/hyphen.tex"))
                    .expect("pinned hyphen.tex")
    ));
    for (resource, name) in first.resources[2..]
        .iter()
        .zip(parity_harness::PLAIN_PRELOAD_FONTS)
    {
        assert!(matches!(
            resource,
            FormatResource::Tfm { logical_name, bytes }
                if logical_name == &format!("{name}.tfm")
                    && bytes.as_ref() == fs::read(repo_root.join(format!("third_party/fonts/{name}.tfm")))
                        .expect("pinned Plain preload TFM")
        ));
    }
    assert_eq!(
        first.identity().expect("Plain identity").key(),
        second.identity().expect("stable Plain identity").key()
    );
    let helper = include_str!("e2e_conformance.rs")
        .split_once("fn plain_format_recipe(")
        .expect("Plain recipe helper")
        .1
        .split_once("\nstruct PlainJobInput")
        .expect("bounded Plain recipe helper")
        .0;
    for forbidden in ["locate_tfm", "kpsewhich", "Command::new", "http", "network"] {
        assert!(
            !helper.contains(forbidden),
            "Plain recipe must not discover {forbidden}"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // Builds one isolated host-side staged job.
fn plain_job_split_types_non_preload_resources() {
    let temp = tempfile::tempdir().expect("temporary staged job");
    fs::write(
        temp.path().join("texput.tex"),
        b"\\input plain.tex\n\\input support.tex\n",
    )
    .expect("root");
    fs::write(temp.path().join("plain.tex"), b"format-only").expect("plain");
    fs::write(temp.path().join("hyphen.tex"), b"format-only").expect("hyphen");
    fs::write(temp.path().join("cmr10.tfm"), b"preloaded").expect("preloaded tfm");
    fs::write(temp.path().join("extra.tfm"), b"job tfm").expect("job tfm");
    fs::write(temp.path().join("support.tex"), b"job input").expect("job input");
    let input = plain_job_input(&temp.path().join("texput.tex")).expect("typed Plain job");
    assert_eq!(input.source.as_ref(), b"\\input support.tex\n");
    assert_eq!(
        input.resources,
        vec![
            LoadedFormatResource::Tfm {
                logical_name: "extra.tfm".into(),
                bytes: Arc::from(&b"job tfm"[..]),
            },
            LoadedFormatResource::Input {
                logical_name: "support.tex".into(),
                resolved_name: "./support.tex".into(),
                source_kind: RegisteredSourceKind::Generated,
                bytes: Arc::from(&b"job input"[..]),
            },
        ]
    );
}

#[test]
fn document_routes_use_plain_while_self_contained_dvi_routes_use_raw_tex82() {
    let source = include_str!("e2e_conformance.rs");
    for removed_definition in [
        concat!("fn staged_", "world("),
        concat!("fn run_file_in_process_", "captured("),
        concat!("struct StagedDir", "ResourceHost"),
        concat!("fn canonical_error_", "message("),
    ] {
        assert!(
            !source.contains(removed_definition),
            "dead full-pipeline bootstrap definition remains: {removed_definition}"
        );
    }
    let repo_root = test_support::repository_root();
    for route in [
        "e2e_conformance_story",
        "e2e_conformance_gentle",
        "e2e_conformance_story_canonical",
        "e2e_conformance_gentle_canonical",
    ] {
        let body = source
            .split_once(&format!("\nfn {route}()"))
            .unwrap_or_else(|| panic!("route {route} exists"))
            .1
            .split_once("\n}")
            .expect("bounded route body")
            .0;
        assert!(body.contains("run_plain_fixture_case"));
    }
    for route in [
        "canonical_ligature_group_boundaries_match_reference_dvi",
        "canonical_rule_space_factor_reset_matches_reference_dvi",
        "canonical_alignment_leading_tabskip_matches_reference_dvi",
        "canonical_rule_follows_pending_characters_in_reference_dvi",
        "canonical_relax_breaks_ligatures_in_reference_dvi",
        "canonical_display_equation_number_preserves_formula_dvi",
        "canonical_math_group_singleton_ord_matches_reference_dvi",
    ] {
        let body = source
            .split_once(&format!("\nfn {route}()"))
            .unwrap_or_else(|| panic!("route {route} exists"))
            .1
            .split_once("\n}")
            .expect("bounded route body")
            .0;
        assert!(body.contains("run_file_in_process_canonical"));
    }
    let raw_canonical = source
        .split_once("\nfn run_file_in_process_canonical(")
        .expect("raw canonical runner")
        .1
        .split_once("\n}")
        .expect("bounded raw canonical runner")
        .0;
    assert!(raw_canonical.contains("run_file_with_raw_tex82_format"));
    assert!(!raw_canonical.contains("run_file_with_plain_format"));
    let plain_canonical = source
        .split_once("\nfn run_file_in_process_plain_canonical(")
        .expect("Plain canonical runner")
        .1
        .split_once("\n}")
        .expect("bounded Plain canonical runner")
        .0;
    assert!(plain_canonical.contains("run_file_with_plain_format"));
    assert!(!plain_canonical.contains("run_file_with_raw_tex82_format"));
    assert_ne!(
        FormatRecipe::raw_tex82()
            .identity()
            .expect("raw identity")
            .key(),
        plain_format_recipe(&repo_root)
            .expect("Plain recipe")
            .identity()
            .expect("Plain identity")
            .key(),
        "raw TeX82 and Plain construction identities must remain disjoint"
    );
    let raw = FormatRecipe::raw_tex82();
    assert_eq!(raw.format_name, "raw-tex82");
    assert_eq!(raw.construction_source.as_ref(), b"\\dump\n");
    assert!(
        raw.resources.is_empty(),
        "raw TeX82 must not hide Plain inputs"
    );
    let shared = source
        .split_once("\nfn run_file_with_plain_format(")
        .expect("shared Plain runner")
        .1
        .split_once("\n#[test]\n#[allow(clippy::disallowed_methods)] // Verifies repository-pinned fixture bytes.\nfn plain_recipe")
        .expect("bounded shared Plain runner")
        .0;
    for required in [
        "plain_format_recipe(&repo_root)",
        "PreparedFormatProvider::from_environment(",
        ".prepare(&recipe)",
        "PreparedFormatJob {",
        ".run(",
    ] {
        assert!(
            shared.contains(required),
            "shared Plain runner requires {required}"
        );
    }
    for forbidden in [
        concat!("staged_", "world("),
        concat!("StagedDir", "ResourceHost"),
        concat!("tex82_", "initex"),
        concat!("run_file_in_process_", "captured"),
        concat!("Universe::from_", "format"),
        concat!("dump_", "format"),
        concat!("FormatCacheStore::", "new"),
        concat!("run_format_", "worker"),
        concat!("CanonicalEngineSession::", "tex82_initex"),
        concat!("Once", "Lock"),
        concat!("Temp", "Dir"),
    ] {
        assert!(
            !shared.contains(forbidden),
            "shared Plain runner forbids {forbidden}"
        );
    }
}

#[test]
fn plain_provider_reuses_one_authenticated_construction_with_fresh_jobs() {
    let repo_root = test_support::repository_root();
    let recipe = plain_format_recipe(&repo_root).expect("complete Plain recipe");
    let cache = tempfile::tempdir().expect("isolated persistent Plain cache");
    let launcher = super::umber_format_worker_launcher();
    let first_provider =
        PreparedFormatProvider::with_store(FormatCacheStore::new(cache.path()), launcher.clone());
    let first = first_provider
        .prepare(&recipe)
        .expect("cold Plain preparation");
    let second_provider =
        PreparedFormatProvider::with_store(FormatCacheStore::new(cache.path()), launcher);
    let second = second_provider
        .prepare(&recipe)
        .expect("independent warm Plain preparation");
    assert_eq!(
        recipe.identity().expect("Plain identity").key(),
        plain_format_recipe(&repo_root)
            .expect("same Plain route recipe")
            .identity()
            .expect("same Plain identity")
            .key()
    );
    assert_eq!(first.image(), second.image());
    assert_eq!(
        first.construction_evidence(),
        second.construction_evidence()
    );
    for image in [first.image(), second.image()] {
        let loaded = Universe::from_format(World::memory(), image)
            .expect("Plain image reconstructs without construction provenance");
        assert_eq!(loaded.provenance_stats().origin_records(), 0);
        assert_eq!(loaded.macro_invocation_provenance_stats().invocations(), 0);
    }
    assert_eq!(
        fs::read_dir(cache.path().join("formats-v2"))
            .expect("Plain provider namespace")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("sha256-"))
            .count(),
        1,
        "all Plain routes must publish exactly one construction identity"
    );
    for (source, expected) in [
        (b"\\count0=41\\end\n".as_slice(), 41),
        (b"\\end\n".as_slice(), 1),
    ] {
        let mut observer = TripObservers::default();
        let run = second_provider
            .run(
                &second,
                PreparedFormatJob {
                    engine: EngineMode::Tex82,
                    engine_binary: tex_exec::EngineBinaryIdentity::Tex82,
                    backend: OutputCapability::Dvi,
                    clock: PLAIN_CLOCK,
                    interaction: tex_state::InteractionMode::Nonstop,
                    error_context_widths: recipe.construction_error_context_widths,
                    guards: plain_guards(),
                    startup_line: "plain-provider-isolation.tex".into(),
                    source_name: "plain-provider-isolation.tex".into(),
                    source_kind: RegisteredSourceKind::Generated,
                    source: Arc::from(source),
                    resources: Vec::new(),
                    terminal_input: Vec::new(),
                    observer: &mut observer,
                },
            )
            .expect("fresh Plain loaded job");
        assert_eq!(run.universe.count(0), expected);
        assert!(!observer.into_captured().is_empty());
    }

    let mut repeated = Vec::new();
    for (provider, fixture) in [(&first_provider, &first), (&second_provider, &second)] {
        let mut observer = TripObservers::default();
        let run = provider
            .run(
                fixture,
                PreparedFormatJob {
                    engine: EngineMode::Tex82,
                    engine_binary: tex_exec::EngineBinaryIdentity::Tex82,
                    backend: OutputCapability::Dvi,
                    clock: PLAIN_CLOCK,
                    interaction: tex_state::InteractionMode::Nonstop,
                    error_context_widths: recipe.construction_error_context_widths,
                    guards: plain_guards(),
                    startup_line: "plain-provider-provenance-isolation.tex".into(),
                    source_name: "plain-provider-provenance-isolation.tex".into(),
                    source_kind: RegisteredSourceKind::Generated,
                    source: Arc::from(&b"\\def\\x{a}\\x\\end\n"[..]),
                    resources: Vec::new(),
                    terminal_input: Vec::new(),
                    observer: &mut observer,
                },
            )
            .expect("fresh repeated Plain loaded job");
        repeated.push((
            run.universe.provenance_stats(),
            run.universe.macro_invocation_provenance_stats(),
        ));
    }
    assert!(repeated[0].1.invocations() > 0);
    assert_eq!(repeated[0].1, repeated[1].1);
    assert!(
        repeated[0].0.retained_layout_eq(repeated[1].0),
        "cold-entry and cache-hit jobs must retain identical job-owned provenance: {:?} vs {:?}",
        repeated[0].0,
        repeated[1].0,
    );
}

fn run_plain_fixture_case(document: &str, gate: &GateAssets) {
    let fixture_name = gate.name;
    run_named_fixture_document(&gate.repo_root, document, &gate.oracle, |path| {
        let run = run_file_with_plain_format(path)?;
        let macro_stats = run.macro_provenance;
        let invocations = macro_stats.invocations();
        if invocations == 0 {
            return Err(format!("{document} executed no macro invocations"));
        }
        let macro_bytes = macro_stats.retained_bytes();
        let bytes_per_invocation = macro_stats.bytes_per_invocation();
        let layout_budget = run.provenance.origin_record_layout_budget_bytes();
        eprintln!(
            "{fixture_name} provenance: invocations={invocations} macro_retained_bytes={macro_bytes} observed_bytes_per_invocation={bytes_per_invocation} origin_record_retained_bytes={} origin_record_layout_budget_bytes={layout_budget} total_retained_bytes={} components={:?}",
            run.provenance.origin_record_retained_bytes(),
            run.provenance.retained_bytes(), run.provenance,
        );
        if run.provenance.origin_record_slot_bytes() > 64 {
            return Err(format!(
                "{document} archived provenance slot is {} bytes (admission charge: 64)",
                run.provenance.origin_record_slot_bytes(),
            ));
        }
        if run.provenance.origin_record_archive_chunk_slots() != 1024 {
            return Err(format!(
                "{document} provenance archive chunk has {} slots (layout contract: 1024)",
                run.provenance.origin_record_archive_chunk_slots(),
            ));
        }
        if run.provenance.origin_key_lease_slots() != 256 {
            return Err(format!(
                "{document} provenance key lease has {} slots (layout contract: 256)",
                run.provenance.origin_key_lease_slots(),
            ));
        }
        let key_run_budget = run
            .provenance
            .origin_records()
            .div_ceil(run.provenance.origin_key_lease_slots());
        if run.provenance.origin_key_runs() > key_run_budget {
            return Err(format!(
                "{document} provenance retained {} affine key runs (fresh-job lease budget: {key_run_budget})",
                run.provenance.origin_key_runs(),
            ));
        }
        if run.provenance.origin_record_retained_bytes() > layout_budget {
            return Err(format!(
                "{document} origin-record containers retained {} bytes (derived layout budget: {layout_budget})",
                run.provenance.origin_record_retained_bytes(),
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

/// Runs one self-contained staged fixture as a fresh job loaded from the
/// shared persistent raw-TeX82 format and returns its assembled DVI bytes.
fn run_file_in_process_canonical(path: &Path) -> Result<Vec<u8>, String> {
    run_file_with_raw_tex82_format(path)?
        .dvi
        .ok_or_else(|| "canonical Umber run did not produce DVI".to_owned())
}

fn run_file_in_process_plain_canonical(path: &Path) -> Result<Vec<u8>, String> {
    run_file_with_plain_format(path)?
        .dvi
        .ok_or_else(|| "canonical Umber run did not produce DVI".to_owned())
}

fn run_plain_fixture_case_canonical(document: &str, gate: &GateAssets) {
    run_named_fixture_document(
        &gate.repo_root,
        document,
        &gate.oracle,
        run_file_in_process_plain_canonical,
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
    let actual_command = run.capture.command(fixture_name, phase, &expected_command);
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
fn run_two_phase_fixture(
    profile: TripEngineProfile,
    source_name: &str,
    local_name: &str,
    gate: &GateAssets,
) {
    let root = &gate.repo_root;
    let fixture_name = gate.name;
    let fixture = &gate.oracle;
    let source = root.join("third_party/trip").join(source_name);

    let source_bytes = fs::read(&source).expect("read conformance source");
    let source_bytes = if profile == TripEngineProfile::ETex {
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
    let recipe = trip_format_recipe(
        profile,
        fixture_name,
        source_identity.canonical_name(),
        Arc::clone(&source_bytes),
        Arc::clone(&tripos),
        Arc::clone(&tfm),
    );
    let engine = recipe.engine;
    let provider = PreparedFormatProvider::from_environment(super::umber_format_worker_launcher())
        .unwrap_or_else(|error| {
            panic!("{fixture_name} persistent format provider failed: {error}")
        });
    let prepared = provider
        .prepare(&recipe)
        .unwrap_or_else(|error| panic!("{fixture_name} format preparation failed: {error}"));
    let format = prepared.image().to_vec();
    let initex_identity = format!("sha256:{:x}", Sha256::digest(&format));
    let initial = InProcessRun {
        dvi: None,
        provenance: ProvenanceStats::default(),
        macro_provenance: MacroInvocationProvenanceStats::default(),
        terminal: Vec::new(),
        log: Vec::new(),
        capture: PhaseCapture::Detached(prepared.construction_evidence().clone()),
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
    let loaded_run = provider
        .run(
            &prepared,
            PreparedFormatJob {
                engine,
                engine_binary: engine.binary_identity(),
                backend: OutputCapability::Dvi,
                clock: recipe.clock,
                interaction: tex_state::InteractionMode::Nonstop,
                error_context_widths: recipe.construction_error_context_widths,
                guards: recipe.guards,
                startup_line: format!(
                    "&{} {}",
                    recipe.format_ident_name,
                    source_identity.canonical_name()
                ),
                source_name: source_identity.canonical_name().to_owned(),
                source_kind: RegisteredSourceKind::Generated,
                source: Arc::clone(&source_bytes),
                resources,
                terminal_input: Vec::new(),
                observer: &mut observers,
            },
        )
        .unwrap_or_else(|error| panic!("{fixture_name} format-loaded run failed: {error}"));
    let dvi = (!loaded_run.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded_run.result.dvi_pages))
        .transpose()
        .expect("serialize loaded DVI");
    let (terminal, log) = transcript_channels(&loaded_run.universe, &loaded_run.result.effects);
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
#[ignore = "manual direct canonical TRIP parity; xfail front: umber2-johp.560"]
fn e2e_conformance_trip_canonical() {
    assets::with_gate("trip", |gate| {
        run_two_phase_fixture(TripEngineProfile::Tex82, "trip.tex", "trip.tex", gate);
    });
}

#[test]
#[ignore = "manual full-document e-TRIP parity; run through scripts/trip.sh"]
fn e2e_conformance_etrip() {
    assets::with_gate("etrip", |gate| {
        run_two_phase_fixture(
            TripEngineProfile::ETex,
            "etrip.tex",
            "etrip-local.tex",
            gate,
        );
    });
}
