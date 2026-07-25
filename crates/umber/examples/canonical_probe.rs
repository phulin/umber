//! Reusable direct canonical Gentle/Story e2e divergence probe.
//!
//! The `umber2-johp` epic converges Umber against instrumented TeX/pdfTeX by
//! fixing one earliest divergence at a time. Each successive fix has had to
//! reconstruct, from scratch, an ad hoc in-process probe that drives Plain
//! bootstrap plus a corpus source through `CanonicalEngineSession` and
//! reports where it first fails. This example commits that probe so a
//! cold-start agent can reproduce the current earliest divergence in one
//! command instead of rebuilding the harness.
//!
//! # Usage
//!
//! ```text
//! cargo run -p umber --example canonical_probe -- gentle
//! cargo run -p umber --example canonical_probe -- story
//! ```
//!
//! The argument names a document in `third_party/corpus/<name>.tex` (the
//! `.tex` suffix is optional); it defaults to `gentle`. The probe requires
//! `third_party/corpus/plain.tex`, `third_party/corpus/<name>.tex`, and
//! `third_party/hyphen/hyphen.tex` (fetched by
//! `scripts/setup-conformance-tests.sh` or `scripts/fetch-conformance-inputs.sh`),
//! plus the plain-format Computer Modern/`manfnt` TFMs, resolved from the
//! committed `crates/tex-fonts/tests/fixtures/cm` fixtures, the gitignored
//! `third_party/fonts` cache, or `kpsewhich` on `PATH` (in that order; see
//! `parity_harness::locate_tfm`).
//!
//! On success it reports the number of artifacts and DVI pages produced. On
//! the first divergence -- an `ExecError`/`CanonicalSessionError`, or a Rust
//! panic -- it reports the live execution mode, the canonical error (with
//! provenance-resolved TeX source context when the error carries an origin),
//! and, for panics, lets the default panic hook report the Rust-side
//! `file:line` origin. Run with `RUST_BACKTRACE=1` for a full Rust backtrace.
//!
//! This is a diagnostic entry point, not the production e2e migration (that
//! is tracked separately as `umber2-johp.28`). It is expected to fail today:
//! see `umber2-johp.58` for the currently tracked earliest Gentle divergence
//! this probe reproduces.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use tex_command::{CommandProfile, FontResource};
use tex_exec::{CanonicalResourceNeed, EngineCheckpoint, ExecError};
use tex_state::{JobClock, Universe, World};
use umber::{
    CanonicalEngineSession, CanonicalResourceFulfillment, CanonicalResourceHost,
    CanonicalResourceWorld, CanonicalSessionError,
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
                "canonical_probe: missing required external input {}; run \
                 scripts/setup-conformance-tests.sh (or scripts/fetch-conformance-inputs.sh) first",
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

    let mut stores = Universe::with_world(world);
    tex_expand::install_expandable_primitives(&mut stores);
    tex_exec::install_unexpandable_primitives(&mut stores);

    let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
    let root_source = format!("\\input plain.tex \\input {source}.tex\n");
    session
        .register_authored_root("job.tex", Arc::from(root_source.into_bytes()))
        .expect("register the probe's authored root");

    let mut host = CorpusHost;
    let mut checkpoints: Vec<EngineCheckpoint> = Vec::new();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.run(&mut host, &mut checkpoints)
    }));

    match outcome {
        Ok(Ok(run)) => {
            println!(
                "canonical_probe: {source} completed with no divergence: {} artifact(s), {} DVI page(s)",
                run.artifacts.len(),
                run.dvi_pages.len()
            );
            ExitCode::SUCCESS
        }
        Ok(Err(error)) => {
            report_error(&source, &session, &error);
            ExitCode::FAILURE
        }
        Err(panic) => {
            report_panic(&source, &session, panic.as_ref());
            ExitCode::FAILURE
        }
    }
}

fn report_error(source: &str, session: &CanonicalEngineSession<'_>, error: &CanonicalSessionError) {
    eprintln!("canonical_probe: {source} run diverged");
    eprintln!("current mode: {:?}", session.current_mode());
    match error {
        CanonicalSessionError::Execution(exec_error) => {
            eprintln!("{}", exec_error.format_with_provenance(session.stores()));
            if let ExecError::MissingToken { context } = exec_error
                && *context == "command processor"
            {
                eprintln!(
                    "note: this generic diagnosis usually means CommandProcessor returned a \
                     CommandError that command_error() collapsed to a plain MissingToken in \
                     crates/tex-exec/src/canonical_main_control.rs; see umber2-johp.58 for the \
                     temporary panic-based CommandError recovery recipe (add a panic!() in the \
                     relevant CommandError match arm and re-run with RUST_BACKTRACE=1)."
                );
            }
        }
        other => eprintln!("{other}"),
    }
}

fn report_panic(
    source: &str,
    session: &CanonicalEngineSession<'_>,
    payload: &(dyn std::any::Any + Send),
) {
    // The default panic hook already printed the Rust-side "panicked at
    // src/file.rs:LINE:COL" location (and a backtrace under
    // RUST_BACKTRACE=1) before unwinding reached this catch_unwind boundary.
    eprintln!("canonical_probe: {source} run panicked");
    eprintln!("current mode: {:?}", session.current_mode());
    let message = payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned());
    eprintln!("panic message: {message}");
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repository root")
}

#[allow(clippy::disallowed_methods)] // Host-side probe staging; engine I/O still goes through World.
fn seed_memory_file(world: &mut World, name: &str, path: &Path) {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    world
        .set_memory_file(name, bytes)
        .unwrap_or_else(|error| panic!("seed {name}: {error}"));
}

fn seed_corpus_tfms(world: &mut World, root: &Path) {
    for name in parity_harness::CORPUS_TFMS {
        match parity_harness::locate_tfm(root, name, true) {
            Ok(Some(path)) => seed_memory_file(world, &format!("{name}.tfm"), &path),
            Ok(None) => eprintln!(
                "canonical_probe: warning: could not locate {name}.tfm; requests for it will be declined"
            ),
            Err(error) => eprintln!("canonical_probe: warning: error locating {name}.tfm: {error}"),
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

fn canonical_font_path(name: &str) -> PathBuf {
    let path = PathBuf::from(name);
    if path.extension().is_none() {
        path.with_extension("tfm")
    } else {
        path
    }
}

struct CorpusHost;

impl CanonicalResourceHost for CorpusHost {
    fn fulfill(
        &mut self,
        world: &mut CanonicalResourceWorld<'_>,
        need: &CanonicalResourceNeed,
    ) -> Option<CanonicalResourceFulfillment> {
        match need {
            CanonicalResourceNeed::Input { name } => world
                .read_file(canonical_input_path(name))
                .ok()
                .map(|content| CanonicalResourceFulfillment::world_input(name, content)),
            CanonicalResourceNeed::Font { request } => world
                .read_file(canonical_font_path(&request.name))
                .ok()
                .map(|metrics| CanonicalResourceFulfillment::Font {
                    request: request.clone(),
                    resource: Box::new(FontResource::Tfm {
                        metrics,
                        opentype: None,
                    }),
                }),
            CanonicalResourceNeed::PdfImage { .. } => None,
        }
    }
}
