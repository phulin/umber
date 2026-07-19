//! Persistent in-process Gentle profiling workload.

use std::env;
use std::fs;
use std::hint::black_box;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tex_exec::{Cancellation, CheckpointSink, EngineCheckpoint, PdfImageRequest, PdfImageResolver};
use tex_expand::{InputResolver, ResourceLookup, ResourceResult};
use tex_incr::{
    AcceptedOutput, BoundaryKey, Edit, ReuseMetrics, RevisionCandidateResult, RevisionId,
    SameHistoryStop, Session,
};
#[cfg(feature = "profiling-stats")]
use tex_lex::ExpansionStats;
use tex_lex::{InputSource, InputStack, MemoryInput, WorldInput};
#[cfg(feature = "profiling-stats")]
use tex_state::measurement::{
    ExactIdentityMeasurement, StateHashMeasurement, exact_identity_measurement,
    state_hash_measurement,
};
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
const GENTLE_REBREAK_ASSIGNMENT: &str = "\\tolerance=201 ";
const GENTLE_FAST_PATH_RETYPED_PAGES: usize = 3;
const STABILIZATION_PASSES: usize = 16;
const STABILIZATION_INPUT: &str = "stabilization-ref.tex";

#[derive(Debug)]
struct Options {
    repo_root: PathBuf,
    iterations: usize,
    warmups: usize,
    checkpoints: bool,
    incremental_edit: bool,
    stabilization_replay: bool,
    incremental_path: Option<IncrementalPath>,
    cold_memo_policy: Option<ColdMemoPolicy>,
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
        let mut stabilization_replay = false;
        let mut incremental_path = None;
        let mut cold_memo_policy = None;
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
                "--stabilization-replay" => stabilization_replay = true,
                "--incremental-path" => {
                    incremental_path = Some(parse_incremental_path(&next_value(
                        &mut args,
                        "--incremental-path",
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
        let repo_root = repo_root
            .canonicalize()
            .map_err(|error| format!("resolve repository root {}: {error}", repo_root.display()))?;
        Ok(Some(Self {
            repo_root,
            iterations,
            warmups,
            checkpoints,
            incremental_edit,
            stabilization_replay,
            incremental_path,
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

    if let Some(policy) = options.cold_memo_policy {
        return run_cold_memo_policy(&options, &template, policy);
    }
    if let Some(path) = options.incremental_path {
        return run_incremental_path(&options, &template, path);
    }
    if options.incremental_edit {
        return run_incremental_edit(&options, &template);
    }
    if options.stabilization_replay {
        return run_stabilization_replay(&options, &template);
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
    break_dependency_edit: usize,
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
    priming_memo: PureMemoStats,
    #[cfg(feature = "profiling-stats")]
    priming_state_hash: StateHashMeasurement,
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
    state_hash: StateHashMeasurement,
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
    paragraph_history_transition: Duration,
    splice: Duration,
    substrate_transition: Duration,
    acceptance: Duration,
    unaccounted: Duration,
    dvi_materialization: Duration,
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
            .saturating_add(reuse.paragraph_history_transition_latency)
            .saturating_add(reuse.splice_latency)
            .saturating_add(reuse.substrate_transition_latency)
            .saturating_add(reuse.acceptance_latency);
        Self {
            revision_setup: reuse.revision_setup_latency,
            restart_fork: reuse.restart_fork_latency,
            executor: reuse.executor_latency,
            executor_shell,
            output_snapshot: reuse.output_snapshot_latency,
            paragraph_history_transition: reuse.paragraph_history_transition_latency,
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
    template: &World,
    policy: ColdMemoPolicy,
) -> Result<(), String> {
    if options.checkpoints
        || options.incremental_edit
        || options.stabilization_replay
        || options.incremental_path.is_some()
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
    #[cfg(feature = "profiling-stats")]
    let mut last_state_hash = StateHashMeasurement::default();
    #[cfg(feature = "profiling-stats")]
    let mut last_survivor = SurvivorMeasurement::default();

    for run in 0..total_runs {
        let mut session = incremental_session(
            template,
            &fixture.original,
            RevisionId::new(1),
            memo,
            recording,
        )?;
        let mut resolvers = FileSessionResolvers::new(&source_path, Vec::new(), Vec::new());
        #[cfg(feature = "profiling-stats")]
        let state_hash_before = state_hash_measurement();
        #[cfg(feature = "profiling-stats")]
        let survivor_before = survivor_measurement();
        let started = Instant::now();
        let (input, font) = resolvers.resolvers();
        let accepted = session
            .cold_with_resolvers(input, font)
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
        last_pages = accepted.artifacts.len();
        last_memo = session.pure_memo_stats();
        #[cfg(feature = "profiling-stats")]
        {
            last_state_hash = state_hash_delta(state_hash_measurement(), state_hash_before);
            last_survivor = survivor_delta(survivor_measurement(), survivor_before);
        }
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
        "gentle-profile isolated cold output: pages={} dvi_bytes={} paragraph_history_metadata_bytes={}",
        last_pages,
        reference_dvi.len(),
        last_memo.paragraph_history_metadata_bytes,
    );
    let phases = last_memo.paragraph_recording;
    println!(
        "gentle-profile isolated cold paragraph phases: front_end_dependency_ns={} input_transition_ns={} region_publication_ns={} break_dependency_ns={} line_provenance_ns={} line_retention_ns={}",
        phases.front_end_dependency_nanos,
        phases.input_transition_nanos,
        phases.region_publication_nanos,
        phases.break_dependency_nanos,
        phases.line_provenance_nanos,
        phases.line_retention_nanos,
    );
    #[cfg(feature = "profiling-stats")]
    {
        println!(
            "gentle-profile isolated cold state hash: calls={} journal_entries={} changed_cells={} peak_changed_scratch_bytes={}",
            last_state_hash.calls,
            last_state_hash.journal_entries,
            last_state_hash.changed_cells,
            last_state_hash.peak_changed_cell_scratch_bytes,
        );
        println!(
            "gentle-profile isolated cold survivor: fresh_promotions={} recycled_promotions={} releases={} promotion_nanos={} source_words={} epoch_source_words={} survivor_source_words={} epoch_source_lists={} survivor_source_lists={}",
            last_survivor.fresh_promotions,
            last_survivor.recycled_promotions,
            last_survivor.releases_to_recycling,
            last_survivor
                .fresh_promotion_nanos
                .saturating_add(last_survivor.recycled_promotion_nanos),
            last_survivor.source_words,
            last_survivor.epoch_source_words,
            last_survivor.survivor_source_words,
            last_survivor.epoch_source_lists,
            last_survivor.survivor_source_lists,
        );
    }
    Ok(())
}

struct OverlayInputResolver<'a> {
    fallback: &'a mut dyn InputResolver,
    generated: &'a str,
}

impl InputResolver for OverlayInputResolver<'_> {
    fn open_input(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        name: &str,
        request_index: u64,
    ) -> ResourceResult<Box<dyn InputSource>> {
        if name == STABILIZATION_INPUT {
            return Ok(ResourceLookup::Available(Box::new(MemoryInput::new(
                self.generated,
            ))));
        }
        self.fallback.open_input(input, name, request_index)
    }
}

struct UnavailableImageResolver;

impl PdfImageResolver for UnavailableImageResolver {
    fn open_image(
        &mut self,
        _input: &mut dyn tex_state::InputReadState,
        _request: &PdfImageRequest,
        _request_index: u64,
    ) -> ResourceResult<tex_state::PdfExternalImageSource> {
        Ok(ResourceLookup::Unavailable)
    }
}

struct StabilizationSample {
    initial: Duration,
    passes: Vec<Duration>,
    dvis: Vec<Vec<u8>>,
    lookups: u64,
    hits: u64,
    misses: u64,
    reexecuted_bytes: usize,
    retained_bytes: usize,
}

fn stabilization_source(fixture: &IncrementalFixture) -> String {
    let prefix = format!("\\input plain.tex\n\\input {STABILIZATION_INPUT}\n");
    let mut body = fixture.original["\\input plain.tex\n".len()..].to_owned();
    let offset = fixture.body_offset;
    body.insert_str(offset, "\\hskip\\stabilizationrefwidth ");
    prefix + &body
}

#[allow(clippy::disallowed_methods)] // Host-side stabilization profiling timer.
fn execute_stabilization_sample(
    template: &World,
    source: &str,
    memo: bool,
    recording: PureMemoRecordingPolicy,
) -> Result<StabilizationSample, String> {
    let path = Path::new(JOB_DIR).join(JOB_FILE);
    let mut session = incremental_session(template, source, RevisionId::new(1), memo, recording)?;
    let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
    let (fallback, font) = resolvers.resolvers();
    let mut input = OverlayInputResolver {
        fallback,
        generated: "\\def\\stabilizationrefwidth{0pt}",
    };
    let started = Instant::now();
    session
        .cold_with_resolvers(&mut input, font)
        .map_err(|error| format!("construct stabilization history: {error}"))?;
    let initial = started.elapsed();

    let mut passes = Vec::with_capacity(STABILIZATION_PASSES);
    let mut dvis = Vec::with_capacity(STABILIZATION_PASSES);
    let mut lookups = 0_u64;
    let mut hits = 0_u64;
    let mut misses = 0_u64;
    let mut reexecuted_bytes = 0_usize;
    for pass in 0..STABILIZATION_PASSES {
        let generated = if pass.is_multiple_of(2) {
            "\\def\\stabilizationrefwidth{10pt}"
        } else {
            "\\def\\stabilizationrefwidth{0pt}"
        };
        let mut candidate = session
            .start_external_input_delta_candidate()
            .map_err(|error| format!("start stabilization pass {}: {error}", pass + 1))?;
        let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
        let (fallback, font) = resolvers.resolvers();
        let mut input = OverlayInputResolver {
            fallback,
            generated,
        };
        let mut image = UnavailableImageResolver;
        let started = Instant::now();
        let outcome = candidate
            .drive_with_resource_resolvers(&mut input, font, &mut image, &Cancellation::new())
            .map_err(|error| format!("drive stabilization pass {}: {error}", pass + 1))?;
        if !matches!(outcome, RevisionCandidateResult::Complete) {
            return Err(format!(
                "stabilization pass {} unexpectedly requested a resource",
                pass + 1
            ));
        }
        let pending = session
            .finish_advance_candidate(candidate)
            .map_err(|error| format!("finish stabilization pass {}: {error}", pass + 1))?;
        let reuse = pending.reuse();
        lookups = lookups.saturating_add(reuse.paragraph_replay_lookups);
        hits = hits.saturating_add(reuse.paragraph_replay_hits);
        misses = misses.saturating_add(reuse.paragraph_replay_validation_misses);
        reexecuted_bytes = reexecuted_bytes.saturating_add(reuse.reexecuted_bytes);
        let accepted = session
            .accept_pending(pending)
            .map_err(|error| format!("accept stabilization pass {}: {error}", pass + 1))?;
        passes.push(started.elapsed());
        dvis.push(accepted.dvi_bytes().map_err(|error| error.to_string())?);
    }
    Ok(StabilizationSample {
        initial,
        passes,
        dvis,
        lookups,
        hits,
        misses,
        reexecuted_bytes,
        retained_bytes: session.pure_memo_stats().paragraph_history_metadata_bytes,
    })
}

fn run_stabilization_replay(options: &Options, template: &World) -> Result<(), String> {
    if options.checkpoints
        || options.incremental_edit
        || options.incremental_path.is_some()
        || options.cold_memo_policy.is_some()
        || options.baseline_memo_recording.is_some()
    {
        return Err("--stabilization-replay cannot be combined with another workload".to_owned());
    }
    if !options.iterations.is_multiple_of(2) {
        return Err(
            "--stabilization-replay requires an even --iterations count for AB/BA pairing"
                .to_owned(),
        );
    }
    let fixture = incremental_fixture(&options.repo_root)?;
    let source = stabilization_source(&fixture);
    for _ in 0..options.warmups {
        let _ = execute_stabilization_sample(
            template,
            &source,
            false,
            PureMemoRecordingPolicy::default(),
        )?;
        let _ = execute_stabilization_sample(template, &source, true, options.memo_recording)?;
    }
    let mut cold_initial = Vec::with_capacity(options.iterations);
    let mut replay_initial = Vec::with_capacity(options.iterations);
    let mut cold_passes = Vec::with_capacity(options.iterations * STABILIZATION_PASSES);
    let mut replay_passes = Vec::with_capacity(options.iterations * STABILIZATION_PASSES);
    let mut paired_total = Vec::with_capacity(options.iterations);
    let mut last_cold = None;
    let mut last_replay = None;
    for iteration in 0..options.iterations {
        let order = if iteration.is_multiple_of(2) {
            [false, true]
        } else {
            [true, false]
        };
        let mut pair: [Option<StabilizationSample>; 2] = [None, None];
        for memo in order {
            pair[usize::from(memo)] = Some(execute_stabilization_sample(
                template,
                &source,
                memo,
                options.memo_recording,
            )?);
        }
        let cold = pair[0].take().expect("cold sample");
        let replay = pair[1].take().expect("replay sample");
        if cold.dvis != replay.dvis {
            return Err(format!(
                "AB/BA stabilization outputs differ in iteration {}",
                iteration + 1
            ));
        }
        cold_initial.push(cold.initial);
        replay_initial.push(replay.initial);
        cold_passes.extend(cold.passes.iter().copied());
        replay_passes.extend(replay.passes.iter().copied());
        let cold_total = cold.initial + cold.passes.iter().copied().sum::<Duration>();
        let replay_total = replay.initial + replay.passes.iter().copied().sum::<Duration>();
        paired_total.push((replay_total.as_secs_f64() - cold_total.as_secs_f64()) * 1_000.0);
        last_cold = Some(cold);
        last_replay = Some(replay);
    }
    let cold = last_cold.expect("measured cold sample");
    let replay = last_replay.expect("measured replay sample");
    println!(
        "gentle-profile stabilization replay: passes_per_session={STABILIZATION_PASSES} measured_sessions={} warmup_sessions={} order=AB/BA",
        options.iterations, options.warmups,
    );
    print_duration_stats("stabilization cold initial", duration_stats(&cold_initial));
    print_duration_stats(
        "stabilization replay initial",
        duration_stats(&replay_initial),
    );
    print_duration_stats("stabilization cold passes", duration_stats(&cold_passes));
    print_duration_stats(
        "stabilization replay passes",
        duration_stats(&replay_passes),
    );
    let delta = scalar_stats(&paired_total);
    println!(
        "gentle-profile stabilization paired total delta: replay-cold mean={:+.3}ms median={:+.3}ms min={:+.3}ms max={:+.3}ms",
        delta.mean, delta.median, delta.min, delta.max,
    );
    println!(
        "gentle-profile stabilization work: policy=cold passes={} lookups={} hits={} misses={} reexecuted_bytes={} retained_bytes={}",
        STABILIZATION_PASSES,
        cold.lookups,
        cold.hits,
        cold.misses,
        cold.reexecuted_bytes,
        cold.retained_bytes,
    );
    println!(
        "gentle-profile stabilization work: policy=replay passes={} lookups={} hits={} misses={} reexecuted_bytes={} retained_bytes={}",
        STABILIZATION_PASSES,
        replay.lookups,
        replay.hits,
        replay.misses,
        replay.reexecuted_bytes,
        replay.retained_bytes,
    );
    Ok(())
}

#[allow(clippy::disallowed_methods)] // Host-side path-isolated profiling timer.
fn run_incremental_path(
    options: &Options,
    template: &World,
    path_kind: IncrementalPath,
) -> Result<(), String> {
    if options.checkpoints || options.incremental_edit || options.stabilization_replay {
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
    let mut session = incremental_session(
        template,
        left,
        RevisionId::new(1),
        true,
        options.memo_recording,
    )?;
    let mut resolvers = FileSessionResolvers::new(&source_path, Vec::new(), Vec::new());
    let (input, font) = resolvers.resolvers();
    let initial = session
        .cold_with_resolvers(input, font)
        .map_err(|error| format!("prepare isolated {} path: {error}", path_kind.name()))?;
    let left_dvi = initial.dvi_bytes().map_err(|error| error.to_string())?;
    let (_, right_cold) = execute_cold_sample(template, right, RevisionId::new(1))?;
    let right_dvi = right_cold.dvi_bytes().map_err(|error| error.to_string())?;

    let mut revision = 1_u64;
    let mut on_left = true;
    let total_steps = options.warmups + options.iterations;
    let mut durations = Vec::with_capacity(options.iterations);
    let mut stages = Vec::with_capacity(options.iterations);
    let mut line_hits = 0_u64;
    let mut commands_skipped = 0_u64;
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
        let previous_memo = session.pure_memo_stats();
        let mut resolvers = FileSessionResolvers::new(&source_path, Vec::new(), Vec::new());
        let started = Instant::now();
        let (input, font) = resolvers.resolvers();
        let accepted = session
            .advance_with_resolvers(RevisionId::new(revision), edit, input, font)
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
        let current_memo = session.pure_memo_stats();
        if step_index >= options.warmups {
            durations.push(elapsed);
            stages.push(IncrementalStages::from_reuse(
                elapsed,
                dvi_latency,
                accepted.reuse,
            ));
            line_hits = line_hits.saturating_add(
                current_memo
                    .paragraph_line_hits
                    .saturating_sub(previous_memo.paragraph_line_hits),
            );
            commands_skipped = commands_skipped.saturating_add(
                current_memo
                    .paragraph_commands_skipped
                    .saturating_sub(previous_memo.paragraph_commands_skipped),
            );
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
        "gentle-profile isolated incremental work: path={} last_pages_retained_prefix={} last_pages_retyped={} last_pages_reused={} last_paragraphs_reexecuted={} last_bytes_reexecuted={} last_tokens_reexecuted={} last_commands_reexecuted={} last_trace_nodes_walked={} last_trace_leaf_hits={} last_trace_subtree_hits={} last_suffixes_adopted={} paragraph_line_hits={} paragraph_commands_skipped={}",
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
        line_hits,
        commands_skipped,
    );
    Ok(())
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
    #[cfg(feature = "profiling-stats")]
    for (name, sample) in [
        (baseline_name, &disabled_sample),
        (candidate_name, &enabled_sample),
    ] {
        println!(
            "gentle-profile priming state hash journal: {name}: calls={} journal_entries={} changed_cells={} peak_changed_scratch_bytes={}",
            sample.priming_state_hash.calls,
            sample.priming_state_hash.journal_entries,
            sample.priming_state_hash.changed_cells,
            sample.priming_state_hash.peak_changed_cell_scratch_bytes,
        );
        let phases = sample.priming_memo.paragraph_recording;
        println!(
            "gentle-profile priming paragraph phases: {name}: front_end_dependency_ns={} input_transition_ns={} region_publication_ns={} break_dependency_ns={} line_provenance_ns={} line_retention_ns={}",
            phases.front_end_dependency_nanos,
            phases.input_transition_nanos,
            phases.region_publication_nanos,
            phases.break_dependency_nanos,
            phases.line_provenance_nanos,
            phases.line_retention_nanos,
        );
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
    let rebreak = fixture.break_dependency_edit;
    let rebreak_step = &enabled_sample.steps[rebreak];
    let line_hits = rebreak_step
        .memo
        .paragraph_line_hits
        .saturating_sub(rebreak_step.previous_memo.paragraph_line_hits);
    let break_index = tex_state::ParagraphValidationFailure::BreakDependency as usize;
    let break_misses = rebreak_step.memo.paragraph_validation_failure_reasons[break_index]
        .saturating_sub(
            rebreak_step
                .previous_memo
                .paragraph_validation_failure_reasons[break_index],
        );
    if line_hits != 0 || break_misses != 1 {
        return Err(format!(
            "line-breaking dependency edit must take one cold-fallback miss: line_hits={line_hits} break_misses={break_misses}"
        ));
    }
    println!(
        "gentle-profile break-dependency cold fallback verified: edit={} ({}) misses={}",
        rebreak + 1,
        fixture.edit_names[rebreak],
        break_misses,
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
        stage!(paragraph_history_transition),
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
        "gentle-profile isolated stage means (ms): path={} revision_setup={:.3} restart_fork={:.3} executor={:.3} executor_shell={:.3} diagnostics_effects_snapshot={:.3} paragraph_history_publish_drop={:.3} splice={:.3} substrate_publish_drop={:.3} acceptance={:.3} unaccounted_system_noise={:.3} dvi_materialization={:.3}",
        path.name(),
        stage!(revision_setup),
        stage!(restart_fork),
        stage!(executor),
        stage!(executor_shell),
        stage!(output_snapshot),
        stage!(paragraph_history_transition),
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
        reuse.paragraph_history_transition_latency.as_micros(),
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
        "gentle-profile state hash journal: {name}: edit={edit} calls={} journal_entries={} changed_cells={} peak_changed_scratch_bytes={}",
        sample.state_hash.calls,
        sample.state_hash.journal_entries,
        sample.state_hash.changed_cells,
        sample.state_hash.peak_changed_cell_scratch_bytes,
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
        "gentle-profile paragraph detail: {name}: edit={edit} eligible={} barriers={} validation_misses={} line_hits={} commands_skipped={} cold_starts_anchored={} cold_starts_unanchored={} cold_starts_blocked={} barrier_display_math={} barrier_scantokens={} barrier_input_open={} barrier_endinput={} barrier_world={} barrier_output={} barrier_unsupported_write={} barrier_unsupported_input_transition={} barrier_unsupported_group_transition={} validation_reasons={}",
        memo_delta!(paragraph_eligible_regions),
        memo_delta!(paragraph_barriers),
        memo_delta!(paragraph_validation_misses),
        memo_delta!(paragraph_line_hits),
        memo_delta!(paragraph_commands_skipped),
        memo_delta!(paragraph_anchored_cold_starts),
        memo_delta!(paragraph_unanchored_cold_starts),
        memo_delta!(paragraph_blocked_cold_starts),
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
            "gentle-profile paragraph recording phases: {name}: edit={edit} timer_samples={} calibrated_timer_pair_floor_ns={} estimated_measurement_floor_ns={} front_end_dependency_ns={} input_transition_ns={} region_publication_ns={} break_dependency_ns={} break_key_discovery_ns={} break_stamp_registration_ns={} break_value_projection_ns={} line_provenance_ns={} line_retention_ns={}",
            phases.timer_samples,
            _timer_pair_floor_ns,
            phases.timer_samples.saturating_mul(_timer_pair_floor_ns),
            phases.front_end_dependency_nanos,
            phases.input_transition_nanos,
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
        ParagraphValidationFailure::ParagraphStart,
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
        break_dependency_edit: 4,
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
    #[cfg(feature = "profiling-stats")]
    let priming_state_hash_before = state_hash_measurement();
    let priming_started = Instant::now();
    let (input, font) = resolvers.resolvers();
    session
        .cold_with_resolvers(input, font)
        .map_err(|error| format!("prepare incremental baseline: {error}"))?;
    let priming_elapsed = priming_started.elapsed();
    #[cfg(feature = "profiling-stats")]
    let priming_state_hash = state_hash_delta(state_hash_measurement(), priming_state_hash_before);
    let priming_memo = session.pure_memo_stats();
    let mut steps = Vec::with_capacity(fixture.edits.len());
    for (index, edit) in fixture.edits.iter().enumerate() {
        let previous_memo = session.pure_memo_stats();
        #[cfg(feature = "profiling-stats")]
        let exact_before = exact_identity_measurement();
        #[cfg(feature = "profiling-stats")]
        let state_hash_before = state_hash_measurement();
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
        let state_hash_after = state_hash_measurement();
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
            state_hash: state_hash_delta(state_hash_after, state_hash_before),
            #[cfg(feature = "profiling-stats")]
            survivor: survivor_delta(survivor_after, survivor_before),
        });
    }
    Ok(IncrementalSample {
        priming_elapsed,
        priming_memo,
        #[cfg(feature = "profiling-stats")]
        priming_state_hash,
        steps,
    })
}

#[cfg(feature = "profiling-stats")]
fn state_hash_delta(
    after: StateHashMeasurement,
    before: StateHashMeasurement,
) -> StateHashMeasurement {
    StateHashMeasurement {
        calls: after.calls.saturating_sub(before.calls),
        journal_entries: after.journal_entries.saturating_sub(before.journal_entries),
        changed_cells: after.changed_cells.saturating_sub(before.changed_cells),
        node_frames: after.node_frames.saturating_sub(before.node_frames),
        owned_node_bytes: after
            .owned_node_bytes
            .saturating_sub(before.owned_node_bytes),
        owned_font_keys: after.owned_font_keys.saturating_sub(before.owned_font_keys),
        peak_changed_cell_scratch_bytes: after.peak_changed_cell_scratch_bytes,
        peak_node_scratch_bytes: after.peak_node_scratch_bytes,
        components: core::array::from_fn(|index| {
            tex_state::measurement::StateHashComponentMeasurement {
                calls: after.components[index]
                    .calls
                    .saturating_sub(before.components[index].calls),
                visits: after.components[index]
                    .visits
                    .saturating_sub(before.components[index].visits),
                nanos: after.components[index]
                    .nanos
                    .saturating_sub(before.components[index].nanos),
            }
        }),
    }
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
        epoch_source_words: after
            .epoch_source_words
            .saturating_sub(before.epoch_source_words),
        survivor_source_words: after
            .survivor_source_words
            .saturating_sub(before.survivor_source_words),
        epoch_source_lists: after
            .epoch_source_lists
            .saturating_sub(before.epoch_source_lists),
        survivor_source_lists: after
            .survivor_source_lists
            .saturating_sub(before.survivor_source_lists),
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
        "Usage: gentle-profile [--iterations N] [--warmups N] [--repo-root PATH] [--checkpoints] [--cold-memo-layers disabled|LIST] [--incremental-edit] [--incremental-path fast|slow|neutral] [--stabilization-replay] [--baseline-memo-layers LIST] [--memo-layers LIST]\n\n\
         Loads Gentle and its support files once, then executes fresh deterministic\n\
         in-memory Umber sessions for profiling. Defaults: {DEFAULT_ITERATIONS} measured\n\
         iterations and {DEFAULT_WARMUPS} warm-up. --checkpoints captures and hashes every\n\
         named executor checkpoint through a bounded profiling sink.\n\
         --incremental-edit compares a memo baseline, memo candidate, and cold compilation\n\
         five accepted edits/session using balanced AB/BA pairs and DVI parity verification;\n\
         the fifth changes a line-breaking dependency to verify one-shot cold fallback.\n\
         --incremental-path repeatedly ping-pongs one fast, slow, or output-neutral edit after cold setup,\n\
         verifies each direction against cold output, and isolates its sampled stacks.\n\
         --stabilization-replay compares sixteen unchanged-root generated-input passes\n\
         with paragraph recording disabled/enabled in balanced AB/BA session order.\n\
         --cold-memo-layers repeats fresh incremental-session cold compiles with memoization\n\
         disabled or enabled for the selected layers, isolating recording overhead.\n\
         --memo-layers configures enabled recording layers; the default is paragraph.\n\
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
        assert!(!recording.paragraphs);
        assert!(!recording.pages);
        assert!(!recording.shipouts);
    }
}
