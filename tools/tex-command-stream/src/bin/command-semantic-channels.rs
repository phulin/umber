//! Derives the per-channel contract for every committed minifixture against
//! the pinned instrumented pdfTeX 1.40.27 oracle.
//!
//! This is the regeneration path for the `command-semantic` corpus, invoked
//! by `scripts/regen-fixtures.sh --area command-semantic`. It runs exactly
//! the code the gate runs -- `tex_command_stream::semantic` -- so a
//! regenerated contract cannot describe a run the gate would not reproduce.
//!
//! Every committed channel file holds the reference engine's bytes, for
//! `file` and `xfail` alike (`umber2-alfh.7`), so this tool reads them from
//! `scripts/run-minifixture-oracle.sh`'s captures under
//! `target/minifixture-oracle/<domain>/<case>/` rather than from Umber's own
//! run. Umber's run decides only the *disposition*: `file` when it matches
//! the oracle bytes exactly, `xfail` with a first-line-divergence
//! fingerprint when it does not. `scripts/run-minifixture-oracle.sh --all`
//! must already have populated that capture tree -- this tool never invokes
//! the live oracle itself -- and a missing or stale capture fails the whole
//! run rather than silently regenerating that one case against nothing, per
//! `docs/testing_policy.md`'s rule that a skipped case must never read as a
//! pass.
//!
//! An `xfail` channel's `bug` id is never invented here: it is read from
//! whatever the case's manifest already declares for that channel (exactly
//! as the projection layer's own `expectation` xfail is hand-authored) and
//! carried forward. A channel that newly diverges from the oracle and has no
//! existing bug to preserve fails the run with the divergence printed, so a
//! human decides which issue it belongs to before the corpus can commit it.
//!
//! The `effects` channel has no oracle-comparable source at all: it is
//! Umber's own structured rendering of stream opens/closes/writes and shell
//! escapes (`open:`/`close:`/`write:`/... lines), not a reproduction of
//! anything a real TeX engine writes. Every case's `effects` capture is
//! empty today, so this tool requires that and fails loudly if it ever stops
//! being true, rather than inventing an authority for content with no
//! reference to check it against.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only regeneration entry point reads its oracle capture and rewrites its committed corpus"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tex_command_stream::semantic::{
    CapturedChannels, ChannelMismatch, ChannelPolicy, DeclaredCase, STREAM_CHANNELS, StreamChannel,
    StreamDisposition, channel_file, execute, first_line_difference, load_suite_with,
    normalize_log_clock, repository_root,
};

fn main() -> ExitCode {
    match run() {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("command-semantic-channels: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `target/minifixture-oracle`, resolved the same way
/// `scripts/run-minifixture-oracle.sh` resolves its own output root: under
/// `CARGO_TARGET_DIR` when set, else under this checkout's `target/`.
fn oracle_root() -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || repository_root().join("target"),
        |value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                repository_root().join(path)
            }
        },
    );
    target_dir.join("minifixture-oracle")
}

