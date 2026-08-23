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
use tex_command::RegisteredSourceKind;
use tex_observe::{
    GeometryEvidenceProfile, LiveSessionOutcome, LiveSessionStreams, LiveSessionTranslator,
    LiveSource, SemanticEvidenceProfile,
};
use tex_oracle::{ObservationStream, SchemaVersion};
use tex_state::{EffectRecord, PrintSink, ProvenanceDemand};
use tex_state::{JobClock, World};

use umber::FormatCacheStore;
use umber::{
    EngineMode, FormatGenerationGuards, FormatRecipe, FormatResource, LoadedFormatProjectionDemand,
    LoadedFormatResource, OutputCapability, PreparedFormatJob, PreparedFormatProvider,
    dvi_from_page_plans,
};

#[path = "e2e_conformance/assets.rs"]
mod assets;
#[path = "e2e_conformance/etrip_official.rs"]
mod etrip_official;

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
    terminal: Vec<u8>,
    log: Vec<u8>,
    capture: PhaseCapture,
}

enum PhaseCapture {
    Live(LiveCapture),
    Detached(tex_oracle::OracleBundle),
}

impl PhaseCapture {
    fn command(&self, fixture_name: &str, phase: &str, oracle: &[u8]) -> Vec<u8> {
        command_stream_for_fixture_phase(fixture_name, phase, self.streams(oracle))
    }

    fn streams(&self, oracle: &[u8]) -> LiveSessionStreams {
        match self {
            Self::Live(capture) => capture.streams(oracle),
            Self::Detached(evidence) => {
                let diagnostic =
                    tex_oracle::canonical_bundle_json_lines(&evidence.semantic, oracle)
                        .expect("construction semantic evidence encodes under oracle header");
                LiveSessionStreams {
                    diagnostic: diagnostic.clone(),
                    stable: diagnostic,
                }
            }
        }
    }

    fn geometry(&self, oracle: &[u8]) -> Vec<u8> {
        match self {
            Self::Live(capture) => capture.geometry(oracle),
            Self::Detached(evidence) => {
                tex_oracle::canonical_bundle_json_lines(&evidence.geometry, oracle)
                    .expect("construction geometry evidence encodes under oracle header")
            }
        }
    }
}

#[test]
fn detached_geometry_uses_the_pinned_schema_three_header() {
    let oracle = b"{\"schema\":3,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let capture = PhaseCapture::Detached(tex_oracle::OracleBundle {
        semantic: Vec::new(),
        geometry: vec![tex_oracle::NormalizedEvent {
            sequence: 0,
            semantic: tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp: 1,
                height_sp: 2,
                depth_sp: 3,
                location: Some(tex_oracle::GeometryLocation {
                    source: "trip.tex".into(),
                    line: 105,
                }),
            }),
        }],
    });
    let actual = capture.geometry(oracle);
    let stream = ObservationStream::from_canonical_json_lines(&actual)
        .expect("schema-v3 detached geometry stream");

    assert_eq!(stream.header.schema, SchemaVersion::V3.number());
    assert_eq!(stream.events.len(), 1);
}

#[test]
fn trip_construction_evidence_is_fresh_complete_and_canonical() {
    let source =
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source");
    let tripos = test_support::read_repository_asset("third_party/trip/tripos.tex")
        .expect("read TRIP terminal input");
    let tfm = test_support::read_repository_asset("third_party/trip/trip.tfm")
        .expect("read TRIP font metrics");
    let recipe = trip_format_recipe(
        TripEngineProfile::Tex82,
        "trip",
        "trip.tex",
        source,
        tripos,
        tfm,
    );
    // A private empty store makes this a construction-path regression. A
    // process-global warm entry would authenticate old detached evidence and
    // never execute the current command engine at all.
    let cache = tempfile::tempdir().expect("isolated TRIP format cache");
    let provider = PreparedFormatProvider::with_store(
        FormatCacheStore::new(cache.path()),
        super::umber_format_worker_launcher(),
    );
    let prepared = provider.prepare(&recipe).expect("focused TRIP format");
    let oracle = b"{\"schema\":3,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let semantic =
        tex_oracle::canonical_bundle_json_lines(&prepared.construction_evidence().semantic, oracle)
            .expect("actual construction semantics validate as schema v3");
    let semantic_stream =
        ObservationStream::from_canonical_json_lines(&semantic).expect("semantic stream");
    assert_eq!(semantic_stream.events.len(), 8707);
    let semantic_payload = semantic
        .splitn(2, |byte| *byte == b'\n')
        .nth(1)
        .expect("semantic stream has a header and events");
    assert_eq!(
        format!("{:x}", Sha256::digest(semantic_payload)),
        // Producer contract 15 initializes tex.web §241's job clock during
        // the fresh INITEX construction episode. That canonical state
        // mutation is part of the detached semantic stream and therefore of
        // this whole-payload pin.
        "953fe73c75581f20c25efe18457b4ccddaa609e95df4f121671cb8375451124a"
    );
    let event = |sequence: usize| &semantic_stream.events[sequence].semantic;
    assert!(matches!(
        event(3451),
        tex_oracle::Event::Command(command)
            if command.delivery == tex_oracle::CommandDelivery::Raw
                && command.command.command == "the"
    ));
    assert!(matches!(
        event(3452),
        tex_oracle::Event::Command(command)
            if command.delivery == tex_oracle::CommandDelivery::Raw
                && command.command.command == "assign_toks"
                && command.command.operand == tex_oracle::CanonicalValue::Integer(25058)
                && command.command.control_sequence.as_deref() == Some("output")
                && command.command.location.as_ref().is_some_and(|location|
                    location.source == "trip.tex" && location.line == 60 && location.byte == 21)
    ));
    assert!(matches!(
        event(3453),
        tex_oracle::Event::Command(command)
            if command.delivery == tex_oracle::CommandDelivery::Expanded
                && command.command.command == "assign_toks"
                && command.command.control_sequence.as_deref() == Some("output")
    ));
    assert!(matches!(
        event(4660),
        tex_oracle::Event::Command(command)
            if command.delivery == tex_oracle::CommandDelivery::Raw
                && command.command.command == "letter"
                && command.command.operand == tex_oracle::CanonicalValue::Integer(65)
                && command.command.location.as_ref().is_some_and(|location|
                    location.source == "trip.tex" && location.line == 77 && location.byte == 13)
    ));
    assert!(matches!(
        event(4661),
        tex_oracle::Event::Command(command)
            if command.delivery == tex_oracle::CommandDelivery::Raw
                && command.command.command == "assign_int"
                && command.command.operand == tex_oracle::CanonicalValue::Integer(27219)
                && command.command.control_sequence.as_deref() == Some("righthyphenmin")
                && command.command.location.as_ref().is_some_and(|location|
                    location.source == "trip.tex" && location.line == 77 && location.byte == 28)
    ));
    assert!(matches!(
        event(4662),
        tex_oracle::Event::Command(command)
            if command.delivery == tex_oracle::CommandDelivery::Expanded
                && command.command.command == "assign_int"
                && command.command.control_sequence.as_deref() == Some("righthyphenmin")
    ));
    let actual =
        tex_oracle::canonical_bundle_json_lines(&prepared.construction_evidence().geometry, oracle)
            .expect("actual construction geometry validates as schema v3");
    let stream = ObservationStream::from_canonical_json_lines(&actual).expect("geometry stream");
    let mut hpack = 0;
    let mut vpack = 0;
    let mut shipout = 0;
    for event in &stream.events {
        match &event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                location: Some(location),
                ..
            }) => {
                hpack += 1;
                assert_eq!(location.source, "trip.tex");
                assert!(location.line > 0);
            }
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Vpack {
                location: Some(location),
                ..
            }) => {
                vpack += 1;
                assert_eq!(location.source, "trip.tex");
                assert!(location.line > 0);
            }
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Shipout {
                location: Some(location),
                ..
            }) => {
                shipout += 1;
                assert_eq!(location.source, "trip.tex");
                assert!(location.line > 0);
            }
            event => panic!("unattributed construction geometry: {event:?}"),
        }
    }
    assert_eq!((hpack, vpack, shipout), (4, 4, 0));
}

