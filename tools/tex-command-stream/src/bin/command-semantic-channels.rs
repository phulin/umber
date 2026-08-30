//! Derives the per-channel contract for every committed minifixture against
//! the pinned instrumented pdfTeX 1.40.29 oracle.
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
//! An `xfail` channel's `bug` id is resolved by
//! `tex_command_stream::semantic::classify_divergence`, which matches the
//! *shape* of the first line the oracle produced that Umber's did not
//! against a fixed set of already-filed bugs (a missing `*` prompt, a
//! `show_context` accuracy gap, a missing box diagnostic, a file's `)`
//! closed early, and so on -- see `tools/tex-command-stream/src/semantic/
//! classify.rs`). It refuses rather than guesses: an unclassifiable
//! divergence falls back to whatever bug id the case's manifest already
//! declares for that channel (correct for a genuine one-off with no shared
//! shape to generalize), and a channel that newly diverges with *neither* a
//! classification *nor* an existing declaration fails the run with the
//! divergence printed, so a human decides which issue it belongs to before
//! the corpus can commit it.
//!
//! The `effects` channel projects the shared typed `tex-oracle` open, write,
//! and close events plus exact writer-created artifacts. Its expected bytes
//! come only from the instrumented reference stream and its captured output
//! files. Umber's replay-internal effect log is never an authority.
//!
//! A named, reviewed new `pass` case may derive its projection `expected`
//! array from this same run, the same way the tool derives `channels`:
//! `docs/testing_infrastructure.md` calls the projection layer
//! "Umber-self-authoritative by design", so `expected` has no oracle to read
//! it from, and the only thing it can mean for a `pass` case is exactly what
//! this run's own projection produces. That is not new information hiding
//! behind a new name -- a `pass` case's `expected` was already required to
//! equal a fresh run's projection byte for byte (`evaluate_expectation`
//! rejects any other outcome), so deriving it removes only the throwaway
//! harness every migration used to hand-build to capture that value once,
//! not a check. An `xfail` case's `expected` is different in kind, not just
//! in accuracy: it names a still-uncorrected divergence's position, which by
//! definition is not what the current run produces, so it is never derived
//! and this tool never rewrites it. Normal regeneration also preserves
//! authored pass fingerprints and reports differences. The targeted
//! `--accept-projection-change DOMAIN/CASE` route is the only mechanical
//! acceptance path and can update exactly one selected pass case.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only regeneration entry point reads its oracle capture and rewrites its committed corpus"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use tex_command_stream::semantic::{
    CapturedChannels, ChannelMismatch, ChannelPolicy, DeclaredCase, EffectArtifact, Expectation,
    STREAM_CHANNELS, SessionProfile, StreamChannel, StreamDisposition, channel_file,
    classify_divergence, execute, first_line_difference, first_line_difference_in, load_suite_with,
    normalize_channel, portable_effect_channel, project, reclassify_no_error_channel,
    repository_root, split_channel_lines, strip_diagnostic_reports,
};
use tex_oracle::{Event, ObservationStream};

