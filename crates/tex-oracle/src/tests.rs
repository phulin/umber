use std::collections::BTreeMap;

use crate::{
    CanonicalCommand, CanonicalValue, CommandDelivery, CommandEvent, DisabledObserver,
    EngineDialect, EngineIdentity, Event, EventObserver, JsonLinesObserver, Manifest,
    ManifestInput, Normalizer, SCHEMA_VERSION,
};

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

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
}

#[test]
fn disabled_observer_accepts_events_without_transport_state() {
    assert_eq!(size_of::<DisabledObserver>(), 0);
    DisabledObserver
        .committed(command("ignored"))
        .expect("no-op");
}