struct LiveCapture {
    root: LiveSource,
    observations: Vec<tex_command::CommandObservation>,
    outcome: LiveSessionOutcome,
}

fn trip_geometry_profile(schema: SchemaVersion) -> GeometryEvidenceProfile {
    if schema >= SchemaVersion::V3 {
        GeometryEvidenceProfile::Located
    } else {
        GeometryEvidenceProfile::Positionless
    }
}

fn command_stream_for_fixture_phase(
    fixture_name: &str,
    phase: &str,
    streams: LiveSessionStreams,
) -> Vec<u8> {
    if fixture_name == "trip" && phase == "format-loaded" {
        streams.stable
    } else {
        streams.diagnostic
    }
}

impl LiveCapture {
    fn streams(&self, oracle: &[u8]) -> LiveSessionStreams {
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
        let header = ObservationStream::from_canonical_json_lines(oracle)
            .expect("oracle geometry stream validates")
            .header;
        let schema = SchemaVersion::try_from(header.schema).expect("supported geometry schema");
        let geometry_profile = trip_geometry_profile(schema);
        let mut translator = LiveSessionTranslator::for_root(schema, "terminal", self.root.clone());
        translator.translate_captured(self.observations.iter().cloned());
        tex_oracle::canonical_bundle_json_lines(
            &translator
                .finalize_profile(SemanticEvidenceProfile::Complete, geometry_profile)
                .geometry,
            oracle,
        )
        .expect("geometry observations translate")
    }
}

#[test]
fn trip_geometry_profile_follows_the_pinned_oracle_schema() {
    assert_eq!(
        trip_geometry_profile(SchemaVersion::V2),
        GeometryEvidenceProfile::Positionless
    );
    assert_eq!(
        trip_geometry_profile(SchemaVersion::V3),
        GeometryEvidenceProfile::Located
    );
}

