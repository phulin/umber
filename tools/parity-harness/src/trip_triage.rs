//! Bounded, schema-aware TRIP mismatch reports.
//!
//! The report intentionally contains identities and a small semantic context,
//! never copied transcripts, logs, DVI files, or full event streams.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use test_support::dvi::normalized_dvi_for_comparison;
use tex_oracle::{
    CanonicalCommand, CanonicalValue, CommandEvent, Event, MutationEvent, NormalizedEvent,
    ObservationStream, StateTarget,
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
    /// Optional earlier command history that established format-loaded macro
    /// meanings on this same side.
    pub initialization_events: Option<&'a [u8]>,
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

/// Outcome of one comparison, separating acceptance from advisory geometry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TripTriageVerdict {
    /// Bounded diagnostic written for either kind of difference.
    pub artifact: Option<PathBuf>,
    /// A command, transcript, log, or normalized-DVI channel differed.
    pub gating_mismatch: bool,
    /// The identity-separated geometry projection differed.
    pub advisory_geometry_mismatch: bool,
}

impl TripTriageVerdict {
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.artifact.is_none()
    }

    #[must_use]
    pub fn is_some(&self) -> bool {
        self.artifact.is_some()
    }

    #[track_caller]
    pub fn expect(self, message: &str) -> PathBuf {
        self.artifact.expect(message)
    }
}

/// Writes one compact report when a gating channel or advisory geometry differs.
///
/// The result is deterministic for equivalent inputs and contains at most 8
/// KiB. Geometry is retained and countable but never affects `gating_mismatch`.
/// A fully equal comparison leaves no artifact behind.
pub fn write_trip_triage_artifact(
    root: &Path,
    input: TripTriageInput<'_>,
) -> Result<TripTriageVerdict> {
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
    let gating_divergence =
        first_gating_divergence(input, expected_dvi.as_deref(), actual_dvi.as_deref())?;
    let geometry_divergence = event_divergence(
        "geometry_events",
        input.expected.geometry_events,
        input.actual.geometry_events,
        None,
        None,
    )?;
    let Some(divergence) = gating_divergence.as_ref().or(geometry_divergence.as_ref()) else {
        if artifact.exists() {
            fs::remove_file(&artifact)
                .with_context(|| format!("failed to remove stale {}", artifact.display()))?;
        }
        return Ok(TripTriageVerdict {
            artifact: None,
            gating_mismatch: false,
            advisory_geometry_mismatch: false,
        });
    };

    let parent = artifact.parent().expect("artifact has a parent");
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let report = report_text(
        input,
        expected_dvi.as_deref(),
        actual_dvi.as_deref(),
        divergence,
        geometry_divergence.as_ref(),
        gating_divergence.is_some(),
    );
    fs::write(&artifact, report)
        .with_context(|| format!("failed to write {}", artifact.display()))?;
    Ok(TripTriageVerdict {
        artifact: Some(artifact),
        gating_mismatch: gating_divergence.is_some(),
        advisory_geometry_mismatch: geometry_divergence.is_some(),
    })
}

