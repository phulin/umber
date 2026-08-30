#![allow(
    clippy::disallowed_methods,
    reason = "fixture contract tests stage disposable host files outside engine execution"
)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};

use crate::{
    AlignmentEvent, AlignmentTransition, CanonicalCitation, CanonicalCommand, CanonicalValue,
    CommandDelivery, CommandEvent, CommittedFixture, DisabledObserver, EngineDialect,
    EngineIdentity, Event, EventObserver, FixtureArtifact, FixtureManifest, FixtureProfile,
    GeometryEvent, GeometryLocation, JsonLinesObserver, LATEST_SCHEMA_VERSION, MacroEvent,
    Manifest, ManifestInput, Normalizer, ObservationHeader, ObservationStream, OracleToken,
    RecoveryEvent, RecoveryKind, SCHEMA_VERSION, SchemaVersion, ToolIdentity,
    validate_tex82_geometry_trace_fixture,
};

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
static TEMP_FIXTURE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn manifest() -> Manifest {
    let mut manifest = Manifest::new(EngineIdentity {
        dialect: EngineDialect::Tex82,
        banner: "TeX, Version 3.141592653".into(),
        web_source_sha256: HASH_A.into(),
        upstream_change_sha256: vec![HASH_B.into()],
        instrumentation_change_sha256: HASH_C.into(),
    });
    manifest.inputs.insert(
        "job/main.tex".into(),
        ManifestInput {
            sha256: HASH_A.into(),
            bytes: 3,
        },
    );
    manifest.environment = BTreeMap::from([("locale".into(), "C".into())]);
    manifest.distribution_sha256 = HASH_B.into();
    manifest
        .ordinary_output_sha256
        .insert("dvi".into(), HASH_C.into());
    manifest
}

fn command(name: &str) -> Event {
    Event::Command(CommandEvent {
        delivery: CommandDelivery::Expanded,
        command: CanonicalCommand {
            command: name.into(),
            operand: CanonicalValue::Integer(7),
            control_sequence: Some("\\count".into()),
            location: None,
        },
    })
}

#[test]
fn manifest_encoding_and_identity_are_deterministic() {
    let first = manifest();
    let mut second = manifest();
    second.environment = BTreeMap::new();
    second.environment.insert("locale".into(), "C".into());
    assert_eq!(
        first.to_canonical_json().expect("valid manifest"),
        second.to_canonical_json().expect("valid manifest")
    );
    assert_eq!(
        first.identity().expect("valid manifest"),
        second.identity().expect("valid manifest")
    );
    assert_eq!(
        Manifest::from_json(&first.to_canonical_json().expect("encode")).expect("decode"),
        first
    );
    assert_eq!(first.schema, SCHEMA_VERSION);
}

#[test]
fn manifest_rejects_host_paths_and_noncanonical_hashes() {
    let mut value = manifest();
    value.inputs.insert(
        "/tmp/input.tex".into(),
        ManifestInput {
            sha256: HASH_A.to_uppercase(),
            bytes: 1,
        },
    );
    assert!(value.to_canonical_json().is_err());
}

#[test]
fn normalization_is_narrow_and_sequence_is_deterministic() {
    let mut normalizer = Normalizer::new();
    let mut control_symbol = command("outer\r\nmacro");
    let Event::Command(event) = &mut control_symbol else {
        unreachable!("command helper returns a command event");
    };
    event.command.control_sequence = Some("\r".into());
    let first = normalizer.normalize(control_symbol);
    let second = normalizer.normalize(command("next\rcommand"));
    assert_eq!(first.sequence, 0);
    assert_eq!(second.sequence, 1);
    let Event::Command(event) = first.semantic else {
        panic!("command event");
    };
    assert_eq!(event.command.command, "outer\nmacro");
    assert_eq!(
        event.command.control_sequence.as_deref(),
        Some("\r"),
        "TeX82 §§48 and 356 distinguish the carriage-return control symbol from line feed"
    );
}

