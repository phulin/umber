//! Every observable a minifixture run produces, and the contract that each one
//! must be accounted for.
//!
//! A `projection` selects one observable and asserts a handful of strings about
//! it. That is a focused property claim, and it stays. What it is not is
//! coverage: the same run also writes a terminal transcript, a log, shipped
//! pages, and ordinary effects, and before this module nothing compared any of
//! them. Measured corpus runs produced far more observations than their concise
//! projections declared, including shipped pages and complete logs that no
//! projection read at all.
//!
//! So a case declares a disposition for *every* channel here. A channel with
//! no disposition fails validation rather than passing quietly, for the same
//! reason `default-members` naming 21 of 34 crates was a defect rather than a
//! configuration: an omission that reads as coverage is worse than a red gate.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tex_oracle::{
    EffectEvent, EffectKind, Event, EventObserver, JsonLinesObserver, ManifestIdentity,
    SchemaVersion,
};
use tex_state::{EffectRecord, PrintSink};

use super::{SemanticRun, valid_bug_id};

/// The stream channels, in the order a report prints them.
///
/// `events` and `status` are scalars rather than streams and are declared
/// inline in the manifest, so they are not part of this list.
pub const STREAM_CHANNELS: [StreamChannel; 5] = [
    StreamChannel::Terminal,
    StreamChannel::Log,
    StreamChannel::Dvi,
    StreamChannel::Effects,
    StreamChannel::Diagnostics,
];

/// One channel whose content is a byte stream rather than a scalar.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum StreamChannel {
    /// TeX82's terminal transcript: `PrintSink::Terminal` and `TerminalAndLog`.
    Terminal,
    /// TeX82's log transcript: `PrintSink::Log` and `TerminalAndLog`.
    Log,
    /// The complete serialized `.dvi` file the run produced, byte for byte --
    /// the same object the oracle's own `.dvi` file is, so this channel can
    /// finally be compared against it directly. A hash listing (this
    /// channel's earlier disposition) could never become oracle-authoritative
    /// for exactly that reason: there is no oracle hash to compare against,
    /// only oracle bytes.
    Dvi,
    /// Ordinary effects that are not stream writes: specials, deferred
    /// `\write`s, stream opens and closes, and shell escapes.
    Effects,
    /// Schema-v4 typed diagnostic reports and their final history/outcome.
    Diagnostics,
}

impl StreamChannel {
    /// The manifest key and committed-file extension for this channel.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Log => "log",
            Self::Dvi => "dvi",
            Self::Effects => "effects",
            Self::Diagnostics => "diagnostics",
        }
    }
}

/// Everything one completed run emitted.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapturedChannels {
    /// Ordered committed observations. Counted rather than committed: the
    /// canonical event stream has an oracle-backed home in the `tex82`
    /// fixtures, and duplicating it here would commit an Umber self-golden.
    pub events: usize,
    /// `clean`, or `fatal:<label>` when the job ended through §81 `jump_out`.
    pub status: String,
    /// Rendered stream channels, in [`STREAM_CHANNELS`] order. Bytes rather
    /// than `String`, because the `dvi` channel is a real serialized DVI
    /// file and lossily reencoding it as UTF-8 would corrupt exactly the
    /// bytes a byte-exact oracle comparison exists to check.
    pub streams: [Vec<u8>; 5],
}

impl CapturedChannels {
    /// Captures every channel from a completed run.
    #[must_use]
    pub fn capture(run: &SemanticRun) -> Self {
        // Both channels seed from `World`'s durable per-sink archive rather
        // than starting empty: `commit_effects` (tex-state) drains
        // `effect_records()` at every commit boundary (every `\shipout`
        // among others), permanently applying each `StreamWrite` into
        // `memory_terminal_output`/`memory_log_output` and removing it from
        // the replay-visible list this function otherwise reads below. A run
        // with a commit boundary -- any case that ships a page -- would
        // otherwise lose every terminal/log byte written before that
        // boundary; job framing's banner and `**` line are exactly such
        // early bytes, which is what surfaced this for the `log` channel
        // (the `terminal` channel was already reading its own archive
        // before this comment existed).
        let (terminal, log) = captured_printable_text(run);
        let effects = portable_effect_channel(
            run.observations
                .iter()
                .filter_map(tex_observe::portable_effect_observation),
            run.effect_artifacts.iter().cloned(),
        );
        let diagnostics = portable_diagnostic_channel(run);
        let streams = run.complete_job_channel_streams.clone().unwrap_or_else(|| {
            [
                terminal.into_bytes(),
                log.into_bytes(),
                run.dvi.clone(),
                effects,
                diagnostics,
            ]
        });
        Self {
            events: run
                .observations
                .iter()
                .filter(|observation| {
                    !matches!(observation, tex_command::CommandObservation::DiagnosticLifecycle(_))
                })
                .count(),
            status: run.fatal.map_or_else(
                || "clean".to_owned(),
                |fatal| format!("fatal:{}", fatal.label()),
            ),
            streams,
        }
    }