fn run() -> Result<String, String> {
    let oracle_root = oracle_root();
    let cases = load_suite_with(ChannelPolicy::Deriving)?;

    let mut plans: BTreeMap<String, BTreeMap<String, CasePlan>> = BTreeMap::new();
    let mut unrunnable = Vec::new();
    let mut errors = Vec::new();

    for declared in &cases {
        let label = format!("{}/{}", declared.domain, declared.case.id);
        let source = fs::read(declared.domain_dir.join(&declared.case.source))
            .map_err(|error| format!("{label}: source read: {error}"))?;
        match execute(&source, &declared.case) {
            Ok(completed) => {
                let captured = CapturedChannels::capture(&completed);
                match plan_case(&oracle_root, declared, &source, &captured) {
                    Ok(plan) => {
                        plans
                            .entry(declared.domain.clone())
                            .or_default()
                            .insert(declared.case.id.clone(), plan);
                    }
                    Err(error) => errors.push(format!("{label}: {error}")),
                }
            }
            // A case whose run fails has no channels to record. It is already
            // an `xfail` on its projection; recording an invented contract
            // here would let the failure read as covered.
            Err(error) => unrunnable.push(format!("{label}: {error}")),
        }
    }

    // Nothing is written until every case in the batch resolved cleanly: a
    // missing oracle capture, a stale one, or a newly diverging channel with
    // no bug to preserve all fail the whole run rather than leaving some
    // cases regenerated against a partial plan and others not.
    if !errors.is_empty() {
        return Err(format!(
            "{} case(s) cannot be regenerated against the oracle:\n  {}",
            errors.len(),
            errors.join("\n  ")
        ));
    }

    let mut rewritten = 0;
    for (domain, cases_by_id) in &plans {
        let domain_dir = repository_root()
            .join("tests/corpus/command-semantic")
            .join(domain);
        for (case_id, plan) in cases_by_id {
            write_channel_files(&domain_dir, case_id, plan)?;
        }
        rewrite_manifest(&domain_dir.join("manifest.json"), cases_by_id)?;
        rewritten += 1;
    }

    let mut summary = format!(
        "derived channel contracts for {} cases across {rewritten} domains against the pinned pdfTeX 1.40.27 oracle",
        plans.values().map(BTreeMap::len).sum::<usize>()
    );
    if !unrunnable.is_empty() {
        summary.push_str(&format!(
            "\n{} case(s) do not run and carry no channel contract:\n  {}",
            unrunnable.len(),
            unrunnable.join("\n  ")
        ));
    }
    Ok(summary)
}

/// One channel's resolved disposition: `Empty` needs no committed file;
/// `File`'s bytes are always the oracle's; `Xfail` additionally carries the
/// bug it is pinned against and where Umber's own output first diverges from
/// those same committed bytes.
enum ChannelPlan {
    Empty,
    File {
        bytes: Vec<u8>,
    },
    Xfail {
        bytes: Vec<u8>,
        bug: String,
        mismatch: ChannelMismatch,
    },
}

struct CasePlan {
    events: usize,
    status: String,
    /// In [`STREAM_CHANNELS`] order.
    channels: [ChannelPlan; 4],
}

/// Resolves every stream channel's disposition for one case by comparing
/// Umber's freshly captured bytes against the pinned oracle's capture.
fn plan_case(
    oracle_root: &Path,
    declared: &DeclaredCase,
    source: &[u8],
    captured: &CapturedChannels,
) -> Result<CasePlan, String> {
    let case_dir = oracle_root.join(&declared.domain).join(&declared.case.id);
    if !case_dir.is_dir() {
        return Err(format!(
            "no oracle capture at {} -- run `scripts/run-minifixture-oracle.sh --all` first \
             (build the oracle itself with scripts/build-pdftex14027-oracle.sh if its binary \
             is missing)",
            case_dir.display()
        ));
    }
    // A capture staged against a source that no longer matches the corpus is
    // worse than a missing one: it would silently regenerate this case
    // against a different program than the one actually committed. Compare
    // the exact bytes the oracle ran rather than trusting a timestamp.
    let staged_source = case_dir.join(&declared.case.source);
    let staged_bytes = fs::read(&staged_source)
        .map_err(|error| format!("{}: {error}", staged_source.display()))?;
    if staged_bytes != source {
        return Err(format!(
            "stale oracle capture under {}: its staged source no longer matches the corpus; \
             rerun scripts/run-minifixture-oracle.sh --all",
            case_dir.display()
        ));
    }

    let stem = declared
        .case
        .source
        .strip_suffix(".tex")
        .expect("validate_case requires a .tex source");

    let mut channels = Vec::with_capacity(STREAM_CHANNELS.len());
    for channel in STREAM_CHANNELS {
        channels.push(plan_channel(declared, channel, stem, &case_dir, captured)?);
    }
    let channels: [ChannelPlan; 4] = channels
        .try_into()
        .unwrap_or_else(|_| panic!("STREAM_CHANNELS has exactly 4 entries"));

    Ok(CasePlan {
        events: captured.events,
        status: captured.status.clone(),
        channels,
    })
}

