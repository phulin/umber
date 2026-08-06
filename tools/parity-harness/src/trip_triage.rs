//! Bounded, schema-aware TRIP mismatch reports.
//!
//! The report intentionally contains identities and a small semantic context,
//! never copied transcripts, logs, DVI files, or full event streams.
//! Complete TeX82 §638 shipout allocator-accounting records are parsed and
//! retained as advisory evidence; every other transcript/log byte still gates.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use test_support::dvi::normalized_dvi_for_comparison;
use tex_command_stream::{
    StrictTripAccounting, StrictTripChannel, StrictTripComparison, StrictTripComparisonPolicy,
    StrictTripDivergence,
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
    /// Canonical schema-v2 or schema-v3 geometry JSONL, kept identity-separate from v1.
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
    /// TeX82 §638 allocator-accounting records differed after typed removal.
    pub advisory_memory_usage_mismatch: bool,
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
    let text = TextComparisons::new(input);
    let command_comparison = event_comparison(
        StrictTripChannel::Command,
        input.expected.command_events,
        input.actual.command_events,
        input.expected.initialization_events,
        input.actual.initialization_events,
    )?;
    let command_divergence = event_divergence("command_events", &command_comparison);
    let gating_divergence = first_gating_divergence(
        &command_divergence,
        &text,
        expected_dvi.as_deref(),
        actual_dvi.as_deref(),
    );
    let memory_usage_divergence = text.memory_usage_divergence();
    let geometry_comparison = event_comparison(
        StrictTripChannel::Geometry,
        input.expected.geometry_events,
        input.actual.geometry_events,
        None,
        None,
    )?;
    let geometry_divergence = event_divergence("geometry_events", &geometry_comparison);
    let Some(divergence) = gating_divergence
        .as_ref()
        .or(memory_usage_divergence.as_ref())
        .or(geometry_divergence.as_ref())
    else {
        if artifact.exists() {
            fs::remove_file(&artifact)
                .with_context(|| format!("failed to remove stale {}", artifact.display()))?;
        }
        return Ok(TripTriageVerdict {
            artifact: None,
            gating_mismatch: false,
            advisory_geometry_mismatch: false,
            advisory_memory_usage_mismatch: false,
        });
    };

    let parent = artifact.parent().expect("artifact has a parent");
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let report = report_text(
        input,
        &text,
        (expected_dvi.as_deref(), actual_dvi.as_deref()),
        ReportAssessment {
            primary: divergence,
            memory_usage: memory_usage_divergence.as_ref(),
            geometry: geometry_divergence.as_ref(),
            gating_mismatch: gating_divergence.is_some(),
            command_accounting: command_comparison.accounting,
            geometry_accounting: geometry_comparison.accounting,
        },
    );
    fs::write(&artifact, report)
        .with_context(|| format!("failed to write {}", artifact.display()))?;
    Ok(TripTriageVerdict {
        artifact: Some(artifact),
        gating_mismatch: gating_divergence.is_some(),
        advisory_geometry_mismatch: geometry_divergence.is_some(),
        advisory_memory_usage_mismatch: memory_usage_divergence.is_some(),
    })
}

fn first_gating_divergence(
    command: &Option<Divergence>,
    text: &TextComparisons,
    expected_dvi: Option<&[u8]>,
    actual_dvi: Option<&[u8]>,
) -> Option<Divergence> {
    if let Some(divergence) = command {
        return Some(divergence.clone());
    }
    if let Some(divergence) = byte_divergence(
        "transcript",
        &text.transcript.expected,
        &text.transcript.actual,
    ) {
        return Some(divergence);
    }
    if let Some(divergence) = byte_divergence("log", &text.log.expected, &text.log.actual) {
        return Some(divergence);
    }
    optional_byte_divergence("normalized_dvi", expected_dvi, actual_dvi)
}

/// A text channel with only complete TeX82 §638 shipout accounting records
/// removed. These values expose the reference allocator's variable/dynamic
/// node occupancy and free-memory gap; they are not engine semantics.
struct TextComparison {
    expected: Vec<u8>,
    actual: Vec<u8>,
    expected_memory_usage: Vec<[u32; 5]>,
    actual_memory_usage: Vec<[u32; 5]>,
}

