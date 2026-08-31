//! Persistent in-process Gentle profiling workload.

use std::env;
use std::fs;
use std::hint::black_box;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tex_command::SourceRegistration;
#[cfg(feature = "profiling")]
use tex_exec::{AlignmentTemplateMeasurement, alignment_template_measurement};
use tex_exec::{Cancellation, CheckpointSink, EngineCheckpoint};
use tex_incr::{
    AcceptedOutput, BoundaryKey, Edit, ReuseMetrics, RevisionCandidateResult, RevisionId,
    SameHistoryStop, Session, new_reachability_store,
};
use tex_state::{ContentHash, JobClock, PureMemoRecordingPolicy, PureMemoStats, World};
use tex_state::{MemoLayerStats, PureMemoLayer};
#[cfg(feature = "profiling")]
use umber::ExpansionStats;
use umber::{EngineSession, FileSessionResolvers, dvi_from_page_plans};

#[cfg(feature = "profiling")]
#[global_allocator]
static HOT_CORE_ALLOCATOR: tex_state::measurement::HotCoreAllocator =
    tex_state::measurement::HotCoreAllocator;

const JOB_DIR: &str = "/gentle-profile";
const JOB_FILE: &str = "profile-job.tex";
const DEFAULT_ITERATIONS: usize = 50;
const DEFAULT_WARMUPS: usize = 1;
/// Match the native editor's default retained-input/checkpoint-root budget.
/// An unlimited benchmark history prevents owner-local journal reclamation
/// and measures an unsupported retention policy instead of editor restart.
const PROFILING_CHECKPOINT_BUDGET: usize = 64 * 1024 * 1024;
const EDIT_RESTART_LATENCY_RATIO_LIMIT: f64 = 1.25;
const SUFFIX_FAST_RATIO_LIMIT: f64 = 0.10;
const LONG_BEFORE_SHA256: &str = "4f666b5cae4caf3443cf621103994c97aad20d534ab5fc34f7ab8c380ae8b24c";
const LONG_AFTER_SHA256: &str = "2b482038cfc1bf12112ecd8a7f36107e39f4cf5516416569f5c96bcaca022b37";
const GENTLE_EDIT_OLD: &str = "There are ten characters which, like the backslash, are used";
const GENTLE_EDIT_SENTENCE: &str = "This deliberately extended explanation adds ordinary words to the same paragraph so that TeX must reconsider many line breaks and carry the resulting vertical material across page boundaries.";
const GENTLE_EDIT_REPETITIONS: usize = 64;
const GENTLE_FOLLOW_UP: &str = " A measured follow-up changes this paragraph again.";
const GENTLE_EQUAL_WIDTH_OLD: &str = "words";
const GENTLE_EQUAL_WIDTH_NEW: &str = "sword";
const GENTLE_REBREAK_ASSIGNMENT: &str = "\\tolerance=201 ";
const GENTLE_FAST_PATH_RETYPED_PAGES: usize = 3;