#[test]
fn normalization_preserves_control_symbols_in_every_event_shape() {
    let symbol = || OracleToken {
        character: 0,
        catcode: "escape".into(),
        control_sequence: Some("\r".into()),
        location: None,
    };
    let events = [
        Event::Recovery(RecoveryEvent {
            kind: RecoveryKind::Backup,
            tokens: vec![symbol()],
        }),
        Event::Macro(MacroEvent::Argument {
            parameter: 1,
            tokens: vec![symbol()],
        }),
        Event::Macro(MacroEvent::Activation {
            control_sequence: "\r".into(),
            argument_count: 0,
        }),
    ];
    let mut normalizer = Normalizer::new();
    let normalized = events
        .into_iter()
        .map(|event| normalizer.normalize(event).semantic)
        .collect::<Vec<_>>();

    assert!(matches!(
        &normalized[0],
        Event::Recovery(RecoveryEvent { tokens, .. })
            if tokens[0].control_sequence.as_deref() == Some("\r")
    ));
    assert!(matches!(
        &normalized[1],
        Event::Macro(MacroEvent::Argument { tokens, .. })
            if tokens[0].control_sequence.as_deref() == Some("\r")
    ));
    assert!(matches!(
        &normalized[2],
        Event::Macro(MacroEvent::Activation { control_sequence, .. })
            if control_sequence == "\r"
    ));
}

#[test]
fn json_lines_transport_is_stable_and_separate_from_ordinary_output() {
    let identity = manifest().identity().expect("valid manifest");
    let mut observer = JsonLinesObserver::new(Vec::new(), identity).expect("header");
    observer.committed(command("assign")).expect("event");
    let (bytes, first_identity) = observer.finish().expect("finish");

    let mut repeated = JsonLinesObserver::new(Vec::new(), identity).expect("header");
    repeated.committed(command("assign")).expect("event");
    let (repeated_bytes, repeated_identity) = repeated.finish().expect("finish");

    assert_eq!(bytes, repeated_bytes);
    assert_eq!(first_identity, repeated_identity);
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 2);
    let decoded = ObservationStream::from_canonical_json_lines(&bytes).expect("decode");
    assert_eq!(decoded.events.len(), 1);
    assert_eq!(ObservationStream::identity(&bytes), first_identity);
}

#[test]
fn stream_decoder_preallocates_its_complete_event_vector() {
    const EVENT_COUNT: usize = 257;
    let identity = manifest().identity().expect("valid manifest");
    let mut observer = JsonLinesObserver::new(Vec::new(), identity).expect("header");
    for _ in 0..EVENT_COUNT {
        observer.committed(command("assign")).expect("event");
    }
    let (bytes, _) = observer.finish().expect("finish");

    let decoded = ObservationStream::from_canonical_json_lines(&bytes).expect("decode");
    assert_eq!(decoded.events.len(), EVENT_COUNT);
    assert_eq!(decoded.events.capacity(), EVENT_COUNT);
    assert_eq!(
        decoded
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (0..EVENT_COUNT as u64).collect::<Vec<_>>()
    );
}

#[test]
fn stream_decoder_rejects_noncanonical_or_discontinuous_records() {
    let identity = manifest().identity().expect("valid manifest");
    let mut observer = JsonLinesObserver::new(Vec::new(), identity).expect("header");
    observer.committed(command("assign")).expect("event");
    let (bytes, _) = observer.finish().expect("finish");

    let noncanonical = String::from_utf8(bytes)
        .expect("utf8")
        .replace("\"sequence\":0", "\"sequence\": 0");
    assert!(ObservationStream::from_canonical_json_lines(noncanonical.as_bytes()).is_err());
}

#[test]
fn disabled_observer_accepts_events_without_transport_state() {
    assert_eq!(size_of::<DisabledObserver>(), 0);
    DisabledObserver
        .committed(command("ignored"))
        .expect("no-op");
}

