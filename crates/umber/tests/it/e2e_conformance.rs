use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use parity_harness::{compare_dvi_files, run_named_fixture_document};
use tex_expand::ExpansionHooks;
use tex_lex::{InputStack, WorldInput};
use tex_state::{FileContent, InputReadState, JobClock, Universe, World};

use umber::{EngineSession, dvi_from_page_plans, prepare_etex_run_stores, prepare_run_stores};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repository root")
}

fn target_dir(repo_root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map_or_else(
            || repo_root.join("target"),
            |path| {
                if path.is_absolute() {
                    path
                } else {
                    repo_root.join(path)
                }
            },
        )
}

struct InProcessRun {
    dvi: Option<Vec<u8>>,
    format: Option<Vec<u8>>,
}

struct InProcessHooks {
    base_dir: PathBuf,
    job_name: String,
}

impl InProcessHooks {
    fn new(path: &Path) -> Self {
        Self {
            base_dir: path.parent().unwrap_or_else(|| Path::new(".")).to_owned(),
            job_name: path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("texput")
                .to_owned(),
        }
    }
}

impl ExpansionHooks<WorldInput> for InProcessHooks {
    fn open_input<C: InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<WorldInput, String> {
        let mut requested = PathBuf::from(name);
        if requested.extension().is_none() {
            requested.set_extension("tex");
        }
        let staged = self.base_dir.join(&requested);
        input
            .read_input_file(&staged)
            .or_else(|_| input.read_input_file(&requested))
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
        &self.job_name
    }
}

#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn run_file_in_process(
    path: &Path,
    format: Option<&[u8]>,
    etex: bool,
) -> Result<InProcessRun, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?;
    let mut world = World::memory_with_clock(JobClock {
        time: 13 * 60 + 36,
        day: 9,
        month: 7,
        year: 2026,
    });
    for entry in fs::read_dir(parent)
        .map_err(|error| format!("read staged directory {}: {error}", parent.display()))?
    {
        let entry = entry.map_err(|error| format!("read staged directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?;
        if file_type.is_file() {
            let bytes = fs::read(entry.path())
                .map_err(|error| format!("read {}: {error}", entry.path().display()))?;
            world
                .set_memory_file(entry.path(), bytes)
                .map_err(|error| error.to_string())?;
        }
    }

    let mut stores = if let Some(format) = format {
        Universe::from_format(world, format).map_err(|error| error.to_string())?
    } else {
        let mut stores = Universe::with_world(world);
        if etex {
            prepare_etex_run_stores(&mut stores);
        } else {
            prepare_run_stores(&mut stores);
        }
        stores
    };
    if etex && format.is_some() {
        tex_exec::install_etex_unexpandable_primitives(&mut stores);
    }
    let content = stores
        .world_mut()
        .read_file(&path)
        .map_err(|error| error.to_string())?;
    let mut input = InputStack::new(WorldInput::from_content(content));
    let mut hooks = InProcessHooks::new(&path);
    let run = EngineSession::new(&mut input, &mut stores, &mut hooks)
        .execute()
        .map_err(|error| error.format_with_provenance(&stores))?;
    let dvi = if run.artifacts.is_empty() {
        None
    } else {
        Some(dvi_from_page_plans(&run.dvi_pages).map_err(|error| error.to_string())?)
    };
    let format = if run.dumped_format {
        Some(stores.dump_format().map_err(|error| error.to_string())?)
    } else {
        None
    };
    Ok(InProcessRun { dvi, format })
}

fn plain_inputs_available(root: &Path, document: &str, fixture: &Path) -> bool {
    let corpus = root.join("third_party/corpus");
    corpus.join(document).is_file()
        && corpus.join("plain.tex").is_file()
        && root.join("third_party/hyphen/hyphen.tex").is_file()
        && fixture.is_file()
}

fn run_plain_fixture_case(document: &str, fixture_name: &str) {
    let root = repo_root();
    let fixture = root
        .join("tests/corpus/e2e")
        .join(format!("{fixture_name}.expected.dvi"));
    if !plain_inputs_available(&root, document, &fixture) {
        eprintln!(
            "skipping {document} end-to-end conformance: an external input or locally generated DVI oracle is absent; run scripts/setup-conformance-tests.sh"
        );
        return;
    }
    run_named_fixture_document(&root, document, &fixture, |path| {
        run_file_in_process(path, None, false)?
            .dvi
            .ok_or_else(|| "Umber did not produce DVI".to_owned())
    })
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
fn e2e_conformance_story() {
    run_plain_fixture_case("story.tex", "story");
}

#[test]
fn e2e_conformance_gentle() {
    run_plain_fixture_case("gentle.tex", "gentle");
}

#[allow(clippy::disallowed_methods)] // Host-side fixture staging and artifact comparison.
fn run_two_phase_fixture(source_name: &str, local_name: &str, fixture_name: &str, etex: bool) {
    let root = repo_root();
    let trip_dir = root.join("third_party/trip");
    let fixture = root
        .join("tests/corpus/e2e")
        .join(format!("{fixture_name}.expected.dvi"));
    let source = trip_dir.join(source_name);
    let tfm = trip_dir.join("trip.tfm");
    if !source.is_file() || !tfm.is_file() || !fixture.is_file() {
        eprintln!(
            "skipping {fixture_name} conformance: an external input or locally generated DVI oracle is absent; run scripts/setup-conformance-tests.sh"
        );
        return;
    }

    let temp = tempfile::tempdir().expect("create two-phase conformance directory");
    let source_bytes = fs::read(&source).expect("read conformance source");
    let source_bytes = if etex {
        let source = String::from_utf8(source_bytes).expect("e-TRIP source is UTF-8");
        format!(
            "%% Local e-TeX 2.6 compatibility adaptation; the official etrip.tex remains unchanged.\n%% Renamed and modified as required by the e-TeX distribution terms.\n{}",
            source.replace("\\def\\etripversion{2.0}", "\\def\\etripversion{2.6}")
        )
        .into_bytes()
    } else {
        source_bytes
    };
    let input = temp.path().join(local_name);
    fs::write(&input, source_bytes).expect("stage conformance source");
    fs::copy(&tfm, temp.path().join(format!("{fixture_name}.tfm"))).expect("stage conformance TFM");

    let initial = run_file_in_process(&input, None, etex)
        .unwrap_or_else(|error| panic!("{fixture_name} format creation failed: {error}"));
    let format = initial
        .format
        .unwrap_or_else(|| panic!("{fixture_name} did not dump a format"));
    let loaded = run_file_in_process(&input, Some(&format), etex)
        .unwrap_or_else(|error| panic!("{fixture_name} format-loaded run failed: {error}"));
    let dvi = loaded
        .dvi
        .unwrap_or_else(|| panic!("{fixture_name} did not produce DVI"));
    let actual = target_dir(&root)
        .join("conformance-artifacts")
        .join(format!("{fixture_name}.dvi"));
    fs::create_dir_all(actual.parent().expect("artifact parent"))
        .expect("create conformance artifact directory");
    fs::write(&actual, dvi).expect("write conformance artifact");
    compare_dvi_files(
        &fixture,
        &actual,
        &target_dir(&root).join("conformance-triage"),
        fixture_name,
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
fn e2e_conformance_trip() {
    run_two_phase_fixture("trip.tex", "trip.tex", "trip", false);
}

#[test]
fn e2e_conformance_etrip() {
    run_two_phase_fixture("etrip.tex", "etrip-local.tex", "etrip", true);
}
