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
use tex_oracle::{
    CanonicalCommand, CanonicalValue, CommandEvent, Event, NormalizedEvent, ObservationStream,
};

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
    /// Canonical schema-v2 geometry JSONL, kept identity-separate from v1.
    pub geometry_events: Option<&'a [u8]>,
    pub transcript: &'a [u8],
    pub log: &'a [u8],
    /// DVI is absent during INITEX format creation and present after loading.
    pub dvi: Option<&'a [u8]>,
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
    let expected_dvi = input
        .expected
        .dvi
        .map(normalized_dvi_for_comparison)
        .transpose()?;
    let actual_dvi = input
        .actual
        .dvi
        .map(normalized_dvi_for_comparison)
        .transpose()?;
    let divergence = first_divergence(input, expected_dvi.as_deref(), actual_dvi.as_deref())?;
    let Some(divergence) = divergence else {
        if artifact.exists() {
            fs::remove_file(&artifact)
                .with_context(|| format!("failed to remove stale {}", artifact.display()))?;
        }
        return Ok(None);
    };

    let parent = artifact.parent().expect("artifact has a parent");
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let report = report_text(
        input,
        expected_dvi.as_deref(),
        actual_dvi.as_deref(),
        &divergence,
    );
    fs::write(&artifact, report)
        .with_context(|| format!("failed to write {}", artifact.display()))?;
    Ok(Some(artifact))
}

fn first_divergence(
    input: TripTriageInput<'_>,
    expected_dvi: Option<&[u8]>,
    actual_dvi: Option<&[u8]>,
) -> Result<Option<Divergence>> {
    if let Some(divergence) = event_divergence(
        "command_events",
        input.expected.command_events,
        input.actual.command_events,
    )? {
        return Ok(Some(divergence));
    }
    if let Some(divergence) = event_divergence(
        "geometry_events",
        input.expected.geometry_events,
        input.actual.geometry_events,
    )? {
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
    Ok(optional_byte_divergence(
        "normalized_dvi",
        expected_dvi,
        actual_dvi,
    ))
}

fn optional_byte_divergence(
    channel: &'static str,
    expected: Option<&[u8]>,
    actual: Option<&[u8]>,
) -> Option<Divergence> {
    match (expected, actual) {
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => Some(Divergence::presence(channel)),
        (Some(expected), Some(actual)) => byte_divergence(channel, expected, actual),
    }
}

fn event_divergence(
    channel: &'static str,
    expected: Option<&[u8]>,
    actual: Option<&[u8]>,
) -> Result<Option<Divergence>> {
    match (expected, actual) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Ok(Some(Divergence::presence(channel))),
        (Some(expected), Some(actual)) => {
            let expected = ObservationStream::from_canonical_json_lines(expected)
                .context("expected TRIP command-event stream is not canonical schema-v1 JSONL")?;
            let actual = ObservationStream::from_canonical_json_lines(actual)
                .context("actual TRIP command-event stream is not canonical schema-v1 JSONL")?;
            let expected_schema = if channel == "command_events" { 1 } else { 2 };
            if expected.header.schema != expected_schema || actual.header.schema != expected_schema
            {
                anyhow::bail!(
                    "{channel} must use canonical schema-v{expected_schema} on both sides"
                );
            }
            if expected.header != actual.header {
                return Ok(Some(Divergence::event_header(
                    channel,
                    &expected.header.manifest,
                    &actual.header.manifest,
                )));
            }
            let index = expected
                .events
                .iter()
                .zip(&actual.events)
                .position(|(left, right)| !trip_events_match(channel, left, right));
            let index = index.or_else(|| {
                (expected.events.len() != actual.events.len())
                    .then_some(expected.events.len().min(actual.events.len()))
            });
            Ok(index.map(|index| {
                Divergence::event(
                    channel,
                    index,
                    expected.events.get(index),
                    actual.events.get(index),
                )
            }))
        }
    }
}

