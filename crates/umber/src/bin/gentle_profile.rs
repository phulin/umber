//! Persistent in-process Gentle profiling workload.

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tex_exec::{CheckpointSink, EngineCheckpoint};
use tex_incr::{AcceptedOutput, Edit, ReuseMetrics, RevisionId, Session};
#[cfg(feature = "profiling-stats")]
use tex_lex::ExpansionStats;
use tex_lex::{InputStack, WorldInput};
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

#[derive(Debug)]
struct Options {
    repo_root: PathBuf,
    iterations: usize,
    warmups: usize,
    checkpoints: bool,
    expansion_memo: bool,
    incremental_edit: bool,
    memo_recording: PureMemoRecordingPolicy,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut iterations = DEFAULT_ITERATIONS;
        let mut warmups = DEFAULT_WARMUPS;
        let mut checkpoints = false;
        let mut expansion_memo = false;
        let mut incremental_edit = false;
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
                "--expansion-memo" => expansion_memo = true,
                "--incremental-edit" => incremental_edit = true,
                "--memo-layers" => {
                    memo_recording = parse_memo_layers(&next_value(&mut args, "--memo-layers")?)?;
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
            expansion_memo,
            incremental_edit,
            memo_recording,
        }))
    }
}

struct RunOutput {
    dvi: Vec<u8>,
    pages: usize,
    checkpoints: usize,
    checkpoint_hash: u64,
    expansion_memo: Option<tex_expand::ExpansionMemoStats>,
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

    let reference = execute_once(&template, options.checkpoints, options.expansion_memo)?;
    for _ in 1..options.warmups {
        let output = execute_once(&template, options.checkpoints, options.expansion_memo)?;
        if output.dvi != reference.dvi {
            return Err("a warm-up DVI differs from the first warm-up DVI".to_owned());
        }
    }

