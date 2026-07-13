//! Persistent in-process Gentle profiling workload.

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tex_expand::ExpansionHooks;
#[cfg(feature = "expansion-stats")]
use tex_lex::ExpansionStats;
use tex_lex::{InputStack, WorldInput};
use tex_state::{FileContent, InputReadState, JobClock, Universe, World};
use umber::{EngineSession, dvi_from_page_plans, prepare_run_stores};

const JOB_DIR: &str = "/gentle-profile";
const JOB_FILE: &str = "profile-job.tex";
const DEFAULT_ITERATIONS: usize = 50;
const DEFAULT_WARMUPS: usize = 1;

#[derive(Debug)]
struct Options {
    repo_root: PathBuf,
    iterations: usize,
    warmups: usize,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut iterations = DEFAULT_ITERATIONS;
        let mut warmups = DEFAULT_WARMUPS;
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
        }))
    }
}

struct ProfileHooks {
    base_dir: PathBuf,
}

impl ProfileHooks {
    fn new() -> Self {
        Self {
            base_dir: PathBuf::from(JOB_DIR),
        }
    }
}

impl ExpansionHooks<WorldInput> for ProfileHooks {
    fn open_input<C: InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<WorldInput, String> {
        let mut requested = PathBuf::from(name);
        if requested.extension().is_none() {
            requested.set_extension("tex");
        }
        input
            .read_input_file(&self.base_dir.join(requested))
            .map(WorldInput::from_content)
            .map_err(|error| error.to_string())
    }

    fn open_font<C: InputReadState>(
        &mut self,
        input: &mut C,
        path: &Path,
    ) -> Result<FileContent, String> {
        let mut requested = path.to_owned();
        if requested.extension().is_none() {
            requested.set_extension("tfm");
        }
        input
            .read_input_file(&self.base_dir.join(requested))
            .map_err(|error| error.to_string())
    }

    fn job_name(&self) -> &str {
        "gentle-profile"
    }
}

struct RunOutput {
    dvi: Vec<u8>,
    pages: usize,
    #[cfg(feature = "expansion-stats")]
    expansion_stats: ExpansionStats,
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

    let reference = execute_once(&template)?;
    for _ in 1..options.warmups {
        let output = execute_once(&template)?;
        if output.dvi != reference.dvi {
            return Err("a warm-up DVI differs from the first warm-up DVI".to_owned());
        }
    }

    let started = Instant::now();
    let mut last = execute_once(&template)?;
    let _ = black_box(last.pages);
    let _ = black_box(last.dvi.len());
    for _ in 1..options.iterations {
        last = execute_once(&template)?;
        let _ = black_box(last.pages);
        let _ = black_box(last.dvi.len());
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

fn execute_once(template: &World) -> Result<RunOutput, String> {
    let mut stores = Universe::with_world(template.clone());
    prepare_run_stores(&mut stores);
    let path = Path::new(JOB_DIR).join(JOB_FILE);
    let content = stores
        .world_mut()
        .read_file(&path)
        .map_err(|error| error.to_string())?;
    let mut input = InputStack::new(WorldInput::from_content(content));
    let mut hooks = ProfileHooks::new();
    let run = EngineSession::new(&mut input, &mut stores, &mut hooks)
        .execute()
        .map_err(|error| error.format_with_provenance(&stores))?;
    if run.artifacts.is_empty() {
        return Err("Gentle produced no page artifacts".to_owned());
    }
    let dvi = dvi_from_page_plans(&run.dvi_pages).map_err(|error| error.to_string())?;
    Ok(RunOutput {
        dvi,
        pages: run.artifacts.len(),
        #[cfg(feature = "expansion-stats")]
        expansion_stats: input.expansion_stats(),
    })
}

fn print_summary(options: &Options, output: &RunOutput, elapsed: Duration) {
    let mean = elapsed.as_secs_f64() * 1_000.0 / options.iterations as f64;
    println!(
        "gentle-profile: {} measured runs after {} warm-up(s): {:.3}s total, {:.3}ms mean; {} pages, {} DVI bytes",
        options.iterations,
        options.warmups,
        elapsed.as_secs_f64(),
        mean,
        output.pages,
        output.dvi.len()
    );
    #[cfg(feature = "expansion-stats")]
    println!(
        "gentle-profile expansion: token_frame_steps={} provenance_resolutions={} character_tokens={} character_fraction={:.6} meaning_lookups={} literal_spans={} literal_tokens={} mean_literal_run={:.6} segmentation_cache_hits={} segmentation_cache_misses={} builder_appends={}",
        output.expansion_stats.token_frame_steps,
        output.expansion_stats.provenance_resolutions,
        output.expansion_stats.character_tokens,
        output.expansion_stats.character_fraction(),
        output.expansion_stats.meaning_lookups,
        output.expansion_stats.literal_spans,
        output.expansion_stats.literal_tokens,
        output.expansion_stats.mean_literal_run(),
        output.expansion_stats.segmentation_cache_hits,
        output.expansion_stats.segmentation_cache_misses,
        output.expansion_stats.builder_appends,
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

fn print_help() {
    println!(
        "Usage: gentle-profile [--iterations N] [--warmups N] [--repo-root PATH]\n\n\
         Loads Gentle and its support files once, then executes fresh deterministic\n\
         in-memory Umber sessions for profiling. Defaults: {DEFAULT_ITERATIONS} measured\n\
         iterations and {DEFAULT_WARMUPS} warm-up."
    );
}