    /// The rendered content of one stream channel.
    #[must_use]
    pub fn stream(&self, channel: StreamChannel) -> &[u8] {
        let index = STREAM_CHANNELS
            .iter()
            .position(|candidate| *candidate == channel)
            .expect("STREAM_CHANNELS covers every StreamChannel");
        &self.streams[index]
    }
}

/// Encodes the source-located schema-v4 diagnostic lifecycle for one run.
/// A spotless run has no diagnostic channel; once a report exists the final
/// outcome is retained as the closing record.
#[must_use]
pub fn portable_diagnostic_channel(run: &SemanticRun) -> Vec<u8> {
    let root_id = run.observations.iter().find_map(|observation| match observation {
        tex_command::CommandObservation::Command(record) => {
            record.provenance.source_location.map(tex_command::SourceLocation::source)
        }
        tex_command::CommandObservation::DiagnosticLifecycle(
            tex_command::DiagnosticLifecycleRecord::Report { location, .. },
        ) => Some(location.source()),
        _ => None,
    });
    let Some(root_id) = root_id else {
        return Vec::new();
    };
    let mut translator = tex_observe::LiveSessionTranslator::for_root(
        SchemaVersion::V4,
        "terminal",
        tex_observe::LiveSource {
            name: run.diagnostic_root_name.clone(),
            source: root_id,
            bytes: Arc::clone(&run.diagnostic_root_bytes),
        },
    );
    translator.translate_captured(run.observations.iter().cloned());
    let lifecycle: Vec<_> = translator
        .into_events()
        .into_iter()
        .filter_map(|observed| match observed.event {
            Event::DiagnosticLifecycle(event) => Some(Event::DiagnosticLifecycle(event)),
            _ => None,
        })
        .collect();
    if !lifecycle.iter().any(|event| {
        matches!(
            event,
            Event::DiagnosticLifecycle(tex_oracle::DiagnosticLifecycleEvent::Report { .. })
        )
    }) {
        return Vec::new();
    }
    let mut observer = JsonLinesObserver::new_for_schema(
        Vec::new(),
        SchemaVersion::V4,
        ManifestIdentity::from_bytes([0; 32]),
    )
    .expect("in-memory diagnostic stream header");
    for event in lifecycle {
        observer.committed(event).expect("in-memory diagnostic event");
    }
    observer.finish().expect("in-memory diagnostic stream").0
}

/// Captures the terminal and transcript projections from their exact TeX82
/// sinks, including both already-committed and pending writes.
///
/// TeX82 §54 makes `term_only`, `log_only`, and `term_and_log` distinct
/// selectors. Keeping this routing in one helper prevents a concise terminal
/// projection from accidentally treating transcript-only text as terminal
/// evidence while preserving the independent log channel.
pub(crate) fn captured_printable_text(run: &SemanticRun) -> (String, String) {
    let mut terminal = String::from_utf8_lossy(&run.terminal).into_owned();
    let mut log = String::from_utf8_lossy(&run.log).into_owned();
    for effect in &run.pending_effects {
        match effect {
            EffectRecord::StreamWrite { sink, text } => match sink {
                PrintSink::Terminal => terminal.push_str(text),
                PrintSink::Log => log.push_str(text),
                PrintSink::TerminalAndLog => {
                    terminal.push_str(text);
                    log.push_str(text);
                }
                PrintSink::Stream(_) => {}
            },
            EffectRecord::StreamWriteBytes { sink, bytes } => {
                let text = String::from_utf8_lossy(bytes);
                match sink {
                    PrintSink::Terminal => terminal.push_str(&text),
                    PrintSink::Log => log.push_str(&text),
                    PrintSink::TerminalAndLog => {
                        terminal.push_str(&text);
                        log.push_str(&text);
                    }
                    PrintSink::Stream(_) => {}
                }
            }
            EffectRecord::StreamOpen { .. }
            | EffectRecord::StreamClose { .. }
            | EffectRecord::DeferredWrite { .. }
            | EffectRecord::Special { .. }
            | EffectRecord::PdfObjectPlaceholder { .. }
            | EffectRecord::ShellEscape(_) => {}
        }
    }
    (terminal, log)
}