fn main() -> ExitCode {
    if let Some(worker) = umber::dispatch_format_worker() {
        return match worker {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("command-semantic-channels format worker: {error}");
                ExitCode::from(70)
            }
        };
    }
    let mut arguments = std::env::args().skip(1);
    let outcome = match arguments.next().as_deref() {
        Some("--diff") => {
            let selector = arguments.next().unwrap_or_default();
            diff(&selector, StreamChannel::Terminal)
        }
        Some("--diff-log") => {
            let selector = arguments.next().unwrap_or_default();
            diff(&selector, StreamChannel::Log)
        }
        Some("--diff-diagnostics") => {
            let selector = arguments.next().unwrap_or_default();
            diff(&selector, StreamChannel::Diagnostics)
        }
        Some("--profile") => parse_profile_invocation(arguments)
            .and_then(|(profile, policy)| run(Some(profile), policy)),
        Some(other) => Err(format!(
            "unknown argument {other:?}; the only options are no arguments (regenerate the \
             corpus), `--diff <substring>` (print the oracle/Umber terminal text of every \
             matching case), `--diff-log <substring>` (the same for the transcript, which is \
             where §90 puts an error's help lines), and `--diff-diagnostics <substring>` (the \
             source-located typed lifecycle stream). No `--diff` option writes anything."
        )),
        None => run(None, ProjectionPolicy::Preserve),
    };
    match outcome {
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

/// Prints the pinned oracle's transcript beside Umber's own for every
/// case whose `<domain>/<id>` label contains `selector`, and writes nothing.
///
/// `channel` picks which transcript: the terminal, or the log -- which is
/// where §90 puts an error's help lines, so a help-routing difference is
/// invisible in the terminal one.
///
/// The committed corpus records only *where* a channel first diverges, which
/// is enough to tell that a case is wrong and never enough to tell what to
/// change. This is the one command that shows both sides in full, so a fix
/// can be aimed at what the reference actually prints rather than at a
/// one-line fingerprint of it.
fn diff(selector: &str, channel: StreamChannel) -> Result<String, String> {
    if selector.is_empty() {
        return Err("--diff needs a <domain>/<case> substring to select cases with".to_owned());
    }
    let oracle_root = oracle_root();
    let cases = load_suite_with(ChannelPolicy::Deriving)?;
    let mut shown = 0usize;
    for declared in &cases {
        let label = format!("{}/{}", declared.domain, declared.case.id);
        if !label.contains(selector) {
            continue;
        }
        shown += 1;
        let source = fs::read(declared.fixture_dir.join(&declared.case.source))
            .map_err(|error| format!("{label}: source read: {error}"))?;
        let completed = execute(&source, &declared.case)
            .map_err(|error| format!("{label}: this case does not run: {error}"))?;
        let captured = CapturedChannels::capture(&completed);
        let projection = project(&completed, &declared.case.projection);
        let stem = declared
            .case
            .source
            .strip_suffix(".tex")
            .expect("validate_case requires a .tex source");
        let case_dir = oracle_root.join(&declared.domain).join(&declared.case.id);
        let oracle_path = match channel {
            StreamChannel::Log => case_dir.join(format!("{stem}.log")),
            StreamChannel::Diagnostics => case_dir.join("pdftex14029-diagnostics.jsonl"),
            _ => case_dir.join("terminal.txt"),
        };
        let raw_oracle = if channel == StreamChannel::Diagnostics {
            oracle_diagnostic_channel(&oracle_path)?
        } else {
            fs::read(&oracle_path).unwrap_or_default()
        };
        let oracle = normalize_channel(channel, &raw_oracle)?;
        let umber = normalize_channel(channel, captured.stream(channel))?;
        println!("=== {label} ({}) ===", channel.name());
        println!(
            "events={} status={} projection={projection:?}",
            captured.events, captured.status
        );
        println!("--- source ---");
        println!("{}", String::from_utf8_lossy(&source).trim_end());
        print_side_by_side(&oracle, &umber);
    }
    if shown == 0 {
        return Err(format!("no case label contains {selector:?}"));
    }
    Ok(format!("{shown} case(s) shown; nothing written"))
}

/// One numbered line per row, oracle on the left and Umber on the right, with
/// every differing row marked. Trailing spaces are shown as `.` because they
/// are load-bearing: §314's descriptors (`<to be read again>␣`) end in one.
fn print_side_by_side(oracle: &[u8], umber: &[u8]) {
    fn lines(bytes: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(|line| line.replace(' ', "\u{b7}"))
            .collect()
    }
    let oracle = lines(oracle);
    let umber = lines(umber);
    println!("--- {:<54} | umber ---", "oracle");
    for index in 0..oracle.len().max(umber.len()) {
        let left = oracle.get(index).map_or("", String::as_str);
        let right = umber.get(index).map_or("", String::as_str);
        let mark = if left == right { ' ' } else { '!' };
        println!("{mark}{:>3} {left:<54} | {right}", index + 1);
    }
}

fn parse_profile(value: &str) -> Result<SessionProfile, String> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| format!("unknown command-semantic profile {value:?}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProjectionPolicy {
    Preserve,
    AcceptNamed(String),
}

fn parse_profile_invocation(
    mut arguments: impl Iterator<Item = String>,
) -> Result<(SessionProfile, ProjectionPolicy), String> {
    let profile = parse_profile(&arguments.next().unwrap_or_default())?;
    let policy = match arguments.next().as_deref() {
        None => ProjectionPolicy::Preserve,
        Some("--accept-projection-change") => {
            let selector = arguments.next().ok_or_else(|| {
                "--accept-projection-change requires exactly one DOMAIN/CASE selector".to_owned()
            })?;
            if selector.split('/').count() != 2
                || selector.starts_with('/')
                || selector.ends_with('/')
            {
                return Err(
                    "--accept-projection-change requires exactly one DOMAIN/CASE selector"
                        .to_owned(),
                );
            }
            ProjectionPolicy::AcceptNamed(selector)
        }
        Some("--accept-projection-changes") => {
            return Err(
                "global projection acceptance is forbidden; use --accept-projection-change \
                 DOMAIN/CASE for one reviewed candidate"
                    .to_owned(),
            );
        }
        Some(argument) => return Err(format!("unexpected --profile argument {argument:?}")),
    };
    if arguments.next().is_some() {
        return Err("--profile accepts at most one named projection candidate".to_owned());
    }
    Ok((profile, policy))
}

fn run(
    profile: Option<SessionProfile>,
    projection_policy: ProjectionPolicy,
) -> Result<String, String> {
    let oracle_root = oracle_root();
    let all_cases = load_suite_with(ChannelPolicy::Deriving)?;
    let cases: Vec<_> = all_cases
        .into_iter()
        .filter(|declared| {
            let label = format!("{}/{}", declared.domain, declared.case.id);
            let named = match &projection_policy {
                ProjectionPolicy::Preserve => true,
                ProjectionPolicy::AcceptNamed(selector) => selector == &label,
            };
            named
                && profile.is_none_or(|profile| {
                    declared.case.profile == profile && declared.case.capture.selected()
                })
        })
        .collect();
    if cases.is_empty() {
        return Err("capture policy selects no cases".to_owned());
    }

    let mut plans: BTreeMap<String, BTreeMap<String, CasePlan>> = BTreeMap::new();
    let mut unrunnable = Vec::new();
    let mut errors = Vec::new();
    let mut projection_drifts = Vec::new();
    let mut channel_drifts = Vec::new();
    let mut accepted_projection = false;

    for declared in &cases {
        let label = format!("{}/{}", declared.domain, declared.case.id);
        let source = fs::read(declared.fixture_dir.join(&declared.case.source))
            .map_err(|error| format!("{label}: source read: {error}"))?;
        match execute(&source, &declared.case) {
            Ok(completed) => {
                let captured = CapturedChannels::capture(&completed);
                let actual_projection = project(&completed, &declared.case.projection);
                let accepts_this_case = matches!(
                    &projection_policy,
                    ProjectionPolicy::AcceptNamed(selector) if selector == &label
                );
                let expected = if accepts_this_case {
                    accepted_projection = true;
                    plan_accepted_expected(declared, &actual_projection)
                } else if matches!(declared.case.expectation, Expectation::Pass)
                    && declared.case.expected.is_empty()
                {
                    Err(format!(
                        "an empty pass projection requires --accept-projection-change {label}"
                    ))
                } else {
                    if matches!(declared.case.expectation, Expectation::Pass)
                        && declared.case.expected != actual_projection
                    {
                        projection_drifts.push(label.clone());
                    }
                    Ok(None)
                };
                let plan = expected.and_then(|expected| {
                    plan_case(&oracle_root, declared, &source, &captured).and_then(|channels| {
                        let channels = if accepts_this_case {
                            Some(channels)
                        } else {
                            let manifest: serde_json::Value = serde_json::from_slice(
                                &fs::read(declared.fixture_dir.join("manifest.json"))
                                    .map_err(|error| error.to_string())?,
                            )
                            .map_err(|error| error.to_string())?;
                            if manifest.get("channels") != Some(&channels.value()) {
                                channel_drifts.push(label.clone());
                            }
                            None
                        };
                        Ok(CasePlan { expected, channels })
                    })
                });
                match plan {
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
    if matches!(projection_policy, ProjectionPolicy::AcceptNamed(_)) && !accepted_projection {
        return Err("the named projection candidate was not selected by this profile".to_owned());
    }
    if profile.is_some() && !unrunnable.is_empty() {
        return Err(format!(
            "{} selected case(s) did not run; refusing a partial rewrite:\n  {}",
            unrunnable.len(),
            unrunnable.join("\n  ")
        ));
    }

    let repository = repository_root();
    let staging_parent = repository.join("target");
    fs::create_dir_all(&staging_parent).map_err(|error| error.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix("command-semantic-publication-")
        .tempdir_in(&staging_parent)
        .map_err(|error| error.to_string())?;
    let mut publication_cases = Vec::new();
    let mut rewritten = 0;
    for (domain, cases_by_id) in &plans {
        let domain_dir = repository
            .join("tests/corpus/command-semantic")
            .join(domain);
        for (case_id, plan) in cases_by_id {
            let fixture_dir = domain_dir.join(case_id);
            let candidate = staging.path().join(domain).join(case_id);
            prepare_fixture(&repository, &fixture_dir, &candidate, case_id, plan)?;
            publication_cases.push(serde_json::json!({
                "staged": candidate.strip_prefix(&repository).map_err(|_| "staging escaped repository")?,
                "destination": fixture_dir.strip_prefix(&repository).map_err(|_| "fixture escaped repository")?,
                "authorities": [fixture_dir.strip_prefix(&repository).map_err(|_| "fixture escaped repository")?],
            }));
            rewritten += 1;
        }
    }
    publish_candidates(&repository, staging.path(), publication_cases)?;

    let mut summary = format!(
        "derived channel contracts for {rewritten} self-contained cases against the pinned pdfTeX 1.40.29 oracle"
    );
    if !unrunnable.is_empty() {
        summary.push_str(&format!(
            "\n{} case(s) do not run and carry no channel contract:\n  {}",
            unrunnable.len(),
            unrunnable.join("\n  ")
        ));
    }
    if !projection_drifts.is_empty() {
        summary.push_str(&format!(
            "\npreserved {} authored projection fingerprint(s) that differ from this run; the \
             correctness gate retains authority: {}",
            projection_drifts.len(),
            projection_drifts.join(", ")
        ));
    }
    if !channel_drifts.is_empty() {
        summary.push_str(&format!(
            "\npreserved {} authored channel contract(s) that differ from this run; the \
             correctness gate retains authority: {}",
            channel_drifts.len(),
            channel_drifts.join(", ")
        ));
    }
    Ok(summary)
}

fn prepare_fixture(
    repository: &Path,
    fixture_dir: &Path,
    candidate: &Path,
    case_id: &str,
    plan: &CasePlan,
) -> Result<(), String> {
    let candidate_parent = candidate
        .parent()
        .ok_or_else(|| format!("{} has no staging parent", candidate.display()))?;
    fs::create_dir_all(candidate_parent)
        .map_err(|error| format!("{}: {error}", candidate_parent.display()))?;
    let relative = fixture_dir
        .strip_prefix(repository)
        .map_err(|_| "command-semantic fixture escaped repository".to_owned())?;
    if let Some(parent) = candidate.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    test_support::closed_case::FixtureCase::discover_tracked_at(
        repository,
        relative,
        format!("{case_id}.tex"),
        "command-semantic-v2",
    )
    .and_then(|case| case.stage_into(candidate))
    .map_err(|error| format!("{}: {error:#}", fixture_dir.display()))?;
    write_channel_files(candidate, plan)?;
    rewrite_manifest(&candidate.join("manifest.json"), plan)?;
    test_support::closed_case::StagedCase::validate(candidate)
        .map_err(|error| format!("{}: {error:#}", candidate.display()))?;
    Ok(())
}

fn publish_candidates(
    repository: &Path,
    staging: &Path,
    cases: Vec<serde_json::Value>,
) -> Result<(), String> {
    if cases.is_empty() {
        return Ok(());
    }
    let fixturegen = std::env::var_os("UMBER_FIXTUREGEN").ok_or_else(|| {
        "UMBER_FIXTUREGEN must name fixturegen for atomic command-semantic publication".to_owned()
    })?;
    let plan = staging.join("publication.json");
    let value = serde_json::json!({
        "schema": "umber-fixture-cohort-plan-v1",
        "repository": repository,
        "cases": cases,
    });
    fs::write(
        &plan,
        serde_json::to_vec(&value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    for mode in ["--plan", "--apply"] {
        let output = Command::new(&fixturegen)
            .args(["--cohort-transaction", mode])
            .arg(&plan)
            .output()
            .map_err(|error| format!("run fixturegen {mode}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "fixturegen {mode} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    Ok(())
}

/// One channel's resolved disposition: `Empty` needs no committed file;
/// `File`'s bytes are always the oracle's; `Xfail` additionally carries the
/// bug it is pinned against and where Umber's own output first diverges from
/// those same committed bytes.
enum ChannelPlan {
    Empty,
    Unsupported {
        reason: String,
    },
    File {
        bytes: Vec<u8>,
    },
    Xfail {
        bytes: Vec<u8>,
        bug: String,
        mismatch: ChannelMismatch,
    },
    /// The case already declares `xfail-diagnostics` for this channel and it
    /// still holds: nothing outside a §82 error report diverges. Nothing here
    /// is derived, which is the point -- a report Umber improves no longer
    /// moves a pin that regeneration would have to absorb.
    XfailDiagnostics {
        bytes: Vec<u8>,
        bug: String,
    },
}

/// A case's complete derived plan: an optional explicitly accepted `expected`
/// projection array and its channel contract.
struct CasePlan {
    expected: Option<Vec<String>>,
    channels: Option<ChannelsPlan>,
}

struct ChannelsPlan {
    events: usize,
    status: String,
    /// In [`STREAM_CHANNELS`] order.
    channels: [ChannelPlan; 5],
}

/// Derives one explicitly named `pass` case's `expected` array from its own
/// freshly projected run. Global or xfail acceptance is unavailable.
fn plan_accepted_expected(
    declared: &DeclaredCase,
    actual: &[String],
) -> Result<Option<Vec<String>>, String> {
    if !matches!(declared.case.expectation, Expectation::Pass) {
        return Err(format!(
            "{} is an xfail projection and cannot be mechanically accepted",
            declared.case.id
        ));
    }
    Ok(Some(actual.to_vec()))
}

/// Resolves every stream channel's disposition for one case by comparing
/// Umber's freshly captured bytes against the pinned oracle's capture.
fn plan_case(
    oracle_root: &Path,
    declared: &DeclaredCase,
    source: &[u8],
    captured: &CapturedChannels,
) -> Result<ChannelsPlan, String> {
    let case_dir = oracle_root.join(&declared.domain).join(&declared.case.id);
    if !case_dir.is_dir() {
        return Err(format!(
            "no oracle capture at {} -- run `scripts/run-minifixture-oracle.sh --all` first \
             (build the oracle itself with python3 scripts/provision.py oracle pdftex14029 if its binary \
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
    let channels: [ChannelPlan; 5] = channels
        .try_into()
        .unwrap_or_else(|_| panic!("STREAM_CHANNELS has exactly 5 entries"));

    Ok(ChannelsPlan {
        events: captured.events,
        status: captured.status.clone(),
        channels,
    })
}

/// Resolves one channel's disposition against the oracle capture at
/// `case_dir`.
fn plan_channel(
    declared: &DeclaredCase,
    channel: StreamChannel,
    stem: &str,
    case_dir: &Path,
    captured: &CapturedChannels,
) -> Result<ChannelPlan, String> {
    if channel == StreamChannel::Effects
        && let Some(StreamDisposition::Unsupported { reason }) = declared
            .case
            .channels
            .as_ref()
            .map(|channels| channels.stream(channel))
    {
        return Ok(ChannelPlan::Unsupported {
            reason: reason.clone(),
        });
    }
    let raw_umber = captured.stream(channel);

    let oracle_path = match channel {
        StreamChannel::Terminal => case_dir.join("terminal.txt"),
        StreamChannel::Log => case_dir.join(format!("{stem}.log")),
        StreamChannel::Dvi => case_dir.join(format!("{stem}.dvi")),
        StreamChannel::Effects => case_dir.join("pdftex14029-events.jsonl"),
        StreamChannel::Diagnostics => case_dir.join("pdftex14029-diagnostics.jsonl"),
    };
    let raw_oracle = if channel == StreamChannel::Effects {
        oracle_effect_channel(case_dir)?
    } else if channel == StreamChannel::Diagnostics {
        oracle_diagnostic_channel(&oracle_path)?
    } else if oracle_path.exists() {
        fs::read(&oracle_path).map_err(|error| format!("{}: {error}", oracle_path.display()))?
    } else {
        Vec::new()
    };

    // `normalize_channel` is the single definition of what this corpus holds
    // uncomparable, shared with the ongoing gate's own `compare`, so a
    // channel this tool marks `file` stays `file` under the gate that reads
    // it back. It used to be spelled out separately here, and the copies had
    // drifted apart on the `dvi` channel.
    let oracle_bytes = normalize_channel(channel, &raw_oracle)?;
    let umber_bytes = normalize_channel(channel, raw_umber)?;

    if raw_oracle.is_empty() && raw_umber.is_empty() {
        return Ok(ChannelPlan::Empty);
    }

    match first_line_difference(&oracle_bytes, &umber_bytes) {
        None => Ok(ChannelPlan::File {
            bytes: oracle_bytes,
        }),
        Some(mismatch) => {
            // A channel already declared `xfail-diagnostics` derives nothing:
            // the disposition holds as long as the divergence is still
            // confined to §82's error reports, and that is the whole claim.
            // A divergence that escapes them is not something regeneration
            // may absorb, so it is reported for a human to adjudicate rather
            // than quietly rewritten into a pin.
            if let Some(bug) = declared_diagnostics_bug(declared, channel) {
                let filtered_oracle = strip_diagnostic_reports(&split_channel_lines(&oracle_bytes));
                let filtered_umber = strip_diagnostic_reports(&split_channel_lines(&umber_bytes));
                return match first_line_difference_in(&filtered_oracle, &filtered_umber) {
                    None => Ok(ChannelPlan::XfailDiagnostics {
                        bytes: oracle_bytes,
                        bug,
                    }),
                    Some(escaped) => Err(format!(
                        "channel {} declares xfail-diagnostics for {bug} but now diverges \
                         outside a diagnostic, at filtered line {} (oracle: {:?}, umber: \
                         {:?}); fix it or restate the disposition by hand before regenerating",
                        channel.name(),
                        escaped.line,
                        escaped.expected,
                        escaped.actual,
                    )),
                };
            }
            // The bug id is never invented (see this file's module doc).
            // Exactly one of three things decides it, in order:
            //
            // 1. A channel already pinned on `umber2-alfh.13` is re-tested
            //    against its own positional mismatch alone
            //    (`reclassify_no_error_channel`), because that guess is the
            //    one this tool exists to correct and a deeper scan would
            //    routinely misattribute it to an unrelated, already-correct
            //    label instead (see `classify.rs`'s module doc). A shape
            //    that reclassifies replaces `.13`; one that does not keeps
            //    it, since `.13` remains the best available answer until a
            //    human reclassifies it by hand.
            // 2. Any other already-declared bug (`.11`, `.14`-`.24`) is
            //    trusted outright and never re-examined: only `.13` was
            //    audited and found wrong.
            // 3. A channel with *no* existing declaration at all (a
            //    brand-new divergence) is classified by the general,
            //    deeper `classify_divergence` scan, since there is no
            //    already-correct label it could disturb. When even that
            //    refuses, a human must hand-author the disposition before
            //    regenerating.
            let bug = match existing_bug(declared, channel) {
                Some(bug) if bug == "umber2-alfh.13" => reclassify_no_error_channel(&mismatch)
                    .map_or(bug, |class| class.bug().to_owned()),
                Some(bug) => bug,
                None => classify_divergence(channel, &oracle_bytes, &umber_bytes)
                    .map(|class| class.bug().to_owned())
                    .ok_or_else(|| {
                        format!(
                            "channel {} newly diverges from the oracle at line {} (oracle: \
                             {:?}, umber: {:?}) and matches no known divergence shape and no \
                             existing xfail bug to preserve; hand-author `channels.{}` in the \
                             manifest as `{{\"kind\": \"xfail\", \"bug\": \
                             \"umber2-<epic>.<n>\", \"mismatch\": {{\"line\": 1, \"expected\": \
                             \"x\", \"actual\": \"y\"}}}}` before regenerating",
                            channel.name(),
                            mismatch.line,
                            mismatch.expected,
                            mismatch.actual,
                            channel.name(),
                        )
                    })?,
            };
            Ok(ChannelPlan::Xfail {
                bytes: oracle_bytes,
                bug,
                mismatch,
            })
        }
    }
}

/// Returns no comparison channel for the reference's header-only diagnostic
/// stream. The instrumented engine opens that stream for every run, while the
/// corpus opts into it only when a typed report exists; a report always keeps
/// its final lifecycle outcome beside it.
fn oracle_diagnostic_channel(path: &Path) -> Result<Vec<u8>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let stream = ObservationStream::from_canonical_json_lines(&bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if stream.events.iter().any(|event| {
        matches!(
            event.semantic,
            Event::DiagnosticLifecycle(tex_oracle::DiagnosticLifecycleEvent::Report { .. })
        )
    }) {
        Ok(bytes)
    } else {
        Ok(Vec::new())
    }
}

fn oracle_effect_channel(case_dir: &Path) -> Result<Vec<u8>, String> {
    let events_path = case_dir.join("pdftex14029-events.jsonl");
    let events_bytes =
        fs::read(&events_path).map_err(|error| format!("{}: {error}", events_path.display()))?;
    let stream = ObservationStream::from_canonical_json_lines(&events_bytes)
        .map_err(|error| format!("{}: {error}", events_path.display()))?;
    let effects = stream
        .events
        .into_iter()
        .filter_map(|event| match event.semantic {
            Event::Effect(effect) => Some(effect),
            _ => None,
        });

    let inventory_path = case_dir.join("effect-artifacts.txt");
    let inventory = fs::read_to_string(&inventory_path)
        .map_err(|error| format!("{}: {error}", inventory_path.display()))?;
    let mut previous: Option<&str> = None;
    let mut artifacts = Vec::new();
    for path in inventory.lines() {
        if path.is_empty()
            || Path::new(path).components().count() != 1
            || previous.is_some_and(|previous| previous >= path)
        {
            return Err(format!(
                "{} is not a strictly sorted list of nonempty file names",
                inventory_path.display()
            ));
        }
        previous = Some(path);
        let artifact_path = case_dir.join(path);
        artifacts.push(EffectArtifact {
            path: path.to_owned(),
            bytes: fs::read(&artifact_path)
                .map_err(|error| format!("{}: {error}", artifact_path.display()))?,
        });
    }
    Ok(portable_effect_channel(effects, artifacts))
}

/// The bug a case's manifest already declares for one channel's `xfail`
/// disposition, if any. See `plan_channel`'s three-way dispatch: this is
/// consulted first, and everything except an already-`.13` declaration is
/// trusted outright rather than re-examined.
/// The bug a case's manifest declares for one channel's `xfail-diagnostics`
/// disposition, if it declares that disposition at all.
fn declared_diagnostics_bug(declared: &DeclaredCase, channel: StreamChannel) -> Option<String> {
    let contract = declared.case.channels.as_ref()?;
    match contract.stream(channel) {
        StreamDisposition::XfailDiagnostics { bug } => Some(bug.clone()),
        StreamDisposition::Empty
        | StreamDisposition::File
        | StreamDisposition::Unsupported { .. }
        | StreamDisposition::Xfail { .. } => None,
    }
}

fn existing_bug(declared: &DeclaredCase, channel: StreamChannel) -> Option<String> {
    let contract = declared.case.channels.as_ref()?;
    match contract.stream(channel) {
        StreamDisposition::Xfail { bug, .. } | StreamDisposition::XfailDiagnostics { bug } => {
            Some(bug.clone())
        }
        StreamDisposition::Empty
        | StreamDisposition::File
        | StreamDisposition::Unsupported { .. } => None,
    }
}

fn write_channel_files(fixture_dir: &Path, plan: &CasePlan) -> Result<(), String> {
    let Some(channels) = &plan.channels else {
        return Ok(());
    };
    for (index, channel) in STREAM_CHANNELS.into_iter().enumerate() {
        let path = channel_file(fixture_dir, channel);
        match &channels.channels[index] {
            ChannelPlan::Empty | ChannelPlan::Unsupported { .. } => {
                if path.exists() {
                    fs::remove_file(&path)
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                }
            }
            ChannelPlan::File { bytes }
            | ChannelPlan::Xfail { bytes, .. }
            | ChannelPlan::XfailDiagnostics { bytes, .. } => {
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

impl ChannelsPlan {
    fn value(&self) -> serde_json::Value {
        let mut fields = serde_json::Map::new();
        fields.insert("events".to_owned(), self.events.into());
        if self.status != "clean" {
            fields.insert("status".to_owned(), self.status.clone().into());
        }
        for (index, channel) in STREAM_CHANNELS.into_iter().enumerate() {
            let disposition = match &self.channels[index] {
                ChannelPlan::Empty | ChannelPlan::File { .. } => continue,
                ChannelPlan::Unsupported { reason } => serde_json::json!({
                    "kind": "unsupported", "reason": reason,
                }),
                ChannelPlan::Xfail { bug, mismatch, .. } => serde_json::json!({
                    "kind": "xfail",
                    "bug": bug,
                    "mismatch": {
                        "line": mismatch.line,
                        "expected": mismatch.expected,
                        "actual": mismatch.actual,
                    },
                }),
                ChannelPlan::XfailDiagnostics { bug, .. } => serde_json::json!({
                    "kind": "xfail-diagnostics", "bug": bug,
                }),
            };
            fields.insert(channel.name().to_owned(), disposition);
        }
        fields.into()
    }
}

/// Replaces only an explicitly accepted projection and resolved channel
/// exceptions in a V2 manifest. Authored pass and xfail projections remain
/// untouched during normal regeneration.
fn rewrite_manifest(path: &Path, plan: &CasePlan) -> Result<(), String> {
    let mut manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?,
    )
    .map_err(|error| format!("{}: {error}", path.display()))?;
    let object = manifest
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a manifest object", path.display()))?;
    if let Some(expected) = &plan.expected {
        object.insert("expected".to_owned(), serde_json::json!(expected));
    }
    if let Some(channels) = &plan.channels {
        object.insert("channels".to_owned(), channels.value());
    }
    let mut bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_profiles_are_typed() {
        assert_eq!(
            parse_profile("raw-tex82-loaded"),
            Ok(SessionProfile::RawTex82Loaded)
        );
        assert!(parse_profile("unknown").is_err());
    }

    #[test]
    fn profile_regeneration_forbids_global_projection_acceptance() {
        let error = parse_profile_invocation(
            [
                "raw-tex82-loaded".to_owned(),
                "--accept-projection-changes".to_owned(),
            ]
            .into_iter(),
        )
        .expect_err("global acceptance must remain unavailable");
        assert!(error.contains("global projection acceptance is forbidden"));

        assert_eq!(
            parse_profile_invocation(
                [
                    "raw-tex82-loaded".to_owned(),
                    "--accept-projection-change".to_owned(),
                    "line-breaking/paragraph-line-shape".to_owned(),
                ]
                .into_iter(),
            ),
            Ok((
                SessionProfile::RawTex82Loaded,
                ProjectionPolicy::AcceptNamed("line-breaking/paragraph-line-shape".to_owned()),
            ))
        );
    }

    #[test]
    fn regeneration_prepares_one_complete_candidate_without_mutating_authority() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let fixture = temporary
            .path()
            .join("tests/corpus/command-semantic/example/case-a");
        fs::create_dir_all(&fixture).expect("fixture directory");
        fs::write(fixture.join("case-a.tex"), b"\\end\n").expect("source");
        fs::write(
            fixture.join("manifest.json"),
            concat!(
                "{\n",
                "  \"schema\": 2,\n",
                "  \"property_id\": \"tex82.example.case\",\n",
                "  \"provenance\": {\n",
                "    \"authority\": \"tex.web\",\n",
                "    \"manifest\": \"tests/tex82-oracle-manifest.txt\",\n",
                "    \"sections\": [1]\n",
                "  },\n",
                "  \"projection\": { \"kind\": \"predicate-outcomes\" },\n",
                "  \"expected\": [\"old\"],\n",
                "  \"channels\": { \"events\": 1 }\n",
                "}\n",
            ),
        )
        .expect("manifest");
        Command::new("git")
            .current_dir(temporary.path())
            .args(["init", "-q"])
            .status()
            .expect("initialize Git fixture");
        Command::new("git")
            .current_dir(temporary.path())
            .args(["add", "."])
            .status()
            .expect("track fixture");
        let plan = CasePlan {
            expected: Some(vec!["new".to_owned()]),
            channels: Some(ChannelsPlan {
                events: 2,
                status: "clean".to_owned(),
                channels: std::array::from_fn(|_| ChannelPlan::Empty),
            }),
        };
        let candidate = temporary.path().join("candidate/example/case-a");
        prepare_fixture(temporary.path(), &fixture, &candidate, "case-a", &plan)
            .expect("prepare candidate");

        assert_eq!(
            fs::read(fixture.join("case-a.tex")).expect("preserved source"),
            b"\\end\n"
        );
        let authority = fs::read_to_string(fixture.join("manifest.json")).expect("authority");
        assert!(authority.contains("\"old\""));
        let manifest = fs::read_to_string(candidate.join("manifest.json")).expect("manifest");
        assert!(manifest.contains("\"new\""));
        assert!(manifest.contains("\"events\": 2"));
    }
}
