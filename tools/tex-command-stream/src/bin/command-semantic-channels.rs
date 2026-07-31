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
//! The `effects` channel has no oracle-comparable source at all: it is
//! Umber's own structured rendering of stream opens/closes/writes and shell
//! escapes (`open:`/`close:`/`write:`/... lines), not a reproduction of
//! anything a real TeX engine writes. Every case's `effects` capture is
//! empty today, so this tool requires that and fails loudly if it ever stops
//! being true, rather than inventing an authority for content with no
//! reference to check it against.
//!
//! This tool also derives a `pass` case's projection `expected` array from
//! this same run (`plan_expected`), the same way it derives `channels`:
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
//! and this tool never rewrites it. A `pass` case whose freshly derived
//! `expected` disagrees with what is already committed is a real behavior
//! change, and this tool refuses to absorb it silently: the whole batch
//! fails with both arrays printed, so a human reviews the diff before it can
//! land.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only regeneration entry point reads its oracle capture and rewrites its committed corpus"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tex_command_stream::semantic::{
    CapturedChannels, ChannelMismatch, ChannelPolicy, DeclaredCase, Expectation, STREAM_CHANNELS,
    StreamChannel, StreamDisposition, channel_file, classify_divergence, execute,
    first_line_difference, first_line_difference_in, load_suite_with, normalize_channel, project,
    reclassify_no_error_channel, repository_root, split_channel_lines, strip_diagnostic_reports,
};

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
        Some("--allowlist") => {
            let path = arguments.next().unwrap_or_default();
            if arguments.next().as_deref() != Some("--accept-projection-changes")
                || arguments.next().is_some()
            {
                Err(
                    "--allowlist requires exactly PATH --accept-projection-changes; the explicit \
                     loaded-profile regeneration route is the only batch allowed to update \
                     reviewed projections"
                        .to_owned(),
                )
            } else {
                run(Some(Path::new(&path)), true)
            }
        }
        Some(other) => Err(format!(
            "unknown argument {other:?}; the only options are no arguments (regenerate the \
             corpus), `--diff <substring>` (print the oracle/Umber terminal text of every \
             matching case), and `--diff-log <substring>` (the same for the transcript, which \
             is where §90 puts an error's help lines). Neither `--diff` writes anything."
        )),
        None => run(None, false),
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
        let stem = declared
            .case
            .source
            .strip_suffix(".tex")
            .expect("validate_case requires a .tex source");
        let case_dir = oracle_root.join(&declared.domain).join(&declared.case.id);
        let oracle_path = match channel {
            StreamChannel::Log => case_dir.join(format!("{stem}.log")),
            _ => case_dir.join("terminal.txt"),
        };
        let raw_oracle = fs::read(&oracle_path).unwrap_or_default();
        let oracle = normalize_channel(channel, &raw_oracle)?;
        let umber = normalize_channel(channel, captured.stream(channel))?;
        println!("=== {label} ({}) ===", channel.name());
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

fn read_allowlist(path: &Path) -> Result<BTreeSet<String>, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let selected: BTreeSet<_> = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if selected.is_empty() {
        return Err(format!("{} selects no cases", path.display()));
    }
    Ok(selected)
}