/// One materialized file produced by TeX's numbered write streams.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectArtifact {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Serialize)]
#[serde(tag = "record", rename_all = "kebab-case")]
enum PortableEffectRecord<'a> {
    Effect { effect: &'a EffectEvent },
    Artifact { path: &'a str, bytes: &'a [u8] },
}

/// Renders the oracle-comparable structured-effects contract.
///
/// TeX82 §§1370 and 1373--1375 make open, write, and close ordered semantic
/// transitions. The instrumented reference engine and Umber both publish
/// those transitions as [`EffectEvent`] values, so this projection retains
/// only those three kinds and serializes the shared schema as canonical
/// one-record-per-line JSON. Materialized output files follow in sorted path
/// order with their exact bytes. Message, shipout, and termination effects are
/// owned by the terminal/log, DVI, and status channels respectively; specials
/// are owned by DVI. No replay-internal [`EffectRecord`] rendering enters this
/// contract.
#[must_use]
pub fn portable_effect_channel(
    effects: impl IntoIterator<Item = EffectEvent>,
    artifacts: impl IntoIterator<Item = EffectArtifact>,
) -> Vec<u8> {
    let mut output = Vec::new();
    for effect in effects.into_iter().filter(|effect| {
        matches!(
            effect.kind,
            EffectKind::Open | EffectKind::Write | EffectKind::Close
        )
    }) {
        serde_json::to_writer(
            &mut output,
            &PortableEffectRecord::Effect { effect: &effect },
        )
        .expect("portable effect records always serialize");
        output.push(b'\n');
    }
    let mut artifacts: Vec<_> = artifacts.into_iter().collect();
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    for artifact in &artifacts {
        serde_json::to_writer(
            &mut output,
            &PortableEffectRecord::Artifact {
                path: &artifact.path,
                bytes: &artifact.bytes,
            },
        )
        .expect("portable effect artifacts always serialize");
        output.push(b'\n');
    }
    output
}

/// What a case declares about one stream channel.
///
/// Every committed channel file holds the pinned reference engine's bytes --
/// that is the one meaning a committed file has, for `file` and `xfail`
/// alike (`umber2-alfh.7`). There is therefore exactly one place bytes can
/// have come from, which is why this type carries no `authority` field: a
/// field that can only ever hold one value distinguishes nothing, and a
/// second source (an unadjudicated implementation-observed baseline, this
/// corpus's own prior authority value until `umber2-alfh.1`) is now
/// impossible to declare rather than merely unused.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StreamDisposition {
    /// The channel must produce nothing at all.
    Empty,
    /// The channel must match the committed reference-engine file byte for
    /// byte.
    File,
    /// No portable reference projection exists for this channel in this
    /// case. This is allowed only for `effects`, must carry a reviewed reason,
    /// and commits no expected bytes. It is an explicit absence of a verdict,
    /// never a baseline derived from Umber's output.
    Unsupported { reason: String },
    /// The committed file always holds the reference engine's bytes for this
    /// channel -- that is the one meaning a committed channel file has, `file`
    /// or `xfail` alike. This disposition instead says Umber does not yet
    /// produce those bytes, and pins exactly how: `mismatch` names the first
    /// line at which Umber's output diverges from the committed reference, so
    /// a change in *what* diverges cannot be mistaken for the pinned bug.
    /// `bug` names the issue that will retire this disposition.
    Xfail {
        bug: String,
        mismatch: ChannelMismatch,
    },
    /// Umber's diagnostics do not yet match the reference engine's, but
    /// everything the channel prints *outside* a diagnostic does.
    ///
    /// `xfail` writes a whole channel off and checks only that its first
    /// divergence is still the pinned one, so nothing after that line is
    /// compared at all and every improvement to a report churns the pin. This
    /// disposition keeps comparing the channel with tex.web §82's error
    /// reports removed from both sides (see [`strip_diagnostic_reports`]), so
    /// the file framing, page output, and job tail a divergent diagnostic
    /// used to hide stay under test. `bug` names the issue that will retire
    /// it; the channel matching the reference *raw* is an xpass exactly as it
    /// is for `xfail`.
    XfailDiagnostics { bug: String },
}