/// Resolves one channel's disposition. `terminal`/`log`/`dvi` compare Umber's
/// captured bytes against the oracle capture at `case_dir`; `effects` has no
/// oracle source and is required to be empty (see this file's module doc).
fn plan_channel(
    declared: &DeclaredCase,
    channel: StreamChannel,
    stem: &str,
    case_dir: &Path,
    captured: &CapturedChannels,
) -> Result<ChannelPlan, String> {
    let raw_umber = captured.stream(channel);

    if channel == StreamChannel::Effects {
        return if raw_umber.is_empty() {
            Ok(ChannelPlan::Empty)
        } else {
            Err(format!(
                "effects channel is {} byte(s) but has no oracle-comparable source; decide how \
                 this channel should be adjudicated before regenerating (see this file's module \
                 doc)",
                raw_umber.len()
            ))
        };
    }

    let oracle_path = match channel {
        StreamChannel::Terminal => case_dir.join("terminal.txt"),
        StreamChannel::Log => case_dir.join(format!("{stem}.log")),
        StreamChannel::Dvi => case_dir.join(format!("{stem}.dvi")),
        StreamChannel::Effects => unreachable!("handled above"),
    };
    let raw_oracle = if oracle_path.exists() {
        fs::read(&oracle_path).map_err(|error| format!("{}: {error}", oracle_path.display()))?
    } else {
        Vec::new()
    };

    // tex.web section 536's clock is the one byte range no two runs of the
    // same job can ever agree on (`docs/job_framing.md`), and it appears only
    // on the log channel's first line. Normalizing both sides the same way
    // here is exactly what the ongoing gate's own `compare` does, so a
    // channel this tool marks `file` stays `file` under the gate that reads
    // it back.
    let normalize = |bytes: &[u8]| -> Vec<u8> {
        if channel == StreamChannel::Log {
            normalize_log_clock(bytes)
        } else {
            bytes.to_vec()
        }
    };
    let oracle_bytes = normalize(&raw_oracle);
    let umber_bytes = normalize(raw_umber);

    if raw_oracle.is_empty() && raw_umber.is_empty() {
        return Ok(ChannelPlan::Empty);
    }

    match first_line_difference(&oracle_bytes, &umber_bytes) {
        None => Ok(ChannelPlan::File {
            bytes: oracle_bytes,
        }),
        Some(mismatch) => {
            let bug = existing_bug(declared, channel).ok_or_else(|| {
                format!(
                    "channel {} newly diverges from the oracle at line {} (oracle: {:?}, umber: \
                     {:?}) and has no existing xfail bug to preserve; hand-author \
                     `channels.{}` in the manifest as `{{\"kind\": \"xfail\", \"bug\": \
                     \"umber2-<epic>.<n>\", \"mismatch\": {{\"line\": 1, \"expected\": \"x\", \
                     \"actual\": \"y\"}}}}` before regenerating",
                    channel.name(),
                    mismatch.line,
                    mismatch.expected,
                    mismatch.actual,
                    channel.name(),
                )
            })?;
            Ok(ChannelPlan::Xfail {
                bytes: oracle_bytes,
                bug,
                mismatch,
            })
        }
    }
}

/// The bug a case's manifest already declares for one channel's `xfail`
/// disposition, if any. Never invented: only ever carried forward from what
/// was already committed, exactly as the projection layer's own
/// `expectation` xfail bug is hand-authored rather than derived.
fn existing_bug(declared: &DeclaredCase, channel: StreamChannel) -> Option<String> {
    let contract = declared.case.channels.as_ref()?;
    match contract.stream(channel) {
        StreamDisposition::Xfail { bug, .. } => Some(bug.clone()),
        StreamDisposition::Empty | StreamDisposition::File => None,
    }
}

fn write_channel_files(domain_dir: &Path, case_id: &str, plan: &CasePlan) -> Result<(), String> {
    for (index, channel) in STREAM_CHANNELS.into_iter().enumerate() {
        let path = channel_file(domain_dir, case_id, channel);
        match &plan.channels[index] {
            ChannelPlan::Empty => {
                if path.exists() {
                    fs::remove_file(&path)
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                }
            }
            ChannelPlan::File { bytes } | ChannelPlan::Xfail { bytes, .. } => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("{}: {error}", parent.display()))?;
                }
                fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
            }
        }
    }
    Ok(())
}

