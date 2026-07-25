//! Bounded, schema-aware TRIP mismatch reports.
//!
//! The report intentionally contains identities and a small semantic context,
//! never copied transcripts, logs, DVI files, or full event streams.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use test_support::dvi::normalized_dvi_for_comparison;
use tex_oracle::ObservationStream;

const ARTIFACT_NAME: &str = "trip-triage-v1.txt";
const ARTIFACT_LIMIT: usize = 8 * 1024;
const CONTEXT_LIMIT: usize = 256;
const EVENT_LIMIT: usize = 768;

/// Stable identity of one side of a TRIP comparison.
#[derive(Clone, Copy, Debug)]
pub struct TripTriageSource<'a> {
    /// Human-readable, repository-relative identity (for example a fixture).
    pub name: &'a str,
    /// Content identity selected by the harness (normally a pinned SHA-256).
    pub identity: &'a str,
}

/// The compared channels for one side of the two-phase workload.
#[derive(Clone, Copy, Debug)]
pub struct TripTriageChannels<'a> {
    /// Canonical schema-v1 JSONL. `None` is itself a meaningful mismatch.
    pub command_events: Option<&'a [u8]>,
    pub transcript: &'a [u8],
    pub log: &'a [u8],
    pub dvi: &'a [u8],
}

/// Inputs to the deterministic TRIP triage writer.
#[derive(Clone, Copy, Debug)]
pub struct TripTriageInput<'a> {
    pub label: &'a str,
    pub phase: &'a str,
    pub expected_source: TripTriageSource<'a>,
    pub actual_source: TripTriageSource<'a>,
    pub expected: TripTriageChannels<'a>,
    pub actual: TripTriageChannels<'a>,
}

/// Writes one compact report only when a semantic/output channel differs.
///
/// The result is deterministic for equivalent inputs and contains at most 8
/// KiB. A fully successful comparison leaves no artifact behind.
pub fn write_trip_triage_artifact(
    root: &Path,
    input: TripTriageInput<'_>,
) -> Result<Option<PathBuf>> {
    let artifact = root.join(safe_component(input.label)).join(ARTIFACT_NAME);
    let expected_dvi = normalized_dvi_for_comparison(input.expected.dvi)?;
    let actual_dvi = normalized_dvi_for_comparison(input.actual.dvi)?;
    let divergence = first_divergence(input, &expected_dvi, &actual_dvi)?;
    let Some(divergence) = divergence else {
        if artifact.exists() {
            fs::remove_file(&artifact)
                .with_context(|| format!("failed to remove stale {}", artifact.display()))?;
        }
        return Ok(None);
    };

    let parent = artifact.parent().expect("artifact has a parent");
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let report = report_text(input, &expected_dvi, &actual_dvi, &divergence);
    fs::write(&artifact, report)
        .with_context(|| format!("failed to write {}", artifact.display()))?;
    Ok(Some(artifact))
}

fn first_divergence(
    input: TripTriageInput<'_>,
    expected_dvi: &[u8],
    actual_dvi: &[u8],
) -> Result<Option<Divergence>> {
    if let Some(divergence) =
        event_divergence(input.expected.command_events, input.actual.command_events)?
    {
        return Ok(Some(divergence));
    }
    if let Some(divergence) = byte_divergence(
        "transcript",
        input.expected.transcript,
        input.actual.transcript,
    ) {
        return Ok(Some(divergence));
    }
    if let Some(divergence) = byte_divergence("log", input.expected.log, input.actual.log) {
        return Ok(Some(divergence));
    }
    Ok(byte_divergence("normalized_dvi", expected_dvi, actual_dvi))
}

fn event_divergence(expected: Option<&[u8]>, actual: Option<&[u8]>) -> Result<Option<Divergence>> {
    match (expected, actual) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Ok(Some(Divergence::presence("command_events"))),
        (Some(expected), Some(actual)) => {
            let expected = ObservationStream::from_canonical_json_lines(expected)
                .context("expected TRIP command-event stream is not canonical schema-v1 JSONL")?;
            let actual = ObservationStream::from_canonical_json_lines(actual)
                .context("actual TRIP command-event stream is not canonical schema-v1 JSONL")?;
            if expected.header != actual.header {
                return Ok(Some(Divergence::event_header(
                    &expected.header.manifest,
                    &actual.header.manifest,
                )));
            }
            let index = expected
                .events
                .iter()
                .zip(&actual.events)
                .position(|(left, right)| left != right);
            let index = index.or_else(|| {
                (expected.events.len() != actual.events.len())
                    .then_some(expected.events.len().min(actual.events.len()))
            });
            Ok(index.map(|index| {
                Divergence::event(index, expected.events.get(index), actual.events.get(index))
            }))
        }
    }
}

