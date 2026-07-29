use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{
    CommittedFixture, DiagnosticEvent, DiagnosticSeverity, EngineDialect, EngineIdentity, Event,
    FixtureArtifact, FixtureManifest, FixtureProfile, Manifest, NormalizedEvent, ObservationHeader,
    ObservationStream,
};

use super::{
    MINIFIXTURE_MAX_EVENTS, MINIFIXTURE_MAX_SOURCE_BYTES, MINIFIXTURE_MAX_SOURCES,
    validate_minifixture_budget,
};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// `source_bytes` is the size given to *each* declared source; the fixture's
/// total is `sources * source_bytes`.
fn synthetic_fixture(sources: usize, source_bytes: u64, events: usize) -> CommittedFixture {
    let mut oracle = Manifest::new(EngineIdentity {
        dialect: EngineDialect::Tex82,
        banner: "TeX, Version 3.141592653".into(),
        web_source_sha256: HASH.into(),
        upstream_change_sha256: vec![HASH.into()],
        instrumentation_change_sha256: HASH.into(),
    });
    oracle.clock = "2026-07-09T13:36:00Z".into();
    oracle.distribution_sha256 = HASH.into();
    oracle.environment.insert("locale".into(), "C".into());
    oracle
        .ordinary_output_sha256
        .insert("status".into(), HASH.into());

    let mut source_map = BTreeMap::new();
    for index in 0..sources {
        source_map.insert(
            format!("source-{index}.tex"),
            FixtureArtifact {
                path: format!("sources/source-{index}.tex"),
                bytes: source_bytes,
                sha256: HASH.into(),
            },
        );
    }
    let manifest = FixtureManifest {
        contract: 1,
        name: "tex82/synthetic-budget".into(),
        profile: FixtureProfile {
            invocation: "initex".into(),
            characters: "eight_bit_exact".into(),
        },
        oracle,
        tools: BTreeMap::new(),
        citations: Vec::new(),
        sources: source_map,
        events: FixtureArtifact {
            path: "events.jsonl".into(),
            bytes: 0,
            sha256: HASH.into(),
        },
        outputs: BTreeMap::from([(
            "status".into(),
            FixtureArtifact {
                path: "outputs/status.txt".into(),
                bytes: 0,
                sha256: HASH.into(),
            },
        )]),
    };

    let stream_events = (0..events)
        .map(|sequence| NormalizedEvent {
            sequence: sequence as u64,
            semantic: Event::Diagnostic(DiagnosticEvent {
                severity: DiagnosticSeverity::Note,
                diagnostic: "synthetic".into(),
                arguments: Vec::new(),
            }),
        })
        .collect();
    let stream = ObservationStream {
        header: ObservationHeader {
            schema: 1,
            manifest: "0".repeat(64),
        },
        events: stream_events,
    };
    CommittedFixture { manifest, stream }
}

#[test]
fn budget_accepts_the_widest_committed_fixture_shape() {
    // command-transitions-v1: 8 sources totaling 2,331 bytes, 3,960 events.
    let fixture = synthetic_fixture(8, 2331 / 8, 3960);
    validate_minifixture_budget(&fixture).expect("within every committed measurement");
}

#[test]
fn budget_rejects_too_many_sources() {
    let fixture = synthetic_fixture(MINIFIXTURE_MAX_SOURCES + 1, 1, 1);
    let error = validate_minifixture_budget(&fixture).expect_err("too many sources");
    assert!(error.contains("tex82/synthetic-budget"), "{error}");
    assert!(
        error.contains("exceeds the tex82 minifixture regeneration budget"),
        "{error}"
    );
}

#[test]
fn budget_rejects_too_many_source_bytes() {
    let fixture = synthetic_fixture(1, MINIFIXTURE_MAX_SOURCE_BYTES + 1, 1);
    let error = validate_minifixture_budget(&fixture).expect_err("too many source bytes");
    assert!(error.contains("source byte(s)"), "{error}");
}

#[test]
fn budget_rejects_too_many_events() {
    let fixture = synthetic_fixture(1, 1, MINIFIXTURE_MAX_EVENTS + 1);
    let error = validate_minifixture_budget(&fixture).expect_err("too many events");
    assert!(error.contains("event(s)"), "{error}");
}

#[test]
fn budget_accepts_every_committed_tex82_fixture() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = repository.join("tests/corpus/command/tex82");
    let mut checked = 0;
    for entry in std::fs::read_dir(&root).expect("fixture root") {
        let entry = entry.expect("fixture entry");
        if !entry.path().join("manifest.json").is_file() {
            continue;
        }
        let fixture = CommittedFixture::load(entry.path()).expect("committed fixture loads");
        validate_minifixture_budget(&fixture)
            .unwrap_or_else(|error| panic!("committed fixture must fit the budget: {error}"));
        checked += 1;
    }
    assert!(
        checked >= 6,
        "expected at least 6 split tex82 fixtures, found {checked}"
    );
}