/// Compares canonical events after removing TeX's allocation-only macro
/// operand.
///
/// TeX82 §382 installs `def_ref` as a macro's `equiv`, so `get_next` exposes
/// that mutable token-list address as `cur_chr` on a `call`. Umber retains
/// immutable macro-definition ownership instead and deliberately emits no
/// allocator identity. The differential tracer applies this same projection;
/// the integrated two-phase TRIP comparator must not invent a semantic
/// mismatch from the reference engine's memory address.
fn trip_events_match(channel: &str, expected: &NormalizedEvent, actual: &NormalizedEvent) -> bool {
    expected == actual
        || (channel == "command_events"
            && expected.sequence == actual.sequence
            && macro_call_operand_is_reference(&expected.semantic, &actual.semantic))
}

fn macro_call_operand_is_reference(expected: &Event, actual: &Event) -> bool {
    let (
        Event::Command(CommandEvent {
            delivery: expected_delivery,
            command:
                CanonicalCommand {
                    command: expected_command,
                    operand: CanonicalValue::Integer(expected_operand),
                    control_sequence: expected_control_sequence,
                    location: expected_location,
                },
        }),
        Event::Command(CommandEvent {
            delivery: actual_delivery,
            command:
                CanonicalCommand {
                    command: actual_command,
                    operand: actual_operand,
                    control_sequence: actual_control_sequence,
                    location: actual_location,
                },
        }),
    ) = (expected, actual)
    else {
        return false;
    };

    let allocator_identity_matches = match actual_operand {
        CanonicalValue::Integer(actual_operand) => expected_operand.abs_diff(*actual_operand) <= 1,
        CanonicalValue::None => true,
        _ => false,
    };

    allocator_identity_matches
        && matches!(
            expected_command.as_str(),
            "call" | "long_call" | "outer_call" | "long_outer_call"
        )
        && expected_delivery == actual_delivery
        && expected_command == actual_command
        && expected_control_sequence == actual_control_sequence
        && expected_location == actual_location
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

    fn event_header(channel: &'static str, expected: &str, actual: &str) -> Self {
        Self {
            channel,
            position: "header.manifest".into(),
            expected: bounded(expected, EVENT_LIMIT),
            actual: bounded(actual, EVENT_LIMIT),
        }
    }

    fn event(
        channel: &'static str,
        index: usize,
        expected: Option<&tex_oracle::NormalizedEvent>,
        actual: Option<&tex_oracle::NormalizedEvent>,
    ) -> Self {
        Self {
            channel,
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
    expected_dvi: Option<&[u8]>,
    actual_dvi: Option<&[u8]>,
    divergence: &Divergence,
) -> String {
    let mut out = String::new();
    let command_accounting = event_accounting(
        "command_events",
        input.expected.command_events,
        input.actual.command_events,
    );
    let geometry_accounting = event_accounting(
        "geometry_events",
        input.expected.geometry_events,
        input.actual.geometry_events,
    );
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
            "expected.command_events.count",
            command_accounting.expected_count,
        ),
        (
            "actual.command_events.count",
            command_accounting.actual_count,
        ),
        (
            "command_events.projected_equivalent.count",
            command_accounting.projected_equivalent_count,
        ),
        (
            "command_events.projected_divergence.count",
            command_accounting.projected_divergence_count,
        ),
        (
            "expected.geometry_events.sha256",
            option_hash(input.expected.geometry_events),
        ),
        (
            "actual.geometry_events.sha256",
            option_hash(input.actual.geometry_events),
        ),
        (
            "expected.geometry_events.count",
            geometry_accounting.expected_count,
        ),
        (
            "actual.geometry_events.count",
            geometry_accounting.actual_count,
        ),
        (
            "geometry_events.projected_divergence.count",
            geometry_accounting.projected_divergence_count,
        ),
        (
            "expected.transcript.sha256",
            sha256(input.expected.transcript),
        ),
        ("actual.transcript.sha256", sha256(input.actual.transcript)),
        ("expected.log.sha256", sha256(input.expected.log)),
        ("actual.log.sha256", sha256(input.actual.log)),
        ("expected.normalized_dvi.sha256", option_hash(expected_dvi)),
        ("actual.normalized_dvi.sha256", option_hash(actual_dvi)),
        ("earliest.channel", divergence.channel.to_string()),
        ("earliest.position", divergence.position.clone()),
        ("earliest.expected", divergence.expected.clone()),
        ("earliest.actual", divergence.actual.clone()),
    ] {
        bounded_line(&mut out, key, &value);
    }
    out
}

struct EventAccounting {
    expected_count: String,
    actual_count: String,
    projected_equivalent_count: String,
    projected_divergence_count: String,
}