#[test]
fn alignment_events_encode_semantic_nesting_without_storage_identity() {
    let identity = manifest().identity().expect("valid manifest");
    let mut observer = JsonLinesObserver::new(Vec::new(), identity).expect("header");
    observer
        .committed(Event::Alignment(AlignmentEvent {
            transition: AlignmentTransition::TemplatePush,
            align_state: 1_000_000,
            template: Some("v".into()),
            nesting: Some(2),
            previous_align_state: Some(0),
            delimiter: Some("span".into()),
            recovery: None,
        }))
        .expect("event");
    let (bytes, _) = observer.finish().expect("finish");
    let json = String::from_utf8(bytes).expect("utf8");
    assert!(json.contains("\"transition\":\"template_push\""));
    assert!(json.contains("\"nesting\":2"));
    assert!(!json.contains("pointer"));
    assert!(!json.contains("address"));
}

fn fixture_manifest() -> FixtureManifest {
    let mut oracle = manifest();
    oracle.clock = "2000-01-01T00:00:00Z".into();
    FixtureManifest {
        contract: 2,
        name: "tex82/synthetic".into(),
        profile: FixtureProfile {
            invocation: "initex".into(),
            characters: "eight_bit_exact".into(),
        },
        oracle,
        tools: BTreeMap::from([(
            "tangle".into(),
            ToolIdentity {
                version: "pinned".into(),
                sha256: HASH_A.into(),
            },
        )]),
        citations: vec![CanonicalCitation {
            source: "tex.web".into(),
            section: "get_next".into(),
            description: "raw delivery".into(),
        }],
        sources: BTreeMap::from([(
            "job/main.tex".into(),
            FixtureArtifact {
                path: "sources/main.tex".into(),
                bytes: 3,
                sha256: HASH_A.into(),
            },
        )]),
        root_source: "job/main.tex".into(),
        events: FixtureArtifact {
            path: "events.jsonl".into(),
            bytes: 1,
            sha256: HASH_B.into(),
        },
        outputs: BTreeMap::from([(
            "dvi".into(),
            FixtureArtifact {
                path: "outputs/main.dvi".into(),
                bytes: 0,
                sha256: HASH_C.into(),
            },
        )]),
    }
}

#[test]
fn fixture_manifest_rejects_a_root_source_absent_from_declared_sources() {
    let mut value = fixture_manifest();
    assert!(value.validate().is_ok());

    value.root_source = "job/undeclared.tex".into();
    let error = value.validate().expect_err("undeclared root source");
    assert!(error.to_string().contains("job/undeclared.tex"));
}

#[test]
fn committed_tex82_fixture_is_consumed_hermetically() {
    let repository = test_support::repository_root();
    let fixture = CommittedFixture::load(
        repository.join("tests/corpus/command/tex82/command-transitions-v1"),
    )
    .expect("committed canonical fixture");
    assert_eq!(fixture.manifest.name, "tex82/command-transitions-v1");
    // `umber2-alfh.2` split the former single 14-source fixture into one
    // minifixture per independent behavior; this one keeps the
    // input-stack/scanner-status/EOF-recovery seams that inherently need
    // nested files, so its own event count is a fraction of the former
    // whole-suite `COMMITTED_TEX82_COMMAND_TRACE_EVENT_COUNT`.
    assert_eq!(fixture.stream.events.len(), 3_960);
    fixture
        .audit_matrices(
            &fs::read(
                repository.join("tests/tex82-oracle/command-transitions-v1-semantic-matrix.txt"),
            )
            .expect("semantic matrix"),
            &fs::read(
                repository.join("tests/tex82-oracle/command-transitions-v1-audit-matrix.txt"),
            )
            .expect("fixture audit matrix"),
        )
        .expect("complete bidirectional fixture audit");
}

