//! Persistent in-process Gentle profiling workload.

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tex_exec::{CheckpointSink, EngineBoundary, EngineCheckpoint};
use tex_incr::{AcceptedOutput, BoundaryKey, Edit, ReuseMetrics, RevisionId, Session};
#[cfg(feature = "profiling-stats")]
use tex_lex::ExpansionStats;
use tex_lex::{InputStack, WorldInput};
#[cfg(feature = "profiling-stats")]
use tex_state::measurement::{ExactIdentityMeasurement, exact_identity_measurement};
#[cfg(feature = "profiling-stats")]
use tex_state::survivor::{SurvivorMeasurement, survivor_measurement};
use tex_state::{
    ContentHash, JobClock, PureMemoConfig, PureMemoRecordingPolicy, PureMemoStats, Universe, World,
};
use tex_state::{MemoLayerStats, ParagraphValidationFailure, PureMemoLayer};
use umber::{EngineSession, FileSessionResolvers, dvi_from_page_plans, prepare_run_stores};

const JOB_DIR: &str = "/gentle-profile";
const JOB_FILE: &str = "profile-job.tex";
const DEFAULT_ITERATIONS: usize = 50;
const DEFAULT_WARMUPS: usize = 1;
const GENTLE_EDIT_OLD: &str = "There are ten characters which, like the backslash, are used";
const GENTLE_EDIT_SENTENCE: &str = "This deliberately extended explanation adds ordinary words to the same paragraph so that TeX must reconsider many line breaks and carry the resulting vertical material across page boundaries.";
const GENTLE_EDIT_REPETITIONS: usize = 64;
const GENTLE_FOLLOW_UP: &str = " A measured follow-up changes this paragraph again.";
const GENTLE_EQUAL_WIDTH_OLD: &str = "words";
const GENTLE_EQUAL_WIDTH_NEW: &str = "sword";
const GENTLE_FAST_PATH_RETYPED_PAGES: usize = 3;

#[derive(Debug)]
struct Options {
    repo_root: PathBuf,
    iterations: usize,
    warmups: usize,
    checkpoints: bool,
    incremental_edit: bool,
    baseline_memo_recording: Option<PureMemoRecordingPolicy>,
    memo_recording: PureMemoRecordingPolicy,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut iterations = DEFAULT_ITERATIONS;
        let mut warmups = DEFAULT_WARMUPS;
        let mut checkpoints = false;
        let mut incremental_edit = false;
        let mut baseline_memo_recording = None;
        let mut memo_recording = PureMemoRecordingPolicy::default();
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--repo-root" => {
                    repo_root = PathBuf::from(next_value(&mut args, "--repo-root")?);
                }
                "--iterations" => {
                    iterations = parse_positive_count(
                        &next_value(&mut args, "--iterations")?,
                        "--iterations",
                    )?;
                }
                "--warmups" => {
                    warmups =
                        parse_positive_count(&next_value(&mut args, "--warmups")?, "--warmups")?;
                }
                "--checkpoints" => checkpoints = true,
                "--incremental-edit" => incremental_edit = true,
                "--memo-layers" => {
                    memo_recording = parse_memo_layers(&next_value(&mut args, "--memo-layers")?)?;
                }
                "--baseline-memo-layers" => {
                    baseline_memo_recording = Some(parse_memo_layers(&next_value(
                        &mut args,
                        "--baseline-memo-layers",
                    )?)?);
                }
                "-h" | "--help" => {
                    print_help();
                    return Ok(None);
                }
                _ => {
                    return Err(format!(
                        "unknown argument: {arg}\n\nRun with --help for usage."
                    ));
                }
            }
        }
        let repo_root = repo_root
            .canonicalize()
            .map_err(|error| format!("resolve repository root {}: {error}", repo_root.display()))?;
        Ok(Some(Self {
            repo_root,
            iterations,
            warmups,
            checkpoints,
            incremental_edit,
            baseline_memo_recording,
            memo_recording,
        }))
    }
}

struct RunOutput {
    dvi: Vec<u8>,
    pages: usize,
    checkpoints: usize,
    checkpoint_hash: u64,
    #[cfg(feature = "profiling-stats")]
    expansion_stats: ExpansionStats,
}

#[derive(Default)]
struct ProfileCheckpointSink {
    count: usize,
    hash: u64,
}