/// Removes tex.web §82's error reports from one text channel's lines.
///
/// A report is recognizable without knowing which error raised it, because
/// §82 gives every one of them the same frame:
///
/// - `print_err(s)` opens it with `print_nl("! ")`. The one thing that prints
///   ahead of that is §306's `runaway`: `print_nl("Runaway ")`, the scanner
///   status's name, `"?"`, `print_ln`, and then one line holding the partial
///   token list it was collecting.
/// - `error` then prints `show_context`'s levels and §90's help lines and
///   closes with its own `print_ln`, so the first empty line at or after the
///   `!␣` line ends the report. No empty line can occur inside one: a context
///   level's second line is padding spaces rather than nothing, and no help
///   line is empty.
///
/// §83's `error_stop_mode` arm is deliberately *not* modelled. It returns
/// from `error` at `prompt_input("? ")` having printed neither help nor the
/// closing blank line, and on the terminal `term_input`'s `term_offset:=0`
/// puts whatever prints next on the same physical line as the `? ` -- so
/// there is no line-level boundary to cut on. A channel whose reports end
/// that way keeps the `xfail` disposition instead.
#[must_use]
pub fn strip_diagnostic_reports<'a>(lines: &[&'a [u8]]) -> Vec<&'a [u8]> {
    let mut kept = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if is_runaway_heading(line) {
            // §306 prints the heading, then one line of partial token list.
            index += 2;
            continue;
        }
        if line.starts_with(b"! ") {
            index += 1;
            while index < lines.len() && !lines[index].is_empty() {
                index += 1;
            }
            // The empty line is `error`'s own closing `print_ln`.
            index += 1;
            continue;
        }
        kept.push(line);
        index += 1;
    }
    kept
}

/// §306's `print_nl("Runaway "); print_esc(...)`, whose four scanner statuses
/// (§305's `defining`, `matching`, `aligning`, `absorbing`) name the four
/// headings it can print.
fn is_runaway_heading(line: &[u8]) -> bool {
    matches!(
        line,
        b"Runaway definition?" | b"Runaway argument?" | b"Runaway preamble?" | b"Runaway text?"
    )
}

/// Where an `xfail` channel's observed divergence from its committed
/// reference bytes was first pinned.
///
/// `expected` and `actual` are the two sides' rendering of that one line,
/// using the literal `<end of channel>` for a side that ran out first --
/// exactly as a `file` channel's own line-difference report does.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChannelMismatch {
    /// 1-based line number of the first divergence.
    pub line: usize,
    /// The committed reference engine's line at that point.
    pub expected: String,
    /// Umber's observed line at that point.
    pub actual: String,
}

/// Every channel disposition one case declares.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChannelContract {
    /// Exact number of committed observations the run must produce.
    pub events: usize,
    /// Exact terminal status: `clean`, or `fatal:<label>`.
    pub status: String,
    pub terminal: StreamDisposition,
    pub log: StreamDisposition,
    pub dvi: StreamDisposition,
    pub effects: StreamDisposition,
    pub diagnostics: StreamDisposition,
}

impl ChannelContract {
    /// The declared disposition for one stream channel.
    #[must_use]
    pub fn stream(&self, channel: StreamChannel) -> &StreamDisposition {
        match channel {
            StreamChannel::Terminal => &self.terminal,
            StreamChannel::Log => &self.log,
            StreamChannel::Dvi => &self.dvi,
            StreamChannel::Effects => &self.effects,
            StreamChannel::Diagnostics => &self.diagnostics,
        }
    }
}