impl TextComparison {
    fn new(expected: &[u8], actual: &[u8]) -> Self {
        let (expected, expected_memory_usage) = split_memory_usage_records(expected);
        let (actual, actual_memory_usage) = split_memory_usage_records(actual);
        Self {
            expected,
            actual,
            expected_memory_usage,
            actual_memory_usage,
        }
    }

    fn memory_usage_divergence(&self, channel: &'static str) -> Option<Divergence> {
        (self.expected_memory_usage != self.actual_memory_usage).then(|| Divergence {
            channel,
            position: "records".into(),
            expected: format!("{} canonical record(s)", self.expected_memory_usage.len()),
            actual: format!("{} canonical record(s)", self.actual_memory_usage.len()),
        })
    }
}

struct TextComparisons {
    transcript: TextComparison,
    log: TextComparison,
}

impl TextComparisons {
    fn new(input: TripTriageInput<'_>) -> Self {
        Self {
            transcript: TextComparison::new(input.expected.transcript, input.actual.transcript),
            log: TextComparison::new(input.expected.log, input.actual.log),
        }
    }

    fn memory_usage_divergence(&self) -> Option<Divergence> {
        self.transcript
            .memory_usage_divergence("transcript.memory_usage")
            .or_else(|| self.log.memory_usage_divergence("log.memory_usage"))
    }
}

fn split_memory_usage_records(bytes: &[u8]) -> (Vec<u8>, Vec<[u32; 5]>) {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut records = Vec::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if let Some(record) = parse_memory_usage_record(line) {
            records.push(record);
        } else {
            normalized.extend_from_slice(line);
        }
    }
    (normalized, records)
}

fn parse_memory_usage_record(line: &[u8]) -> Option<[u32; 5]> {
    const PREFIX: &[u8] = b"Memory usage before: ";
    const AFTER: &[u8] = b"; after: ";
    const UNTOUCHED: &[u8] = b"; still untouched: ";
    let mut rest = line.strip_prefix(PREFIX)?;
    let before_var = parse_memory_usage_value(&mut rest, b"&")?;
    let before_dyn = parse_memory_usage_value(&mut rest, AFTER)?;
    let after_var = parse_memory_usage_value(&mut rest, b"&")?;
    let after_dyn = parse_memory_usage_value(&mut rest, UNTOUCHED)?;
    let untouched = parse_memory_usage_value(&mut rest, b"\n")?;
    rest.is_empty()
        .then_some([before_var, before_dyn, after_var, after_dyn, untouched])
}

