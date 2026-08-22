//! First-failure locator for the canonical Gentle/Story e2e path.
//!
//! This is a *first-failure locator*: it drives Plain bootstrap plus a corpus
//! source directly through `EngineSession` and reports where
//! execution first stops. It can only show that execution stopped, never that
//! completed output is wrong -- see the Glossary in
//! `docs/canonical_divergence_workflow.md`. The `umber2-johp` epic converges
//! Umber against instrumented TeX/pdfTeX by fixing one earliest divergence at
//! a time; each successive fix had to reconstruct this ad hoc in-process
//! locator from scratch, so this example commits it once so a cold-start
//! agent can reproduce the current earliest failure in one command instead of
//! rebuilding the harness.
//!
//! # Usage
//!
//! ```text
//! cargo run -p umber --example first_failure_locator -- gentle
//! cargo run -p umber --example first_failure_locator -- story
//! cargo run -p umber --example first_failure_locator -- story /tmp/story.actual.dvi
//! ```
//!
//! The first argument names a document in `third_party/corpus/<name>.tex`
//! (the `.tex` suffix is optional); it defaults to `gentle`. An optional
//! second argument is a path to write the assembled DVI bytes to, for
//! comparison against a fixture such as `tests/corpus/e2e/story.expected.dvi`
//! (e.g. with `cmp` or `sha256sum`). The locator requires
//! `third_party/corpus/plain.tex`, `third_party/corpus/<name>.tex`, and
//! `third_party/hyphen/hyphen.tex` (fetched by
//! `python3 scripts/provision.py worktree .`),
//! plus the plain-format Computer Modern/`manfnt` TFMs, resolved from the
//! committed `crates/tex-fonts/tests/fixtures/cm` fixtures, the gitignored
//! `third_party/fonts` cache (see
//! `parity_harness::locate_tfm`).
//!
//! On success it reports the number of artifacts and DVI pages produced. On
//! the first failure -- an `ExecError`/`SessionError`, or a Rust
//! panic -- it reports the live execution mode, the canonical error (with
//! provenance-resolved TeX source context when the error carries an origin),
//! and, for panics, lets the default panic hook report the Rust-side
//! `file:line` origin. Run with `RUST_BACKTRACE=1` for a full Rust backtrace.
//!
//! This is a diagnostic entry point, not the production e2e migration (that
//! is tracked separately as `umber2-johp.28`). Running against `gentle` is
//! expected to fail today: see the current open successor issue under the
//! `umber2-johp` epic (`bd show umber2-johp` for its children, or `bd ready`)
//! for the earliest tracked Gentle divergence this locator reproduces -- that
//! issue ID advances every time a divergence is fixed, so it is deliberately
//! not hardcoded here.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use tex_command::FontResource;
use tex_exec::{EngineCheckpoint, ExecError, ResourceNeed, canonical_font_resource_path};
use tex_state::{JobClock, Universe, World};
use umber::{
    EngineSession, ResourceFulfillment, ResourceHost, ResourceOutcome, ResourceWorld, SessionError,
};

const DEFAULT_SOURCE: &str = "gentle";

fn main() -> ExitCode {
    let source = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_SOURCE.to_owned());
    let source = source.strip_suffix(".tex").unwrap_or(&source).to_owned();

    let root = repo_root();
    let corpus = root.join("third_party/corpus");
    let plain_path = corpus.join("plain.tex");
    let doc_path = corpus.join(format!("{source}.tex"));
    let hyphen_path = root.join("third_party/hyphen/hyphen.tex");
    for path in [&plain_path, &doc_path, &hyphen_path] {
        if !path.is_file() {
            eprintln!(
                "first_failure_locator: missing required external input {}; run \
                 python3 scripts/provision.py worktree . first",
                path.display()
            );
            return ExitCode::from(2);
        }
    }

    let mut world = World::memory_with_clock(JobClock {
        time: 13 * 60 + 36,
        second: 0,
        day: 9,
        month: 7,
        year: 2026,
    });
    seed_memory_file(&mut world, "plain.tex", &plain_path);
    seed_memory_file(&mut world, &format!("{source}.tex"), &doc_path);
    seed_memory_file(&mut world, "hyphen.tex", &hyphen_path);
    seed_corpus_tfms(&mut world, &root);

    umber::with_engine_world(world, |stores| run_locator(stores, &source))
        .expect("create the locator's branded runtime")
}