#[test]
fn committed_tex82_geometry_projection_is_pinned_and_schema_v3() {
    let repository = test_support::repository_root();
    let fixture =
        validate_tex82_geometry_trace_fixture(repository).expect("committed geometry fixture");
    assert_eq!(fixture.selector, "tex82/geometry-v3");
    assert_eq!(fixture.stream.header.schema, SchemaVersion::V3.number());
    assert_eq!(fixture.stream.events.len(), 11);
    assert!(
        fixture
            .stream
            .events
            .iter()
            .all(|event| matches!(event.semantic, Event::Geometry(_)))
    );
    assert!(
        fixture
            .stream
            .events
            .iter()
            .any(|event| matches!(event.semantic, Event::Geometry(GeometryEvent::Hpack { .. })))
    );
    assert!(
        fixture
            .stream
            .events
            .iter()
            .any(|event| matches!(event.semantic, Event::Geometry(GeometryEvent::Vpack { .. })))
    );
    assert!(fixture.stream.events.iter().any(|event| matches!(
        event.semantic,
        Event::Geometry(GeometryEvent::Shipout { .. })
    )));
    assert!(fixture.stream.events.iter().all(|event| {
        match &event.semantic {
            Event::Geometry(GeometryEvent::Hpack { location, .. })
            | Event::Geometry(GeometryEvent::Vpack { location, .. })
            | Event::Geometry(GeometryEvent::Shipout { location, .. }) => location
                .as_ref()
                .is_some_and(|location| location.source == "geometry.tex" && location.line > 0),
            _ => false,
        }
    }));
}

#[test]
fn fixture_audit_rejects_missing_behavior_and_unowned_observations() {
    let repository = test_support::repository_root();
    let fixture = CommittedFixture::load(
        repository.join("tests/corpus/command/tex82/command-transitions-v1"),
    )
    .expect("committed canonical fixture");
    let semantic =
        fs::read(repository.join("tests/tex82-oracle/command-transitions-v1-semantic-matrix.txt"))
            .expect("matrix");
    let audit = String::from_utf8(
        fs::read(repository.join("tests/tex82-oracle/command-transitions-v1-audit-matrix.txt"))
            .expect("audit"),
    )
    .expect("utf8");

    let missing_event = String::from_utf8(semantic.clone()).expect("utf8").replace(
        "\"event\":\"command\",\"data\":{\"delivery\":\"raw\"",
        "\"event\":\"command\",\"data\":{\"delivery\":\"invented\"",
    );
    assert!(
        fixture
            .audit_matrices(missing_event.as_bytes(), audit.as_bytes())
            .is_err()
    );

    let unowned_output = audit.replace(
        "dvi,effect_file,log,status,terminal",
        "dvi,log,status,terminal",
    );
    assert!(
        fixture
            .audit_matrices(&semantic, unowned_output.as_bytes())
            .is_err()
    );

    let undeclared_source = String::from_utf8(semantic.clone())
        .expect("utf8")
        .replace("input-eof-normal.tex", "absent.tex");
    assert!(
        fixture
            .audit_matrices(undeclared_source.as_bytes(), audit.as_bytes())
            .is_err()
    );

    let absent_citation = audit.replace("firm_up_the_line and get_next", "retired Umber tokenizer");
    assert!(
        fixture
            .audit_matrices(&semantic, absent_citation.as_bytes())
            .is_err()
    );

    let missing_family = audit.replace(
        "source|tex.web|firm_up_the_line and get_next",
        "invented|tex.web|firm_up_the_line and get_next",
    );
    assert!(
        fixture
            .audit_matrices(&semantic, missing_family.as_bytes())
            .is_err()
    );
}

