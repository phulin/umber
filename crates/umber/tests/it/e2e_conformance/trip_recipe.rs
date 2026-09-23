//! Typed TRIP and e-TRIP recipes, observers, and provider reuse.

use super::*;

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
            b"pinned-trip-public-format-boundary-v2"
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
fn trip_profiles_reuse_verified_provider_entries_and_fresh_jobs() {
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
            "warm entry must retain verified construction evidence"
        );

        for (assignment, expected) in [("\\count0=41\\end\n", 41), ("\\end\n", 0)] {
            let mut observer = CapturedObservations::default();
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
        fs::read_dir(cache.path().join("blobs-v2"))
            .expect("provider cache namespace")
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("ahash64-v1-"))
            .count(),
        2,
        "one verified entry must be published for each profile identity"
    );
}