/// Validates one channel's `xfail` disposition in isolation from the
/// filesystem: `bug` must be a concrete Beads id, and `mismatch` must
/// actually pin a divergence -- an `expected` equal to `actual` pins nothing.
pub fn validate_xfail_disposition(
    channel: StreamChannel,
    bug: &str,
    mismatch: &ChannelMismatch,
) -> Result<(), String> {
    if !valid_bug_id(bug) {
        return Err(format!(
            "channel {} pins malformed bug {bug:?}",
            channel.name()
        ));
    }
    if mismatch.expected == mismatch.actual {
        return Err(format!(
            "channel {} mismatch has equal expected and actual, which pins nothing",
            channel.name()
        ));
    }
    Ok(())
}

/// Validates one channel's `xfail-diagnostics` disposition: `bug` must be a
/// concrete Beads id, and the channel must be one whose bytes are a tex.web
/// print stream at all. `dvi` and `effects` hold no §82 error reports, so
/// declaring the disposition on either would silently mean "compare
/// normally" and hide a real divergence behind a bug id.
pub fn validate_xfail_diagnostics_disposition(
    channel: StreamChannel,
    bug: &str,
) -> Result<(), String> {
    if !valid_bug_id(bug) {
        return Err(format!(
            "channel {} pins malformed bug {bug:?}",
            channel.name()
        ));
    }
    if !matches!(channel, StreamChannel::Terminal | StreamChannel::Log) {
        return Err(format!(
            "channel {} carries no diagnostics, so xfail-diagnostics would compare it normally",
            channel.name()
        ));
    }
    Ok(())
}