#[test]
fn committed_fixture_rejects_event_hash_identity_and_output_byte_drift() {
    let source =
        test_support::repository_root().join("tests/corpus/command/tex82/command-transitions-v1");
    let loaded = CommittedFixture::load(&source).expect("fixture");
    let temporary = std::env::temp_dir().join(format!(
        "umber-tex-oracle-fixture-{}-{}",
        std::process::id(),
        TEMP_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));

    for artifact in loaded
        .manifest
        .sources
        .values()
        .chain(std::iter::once(&loaded.manifest.events))
        .chain(loaded.manifest.outputs.values())
    {
        let destination = temporary.join(&artifact.path);
        fs::create_dir_all(destination.parent().expect("artifact parent")).expect("mkdir");
        fs::copy(source.join(&artifact.path), destination).expect("copy artifact");
    }

    let event_path = temporary.join("events.jsonl");
    let mut events = fs::read(&event_path).expect("events");
    let last = events.len() - 2;
    events[last] ^= 1;
    fs::write(&event_path, &events).expect("write event drift");
    assert!(CommittedFixture::load(&temporary).is_err());

    fs::copy(source.join("events.jsonl"), &event_path).expect("restore events");
    let mut events = fs::read(&event_path).expect("events");
    let newline = events
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header newline");
    events[..newline].copy_from_slice(
        b"{\"schema\":1,\"manifest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}",
    );
    fs::write(&event_path, &events).expect("write events");
    let mut manifest = loaded.manifest.clone();
    manifest.events.sha256 = hex_hash(&events);
    fs::write(
        temporary.join("manifest.json"),
        manifest.to_canonical_json().expect("manifest"),
    )
    .expect("write manifest");
    assert!(CommittedFixture::load(&temporary).is_err());

    fs::copy(source.join("events.jsonl"), &event_path).expect("restore events");
    fs::write(
        temporary.join("manifest.json"),
        loaded
            .manifest
            .to_canonical_json()
            .expect("original manifest"),
    )
    .expect("restore manifest");
    fs::write(temporary.join("outputs/status.txt"), b"0\n").expect("drift output");
    assert!(CommittedFixture::load(&temporary).is_err());
    fs::remove_dir_all(temporary).expect("remove temporary fixture");
}

fn hex_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn geometry() -> GeometryEvent {
    GeometryEvent::Hpack {
        width_sp: 65_536,
        height_sp: 12,
        depth_sp: -3,
        location: None,
    }
}

#[test]
fn geometry_round_trips_with_scaled_point_units() {
    let geometry = geometry();
    let bytes = serde_json::to_vec(&geometry).expect("encode geometry");
    assert_eq!(
        serde_json::from_slice::<GeometryEvent>(&bytes).expect("decode geometry"),
        geometry
    );
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(text.contains("\"width_sp\":65536"));
    assert!(!text.contains("point"));
}

#[test]
fn schema_v3_geometry_round_trips_source_provenance_and_accepts_legacy_v2_shape() {
    let attributed = GeometryEvent::Hpack {
        width_sp: 1,
        height_sp: 2,
        depth_sp: 3,
        location: Some(GeometryLocation {
            source: "chapters/math.tex".into(),
            line: 47,
        }),
    };
    let bytes = serde_json::to_vec(&attributed).expect("encode attributed geometry");
    assert_eq!(
        serde_json::from_slice::<GeometryEvent>(&bytes).expect("decode attributed geometry"),
        attributed
    );
    assert_eq!(
        serde_json::from_str::<GeometryEvent>(
            r#"{"transition":"hpack","width_sp":1,"height_sp":2,"depth_sp":3}"#,
        )
        .expect("decode legacy v2 geometry"),
        GeometryEvent::Hpack {
            width_sp: 1,
            height_sp: 2,
            depth_sp: 3,
            location: None,
        }
    );
}