fn run(allowlist: Option<&Path>, accept_projection_changes: bool) -> Result<String, String> {
    let oracle_root = oracle_root();
    let all_cases = load_suite_with(ChannelPolicy::Deriving)?;
    let selected = allowlist.map(read_allowlist).transpose()?;
    let cases: Vec<_> = all_cases
        .into_iter()
        .filter(|declared| {
            selected.as_ref().is_none_or(|selected| {
                selected.contains(&format!("{}/{}", declared.domain, declared.case.id))
            })
        })
        .collect();
    if let Some(selected) = &selected {
        let found: BTreeSet<_> = cases
            .iter()
            .map(|declared| format!("{}/{}", declared.domain, declared.case.id))
            .collect();
        if found != *selected {
            let missing: Vec<_> = selected.difference(&found).cloned().collect();
            return Err(format!(
                "allowlist names unknown case(s): {}",
                missing.join(", ")
            ));
        }
    }

    let mut plans: BTreeMap<String, BTreeMap<String, CasePlan>> = BTreeMap::new();
    let mut unrunnable = Vec::new();
    let mut errors = Vec::new();

    for declared in &cases {
        let label = format!("{}/{}", declared.domain, declared.case.id);
        let source = fs::read(declared.fixture_dir.join(&declared.case.source))
            .map_err(|error| format!("{label}: source read: {error}"))?;
        match execute(&source, &declared.case) {
            Ok(completed) => {
                let captured = CapturedChannels::capture(&completed);
                let actual_projection = project(&completed, &declared.case.projection);
                let expected = if accept_projection_changes
                    && matches!(declared.case.expectation, Expectation::Pass)
                {
                    Ok(Some(actual_projection.clone()))
                } else {
                    plan_expected(declared, &actual_projection)
                };
                let plan = expected.and_then(|expected| {
                    plan_case(&oracle_root, declared, &source, &captured)
                        .map(|channels| CasePlan { expected, channels })
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
    if selected.is_some() && !unrunnable.is_empty() {
        return Err(format!(
            "{} allowlisted case(s) did not run; refusing a partial rewrite:\n  {}",
            unrunnable.len(),
            unrunnable.join("\n  ")
        ));
    }

    let mut rewritten = 0;
    for (domain, cases_by_id) in &plans {
        let domain_dir = repository_root()
            .join("tests/corpus/command-semantic")
            .join(domain);
        for (case_id, plan) in cases_by_id {
            let fixture_dir = domain_dir.join(case_id);
            rewrite_fixture_atomically(
                &|from, to| fs::rename(from, to),
                &fixture_dir,
                plan,
                cases_by_id,
            )?;
            rewritten += 1;
        }
    }

    let mut summary = format!(
        "derived channel contracts for {rewritten} self-contained cases against the pinned pdfTeX 1.40.27 oracle"
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
    /// The case already declares `xfail-diagnostics` for this channel and it
    /// still holds: nothing outside a §82 error report diverges. Nothing here
    /// is derived, which is the point -- a report Umber improves no longer
    /// moves a pin that regeneration would have to absorb.
    XfailDiagnostics {
        bytes: Vec<u8>,
        bug: String,
    },
}

/// A case's complete derived plan: its `expected` projection array (a
/// `pass` case only; `None` for `xfail`, which is never derived -- see this
/// file's module doc) and its channel contract.
struct CasePlan {
    expected: Option<Vec<String>>,
    channels: ChannelsPlan,
}

struct ChannelsPlan {
    events: usize,
    status: String,
    /// In [`STREAM_CHANNELS`] order.
    channels: [ChannelPlan; 4],
}

/// Derives a `pass` case's `expected` array from its own freshly projected
/// run, or refuses when that would silently absorb a behavior change. See
/// this file's module doc.
fn plan_expected(
    declared: &DeclaredCase,
    actual: &[String],
) -> Result<Option<Vec<String>>, String> {
    if !matches!(declared.case.expectation, Expectation::Pass) {
        return Ok(None);
    }
    if !declared.case.expected.is_empty() && declared.case.expected != actual {
        return Err(format!(
            "expected observations changed: committed {:?}, this run's projection now produces \
             {actual:?}; regeneration never absorbs a pass case's behavior change silently -- \
             review the difference and, if it is an intended, reviewed change, update this \
             case's committed \"expected\" to the new array by hand before regenerating again",
            declared.case.expected,
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

    Ok(ChannelsPlan {
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
        StreamDisposition::Empty | StreamDisposition::File | StreamDisposition::Xfail { .. } => {
            None
        }
    }
}

fn existing_bug(declared: &DeclaredCase, channel: StreamChannel) -> Option<String> {
    let contract = declared.case.channels.as_ref()?;
    match contract.stream(channel) {
        StreamDisposition::Xfail { bug, .. } | StreamDisposition::XfailDiagnostics { bug } => {
            Some(bug.clone())
        }
        StreamDisposition::Empty | StreamDisposition::File => None,
    }
}

fn write_channel_files(fixture_dir: &Path, plan: &CasePlan) -> Result<(), String> {
    for (index, channel) in STREAM_CHANNELS.into_iter().enumerate() {
        let path = channel_file(fixture_dir, channel);
        match &plan.channels.channels[index] {
            ChannelPlan::Empty => {
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

/// Prepares the complete regenerated fixture beside the live directory, then
/// swaps it into place. A failed derivation therefore cannot leave a fixture
/// with new channel bytes and old metadata (or the reverse).
fn rewrite_fixture_atomically(
    rename: &impl Fn(&Path, &Path) -> io::Result<()>,
    fixture_dir: &Path,
    plan: &CasePlan,
    cases: &BTreeMap<String, CasePlan>,
) -> Result<(), String> {
    let parent = fixture_dir
        .parent()
        .ok_or_else(|| format!("{} has no parent", fixture_dir.display()))?;
    let name = fixture_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("non-UTF-8 fixture directory {}", fixture_dir.display()))?;
    let staging = parent.join(format!(".{name}.regen-staging"));
    let backup = parent.join(format!(".{name}.regen-backup"));
    if staging.exists() || backup.exists() {
        return Err(format!(
            "refusing to overwrite interrupted regeneration state {} or {}",
            staging.display(),
            backup.display()
        ));
    }
    fs::create_dir(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let prepared = (|| {
        for entry in fs::read_dir(fixture_dir)
            .map_err(|error| format!("{}: {error}", fixture_dir.display()))?
        {
            let entry = entry.map_err(|error| format!("{}: {error}", fixture_dir.display()))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            if !file_type.is_file() {
                return Err(format!(
                    "fixture entry {} is not a regular file",
                    entry.path().display()
                ));
            }
            fs::copy(entry.path(), staging.join(entry.file_name()))
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        }
        write_channel_files(&staging, plan)?;
        rewrite_manifest(&staging.join("manifest.json"), cases)
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    rename(fixture_dir, &backup)
        .map_err(|error| format!("{} -> {}: {error}", fixture_dir.display(), backup.display()))?;
    if let Err(install_error) = rename(&staging, fixture_dir) {
        return match rename(&backup, fixture_dir) {
            Ok(()) => Err(format!(
                "{} -> {}: {install_error}; restored original fixture from {}",
                staging.display(),
                fixture_dir.display(),
                backup.display()
            )),
            Err(restore_error) => Err(format!(
                "{} -> {}: {install_error}; restoring authoritative backup {} -> {} also failed: \
                 {restore_error}; original fixture remains recoverable at {}",
                staging.display(),
                fixture_dir.display(),
                backup.display(),
                fixture_dir.display(),
                backup.display()
            )),
        };
    }
    fs::remove_dir_all(&backup).map_err(|error| format!("{}: {error}", backup.display()))
}

/// Renders a derived `expected` array close to the shape `dprint` produces,
/// one observation per line, exactly like every hand-authored one already
/// committed; `dprint fmt` (part of `scripts/regen-fixtures.sh`'s own
/// workflow) settles any remaining wrapping.
fn expected_json(expected: &[String], indent: &str) -> String {
    let inner = format!("{indent}  ");
    let items: Vec<String> = expected
        .iter()
        .map(|observation| format!("{inner}{}", json_string(observation)))
        .collect();
    format!("[\n{}\n{indent}]", items.join(",\n"))
}

impl ChannelsPlan {
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
                ChannelPlan::XfailDiagnostics { bug, .. } => format!(
                    "{{ \"kind\": \"xfail-diagnostics\", \"bug\": {} }}",
                    json_string(bug),
                ),
            };
            fields.push(format!("{inner}\"{}\": {disposition}", channel.name()));
        }
        format!("{{\n{}\n{indent}}}", fields.join(",\n"))
    }
}

/// Inserts each case's derived `expected` array (when derived at all -- an
/// `xfail` case's is left untouched) and `channels` block into the manifest
/// immediately *before* its `expectation` key, preserving every other byte
/// of the file.
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

    let mut skipping_expected = 0i32;
    let mut skipping_channels = 0i32;
    for line in text.lines() {
        let trimmed = line.trim_start();

        // Drop any previously derived block, which the formatter may have
        // expanded across several lines. Counting delimiters rather than
        // matching one line is what makes the tool re-runnable on its own
        // output.
        if skipping_expected > 0 {
            skipping_expected += brackets(line);
            continue;
        }
        if skipping_channels > 0 {
            skipping_channels += braces(line);
            continue;
        }

        if let Some(id) = trimmed
            .strip_prefix("\"id\": \"")
            .and_then(|rest| rest.split('"').next())
        {
            current = cases.get(id);
        }
        // Only a case whose `expected` was actually derived (a `pass` case)
        // has its existing array stripped here; an `xfail` case's committed
        // `expected` passes through this loop untouched, exactly like every
        // other field this tool does not derive.
        if current.is_some_and(|plan| plan.expected.is_some())
            && trimmed.starts_with("\"expected\":")
        {
            skipping_expected = brackets(line);
            continue;
        }
        if trimmed.starts_with("\"channels\":") {
            skipping_channels = braces(line);
            continue;
        }

        if let Some(plan) = current
            && trimmed.starts_with("\"expectation\":")
        {
            let indent = &line[..line.len() - trimmed.len()];
            if let Some(expected) = &plan.expected {
                output.push_str(&format!(
                    "{indent}\"expected\": {},\n",
                    expected_json(expected, indent)
                ));
            }
            output.push_str(&format!(
                "{indent}\"channels\": {},\n",
                plan.channels.json(indent)
            ));
            applied += 1;
            current = None;
        }
        output.push_str(line);
        output.push('\n');
    }
    if skipping_expected != 0 {
        return Err(format!(
            "{}: an existing \"expected\" array does not close",
            path.display()
        ));
    }
    if skipping_channels != 0 {
        return Err(format!(
            "{}: an existing \"channels\" block does not close",
            path.display()
        ));
    }

    if applied != 1 {
        return Err(format!(
            "{}: rewrote {applied} cases, expected exactly one local fixture case with an \
             \"expectation\" anchor",
            path.display()
        ));
    }
    fs::write(path, output).map_err(|error| format!("{}: {error}", path.display()))
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

/// Net `{`/`}` depth a line contributes, ignoring braces inside JSON strings.
fn braces(line: &str) -> i32 {
    net_delimiter_depth(line, '{', '}')
}

/// Net `[`/`]` depth a line contributes, ignoring brackets inside JSON
/// strings.
fn brackets(line: &str) -> i32 {
    net_delimiter_depth(line, '[', ']')
}

fn net_delimiter_depth(line: &str, open: char, close: char) -> i32 {
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for character in line.chars() {
        match character {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            character if !in_string && character == open => depth += 1,
            character if !in_string && character == close => depth -= 1,
            _ => {}
        }
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn allowlist_is_exact_and_rejects_empty_input() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let allowlist = temporary.path().join("cases");
        fs::write(&allowlist, "# comment\nb/two\n\na/one # note\nb/two\n").expect("allowlist");
        assert_eq!(
            read_allowlist(&allowlist).expect("valid allowlist"),
            BTreeSet::from(["a/one".to_owned(), "b/two".to_owned()])
        );
        fs::write(&allowlist, "# only comments\n").expect("empty allowlist");
        assert!(read_allowlist(&allowlist).is_err());
    }

    #[test]
    fn regeneration_replaces_one_complete_fixture_directory() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let fixture = temporary.path().join("case-a");
        fs::create_dir(&fixture).expect("fixture directory");
        fs::write(fixture.join("case-a.tex"), b"\\end\n").expect("source");
        fs::write(
            fixture.join("manifest.json"),
            concat!(
                "{\n",
                "  \"cases\": [\n",
                "    {\n",
                "      \"id\": \"case-a\",\n",
                "      \"expected\": [\n",
                "        \"old\"\n",
                "      ],\n",
                "      \"channels\": {\n",
                "        \"events\": 1\n",
                "      },\n",
                "      \"expectation\": { \"kind\": \"pass\" }\n",
                "    }\n",
                "  ]\n",
                "}\n",
            ),
        )
        .expect("manifest");
        let plan = CasePlan {
            expected: Some(vec!["new".to_owned()]),
            channels: ChannelsPlan {
                events: 2,
                status: "clean".to_owned(),
                channels: std::array::from_fn(|_| ChannelPlan::Empty),
            },
        };
        let plans = BTreeMap::from([("case-a".to_owned(), plan)]);
        rewrite_fixture_atomically(
            &|from, to| fs::rename(from, to),
            &fixture,
            plans.get("case-a").expect("plan"),
            &plans,
        )
        .expect("atomic rewrite");

        assert_eq!(
            fs::read(fixture.join("case-a.tex")).expect("preserved source"),
            b"\\end\n"
        );
        let manifest = fs::read_to_string(fixture.join("manifest.json")).expect("manifest");
        assert!(manifest.contains("\"new\""));
        assert!(manifest.contains("\"events\": 2"));
        assert!(!temporary.path().join(".case-a.regen-staging").exists());
        assert!(!temporary.path().join(".case-a.regen-backup").exists());
    }

    fn fixture_and_plans(parent: &Path) -> (PathBuf, BTreeMap<String, CasePlan>) {
        let fixture = parent.join("case-a");
        fs::create_dir(&fixture).expect("fixture directory");
        fs::write(fixture.join("case-a.tex"), b"\\end\n").expect("source");
        fs::write(
            fixture.join("manifest.json"),
            concat!(
                "{\n  \"cases\": [{\n",
                "    \"id\": \"case-a\",\n",
                "    \"expectation\": { \"kind\": \"pass\" }\n",
                "  }]\n}\n",
            ),
        )
        .expect("manifest");
        let plan = CasePlan {
            expected: Some(vec!["new".to_owned()]),
            channels: ChannelsPlan {
                events: 2,
                status: "clean".to_owned(),
                channels: std::array::from_fn(|_| ChannelPlan::Empty),
            },
        };
        (fixture, BTreeMap::from([("case-a".to_owned(), plan)]))
    }

    #[test]
    fn install_failure_restores_original_fixture_and_reports_recovery() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let (fixture, plans) = fixture_and_plans(temporary.path());
        let calls = Cell::new(0);
        let rename = |from: &Path, to: &Path| {
            let call = calls.get();
            calls.set(call + 1);
            if call == 1 {
                Err(io::Error::other("injected candidate install failure"))
            } else {
                fs::rename(from, to)
            }
        };

        let error = rewrite_fixture_atomically(
            &rename,
            &fixture,
            plans.get("case-a").expect("plan"),
            &plans,
        )
        .expect_err("candidate install must fail");

        assert!(error.contains("injected candidate install failure"));
        assert!(error.contains("restored original fixture"));
        assert_eq!(
            fs::read(fixture.join("case-a.tex")).expect("restored source"),
            b"\\end\n"
        );
        assert!(!temporary.path().join(".case-a.regen-backup").exists());
        assert!(temporary.path().join(".case-a.regen-staging").exists());
    }

    #[test]
    fn install_and_restore_failure_preserves_backup_and_reports_both_errors() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let (fixture, plans) = fixture_and_plans(temporary.path());
        let calls = Cell::new(0);
        let rename = |from: &Path, to: &Path| {
            let call = calls.get();
            calls.set(call + 1);
            match call {
                0 => fs::rename(from, to),
                1 => Err(io::Error::other("injected candidate install failure")),
                2 => Err(io::Error::other("injected backup restore failure")),
                _ => unreachable!("unexpected rename call"),
            }
        };

        let error = rewrite_fixture_atomically(
            &rename,
            &fixture,
            plans.get("case-a").expect("plan"),
            &plans,
        )
        .expect_err("candidate install and restoration must fail");

        assert!(error.contains("injected candidate install failure"));
        assert!(error.contains("injected backup restore failure"));
        assert!(error.contains("original fixture remains recoverable at"));
        let backup = temporary.path().join(".case-a.regen-backup");
        assert_eq!(
            fs::read(backup.join("case-a.tex")).expect("recoverable source"),
            b"\\end\n"
        );
        assert!(!fixture.exists());
        assert!(temporary.path().join(".case-a.regen-staging").exists());
    }
}