/// One way a run failed to match its declared channel contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChannelFailure {
    /// The observation count moved.
    EventCount { declared: usize, observed: usize },
    /// The terminal status moved.
    Status { declared: String, observed: String },
    /// A channel declared `empty` produced output.
    NotEmpty { channel: &'static str, bytes: usize },
    /// A channel declared `file` or `xfail` had no committed reference file.
    MissingFile { channel: &'static str, path: String },
    /// A channel declared `file` diverged from its committed reference.
    Content {
        channel: &'static str,
        line: usize,
        declared: String,
        observed: String,
    },
    /// An `xfail` channel now byte-matches its committed reference exactly.
    /// The pin describes a divergence, so this is a failure and not a quiet
    /// improvement: the author must promote the channel to `file` and close
    /// `bug`, or the fix stays unrecorded and can regress unnoticed.
    Xpass { channel: &'static str, bug: String },
    /// An `xfail` channel still diverges from its committed reference, but
    /// not the way `pinned` says: a different line, or the same line with
    /// different text. Reports both so the failure shows what was pinned
    /// alongside what Umber now actually does.
    ChangedFailure {
        channel: &'static str,
        bug: String,
        pinned: ChannelMismatch,
        observed: ChannelMismatch,
    },
    /// An `xfail-diagnostics` channel diverged somewhere that is not a
    /// tex.web §82 error report, which is the part of the channel that
    /// disposition still holds to the reference engine. Line numbers are
    /// those of the filtered text, not of the committed file.
    DiagnosticsAside {
        channel: &'static str,
        bug: String,
        line: usize,
        declared: String,
        observed: String,
    },
    /// A channel's bytes could not be brought into comparable form, so no
    /// verdict about them is available. Only the `dvi` channel can reach
    /// this: its normalization has to locate the preamble comment, and a
    /// non-empty artifact with no valid preamble is corrupt rather than
    /// divergent. Reported instead of comparing raw bytes, because falling
    /// back to a raw comparison would turn a corrupt artifact into an
    /// ordinary-looking content divergence.
    Unnormalizable {
        channel: &'static str,
        side: &'static str,
        detail: String,
    },
}

/// Compares one run against its declared channel contract.
///
/// Reports every channel that diverged rather than the first, so one run
/// answers for the whole contract.
#[must_use]
pub fn compare(
    captured: &CapturedChannels,
    contract: &ChannelContract,
    committed: &dyn Fn(StreamChannel) -> Option<Vec<u8>>,
) -> Vec<ChannelFailure> {
    let mut failures = Vec::new();
    if captured.events != contract.events {
        failures.push(ChannelFailure::EventCount {
            declared: contract.events,
            observed: captured.events,
        });
    }
    if captured.status != contract.status {
        failures.push(ChannelFailure::Status {
            declared: contract.status.clone(),
            observed: captured.status.clone(),
        });
    }
    for channel in STREAM_CHANNELS {
        let observed = captured.stream(channel);
        let name = channel.name();
        // Every comparison below is decided on `normalize_channel`'s output,
        // applied symmetrically to the committed reference and to Umber's own
        // capture, rather than baked into either side's stored bytes alone.
        // The regeneration tool calls the same function for the same reason.
        match contract.stream(channel) {
            StreamDisposition::Empty => {
                if !observed.is_empty() {
                    failures.push(ChannelFailure::NotEmpty {
                        channel: name,
                        bytes: observed.len(),
                    });
                }
            }
            StreamDisposition::File => {
                let Some(declared) = committed(channel) else {
                    failures.push(ChannelFailure::MissingFile {
                        channel: name,
                        path: format!("expected.{name}"),
                    });
                    continue;
                };
                let (Some(declared), Some(observed)) = (
                    normalize_side(channel, &declared, "committed", &mut failures),
                    normalize_side(channel, observed, "observed", &mut failures),
                ) else {
                    continue;
                };
                if let Some(divergence) = first_line_difference(&declared, &observed) {
                    failures.push(ChannelFailure::Content {
                        channel: name,
                        line: divergence.line,
                        declared: divergence.expected,
                        observed: divergence.actual,
                    });
                }
            }
            StreamDisposition::Unsupported { .. } => {}
            // The committed file always holds the reference engine's bytes,
            // exactly as a `file` channel's does. `mismatch` pins where
            // Umber's own output first diverges from those bytes, so the
            // three outcomes mirror `evaluate_expectation`'s own projection
            // discipline: an unchanged divergence passes quietly, a
            // divergence that vanished is an xpass (Umber owes a promotion to
            // `file` and the bug owes closing), and a divergence that moved is
            // a changed failure reporting pinned against observed.
            StreamDisposition::Xfail { bug, mismatch } => {
                let Some(reference) = committed(channel) else {
                    failures.push(ChannelFailure::MissingFile {
                        channel: name,
                        path: format!("expected.{name}"),
                    });
                    continue;
                };
                let (Some(reference), Some(observed)) = (
                    normalize_side(channel, &reference, "committed", &mut failures),
                    normalize_side(channel, observed, "observed", &mut failures),
                ) else {
                    continue;
                };
                match first_line_difference(&reference, &observed) {
                    None => failures.push(ChannelFailure::Xpass {
                        channel: name,
                        bug: bug.clone(),
                    }),
                    Some(divergence) if divergence == *mismatch => {}
                    Some(divergence) => failures.push(ChannelFailure::ChangedFailure {
                        channel: name,
                        bug: bug.clone(),
                        pinned: mismatch.clone(),
                        observed: divergence,
                    }),
                }
            }
            // Everything outside a §82 error report is still held to the
            // reference engine byte for byte; the reports themselves are cut
            // out of both sides. A channel that matches without the cut has
            // nothing left for `bug` to describe and owes a promotion to
            // `file`, exactly as an `xfail` channel does.
            StreamDisposition::XfailDiagnostics { bug } => {
                let Some(reference) = committed(channel) else {
                    failures.push(ChannelFailure::MissingFile {
                        channel: name,
                        path: format!("expected.{name}"),
                    });
                    continue;
                };
                let (Some(reference), Some(observed)) = (
                    normalize_side(channel, &reference, "committed", &mut failures),
                    normalize_side(channel, observed, "observed", &mut failures),
                ) else {
                    continue;
                };
                if reference == observed {
                    failures.push(ChannelFailure::Xpass {
                        channel: name,
                        bug: bug.clone(),
                    });
                    continue;
                }
                let reference = strip_diagnostic_reports(&split_channel_lines(&reference));
                let observed = strip_diagnostic_reports(&split_channel_lines(&observed));
                if let Some(divergence) = first_line_difference_in(&reference, &observed) {
                    failures.push(ChannelFailure::DiagnosticsAside {
                        channel: name,
                        bug: bug.clone(),
                        line: divergence.line,
                        declared: divergence.expected,
                        observed: divergence.actual,
                    });
                }
            }
        }
    }
    failures
}

/// Splits a channel's raw bytes into lines the way `str::lines` splits text:
/// on `\n`, with a trailing `\r` dropped from each line and no empty final
/// line for a trailing `\n`.
///
/// Operating on bytes rather than `str` is what lets this serve the binary
/// `dvi` channel and the text channels alike without lossily reencoding
/// either: a text channel's bytes are valid UTF-8 by construction, and a
/// binary channel's bytes are compared exactly as recorded, with only the
/// *rendering* of a divergent line (below) falling back to a lossy decode
/// for a human-readable report.
pub fn split_channel_lines(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let trimmed = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    trimmed
        .split(|&byte| byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .collect()
}

/// tex.web §1328's dump date, inside the log's `(preloaded format=NAME
/// YYYY.M.D)`.
///
/// `store_fmt_file` stamps the dumped `format_ident` with `\year`/`\month`/
/// `\day` as they stood *when the format was dumped*, and §536's
/// `slow_print(format_ident)` reproduces it on the log's first line. For this
/// corpus that is the day `scripts/run-minifixture-oracle.sh` happened to
/// build the format, so it is a second wall clock -- no more reproducible
/// than §536's own, and equally not a fact about typesetting. The terminal
/// never shows it (web2c prints `dump_name` there instead, with no date), so
/// only the log needs this.
///
/// Idempotent: the replacement holds no digits, so a second pass finds no
/// date to rewrite.
fn normalize_dump_date(bytes: &[u8]) -> Vec<u8> {
    const MARKER: &[u8] = b" (preloaded format=";
    let Some(start) = bytes
        .windows(MARKER.len())
        .position(|window| window == MARKER)
    else {
        return bytes.to_vec();
    };
    let open = start + MARKER.len();
    let Some(close) = bytes[open..].iter().position(|&byte| byte == b')') else {
        return bytes.to_vec();
    };
    let inner = &bytes[open..open + close];
    // `NAME YYYY.M.D` -- split at the last space so a format name containing
    // one is still handled, and leave a dateless `(preloaded format=NAME)`
    // (what the terminal prints) untouched.
    let Some(space) = inner.iter().rposition(|&byte| byte == b' ') else {
        return bytes.to_vec();
    };
    if !inner[space + 1..]
        .iter()
        .all(|byte| byte.is_ascii_digit() || *byte == b'.')
        || inner[space + 1..].is_empty()
    {
        return bytes.to_vec();
    }
    let mut normalized = Vec::with_capacity(bytes.len());
    normalized.extend_from_slice(&bytes[..open + space + 1]);
    normalized.extend_from_slice(b"<DUMP-DATE>");
    normalized.extend_from_slice(&bytes[open + close..]);
    normalized
}

/// Brings one channel's bytes into the form every comparison in this corpus
/// is decided on, for the committed reference and Umber's own capture alike.
///
/// This exists because the choice of what to normalize is not local to one
/// caller. Both the ongoing gate (`compare` below) and the regeneration tool
/// (`command-semantic-channels`) have to agree byte for byte, or a channel
/// the tool writes as `file` fails under the gate that reads it back. They
/// used to spell the rule out separately, and the two copies had already
/// drifted: each normalized the log clock and nothing else, so the `dvi`
/// channel's preamble comment -- a timestamp and engine-identity string that
/// `test-support` has always normalized for the byte-exact DVI parity
/// harness -- was compared raw here, and pinned 66 cases as `xfail` for
/// differing in exactly the bytes the rest of the repository had already
/// ruled uncomparable (`umber2-alfh.22`). One function, two callers.
///
/// Both normalizations are idempotent, which is what lets a caller apply
/// this to a committed file (normalized when it was written) and a fresh
/// capture (never normalized) with the same call.
pub fn normalize_channel(channel: StreamChannel, bytes: &[u8]) -> Result<Vec<u8>, String> {
    match channel {
        StreamChannel::Log => Ok(normalize_log_clock(bytes)),
        // A case that ships no page has no preamble to normalize, and that
        // is an ordinary observation rather than a malformed artifact: an
        // empty capture against a committed reference stays a divergence,
        // reported below as content rather than as corruption.
        StreamChannel::Dvi if bytes.is_empty() => Ok(Vec::new()),
        StreamChannel::Dvi => test_support::dvi::normalized_dvi_for_comparison(bytes)
            .map_err(|error| format!("{error:#}")),
        StreamChannel::Terminal | StreamChannel::Effects | StreamChannel::Diagnostics => {
            Ok(bytes.to_vec())
        }
    }
}

/// Normalizes one side of a comparison, recording a failure instead of
/// returning bytes when the channel's own artifact is malformed.
fn normalize_side(
    channel: StreamChannel,
    bytes: &[u8],
    side: &'static str,
    failures: &mut Vec<ChannelFailure>,
) -> Option<Vec<u8>> {
    match normalize_channel(channel, bytes) {
        Ok(normalized) => Some(normalized),
        Err(detail) => {
            failures.push(ChannelFailure::Unnormalizable {
                channel: channel.name(),
                side,
                detail,
            });
            None
        }
    }
}

/// tex.web section 536's clock suffix on the log channel's very first line --
/// the one byte range no two runs of the same job can ever agree on, and one
/// of the two normalizations this corpus applies (`docs/job_framing.md`'s
/// "Why the notices are configuration, not output" section; the other is the
/// `dvi` preamble comment, see [`normalize_channel`]). Replaces everything from
/// the first `)  ` (a closing paren immediately followed by section 536's
/// fixed two-space separator -- `format_ident`'s own closing paren, e.g.
/// `(INITEX)` or `(preloaded format=…)`, followed by that separator and the
/// clock) through the end of the first line with `) <HOST-CLOCK>`.
///
/// A first line with no such marker is returned unchanged, and so is a
/// buffer whose first line has already been normalized: the normalized form
/// has one space before `<HOST-CLOCK>`, not two, so it contains no `)  `
/// marker for a second pass to find. That idempotence is what lets `compare`
/// below apply this to both the committed reference (already normalized when
/// it was written) and Umber's own freshly captured bytes (never
/// normalized) with the same call.
#[must_use]
pub fn normalize_log_clock(bytes: &[u8]) -> Vec<u8> {
    let bytes = normalize_dump_date(bytes);
    let bytes = bytes.as_slice();
    let first_newline = bytes.iter().position(|&byte| byte == b'\n');
    let (first_line, rest) = match first_newline {
        Some(index) => (&bytes[..index], &bytes[index..]),
        None => (bytes, &b""[..]),
    };
    const MARKER: &[u8] = b")  ";
    let Some(offset) = first_line
        .windows(MARKER.len())
        .position(|window| window == MARKER)
    else {
        return bytes.to_vec();
    };
    let mut normalized = Vec::with_capacity(bytes.len());
    normalized.extend_from_slice(&first_line[..=offset]);
    normalized.extend_from_slice(b" <HOST-CLOCK>");
    normalized.extend_from_slice(rest);
    normalized
}

/// Locates the first line at which two channel renderings differ, so a
/// failure names what moved rather than printing two transcripts. `None`
/// means the two sides match exactly.
///
/// Equality is decided on the raw bytes (`declared == observed` below, and
/// each line compared as `&[u8]`), so a divergence cannot be masked by lossy
/// UTF-8 replacement; only the *report* -- `ChannelMismatch`'s `expected`/
/// `actual`, which exist to be read -- renders a differing line with
/// [`String::from_utf8_lossy`].
#[must_use]
pub fn first_line_difference(declared: &[u8], observed: &[u8]) -> Option<ChannelMismatch> {
    if declared == observed {
        return None;
    }
    first_line_difference_in(
        &split_channel_lines(declared),
        &split_channel_lines(observed),
    )
}

/// [`first_line_difference`] over lines a caller has already split, so a
/// filtered comparison ([`strip_diagnostic_reports`]) reports the line
/// numbers of what it actually compared.
#[must_use]
pub fn first_line_difference_in(declared: &[&[u8]], observed: &[&[u8]]) -> Option<ChannelMismatch> {
    let mut declared_lines = declared.iter().copied();
    let mut observed_lines = observed.iter().copied();
    let mut line = 0;
    loop {
        line += 1;
        match (declared_lines.next(), observed_lines.next()) {
            (None, None) => return None,
            (declared_line, observed_line) => {
                if declared_line != observed_line {
                    let render = |line: Option<&[u8]>| {
                        line.map_or_else(
                            || "<end of channel>".to_owned(),
                            |bytes| String::from_utf8_lossy(bytes).into_owned(),
                        )
                    };
                    return Some(ChannelMismatch {
                        line,
                        expected: render(declared_line),
                        actual: render(observed_line),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