fn parse_memory_usage_value(rest: &mut &[u8], separator: &[u8]) -> Option<u32> {
    let index = rest
        .windows(separator.len())
        .position(|window| window == separator)?;
    let digits = &rest[..index];
    if digits.is_empty() || digits.len() > 10 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    // TeX82's `print_int` takes a signed 32-bit `integer`. Parsing the value,
    // rather than accepting an arbitrary digit run, therefore supplies both
    // the canonical numeric bound and a fixed record-length bound.
    let value = std::str::from_utf8(digits).ok()?.parse::<i32>().ok()?;
    *rest = &rest[index + separator.len()..];
    Some(value as u32)
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

fn event_comparison(
    channel: StrictTripChannel,
    expected: Option<&[u8]>,
    actual: Option<&[u8]>,
    expected_initialization: Option<&[u8]>,
    actual_initialization: Option<&[u8]>,
) -> Result<StrictTripComparison> {
    StrictTripComparisonPolicy {
        channel,
        expected_initialization,
        actual_initialization,
    }
    .compare(expected, actual)
    .map_err(anyhow::Error::msg)
}

fn event_divergence(
    channel: &'static str,
    comparison: &StrictTripComparison,
) -> Option<Divergence> {
    match comparison.divergence.as_ref()? {
        StrictTripDivergence::Presence => Some(Divergence::presence(channel)),
        StrictTripDivergence::Header { expected, actual } => {
            Some(Divergence::event_header(channel, expected, actual))
        }
        StrictTripDivergence::Event {
            index,
            expected,
            actual,
        } => Some(Divergence::event(
            channel,
            *index,
            expected.as_deref(),
            actual.as_deref(),
        )),
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

#[derive(Clone, Debug)]
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

struct ReportAssessment<'a> {
    primary: &'a Divergence,
    memory_usage: Option<&'a Divergence>,
    geometry: Option<&'a Divergence>,
    gating_mismatch: bool,
    command_accounting: StrictTripAccounting,
    geometry_accounting: StrictTripAccounting,
}

fn report_text(
    input: TripTriageInput<'_>,
    text: &TextComparisons,
    dvi: (Option<&[u8]>, Option<&[u8]>),
    assessment: ReportAssessment<'_>,
) -> String {
    let (expected_dvi, actual_dvi) = dvi;
    let mut out = String::new();
    let command_accounting = assessment.command_accounting;
    let geometry_accounting = assessment.geometry_accounting;
    for (key, value) in [
        ("schema", "umber.trip-triage.v1".to_string()),
        ("label", bounded(input.label, 128)),
        ("phase", bounded(input.phase, 128)),
        (
            "status",
            if assessment.gating_mismatch {
                "gating-mismatch"
            } else if assessment.memory_usage.is_some() {
                "advisory-memory-usage-mismatch"
            } else {
                "advisory-geometry-mismatch"
            }
            .to_string(),
        ),
        ("geometry.policy", "advisory-non-gating".to_string()),
        (
            "memory_usage.policy",
            "tex82-section-638-allocator-accounting-advisory-non-gating".to_string(),
        ),
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
            presence_count_text(command_accounting.expected_events),
        ),
        (
            "actual.command_events.count",
            presence_count_text(command_accounting.actual_events),
        ),
        (
            "command_events.projected_equivalent.count",
            count_text(command_accounting.projected_equivalent),
        ),
        (
            "command_events.projected_divergence.count",
            count_text(command_accounting.projected_divergences),
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
            presence_count_text(geometry_accounting.expected_events),
        ),
        (
            "actual.geometry_events.count",
            presence_count_text(geometry_accounting.actual_events),
        ),
        (
            "geometry_events.projected_divergence.count",
            count_text(geometry_accounting.projected_divergences),
        ),
        (
            "geometry_events.advisory_mismatch",
            assessment.geometry.is_some().to_string(),
        ),
        (
            "expected.transcript.sha256",
            sha256(input.expected.transcript),
        ),
        ("actual.transcript.sha256", sha256(input.actual.transcript)),
        (
            "expected.transcript.memory_usage.count",
            text.transcript.expected_memory_usage.len().to_string(),
        ),
        (
            "actual.transcript.memory_usage.count",
            text.transcript.actual_memory_usage.len().to_string(),
        ),
        ("expected.log.sha256", sha256(input.expected.log)),
        ("actual.log.sha256", sha256(input.actual.log)),
        (
            "expected.log.memory_usage.count",
            text.log.expected_memory_usage.len().to_string(),
        ),
        (
            "actual.log.memory_usage.count",
            text.log.actual_memory_usage.len().to_string(),
        ),
        (
            "memory_usage.advisory_mismatch",
            assessment.memory_usage.is_some().to_string(),
        ),
        ("expected.normalized_dvi.sha256", option_hash(expected_dvi)),
        ("actual.normalized_dvi.sha256", option_hash(actual_dvi)),
        ("earliest.channel", assessment.primary.channel.to_string()),
        ("earliest.position", assessment.primary.position.clone()),
        ("earliest.expected", assessment.primary.expected.clone()),
        ("earliest.actual", assessment.primary.actual.clone()),
    ] {
        bounded_line(&mut out, key, &value);
    }
    out
}

fn count_text(count: Option<usize>) -> String {
    count.map_or_else(|| "unavailable".into(), |count| count.to_string())
}

fn presence_count_text(count: Option<usize>) -> String {
    count.map_or_else(|| "absent".into(), |count| count.to_string())
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
        CanonicalCommand, CanonicalValue, CommandDelivery, CommandEvent, EffectEvent, EffectKind,
        Event, InputEvent, InputReason, InputTransition, MutationEvent, Normalizer, OracleToken,
        SCHEMA_VERSION, SchemaVersion, StateTarget, TokenListEvent, TokenListTransition,
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
        geometry_events_for_schema(count0, SchemaVersion::V2, None)
    }

    fn geometry_events_for_schema(
        count0: i32,
        schema: SchemaVersion,
        location: Option<tex_oracle::GeometryLocation>,
    ) -> Vec<u8> {
        let mut normalizer = Normalizer::new();
        let mut counts = [0; 10];
        counts[0] = count0;
        let event = Event::Geometry(tex_oracle::GeometryEvent::Shipout {
            page_width_sp: 10,
            page_height_sp: 20,
            counts,
            location,
        });
        let mut out = format!(
            "{{\"schema\":{},\"manifest\":\"{}\"}}\n",
            schema.number(),
            "b".repeat(64)
        )
        .into_bytes();
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

    fn scoped_macro_override_events(operand: CanonicalValue, override_scope: &str) -> Vec<u8> {
        let command = |delivery, name: &str, control_sequence: &str, operand| {
            Event::Command(CommandEvent {
                delivery,
                command: CanonicalCommand {
                    command: name.into(),
                    operand,
                    control_sequence: Some(control_sequence.into()),
                    location: None,
                },
            })
        };
        let events = vec![
            Event::Mutation(MutationEvent {
                target: StateTarget::Meaning,
                key: CanonicalValue::Name("probe".into()),
                value: macro_body(b'a'),
                scope: "global".into(),
            }),
            command(
                CommandDelivery::Raw,
                "begin_group",
                "begingroup",
                CanonicalValue::Integer(0),
            ),
            command(
                CommandDelivery::Expanded,
                "begin_group",
                "begingroup",
                CanonicalValue::Integer(0),
            ),
            Event::Mutation(MutationEvent {
                target: StateTarget::Meaning,
                key: CanonicalValue::Name("probe".into()),
                value: CanonicalValue::Name("hskip".into()),
                scope: override_scope.into(),
            }),
            command(
                CommandDelivery::Raw,
                "end_group",
                "endgroup",
                CanonicalValue::Integer(0),
            ),
            command(
                CommandDelivery::Expanded,
                "end_group",
                "endgroup",
                CanonicalValue::Integer(0),
            ),
            command(CommandDelivery::Raw, "call", "probe", operand),
        ];
        let mut normalizer = Normalizer::new();
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
    fn only_complete_shipout_memory_records_are_advisory() {
        let temp = tempfile::tempdir().expect("temp");
        let expected_text = b"compute silently for awhile,...\nMemory usage before: 159&313; after: 158&309; still untouched: 19649\n\nOverfull \\hbox\n! Missing number, treated as zero.\narbitrary text\n";
        let actual_text = b"compute silently for awhile,...\n\nOverfull \\hbox\n! Missing number, treated as zero.\narbitrary text\n";
        let expected = TripTriageChannels {
            initialization_events: None,
            command_events: None,
            geometry_events: None,
            transcript: expected_text,
            log: expected_text,
            dvi: None,
        };
        let actual = TripTriageChannels {
            transcript: actual_text,
            log: actual_text,
            ..expected
        };
        let verdict = write_trip_triage_artifact(temp.path(), input(expected, actual))
            .expect("advisory comparison");
        assert!(!verdict.gating_mismatch);
        assert!(verdict.advisory_memory_usage_mismatch);
        let report = fs::read_to_string(verdict.expect("advisory report")).expect("report");
        assert!(
            report.contains("earliest.channel: transcript.memory_usage"),
            "{report}"
        );
        assert!(
            report.contains("expected.transcript.memory_usage.count: 1"),
            "{report}"
        );
        assert!(
            report.contains("actual.transcript.memory_usage.count: 0"),
            "{report}"
        );

        for changed in [
            b"compute silently for awhile,...\n\nOverfull changed\n! Missing number, treated as zero.\narbitrary text\n".as_slice(),
            b"compute silently for awhile,...\n\n! Missing number, treated as zero.\nOverfull \\hbox\narbitrary text\n".as_slice(),
            b"compute silently for awhile,...\n\nOverfull \\hbox\n! Missing number, treated as zero.\nchanged text\n".as_slice(),
            b"compute silently for awhile,...\nMemory usage before: x&313; after: 158&309; still untouched: 19649\n\nOverfull \\hbox\n! Missing number, treated as zero.\narbitrary text\n".as_slice(),
        ] {
            let changed = TripTriageChannels {
                transcript: changed,
                log: actual_text,
                ..actual
            };
            let verdict = write_trip_triage_artifact(temp.path(), input(expected, changed))
                .expect("negative control");
            assert!(verdict.gating_mismatch, "changed text must remain gating");
            let report = fs::read_to_string(verdict.expect("gating report")).expect("report");
            assert!(report.contains("earliest.channel: transcript"), "{report}");
        }
    }

    #[test]
    fn shipout_memory_record_parser_is_complete_ascii_and_bounded() {
        assert_eq!(
            parse_memory_usage_record(
                b"Memory usage before: 159&313; after: 158&309; still untouched: 19649\n"
            ),
            Some([159, 313, 158, 309, 19_649])
        );

        for malformed in [
            b"Memory usage before: -1&313; after: 158&309; still untouched: 19649\n".as_slice(),
            b"Memory usage before: +1&313; after: 158&309; still untouched: 19649\n".as_slice(),
            b"Memory usage before: 2147483648&313; after: 158&309; still untouched: 19649\n"
                .as_slice(),
            b"Memory usage before: 00000000001&313; after: 158&309; still untouched: 19649\n"
                .as_slice(),
            b"Memory usage before: 159 &313; after: 158&309; still untouched: 19649\n".as_slice(),
            b"Memory usage before: 159&313;  after: 158&309; still untouched: 19649\n".as_slice(),
            b"Memory usage before: 159&313; after: 158&309; still untouched: 19649 \n".as_slice(),
            b"Memory usage before: 159&313; after: 158&309; still untouched: 19649\r\n".as_slice(),
            b"Memory usage before: 159&313; after: 158&309; still untouched: 19649".as_slice(),
            b"prefix Memory usage before: 159&313; after: 158&309; still untouched: 19649\n"
                .as_slice(),
            b"Memory usage before: 159&313; after: 158&309; still untouched: 19649\ntrailing"
                .as_slice(),
        ] {
            assert_eq!(parse_memory_usage_record(malformed), None, "{malformed:?}");
        }
    }

    #[test]
    fn malformed_memory_like_lines_remain_in_place_and_gating() {
        let canonical =
            b"before\nMemory usage before: 1&2; after: 3&4; still untouched: 5\nafter\n";
        let (without_record, records) = split_memory_usage_records(canonical);
        assert_eq!(without_record, b"before\nafter\n");
        assert_eq!(records, vec![[1, 2, 3, 4, 5]]);

        for malformed in [
            b"before\nMemory usage before: 2147483648&2; after: 3&4; still untouched: 5\nafter\n"
                .as_slice(),
            b"before\nMemory usage before: 1&2; after: 3&4; still untouched: 5\r\nafter\n"
                .as_slice(),
            b"before Memory usage before: 1&2; after: 3&4; still untouched: 5\nafter\n".as_slice(),
        ] {
            let (preserved, records) = split_memory_usage_records(malformed);
            assert_eq!(preserved, malformed);
            assert!(records.is_empty());
        }
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
    fn macro_call_projection_restores_local_explicit_group_overrides() {
        // TeX82 §§277/282: a local meaning saves the prior eqtb value, and
        // `unsave` restores it when the semi-simple group ends. The restored
        // macro's `def_ref` remains allocator-owned (§382).
        let temp = tempfile::tempdir().expect("temp");
        let reference = scoped_macro_override_events(CanonicalValue::Integer(249_682), "local");
        let actual = scoped_macro_override_events(CanonicalValue::Integer(23), "local");
        let expected = TripTriageChannels {
            initialization_events: None,
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

        let globally_overridden =
            scoped_macro_override_events(CanonicalValue::Integer(23), "global");
        assert!(
            write_trip_triage_artifact(
                temp.path(),
                input(
                    expected,
                    TripTriageChannels {
                        command_events: Some(&globally_overridden),
                        ..actual
                    },
                ),
            )
            .expect("comparison")
            .is_some(),
            "§282 must not restore a global override"
        );
    }

    #[test]
    fn protected_alignment_delivery_marker_is_reference_instrumentation_only() {
        // e-TeX [53a] returns a protected macro directly from `get_token`;
        // [37.785]/[37.791] then back that same token up for the alignment
        // template. The reference-only marker precedes, but does not perform,
        // either semantic transition.
        let backup = Event::Input(InputEvent {
            transition: InputTransition::Push,
            reason: InputReason::Backup,
            name: "backup".into(),
        });
        let marker = Event::TokenList(TokenListEvent {
            transition: TokenListTransition::Splice,
            purpose: "protected_delivery_suppression".into(),
            tokens: vec![OracleToken {
                character: 0,
                catcode: "escape".into(),
                control_sequence: Some("probe".into()),
                location: None,
            }],
        });
        let reference = macro_initialization_events_with_suffix(&[], &[marker, backup.clone()]);
        let actual = macro_initialization_events_with_suffix(&[], &[backup]);
        let temp = tempfile::tempdir().expect("temp");
        let expected = TripTriageChannels {
            initialization_events: None,
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

    #[test]
    fn schema_three_geometry_writes_and_preserves_source_locations() {
        let temp = tempfile::tempdir().expect("temp");
        let command = events(1);
        let dvi = dvi(b"ref", &[140]);
        let expected = geometry_events_for_schema(
            117,
            SchemaVersion::V3,
            Some(tex_oracle::GeometryLocation {
                source: "trip.tex".into(),
                line: 105,
            }),
        );
        let actual = geometry_events_for_schema(
            118,
            SchemaVersion::V3,
            Some(tex_oracle::GeometryLocation {
                source: "trip.tex".into(),
                line: 106,
            }),
        );
        let base = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&command),
            geometry_events: Some(&expected),
            transcript: b"same",
            log: b"same",
            dvi: Some(&dvi),
        };
        let verdict = write_trip_triage_artifact(
            temp.path(),
            input(
                base,
                TripTriageChannels {
                    geometry_events: Some(&actual),
                    ..base
                },
            ),
        )
        .expect("schema-v3 geometry comparison writes");
        assert!(verdict.advisory_geometry_mismatch);
        let report = fs::read_to_string(verdict.expect("schema-v3 report")).expect("report");
        assert!(
            report.contains("\"source\":\"trip.tex\",\"line\":105"),
            "{report}"
        );
        assert!(
            report.contains("\"source\":\"trip.tex\",\"line\":106"),
            "{report}"
        );
    }

    #[test]
    fn schema_two_geometry_remains_compatible() {
        let temp = tempfile::tempdir().expect("temp");
        let command = events(1);
        let geometry = geometry_events(117);
        let dvi = dvi(b"ref", &[140]);
        let channels = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&command),
            geometry_events: Some(&geometry),
            transcript: b"same",
            log: b"same",
            dvi: Some(&dvi),
        };

        assert!(
            write_trip_triage_artifact(temp.path(), input(channels, channels))
                .expect("schema-v2 comparison")
                .is_none()
        );
    }

    #[test]
    fn geometry_schema_mismatch_is_rejected() {
        let temp = tempfile::tempdir().expect("temp");
        let command = events(1);
        let v2 = geometry_events_for_schema(117, SchemaVersion::V2, None);
        let v3 = geometry_events_for_schema(
            117,
            SchemaVersion::V3,
            Some(tex_oracle::GeometryLocation {
                source: "trip.tex".into(),
                line: 105,
            }),
        );
        let dvi = dvi(b"ref", &[140]);
        let base = TripTriageChannels {
            initialization_events: None,
            command_events: Some(&command),
            geometry_events: Some(&v2),
            transcript: b"same",
            log: b"same",
            dvi: Some(&dvi),
        };
        let error = write_trip_triage_artifact(
            temp.path(),
            input(
                base,
                TripTriageChannels {
                    geometry_events: Some(&v3),
                    ..base
                },
            ),
        )
        .expect_err("schema mismatch must fail");

        assert!(
            error.to_string().contains("expected v2, actual v3"),
            "{error:#}"
        );
    }
}