fn transcript_channels<G>(
    stores: &tex_state::Universe<G>,
    effects: &[EffectRecord],
) -> (Vec<u8>, Vec<u8>) {
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
    umber::with_engine_world(World::memory(), |stores| {
        stores
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "shared-prefix:");
        stores
            .publish_effect_prefix(stores.world().effect_pos())
            .expect("commit shared prefix");
        stores
            .world_mut()
            .write_text(PrintSink::Terminal, "terminal-prefix:");
        stores.world_mut().write_text(PrintSink::Log, "log-prefix:");
        stores
            .publish_effect_prefix(stores.world().effect_pos())
            .expect("commit per-channel prefixes");
        stores
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "shared-tail");
        stores
            .world_mut()
            .write_text(PrintSink::Terminal, "-terminal");
        stores.world_mut().write_text(PrintSink::Log, "-log");

        let effects = stores.world().effect_records().to_vec();
        let (terminal, log) = transcript_channels(stores, &effects);

        assert_eq!(
            terminal,
            b"shared-prefix:terminal-prefix:shared-tail-terminal"
        );
        assert_eq!(log, b"shared-prefix:log-prefix:shared-tail-log");
    })
    .expect("fresh transcript-capture universe");
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
            LiveSessionStreams {
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
    source: Vec<u8>,
    tripos: Vec<u8>,
    tfm: Vec<u8>,
) -> FormatRecipe {
    let mut recipe = profile.recipe();
    // Knuth's TRIP build deliberately selects this non-production `hyph_size`.
    recipe.hyphenation_exception_capacity = 659;
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
    recipe.distribution_identity = b"pinned-trip-public-format-boundary-v2".to_vec();
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
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_and_etrip_recipes_select_typed_public_format_inputs() {
    let source = b"fixture source".to_vec();
    let tripos = b"tripos".to_vec();
    let tfm = b"tfm".to_vec();
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
            source.clone(),
            tripos.clone(),
            tfm.clone(),
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
                    bytes: tripos.clone(),
                },
                FormatResource::Tfm {
                    logical_name: format!("{fixture_name}.tfm"),
                    bytes: tfm.clone(),
                },
            ]
        );
        assert_eq!(
            recipe.distribution_identity.as_slice(),
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
        concat!("EngineSession::", "tex82_initex"),
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
        let tripos = b"complete input closure".to_vec();
        let tfm = b"complete TFM closure".to_vec();
        let recipe = trip_format_recipe(
            profile,
            fixture_name,
            &format!("{fixture_name}.tex"),
            b"\\dump\n".to_vec(),
            tripos.clone(),
            tfm.clone(),
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
                        provenance_demand: ProvenanceDemand::DIAGNOSTICS,
                        guards: recipe.guards,
                        startup_line: format!("{fixture_name}-provider-control.tex"),
                        source_name: format!("{fixture_name}-provider-control.tex"),
                        source_kind: RegisteredSourceKind::Generated,
                        source: assignment.as_bytes().to_vec(),
                        resources: vec![
                            LoadedFormatResource::Input {
                                logical_name: "tripos.tex".into(),
                                resolved_name: "./tripos.tex".into(),
                                source_kind: RegisteredSourceKind::Generated,
                                bytes: tripos.clone(),
                            },
                            LoadedFormatResource::Tfm {
                                logical_name: format!("{fixture_name}.tfm"),
                                bytes: tfm.clone(),
                            },
                        ],
                        terminal_input: Vec::new(),
                        projection: LoadedFormatProjectionDemand {
                            count_registers: vec![0],
                            ..LoadedFormatProjectionDemand::default()
                        },
                        observer: &mut observer,
                    },
                )
                .expect("fresh loaded provider job");
            assert_eq!(run.projection.counts, [(0, expected)]);
            assert!(!observer.into_captured().is_empty());
        }
    }

    assert_ne!(identities[0], identities[1]);
    assert_eq!(
        fs::read_dir(cache.path().join("blobs-v1"))
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
        hyphenation_exception_capacity: 307,
        format_name: "repository-plain-tex82".into(),
        format_ident_name: "repository-plain-tex82".into(),
        construction_source_name: "repository-plain-tex82.ini".into(),
        construction_source: b"\\input plain.tex\n\\dump\n".to_vec(),
        resources,
        distribution_identity: b"repository-pinned-plain-tex82-v1".to_vec(),
        clock: PLAIN_CLOCK,
        construction_interaction: tex_state::InteractionMode::Nonstop,
        construction_error_context_widths: tex_state::print::ErrorContextWidths::new(64, 32)
            .expect("canonical Plain context widths"),
        guards: plain_guards(),
    })
}

