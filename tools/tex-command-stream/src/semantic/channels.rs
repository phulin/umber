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

use super::SemanticRun;

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
    /// One line per page the run shipped, by committed content identity.
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
    /// Rendered stream channels, in [`STREAM_CHANNELS`] order.
    pub streams: [String; 4],
}

impl CapturedChannels {
    /// Captures every channel from a completed run.
    #[must_use]
    pub fn capture(run: &SemanticRun) -> Self {
        let mut terminal = run
            .universe
            .world()
            .memory_terminal_output()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        let mut log = String::new();
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
        let mut dvi = String::new();
        for (index, page) in run.artifacts.iter().enumerate() {
            let _ = writeln!(dvi, "page:{index}:{}", page.hex());
        }
        Self {
            events: run.observations.len(),
            status: run.fatal.map_or_else(
                || "clean".to_owned(),
                |fatal| format!("fatal:{}", fatal.label()),
            ),
            streams: [terminal, log, dvi, effects],
        }
    }

    /// The rendered content of one stream channel.
    #[must_use]
    pub fn stream(&self, channel: StreamChannel) -> &str {
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StreamDisposition {
    /// The channel must produce nothing at all.
    Empty,
    /// The channel must match the committed file byte for byte.
    ///
    /// `authority` records where those bytes came from. `oracle` means a
    /// reference engine produced them; `umber-baseline` means they are this
    /// implementation's observed output, recorded so it cannot change
    /// silently, and still owed an oracle adjudication.
    File { authority: ChannelAuthority },
    /// The channel is known-divergent. The committed file holds the observed
    /// bytes, and `bug` names the issue that will retire this disposition.
    Xfail {
        bug: String,
        authority: ChannelAuthority,
    },
}

/// Where a committed channel expectation's bytes came from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ChannelAuthority {
    /// A pinned reference engine produced these bytes.
    Oracle,
    /// Umber produced these bytes. They are pinned against silent drift but
    /// are not yet evidence of correctness.
    UmberBaseline,
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

/// One way a run failed to match its declared channel contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChannelFailure {
    /// The observation count moved.
    EventCount { declared: usize, observed: usize },
    /// The terminal status moved.
    Status { declared: String, observed: String },
    /// A channel declared `empty` produced output.
    NotEmpty { channel: &'static str, bytes: usize },
    /// A channel declared `file` had no committed file.
    MissingFile { channel: &'static str, path: String },
    /// A channel's content diverged from its committed file.
    Content {
        channel: &'static str,
        line: usize,
        declared: String,
        observed: String,
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
    committed: &dyn Fn(StreamChannel) -> Option<String>,
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
        match contract.stream(channel) {
            StreamDisposition::Empty => {
                if !observed.is_empty() {
                    failures.push(ChannelFailure::NotEmpty {
                        channel: name,
                        bytes: observed.len(),
                    });
                }
            }
            // `xfail` compares exactly like `file`. The committed bytes are the
            // observed, known-wrong ones, so byte-identity is what pins the
            // divergence; the disposition differs only in what it claims those
            // bytes mean. Without an oracle for this channel a change cannot be
            // told apart from a fix, so the pin is deliberately not an xpass
            // detector -- that adjudication belongs to the bug's own gate.
            StreamDisposition::File { .. } | StreamDisposition::Xfail { .. } => {
                let Some(declared) = committed(channel) else {
                    failures.push(ChannelFailure::MissingFile {
                        channel: name,
                        path: format!("expected/<case>.{name}"),
                    });
                    continue;
                };
                if let Some(failure) = first_line_difference(name, &declared, observed) {
                    failures.push(failure);
                }
            }
        }
    }
    failures
}

/// Locates the first differing line so a failure names what moved rather than
/// printing two transcripts.
fn first_line_difference(
    channel: &'static str,
    declared: &str,
    observed: &str,
) -> Option<ChannelFailure> {
    if declared == observed {
        return None;
    }
    let mut declared_lines = declared.lines();
    let mut observed_lines = observed.lines();
    let mut line = 0;
    loop {
        line += 1;
        match (declared_lines.next(), observed_lines.next()) {
            (None, None) => return None,
            (declared_line, observed_line) => {
                if declared_line != observed_line {
                    return Some(ChannelFailure::Content {
                        channel,
                        line,
                        declared: declared_line.unwrap_or("<end of channel>").to_owned(),
                        observed: observed_line.unwrap_or("<end of channel>").to_owned(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
