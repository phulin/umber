#![allow(
    clippy::disallowed_methods,
    reason = "fixture contract tests stage disposable host files outside engine execution"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};

use crate::{
    AlignmentEvent, AlignmentTransition, CanonicalCitation, CanonicalCommand, CanonicalValue,
    CommandDelivery, CommandEvent, CommittedFixture, DisabledObserver, EngineDialect,
    EngineIdentity, Event, EventObserver, FixtureArtifact, FixtureManifest, FixtureProfile,
    JsonLinesObserver, Manifest, ManifestInput, Normalizer, ObservationStream, SCHEMA_VERSION,
    ToolIdentity,
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
    let first = normalizer.normalize(command("outer\r\nmacro"));
    let second = normalizer.normalize(command("next\rcommand"));
    assert_eq!(first.sequence, 0);
    assert_eq!(second.sequence, 1);
    let Event::Command(event) = first.semantic else {
        panic!("command event");
    };
    assert_eq!(event.command.command, "outer\nmacro");
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
        contract: 1,
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
fn fixture_manifest_rejects_schema_profile_output_and_citation_drift() {
    let mut value = fixture_manifest();
    assert!(value.validate().is_ok());

    value.oracle.schema = SCHEMA_VERSION + 1;
    assert!(value.validate().is_err());
    value = fixture_manifest();
    value.profile.characters = "unicode_extended".into();
    assert!(value.validate().is_err());
    value = fixture_manifest();
    value.outputs.get_mut("dvi").expect("output").sha256 = HASH_A.into();
    assert!(value.validate().is_err());
    value = fixture_manifest();
    value.citations.clear();
    assert!(value.validate().is_err());
}

#[test]
fn committed_tex82_fixture_is_consumed_hermetically() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture = CommittedFixture::load(
        repository.join("tests/corpus/command/tex82/command-transitions-v1"),
    )
    .expect("committed canonical fixture");
    assert_eq!(fixture.manifest.name, "tex82/command-transitions-v1");
    assert_eq!(fixture.stream.events.len(), 11_665);
    fixture
        .audit_matrices(
            &fs::read(repository.join("tests/tex82-oracle/semantic-event-matrix.txt"))
                .expect("semantic matrix"),
            &fs::read(repository.join("tests/tex82-oracle/fixture-audit-matrix.txt"))
                .expect("fixture audit matrix"),
        )
        .expect("complete bidirectional fixture audit");
}

#[test]
fn fixture_audit_rejects_missing_behavior_and_unowned_observations() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture = CommittedFixture::load(
        repository.join("tests/corpus/command/tex82/command-transitions-v1"),
    )
    .expect("committed canonical fixture");
    let semantic =
        fs::read(repository.join("tests/tex82-oracle/semantic-event-matrix.txt")).expect("matrix");
    let audit = String::from_utf8(
        fs::read(repository.join("tests/tex82-oracle/fixture-audit-matrix.txt")).expect("audit"),
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
fn committed_fixture_rejects_event_identity_and_output_byte_drift() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/command/tex82/command-transitions-v1");
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

    let mut events = fs::read(temporary.join("events.jsonl")).expect("events");
    let newline = events
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header newline");
    events[..newline].copy_from_slice(
        b"{\"schema\":1,\"manifest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}",
    );
    fs::write(temporary.join("events.jsonl"), &events).expect("write events");
    let mut manifest = loaded.manifest.clone();
    manifest.events.sha256 = hex_hash(&events);
    fs::write(
        temporary.join("manifest.json"),
        manifest.to_canonical_json().expect("manifest"),
    )
    .expect("write manifest");
    assert!(CommittedFixture::load(&temporary).is_err());

    fs::copy(source.join("events.jsonl"), temporary.join("events.jsonl")).expect("restore events");
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
