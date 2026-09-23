//! Shared phase support for the conformance families.

use super::*;

#[allow(clippy::disallowed_methods)] // Host-side fixture staging and artifact comparison.
pub(super) fn compare_trip_phase(
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
        PhaseLogContract::EtripLoadedLogProjection => {
            expected_log_projection = etrip_official::normalize_loaded_log(expected_log)
                .expect("normalize e-TRIP oracle loaded log");
            actual_log_projection = etrip_official::normalize_loaded_log(actual_log)
                .expect("normalize e-TRIP actual loaded log");
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
pub(super) enum PhaseParityContract {
    DumpConstruction,
    OutputProducing,
}

pub(super) struct PhaseComparison<'a> {
    dvi_pair: Option<(&'a [u8], &'a [u8])>,
    contract: PhaseParityContract,
    log_contract: PhaseLogContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PhaseLogContract {
    Exact,
    /// e-TRIP's final storage counters describe WEB memory/string/font-table
    /// representations, and its same-run output framing has a host path
    /// spelling difference. The official artifact comparator applies this
    /// same narrow loaded-log projection.
    EtripLoadedLogProjection,
}

impl PhaseParityContract {
    pub(super) fn text_channel<'a>(
        self,
        expected: &'a [u8],
        actual: &'a [u8],
    ) -> (&'a [u8], &'a [u8]) {
        match self {
            Self::DumpConstruction => (&[], &[]),
            Self::OutputProducing => (expected, actual),
        }
    }
}

#[allow(clippy::disallowed_methods)] // Failure reporting reads the bounded triage artifact.
pub(super) fn assert_trip_channels_match(verdict: &parity_harness::TripTriageVerdict) {
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

pub(super) fn assert_format_image_contract(format: &[u8], engine: EngineMode) {
    let image = tex_state::DetachedFormatImage::try_from_bytes(format.to_vec())
        .expect("validated detached format image");
    let mut host_world = World::memory();
    host_world
        .set_memory_file("host-only-capability.tex", b"host-only".to_vec())
        .expect("stage host-only capability");
    for world in [host_world, World::memory()] {
        let image = tex_state::DetachedFormatImage::try_from_bytes(image.as_bytes().to_vec())
            .expect("validated detached format image copy");
        tex_state::with_materialized_format(
            tex_state::EngineCapacityProfile::Texlive2026.interner_budget(),
            world,
            image,
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
                engine
                    .install_after_format(loaded)
                    .expect("valid format activation");
                assert!(
                    loaded.primitive_meaning("relax").is_some(),
                    "format loading reconstructs the selected engine registry"
                );
            },
        )
        .expect("materialize format contract fixture");
    }
}

#[allow(clippy::disallowed_methods)] // Host-side fixture staging and artifact comparison.
pub(super) fn run_two_phase_fixture(
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
                bytes: Arc::<[u8]>::from(source_bytes).into(),
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
                PhaseLogContract::EtripLoadedLogProjection
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