fn event_accounting(
    channel: &str,
    expected: Option<&[u8]>,
    actual: Option<&[u8]>,
) -> EventAccounting {
    let (Some(expected), Some(actual)) = (expected, actual) else {
        return EventAccounting {
            expected_count: expected.map_or_else(|| "absent".into(), event_count),
            actual_count: actual.map_or_else(|| "absent".into(), event_count),
            projected_equivalent_count: "unavailable".into(),
            projected_divergence_count: "unavailable".into(),
        };
    };
    let expected = ObservationStream::from_canonical_json_lines(expected)
        .expect("event streams were validated before report rendering");
    let actual = ObservationStream::from_canonical_json_lines(actual)
        .expect("event streams were validated before report rendering");
    let projected_equivalent_count = expected
        .events
        .iter()
        .zip(&actual.events)
        .filter(|(left, right)| left != right && trip_events_match(channel, left, right))
        .count();
    let projected_divergence_count = expected
        .events
        .iter()
        .zip(&actual.events)
        .filter(|(left, right)| !trip_events_match(channel, left, right))
        .count()
        + expected.events.len().abs_diff(actual.events.len());
    EventAccounting {
        expected_count: expected.events.len().to_string(),
        actual_count: actual.events.len().to_string(),
        projected_equivalent_count: projected_equivalent_count.to_string(),
        projected_divergence_count: projected_divergence_count.to_string(),
    }
}