fn run_locator<G>(stores: &mut Universe<G>, source: &str) -> ExitCode {
    let mut session = EngineSession::tex82_initex(stores);
    let root_source = format!("\\input plain.tex \\input {source}.tex\n");
    session
        .register_authored_job("job.tex", Arc::from(root_source.into_bytes()))
        .expect("register the locator's authored root");

    let mut host = CorpusHost;
    let mut checkpoints: Vec<EngineCheckpoint<G>> = Vec::new();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.run(&mut host, &mut checkpoints)
    }));

    match outcome {
        Ok(Ok(run)) => {
            println!(
                "first_failure_locator: {source} completed with no failure: {} artifact(s), {} DVI page(s)",
                run.artifacts.len(),
                run.dvi_pages.len()
            );
            if !run.dvi_pages.is_empty() {
                match umber::dvi_from_page_plans(&run.dvi_pages) {
                    Ok(dvi) => {
                        if let Some(out_path) = env::args().nth(2) {
                            if let Err(error) = write_locator_output(&out_path, &dvi) {
                                eprintln!(
                                    "first_failure_locator: failed to write DVI to {out_path}: {error}"
                                );
                            } else {
                                println!(
                                    "first_failure_locator: wrote {} DVI byte(s) to {out_path}",
                                    dvi.len()
                                );
                            }
                        } else {
                            println!("first_failure_locator: assembled {} DVI byte(s)", dvi.len());
                        }
                    }
                    Err(error) => {
                        eprintln!("first_failure_locator: failed to assemble DVI: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Ok(Err(error)) => {
            report_error(source, &mut session, &error);
            ExitCode::FAILURE
        }
        Err(panic) => {
            report_panic(source, &session, panic.as_ref());
            ExitCode::FAILURE
        }
    }
}

fn report_error<G>(source: &str, session: &mut EngineSession<'_, G>, error: &SessionError) {
    eprintln!("first_failure_locator: {source} run stopped");
    eprintln!("current mode: {:?}", session.current_mode());
    match error {
        SessionError::Execution(exec_error) => {
            eprintln!("{}", session.format_execution_error(exec_error));
            if let ExecError::Command(command_error) = exec_error {
                eprintln!(
                    "note: this is a canonical command-core failure ({command_error:?}); \
                     `command_error()` in crates/tex-exec/src/main_control.rs \
                     names every `CommandError` variant explicitly (see umber2-johp.59), so \
                     this message and variant identify the true origin directly -- no debug \
                     panic needed."
                );
            }
        }
        other => eprintln!("{other}"),
    }
}

fn report_panic<G>(
    source: &str,
    session: &EngineSession<'_, G>,
    payload: &(dyn std::any::Any + Send),
) {
    // The default panic hook already printed the Rust-side "panicked at
    // src/file.rs:LINE:COL" location (and a backtrace under
    // RUST_BACKTRACE=1) before unwinding reached this catch_unwind boundary.
    eprintln!("first_failure_locator: {source} run panicked");
    eprintln!("current mode: {:?}", session.current_mode());
    let message = payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned());
    eprintln!("panic message: {message}");
}

fn repo_root() -> PathBuf {
    test_support::repository_root()
        .join("crates/umber")
        .join("../..")
        .canonicalize()
        .expect("resolve repository root")
}

#[allow(clippy::disallowed_methods)] // Host-side locator output; engine I/O still goes through World.
fn write_locator_output(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    fs::write(path, bytes)
}

#[allow(clippy::disallowed_methods)] // Host-side locator staging; engine I/O still goes through World.
fn seed_memory_file(world: &mut World, name: &str, path: &Path) {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    world
        .set_memory_file(name, bytes)
        .unwrap_or_else(|error| panic!("seed {name}: {error}"));
}

fn seed_corpus_tfms(world: &mut World, root: &Path) {
    for name in parity_harness::PLAIN_PRELOAD_FONTS {
        match parity_harness::locate_tfm(root, name) {
            Ok(Some(path)) => seed_memory_file(world, &format!("{name}.tfm"), &path),
            Ok(None) => eprintln!(
                "first_failure_locator: warning: could not locate {name}.tfm; requests for it will be declined"
            ),
            Err(error) => {
                eprintln!("first_failure_locator: warning: error locating {name}.tfm: {error}")
            }
        }
    }
}

fn canonical_input_path(name: &str) -> PathBuf {
    let path = PathBuf::from(name);
    if path.extension().is_none() {
        path.with_extension("tex")
    } else {
        path
    }
}

struct CorpusHost;

impl ResourceHost for CorpusHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        match need {
            ResourceNeed::Input { name, .. } => world
                .read_file(canonical_input_path(name))
                .ok()
                .map_or(ResourceOutcome::Unavailable, |content| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::world_input(name, content))
                }),
            ResourceNeed::InputProbe { request } => world
                .read_file(canonical_input_path(&request.name))
                .ok()
                .map_or(ResourceOutcome::Unavailable, |content| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::world_input_probe(
                        request.clone(),
                        content,
                    ))
                }),
            ResourceNeed::Font { request } => world
                .read_file(canonical_font_resource_path(&request.name))
                .ok()
                .map_or(ResourceOutcome::Unavailable, |metrics| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::Font {
                        request: request.clone(),
                        resource: Box::new(FontResource::Tfm {
                            metrics,
                            opentype: None,
                        }),
                    })
                }),
            ResourceNeed::PdfImage { .. } => ResourceOutcome::Unavailable,
        }
    }
}
