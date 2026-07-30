//! Every observable a minifixture run produces, and the contract that each one
//! must be accounted for.
//!
//! A `projection` selects one observable and asserts a handful of strings about
//! it. That is a focused property claim, and it stays. What it is not is
//! coverage: the same run also writes a terminal transcript, a log, shipped
//! pages, and ordinary effects, and before this module nothing compared any of
//! them. Measured across the committed corpus, 130 cases produced 33,112
//! events, 23,013 bytes of terminal and log text, and 26 shipped pages against
//! 698 declared assertion strings -- and the log channel was read by no
//! projection that exists.
//!
//! So a case declares a disposition for *every* channel here. A channel with
//! no disposition fails validation rather than passing quietly, for the same
//! reason `default-members` naming 21 of 34 crates was a defect rather than a
//! configuration: an omission that reads as coverage is worse than a red gate.

use std::fmt::Write as _;

use serde::Deserialize;
use tex_state::{EffectRecord, PrintSink};

use super::{SemanticRun, valid_bug_id};

/// The stream channels, in the order a report prints them.
///
/// `events` and `status` are scalars rather than streams and are declared
/// inline in the manifest, so they are not part of this list.
pub const STREAM_CHANNELS: [StreamChannel; 4] = [
    StreamChannel::Terminal,
    StreamChannel::Log,
    StreamChannel::Dvi,
    StreamChannel::Effects,
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
    pub streams: [Vec<u8>; 4],
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
        let mut terminal = run
            .universe
            .world()
            .memory_terminal_output()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        let mut log = run
            .universe
            .world()
            .memory_log_output()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        let mut effects = String::new();
        for effect in run.universe.world().effect_records() {
            match effect {
                EffectRecord::StreamWrite { sink, text } => match sink {
                    PrintSink::Terminal => terminal.push_str(text),
                    PrintSink::Log => log.push_str(text),
                    PrintSink::TerminalAndLog => {
                        terminal.push_str(text);
                        log.push_str(text);
                    }
                    other => {
                        let _ = writeln!(effects, "write:{other:?}:{}", rendered(text));
                    }
                },
                EffectRecord::StreamOpen { slot, target } => {
                    let _ = writeln!(effects, "open:{slot:?}:{target:?}");
                }
                EffectRecord::StreamClose { slot } => {
                    let _ = writeln!(effects, "close:{slot:?}");
                }
                EffectRecord::DeferredWrite { stream, tokens } => {
                    let _ = writeln!(effects, "deferred-write:{stream:?}:{tokens:?}");
                }
                EffectRecord::Special { class, payload } => {
                    let _ = writeln!(
                        effects,
                        "special:{class}:{}",
                        rendered(&String::from_utf8_lossy(payload))
                    );
                }
                EffectRecord::PdfObjectPlaceholder { label } => {
                    let _ = writeln!(effects, "pdf-object:{label}");
                }
                EffectRecord::ShellEscape(record) => {
                    let _ = writeln!(effects, "shell-escape:{record:?}");
                }
            }
        }
        Self {
            events: run.observations.len(),
            status: run.fatal.map_or_else(
                || "clean".to_owned(),
                |fatal| format!("fatal:{}", fatal.label()),
            ),
            streams: [
                terminal.into_bytes(),
                log.into_bytes(),
                run.dvi.clone(),
                effects.into_bytes(),
            ],
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

/// Renders control characters so a committed expectation stays one line per
/// record and survives review in a terminal.
fn rendered(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' => "\\n".to_owned(),
            '\r' => "\\r".to_owned(),
            '\t' => "\\t".to_owned(),
            other => other.to_string(),
        })
        .collect()
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StreamDisposition {
    /// The channel must produce nothing at all.
    Empty,
    /// The channel must match the committed reference-engine file byte for
    /// byte.
    File,
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
}

/// Where an `xfail` channel's observed divergence from its committed
/// reference bytes was first pinned.
///
/// `expected` and `actual` are the two sides' rendering of that one line,
/// using the literal `<end of channel>` for a side that ran out first --
/// exactly as a `file` channel's own line-difference report does.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
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
        // The log channel's first line carries tex.web section 536's clock,
        // which no two runs of the same job can ever agree on byte for byte.
        // `normalize_log_clock` is the one normalization this corpus applies
        // (`docs/job_framing.md`), so it is applied here, symmetrically, to
        // both sides of every log comparison below rather than baked into
        // either side's stored bytes alone.
        let normalize = |bytes: &[u8]| -> Vec<u8> {
            if channel == StreamChannel::Log {
                normalize_log_clock(bytes)
            } else {
                bytes.to_vec()
            }
        };
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
                if let Some(divergence) =
                    first_line_difference(&normalize(&declared), &normalize(observed))
                {
                    failures.push(ChannelFailure::Content {
                        channel: name,
                        line: divergence.line,
                        declared: divergence.expected,
                        observed: divergence.actual,
                    });
                }
            }
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
                match first_line_difference(&normalize(&reference), &normalize(observed)) {
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
fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let trimmed = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    trimmed
        .split(|&byte| byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .collect()
}

/// tex.web section 536's clock suffix on the log channel's very first line --
/// the one byte range no two runs of the same job can ever agree on, and the
/// only normalization this corpus applies (`docs/job_framing.md`'s "Why the
/// notices are configuration, not output" section). Replaces everything from
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
    let declared_lines = split_lines(declared);
    let observed_lines = split_lines(observed);
    let mut declared_lines = declared_lines.into_iter();
    let mut observed_lines = observed_lines.into_iter();
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