struct PlainJobInput {
    source_name: String,
    source: Vec<u8>,
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
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("read {}: {error}", entry.path().display()))?;
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
        source,
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
    let source = input.source.clone();
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
                provenance_demand: ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
                guards: plain_guards(),
                startup_line: source_name.clone(),
                source_name: source_name.clone(),
                source_kind: RegisteredSourceKind::Generated,
                source: source.clone(),
                resources: input.resources,
                terminal_input: Vec::new(),
                projection: LoadedFormatProjectionDemand {
                    channels: true,
                    ..LoadedFormatProjectionDemand::default()
                },
                observer: &mut observers,
            },
        )
        .map_err(|error| format!("Plain loaded job failed: {error}"))?;
    let dvi = (!loaded.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded.result.dvi_pages))
        .transpose()
        .map_err(|error| error.to_string())?;
    let channels = loaded
        .projection
        .channels
        .expect("Plain job channel projection");
    Ok(InProcessRun {
        dvi,
        terminal: channels.terminal,
        log: channels.log,
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: source_name,
                source: tex_state::SourceId::new(0),
                bytes: Arc::from(source),
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
    let source = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
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
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("read {}: {error}", entry.path().display()))?;
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
                provenance_demand: ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
                guards: plain_guards(),
                startup_line: source_name.clone(),
                source_name: source_name.clone(),
                source_kind: RegisteredSourceKind::Generated,
                source: source.clone(),
                resources,
                terminal_input: Vec::new(),
                projection: LoadedFormatProjectionDemand {
                    channels: true,
                    ..LoadedFormatProjectionDemand::default()
                },
                observer: &mut observers,
            },
        )
        .map_err(|error| format!("raw TeX82 loaded job failed: {error}"))?;
    let dvi = (!loaded.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded.result.dvi_pages))
        .transpose()
        .map_err(|error| error.to_string())?;
    let channels = loaded
        .projection
        .channels
        .expect("raw TeX82 job channel projection");
    Ok(InProcessRun {
        dvi,
        terminal: channels.terminal,
        log: channels.log,
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: source_name,
                source: tex_state::SourceId::new(0),
                bytes: Arc::from(source),
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
        first.construction_source.as_slice(),
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
    assert_eq!(input.source.as_slice(), b"\\input support.tex\n");
    assert_eq!(
        input.resources,
        vec![
            LoadedFormatResource::Tfm {
                logical_name: "extra.tfm".into(),
                bytes: b"job tfm".to_vec(),
            },
            LoadedFormatResource::Input {
                logical_name: "support.tex".into(),
                resolved_name: "./support.tex".into(),
                source_kind: RegisteredSourceKind::Generated,
                bytes: b"job input".to_vec(),
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
    assert_eq!(raw.construction_source.as_slice(), b"\\dump\n");
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
        concat!("EngineSession::", "tex82_initex"),
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
    assert_eq!(
        fs::read_dir(cache.path().join("blobs-v1"))
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
                    provenance_demand: ProvenanceDemand::DIAGNOSTICS,
                    guards: plain_guards(),
                    startup_line: "plain-provider-isolation.tex".into(),
                    source_name: "plain-provider-isolation.tex".into(),
                    source_kind: RegisteredSourceKind::Generated,
                    source: source.to_vec(),
                    resources: Vec::new(),
                    terminal_input: Vec::new(),
                    projection: LoadedFormatProjectionDemand {
                        count_registers: vec![0],
                        ..LoadedFormatProjectionDemand::default()
                    },
                    observer: &mut observer,
                },
            )
            .expect("fresh Plain loaded job");
        assert_eq!(run.projection.counts, [(0, expected)]);
        assert!(!observer.into_captured().is_empty());
    }
}

fn run_plain_fixture_case(document: &str, gate: &GateAssets) {
    run_named_fixture_document(&gate.repo_root, document, &gate.oracle, |path| {
        let run = run_file_with_plain_format(path)?;
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
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
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
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
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
        log_contract,
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
    fs::write(
        artifact_root.join(format!("{phase}-terminal.txt")),
        &run.terminal,
    )
    .expect("write terminal artifact");
    fs::write(artifact_root.join(format!("{phase}.log")), &run.log).expect("write log artifact");
    let label = format!("{fixture_name}-{phase}");
    let (expected_terminal, actual_terminal) =
        phase_contract.text_channel(&expected_terminal, &run.terminal);
    let (expected_log, actual_log) = phase_contract.text_channel(&expected_log, &run.log);
    let expected_log_projection;
    let actual_log_projection;
    let (expected_log, actual_log) = match log_contract {
        PhaseLogContract::Exact => (expected_log, actual_log),
        PhaseLogContract::EtripRepresentationNeutralEngineUsage => {
            expected_log_projection =
                etrip_official::normalize_loaded_log_engine_usage(expected_log)
                    .expect("normalize e-TRIP oracle engine usage");
            actual_log_projection = etrip_official::normalize_loaded_log_engine_usage(actual_log)
                .expect("normalize e-TRIP actual engine usage");
            (&expected_log_projection[..], &actual_log_projection[..])
        }
    };
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
    log_contract: PhaseLogContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhaseLogContract {
    Exact,
    /// e-TRIP's final storage counters describe WEB memory/string/font-table
    /// representations, not TeX-visible semantics. The official artifact
    /// comparator applies this same narrow projection.
    EtripRepresentationNeutralEngineUsage,
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
    let image = tex_state::DetachedFormatImage::try_from_bytes(format.to_vec())
        .expect("validated detached format image");
    let mut host_world = World::memory();
    host_world
        .set_memory_file("host-only-capability.tex", b"host-only".to_vec())
        .expect("stage host-only capability");
    for world in [host_world, World::memory()] {
        tex_state::with_materialized_format(
            tex_state::interner::InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024)
                .expect("test interner budget"),
            world,
            &image,
            |loaded| {
                assert!(
                    loaded.world().effect_records().is_empty(),
                    "host effects must not enter a materialized format"
                );
                assert_eq!(
                    loaded.primitive_meaning("relax"),
                    None,
                    "primitive registry is runtime state and must not be serialized"
                );
                engine.install_after_format(loaded);
                assert!(
                    loaded.primitive_meaning("relax").is_some(),
                    "format loading reconstructs the selected engine registry"
                );
            },
        )
        .expect("materialize format contract fixture");
    }
}

#[test]
fn format_image_contract_excludes_runtime_state_and_rebuilds_registry() {
    let format = umber::with_engine_universe(|source| {
        EngineMode::Tex82.prepare_initex(source);
        source
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "host effect excluded");
        let mut session =
            umber::EngineSession::prepared_initex(source, tex_command::CommandProfile::TEX82);
        session
            .register_authored_job("format.tex", Arc::from(&b"\\dump"[..]))
            .expect("format contract root registers");
        let mut host =
            umber::FileSessionResolvers::new(Path::new("format.tex"), Vec::new(), Vec::new());
        session
            .run(&mut host, &mut Vec::new())
            .expect("format contract construction")
            .format_dump
            .expect("bounded format image")
            .image
            .into_bytes()
    })
    .expect("fresh format-contract universe");

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
    let tripos =
        fs::read(root.join("third_party/trip/tripos.tex")).expect("read shared TRIP input");
    let tfm = fs::read(root.join("third_party/trip/trip.tfm")).expect("read conformance TFM");
    let recipe = trip_format_recipe(
        profile,
        fixture_name,
        source_identity.canonical_name(),
        source_bytes.clone(),
        tripos.clone(),
        tfm.clone(),
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
            log_contract: PhaseLogContract::Exact,
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
                provenance_demand: ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
                guards: recipe.guards,
                startup_line: format!(
                    "&{} {}",
                    recipe.format_ident_name,
                    source_identity.canonical_name()
                ),
                source_name: source_identity.canonical_name().to_owned(),
                source_kind: RegisteredSourceKind::Generated,
                source: source_bytes.clone(),
                resources,
                terminal_input: Vec::new(),
                projection: LoadedFormatProjectionDemand {
                    channels: true,
                    ..LoadedFormatProjectionDemand::default()
                },
                observer: &mut observers,
            },
        )
        .unwrap_or_else(|error| panic!("{fixture_name} format-loaded run failed: {error}"));
    let dvi = (!loaded_run.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded_run.result.dvi_pages))
        .transpose()
        .expect("serialize loaded DVI");
    let channels = loaded_run
        .projection
        .channels
        .as_ref()
        .expect("loaded TRIP channel projection");
    let terminal = channels.terminal.clone();
    let log = channels.log.clone();
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
        terminal: terminal.clone(),
        log: log.clone(),
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: source_identity.canonical_name().to_owned(),
                source: tex_state::SourceId::new(0),
                bytes: Arc::from(source_bytes),
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
    if profile == TripEngineProfile::ETex {
        let output = channels
            .outputs
            .iter()
            .find(|output| output.path == Path::new("etrip.out"))
            .map(|output| output.bytes.clone())
            .expect("e-TRIP produced etrip.out");
        let initex_log = fs::read(
            target_dir(root)
                .join("trip-oracles/etrip")
                .join("initex.log"),
        )
        .expect("read exact e-TeX 2.6 INITEX log oracle");
        etrip_official::compare(
            root,
            etrip_official::OfficialEtripRun {
                initex_log: &initex_log,
                terminal: &terminal,
                log: &log,
                dvi: &actual_dvi,
                output: &output,
            },
        )
        .unwrap_or_else(|error| panic!("official e-TRIP artifact parity failed: {error}"));
    }
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
            log_contract: if profile == TripEngineProfile::ETex {
                PhaseLogContract::EtripRepresentationNeutralEngineUsage
            } else {
                PhaseLogContract::Exact
            },
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
#[ignore = "manual direct canonical TRIP parity; xfail front: umber2-johp.568"]
fn e2e_conformance_trip_canonical() {
    assets::with_gate("trip", |gate| {
        run_two_phase_fixture(TripEngineProfile::Tex82, "trip.tex", "trip.tex", gate);
    });
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_vadjust_diagnostic_uses_detached_replacement_layout() {
    // Construct the real TRIP format, then retain the loaded-job prefix that
    // establishes the page, paragraph, and three preceding \vadjust states.
    // Replaying INITEX source cannot reproduce the dumped font ligature and
    // hyphenation state responsible for this diagnostic.
    let log = run_focused_loaded_trip_through(203);
    assert!(
        log.contains(concat!(
            "Underfull \\hbox (badness 10000) in paragraph at lines 109--109\n",
            " [] []\\rip BB-B-BBB\n",
        )),
        "{log}"
    );
    assert!(!log.contains(" [] []\\rip BB-BBBB\n"), "{log}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_display_diagnostic_includes_overfull_rule() {
    let log = run_focused_loaded_trip_through(285);
    assert!(
        log.contains(concat!(
            "Overfull \\hbox (48.4746pt too wide) detected at line 193\n",
            "[][][] [] [] []|\n",
        )),
        "{log}"
    );
    assert_eq!(
        log.matches("{horizontal mode: \\expandafter}").count(),
        1,
        "{log}"
    );
    let undefined = log
        .find("{undefined}")
        .unwrap_or_else(|| panic!("undefined command trace:\n{log}"));
    let page = log
        .find("% t=21.7 plus")
        .unwrap_or_else(|| panic!("page-builder trace:\n{log}"));
    assert!(undefined < page, "{log}");
    assert!(
        !log.contains("% t=191.11256 plus 40.0 plus 1.0fil"),
        "{log}"
    );
    assert!(
        log.contains("% t=262.41258 plus 80.0 plus 1.0fil plus -803.0fill g=10000.0 b=0 p=7 c="),
        "{log}"
    );
    assert!(
        log.as_bytes()
            .windows(10)
            .any(|window| window == b"\\bigtr\np -"),
        "{log}"
    );
    assert!(
        !log.as_bytes()
            .windows(10)
            .any(|window| window == b"\\bigtr\0p -"),
        "{log}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_vsplit_diagnostics_freeze_canonical_scan_contexts() {
    let log = run_focused_loaded_trip_through(377);
    assert!(
        log.contains(concat!(
            "! Missing `to' inserted.\n",
            "<to be read again> \n",
            "                   0\n",
            "l.285 ...\\hbox{\\vfill\\vsplit 3 0\n",
            "                                pt}\n",
        )),
        "{log}"
    );
    assert!(
        log.contains(concat!(
            "! \\vsplit needs a \\vbox.\n",
            "<to be read again> \n",
            "                   }\n",
            "l.285 ...ox{\\vfill\\vsplit 3 0pt}\n",
        )),
        "{log}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_deferred_write_condition_replaces_the_final_stack_front() {
    // Exact TRIP source through lines 419 and 441--442. TeX82 §§1370/1335:
    // the deferred write's ordinary `\if` remains above the older selected
    // `\ifcase` and is reported first during final cleanup.
    let log = run_focused_loaded_trip_through(442);
    let condition_reports = || {
        log.lines()
            .filter(|line| line.contains("end occurred when"))
            .collect::<Vec<_>>()
    };
    let write_if = log
        .find("(end occurred when if on line 350 was incomplete)")
        .unwrap_or_else(|| panic!("deferred-write condition: {:?}", condition_reports()));
    let old_ifcase = log
        .find("(end occurred when ifcase on line 327 was incomplete)")
        .unwrap_or_else(|| panic!("older condition: {:?}", condition_reports()));
    assert!(
        write_if < old_ifcase,
        "innermost condition reports first: {:?}",
        condition_reports()
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_character_constant_alignment_template_traces_endv_once() {
    let log = run_focused_loaded_trip_through(337);
    assert!(
        log.contains(
            "Missing character: There is no } in font trip!\n\
             {end of alignment template}\n\
             @firstpass"
        ),
        "the character-constant brace must preserve alignment depth without duplicating end-v"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_delete_last_error_shows_command_context_before_help() {
    // TeX82 §§82/1105: the page-removal apology installs its help and calls
    // `error`; the live command input context is printed before §90's help.
    let log = run_focused_loaded_trip_through(345);
    let message = log
        .rfind("You can't use `\\unpenalty' in vertical mode.")
        .expect("line-345 unpenalty error");
    let report = &log[message..];
    let context = report
        .find("\\lastpenalty\\unpenalty")
        .expect("unpenalty source context");
    let help = report
        .find("Sorry...I usually can't take things from the current page.")
        .expect("delete-last help");
    assert!(context < help, "{report}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_lastbox_error_shows_command_context_before_help() {
    // TeX82 §§82/1081: `begin_box` rejects `\lastbox` on the current page,
    // then §82 prints the still-live command input before §90's help.
    let log = run_focused_loaded_trip_through(346);
    let message = log
        .rfind("You can't use `\\lastbox' in vertical mode.")
        .expect("line-346 lastbox error");
    let report = &log[message..];
    let context = report.find("\\penalty5").expect("lastbox source context");
    let help = report
        .find("Sorry...I usually can't take things from the current page.")
        .expect("lastbox help");
    assert!(context < help, "{report}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_missing_definition_target_recovers_once() {
    // TeX82 §1215's `get_r_token` owns one complete `ins_error` and restart:
    // the rejected `{` supplies the empty definition and `?` resumes normally.
    let log = run_focused_loaded_trip_through(347);
    let lastbox = log
        .rfind("You can't use `\\lastbox' in vertical mode.")
        .expect("line-346 lastbox error");
    let report = &log[lastbox..];
    assert_eq!(
        report
            .matches("! Missing control sequence inserted.")
            .count(),
        1,
        "{report}"
    );
    assert!(report.contains("{the character ?}"), "{report}");
    assert!(
        report.contains("{horizontal mode: the character ?}"),
        "{report}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_invalid_character_error_precedes_following_trace() {
    // TeX82 §§345/367/370/380 complete both expansion-time errors before the
    // next begin-group command reaches tracing.
    let log = run_focused_loaded_trip_through(352);
    let undefined = log
        .rfind("{undefined}")
        .expect("line-351 undefined command trace");
    let report = &log[undefined..];
    let undefined_error = report
        .find("! Undefined control sequence.")
        .expect("undefined-control report");
    let invalid_error = report
        .find("! Text line contains an invalid character.")
        .expect("invalid-character report");
    let following = report
        .find("{begin-group character {}")
        .unwrap_or_else(|| panic!("following begin-group trace:\n{report}"));
    assert!(undefined_error < invalid_error, "{report}");
    assert!(invalid_error < following, "{report}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_tokens_runaway_names_assignment_scanner() {
    // TeX82 §§306/336/1227 retain the current token-register shorthand as
    // `warning_index` while its balanced right-hand side is absorbed.
    let log = run_focused_loaded_trip_through(354);
    assert!(
        log.contains("Forbidden control sequence found while scanning text of \\tokens."),
        "{log}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_runaway_context_escapes_nul_control_sequence_name() {
    // TeX82 §§59/262/315 pseudoprint token-list context through the active
    // selector, so NUL bytes use printable double-caret notation.
    let log = run_focused_loaded_trip_through(354);
    let runaway = log
        .rfind("Forbidden control sequence found while scanning text of \\tokens.")
        .expect("line-354 tokens runaway");
    let report = &log[runaway..];
    assert!(report.contains("\\a^^@^^@a"), "{report}");
    assert!(!report.contains('\0'), "{report:?}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_nested_ifcase_operand_preserves_skip_nesting() {
    // TeX82 §509 keeps skipping while an operand-expanded conditional is
    // above the saved `\ifcase` frame, popping only that newer frame's `\fi`.
    let log = run_focused_loaded_trip_through(359);
    let case_negative = log.rfind("{case -1}").expect("line-359 negative case");
    let report = &log[case_negative..];
    let nested_ifcase = report.find("{\\ifcase}").expect("skipped nested ifcase");
    let nested_fi = report.find("{\\fi}").expect("skipped nested fi");
    let case_five = report.find("{case 5}").expect("outer else-branch case");
    assert!(
        nested_ifcase < nested_fi && nested_fi < case_five,
        "{report}"
    );
    assert!(!report[..case_five].contains("{case 0}"), "{report}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_runaway_preamble_finishes_partial_before_error() {
    // TeX82 §§338--339 prints the already collected preamble token list
    // before `print_err` opens the forbidden-control diagnostic.
    let log = run_focused_loaded_trip_through(363);
    assert!(
        log.contains(
            "Runaway preamble?\n{\n! Forbidden control sequence found while scanning preamble"
        ),
        "{log}"
    );
    let frontier = log
        .rfind("! Incomplete \\ifcase;")
        .expect("line-363 conditional recovery");
    let recovery = &log[frontier..];
    let first = recovery
        .find("Runaway preamble?\n{")
        .expect("line-363 runaway");
    let after_first = &recovery[first + "Runaway preamble?".len()..];
    let missing = after_first
        .find("! Missing # inserted in alignment preamble.")
        .expect("missing-parameter recovery follows runaway");
    assert!(
        !after_first[..missing].contains("Runaway preamble?"),
        "{recovery}"
    );
    assert!(
        recovery.contains("\\lo #1#2U3#4#5#6#7#8#989{"),
        "{recovery}"
    );
    assert!(recovery.contains("\nU3<-.\n"), "{recovery}");
    assert!(!recovery.contains("\\lo #1#2#3#4#5"), "{recovery}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_runaway_definition_pseudoprints_nonstandard_match_marker() {
    let log = run_focused_loaded_trip_through(364);
    let runaway = log.rfind("Runaway definition?").expect("line-364 runaway");
    let report = &log[runaway..];
    assert!(
        report.contains("^^C1->\\d ^^C1\\d \\l {##2}\\l ^^C1\\par"),
        "{report}"
    );
    assert!(!report.contains('\u{3}'), "{report:?}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_macro_mismatch_precedes_following_malformed_invocation_trace() {
    // TeX82 §§82/391 completes \T's compulsory-prefix mismatch before
    // §389 can trace the later malformed \a invocation.
    let log = run_focused_loaded_trip_through(366);
    let mismatch = log
        .rfind("Use of \\T doesn't match its definition")
        .expect("line-366 compulsory-prefix mismatch");
    let report = &log[mismatch..];
    let inserted_context = report
        .find("<inserted text> ")
        .expect("§336 inserted paragraph context");
    let help = report
        .find("If you say, e.g., `\\def\\a1{...}'")
        .expect("§391 mismatch help");
    let following_trace = report
        .find("\\a^^@^^@a #1\\par #2->")
        .expect("following malformed macro trace");
    assert!(
        inserted_context < help && help < following_trace,
        "{report}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_script_pair_dump_uses_a_normal_kern() {
    // Exact TRIP source through lines 438--440 reaches the malformed formula's
    // sup/sub pair and `\showbox9`. TeX82 §§135/158/184 make its generated
    // separator a normal kern, printed without the explicit-subtype space.
    let log = run_focused_loaded_trip_through(440);
    let expected = ".....\\ip /\n.....\\kern12.3\n.....\\hbox(0.0+0.0)x-0.01";
    assert!(log.contains(expected), "script-pair node dump:\n{log}");
    assert!(!log.contains(".....\\kern 12.3"), "{log}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_final_operator_has_one_zero_before_rebox() {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..440].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let natural = (64_881, 0, 0);
    let exact = (64_881, 1_284_506, 0);

    assert!(
        hpacks
            .windows(3)
            .any(|packs| packs == [(0, 0, 0), natural, exact])
    );
    assert!(
        !hpacks
            .windows(4)
            .any(|packs| packs == [(0, 0, 0), (0, 0, 0), natural, exact])
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_hairy_display_preserves_appendix_g_pack_order() {
    // TRIP line 285 exercises TeX82 §§720, 724, 733, and 749 together. Keep
    // the repeated package calls in their canonical order; equal dimensions
    // are distinct completed operations, not deduplication candidates.
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..285].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(hpacks.windows(5).any(|packs| {
        packs
            == [
                (0, 0, 0),
                (0, 0, 0),
                (392_561, 1_120_666, 275_251),
                (392_561, 1_120_666, 275_251),
                (392_561, 1_120_666, 275_251),
            ]
    }));
    assert!(hpacks.windows(12).any(|packs| {
        packs
            == [
                (196_608, 524_288, 131_072),
                (196_608, 524_288, 131_072),
                (131_072, 0, 0),
                (131_072, 0, 0),
                (196_608, 786_432, 0),
                (196_608, 0, 0),
                (196_608, 1_835_008, 0),
                (524_288, 0, 0),
                (524_288, 0, 0),
                (524_288, 458_752, 0),
                (131_072, 0, 0),
                (131_072, 0, 0),
            ]
    }));
    assert!(hpacks.windows(3).any(|packs| {
        packs
            == [
                (393_216, 1_048_576, 262_145),
                (393_216, 1_048_576, 262_145),
                (0, 458_752, 0),
            ]
    }));
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_radical_overbar_uses_normal_kerns() {
    // The full loaded stream is necessary: skipping its pre-format prefix
    // does not reproduce the showbox9 list reached through TRIP lines 438--440.
    // TeX82 §§714/135 create both overbar spacers as normal `new_kern` nodes;
    // §184 consequently prints no explicit-subtype space after `\kern`.
    let source: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let log = run_loaded_trip_source(source);
    let overbar = log.lines().collect::<Vec<_>>().windows(4).any(|lines| {
        lines[0].starts_with("..\\vbox(")
            && lines[1].starts_with("...\\kern")
            && !lines[1].starts_with("...\\kern ")
            && lines[2].starts_with("...\\rule(")
            && lines[3].starts_with("...\\kern")
            && !lines[3].starts_with("...\\kern ")
    });
    assert!(overbar, "normal-kern radical overbar:\n{log}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_empty_operator_box_keeps_axis_shift() {
    // Full format-loaded history is required. Its malformed class-Op noad
    // reaches TeX82 §749 with a missing math character; the resulting empty
    // hbox must still pass through the common math-axis centering step.
    let source: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let log = run_loaded_trip_source(source);
    assert!(
        log.contains(
            ".............\\hbox(0.0+0.0)x0.0, shifted -7.0\n.............\\glue(\\nonscript)"
        ),
        "shifted operator nucleus is absent"
    );
}

fn run_focused_loaded_trip_through(last_source_line: usize) -> String {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..last_source_line].join("\n")).into_bytes());
    run_loaded_trip_source(source)
}

#[test]
fn loaded_immediate_write_traces_expansion_in_no_mode() {
    // TeX82 §§299/367/1370: immediate `write_out` sets `mode:=0` around its
    // expanded scan. A trace inside the scan says `no mode`, and the next
    // main-control trace names the restored vertical mode because §367 left
    // `shown_mode=0`. The malformed delimited macro is TRIP's generic scanner
    // boundary, reduced independently of the rest of the document.
    let source: Arc<[u8]> = Arc::from(
        &b"\\tracingcommands=2\\tracingmacros=2\\tracingonline=1\\long\\def\\l#1\\l{#1}\\immediate\\write10{\\string\\caution \\l}\\escapechar=92\\end\n"[..],
    );
    let log = run_loaded_trip_source(source);
    let string_trace = log
        .find("{no mode: \\string}")
        .unwrap_or_else(|| panic!("§1370 immediate-write trace:\n{log}"));
    let macro_trace = log
        .find("\\l #1\\l ->#1")
        .unwrap_or_else(|| panic!("write macro trace:\n{log}"));
    let runaway = log
        .find("Runaway argument?")
        .unwrap_or_else(|| panic!("write scanner recovery:\n{log}"));

    assert!(string_trace < macro_trace && macro_trace < runaway, "{log}");
    assert!(log.contains("{vertical mode: \\escapechar}"), "{log}");
}

/// TeX82 §1110 distinguishes a void register from a nonempty incompatible
/// register. TRIP line 396's void `\unhbox234` is silent, while nonvoid
/// `\unhcopy3` in math mode reports before §1166 dispatches the following
/// text accent.
#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_math_unboxing_diagnoses_before_following_accent() {
    let log = run_focused_loaded_trip_through(396);
    let incompatible = log
        .find("Incompatible list can't be unboxed")
        .expect("unhcopy diagnostic");
    let accent = log
        .find("Please use \\mathaccent for accents in math mode")
        .expect("following accent diagnostic");

    assert!(incompatible < accent, "diagnostic order:\n{log}");
    assert_eq!(
        log.matches("Incompatible list can't be unboxed").count(),
        1,
        "void unhbox must remain silent:\n{log}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_nested_empty_math_box_does_not_republish_source_hpack() {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..210].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(hpacks.windows(3).any(|packs| {
        packs
            == [
                (7_864_320, 0, 0),
                (7_864_320, 0, 0),
                (6_553_600, 458_752, 65_536),
            ]
    }));
    assert!(
        !hpacks
            .windows(3)
            .any(|packs| { packs == [(7_864_320, 0, 0), (7_864_320, 0, 0), (7_864_320, 0, 0),] })
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_hairy_display_publishes_both_clean_character_packs() {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..285].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(hpacks.windows(4).any(|packs| {
        packs
            == [
                (0, 393_216, 0),
                (196_608, 458_752, 65_536),
                (196_608, 458_752, 65_536),
                (131_072, 589_824, 0),
            ]
    }));
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_missing_accent_publishes_clean_nucleus_pack() {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..396].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(hpacks.windows(5).any(|packs| {
        packs
            == [
                (26_214, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (6_553_600, 0, 0),
            ]
    }));
}

fn run_loaded_trip_source(source: Arc<[u8]>) -> String {
    run_loaded_trip_source_observed(source).0
}

fn positionless_geometry(observer: TripObservers, oracle: &[u8]) -> Vec<u8> {
    let mut translator = LiveSessionTranslator::new("terminal", SchemaVersion::V2);
    translator.translate_captured(observer.into_captured());
    let evidence = translator.finalize_profile(
        SemanticEvidenceProfile::Complete,
        GeometryEvidenceProfile::Positionless,
    );
    tex_oracle::canonical_bundle_json_lines(&evidence.geometry, oracle)
        .expect("focused geometry stream")
}

fn run_loaded_trip_source_observed(source: Arc<[u8]>) -> (String, TripObservers) {
    let trip =
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source");
    let tripos = test_support::read_repository_asset("third_party/trip/tripos.tex")
        .expect("read TRIP terminal input");
    let tfm = test_support::read_repository_asset("third_party/trip/trip.tfm")
        .expect("read TRIP font metrics");
    let recipe = trip_format_recipe(
        TripEngineProfile::Tex82,
        "trip",
        "trip.tex",
        trip,
        tripos.clone(),
        tfm.clone(),
    );
    let provider = PreparedFormatProvider::from_environment(super::umber_format_worker_launcher())
        .expect("focused TRIP format provider");
    let prepared = provider.prepare(&recipe).expect("focused TRIP format");
    let mut observer = TripObservers::default();
    let loaded = provider
        .run(
            &prepared,
            PreparedFormatJob {
                engine: EngineMode::Tex82,
                engine_binary: EngineMode::Tex82.binary_identity(),
                backend: OutputCapability::Dvi,
                clock: recipe.clock,
                interaction: tex_state::InteractionMode::Nonstop,
                error_context_widths: recipe.construction_error_context_widths,
                provenance_demand: ProvenanceDemand::DIAGNOSTICS,
                guards: recipe.guards,
                startup_line: "&trip focused.tex".into(),
                source_name: "focused.tex".into(),
                source_kind: RegisteredSourceKind::Generated,
                source: source.to_vec(),
                resources: vec![
                    LoadedFormatResource::Input {
                        logical_name: "tripos.tex".into(),
                        resolved_name: "./tripos.tex".into(),
                        source_kind: RegisteredSourceKind::Generated,
                        bytes: tripos,
                    },
                    LoadedFormatResource::Tfm {
                        logical_name: "trip.tfm".into(),
                        bytes: tfm,
                    },
                ],
                terminal_input: Vec::new(),
                projection: LoadedFormatProjectionDemand {
                    channels: true,
                    ..LoadedFormatProjectionDemand::default()
                },
                observer: &mut observer,
            },
        )
        .expect("focused loaded TRIP run");
    let channels = loaded
        .projection
        .channels
        .expect("focused TRIP channel projection");
    let (_, log) =
        append_transcript_suffix(channels.terminal, channels.log, &channels.pending_effects);
    (String::from_utf8(log).expect("TRIP log is UTF-8"), observer)
}

#[test]
#[ignore = "manual full-document e-TRIP parity tier"]
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