impl CasePlan {
    /// Renders the block close to the shape `dprint` produces so a
    /// regeneration run leaves it little to do; an explicit `dprint fmt`
    /// pass (part of `scripts/regen-fixtures.sh`'s own workflow) settles any
    /// remaining wrapping rather than this function trying to replicate
    /// dprint's line-width heuristics exactly.
    fn json(&self, indent: &str) -> String {
        let inner = format!("{indent}  ");
        let mut fields = vec![
            format!("{inner}\"events\": {}", self.events),
            format!("{inner}\"status\": {}", json_string(&self.status)),
        ];
        for (index, channel) in STREAM_CHANNELS.into_iter().enumerate() {
            let disposition = match &self.channels[index] {
                ChannelPlan::Empty => "{ \"kind\": \"empty\" }".to_owned(),
                ChannelPlan::File { .. } => "{ \"kind\": \"file\" }".to_owned(),
                ChannelPlan::Xfail { bug, mismatch, .. } => format!(
                    "{{ \"kind\": \"xfail\", \"bug\": {}, \"mismatch\": {{ \"line\": {}, \
                     \"expected\": {}, \"actual\": {} }} }}",
                    json_string(bug),
                    mismatch.line,
                    json_string(&mismatch.expected),
                    json_string(&mismatch.actual),
                ),
            };
            fields.push(format!("{inner}\"{}\": {disposition}", channel.name()));
        }
        format!("{{\n{}\n{indent}}}", fields.join(",\n"))
    }
}

/// Inserts each case's derived `channels` block into the manifest immediately
/// *before* its `expectation` key, preserving every other byte of the file.
///
/// A structural rewrite through `serde_json` would reorder keys and drop the
/// hand-authored formatting the corpus is reviewed in, so the edit is textual.
/// It anchors before `expectation` rather than after because an `xfail`
/// expectation spans several lines: inserting before a key needs no knowledge
/// of where its value ends, while inserting after would have to parse it.
fn rewrite_manifest(path: &Path, cases: &BTreeMap<String, CasePlan>) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut output = String::with_capacity(text.len());
    let mut current: Option<&CasePlan> = None;
    let mut applied = 0;

    let mut skipping_channels = 0i32;
    for line in text.lines() {
        let trimmed = line.trim_start();

        // Drop any previously derived block, which the formatter may have
        // expanded across several lines. Counting braces rather than matching
        // one line is what makes the tool re-runnable on its own output.
        if skipping_channels > 0 {
            skipping_channels += braces(line);
            continue;
        }
        if trimmed.starts_with("\"channels\":") {
            skipping_channels = braces(line);
            continue;
        }

        if let Some(id) = trimmed
            .strip_prefix("\"id\": \"")
            .and_then(|rest| rest.split('"').next())
        {
            current = cases.get(id);
        }
        if let Some(plan) = current
            && trimmed.starts_with("\"expectation\":")
        {
            let indent = &line[..line.len() - trimmed.len()];
            output.push_str(&format!("{indent}\"channels\": {},\n", plan.json(indent)));
            applied += 1;
            current = None;
        }
        output.push_str(line);
        output.push('\n');
    }
    if skipping_channels != 0 {
        return Err(format!(
            "{}: an existing \"channels\" block does not close",
            path.display()
        ));
    }

    if applied != cases.len() {
        return Err(format!(
            "{}: rewrote {applied} of {} cases; a case's \"expectation\" anchor is missing",
            path.display(),
            cases.len()
        ));
    }
    fs::write(path, output).map_err(|error| format!("{}: {error}", path.display()))
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

/// Net brace depth a line contributes, ignoring braces inside JSON strings.
fn braces(line: &str) -> i32 {
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for character in line.chars() {
        match character {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => depth -= 1,
            _ => {}
        }
    }
    depth
}