fn event_count(bytes: &[u8]) -> String {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .count()
        .saturating_sub(1)
        .to_string()
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

    fn geometry_events(count0: i32) -> Vec<u8> {
        let mut normalizer = Normalizer::new();
        let mut counts = [0; 10];
        counts[0] = count0;
        let event = Event::Geometry(tex_oracle::GeometryEvent::Shipout {
            page_width_sp: 10,
            page_height_sp: 20,
            counts,
        });
        let mut out =
            format!("{{\"schema\":2,\"manifest\":\"{}\"}}\n", "b".repeat(64)).into_bytes();
        out.extend_from_slice(&serde_json::to_vec(&normalizer.normalize(event)).expect("event"));
        out.push(b'\n');
        out
    }

    fn macro_command_events(operand: CanonicalValue) -> Vec<u8> {
        macro_command_events_named(operand, "probe")
    }

    fn macro_command_events_named(operand: CanonicalValue, name: &str) -> Vec<u8> {
        let mut normalizer = Normalizer::new();
        let event = Event::Command(CommandEvent {
            delivery: tex_oracle::CommandDelivery::Raw,
            command: CanonicalCommand {
                command: "call".into(),
                operand,
                control_sequence: Some(name.into()),
                location: None,
            },
        });
        let mut out = format!(
            "{{\"schema\":{SCHEMA_VERSION},\"manifest\":\"{}\"}}\n",
            "a".repeat(64)
        )
        .into_bytes();
        out.extend_from_slice(&serde_json::to_vec(&normalizer.normalize(event)).expect("event"));
        out.push(b'\n');
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
            geometry_events: None,
            transcript: b"alpha",
            log: b"log",
            dvi: Some(&reference_dvi),
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
                    dvi: Some(&actual_dvi),
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
    fn macro_call_def_ref_is_not_a_semantic_trip_divergence() {
        // TeX82 §382 stores the allocator-owned `def_ref` address as the
        // macro's `equiv`; Umber's immutable macro identity has no integer
        // operand. Both still describe the same raw `call`.
        let temp = tempfile::tempdir().expect("temp");
        let reference = macro_command_events(CanonicalValue::Integer(249_985));
        for operand in [
            CanonicalValue::Integer(249_984),
            CanonicalValue::Integer(249_986),
            CanonicalValue::None,
        ] {
            let actual = macro_command_events(operand);
            let expected = TripTriageChannels {
                command_events: Some(&reference),
                geometry_events: None,
                transcript: b"same",
                log: b"same",
                dvi: None,
            };
            let actual = TripTriageChannels {
                command_events: Some(&actual),
                ..expected
            };

            assert!(
                write_trip_triage_artifact(temp.path(), input(expected, actual))
                    .expect("comparison")
                    .is_none()
            );
        }
    }

    #[test]
    fn macro_call_projection_preserves_non_adjacent_allocator_differences() {
        let temp = tempfile::tempdir().expect("temp");
        let reference = macro_command_events(CanonicalValue::Integer(249_985));
        let actual = macro_command_events(CanonicalValue::Integer(249_983));
        let expected = TripTriageChannels {
            command_events: Some(&reference),
            geometry_events: None,
            transcript: b"same",
            log: b"same",
            dvi: None,
        };
        let actual = TripTriageChannels {
            command_events: Some(&actual),
            ..expected
        };

        let artifact = write_trip_triage_artifact(temp.path(), input(expected, actual))
            .expect("comparison")
            .expect("non-adjacent allocator identity must diverge");
        let report = fs::read_to_string(artifact).expect("report");
        assert!(report.contains("earliest.position: event[0]"), "{report}");
        assert!(
            report.contains("expected.command_events.count: 1"),
            "{report}"
        );
        assert!(
            report.contains("actual.command_events.count: 1"),
            "{report}"
        );
        assert!(
            report.contains("command_events.projected_equivalent.count: 0"),
            "{report}"
        );
        assert!(
            report.contains("command_events.projected_divergence.count: 1"),
            "{report}"
        );
    }

    #[test]
    fn macro_call_projection_preserves_macro_identity_differences() {
        let temp = tempfile::tempdir().expect("temp");
        let reference = macro_command_events_named(CanonicalValue::Integer(249_985), "older");
        let actual = macro_command_events_named(CanonicalValue::Integer(249_986), "newer");
        let expected = TripTriageChannels {
            command_events: Some(&reference),
            geometry_events: None,
            transcript: b"same",
            log: b"same",
            dvi: None,
        };
        let actual = TripTriageChannels {
            command_events: Some(&actual),
            ..expected
        };

        let artifact = write_trip_triage_artifact(temp.path(), input(expected, actual))
            .expect("comparison")
            .expect("a different macro identity must diverge");
        let report = fs::read_to_string(artifact).expect("report");
        assert!(report.contains("earliest.position: event[0]"), "{report}");
        assert!(
            report.contains("command_events.projected_divergence.count: 1"),
            "{report}"
        );
    }

    #[test]
    fn reports_are_reproducible_and_never_embed_an_unbounded_log() {
        let temp = tempfile::tempdir().expect("temp");
        let events = events(1);
        let dvi = dvi(b"ref", &[140]);
        let long_log = vec![b'x'; ARTIFACT_LIMIT * 4];
        let expected = TripTriageChannels {
            command_events: Some(&events),
            geometry_events: None,
            transcript: b"same",
            log: &long_log,
            dvi: Some(&dvi),
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

    #[test]
    fn command_events_precede_geometry_and_geometry_precedes_output_channels() {
        let temp = tempfile::tempdir().expect("temp");
        let command = events(1);
        let changed_command = events(2);
        let geometry = geometry_events(117);
        let changed_geometry = geometry_events(118);
        let dvi = dvi(b"ref", &[140]);
        let base = TripTriageChannels {
            command_events: Some(&command),
            geometry_events: Some(&geometry),
            transcript: b"same",
            log: b"same",
            dvi: Some(&dvi),
        };
        let artifact = write_trip_triage_artifact(
            temp.path(),
            input(
                base,
                TripTriageChannels {
                    command_events: Some(&changed_command),
                    geometry_events: Some(&changed_geometry),
                    transcript: b"different",
                    ..base
                },
            ),
        )
        .expect("command comparison")
        .expect("command mismatch");
        let report = fs::read_to_string(&artifact).expect("command report");
        assert!(
            report.contains("earliest.channel: command_events"),
            "{report}"
        );

        let artifact = write_trip_triage_artifact(
            temp.path(),
            input(
                base,
                TripTriageChannels {
                    geometry_events: Some(&changed_geometry),
                    transcript: b"different",
                    ..base
                },
            ),
        )
        .expect("geometry comparison")
        .expect("geometry mismatch");
        let report = fs::read_to_string(artifact).expect("geometry report");
        assert!(
            report.contains("earliest.channel: geometry_events"),
            "{report}"
        );
        assert!(
            report.contains("\"counts\":[118,0,0,0,0,0,0,0,0,0]"),
            "{report}"
        );
    }
}