fn byte_divergence(channel: &'static str, expected: &[u8], actual: &[u8]) -> Option<Divergence> {
    let index = expected
        .iter()
        .zip(actual)
        .position(|(left, right)| left != right)
        .or_else(|| (expected.len() != actual.len()).then_some(expected.len().min(actual.len())))?;
    Some(Divergence::bytes(channel, index, expected, actual))
}

#[derive(Debug)]
struct Divergence {
    channel: &'static str,
    position: String,
    expected: String,
    actual: String,
}

impl Divergence {
    fn presence(channel: &'static str) -> Self {
        Self {
            channel,
            position: "presence".into(),
            expected: "present".into(),
            actual: "absent".into(),
        }
    }

    fn event_header(expected: &str, actual: &str) -> Self {
        Self {
            channel: "command_events",
            position: "header.manifest".into(),
            expected: bounded(expected, EVENT_LIMIT),
            actual: bounded(actual, EVENT_LIMIT),
        }
    }

    fn event(
        index: usize,
        expected: Option<&tex_oracle::NormalizedEvent>,
        actual: Option<&tex_oracle::NormalizedEvent>,
    ) -> Self {
        Self {
            channel: "command_events",
            position: format!("event[{index}]"),
            expected: event_text(expected),
            actual: event_text(actual),
        }
    }

    fn bytes(channel: &'static str, index: usize, expected: &[u8], actual: &[u8]) -> Self {
        Self {
            channel,
            position: format!("byte[{index}]"),
            expected: byte_context(expected, index),
            actual: byte_context(actual, index),
        }
    }
}

fn report_text(
    input: TripTriageInput<'_>,
    expected_dvi: &[u8],
    actual_dvi: &[u8],
    divergence: &Divergence,
) -> String {
    let mut out = String::new();
    for (key, value) in [
        ("schema", "umber.trip-triage.v1".to_string()),
        ("label", bounded(input.label, 128)),
        ("phase", bounded(input.phase, 128)),
        ("status", "mismatch".to_string()),
        (
            "expected_source.name",
            bounded(input.expected_source.name, 256),
        ),
        (
            "expected_source.identity",
            bounded(input.expected_source.identity, 256),
        ),
        ("actual_source.name", bounded(input.actual_source.name, 256)),
        (
            "actual_source.identity",
            bounded(input.actual_source.identity, 256),
        ),
        (
            "expected.command_events.sha256",
            option_hash(input.expected.command_events),
        ),
        (
            "actual.command_events.sha256",
            option_hash(input.actual.command_events),
        ),
        (
            "expected.transcript.sha256",
            sha256(input.expected.transcript),
        ),
        ("actual.transcript.sha256", sha256(input.actual.transcript)),
        ("expected.log.sha256", sha256(input.expected.log)),
        ("actual.log.sha256", sha256(input.actual.log)),
        ("expected.normalized_dvi.sha256", sha256(expected_dvi)),
        ("actual.normalized_dvi.sha256", sha256(actual_dvi)),
        ("earliest.channel", divergence.channel.to_string()),
        ("earliest.position", divergence.position.clone()),
        ("earliest.expected", divergence.expected.clone()),
        ("earliest.actual", divergence.actual.clone()),
    ] {
        bounded_line(&mut out, key, &value);
    }
    out
}

fn bounded_line(out: &mut String, key: &str, value: &str) {
    if out.len() >= ARTIFACT_LIMIT {
        return;
    }
    let remaining = ARTIFACT_LIMIT - out.len();
    let mut line = format!(
        "{key}: {}\n",
        bounded(value, remaining.saturating_sub(key.len() + 3))
    );
    if line.len() > remaining {
        line.truncate(remaining);
    }
    out.push_str(&line);
}

fn event_text(event: Option<&tex_oracle::NormalizedEvent>) -> String {
    event.map_or_else(
        || "<EOF>".to_string(),
        |event| {
            bounded(
                &serde_json::to_string(event).expect("schema events serialize"),
                EVENT_LIMIT,
            )
        },
    )
}

fn byte_context(bytes: &[u8], index: usize) -> String {
    if index == bytes.len() {
        return "<EOF>".to_string();
    }
    let start = index.saturating_sub(32);
    let end = bytes.len().min(index.saturating_add(33));
    let mut out = String::new();
    for (offset, byte) in bytes[start..end].iter().enumerate() {
        if start + offset == index {
            out.push('[');
        }
        if byte.is_ascii_graphic() || *byte == b' ' {
            out.push(*byte as char);
        } else {
            let _ = write!(out, "\\x{byte:02x}");
        }
        if start + offset == index {
            out.push(']');
        }
    }
    bounded(&out, CONTEXT_LIMIT)
}