    let started = Instant::now();
    let mut last = execute_once(&template, options.checkpoints, options.expansion_memo)?;
    let _ = black_box(last.pages);
    let _ = black_box(last.dvi.len());
    let _ = black_box((last.checkpoints, last.checkpoint_hash));
    for _ in 1..options.iterations {
        last = execute_once(&template, options.checkpoints, options.expansion_memo)?;
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
    body_offset: usize,
    body_len: usize,
    inserted_bytes: usize,
    inserted_words: usize,
}

struct IncrementalSample {
    steps: Vec<IncrementalStep>,
}

struct IncrementalStep {
    elapsed: Duration,
    dvi: Vec<u8>,
    pages: usize,
    reuse: ReuseMetrics,
    memo: PureMemoStats,
    previous_memo: PureMemoStats,
}

fn run_incremental_edit(options: &Options, template: &World) -> Result<(), String> {
    if options.checkpoints || options.expansion_memo {
        return Err(
            "--incremental-edit cannot be combined with --checkpoints or --expansion-memo"
                .to_owned(),
        );
    }
    if !options.iterations.is_multiple_of(2) {
        return Err(
            "--incremental-edit requires an even --iterations count for balanced AB/BA pairing"
                .to_owned(),
        );
    }
    let fixture = incremental_fixture(&options.repo_root)?;

    for _ in 0..options.warmups {
        let _ = execute_incremental_sample(template, &fixture, false, options.memo_recording)?;
        let _ = execute_incremental_sample(template, &fixture, true, options.memo_recording)?;
        for (index, source) in fixture.revisions.iter().enumerate() {
            let _ = execute_cold_sample(template, source, RevisionId::new(index as u64 + 2))?;
        }
    }

    let edit_count = fixture.edits.len();
    let mut disabled = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut enabled = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut cold = vec![Vec::with_capacity(options.iterations); edit_count];
    let mut paired_millis = vec![Vec::with_capacity(options.iterations); edit_count];
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
            let sample =
                execute_incremental_sample(template, &fixture, memo, options.memo_recording)?;
            for (index, step) in sample.steps.iter().enumerate() {
                if memo {
                    enabled[index].push(step.elapsed);
                } else {
                    disabled[index].push(step.elapsed);
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
    }

    let disabled_sample = last_disabled.expect("at least one disabled sample");
    let enabled_sample = last_enabled.expect("at least one enabled sample");
    for (index, cold_output) in cold_reference.iter().enumerate() {
        let cold_output = cold_output.as_ref().expect("at least one cold sample");
        let expected = cold_output.dvi_bytes().map_err(|error| error.to_string())?;
        for (name, sample) in [
            ("memo-disabled", &disabled_sample),
            ("memo-enabled", &enabled_sample),
        ] {
            if sample.steps[index].dvi != expected {
                return Err(format!(
                    "{name} incremental edit {} DVI differs from its cold DVI",
                    index + 1
                ));
            }
        }
    }

    println!(
        "gentle-profile incremental edit: byte={} ({:.2}% through gentle.tex), inserted_bytes={} inserted_words={} into one paragraph; three accepted edits/session; {} AB/BA-paired runs after {} warm-up(s)",
        fixture.body_offset,
        fixture.body_offset as f64 * 100.0 / fixture.body_len as f64,
        fixture.inserted_bytes,
        fixture.inserted_words,
        options.iterations,
        options.warmups,
    );
    for index in 0..edit_count {
        let disabled_stats = duration_stats(&disabled[index]);
        let enabled_stats = duration_stats(&enabled[index]);
        let cold_stats = duration_stats(&cold[index]);
        let paired = scalar_stats(&paired_millis[index]);
        println!("gentle-profile accepted edit {}:", index + 1);
        print_duration_stats("memo disabled", disabled_stats);
        print_duration_stats("memo enabled", enabled_stats);
        print_duration_stats("cold", cold_stats);
        println!(
            "gentle-profile paired delta: edit={}: enabled-disabled mean={:+.3}ms median={:+.3}ms min={:+.3}ms max={:+.3}ms",
            index + 1,
            paired.mean,
            paired.median,
            paired.min,
            paired.max,
        );
        print_incremental_work("memo disabled", index + 1, &disabled_sample.steps[index]);
        print_incremental_work("memo enabled", index + 1, &enabled_sample.steps[index]);
        println!(
            "gentle-profile incremental output: edit={}: {} pages, {} DVI bytes; both incremental modes are byte-identical to cold",
            index + 1,
            enabled_sample.steps[index].pages,
            enabled_sample.steps[index].dvi.len(),
        );
    }
    Ok(())
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

fn print_incremental_work(name: &str, edit: usize, sample: &IncrementalStep) {
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
        "gentle-profile incremental work: {name}: edit={edit} pages_retyped={} pages_reused={} paragraphs_reexecuted={} bytes_reexecuted={} tokens_reexecuted={} commands_reexecuted={} exact_checks={} suffixes_adopted={} fork_us={} reexecute_us={} splice_us={}",
        reuse.pages_retyped,
        reuse.pages_reused,
        reuse.reexecuted_paragraphs,
        reuse.reexecuted_bytes,
        reuse.reexecuted_tokens,
        reuse.reexecuted_commands,
        reuse.same_history_attempts,
        reuse.suffixes_adopted,
        reuse.restart_fork_latency.as_micros(),
        reuse.reexecution_latency.as_micros(),
        reuse.splice_latency.as_micros(),
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
        "gentle-profile memo retention: {name}: edit={edit} detached_cache_bytes={} paragraph_generation_metadata_bytes={}",
        sample.memo.retained_bytes, sample.memo.paragraph_generation_metadata_bytes,
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
    Ok(IncrementalFixture {
        original,
        revisions: vec![edited.clone(), followed_up, edited],
        edits: vec![edit_one, edit_two, edit_three],
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
    let (input, font) = resolvers.resolvers();
    session
        .cold_with_resolvers(input, font)
        .map_err(|error| format!("prepare incremental baseline: {error}"))?;
    let mut steps = Vec::with_capacity(fixture.edits.len());
    for (index, edit) in fixture.edits.iter().enumerate() {
        let previous_memo = session.pure_memo_stats();
        let mut resolvers = FileSessionResolvers::new(&path, Vec::new(), Vec::new());
        let started = Instant::now();
        let (input, font) = resolvers.resolvers();
        let accepted = session
            .advance_with_resolvers(RevisionId::new(index as u64 + 2), edit.clone(), input, font)
            .map_err(|error| format!("advance incremental edit {}: {error}", index + 1))?;
        let elapsed = started.elapsed();
        let memo = session.pure_memo_stats();
        let dvi = accepted.dvi_bytes().map_err(|error| error.to_string())?;
        let _ = black_box(accepted.artifacts.len());
        steps.push(IncrementalStep {
            elapsed,
            dvi,
            pages: accepted.artifacts.len(),
            reuse: accepted.reuse,
            memo,
            previous_memo,
        });
    }
    Ok(IncrementalSample { steps })
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

fn execute_once(
    template: &World,
    capture_checkpoints: bool,
    expansion_memo: bool,
) -> Result<RunOutput, String> {
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
    let mut context = resolvers.context();
    if expansion_memo {
        context = context.memoizing(tex_expand::ExpansionMemoConfig::default());
    }
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
    let expansion_memo = session.expansion_memo_stats();
    if run.artifacts.is_empty() {
        return Err("Gentle produced no page artifacts".to_owned());
    }
    let dvi = dvi_from_page_plans(&run.dvi_pages).map_err(|error| error.to_string())?;
    Ok(RunOutput {
        dvi,
        pages: run.artifacts.len(),
        checkpoints: checkpoints.count,
        checkpoint_hash: checkpoints.hash,
        expansion_memo,
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
    if let Some(memo) = output.expansion_memo {
        println!(
            "gentle-profile expansion memo: substitution={}/{} episode={}/{} reused_tokens={} retained_entries={} retained_bytes={} evictions={} lookup_ns={}",
            memo.substitution_hits,
            memo.substitution_lookups,
            memo.episode_hits,
            memo.episode_lookups,
            memo.substituted_tokens_reused
                .saturating_add(memo.expanded_tokens_reused),
            memo.retained_entries,
            memo.retained_bytes,
            memo.evictions,
            memo.lookup_nanos,
        );
    }
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
        "Usage: gentle-profile [--iterations N] [--warmups N] [--repo-root PATH] [--checkpoints] [--expansion-memo] [--incremental-edit] [--memo-layers LIST]\n\n\
         Loads Gentle and its support files once, then executes fresh deterministic\n\
         in-memory Umber sessions for profiling. Defaults: {DEFAULT_ITERATIONS} measured\n\
         iterations and {DEFAULT_WARMUPS} warm-up. --checkpoints captures and hashes every\n\
         named executor checkpoint through a bounded profiling sink. --expansion-memo enables\n\
         the bounded session-local expansion caches and reports their work and retention.\n\
         --incremental-edit compares memo-disabled, memo-enabled, and cold compilation for\n\
         three accepted edits/session using balanced AB/BA pairs and DVI parity verification.\n\
         --memo-layers configures enabled recording layers; the default is paragraph."
    );
}