#[derive(Debug)]
struct Options {
    repo_root: PathBuf,
    iterations: usize,
    warmups: usize,
    checkpoints: bool,
    incremental_edit: bool,
    incremental_path: Option<IncrementalPath>,
    edit_restart_workload: Option<EditRestartWorkload>,
    cold_memo_policy: Option<ColdMemoPolicy>,
    baseline_memo_recording: Option<PureMemoRecordingPolicy>,
    memo_recording: PureMemoRecordingPolicy,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut repo_root = None;
        let mut iterations = DEFAULT_ITERATIONS;
        let mut warmups = DEFAULT_WARMUPS;
        let mut checkpoints = false;
        let mut incremental_edit = false;
        let mut incremental_path = None;
        let mut edit_restart_workload = None;
        let mut cold_memo_policy = None;
        let mut baseline_memo_recording = None;
        let mut memo_recording = PureMemoRecordingPolicy::default();
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--repo-root" => {
                    repo_root = Some(PathBuf::from(next_value(&mut args, "--repo-root")?));
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
                "--incremental-path" => {
                    incremental_path = Some(parse_incremental_path(&next_value(
                        &mut args,
                        "--incremental-path",
                    )?)?);
                }
                "--edit-restart-workload" => {
                    edit_restart_workload = Some(parse_edit_restart_workload(&next_value(
                        &mut args,
                        "--edit-restart-workload",
                    )?)?);
                }
                "--cold-memo-layers" => {
                    cold_memo_policy = Some(parse_cold_memo_policy(&next_value(
                        &mut args,
                        "--cold-memo-layers",
                    )?)?);
                }
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
        let repo_root = match repo_root {
            Some(repo_root) => repo_root,
            None => {
                let current = env::current_dir()
                    .map_err(|error| format!("determine current directory: {error}"))?;
                test_support::repository_root_at(&current).map_err(|error| {
                    format!("resolve repository root {}: {error:#}", current.display())
                })?
            }
        };
        let repo_root = repo_root
            .canonicalize()
            .map_err(|error| format!("resolve repository root {}: {error}", repo_root.display()))?;
        Ok(Some(Self {
            repo_root,
            iterations,
            warmups,
            checkpoints,
            incremental_edit,
            incremental_path,
            edit_restart_workload,
            cold_memo_policy,
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
    #[cfg(feature = "profiling")]
    expansion_stats: ExpansionStats,
}

struct ProfileTemplate {
    world: World,
    inputs: Vec<(PathBuf, Arc<[u8]>)>,
}

impl ProfileTemplate {
    fn register_inputs(&self, session: &mut Session<'_>) -> Result<(), String> {
        session.set_job_clock(self.world.job_clock());
        for (path, bytes) in &self.inputs {
            session
                .register_input_file(path, Arc::clone(bytes))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct ProfileCheckpointSink {
    enabled: bool,
    count: usize,
    hash: u64,
}

impl<G> CheckpointSink<G> for ProfileCheckpointSink {
    fn wants_checkpoint(&self, _boundary: tex_exec::EngineBoundary) -> bool {
        self.enabled
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint<G>) {
        self.count += 1;
        let identity = checkpoint.reachable_state_identity().map_or_else(
            || {
                u64::try_from(checkpoint.root_anchor()).unwrap_or(u64::MAX)
                    ^ u64::try_from(checkpoint.effect_prefix_len())
                        .unwrap_or(u64::MAX)
                        .rotate_left(19)
                    ^ u64::try_from(checkpoint.artifact_prefix_len())
                        .unwrap_or(u64::MAX)
                        .rotate_left(37)
            },
            |identity| identity.fingerprint(),
        );
        self.hash = self.hash.rotate_left(7) ^ identity;
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

    if let Some(policy) = options.cold_memo_policy {
        return run_cold_memo_policy(&options, &template, policy);
    }
    if let Some(path) = options.incremental_path {
        return run_incremental_path(&options, &template, path);
    }
    if let Some(workload) = options.edit_restart_workload {
        return run_edit_restart_workload(&options, &template, workload);
    }
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

    #[cfg(feature = "profiling")]
    let alignment_template_before = alignment_template_measurement();
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
    #[cfg(feature = "profiling")]
    {
        let templates =
            alignment_template_delta(alignment_template_measurement(), alignment_template_before);
        println!(
            "gentle-profile alignment u-templates: invocations={} delivered_tokens={} character_tokens={} control_sequence_tokens={} relax_commands={} font_commands={} unexpandable_commands={} inert_glue_commands={} other_commands={}",
            templates.invocations,
            templates.delivered_tokens,
            templates.character_tokens,
            templates.control_sequence_tokens,
            templates.relax_commands,
            templates.font_commands,
            templates.unexpandable_commands,
            templates.inert_glue_commands,
            templates.other_commands,
        );
    }
    Ok(())
}

#[cfg(feature = "profiling")]
fn alignment_template_delta(
    after: AlignmentTemplateMeasurement,
    before: AlignmentTemplateMeasurement,
) -> AlignmentTemplateMeasurement {
    AlignmentTemplateMeasurement {
        invocations: after.invocations.saturating_sub(before.invocations),
        delivered_tokens: after
            .delivered_tokens
            .saturating_sub(before.delivered_tokens),
        character_tokens: after
            .character_tokens
            .saturating_sub(before.character_tokens),
        control_sequence_tokens: after
            .control_sequence_tokens
            .saturating_sub(before.control_sequence_tokens),
        relax_commands: after.relax_commands.saturating_sub(before.relax_commands),
        font_commands: after.font_commands.saturating_sub(before.font_commands),
        unexpandable_commands: after
            .unexpandable_commands
            .saturating_sub(before.unexpandable_commands),
        inert_glue_commands: after
            .inert_glue_commands
            .saturating_sub(before.inert_glue_commands),
        other_commands: after.other_commands.saturating_sub(before.other_commands),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColdMemoPolicy {
    Disabled,
    Enabled(PureMemoRecordingPolicy),
}

impl ColdMemoPolicy {
    fn config(self) -> (bool, PureMemoRecordingPolicy) {
        match self {
            Self::Disabled => (false, PureMemoRecordingPolicy::default()),
            Self::Enabled(recording) => (true, recording),
        }
    }
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
    Neutral,
    Rebreak,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditRestartWorkload {
    Unchanged,
    Prefix,
    Suffix,
    DisplayMath,
    Macro,
    Conditional,
    Long,
}

impl EditRestartWorkload {
    const fn name(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Prefix => "prefix",
            Self::Suffix => "suffix",
            Self::DisplayMath => "display-math",
            Self::Macro => "macro",
            Self::Conditional => "conditional",
            Self::Long => "long",
        }
    }
}

impl IncrementalPath {
    const ALL: [Self; 4] = [Self::Slow, Self::Interaction, Self::Fast, Self::Rebreak];

    const fn name(self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Interaction => "interaction",
            Self::Fast => "fast",
            Self::Neutral => "neutral",
            Self::Rebreak => "rebreak",
        }
    }
}

struct IncrementalSample {
    priming_elapsed: Duration,
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
}

#[derive(Clone, Copy, Default)]
struct IncrementalStages {
    revision_setup: Duration,
    restart_fork: Duration,
    executor: Duration,
    executor_shell: Duration,
    output_snapshot: Duration,
    splice: Duration,
    substrate_transition: Duration,
    acceptance: Duration,
    unaccounted: Duration,
    dvi_materialization: Duration,
}

#[derive(Clone, Copy)]
struct EditRestartSample {
    elapsed: Duration,
    target_cold: Duration,
    reuse: ReuseMetrics,
}

impl IncrementalStages {
    fn from_step(step: &IncrementalStep) -> Self {
        Self::from_reuse(step.elapsed, step.dvi_latency, step.reuse)
    }

    fn from_reuse(elapsed: Duration, dvi_latency: Duration, reuse: ReuseMetrics) -> Self {
        let executor_shell = reuse
            .reexecution_latency
            .saturating_sub(reuse.executor_latency);
        let accounted = reuse
            .revision_setup_latency
            .saturating_add(reuse.restart_fork_latency)
            .saturating_add(reuse.reexecution_latency)
            .saturating_add(reuse.output_snapshot_latency)
            .saturating_add(reuse.splice_latency)
            .saturating_add(reuse.substrate_transition_latency)
            .saturating_add(reuse.acceptance_latency);
        Self {
            revision_setup: reuse.revision_setup_latency,
            restart_fork: reuse.restart_fork_latency,
            executor: reuse.executor_latency,
            executor_shell,
            output_snapshot: reuse.output_snapshot_latency,
            splice: reuse.splice_latency,
            substrate_transition: reuse.substrate_transition_latency,
            acceptance: reuse.acceptance_latency,
            unaccounted: elapsed.saturating_sub(accounted),
            dvi_materialization: dvi_latency,
        }
    }
}

#[allow(clippy::disallowed_methods)] // Host-side cold-policy profiling timer.
fn run_cold_memo_policy(
    options: &Options,
    template: &ProfileTemplate,
    policy: ColdMemoPolicy,
) -> Result<(), String> {
    if options.checkpoints
        || options.incremental_edit
        || options.incremental_path.is_some()
        || options.edit_restart_workload.is_some()
    {
        return Err("--cold-memo-layers cannot be combined with another workload".to_owned());
    }
    let fixture = incremental_fixture(&options.repo_root)?;
    let source_path = Path::new(JOB_DIR).join(JOB_FILE);
    let (memo, recording) = policy.config();
    let total_runs = options.warmups.saturating_add(options.iterations);
    let mut durations = Vec::with_capacity(options.iterations);
    let (_, cold_reference) = execute_cold_sample(template, &fixture.original, RevisionId::new(1))?;
    let reference_dvi = cold_reference
        .dvi_bytes()
        .map_err(|error| error.to_string())?;
    let mut last_pages = 0;
    let mut last_memo = PureMemoStats::default();
    for run in 0..total_runs {
        let reachability_store = new_reachability_store();
        let mut session = incremental_session(
            &reachability_store,
            template,
            &fixture.original,
            RevisionId::new(1),
            memo,
            recording,
        )?;
        let mut resolvers = FileSessionResolvers::new(&source_path, Vec::new(), Vec::new());
        let started = Instant::now();
        let accepted = session
            .cold_with_resolvers(&mut resolvers)
            .map_err(|error| format!("cold memo-policy run {}: {error}", run + 1))?;
        let elapsed = started.elapsed();
        let dvi = accepted.dvi_bytes().map_err(|error| error.to_string())?;
        if reference_dvi != dvi {
            return Err(format!(
                "cold memo-policy run {} differs from memo-disabled cold output",
                run + 1
            ));
        }
        if run >= options.warmups {
            durations.push(elapsed);
        }
        last_pages = accepted.pages().len();
        last_memo = session.pure_memo_stats();
        let _ = black_box(last_pages);
        let _ = black_box(dvi.len());
    }

    let name = match policy {
        ColdMemoPolicy::Disabled => "disabled",
        ColdMemoPolicy::Enabled(_) => "enabled",
    };
    println!(
        "gentle-profile isolated cold: memo={name} recording={recording:?} measured_runs={} warmup_runs={}",
        options.iterations, options.warmups
    );
    print_duration_stats("isolated cold", duration_stats(&durations));
    println!(
        "gentle-profile isolated cold output: pages={} dvi_bytes={} memo_bytes={}",
        last_pages,
        reference_dvi.len(),
        last_memo.retained_bytes,
    );
    Ok(())
}

fn run_incremental_path(
    options: &Options,
    template: &ProfileTemplate,
    path_kind: IncrementalPath,
) -> Result<(), String> {
    if options.checkpoints || options.incremental_edit {
        return Err("--incremental-path cannot be combined with another workload".to_owned());
    }
    let fixture = incremental_fixture(&options.repo_root)?;
    let mut neutral = fixture.original.clone();
    neutral.insert_str("\\input plain.tex\n".len(), "% neutral editor comment\n");
    let (left, right) = match path_kind {
        IncrementalPath::Slow => (fixture.original.as_str(), fixture.revisions[0].as_str()),
        IncrementalPath::Fast => (fixture.revisions[2].as_str(), fixture.revisions[3].as_str()),
        IncrementalPath::Neutral => (fixture.original.as_str(), neutral.as_str()),
        IncrementalPath::Interaction | IncrementalPath::Rebreak => {
            return Err(
                "--incremental-path currently accepts only fast, slow, or neutral".to_owned(),
            );
        }
    };
    let source_path = Path::new(JOB_DIR).join(JOB_FILE);
    let reachability_store = new_reachability_store();
    let mut session = incremental_session(
        &reachability_store,
        template,
        left,
        RevisionId::new(1),
        true,
        options.memo_recording,
    )?;
    let mut resolvers = FileSessionResolvers::new(&source_path, Vec::new(), Vec::new());
    let initial = session
        .cold_with_resolvers(&mut resolvers)
        .map_err(|error| format!("prepare isolated {} path: {error}", path_kind.name()))?;
    let left_dvi = initial.dvi_bytes().map_err(|error| error.to_string())?;
    let (_, right_cold) = execute_cold_sample(template, right, RevisionId::new(1))?;
    let right_dvi = right_cold.dvi_bytes().map_err(|error| error.to_string())?;

    let mut revision = 1_u64;
    let mut on_left = true;
    let total_steps = options.warmups + options.iterations;
    let mut durations = Vec::with_capacity(options.iterations);
    let mut stages = Vec::with_capacity(options.iterations);
    let mut last_reuse = ReuseMetrics::default();
    for step_index in 0..total_steps {
        let (from, to, expected_dvi) = if on_left {
            (left, right, right_dvi.as_slice())
        } else {
            (right, left, left_dvi.as_slice())
        };
        debug_assert_eq!(session.source(), from);
        revision += 1;
        let edit = replacement_edit(from, to, session.revision(), session.content_hash());
        let mut resolvers = FileSessionResolvers::new(&source_path, Vec::new(), Vec::new());
        let started = Instant::now();
        let accepted = session
            .advance_with_resolvers(RevisionId::new(revision), edit, &mut resolvers)
            .map_err(|error| {
                format!(
                    "advance isolated {} path step {}: {error}",
                    path_kind.name(),
                    step_index + 1,
                )
            })?;
        let elapsed = started.elapsed();
        let dvi_started = Instant::now();
        let dvi = accepted.dvi_bytes().map_err(|error| error.to_string())?;
        let dvi_latency = dvi_started.elapsed();
        if dvi != expected_dvi {
            return Err(format!(
                "isolated {} path step {} differs from cold output",
                path_kind.name(),
                step_index + 1,
            ));
        }
        if step_index >= options.warmups {
            durations.push(elapsed);
            stages.push(IncrementalStages::from_reuse(
                elapsed,
                dvi_latency,
                accepted.reuse,
            ));
        }
        last_reuse = accepted.reuse;
        on_left = !on_left;
    }

    println!(
        "gentle-profile isolated incremental path: path={} measured_advances={} warmup_advances={} memo_layers={:?}",
        path_kind.name(),
        options.iterations,
        options.warmups,
        options.memo_recording,
    );
    print_duration_stats(
        &format!("isolated {}", path_kind.name()),
        duration_stats(&durations),
    );
    print_isolated_stage_attribution(path_kind, &stages);
    println!(
        "gentle-profile isolated incremental work: path={} last_pages_retained_prefix={} last_pages_retyped={} last_pages_reused={} last_paragraphs_reexecuted={} last_bytes_reexecuted={} last_tokens_reexecuted={} last_commands_reexecuted={} last_trace_nodes_walked={} last_trace_leaf_hits={} last_trace_subtree_hits={} last_suffixes_adopted={}",
        path_kind.name(),
        last_reuse.pages_retained_prefix,
        last_reuse.pages_retyped,
        last_reuse.pages_reused,
        last_reuse.reexecuted_paragraphs,
        last_reuse.reexecuted_bytes,
        last_reuse.reexecuted_tokens,
        last_reuse.reexecuted_commands,
        last_reuse.trace_nodes_walked,
        last_reuse.trace_leaf_hits,
        last_reuse.trace_subtree_hits,
        last_reuse.suffixes_adopted,
    );
    Ok(())
}

#[allow(clippy::disallowed_methods)] // Host-side deletion-baseline timer.
fn run_edit_restart_workload(
    options: &Options,
    template: &ProfileTemplate,
    workload: EditRestartWorkload,
) -> Result<(), String> {
    if options.checkpoints || options.incremental_edit || options.incremental_path.is_some() {
        return Err("--edit-restart-workload cannot be combined with another workload".to_owned());
    }
    if workload == EditRestartWorkload::Long && (options.iterations != 2 || options.warmups != 1) {
        return Err("the fixed long acceptance requires --iterations 2 --warmups 1".to_owned());
    }
    let (before, after) = if workload == EditRestartWorkload::Long {
        long_edit_restart_workload()?
    } else {
        let directory = options.repo_root.join("benchmarks/edit-restart/workloads");
        let before_path = directory.join(format!("{}-before.tex", workload.name()));
        let after_path = directory.join(format!("{}-after.tex", workload.name()));
        (
            fs::read_to_string(&before_path)
                .map_err(|error| format!("read workload {}: {error}", before_path.display()))?,
            fs::read_to_string(&after_path)
                .map_err(|error| format!("read workload {}: {error}", after_path.display()))?,
        )
    };
    let source_path = Path::new(JOB_DIR).join(JOB_FILE);
    let reachability_store = new_reachability_store();
    let mut session = incremental_session(
        &reachability_store,
        template,
        &before,
        RevisionId::new(1),
        true,
        options.memo_recording,
    )?;
    let mut resolvers = FileSessionResolvers::new(&source_path, Vec::new(), Vec::new());
    let initial_started = Instant::now();
    let initial = session
        .cold_with_resolvers(&mut resolvers)
        .map_err(|error| format!("prepare {} workload: {error}", workload.name()))?;
    let initial_duration = initial_started.elapsed();
    let before_dvi = initial.dvi_bytes().map_err(|error| error.to_string())?;
    drop(initial);
    validate_edit_restart_session(&session, RevisionId::new(1), "initial cold")?;
    let (after_cold_duration, after_cold) =
        execute_cold_sample(template, &after, RevisionId::new(1))?;
    let after_dvi = after_cold.dvi_bytes().map_err(|error| error.to_string())?;
    drop(after_cold);
    let mut revision = 1_u64;
    let mut on_before = true;
    let mut durations = Vec::with_capacity(options.iterations);
    let mut stages = Vec::with_capacity(options.iterations);
    let mut samples = Vec::with_capacity(options.iterations);
    let mut last_reuse = ReuseMetrics::default();
    for step in 0..options.warmups.saturating_add(options.iterations) {
        let (from, to, expected, target_cold) = if on_before {
            (
                before.as_str(),
                after.as_str(),
                after_dvi.as_slice(),
                after_cold_duration,
            )
        } else {
            (
                after.as_str(),
                before.as_str(),
                before_dvi.as_slice(),
                initial_duration,
            )
        };
        revision += 1;
        let edit = replacement_edit(from, to, session.revision(), session.content_hash());
        let mut resolvers = FileSessionResolvers::new(&source_path, Vec::new(), Vec::new());
        let started = Instant::now();
        let mut candidate = session
            .start_advance_candidate(RevisionId::new(revision), edit)
            .map_err(|error| format!("start {} workload candidate: {error}", workload.name()))?;
        validate_edit_restart_generations(&session, 1, 1, "live candidate")?;
        match candidate
            .drive_with_resource_resolvers(&mut resolvers, &Cancellation::new())
            .map_err(|error| format!("drive {} workload candidate: {error}", workload.name()))?
        {
            RevisionCandidateResult::Complete => {}
            RevisionCandidateResult::AwaitingResources(need) => {
                return Err(format!(
                    "{} workload candidate made no resource progress: {need:?}",
                    workload.name(),
                ));
            }
        }
        let transaction = session
            .prepare_revision_candidate(candidate)
            .map_err(|error| format!("prepare {} workload revision: {error}", workload.name()))?;
        validate_edit_restart_generations(&session, 1, 1, "completed candidate")?;
        let accepted = session
            .accept_revision(transaction)
            .map_err(|error| format!("accept {} workload revision: {error}", workload.name()))?;
        let elapsed = started.elapsed();
        let dvi_started = Instant::now();
        let dvi = accepted.dvi_bytes().map_err(|error| error.to_string())?;
        let dvi_latency = dvi_started.elapsed();
        if dvi != expected {
            return Err(format!(
                "{} workload step {} differs from cold output",
                workload.name(),
                step + 1,
            ));
        }
        if step >= options.warmups {
            durations.push(elapsed);
            stages.push(IncrementalStages::from_reuse(
                elapsed,
                dvi_latency,
                accepted.reuse,
            ));
            samples.push(EditRestartSample {
                elapsed,
                target_cold,
                reuse: accepted.reuse,
            });
        }
        last_reuse = accepted.reuse;
        drop(accepted);
        validate_edit_restart_session(
            &session,
            RevisionId::new(revision),
            &format!("accepted step {}", step + 1),
        )?;
        on_before = !on_before;
    }
    let edit_stats = duration_stats(&durations);
    let fresh_mean_ms = (initial_duration + after_cold_duration).as_secs_f64() * 500.0;
    let cold_mean_ratio = edit_stats.mean / fresh_mean_ms;
    let (latency_ratio, latency_limit) =
        validate_edit_restart_latency(workload, &samples, cold_mean_ratio)?;
    let retention = session
        .retention_metrics()
        .expect("a completed edit-restart session has retention metrics");
    let before_sha256 = format!("{:x}", Sha256::digest(before.as_bytes()));
    let after_sha256 = format!("{:x}", Sha256::digest(after.as_bytes()));
    println!(
        "gentle-profile edit-restart workload: name={} measured_advances={} warmup_advances={} before_sha256={} after_sha256={} before_content_hash={:?} after_content_hash={:?} memo_layers={:?} checkpoint_budget_bytes={}",
        workload.name(),
        options.iterations,
        options.warmups,
        before_sha256,
        after_sha256,
        ContentHash::from_bytes(before.as_bytes()),
        ContentHash::from_bytes(after.as_bytes()),
        options.memo_recording,
        PROFILING_CHECKPOINT_BUDGET,
    );
    print_duration_stats(
        &format!("edit-restart workload {}", workload.name()),
        edit_stats,
    );
    println!(
        "gentle-profile edit-restart workload fresh: name={} before_ms={:.3} after_ms={:.3}",
        workload.name(),
        initial_duration.as_secs_f64() * 1_000.0,
        after_cold_duration.as_secs_f64() * 1_000.0,
    );
    println!(
        "gentle-profile edit workload reuse: name={} pages_retained_prefix={} pages_retyped={} pages_reused={} paragraphs_reexecuted={} bytes_reexecuted={} tokens_reexecuted={} commands_reexecuted={} suffixes_adopted={}",
        workload.name(),
        last_reuse.pages_retained_prefix,
        last_reuse.pages_retyped,
        last_reuse.pages_reused,
        last_reuse.reexecuted_paragraphs,
        last_reuse.reexecuted_bytes,
        last_reuse.reexecuted_tokens,
        last_reuse.reexecuted_commands,
        last_reuse.suffixes_adopted,
    );
    let mean_stages = mean_incremental_stages(&stages);
    println!(
        "gentle-profile edit-restart workload stages: name={} snapshot_us={} detach_materialize_fork_us={} executor_us={} dvi_materialization_us={}",
        workload.name(),
        mean_stages.output_snapshot.as_micros(),
        mean_stages.restart_fork.as_micros(),
        mean_stages.executor.as_micros(),
        mean_stages.dvi_materialization.as_micros(),
    );
    println!(
        "gentle-profile edit-restart acceptance: name={} cold_dvi=true cold_mean_ratio={cold_mean_ratio:.6} gated_latency_ratio={latency_ratio:.6} latency_limit={latency_limit:.2} replay_retained_bytes={} checkpoint_root_bytes={} checkpoint_roots={} protected_overage_bytes={} retained_generations={} candidate_generations={} accepted_revision={} revision_chain_depth=1",
        workload.name(),
        retention.memo_result_bytes,
        retention.checkpoint_root_bytes,
        session.current_retained_checkpoint_count(),
        retention.protected_overage_bytes,
        session.retained_generation_count(),
        session.current_candidate_generation_count(),
        session.revision().raw(),
    );
    Ok(())
}

fn validate_edit_restart_generations(
    session: &Session<'_>,
    retained: usize,
    candidate: usize,
    stage: &str,
) -> Result<(), String> {
    let occupied = retained.saturating_add(candidate);
    if session.retained_generation_count() != retained
        || session.current_candidate_generation_count() != candidate
        || session.occupied_generation_slot_count() != occupied
        || occupied > 2
    {
        return Err(format!(
            "edit-restart {stage} violates generation lifecycle: retained={} candidate={} occupied={}",
            session.retained_generation_count(),
            session.current_candidate_generation_count(),
            session.occupied_generation_slot_count(),
        ));
    }
    Ok(())
}

fn validate_edit_restart_session(
    session: &Session<'_>,
    revision: RevisionId,
    stage: &str,
) -> Result<(), String> {
    validate_edit_restart_generations(session, 1, 0, stage)?;
    if session.revision() != revision
        || session
            .retained_revision_ids()
            .ne(std::iter::once(revision))
    {
        return Err(format!(
            "edit-restart {stage} does not directly own accepted revision {}",
            revision.raw(),
        ));
    }
    let retention = session
        .retention_metrics()
        .ok_or_else(|| format!("edit-restart {stage} has no accepted retention metrics"))?;
    if retention.checkpoint_root_bytes > PROFILING_CHECKPOINT_BUDGET
        || retention.protected_overage_bytes != 0
    {
        return Err(format!(
            "edit-restart {stage} exceeds the checkpoint-root budget: retained={} budget={} protected_overage={}",
            retention.checkpoint_root_bytes,
            PROFILING_CHECKPOINT_BUDGET,
            retention.protected_overage_bytes,
        ));
    }
    if retention.memo_result_bytes != 0 || session.pure_memo_stats() != PureMemoStats::default() {
        return Err(format!(
            "edit-restart {stage} retained deleted replay/memo state"
        ));
    }
    Ok(())
}

fn validate_edit_restart_latency(
    workload: EditRestartWorkload,
    samples: &[EditRestartSample],
    latency_ratio: f64,
) -> Result<(f64, f64), String> {
    if workload == EditRestartWorkload::Suffix {
        if !samples
            .iter()
            .any(|sample| sample.reuse.suffixes_adopted > 0)
        {
            return Err("suffix workload did not adopt a generic suffix".to_owned());
        }
        let fastest = samples
            .iter()
            .min_by(|left, right| left.elapsed.cmp(&right.elapsed))
            .expect("edit-restart requires positive measured iterations");
        let ratio = fastest.elapsed.as_secs_f64() / fastest.target_cold.as_secs_f64();
        if ratio > SUFFIX_FAST_RATIO_LIMIT {
            return Err(format!(
                "suffix workload fastest measured advance is {ratio:.6}x cold, limit is {SUFFIX_FAST_RATIO_LIMIT:.2}x"
            ));
        }
        return Ok((ratio, SUFFIX_FAST_RATIO_LIMIT));
    }
    if latency_ratio > EDIT_RESTART_LATENCY_RATIO_LIMIT {
        return Err(format!(
            "{} workload edit mean is {latency_ratio:.6}x cold mean, limit is {EDIT_RESTART_LATENCY_RATIO_LIMIT:.2}x",
            workload.name(),
        ));
    }
    Ok((latency_ratio, EDIT_RESTART_LATENCY_RATIO_LIMIT))
}

fn long_edit_restart_workload() -> Result<(String, String), String> {
    const PARAGRAPHS: usize = 384;
    const PARAGRAPH: &str = "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega. Ordinary repeated prose gives the line breaker and page builder stable multi-line material for the edit-restart deletion comparison.\n\n";
    let header = "\\input plain.tex\n\\hsize=220pt \\vsize=500pt\n";
    let mut before = String::with_capacity(header.len() + PARAGRAPHS * PARAGRAPH.len() + 6);
    before.push_str(header);
    for _ in 0..PARAGRAPHS {
        before.push_str(PARAGRAPH);
    }
    before.push_str("\\bye\n");
    let mut after = before.clone();
    let edit = after
        .find("Alpha")
        .expect("generated long workload has its prefix edit");
    after.replace_range(edit..edit + "Alpha".len(), "Omega");
    for (name, source, expected) in [
        ("before", before.as_bytes(), LONG_BEFORE_SHA256),
        ("after", after.as_bytes(), LONG_AFTER_SHA256),
    ] {
        let actual = format!("{:x}", Sha256::digest(source));
        if actual != expected {
            return Err(format!(
                "generated long {name} SHA-256 changed: expected {expected}, got {actual}"
            ));
        }
    }
    Ok((before, after))
}

fn mean_incremental_stages(samples: &[IncrementalStages]) -> IncrementalStages {
    fn mean(
        samples: &[IncrementalStages],
        field: impl Fn(&IncrementalStages) -> Duration,
    ) -> Duration {
        if samples.is_empty() {
            return Duration::ZERO;
        }
        let nanos = samples
            .iter()
            .map(|sample| field(sample).as_nanos())
            .sum::<u128>()
            / samples.len() as u128;
        Duration::from_nanos(nanos.min(u128::from(u64::MAX)) as u64)
    }
    IncrementalStages {
        revision_setup: mean(samples, |sample| sample.revision_setup),
        restart_fork: mean(samples, |sample| sample.restart_fork),
        executor: mean(samples, |sample| sample.executor),
        executor_shell: mean(samples, |sample| sample.executor_shell),
        output_snapshot: mean(samples, |sample| sample.output_snapshot),
        splice: mean(samples, |sample| sample.splice),
        substrate_transition: mean(samples, |sample| sample.substrate_transition),
        acceptance: mean(samples, |sample| sample.acceptance),
        unaccounted: mean(samples, |sample| sample.unaccounted),
        dvi_materialization: mean(samples, |sample| sample.dvi_materialization),
    }
}

fn replacement_edit(
    from: &str,
    to: &str,
    base_revision: RevisionId,
    expected_hash: ContentHash,
) -> Edit {
    let (range, replacement) = replacement_between(from, to);
    Edit {
        base_revision,
        expected_hash,
        range,
        replacement,
    }
}

fn replacement_between(from: &str, to: &str) -> (Range<usize>, String) {
    let mut prefix = from
        .as_bytes()
        .iter()
        .zip(to.as_bytes())
        .take_while(|(left, right)| left == right)
        .count();
    while !from.is_char_boundary(prefix) || !to.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let max_suffix = from.len().min(to.len()).saturating_sub(prefix);
    let mut suffix = from
        .as_bytes()
        .iter()
        .rev()
        .zip(to.as_bytes().iter().rev())
        .take(max_suffix)
        .take_while(|(left, right)| left == right)
        .count();
    while !from.is_char_boundary(from.len() - suffix) || !to.is_char_boundary(to.len() - suffix) {
        suffix -= 1;
    }
    (
        prefix..from.len() - suffix,
        to[prefix..to.len() - suffix].to_owned(),
    )
}

fn run_incremental_edit(options: &Options, template: &ProfileTemplate) -> Result<(), String> {
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
    let mut cold_reference = std::iter::repeat_with(|| None)
        .take(edit_count)
        .collect::<Vec<_>>();
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
        if fast_path.reuse.convergence_boundary.is_none()
            || fast_path.reuse.same_history_stop != SameHistoryStop::Matched
        {
            return Err(format!(
                "{name} height-preserving edit did not report a matched named-boundary convergence"
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
            IncrementalPath::Slow | IncrementalPath::Rebreak => {
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
            IncrementalPath::Interaction | IncrementalPath::Fast | IncrementalPath::Neutral => {
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
                    || baseline.reuse.convergence_boundary != candidate.reuse.convergence_boundary
                {
                    return Err(format!(
                        "{} edit {} did not preserve equivalent suffix adoption: baseline={baseline_pages:?}/{:?} candidate={candidate_pages:?}/{:?}",
                        fixture.edit_paths[index].name(),
                        index + 1,
                        baseline.reuse.convergence_boundary,
                        candidate.reuse.convergence_boundary,
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
        cfg!(feature = "profiling"),
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
        "gentle-profile fast path verified: edit={} ({}) retained_prefix={} re-shipped={} adopted={} convergence={:?} leaf_hits={} subtree_hits={} baseline_vs_cold={:.3}x candidate_vs_cold={:.3}x",
        fixture.suffix_adoption_edit + 1,
        fixture.edit_names[fixture.suffix_adoption_edit],
        work.pages_retained_prefix,
        work.pages_retyped,
        work.pages_reused,
        work.convergence_boundary.map(|boundary| boundary.boundary),
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
        "gentle-profile stage attribution (baseline/candidate/delta ms): edit={edit} baseline={baseline_name:?} candidate={candidate_name:?} delta={delta_name:?} revision_setup={} restart_fork={} executor={} executor_shell={} diagnostics_effects_snapshot={} splice={} substrate_publish_drop={} acceptance={} unaccounted_system_noise={} dvi_materialization={}",
        stage!(revision_setup),
        stage!(restart_fork),
        stage!(executor),
        stage!(executor_shell),
        stage!(output_snapshot),
        stage!(splice),
        stage!(substrate_transition),
        stage!(acceptance),
        stage!(unaccounted),
        stage!(dvi_materialization),
    );
}

fn print_isolated_stage_attribution(path: IncrementalPath, samples: &[IncrementalStages]) {
    macro_rules! stage {
        ($field:ident) => {
            stage_mean(samples, |sample| sample.$field)
        };
    }
    println!(
        "gentle-profile isolated stage means (ms): path={} revision_setup={:.3} restart_fork={:.3} executor={:.3} executor_shell={:.3} diagnostics_effects_snapshot={:.3} splice={:.3} substrate_publish_drop={:.3} acceptance={:.3} unaccounted_system_noise={:.3} dvi_materialization={:.3}",
        path.name(),
        stage!(revision_setup),
        stage!(restart_fork),
        stage!(executor),
        stage!(executor_shell),
        stage!(output_snapshot),
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
    println!(
        "gentle-profile incremental work: {name}: edit={edit} pages_retained_prefix={} pages_retyped={} pages_reused={} paragraphs_reexecuted={} bytes_reexecuted={} tokens_reexecuted={} commands_reexecuted={} trace_nodes_walked={} trace_leaf_hits={} trace_subtree_hits={} suffixes_adopted={} same_history_stop={:?} revision_setup_us={} fork_us={} executor_us={} reexecute_us={} snapshot_us={} trace_validation_us={} splice_us={} substrate_us={} acceptance_us={} dvi_us={}",
        reuse.pages_retained_prefix,
        reuse.pages_retyped,
        reuse.pages_reused,
        reuse.reexecuted_paragraphs,
        reuse.reexecuted_bytes,
        reuse.reexecuted_tokens,
        reuse.reexecuted_commands,
        reuse.trace_nodes_walked,
        reuse.trace_leaf_hits,
        reuse.trace_subtree_hits,
        reuse.suffixes_adopted,
        reuse.same_history_stop,
        reuse.revision_setup_latency.as_micros(),
        reuse.restart_fork_latency.as_micros(),
        reuse.executor_latency.as_micros(),
        reuse.reexecution_latency.as_micros(),
        reuse.output_snapshot_latency.as_micros(),
        reuse.trace_validation_latency.as_micros(),
        reuse.splice_latency.as_micros(),
        reuse.substrate_transition_latency.as_micros(),
        reuse.acceptance_latency.as_micros(),
        sample.dvi_latency.as_micros(),
    );
    for (layer_name, layer) in [
        ("pretolerance", PureMemoLayer::Pretolerance),
        ("page", PureMemoLayer::Page),
        ("shipout", PureMemoLayer::Shipout),
    ] {
        let current = sample.memo.layer(layer);
        let previous = sample.previous_memo.layer(layer);
        print_memo_layer(name, edit, layer_name, current.saturating_since(previous));
    }
    println!(
        "gentle-profile memo retention: {name}: edit={edit} detached_cache_bytes={}",
        sample.memo.retained_bytes,
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

#[allow(clippy::disallowed_methods)] // Profiling setup reads the pinned corpus once.
fn incremental_fixture(repo_root: &Path) -> Result<IncrementalFixture, String> {
    let body_path = repo_root.join("third_party/corpus/gentle.tex");
    let body = fs::read_to_string(&body_path).map_err(|error| {
        format!(
            "read required input {}: {error}; run python3 scripts/provision.py worktree .",
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
    let mut rebreak_edited = equal_width_edited.clone();
    rebreak_edited.insert_str(start, GENTLE_REBREAK_ASSIGNMENT);
    let edit_five = Edit {
        base_revision: RevisionId::new(5),
        expected_hash: ContentHash::from_bytes(equal_width_edited.as_bytes()),
        range: start..start,
        replacement: GENTLE_REBREAK_ASSIGNMENT.to_owned(),
    };
    Ok(IncrementalFixture {
        original,
        revisions: vec![
            edited.clone(),
            followed_up,
            edited,
            equal_width_edited,
            rebreak_edited,
        ],
        edits: vec![edit_one, edit_two, edit_three, edit_four, edit_five],
        edit_names: vec![
            "large pagination-changing insertion",
            "follow-up insertion",
            "inverse removal",
            "height-preserving equal-width substitution",
            "line-breaking dependency change",
        ],
        edit_paths: vec![
            IncrementalPath::Slow,
            IncrementalPath::Interaction,
            IncrementalPath::Slow,
            IncrementalPath::Fast,
            IncrementalPath::Rebreak,
        ],
        suffix_adoption_edit: 3,
        body_offset,
        body_len: body.len(),
        inserted_bytes: insertion.len() + 1,
        inserted_words: insertion.split_whitespace().count(),
    })
}

fn incremental_session<'store>(
    reachability_store: &'store tex_state::ReachabilityStore,
    template: &ProfileTemplate,
    source: &str,
    revision: RevisionId,
    _memo: bool,
    _recording: PureMemoRecordingPolicy,
) -> Result<Session<'store>, String> {
    let mut session = Session::start_with_source_path(
        reachability_store,
        "gentle-profile",
        Path::new(JOB_DIR).join(JOB_FILE).to_string_lossy(),
        revision,
        source,
        PROFILING_CHECKPOINT_BUDGET,
    )
    .map_err(|error| error.to_string())?;
    template.register_inputs(&mut session)?;
    Ok(session)
}

#[allow(clippy::disallowed_methods)] // Host-side benchmark timer; no engine fact observes it.
fn execute_incremental_sample(
    template: &ProfileTemplate,
    fixture: &IncrementalFixture,
    memo: bool,
    recording: PureMemoRecordingPolicy,
) -> Result<IncrementalSample, String> {
    let path = Path::new(JOB_DIR).join(JOB_FILE);
    let reachability_store = new_reachability_store();
    let mut session = incremental_session(
        &reachability_store,
        template,
        &fixture.original,
        RevisionId::new(1),
        memo,
        recording,
    )?;
    let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
    let priming_started = Instant::now();
    session
        .cold_with_resolvers(&mut resolvers)
        .map_err(|error| format!("prepare incremental baseline: {error}"))?;
    let priming_elapsed = priming_started.elapsed();
    let mut steps = Vec::with_capacity(fixture.edits.len());
    for (index, edit) in fixture.edits.iter().enumerate() {
        let previous_memo = session.pure_memo_stats();
        let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
        let started = Instant::now();
        let accepted = session
            .advance_with_resolvers(
                RevisionId::new(index as u64 + 2),
                edit.clone(),
                &mut resolvers,
            )
            .map_err(|error| format!("advance incremental edit {}: {error}", index + 1))?;
        let elapsed = started.elapsed();
        let memo = session.pure_memo_stats();
        let dvi_started = Instant::now();
        let dvi = accepted.dvi_bytes().map_err(|error| error.to_string())?;
        let dvi_latency = dvi_started.elapsed();
        let _ = black_box(accepted.pages().len());
        steps.push(IncrementalStep {
            elapsed,
            dvi_latency,
            dvi,
            pages: accepted.pages().len(),
            reuse: accepted.reuse,
            history: session
                .history()
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
        });
    }
    Ok(IncrementalSample {
        priming_elapsed,
        steps,
    })
}

#[allow(clippy::disallowed_methods)] // Host-side benchmark timer; no engine fact observes it.
fn execute_cold_sample(
    template: &ProfileTemplate,
    source: &str,
    revision: RevisionId,
) -> Result<(Duration, AcceptedOutput), String> {
    let path = Path::new(JOB_DIR).join(JOB_FILE);
    let reachability_store = new_reachability_store();
    let mut session = incremental_session(
        &reachability_store,
        template,
        source,
        revision,
        false,
        PureMemoRecordingPolicy::default(),
    )?;
    let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
    let started = Instant::now();
    let accepted = session
        .cold_with_resolvers(&mut resolvers)
        .map_err(|error| format!("compile cold edited document: {error}"))?;
    let elapsed = started.elapsed();
    let _ = black_box(accepted.pages().len());
    Ok((elapsed, accepted))
}

fn load_template(repo_root: &Path) -> Result<ProfileTemplate, String> {
    let corpus = repo_root.join("third_party/corpus");
    let mut template = ProfileTemplate {
        world: World::memory_with_clock(JobClock {
            time: 13 * 60 + 36,
            second: 0,
            day: 9,
            month: 7,
            year: 2026,
        }),
        inputs: Vec::new(),
    };
    seed_file(&mut template, &corpus.join("plain.tex"), "plain.tex")?;
    seed_file(&mut template, &corpus.join("gentle.tex"), "gentle.tex")?;
    seed_file(
        &mut template,
        &repo_root.join("third_party/hyphen/hyphen.tex"),
        "hyphen.tex",
    )?;
    seed_font_dir(&mut template, &repo_root.join("third_party/fonts"))?;
    seed_font_dir(
        &mut template,
        &repo_root.join("crates/tex-fonts/tests/fixtures/cm"),
    )?;
    template
        .world
        .set_memory_file(
            Path::new(JOB_DIR).join(JOB_FILE),
            b"\\input plain.tex\n\\input gentle.tex\n".to_vec(),
        )
        .map_err(|error| error.to_string())?;
    Ok(template)
}

#[allow(clippy::disallowed_methods)] // Profiling setup reads host inputs once before the run loop.
fn seed_file(template: &mut ProfileTemplate, source: &Path, name: &str) -> Result<(), String> {
    let bytes: Arc<[u8]> = fs::read(source)
        .map_err(|error| {
            format!(
                "read required input {}: {error}; run python3 scripts/provision.py worktree .",
                source.display()
            )
        })?
        .into();
    let path = Path::new(JOB_DIR).join(name);
    template
        .world
        .set_shared_memory_file(&path, Arc::clone(&bytes))
        .map_err(|error| error.to_string())?;
    template.inputs.push((path, bytes));
    Ok(())
}

#[allow(clippy::disallowed_methods)] // Profiling setup enumerates and reads host fonts once.
fn seed_font_dir(template: &mut ProfileTemplate, dir: &Path) -> Result<(), String> {
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
        let bytes: Arc<[u8]> = fs::read(&path)
            .map_err(|error| format!("read font metric {}: {error}", path.display()))?
            .into();
        let memory_path = Path::new(JOB_DIR).join(name);
        template
            .world
            .set_shared_memory_file(&memory_path, Arc::clone(&bytes))
            .map_err(|error| error.to_string())?;
        template.inputs.push((memory_path, bytes));
    }
    Ok(())
}

fn execute_once(
    template: &ProfileTemplate,
    capture_checkpoints: bool,
) -> Result<RunOutput, String> {
    umber::with_engine_world(template.world.clone(), |stores| {
        let path = Path::new(JOB_DIR).join(JOB_FILE);
        let content = stores
            .world_mut()
            .read_file(&path)
            .map_err(|error| error.to_string())?;
        let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
        let mut checkpoints = ProfileCheckpointSink {
            enabled: capture_checkpoints,
            ..ProfileCheckpointSink::default()
        };
        let startup_name = path.to_string_lossy();
        let mut session = EngineSession::tex82_initex(stores);
        session
            .register_retained_root(
                startup_name.as_ref(),
                SourceRegistration::world(content).with_name(startup_name.as_ref()),
            )
            .map_err(|error| error.to_string())?;
        let (run, _expansion_stats) = session
            .run_with_expansion_stats(&mut resolvers, &mut checkpoints)
            .map_err(|error| error.to_string())?;
        if run.artifacts.is_empty() {
            return Err("Gentle produced no page artifacts".to_owned());
        }
        let dvi = dvi_from_page_plans(&run.dvi_pages).map_err(|error| error.to_string())?;
        Ok(RunOutput {
            dvi,
            pages: run.artifacts.len(),
            checkpoints: checkpoints.count,
            checkpoint_hash: checkpoints.hash,
            #[cfg(feature = "profiling")]
            expansion_stats: _expansion_stats,
        })
    })
    .map_err(|error| format!("{error:?}"))?
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
    #[cfg(feature = "profiling")]
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
    #[cfg(feature = "profiling")]
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

fn parse_incremental_path(value: &str) -> Result<IncrementalPath, String> {
    match value {
        "fast" => Ok(IncrementalPath::Fast),
        "slow" => Ok(IncrementalPath::Slow),
        "neutral" => Ok(IncrementalPath::Neutral),
        _ => Err(format!(
            "--incremental-path expects fast, slow, or neutral, got {value:?}"
        )),
    }
}

fn parse_edit_restart_workload(value: &str) -> Result<EditRestartWorkload, String> {
    match value {
        "unchanged" => Ok(EditRestartWorkload::Unchanged),
        "prefix" => Ok(EditRestartWorkload::Prefix),
        "suffix" => Ok(EditRestartWorkload::Suffix),
        "display-math" => Ok(EditRestartWorkload::DisplayMath),
        "macro" => Ok(EditRestartWorkload::Macro),
        "conditional" => Ok(EditRestartWorkload::Conditional),
        "long" => Ok(EditRestartWorkload::Long),
        _ => Err(format!(
            "--edit-restart-workload expects unchanged, prefix, suffix, display-math, macro, conditional, or long, got {value:?}"
        )),
    }
}

fn parse_cold_memo_policy(value: &str) -> Result<ColdMemoPolicy, String> {
    if value == "disabled" {
        return Ok(ColdMemoPolicy::Disabled);
    }
    parse_memo_layers(value)
        .map(ColdMemoPolicy::Enabled)
        .map_err(|_| {
            format!(
                "--cold-memo-layers expects disabled, all, none, or a comma-separated layer list, got {value:?}"
            )
        })
}

fn parse_memo_layers(value: &str) -> Result<PureMemoRecordingPolicy, String> {
    if value == "all" {
        return Ok(PureMemoRecordingPolicy::all());
    }
    let mut policy = PureMemoRecordingPolicy {
        pretolerance: false,
        pages: false,
        shipouts: false,
    };
    for layer in value.split(',') {
        match layer {
            "pretolerance" => policy.pretolerance = true,
            "page" => policy.pages = true,
            "shipout" => policy.shipouts = true,
            "none" if value == "none" => {}
            _ => {
                return Err(format!(
                    "--memo-layers expects comma-separated pretolerance,page,shipout, got {value:?}"
                ));
            }
        }
    }
    Ok(policy)
}

fn print_help() {
    println!(
        "Usage: gentle-profile [--iterations N] [--warmups N] [--repo-root PATH] [--checkpoints] [--cold-memo-layers disabled|LIST] [--incremental-edit] [--incremental-path fast|slow|neutral] [--edit-restart-workload NAME] [--baseline-memo-layers LIST] [--memo-layers LIST]\n\n\
         Loads Gentle and its support files once, then executes fresh deterministic\n\
         in-memory Umber sessions for profiling. Defaults: {DEFAULT_ITERATIONS} measured\n\
         iterations and {DEFAULT_WARMUPS} warm-up. --checkpoints captures and hashes every\n\
         named executor checkpoint through a bounded profiling sink.\n\
         --incremental-edit compares a memo baseline, memo candidate, and cold compilation\n\
         five accepted edits/session using balanced AB/BA pairs and DVI parity verification;\n\
         the fifth changes a line-breaking dependency to verify one-shot cold fallback.\n\
         --incremental-path repeatedly ping-pongs one fast, slow, or output-neutral edit after cold setup,\n\
         --edit-restart-workload repeatedly ping-pongs one committed edit pair,\n\
         verifies each direction against cold output, and isolates its sampled stacks.\n\
         --cold-memo-layers repeats fresh incremental-session cold compiles with memoization\n\
         disabled or enabled for the selected layers, isolating recording overhead.\n\
         --memo-layers configures the remaining pure-query recording layers.\n\
         --baseline-memo-layers replaces the disabled control with an explicit recording\n\
         policy for direct marginal layer comparisons."
    );
}

#[cfg(test)]
mod tests {
    use super::{ColdMemoPolicy, parse_cold_memo_policy, replacement_between};

    fn assert_replacement(from: &str, to: &str) {
        let (range, replacement) = replacement_between(from, to);
        let mut actual = from.to_owned();
        actual.replace_range(range, &replacement);
        assert_eq!(actual, to);
    }

    #[test]
    fn replacement_between_round_trips_ascii_and_unicode_edits() {
        for (left, right) in [
            ("prefix words suffix", "prefix sword suffix"),
            ("prefix suffix", "prefix inserted suffix"),
            ("préfixe naïf suffix", "préfixe brûlé suffix"),
        ] {
            assert_replacement(left, right);
            assert_replacement(right, left);
        }
    }

    #[test]
    fn cold_memo_policy_distinguishes_disabled_from_empty_recording() {
        assert_eq!(
            parse_cold_memo_policy("disabled").expect("disabled policy"),
            ColdMemoPolicy::Disabled
        );
        let ColdMemoPolicy::Enabled(recording) =
            parse_cold_memo_policy("none").expect("empty enabled policy")
        else {
            panic!("none must still enable the memo runtime");
        };
        assert!(!recording.pretolerance);
        assert!(!recording.pages);
        assert!(!recording.shipouts);
    }
}