fn option_hash(bytes: Option<&[u8]>) -> String {
    bytes.map_or_else(|| "absent".to_string(), sha256)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn bounded(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit.saturating_sub(3);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

fn safe_component(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_oracle::{
        CanonicalValue, EffectEvent, EffectKind, Event, InputEvent, InputReason, InputTransition,
        Normalizer, SCHEMA_VERSION,
    };

    fn events(page: i64) -> Vec<u8> {
        let mut normalizer = Normalizer::new();
        let values = [
            Event::Effect(EffectEvent {
                kind: EffectKind::Shipout,
                channel: "dvi".into(),
                value: CanonicalValue::Integer(page),
            }),
            Event::Input(InputEvent {
                transition: InputTransition::Stop,
                reason: InputReason::Source,
                name: "terminal".into(),
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Terminate,
                channel: "engine".into(),
                value: CanonicalValue::None,
            }),
        ];
        let mut out = format!(
            "{{\"schema\":{SCHEMA_VERSION},\"manifest\":\"{}\"}}\n",
            "a".repeat(64)
        )
        .into_bytes();
        for value in values {
            out.extend_from_slice(
                &serde_json::to_vec(&normalizer.normalize(value)).expect("event"),
            );
            out.push(b'\n');
        }
        out
    }

    fn dvi(comment: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = vec![247, 2];
        out.extend_from_slice(&25_400_000i32.to_be_bytes());
        out.extend_from_slice(&473_628_672i32.to_be_bytes());
        out.extend_from_slice(&1000i32.to_be_bytes());
        out.push(comment.len() as u8);
        out.extend_from_slice(comment);
        out.extend_from_slice(body);
        out
    }

    fn input<'a>(
        expected: TripTriageChannels<'a>,
        actual: TripTriageChannels<'a>,
    ) -> TripTriageInput<'a> {
        TripTriageInput {
            label: "trip",
            phase: "format-loaded",
            expected_source: TripTriageSource {
                name: "reference",
                identity: "ref-id",
            },
            actual_source: TripTriageSource {
                name: "umber",
                identity: "umber-id",
            },
            expected,
            actual,
        }
    }

    #[test]
    fn each_channel_reports_its_earliest_bounded_divergence_and_success_is_silent() {
        let temp = tempfile::tempdir().expect("temp");
        let reference_events = events(1);
        let actual_events = events(2);
        let reference_dvi = dvi(b"ref", &[140]);
        let actual_dvi = dvi(b"other", &[141]);
        let base = TripTriageChannels {
            command_events: Some(&reference_events),
            transcript: b"alpha",
            log: b"log",
            dvi: &reference_dvi,
        };
        let event_mismatch = TripTriageChannels {
            command_events: Some(&actual_events),
            ..base
        };
        for (actual, channel) in [
            (event_mismatch, "command_events"),
            (
                TripTriageChannels {
                    transcript: b"alpHa",
                    ..base
                },
                "transcript",
            ),
            (
                TripTriageChannels {
                    log: b"loG",
                    ..base
                },
                "log",
            ),
            (
                TripTriageChannels {
                    dvi: &actual_dvi,
                    ..base
                },
                "normalized_dvi",
            ),
        ] {
            let artifact = write_trip_triage_artifact(temp.path(), input(base, actual))
                .expect("write")
                .expect("mismatch");
            let report = fs::read_to_string(artifact).expect("report");
            assert!(
                report.contains(&format!("earliest.channel: {channel}")),
                "{report}"
            );
            assert!(
                report.contains("expected_source.identity: ref-id"),
                "{report}"
            );
            if channel == "command_events" {
                assert!(report.contains("earliest.position: event[0]"), "{report}");
            }
            assert!(report.len() <= ARTIFACT_LIMIT, "{}", report.len());
        }
        assert!(
            write_trip_triage_artifact(temp.path(), input(base, base))
                .expect("success")
                .is_none()
        );
        assert!(!temp.path().join("trip").join(ARTIFACT_NAME).exists());
    }

    #[test]
    fn reports_are_reproducible_and_never_embed_an_unbounded_log() {
        let temp = tempfile::tempdir().expect("temp");
        let events = events(1);
        let dvi = dvi(b"ref", &[140]);
        let long_log = vec![b'x'; ARTIFACT_LIMIT * 4];
        let expected = TripTriageChannels {
            command_events: Some(&events),
            transcript: b"same",
            log: &long_log,
            dvi: &dvi,
        };
        let actual = TripTriageChannels {
            log: b"different",
            ..expected
        };
        let artifact = write_trip_triage_artifact(temp.path(), input(expected, actual))
            .expect("first")
            .expect("artifact");
        let first = fs::read(&artifact).expect("first report");
        let artifact = write_trip_triage_artifact(temp.path(), input(expected, actual))
            .expect("second")
            .expect("artifact");
        assert_eq!(first, fs::read(artifact).expect("second report"));
        assert!(first.len() <= ARTIFACT_LIMIT);
    }
}
