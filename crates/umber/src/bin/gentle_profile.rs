//! Persistent in-process Gentle profiling workload.

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tex_exec::{CheckpointSink, EngineCheckpoint};
#[cfg(feature = "profiling-stats")]
use tex_lex::ExpansionStats;
use tex_lex::{InputStack, WorldInput};
use tex_state::{JobClock, Universe, World};
use umber::{EngineSession, FileSessionResolvers, dvi_from_page_plans, prepare_run_stores};

const JOB_DIR: &str = "/gentle-profile";
const JOB_FILE: &str = "profile-job.tex";
const DEFAULT_ITERATIONS: usize = 50;
const DEFAULT_WARMUPS: usize = 1;

#[derive(Debug)]
struct Options {
    repo_root: PathBuf,
    iterations: usize,
    warmups: usize,
    checkpoints: bool,
    expansion_memo: bool,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut iterations = DEFAULT_ITERATIONS;
        let mut warmups = DEFAULT_WARMUPS;
        let mut checkpoints = false;
        let mut expansion_memo = false;
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

fn print_help() {
    println!(
        "Usage: gentle-profile [--iterations N] [--warmups N] [--repo-root PATH] [--checkpoints] [--expansion-memo]\n\n\
         Loads Gentle and its support files once, then executes fresh deterministic\n\
         in-memory Umber sessions for profiling. Defaults: {DEFAULT_ITERATIONS} measured\n\
         iterations and {DEFAULT_WARMUPS} warm-up. --checkpoints captures and hashes every\n\
         named executor checkpoint through a bounded profiling sink. --expansion-memo enables\n\
         the bounded session-local expansion caches and reports their work and retention."
    );
}