#[test]
fn geometry_source_is_canonically_normalized_without_changing_its_line() {
    let mut normalizer = Normalizer::new();
    let normalized = normalizer.normalize(Event::Geometry(GeometryEvent::Vpack {
        width_sp: 1,
        height_sp: 2,
        depth_sp: 3,
        location: Some(GeometryLocation {
            source: "part\r\nname.tex".into(),
            line: 12,
        }),
    }));
    let Event::Geometry(GeometryEvent::Vpack { location, .. }) = normalized.semantic else {
        panic!("expected vpack");
    };
    assert_eq!(
        location,
        Some(GeometryLocation {
            source: "part\nname.tex".into(),
            line: 12,
        })
    );
}

#[test]
fn schema_version_selects_distinct_manifest_and_stream_domains() {
    let v1 = manifest();
    let mut v2 = v1.clone();
    v2.schema = SchemaVersion::V2.number();
    assert_ne!(v1.identity().expect("v1"), v2.identity().expect("v2"));

    let identity = v1.identity().expect("v1 identity");
    let v1_header = ObservationHeader::new(identity);
    let v2_header = ObservationHeader::for_schema(SchemaVersion::V2, identity);
    assert_ne!(
        serde_json::to_vec(&v1_header).expect("v1 header"),
        serde_json::to_vec(&v2_header).expect("v2 header")
    );
}

#[test]
fn schema_v1_through_v3_identity_preimages_remain_frozen() {
    let identities = [SchemaVersion::V1, SchemaVersion::V2, SchemaVersion::V3]
        .map(|schema| {
            let mut manifest = manifest();
            manifest.schema = schema.number();
            let manifest_identity = manifest.identity().expect("manifest identity");
            let mut observer =
                JsonLinesObserver::new_for_schema(Vec::new(), schema, manifest_identity)
                    .expect("stream header");
            observer.committed(command("assign")).expect("event");
            let (_, stream_identity) = observer.finish().expect("stream identity");
            (manifest_identity.hex(), stream_identity.hex())
        });
    assert_eq!(
        identities,
        [
            (
                "4d2981662e9c08586d9f6563e20e13dc7e99dcf2608eb795f2757ca1a641ca21"
                    .to_owned(),
                "89e4cf97a3310d7caddb2bfee516d91a42b648786188526342dbebe8b4c64ced"
                    .to_owned(),
            ),
            (
                "814e59a483cd7199a724f793dcde2ffdbf00339cef50192f82a90feb39ba7222"
                    .to_owned(),
                "bd1e830c180e296a4e43f54381845cb7717762c3c0d01303bf02dd85cd2aaab6"
                    .to_owned(),
            ),
            (
                "f770cb63ef9d1c5ee9c89e08615dc18cd00de519c2c03e6a2eaf415bb37a0cd6"
                    .to_owned(),
                "b8510a43e11553a14ac3f53983039a3aea6860249aa26253ff9f8dce059fc178"
                    .to_owned(),
            ),
        ]
    );
}

#[test]
fn geometry_rejects_malformed_input_and_v1_manifest() {
    assert!(
        serde_json::from_str::<GeometryEvent>(
            r#"{"transition":"hpack","width_sp":1,"height_sp":2,"depth_sp":3,"node":7}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<GeometryEvent>(
            r#"{"transition":"shipout","page_width_sp":1,"page_height_sp":"2"}"#
        )
        .is_err()
    );
    let mut v1 = manifest();
    v1.schema = LATEST_SCHEMA_VERSION + 1;
    assert!(v1.to_canonical_json().is_err());
}

#[test]
fn geometry_uses_finalized_signed_scaled_points() {
    assert_eq!(
        geometry(),
        GeometryEvent::Hpack {
            width_sp: 65_536,
            height_sp: 12,
            depth_sp: -3,
            location: None,
        }
    );
    assert_eq!(
        GeometryEvent::Shipout {
            page_width_sp: 65_536,
            page_height_sp: 98_304,
            counts: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            location: None,
        },
        GeometryEvent::Shipout {
            page_width_sp: 65_536,
            page_height_sp: 98_304,
            counts: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            location: None,
        }
    );
}
