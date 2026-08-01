use std::fs;
use std::process::Command;

use tex_oracle::{
    DiagnosticEvent, DiagnosticSeverity, Event, Normalizer, ObservationHeader, SchemaVersion,
};

fn stream(events: impl IntoIterator<Item = Event>) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&ObservationHeader {
        schema: SchemaVersion::V1.number(),
        manifest: "a".repeat(64),
    })
    .expect("header serializes");
    bytes.push(b'\n');
    let mut normalizer = Normalizer::new();
    for event in events {
        bytes.extend_from_slice(
            &serde_json::to_vec(&normalizer.normalize(event)).expect("event serializes"),
        );
        bytes.push(b'\n');
    }
    bytes
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary test launches the built host-only binary.
fn captured_stream_cli_aligns_exhaustively_and_groups_exact_roots() {
    let directory = tempfile::tempdir().expect("temporary streams");
    let expected = directory.path().join("expected.jsonl");
    let actual = directory.path().join("actual.jsonl");
    let diagnostic = |name: &str| {
        Event::Diagnostic(DiagnosticEvent {
            severity: DiagnosticSeverity::Error,
            diagnostic: name.into(),
            arguments: Vec::new(),
        })
    };
    fs::write(
        &expected,
        stream([diagnostic("first"), diagnostic("second")]),
    )
    .expect("write expected stream");
    fs::write(
        &actual,
        stream([
            diagnostic("extra"),
            diagnostic("first"),
            diagnostic("second"),
        ]),
    )
    .expect("write actual stream");
    let output = Command::new(env!("CARGO_BIN_EXE_event-stream-diff"))
        .args([&expected, &actual])
        .output()
        .expect("run captured-stream diff");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(
        stdout.contains("1 ordered divergence(s), 1 root site(s), budget_reached=false"),
        "{stdout}"
    );
    assert!(stdout.contains("1 extra Umber event(s)"), "{stdout}");
}