impl CheckpointSink for ProfileCheckpointSink {
    fn checkpoint(&mut self, checkpoint: EngineCheckpoint) {
        self.count += 1;
        self.hash = self.hash.rotate_left(7) ^ checkpoint.state_hash();
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("gentle-profile: {error}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::disallowed_methods)] // Host-side profiling timer; no engine fact observes it.
fn run() -> Result<(), String> {
    let Some(options) = Options::parse()? else {
        return Ok(());
    };
    let template = load_template(&options.repo_root)?;

    if options.incremental_edit {
        return run_incremental_edit(&options, &template);
    }

    let reference = execute_once(&template, options.checkpoints)?;
    for _ in 1..options.warmups {
        let output = execute_once(&template, options.checkpoints)?;
        if output.dvi != reference.dvi {
            return Err("a warm-up DVI differs from the first warm-up DVI".to_owned());
        }
    }

    let started = Instant::now();
    let mut last = execute_once(&template, options.checkpoints)?;
    let _ = black_box(last.pages);
    let _ = black_box(last.dvi.len());
    let _ = black_box((last.checkpoints, last.checkpoint_hash));
    for _ in 1..options.iterations {
        last = execute_once(&template, options.checkpoints)?;
        let _ = black_box(last.pages);
        let _ = black_box(last.dvi.len());
        let _ = black_box((last.checkpoints, last.checkpoint_hash));
    }
    let elapsed = started.elapsed();
    if last.dvi != reference.dvi {
        return Err("the final measured DVI differs from the warm-up DVI".to_owned());
    }

    print_summary(&options, &last, elapsed);
    Ok(())
}

struct IncrementalFixture {
    original: String,
    revisions: Vec<String>,
    edits: Vec<Edit>,
    edit_names: Vec<&'static str>,
    edit_paths: Vec<IncrementalPath>,
    suffix_adoption_edit: usize,
    body_offset: usize,
    body_len: usize,
    inserted_bytes: usize,
    inserted_words: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IncrementalPath {
    Slow,
    Interaction,
    Fast,
}

impl IncrementalPath {
    const ALL: [Self; 3] = [Self::Slow, Self::Interaction, Self::Fast];

    const fn name(self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Interaction => "interaction",
            Self::Fast => "fast",
        }
    }
}

struct IncrementalSample {
    priming_elapsed: Duration,
    priming_memo: PureMemoStats,
    steps: Vec<IncrementalStep>,
}

struct IncrementalStep {
    elapsed: Duration,
    dvi_latency: Duration,
    dvi: Vec<u8>,
    pages: usize,
    reuse: ReuseMetrics,
    history: Vec<(BoundaryKey, usize, usize)>,
    memo: PureMemoStats,
    previous_memo: PureMemoStats,
    #[cfg(feature = "profiling-stats")]
    exact_identity: ExactIdentityMeasurement,
    #[cfg(feature = "profiling-stats")]
    survivor: SurvivorMeasurement,
}

#[derive(Clone, Copy, Default)]
struct IncrementalStages {
    revision_setup: Duration,
    restart_fork: Duration,
    executor: Duration,
    executor_shell: Duration,
    output_snapshot: Duration,
    generation_transition: Duration,
    splice: Duration,
    substrate_transition: Duration,
    acceptance: Duration,
    unaccounted: Duration,
    dvi_materialization: Duration,
}

impl IncrementalStages {
    fn from_step(step: &IncrementalStep) -> Self {
        let reuse = step.reuse;
        let executor_shell = reuse
            .reexecution_latency
            .saturating_sub(reuse.executor_latency);
        let accounted = reuse
            .revision_setup_latency
            .saturating_add(reuse.restart_fork_latency)
            .saturating_add(reuse.reexecution_latency)
            .saturating_add(reuse.output_snapshot_latency)
            .saturating_add(reuse.generation_transition_latency)
            .saturating_add(reuse.splice_latency)
            .saturating_add(reuse.substrate_transition_latency)
            .saturating_add(reuse.acceptance_latency);
        Self {
            revision_setup: reuse.revision_setup_latency,
            restart_fork: reuse.restart_fork_latency,
            executor: reuse.executor_latency,
            executor_shell,
            output_snapshot: reuse.output_snapshot_latency,
            generation_transition: reuse.generation_transition_latency,
            splice: reuse.splice_latency,
            substrate_transition: reuse.substrate_transition_latency,
            acceptance: reuse.acceptance_latency,
            unaccounted: step.elapsed.saturating_sub(accounted),
            dvi_materialization: step.dvi_latency,
        }
    }
}

fn run_incremental_edit(options: &Options, template: &World) -> Result<(), String> {
    if options.checkpoints {
        return Err("--incremental-edit cannot be combined with --checkpoints".to_owned());
    }
    if !options.iterations.is_multiple_of(2) {
        return Err(
            "--incremental-edit requires an even --iterations count for balanced AB/BA pairing"
                .to_owned(),
        );
    }
    let fixture = incremental_fixture(&options.repo_root)?;
    let baseline_recording = options.baseline_memo_recording.unwrap_or_default();
    let baseline_memo = options.baseline_memo_recording.is_some();
    let (baseline_name, candidate_name, delta_name) = if baseline_memo {
        ("memo baseline", "memo candidate", "candidate-baseline")
    } else {
        ("memo disabled", "memo enabled", "enabled-disabled")
    };

    for _ in 0..options.warmups {
        let _ = execute_incremental_sample(template, &fixture, baseline_memo, baseline_recording)?;
        let _ = execute_incremental_sample(template, &fixture, true, options.memo_recording)?;
        for (index, source) in fixture.revisions.iter().enumerate() {
            let _ = execute_cold_sample(template, source, RevisionId::new(index as u64 + 2))?;
        }
    }
    let timer_pair_floor_ns = instant_pair_floor_nanos();

    let edit_count = fixture.edits.len();
    let mut disabled = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut enabled = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut disabled_stages = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut enabled_stages = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut cold = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut paired_millis = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut disabled_priming = Vec::with_capacity(options.iterations);
    let mut enabled_priming = Vec::with_capacity(options.iterations);
    let mut paired_total_millis = Vec::with_capacity(options.iterations);
    let mut last_disabled = None;
    let mut last_enabled = None;
    let mut cold_reference = vec![None; edit_count];
    for iteration in 0..options.iterations {
        let order = if iteration % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        let mut pair = [None, None];
        for memo in order {
            let recording = if memo {
                options.memo_recording
            } else {
                baseline_recording
            };
            let sample =
                execute_incremental_sample(template, &fixture, memo || baseline_memo, recording)?;
            if memo {
                enabled_priming.push(sample.priming_elapsed);
            } else {
                disabled_priming.push(sample.priming_elapsed);
            }
            for (index, step) in sample.steps.iter().enumerate() {
                if memo {
                    enabled[index].push(step.elapsed);
                    enabled_stages[index].push(IncrementalStages::from_step(step));
                } else {
                    disabled[index].push(step.elapsed);
                    disabled_stages[index].push(IncrementalStages::from_step(step));
                }
            }
            pair[usize::from(memo)] = Some(
                sample
                    .steps
                    .iter()
                    .map(|step| step.elapsed)
                    .collect::<Vec<_>>(),
            );
            if memo {
                last_enabled = Some(sample);
            } else {
                last_disabled = Some(sample);
            }
        }
        for index in 0..edit_count {
            let disabled_elapsed = pair[0].as_ref().expect("disabled pair")[index];
            let enabled_elapsed = pair[1].as_ref().expect("enabled pair")[index];
            paired_millis[index]
                .push((enabled_elapsed.as_secs_f64() - disabled_elapsed.as_secs_f64()) * 1_000.0);
            let (elapsed, output) = execute_cold_sample(
                template,
                &fixture.revisions[index],
                RevisionId::new(index as u64 + 2),
            )?;
            cold[index].push(elapsed);
            cold_reference[index] = Some(output);
        }
        let disabled_sample = pair[0].as_ref().expect("disabled pair");
        let enabled_sample = pair[1].as_ref().expect("enabled pair");
        let disabled_total = disabled_priming.last().copied().unwrap_or_default()
            + disabled_sample.iter().copied().sum::<Duration>();
        let enabled_total = enabled_priming.last().copied().unwrap_or_default()
            + enabled_sample.iter().copied().sum::<Duration>();
        paired_total_millis
            .push((enabled_total.as_secs_f64() - disabled_total.as_secs_f64()) * 1_000.0);
    }

    let disabled_sample = last_disabled.expect("at least one disabled sample");
    let enabled_sample = last_enabled.expect("at least one enabled sample");
    for (index, cold_output) in cold_reference.iter().enumerate() {
        let cold_output = cold_output.as_ref().expect("at least one cold sample");
        let expected = cold_output.dvi_bytes().map_err(|error| error.to_string())?;
        for (name, sample) in [
            (baseline_name, &disabled_sample),
            (candidate_name, &enabled_sample),
        ] {
            if sample.steps[index].dvi != expected {
                let first = sample.steps[index]
                    .dvi
                    .iter()
                    .zip(expected.iter())
                    .position(|(left, right)| left != right);
                let page = first.map(|first| {
                    expected[..first]
                        .iter()
                        .filter(|&&byte| byte == 139)
                        .count()
                });
                return Err(format!(
                    "{name} incremental edit {} DVI differs from its cold DVI at {first:?}, approximate page {page:?} (incremental_len={}, cold_len={})",
                    index + 1,
                    sample.steps[index].dvi.len(),
                    expected.len(),
                ));
            }
        }
    }
    for (name, sample) in [
        (baseline_name, &disabled_sample),
        (candidate_name, &enabled_sample),
    ] {
        let fast_path = &sample.steps[fixture.suffix_adoption_edit];
        let previous = &sample.steps[fixture.suffix_adoption_edit - 1];
        if fast_path.dvi == previous.dvi || fast_path.pages != previous.pages {
            return Err(format!(
                "{name} equal-width edit did not change page content while preserving page count"
            ));
        }
        if fast_path.reuse.suffixes_adopted == 0 || fast_path.reuse.pages_reused == 0 {
            return Err(format!(
                "{name} height-preserving edit did not adopt a page suffix"
            ));
        }
        if fast_path.reuse.convergence_boundary.map(|key| key.boundary)
            != Some(EngineBoundary::ShipoutComplete)
        {
            return Err(format!(
                "{name} height-preserving edit did not reconverge at shipout"
            ));
        }
        if fast_path.reuse.pages_retyped != GENTLE_FAST_PATH_RETYPED_PAGES {
            return Err(format!(
                "{name} height-preserving edit re-shipped {} pages instead of the pinned {GENTLE_FAST_PATH_RETYPED_PAGES}",
                fast_path.reuse.pages_retyped,
            ));
        }
        if fast_path.reuse.pages_retained_prefix
            + fast_path.reuse.pages_retyped
            + fast_path.reuse.pages_reused
            != fast_path.pages
        {
            return Err(format!(
                "{name} height-preserving edit did not account for the complete retained prefix, changed pages, and adopted suffix"
            ));
        }
        if fast_path.reuse.trace_subtree_hits != 1
            || fast_path.reuse.trace_leaf_hits != fast_path.reuse.pages_reused
            || fast_path.reuse.trace_nodes_walked != fast_path.reuse.same_history_attempts
        {
            return Err(format!(
                "{name} height-preserving edit reported inconsistent trace replay telemetry"
            ));
        }
    }
    for index in 0..edit_count {
        let baseline = &disabled_sample.steps[index];
        let candidate = &enabled_sample.steps[index];
        if baseline.history != candidate.history {
            return Err(format!(
                "{} edit {} produced different baseline and candidate named-boundary schedules",
                fixture.edit_paths[index].name(),
                index + 1,
            ));
        }
        match fixture.edit_paths[index] {
            IncrementalPath::Slow => {
                if baseline.reuse.suffixes_adopted != 0
                    || candidate.reuse.suffixes_adopted != 0
                    || baseline.reuse.pages_reused != 0
                    || candidate.reuse.pages_reused != 0
                {
                    return Err(format!(
                        "slow edit {} unexpectedly adopted a page suffix",
                        index + 1,
                    ));
                }
            }
            IncrementalPath::Interaction | IncrementalPath::Fast => {
                let baseline_pages = (
                    baseline.reuse.pages_retained_prefix,
                    baseline.reuse.pages_retyped,
                    baseline.reuse.pages_reused,
                );
                let candidate_pages = (
                    candidate.reuse.pages_retained_prefix,
                    candidate.reuse.pages_retyped,
                    candidate.reuse.pages_reused,
                );
                if baseline.reuse.suffixes_adopted == 0
                    || candidate.reuse.suffixes_adopted == 0
                    || baseline_pages != candidate_pages
                {
                    return Err(format!(
                        "{} edit {} did not preserve equivalent suffix adoption: baseline={baseline_pages:?} candidate={candidate_pages:?}",
                        fixture.edit_paths[index].name(),
                        index + 1,
                    ));
                }
            }
        }
    }

    println!(
        "gentle-profile incremental edit: byte={} ({:.2}% through gentle.tex), inserted_bytes={} inserted_words={} into one paragraph; {} accepted edits/session; {} AB/BA-paired runs after {} warm-up(s); profiling_stats={}",
        fixture.body_offset,
        fixture.body_offset as f64 * 100.0 / fixture.body_len as f64,
        fixture.inserted_bytes,
        fixture.inserted_words,
        fixture.edits.len(),
        options.iterations,
        options.warmups,
        cfg!(feature = "profiling-stats"),
    );
    print_duration_stats(
        &format!("{baseline_name} priming"),
        duration_stats(&disabled_priming),
    );
    print_duration_stats(
        &format!("{candidate_name} priming"),
        duration_stats(&enabled_priming),
    );
    let total = scalar_stats(&paired_total_millis);
    println!(
        "gentle-profile baseline-inclusive paired delta: {delta_name} mean={:+.3}ms median={:+.3}ms min={:+.3}ms max={:+.3}ms",
        total.mean, total.median, total.min, total.max,
    );
    for path in IncrementalPath::ALL {
        let paired = (0..options.iterations)
            .map(|iteration| {
                fixture
                    .edit_paths
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| **candidate == path)
                    .map(|(index, _)| paired_millis[index][iteration])
                    .sum::<f64>()
            })
            .collect::<Vec<_>>();
        let stats = scalar_stats(&paired);
        println!(
            "gentle-profile path paired delta: path={} {delta_name} mean={:+.3}ms median={:+.3}ms min={:+.3}ms max={:+.3}ms",
            path.name(),
            stats.mean,
            stats.median,
            stats.min,
            stats.max,
        );
        if path == IncrementalPath::Slow {
            let priming_inclusive = paired
                .iter()
                .enumerate()
                .map(|(iteration, delta)| {
                    delta
                        + (enabled_priming[iteration].as_secs_f64()
                            - disabled_priming[iteration].as_secs_f64())
                            * 1_000.0
                })
                .collect::<Vec<_>>();
            let stats = scalar_stats(&priming_inclusive);
            println!(
                "gentle-profile path paired delta: path=slow-priming-inclusive {delta_name} mean={:+.3}ms median={:+.3}ms min={:+.3}ms max={:+.3}ms",
                stats.mean, stats.median, stats.min, stats.max,
            );
        }
    }
    print_paragraph_opportunities(
        baseline_name,
        "priming",
        disabled_sample.priming_memo.paragraph_opportunities,
    );
    print_paragraph_opportunities(
        candidate_name,
        "priming",
        enabled_sample.priming_memo.paragraph_opportunities,
    );
    for index in 0..edit_count {
        let disabled_stats = duration_stats(&disabled[index]);
        let enabled_stats = duration_stats(&enabled[index]);
        let cold_stats = duration_stats(&cold[index]);
        let paired = scalar_stats(&paired_millis[index]);
        println!(
            "gentle-profile accepted edit {}: path={} {}",
            index + 1,
            fixture.edit_paths[index].name(),
            fixture.edit_names[index]
        );
        print_duration_stats(baseline_name, disabled_stats);
        print_duration_stats(candidate_name, enabled_stats);
        print_duration_stats("cold", cold_stats);
        println!(
            "gentle-profile paired delta: edit={}: {delta_name} mean={:+.3}ms median={:+.3}ms min={:+.3}ms max={:+.3}ms",
            index + 1,
            paired.mean,
            paired.median,
            paired.min,
            paired.max,
        );
        print_stage_attribution(
            index + 1,
            baseline_name,
            candidate_name,
            delta_name,
            &disabled_stages[index],
            &enabled_stages[index],
        );
        print_history_comparison(
            index + 1,
            baseline_name,
            candidate_name,
            &disabled_sample.steps[index].history,
            &enabled_sample.steps[index].history,
        );
        print_incremental_work(
            baseline_name,
            index + 1,
            &disabled_sample.steps[index],
            timer_pair_floor_ns,
        );
        print_incremental_work(
            candidate_name,
            index + 1,
            &enabled_sample.steps[index],
            timer_pair_floor_ns,
        );
        println!(
            "gentle-profile incremental output: edit={}: {} pages, {} DVI bytes; both incremental modes are byte-identical to cold",
            index + 1,
            enabled_sample.steps[index].pages,
            enabled_sample.steps[index].dvi.len(),
        );
    }
    let fast = fixture.suffix_adoption_edit;
    let disabled_fast = duration_stats(&disabled[fast]);
    let enabled_fast = duration_stats(&enabled[fast]);
    let cold_fast = duration_stats(&cold[fast]);
    let work = disabled_sample.steps[fast].reuse;
    println!(
        "gentle-profile fast path verified: edit={} ({}) retained_prefix={} re-shipped={} adopted={} convergence=shipout leaf_hits={} subtree_hits={} baseline_vs_cold={:.3}x candidate_vs_cold={:.3}x",
        fixture.suffix_adoption_edit + 1,
        fixture.edit_names[fixture.suffix_adoption_edit],
        work.pages_retained_prefix,
        work.pages_retyped,
        work.pages_reused,
        work.trace_leaf_hits,
        work.trace_subtree_hits,
        disabled_fast.mean / cold_fast.mean,
        enabled_fast.mean / cold_fast.mean,
    );
    Ok(())
}

fn print_history_comparison(
    edit: usize,
    baseline_name: &str,
    candidate_name: &str,
    baseline: &[(BoundaryKey, usize, usize)],
    candidate: &[(BoundaryKey, usize, usize)],
) {
    let first_mismatch = baseline
        .iter()
        .zip(candidate)
        .position(|(left, right)| left != right)
        .or_else(|| {
            (baseline.len() != candidate.len()).then_some(baseline.len().min(candidate.len()))
        });
    let describe = |schedule: &[(BoundaryKey, usize, usize)]| {
        first_mismatch
            .and_then(|index| schedule.get(index))
            .copied()
    };
    println!(
        "gentle-profile boundary schedule: edit={edit} baseline={baseline_name:?} candidate={candidate_name:?} equivalent={} baseline_entries={} candidate_entries={} first_mismatch={first_mismatch:?} baseline_entry={:?} candidate_entry={:?}",
        first_mismatch.is_none(),
        baseline.len(),
        candidate.len(),
        describe(baseline),
        describe(candidate),
    );
}

#[derive(Clone, Copy)]
struct DurationStats {
    mean: f64,
    median: f64,
    min: f64,
    max: f64,
}

fn duration_stats(samples: &[Duration]) -> DurationStats {
    let millis = samples
        .iter()
        .map(|sample| sample.as_secs_f64() * 1_000.0)
        .collect::<Vec<_>>();
    scalar_stats(&millis)
}

fn scalar_stats(samples: &[f64]) -> DurationStats {
    let mut millis = samples.to_vec();
    millis.sort_by(f64::total_cmp);
    let mean = millis.iter().sum::<f64>() / millis.len() as f64;
    DurationStats {
        mean,
        median: millis[millis.len() / 2],
        min: millis[0],
        max: millis[millis.len() - 1],
    }
}

fn print_duration_stats(name: &str, stats: DurationStats) {
    println!(
        "gentle-profile incremental timing: {name}: mean={:.3}ms median={:.3}ms min={:.3}ms max={:.3}ms",
        stats.mean, stats.median, stats.min, stats.max,
    );
}

fn stage_mean(
    samples: &[IncrementalStages],
    select: impl Fn(IncrementalStages) -> Duration,
) -> f64 {
    samples
        .iter()
        .copied()
        .map(select)
        .map(|duration| duration.as_secs_f64() * 1_000.0)
        .sum::<f64>()
        / samples.len() as f64
}

fn print_stage_attribution(
    edit: usize,
    baseline_name: &str,
    candidate_name: &str,
    delta_name: &str,
    baseline: &[IncrementalStages],
    candidate: &[IncrementalStages],
) {
    macro_rules! stage {
        ($field:ident) => {{
            let baseline = stage_mean(baseline, |sample| sample.$field);
            let candidate = stage_mean(candidate, |sample| sample.$field);
            format!("{baseline:.3}/{candidate:.3}/{:+.3}", candidate - baseline)
        }};
    }
    println!(
        "gentle-profile stage attribution (baseline/candidate/delta ms): edit={edit} baseline={baseline_name:?} candidate={candidate_name:?} delta={delta_name:?} revision_setup={} restart_fork={} executor={} executor_shell={} diagnostics_effects_snapshot={} paragraph_history_publish_drop={} splice={} substrate_publish_drop={} acceptance={} unaccounted_system_noise={} dvi_materialization={}",
        stage!(revision_setup),
        stage!(restart_fork),
        stage!(executor),
        stage!(executor_shell),
        stage!(output_snapshot),
        stage!(generation_transition),
        stage!(splice),
        stage!(substrate_transition),
        stage!(acceptance),
        stage!(unaccounted),
        stage!(dvi_materialization),
    );
}

#[allow(clippy::disallowed_methods)] // Profiling-only timer-floor calibration.
fn instant_pair_floor_nanos() -> u64 {
    const SAMPLES: u32 = 20_000;
    let calibration_started = Instant::now();
    for _ in 0..SAMPLES {
        let started = black_box(Instant::now());
        let _ = black_box(started.elapsed());
    }
    u64::try_from(calibration_started.elapsed().as_nanos() / u128::from(SAMPLES))
        .unwrap_or(u64::MAX)
}

fn print_incremental_work(
    name: &str,
    edit: usize,
    sample: &IncrementalStep,
    _timer_pair_floor_ns: u64,
) {
    let reuse = sample.reuse;
    macro_rules! memo_delta {
        ($field:ident) => {
            sample
                .memo
                .$field
                .saturating_sub(sample.previous_memo.$field)
        };
    }
    println!(
        "gentle-profile incremental work: {name}: edit={edit} pages_retained_prefix={} pages_retyped={} pages_reused={} paragraphs_reexecuted={} bytes_reexecuted={} tokens_reexecuted={} commands_reexecuted={} macro_text_span_tokens={} source_text_span_tokens={} trace_nodes_walked={} trace_leaf_hits={} trace_subtree_hits={} trace_bytes={} exact_checks={} suffixes_adopted={} same_history_stop={:?} revision_setup_us={} fork_us={} executor_us={} reexecute_us={} diagnostics_effects_snapshot_us={} paragraph_history_publish_drop_us={} trace_validation_us={} trace_replay_us={} splice_us={} substrate_publish_drop_us={} acceptance_us={} dvi_materialization_us={}",
        reuse.pages_retained_prefix,
        reuse.pages_retyped,
        reuse.pages_reused,
        reuse.reexecuted_paragraphs,
        reuse.reexecuted_bytes,
        reuse.reexecuted_tokens,
        reuse.reexecuted_commands,
        reuse.reexecuted_macro_text_span_tokens,
        reuse.reexecuted_source_text_span_tokens,
        reuse.trace_nodes_walked,
        reuse.trace_leaf_hits,
        reuse.trace_subtree_hits,
        reuse.trace_retained_bytes,
        reuse.same_history_attempts,
        reuse.suffixes_adopted,
        reuse.same_history_stop,
        reuse.revision_setup_latency.as_micros(),
        reuse.restart_fork_latency.as_micros(),
        reuse.executor_latency.as_micros(),
        reuse.reexecution_latency.as_micros(),
        reuse.output_snapshot_latency.as_micros(),
        reuse.generation_transition_latency.as_micros(),
        reuse.trace_validation_latency.as_micros(),
        reuse.trace_replay_latency.as_micros(),
        reuse.splice_latency.as_micros(),
        reuse.substrate_transition_latency.as_micros(),
        reuse.acceptance_latency.as_micros(),
        sample.dvi_latency.as_micros(),
    );
    print_paragraph_opportunities(
        name,
        &format!("edit-{edit}"),
        sample
            .memo
            .paragraph_opportunities
            .saturating_since(sample.previous_memo.paragraph_opportunities),
    );
    #[cfg(feature = "profiling-stats")]
    println!(
        "gentle-profile exact identity: {name}: edit={edit} calls={} nanos={} projection_calls={} projection_visits={} projection_nanos={} root_cache_hits={} root_cache_misses={} dirty_leaves={}",
        sample.exact_identity.calls,
        sample.exact_identity.nanos,
        sample.exact_identity.projection_calls,
        sample.exact_identity.projection_visits,
        sample.exact_identity.projection_nanos,
        sample.exact_identity.root_cache_hits,
        sample.exact_identity.root_cache_misses,
        sample.exact_identity.dirty_leaves,
    );
    #[cfg(feature = "profiling-stats")]
    println!(
        "gentle-profile survivor work: {name}: edit={edit} fresh_promotions={} recycled_promotions={} releases={} shared_payload_drops={} promotion_nanos={} release_nanos={} shared_payload_drop_nanos={} source_words={} child_bearing_nodes={}",
        sample.survivor.fresh_promotions,
        sample.survivor.recycled_promotions,
        sample.survivor.releases_to_recycling,
        sample.survivor.shared_payload_drops,
        sample
            .survivor
            .fresh_promotion_nanos
            .saturating_add(sample.survivor.recycled_promotion_nanos),
        sample.survivor.release_nanos,
        sample.survivor.shared_payload_drop_nanos,
        sample.survivor.source_words,
        sample.survivor.child_bearing_nodes,
    );
    for (layer_name, layer) in [
        ("pretolerance", PureMemoLayer::Pretolerance),
        ("paragraph", PureMemoLayer::Paragraph),
        ("page", PureMemoLayer::Page),
        ("shipout", PureMemoLayer::Shipout),
    ] {
        let current = sample.memo.layer(layer);
        let previous = sample.previous_memo.layer(layer);
        print_memo_layer(name, edit, layer_name, current.saturating_since(previous));
    }
    println!(
        "gentle-profile paragraph detail: {name}: edit={edit} eligible={} barriers={} validation_misses={} import_failures={} line_hits={} hlist_fallbacks={} commands_skipped={} imported_bytes={} barrier_display_math={} barrier_scantokens={} barrier_input_open={} barrier_endinput={} barrier_world={} barrier_output={} barrier_unsupported_write={} barrier_unsupported_input_transition={} barrier_unsupported_group_transition={} validation_reasons={}",
        memo_delta!(paragraph_eligible_regions),
        memo_delta!(paragraph_barriers),
        memo_delta!(paragraph_validation_misses),
        memo_delta!(paragraph_import_failures),
        memo_delta!(paragraph_line_hits),
        memo_delta!(paragraph_hlist_fallbacks),
        memo_delta!(paragraph_commands_skipped),
        memo_delta!(paragraph_imported_bytes),
        memo_delta!(paragraph_display_math_barriers),
        memo_delta!(paragraph_scantokens_barriers),
        memo_delta!(paragraph_input_open_barriers),
        memo_delta!(paragraph_endinput_barriers),
        memo_delta!(paragraph_untracked_world_barriers),
        memo_delta!(paragraph_output_routine_barriers),
        memo_delta!(paragraph_unsupported_write_barriers),
        memo_delta!(paragraph_unsupported_input_transition_barriers),
        memo_delta!(paragraph_unsupported_group_transition_barriers),
        paragraph_validation_reasons(&sample.memo, &sample.previous_memo),
    );
    println!(
        "gentle-profile memo retention: {name}: edit={edit} detached_cache_bytes={} paragraph_history_metadata_bytes={}",
        sample.memo.retained_bytes, sample.memo.paragraph_history_metadata_bytes,
    );
    #[cfg(feature = "profiling-stats")]
    {
        let phases = sample
            .memo
            .paragraph_recording
            .saturating_since(sample.previous_memo.paragraph_recording);
        println!(
            "gentle-profile paragraph recording phases: {name}: edit={edit} timer_samples={} calibrated_timer_pair_floor_ns={} estimated_measurement_floor_ns={} trace_capture_ns={} front_end_dependency_ns={} input_transition_ns={} front_end_provenance_ns={} hlist_retention_ns={} region_publication_ns={} break_dependency_ns={} break_key_discovery_ns={} break_stamp_registration_ns={} break_value_projection_ns={} line_provenance_ns={} line_retention_ns={}",
            phases.timer_samples,
            _timer_pair_floor_ns,
            phases.timer_samples.saturating_mul(_timer_pair_floor_ns),
            phases.trace_capture_nanos,
            phases.front_end_dependency_nanos,
            phases.input_transition_nanos,
            phases.front_end_provenance_nanos,
            phases.hlist_retention_nanos,
            phases.region_publication_nanos,
            phases.break_dependency_nanos,
            phases.break_key_discovery_nanos,
            phases.break_stamp_registration_nanos,
            phases.break_value_projection_nanos,
            phases.line_provenance_nanos,
            phases.line_retention_nanos,
        );
    }
}

fn print_paragraph_opportunities(
    name: &str,
    stage: &str,
    stats: tex_state::ParagraphOpportunityStats,
) {
    let metric = |metric: tex_state::ParagraphOpportunityMetric| {
        format!("{}/{}/{}", metric.regions, metric.bytes, metric.nanos)
    };
    println!(
        "gentle-profile paragraph history: {name}: stage={stage} metric=regions/bytes/nanos carried_forward={} published={}",
        metric(stats.carried_forward),
        metric(stats.published),
    );
}

fn print_memo_layer(name: &str, edit: usize, layer: &str, stats: MemoLayerStats) {
    println!(
        "gentle-profile memo layer: {name}: edit={edit} layer={layer} lookups={} hits={} miss_not_attempted={} miss_ineligible_barrier={} miss_key={} miss_validation={} miss_evicted_before_reuse={} miss_import={} inserts={} evictions={} retained_bytes={} record_ns={} lookup_ns={} validation_ns={} import_ns={}",
        stats.lookups,
        stats.hits,
        stats.not_attempted,
        stats.ineligible_barriers,
        stats.key_misses,
        stats.validation_failures,
        stats.evicted_before_reuse,
        stats.import_failures,
        stats.inserts,
        stats.evictions,
        stats.retained_bytes,
        stats.record_nanos,
        stats.lookup_nanos,
        stats.validation_nanos,
        stats.import_nanos,
    );
}

fn paragraph_validation_reasons(current: &PureMemoStats, previous: &PureMemoStats) -> String {
    let reasons = [
        ParagraphValidationFailure::Meaning,
        ParagraphValidationFailure::Cell,
        ParagraphValidationFailure::Code,
        ParagraphValidationFailure::Font,
        ParagraphValidationFailure::Hyphenation,
        ParagraphValidationFailure::Input,
        ParagraphValidationFailure::Engine,
        ParagraphValidationFailure::Page,
        ParagraphValidationFailure::World,
        ParagraphValidationFailure::Query,
        ParagraphValidationFailure::Mutation,
        ParagraphValidationFailure::Effect,
        ParagraphValidationFailure::InputTransition,
        ParagraphValidationFailure::RetainedResult,
        ParagraphValidationFailure::BreakDependency,
    ];
    reasons
        .into_iter()
        .map(|reason| {
            let count = current
                .paragraph_validation_failure_count(reason)
                .saturating_sub(previous.paragraph_validation_failure_count(reason));
            format!("{reason:?}:{count}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[allow(clippy::disallowed_methods)] // Profiling setup reads the pinned corpus once.
fn incremental_fixture(repo_root: &Path) -> Result<IncrementalFixture, String> {
    let body_path = repo_root.join("third_party/corpus/gentle.tex");
    let body = fs::read_to_string(&body_path).map_err(|error| {
        format!(
            "read required input {}: {error}; run scripts/setup-conformance-tests.sh",
            body_path.display()
        )
    })?;
    let mut matches = body.match_indices(GENTLE_EDIT_OLD);
    let body_offset = matches
        .next()
        .map(|(offset, _)| offset)
        .ok_or_else(|| "the pinned Gentle edit paragraph was not found".to_owned())?;
    if matches.next().is_some() {
        return Err("the pinned Gentle edit paragraph is not unique".to_owned());
    }
    let prefix = "\\input plain.tex\n";
    let original = format!("{prefix}{body}");
    let start = prefix.len() + body_offset;
    let end = start + GENTLE_EDIT_OLD.len();
    let insertion = std::iter::repeat_n(GENTLE_EDIT_SENTENCE, GENTLE_EDIT_REPETITIONS)
        .collect::<Vec<_>>()
        .join(" ");
    let replacement = format!("{GENTLE_EDIT_OLD} {insertion}");
    let mut edited = original.clone();
    edited.replace_range(start..end, &replacement);
    let follow_up_start = start + replacement.len();
    let mut followed_up = edited.clone();
    followed_up.insert_str(follow_up_start, GENTLE_FOLLOW_UP);
    let edit_one = Edit {
        base_revision: RevisionId::new(1),
        expected_hash: ContentHash::from_bytes(original.as_bytes()),
        range: start..end,
        replacement,
    };
    let edit_two = Edit {
        base_revision: RevisionId::new(2),
        expected_hash: ContentHash::from_bytes(edited.as_bytes()),
        range: follow_up_start..follow_up_start,
        replacement: GENTLE_FOLLOW_UP.to_owned(),
    };
    let edit_three = Edit {
        base_revision: RevisionId::new(3),
        expected_hash: ContentHash::from_bytes(followed_up.as_bytes()),
        range: follow_up_start..follow_up_start + GENTLE_FOLLOW_UP.len(),
        replacement: String::new(),
    };
    // `words` and `sword` contain the same cmr10 glyphs, with the same `wo`
    // kern and no other kern or ligature pairs. Reordering the leading `s`
    // therefore changes the shipped page while preserving every line width.
    let equal_width_start = edited[start..]
        .find(GENTLE_EQUAL_WIDTH_OLD)
        .map(|offset| start + offset)
        .ok_or_else(|| "the equal-width edit word was not found in the insertion".to_owned())?;
    let equal_width_end = equal_width_start + GENTLE_EQUAL_WIDTH_OLD.len();
    let mut equal_width_edited = edited.clone();
    equal_width_edited.replace_range(equal_width_start..equal_width_end, GENTLE_EQUAL_WIDTH_NEW);
    let edit_four = Edit {
        base_revision: RevisionId::new(4),
        expected_hash: ContentHash::from_bytes(edited.as_bytes()),
        range: equal_width_start..equal_width_end,
        replacement: GENTLE_EQUAL_WIDTH_NEW.to_owned(),
    };
    Ok(IncrementalFixture {
        original,
        revisions: vec![edited.clone(), followed_up, edited, equal_width_edited],
        edits: vec![edit_one, edit_two, edit_three, edit_four],
        edit_names: vec![
            "large pagination-changing insertion",
            "follow-up insertion",
            "inverse removal",
            "height-preserving equal-width substitution",
        ],
        edit_paths: vec![
            IncrementalPath::Slow,
            IncrementalPath::Interaction,
            IncrementalPath::Slow,
            IncrementalPath::Fast,
        ],
        suffix_adoption_edit: 3,
        body_offset,
        body_len: body.len(),
        inserted_bytes: insertion.len() + 1,
        inserted_words: insertion.split_whitespace().count(),
    })
}

fn incremental_session(
    template: &World,
    source: &str,
    revision: RevisionId,
    memo: bool,
    recording: PureMemoRecordingPolicy,
) -> Result<Session, String> {
    let mut stores = Universe::with_world(template.clone());
    prepare_run_stores(&mut stores);
    if memo {
        stores.enable_pure_memo(PureMemoConfig {
            recording,
            ..PureMemoConfig::default()
        });
    }
    Session::start_with_source_path(
        stores,
        "gentle-profile",
        Path::new(JOB_DIR).join(JOB_FILE).to_string_lossy(),
        revision,
        source,
        usize::MAX,
    )
    .map_err(|error| error.to_string())
}

#[allow(clippy::disallowed_methods)] // Host-side benchmark timer; no engine fact observes it.
fn execute_incremental_sample(
    template: &World,
    fixture: &IncrementalFixture,
    memo: bool,
    recording: PureMemoRecordingPolicy,
) -> Result<IncrementalSample, String> {
    let path = Path::new(JOB_DIR).join(JOB_FILE);
    let mut session = incremental_session(
        template,
        &fixture.original,
        RevisionId::new(1),
        memo,
        recording,
    )?;
    let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
    let priming_started = Instant::now();
    let (input, font) = resolvers.resolvers();
    session
        .cold_with_resolvers(input, font)
        .map_err(|error| format!("prepare incremental baseline: {error}"))?;
    let priming_elapsed = priming_started.elapsed();
    let priming_memo = session.pure_memo_stats();
    let mut steps = Vec::with_capacity(fixture.edits.len());
    for (index, edit) in fixture.edits.iter().enumerate() {
        let previous_memo = session.pure_memo_stats();
        #[cfg(feature = "profiling-stats")]
        let exact_before = exact_identity_measurement();
        #[cfg(feature = "profiling-stats")]
        let survivor_before = survivor_measurement();
        let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
        let started = Instant::now();
        let (input, font) = resolvers.resolvers();
        let accepted = session
            .advance_with_resolvers(RevisionId::new(index as u64 + 2), edit.clone(), input, font)
            .map_err(|error| format!("advance incremental edit {}: {error}", index + 1))?;
        let elapsed = started.elapsed();
        let memo = session.pure_memo_stats();
        #[cfg(feature = "profiling-stats")]
        let exact_after = exact_identity_measurement();
        #[cfg(feature = "profiling-stats")]
        let survivor_after = survivor_measurement();
        let dvi_started = Instant::now();
        let dvi = accepted.dvi_bytes().map_err(|error| error.to_string())?;
        let dvi_latency = dvi_started.elapsed();
        let _ = black_box(accepted.artifacts.len());
        steps.push(IncrementalStep {
            elapsed,
            dvi_latency,
            dvi,
            pages: accepted.artifacts.len(),
            reuse: accepted.reuse,
            history: accepted
                .history
                .iter()
                .map(|record| {
                    (
                        record.key(),
                        record.effect_prefix(),
                        record.artifact_prefix(),
                    )
                })
                .collect(),
            memo,
            previous_memo,
            #[cfg(feature = "profiling-stats")]
            exact_identity: ExactIdentityMeasurement {
                calls: exact_after.calls.saturating_sub(exact_before.calls),
                nanos: exact_after.nanos.saturating_sub(exact_before.nanos),
                projection_calls: exact_after
                    .projection_calls
                    .saturating_sub(exact_before.projection_calls),
                projection_visits: exact_after
                    .projection_visits
                    .saturating_sub(exact_before.projection_visits),
                projection_nanos: exact_after
                    .projection_nanos
                    .saturating_sub(exact_before.projection_nanos),
                root_cache_hits: exact_after
                    .root_cache_hits
                    .saturating_sub(exact_before.root_cache_hits),
                root_cache_misses: exact_after
                    .root_cache_misses
                    .saturating_sub(exact_before.root_cache_misses),
                dirty_leaves: exact_after
                    .dirty_leaves
                    .saturating_sub(exact_before.dirty_leaves),
            },
            #[cfg(feature = "profiling-stats")]
            survivor: survivor_delta(survivor_after, survivor_before),
        });
    }
    Ok(IncrementalSample {
        priming_elapsed,
        priming_memo,
        steps,
    })
}

#[cfg(feature = "profiling-stats")]
fn survivor_delta(after: SurvivorMeasurement, before: SurvivorMeasurement) -> SurvivorMeasurement {
    SurvivorMeasurement {
        fresh_promotions: after
            .fresh_promotions
            .saturating_sub(before.fresh_promotions),
        fresh_promotion_nanos: after
            .fresh_promotion_nanos
            .saturating_sub(before.fresh_promotion_nanos),
        recycled_promotions: after
            .recycled_promotions
            .saturating_sub(before.recycled_promotions),
        recycled_promotion_nanos: after
            .recycled_promotion_nanos
            .saturating_sub(before.recycled_promotion_nanos),
        releases_to_recycling: after
            .releases_to_recycling
            .saturating_sub(before.releases_to_recycling),
        release_nanos: after.release_nanos.saturating_sub(before.release_nanos),
        shared_payload_drops: after
            .shared_payload_drops
            .saturating_sub(before.shared_payload_drops),
        shared_payload_drop_nanos: after
            .shared_payload_drop_nanos
            .saturating_sub(before.shared_payload_drop_nanos),
        source_words: after.source_words.saturating_sub(before.source_words),
        child_bearing_nodes: after
            .child_bearing_nodes
            .saturating_sub(before.child_bearing_nodes),
        remap_entries: after.remap_entries.saturating_sub(before.remap_entries),
        pending_entries: after.pending_entries.saturating_sub(before.pending_entries),
        peak_promotion_scratch_logical_bytes: after.peak_promotion_scratch_logical_bytes,
        peak_promotion_scratch_retained_bytes: after.peak_promotion_scratch_retained_bytes,
        peak_remap_entries: after.peak_remap_entries,
        peak_pending_entries: after.peak_pending_entries,
    }
}

#[allow(clippy::disallowed_methods)] // Host-side benchmark timer; no engine fact observes it.
fn execute_cold_sample(
    template: &World,
    source: &str,
    revision: RevisionId,
) -> Result<(Duration, AcceptedOutput), String> {
    let path = Path::new(JOB_DIR).join(JOB_FILE);
    let mut session = incremental_session(
        template,
        source,
        revision,
        false,
        PureMemoRecordingPolicy::default(),
    )?;
    let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
    let started = Instant::now();
    let (input, font) = resolvers.resolvers();
    let accepted = session
        .cold_with_resolvers(input, font)
        .map_err(|error| format!("compile cold edited document: {error}"))?;
    let elapsed = started.elapsed();
    let _ = black_box(accepted.artifacts.len());
    Ok((elapsed, accepted))
}

fn load_template(repo_root: &Path) -> Result<World, String> {
    let corpus = repo_root.join("third_party/corpus");
    let mut world = World::memory_with_clock(JobClock {
        time: 13 * 60 + 36,
        second: 0,
        day: 9,
        month: 7,
        year: 2026,
    });
    seed_file(&mut world, &corpus.join("plain.tex"), "plain.tex")?;
    seed_file(&mut world, &corpus.join("gentle.tex"), "gentle.tex")?;
    seed_file(
        &mut world,
        &repo_root.join("third_party/hyphen/hyphen.tex"),
        "hyphen.tex",
    )?;
    seed_font_dir(&mut world, &repo_root.join("third_party/fonts"))?;
    seed_font_dir(
        &mut world,
        &repo_root.join("crates/tex-fonts/tests/fixtures/cm"),
    )?;
    world
        .set_memory_file(
            Path::new(JOB_DIR).join(JOB_FILE),
            b"\\input plain.tex\n\\input gentle.tex\n".to_vec(),
        )
        .map_err(|error| error.to_string())?;
    Ok(world)
}

#[allow(clippy::disallowed_methods)] // Profiling setup reads host inputs once before the run loop.
fn seed_file(world: &mut World, source: &Path, name: &str) -> Result<(), String> {
    let bytes = fs::read(source).map_err(|error| {
        format!(
            "read required input {}: {error}; run scripts/setup-conformance-tests.sh",
            source.display()
        )
    })?;
    world
        .set_memory_file(Path::new(JOB_DIR).join(name), bytes)
        .map_err(|error| error.to_string())
}

#[allow(clippy::disallowed_methods)] // Profiling setup enumerates and reads host fonts once.
fn seed_font_dir(world: &mut World, dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut paths = fs::read_dir(dir)
        .map_err(|error| format!("read font directory {}: {error}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read font directory entry: {error}"))?;
    paths.sort();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some("tfm") {
            continue;
        }
        let name = path
            .file_name()
            .ok_or_else(|| format!("font path has no file name: {}", path.display()))?;
        let bytes = fs::read(&path)
            .map_err(|error| format!("read font metric {}: {error}", path.display()))?;
        world
            .set_memory_file(Path::new(JOB_DIR).join(name), bytes)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn execute_once(template: &World, capture_checkpoints: bool) -> Result<RunOutput, String> {
    let mut stores = Universe::with_world(template.clone());
    prepare_run_stores(&mut stores);
    let path = Path::new(JOB_DIR).join(JOB_FILE);
    let content = stores
        .world_mut()
        .read_file(&path)
        .map_err(|error| error.to_string())?;
    let mut input = InputStack::new(WorldInput::from_content(content));
    let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
    let mut checkpoints = ProfileCheckpointSink::default();
    let context = resolvers.context();
    let mut session = EngineSession::new(&mut input, &mut stores, context);
    let run = if capture_checkpoints {
        session.execute_with_checkpoints(&mut checkpoints)
    } else {
        session.execute()
    };
    let run = match run {
        Ok(run) => run,
        Err(error) => return Err(error.format_with_provenance(session.stores())),
    };
    if run.artifacts.is_empty() {
        return Err("Gentle produced no page artifacts".to_owned());
    }
    let dvi = dvi_from_page_plans(&run.dvi_pages).map_err(|error| error.to_string())?;
    Ok(RunOutput {
        dvi,
        pages: run.artifacts.len(),
        checkpoints: checkpoints.count,
        checkpoint_hash: checkpoints.hash,
        #[cfg(feature = "profiling-stats")]
        expansion_stats: input.expansion_stats(),
    })
}

fn print_summary(options: &Options, output: &RunOutput, elapsed: Duration) {
    let mean = elapsed.as_secs_f64() * 1_000.0 / options.iterations as f64;
    println!(
        "gentle-profile: {} measured runs after {} warm-up(s): {:.3}s total, {:.3}ms mean; {} pages, {} DVI bytes, {} checkpoints",
        options.iterations,
        options.warmups,
        elapsed.as_secs_f64(),
        mean,
        output.pages,
        output.dvi.len(),
        output.checkpoints
    );
    #[cfg(feature = "profiling-stats")]
    println!(
        "gentle-profile expansion: token_frame_steps={} provenance_resolutions={} character_tokens={} character_fraction={:.6} meaning_lookups={} meaning_cache_hits={} meaning_cache_misses={} literal_spans={} literal_tokens={} mean_literal_run={:.6} segmentation_cache_hits={} segmentation_cache_misses={} builder_appends={} source_text_span_attempts={} source_text_spans={} source_text_tokens={} mean_source_text_run={:.6}",
        output.expansion_stats.token_frame_steps,
        output.expansion_stats.provenance_resolutions,
        output.expansion_stats.character_tokens,
        output.expansion_stats.character_fraction(),
        output.expansion_stats.meaning_lookups,
        output.expansion_stats.meaning_cache_hits,
        output.expansion_stats.meaning_cache_misses,
        output.expansion_stats.literal_spans,
        output.expansion_stats.literal_tokens,
        output.expansion_stats.mean_literal_run(),
        output.expansion_stats.segmentation_cache_hits,
        output.expansion_stats.segmentation_cache_misses,
        output.expansion_stats.builder_appends,
        output.expansion_stats.source_text_span_attempts,
        output.expansion_stats.source_text_spans,
        output.expansion_stats.source_text_tokens,
        output.expansion_stats.mean_source_text_run(),
    );
    #[cfg(feature = "profiling-stats")]
    println!(
        "gentle-profile expansion timers (ns): frame_step={} frame_step_samples={} provenance={} provenance_samples={} classification_meaning={} classification_meaning_samples={} builder_append={} builder_append_samples={} attributed_total={}",
        output.expansion_stats.frame_step_nanos,
        output.expansion_stats.frame_step_timer_samples,
        output.expansion_stats.provenance_nanos,
        output.expansion_stats.provenance_timer_samples,
        output.expansion_stats.classification_meaning_nanos,
        output.expansion_stats.classification_meaning_timer_samples,
        output.expansion_stats.builder_append_nanos,
        output.expansion_stats.builder_append_timer_samples,
        output.expansion_stats.attributed_nanos(),
    );
    #[cfg(feature = "profiling-stats")]
    {
        let invalidations = tex_state::measurement::meaning_cache_invalidation_measurement();
        println!(
            "gentle-profile meaning invalidations: local_writes={} global_writes={} group_exits={} rollbacks={}",
            invalidations.local_writes,
            invalidations.global_writes,
            invalidations.group_exits,
            invalidations.rollbacks,
        );
    }
}

fn next_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn parse_positive_count(value: &str, option: &str) -> Result<usize, String> {
    let value = value
        .parse::<usize>()
        .map_err(|_| format!("{option} requires a positive integer, got {value:?}"))?;
    if value == 0 {
        return Err(format!("{option} must be greater than zero"));
    }
    Ok(value)
}

fn parse_memo_layers(value: &str) -> Result<PureMemoRecordingPolicy, String> {
    if value == "all" {
        return Ok(PureMemoRecordingPolicy::all());
    }
    let mut policy = PureMemoRecordingPolicy {
        pretolerance: false,
        paragraphs: false,
        pages: false,
        shipouts: false,
    };
    for layer in value.split(',') {
        match layer {
            "pretolerance" => policy.pretolerance = true,
            "paragraph" => policy.paragraphs = true,
            "page" => policy.pages = true,
            "shipout" => policy.shipouts = true,
            "none" if value == "none" => {}
            _ => {
                return Err(format!(
                    "--memo-layers expects comma-separated pretolerance,paragraph,page,shipout, got {value:?}"
                ));
            }
        }
    }
    Ok(policy)
}

fn print_help() {
    println!(
        "Usage: gentle-profile [--iterations N] [--warmups N] [--repo-root PATH] [--checkpoints] [--incremental-edit] [--baseline-memo-layers LIST] [--memo-layers LIST]\n\n\
         Loads Gentle and its support files once, then executes fresh deterministic\n\
         in-memory Umber sessions for profiling. Defaults: {DEFAULT_ITERATIONS} measured\n\
         iterations and {DEFAULT_WARMUPS} warm-up. --checkpoints captures and hashes every\n\
         named executor checkpoint through a bounded profiling sink.\n\
         --incremental-edit compares a memo baseline, memo candidate, and cold compilation\n\
         four accepted edits/session using balanced AB/BA pairs and DVI parity verification.\n\
         --memo-layers configures enabled recording layers; the default is paragraph.\n\
         --baseline-memo-layers replaces the disabled control with an explicit recording\n\
         policy for direct marginal layer comparisons."
    );
}