fn first_gating_divergence(
    input: TripTriageInput<'_>,
    expected_dvi: Option<&[u8]>,
    actual_dvi: Option<&[u8]>,
) -> Result<Option<Divergence>> {
    if let Some(divergence) = event_divergence(
        "command_events",
        input.expected.command_events,
        input.actual.command_events,
        input.expected.initialization_events,
        input.actual.initialization_events,
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
    expected_initialization: Option<&[u8]>,
    actual_initialization: Option<&[u8]>,
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
            let mut projection = TripProjection::from_initialization(
                expected_initialization,
                actual_initialization,
            )?;
            let index = expected
                .events
                .iter()
                .zip(&actual.events)
                .position(|(left, right)| !projection.events_match(channel, left, right));
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
#[derive(Default)]
struct TripProjection {
    /// Control sequences whose latest meaning mutation has established the
    /// same complete macro definition on both sides.
    matched_macros: BTreeSet<String>,
}

impl TripProjection {
    fn from_initialization(expected: Option<&[u8]>, actual: Option<&[u8]>) -> Result<Self> {
        let mut projection = Self::default();
        let (Some(expected), Some(actual)) = (expected, actual) else {
            return Ok(projection);
        };
        let expected = ObservationStream::from_canonical_json_lines(expected)
            .context("expected initialization history is not canonical schema-v1 JSONL")?;
        let actual = ObservationStream::from_canonical_json_lines(actual)
            .context("actual initialization history is not canonical schema-v1 JSONL")?;
        for (expected, actual) in expected.events.iter().zip(&actual.events) {
            if !projection.events_match("command_events", expected, actual) {
                return Ok(Self::default());
            }
        }
        // TeX82 §1309 restores a format before the loaded job begins. A
        // terminal observation missing after the shorter initialization
        // stream cannot invalidate macro meanings already established by the
        // common completed history. A one-sided meaning mutation can, so
        // discard only the affected proof from an otherwise exact prefix.
        for event in expected
            .events
            .iter()
            .skip(actual.events.len())
            .chain(actual.events.iter().skip(expected.events.len()))
        {
            if let Some((name, _)) = meaning_mutation(&event.semantic) {
                projection.matched_macros.remove(name);
            }
        }
        Ok(projection)
    }

    fn events_match(
        &mut self,
        channel: &str,
        expected: &NormalizedEvent,
        actual: &NormalizedEvent,
    ) -> bool {
        let matches = expected == actual
            || (channel == "command_events"
                && expected.sequence == actual.sequence
                && (macro_call_operand_is_reference(
                    &expected.semantic,
                    &actual.semantic,
                    &self.matched_macros,
                ) || frozen_endwrite_operand_is_reference(
                    &expected.semantic,
                    &actual.semantic,
                )));
        if channel == "command_events" {
            self.observe_meaning_mutations(&expected.semantic, &actual.semantic);
        }
        matches
    }

    fn observe_meaning_mutations(&mut self, expected: &Event, actual: &Event) {
        let expected = meaning_mutation(expected);
        let actual = meaning_mutation(actual);
        for name in expected
            .map(|(name, _)| name)
            .into_iter()
            .chain(actual.map(|(name, _)| name))
        {
            self.matched_macros.remove(name);
        }
        let (Some((expected_name, expected_mutation)), Some((actual_name, actual_mutation))) =
            (expected, actual)
        else {
            return;
        };
        if expected_name == actual_name
            && expected_mutation == actual_mutation
            && matches!(expected_mutation.value, CanonicalValue::Tokens(_))
        {
            self.matched_macros.insert(expected_name.to_owned());
        }
    }
}

fn meaning_mutation(event: &Event) -> Option<(&str, &MutationEvent)> {
    let Event::Mutation(mutation) = event else {
        return None;
    };
    if mutation.target != StateTarget::Meaning {
        return None;
    }
    let CanonicalValue::Name(name) = &mutation.key else {
        return None;
    };
    Some((name, mutation))
}

fn macro_call_operand_is_reference(
    expected: &Event,
    actual: &Event,
    matched_macros: &BTreeSet<String>,
) -> bool {
    let (
        Event::Command(CommandEvent {
            delivery: expected_delivery,
            command:
                CanonicalCommand {
                    command: expected_command,
                    operand: CanonicalValue::Integer(_),
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

    let Some(expected_control_sequence) = expected_control_sequence.as_deref() else {
        return false;
    };

    matches!(
        actual_operand,
        CanonicalValue::Integer(_) | CanonicalValue::None
    ) && matched_macros.contains(expected_control_sequence)
        && matches!(
            expected_command.as_str(),
            "call" | "long_call" | "outer_call" | "long_outer_call"
        )
        && expected_delivery == actual_delivery
        && expected_command == actual_command
        && Some(expected_control_sequence) == actual_control_sequence.as_deref()
        && expected_location == actual_location
}

/// TeX82 §§222/1369 give the inaccessible `\endwrite` stopper an
/// `outer_call` meaning whose `equiv` is `null`. The observed operand is
/// therefore TeX's representation of that sentinel value, while Umber owns
/// the same empty outer macro in its immutable definition store. Neither
/// integer is semantic identity.
///
/// Unlike ordinary macros, this meaning has no mutation event from which the
/// projection can learn a definition. Its frozen name, command, and lack of
/// source provenance identify the one allocation-insensitive case.
fn frozen_endwrite_operand_is_reference(expected: &Event, actual: &Event) -> bool {
    let (
        Event::Command(CommandEvent {
            delivery: expected_delivery,
            command:
                CanonicalCommand {
                    command: expected_command,
                    operand: CanonicalValue::Integer(_),
                    control_sequence: expected_control_sequence,
                    location: None,
                },
        }),
        Event::Command(CommandEvent {
            delivery: actual_delivery,
            command:
                CanonicalCommand {
                    command: actual_command,
                    operand: actual_operand,
                    control_sequence: actual_control_sequence,
                    location: None,
                },
        }),
    ) = (expected, actual)
    else {
        return false;
    };

    matches!(
        actual_operand,
        CanonicalValue::Integer(_) | CanonicalValue::None
    ) && expected_delivery == actual_delivery
        && expected_command == "outer_call"
        && actual_command == expected_command
        && expected_control_sequence.as_deref() == Some("endwrite")
        && actual_control_sequence == expected_control_sequence
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
    geometry_divergence: Option<&Divergence>,
    gating_mismatch: bool,
) -> String {
    let mut out = String::new();
    let command_accounting = event_accounting(
        "command_events",
        input.expected.command_events,
        input.actual.command_events,
        input.expected.initialization_events,
        input.actual.initialization_events,
    );
    let geometry_accounting = event_accounting(
        "geometry_events",
        input.expected.geometry_events,
        input.actual.geometry_events,
        None,
        None,
    );
    for (key, value) in [
        ("schema", "umber.trip-triage.v1".to_string()),
        ("label", bounded(input.label, 128)),
        ("phase", bounded(input.phase, 128)),
        (
            "status",
            if gating_mismatch {
                "gating-mismatch"
            } else {
                "advisory-geometry-mismatch"
            }
            .to_string(),
        ),
        ("geometry.policy", "advisory-non-gating".to_string()),
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
            "geometry_events.advisory_mismatch",
            geometry_divergence.is_some().to_string(),
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
    expected_initialization: Option<&[u8]>,
    actual_initialization: Option<&[u8]>,
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
    let mut projection =
        TripProjection::from_initialization(expected_initialization, actual_initialization)
            .expect("initialization streams were validated before report rendering");
    let mut projected_equivalent_count = 0;
    let mut projected_divergence_count = 0;
    for (left, right) in expected.events.iter().zip(&actual.events) {
        let matches = projection.events_match(channel, left, right);
        projected_equivalent_count += usize::from(left != right && matches);
        projected_divergence_count += usize::from(!matches);
    }
    projected_divergence_count += expected.events.len().abs_diff(actual.events.len());
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
        MutationEvent, Normalizer, OracleToken, SCHEMA_VERSION, StateTarget,
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
        macro_history_events(operand, name, "call", macro_body(b'a'))
    }

    fn macro_body(character: u8) -> CanonicalValue {
        CanonicalValue::Tokens(vec![
            OracleToken {
                character: 0,
                catcode: "end_match".into(),
                control_sequence: None,
                location: None,
            },
            OracleToken {
                character: u32::from(character),
                catcode: "letter".into(),
                control_sequence: None,
                location: None,
            },
        ])
    }

    fn macro_history_events(
        operand: CanonicalValue,
        name: &str,
        command: &str,
        body: CanonicalValue,
    ) -> Vec<u8> {
        let mut normalizer = Normalizer::new();
        let events = [
            Event::Mutation(MutationEvent {
                target: StateTarget::Meaning,
                key: CanonicalValue::Name(name.into()),
                value: body,
                scope: "local".into(),
            }),
            Event::Command(CommandEvent {
                delivery: tex_oracle::CommandDelivery::Raw,
                command: CanonicalCommand {
                    command: command.into(),
                    operand,
                    control_sequence: Some(name.into()),
                    location: None,
                },
            }),
        ];
        let mut out = format!(
            "{{\"schema\":{SCHEMA_VERSION},\"manifest\":\"{}\"}}\n",
            "a".repeat(64)
        )
        .into_bytes();
        for event in events {
            out.extend_from_slice(
                &serde_json::to_vec(&normalizer.normalize(event)).expect("event"),
            );
            out.push(b'\n');
        }
        out
    }

    fn macro_initialization_events(bodies: &[CanonicalValue]) -> Vec<u8> {
        macro_initialization_events_with_suffix(bodies, &[])
    }

    fn macro_initialization_events_with_suffix(
        bodies: &[CanonicalValue],
        suffix: &[Event],
    ) -> Vec<u8> {
        let mut normalizer = Normalizer::new();
        let mut out = format!(
            "{{\"schema\":{SCHEMA_VERSION},\"manifest\":\"{}\"}}\n",
            "a".repeat(64)
        )
        .into_bytes();
        for body in bodies {
            let event = Event::Mutation(MutationEvent {
                target: StateTarget::Meaning,
                key: CanonicalValue::Name("probe".into()),
                value: body.clone(),
                scope: "local".into(),
            });
            out.extend_from_slice(
                &serde_json::to_vec(&normalizer.normalize(event)).expect("event"),
            );
            out.push(b'\n');
        }
        for event in suffix {
            out.extend_from_slice(
                &serde_json::to_vec(&normalizer.normalize(event.clone())).expect("event"),
            );
            out.push(b'\n');
        }
        out
    }

    fn macro_call_only_events(operand: CanonicalValue, name: &str, command: &str) -> Vec<u8> {
        let mut normalizer = Normalizer::new();
        let event = Event::Command(CommandEvent {
            delivery: tex_oracle::CommandDelivery::Raw,
            command: CanonicalCommand {
                command: command.into(),
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
            initialization_events: None,
            command_events: Some(&reference_events),
            geometry_events: None,
            transcript: b"alpha",
            log: b"log",
            dvi: Some(&reference_dvi),
        };
        let event_mismatch = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&actual_events),
            ..base
        };
        for (actual, channel) in [
            (event_mismatch, "command_events"),
            (
                TripTriageChannels {
                    initialization_events: None,
                    transcript: b"alpHa",
                    ..base
                },
                "transcript",
            ),
            (
                TripTriageChannels {
                    initialization_events: None,
                    log: b"loG",
                    ..base
                },
                "log",
            ),
            (
                TripTriageChannels {
                    initialization_events: None,
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
        // macro's `equiv`. Once both streams have established the same latest
        // complete meaning, any address delta is allocation-only.
        let temp = tempfile::tempdir().expect("temp");
        let reference = macro_command_events(CanonicalValue::Integer(249_985));
        for operand in [
            CanonicalValue::Integer(249_984),
            CanonicalValue::Integer(17),
            CanonicalValue::None,
        ] {
            let actual = macro_command_events(operand);
            let expected = TripTriageChannels {
                initialization_events: None,
                command_events: Some(&reference),
                geometry_events: None,
                transcript: b"same",
                log: b"same",
                dvi: None,
            };
            let actual = TripTriageChannels {
                initialization_events: None,
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
    fn frozen_endwrite_null_operand_is_not_allocator_identity() {
        // TeX82 §§222/1369: `end_write` is an inaccessible frozen
        // `outer_call` whose `equiv` is `null`, not an ordinary definition
        // established by a meaning mutation.
        let command = |operand, name: &str, command: &str, location| {
            Event::Command(CommandEvent {
                delivery: tex_oracle::CommandDelivery::Raw,
                command: CanonicalCommand {
                    command: command.into(),
                    operand,
                    control_sequence: Some(name.into()),
                    location,
                },
            })
        };
        let expected = command(
            CanonicalValue::Integer(-268_435_455),
            "endwrite",
            "outer_call",
            None,
        );
        for actual_operand in [CanonicalValue::Integer(249_877), CanonicalValue::None] {
            let actual = command(actual_operand, "endwrite", "outer_call", None);
            assert!(frozen_endwrite_operand_is_reference(&expected, &actual));
        }

        let source_location = Some(tex_oracle::SourceLocation {
            source: "probe.tex".into(),
            line: 1,
            byte: 0,
        });
        for actual in [
            command(
                CanonicalValue::Integer(249_877),
                "other",
                "outer_call",
                None,
            ),
            command(CanonicalValue::Integer(249_877), "endwrite", "call", None),
            command(
                CanonicalValue::Integer(249_877),
                "endwrite",
                "outer_call",
                source_location,
            ),
        ] {
            assert!(!frozen_endwrite_operand_is_reference(&expected, &actual));
        }
    }

    #[test]
    fn format_loaded_macro_call_uses_matching_initialization_definition() {
        // TeX82 §§382/1309: a loaded call has no in-phase meaning mutation,
        // so the completed INITEX definition is the proof that only the
        // allocation-owned `def_ref` differs.
        let temp = tempfile::tempdir().expect("temp");
        let reference_history = macro_history_events(
            CanonicalValue::Integer(249_985),
            "probe",
            "call",
            macro_body(b'a'),
        );
        let actual_history = macro_history_events(
            CanonicalValue::Integer(17),
            "probe",
            "call",
            macro_body(b'a'),
        );
        let reference_call =
            macro_call_only_events(CanonicalValue::Integer(249_985), "probe", "call");
        let actual_call = macro_call_only_events(CanonicalValue::Integer(17), "probe", "call");
        let expected = TripTriageChannels {
            initialization_events: Some(&reference_history),
            command_events: Some(&reference_call),
            geometry_events: None,
            transcript: b"same",
            log: b"same",
            dvi: None,
        };
        let actual = TripTriageChannels {
            initialization_events: Some(&actual_history),
            command_events: Some(&actual_call),
            ..expected
        };
        assert!(
            write_trip_triage_artifact(temp.path(), input(expected, actual))
                .expect("comparison")
                .is_none()
        );

        let changed_history = macro_history_events(
            CanonicalValue::Integer(17),
            "probe",
            "call",
            macro_body(b'b'),
        );
        let artifact = write_trip_triage_artifact(
            temp.path(),
            input(
                expected,
                TripTriageChannels {
                    initialization_events: Some(&changed_history),
                    ..actual
                },
            ),
        )
        .expect("comparison")
        .expect("changed loaded definition must diverge");
        assert!(
            fs::read_to_string(artifact)
                .expect("report")
                .contains("earliest.position: event[0]")
        );

        let reference_history = macro_initialization_events(&[macro_body(b'a'), macro_body(b'b')]);
        let actual_history = macro_initialization_events(&[macro_body(b'a')]);
        let artifact = write_trip_triage_artifact(
            temp.path(),
            input(
                TripTriageChannels {
                    initialization_events: Some(&reference_history),
                    ..expected
                },
                TripTriageChannels {
                    initialization_events: Some(&actual_history),
                    ..actual
                },
            ),
        )
        .expect("comparison")
        .expect("one-sided loaded redefinition must invalidate the proof");
        assert!(
            fs::read_to_string(artifact)
                .expect("report")
                .contains("earliest.position: event[0]")
        );
    }

    #[test]
    fn format_loaded_macro_call_survives_missing_initialization_termination() {
        // TeX82 §1309: format restoration uses the completed INITEX meaning.
        // A missing terminal observation is an envelope defect after that
        // definition, not evidence that its allocator-owned `def_ref` differs
        // semantically in the loaded run.
        let temp = tempfile::tempdir().expect("temp");
        let body = macro_body(b'a');
        let terminal = Event::Input(tex_oracle::InputEvent {
            transition: tex_oracle::InputTransition::Stop,
            reason: tex_oracle::InputReason::Source,
            name: "terminal".into(),
        });
        let reference_history =
            macro_initialization_events_with_suffix(std::slice::from_ref(&body), &[terminal]);
        let actual_history = macro_initialization_events(std::slice::from_ref(&body));
        let reference_call =
            macro_call_only_events(CanonicalValue::Integer(249_985), "probe", "call");
        let actual_call = macro_call_only_events(CanonicalValue::Integer(17), "probe", "call");
        let expected = TripTriageChannels {
            initialization_events: Some(&reference_history),
            command_events: Some(&reference_call),
            geometry_events: None,
            transcript: b"same",
            log: b"same",
            dvi: None,
        };
        let actual = TripTriageChannels {
            initialization_events: Some(&actual_history),
            command_events: Some(&actual_call),
            ..expected
        };

        assert!(
            write_trip_triage_artifact(temp.path(), input(expected, actual))
                .expect("comparison")
                .is_none()
        );
    }

    #[test]
    fn macro_call_projection_preserves_definition_body_differences() {
        let temp = tempfile::tempdir().expect("temp");
        let reference = macro_history_events(
            CanonicalValue::Integer(249_985),
            "probe",
            "call",
            macro_body(b'a'),
        );
        let actual = macro_history_events(
            CanonicalValue::Integer(249_983),
            "probe",
            "call",
            macro_body(b'b'),
        );
        let expected = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&reference),
            geometry_events: None,
            transcript: b"same",
            log: b"same",
            dvi: None,
        };
        let actual = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&actual),
            ..expected
        };

        let artifact = write_trip_triage_artifact(temp.path(), input(expected, actual))
            .expect("comparison")
            .expect("different definition bodies must diverge");
        let report = fs::read_to_string(artifact).expect("report");
        assert!(report.contains("earliest.position: event[0]"), "{report}");
        assert!(
            report.contains("expected.command_events.count: 2"),
            "{report}"
        );
        assert!(
            report.contains("actual.command_events.count: 2"),
            "{report}"
        );
        assert!(
            report.contains("command_events.projected_equivalent.count: 0"),
            "{report}"
        );
        assert!(
            report.contains("command_events.projected_divergence.count: 2"),
            "{report}"
        );
    }

    #[test]
    fn macro_call_projection_preserves_macro_identity_differences() {
        let temp = tempfile::tempdir().expect("temp");
        let reference = macro_command_events_named(CanonicalValue::Integer(249_985), "older");
        let actual = macro_command_events_named(CanonicalValue::Integer(249_986), "newer");
        let expected = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&reference),
            geometry_events: None,
            transcript: b"same",
            log: b"same",
            dvi: None,
        };
        let actual = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&actual),
            ..expected
        };

        let artifact = write_trip_triage_artifact(temp.path(), input(expected, actual))
            .expect("comparison")
            .expect("a different macro identity must diverge");
        let report = fs::read_to_string(artifact).expect("report");
        assert!(report.contains("earliest.position: event[0]"), "{report}");
        assert!(
            report.contains("command_events.projected_divergence.count: 2"),
            "{report}"
        );
    }

    #[test]
    fn macro_call_projection_requires_matching_definition_history_and_flags() {
        let temp = tempfile::tempdir().expect("temp");
        let reference = macro_history_events(
            CanonicalValue::Integer(249_985),
            "probe",
            "call",
            macro_body(b'a'),
        );
        let unmatched = macro_call_only_events(CanonicalValue::Integer(17), "probe", "call");
        let changed_flags = macro_history_events(
            CanonicalValue::Integer(17),
            "probe",
            "long_call",
            macro_body(b'a'),
        );
        for actual in [&unmatched, &changed_flags] {
            let expected = TripTriageChannels {
                initialization_events: None,
                command_events: Some(&reference),
                geometry_events: None,
                transcript: b"same",
                log: b"same",
                dvi: None,
            };
            let actual = TripTriageChannels {
                initialization_events: None,
                command_events: Some(actual),
                ..expected
            };
            assert!(
                write_trip_triage_artifact(temp.path(), input(expected, actual))
                    .expect("comparison")
                    .is_some()
            );
        }
    }

    #[test]
    fn reports_are_reproducible_and_never_embed_an_unbounded_log() {
        let temp = tempfile::tempdir().expect("temp");
        let events = events(1);
        let dvi = dvi(b"ref", &[140]);
        let long_log = vec![b'x'; ARTIFACT_LIMIT * 4];
        let expected = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&events),
            geometry_events: None,
            transcript: b"same",
            log: &long_log,
            dvi: Some(&dvi),
        };
        let actual = TripTriageChannels {
            initialization_events: None,
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
    fn gating_channels_precede_advisory_geometry() {
        let temp = tempfile::tempdir().expect("temp");
        let command = events(1);
        let changed_command = events(2);
        let geometry = geometry_events(117);
        let changed_geometry = geometry_events(118);
        let dvi = dvi(b"ref", &[140]);
        let base = TripTriageChannels {
            initialization_events: None,
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
                    initialization_events: None,
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

        let verdict = write_trip_triage_artifact(
            temp.path(),
            input(
                base,
                TripTriageChannels {
                    initialization_events: None,
                    geometry_events: Some(&changed_geometry),
                    transcript: b"different",
                    ..base
                },
            ),
        )
        .expect("mixed comparison");
        assert!(verdict.gating_mismatch);
        assert!(verdict.advisory_geometry_mismatch);
        let report =
            fs::read_to_string(verdict.expect("mixed mismatch report")).expect("mixed report");
        assert!(report.contains("earliest.channel: transcript"), "{report}");
        assert!(
            report.contains("geometry_events.projected_divergence.count: 1"),
            "{report}"
        );

        let verdict = write_trip_triage_artifact(
            temp.path(),
            input(
                base,
                TripTriageChannels {
                    initialization_events: None,
                    geometry_events: Some(&changed_geometry),
                    ..base
                },
            ),
        )
        .expect("advisory comparison");
        assert!(!verdict.gating_mismatch);
        assert!(verdict.advisory_geometry_mismatch);
        let report =
            fs::read_to_string(verdict.expect("advisory report")).expect("advisory report bytes");
        assert!(
            report.contains("status: advisory-geometry-mismatch"),
            "{report}"
        );
        assert!(
            report.contains("earliest.channel: geometry_events"),
            "{report}"
        );
    }
}
