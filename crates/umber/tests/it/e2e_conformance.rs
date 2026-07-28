use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parity_harness::{
    TripTriageChannels, TripTriageInput, TripTriageSource, compare_dvi_files,
    run_named_fixture_document, write_trip_triage_artifact,
};
use sha2::{Digest, Sha256};
use test_support::dvi::normalized_dvi_for_comparison;
use tex_command::FontResource;
use tex_exec::{CanonicalResourceNeed, EngineCheckpoint, ExecutionContext, FontResolver};
use tex_expand::InputResolver;
use tex_lex::{InputStack, WorldInput};
use tex_state::provenance::MacroInvocationProvenanceStats;
use tex_state::provenance::ProvenanceStats;
use tex_state::{InputReadState, JobClock, Universe, World};

use umber::{
    CanonicalEngineSession, CanonicalResourceFulfillment, CanonicalResourceHost,
    CanonicalResourceWorld, CanonicalSessionError, EngineMode, EngineSession, dvi_from_page_plans,
};

#[path = "e2e_conformance/assets.rs"]
mod assets;

use assets::GateAssets;

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
    provenance: ProvenanceStats,
    macro_provenance: MacroInvocationProvenanceStats,
}

struct InProcessInputResolver {
    base_dir: PathBuf,
}

impl InputResolver for InProcessInputResolver {
    fn open_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<Box<dyn tex_lex::InputSource>> {
        let mut path = PathBuf::from(name);
        if path.extension().is_none() {
            path.set_extension("tex");
        }
        input
            .read_input_file(&self.base_dir.join(&path))
            .or_else(|_| input.read_input_file(&path))
            .map(WorldInput::from_content)
            .map(|source| {
                tex_expand::ResourceLookup::Available(
                    Box::new(source) as Box<dyn tex_lex::InputSource>
                )
            })
            .map_err(|error| error.to_string())
    }
}

struct InProcessFontResolver {
    base_dir: PathBuf,
}

impl FontResolver for InProcessFontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        _request_index: u64,
    ) -> tex_expand::ResourceResult<tex_exec::FontSource> {
        let mut path = path.to_owned();
        if path.extension().is_none() {
            path.set_extension("tfm");
        }
        Ok(match input.read_input_file(&self.base_dir.join(&path)) {
            Ok(metrics) => tex_expand::ResourceLookup::Available(tex_exec::FontSource::Tfm {
                metrics,
                opentype: None,
            }),
            Err(_) => tex_expand::ResourceLookup::Unavailable,
        })
    }
}

struct InProcessResolvers {
    input: InProcessInputResolver,
    font: InProcessFontResolver,
    job_name: String,
}

impl InProcessResolvers {
    fn new(path: &Path) -> Self {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_owned();
        Self {
            input: InProcessInputResolver {
                base_dir: base_dir.clone(),
            },
            font: InProcessFontResolver { base_dir },
            job_name: path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("texput")
                .to_owned(),
        }
    }

    fn context(&mut self) -> ExecutionContext<'_> {
        ExecutionContext::with_resolvers(&self.job_name, &mut self.input, &mut self.font)
    }
}

/// Canonicalizes a staged fixture directory's job path and loads every file
/// it contains into a memory `World`, keyed by absolute path so both the
/// legacy and canonical in-process runners can address the same staged
/// inputs (source document, format source, hyphenation data, TFMs) the same
/// way `parity_harness::staged_source_dir` assembled them.
#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn staged_world(path: &Path) -> Result<(World, PathBuf), String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?;
    let mut world = World::memory_with_clock(JobClock {
        time: 13 * 60 + 36,
        second: 0,
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
    Ok((world, path))
}

#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn run_file_in_process(
    path: &Path,
    format: Option<&[u8]>,
    engine: EngineMode,
) -> Result<InProcessRun, String> {
    let (world, path) = staged_world(path)?;

    let mut stores = if let Some(format) = format {
        let mut stores = Universe::from_format(world, format).map_err(|error| error.to_string())?;
        engine.install_after_format(&mut stores);
        stores
    } else {
        let mut stores = Universe::with_world(world);
        engine.prepare_fresh(&mut stores);
        stores
    };
    let content = stores
        .world_mut()
        .read_file(&path)
        .map_err(|error| error.to_string())?;
    let mut input = InputStack::new(WorldInput::from_content(content));
    let mut resolvers = InProcessResolvers::new(&path);
    let context = resolvers
        .context()
        .with_expansion_fuel(tex_expand::DEFAULT_EXPANSION_FUEL);
    let run = EngineSession::new(&mut input, &mut stores, context)
        .execute()
        .map_err(|error| error.format_with_provenance(&stores))?;
    for (index, committed) in run.committed_artifacts.iter().enumerate() {
        let page = tex_out::PageArtifact::from_bytes(committed.bytes())
            .map_err(|error| format!("decode page {} for HTML: {error}", index + 1))?;
        let positioned = tex_out::positioned::lower_page(&page, (index + 1) as u32)
            .map_err(|error| format!("lower page {} for HTML: {error}", index + 1))?;
        tex_out::dvi::coordinates::compare_page(&page, &positioned)
            .map_err(|error| format!("validate page {} HTML coordinates: {error}", index + 1))?;
    }
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
    let provenance = stores.provenance_stats();
    let macro_provenance = stores.macro_invocation_provenance_stats();
    Ok(InProcessRun {
        dvi,
        format,
        provenance,
        macro_provenance,
    })
}

fn run_plain_fixture_case(document: &str, gate: &GateAssets) {
    let fixture_name = gate.name;
    run_named_fixture_document(&gate.repo_root, document, &gate.oracle, |path| {
        let run = run_file_in_process(path, None, EngineMode::Tex82)?;
        let macro_stats = run.macro_provenance;
        let invocations = macro_stats.invocations();
        if invocations == 0 {
            return Err(format!("{document} executed no macro invocations"));
        }
        let macro_bytes = macro_stats.retained_bytes();
        let bytes_per_invocation = macro_stats.bytes_per_invocation();
        eprintln!(
            "{fixture_name} provenance: invocations={invocations} macro_retained_bytes={macro_bytes} bytes_per_invocation={bytes_per_invocation} total_retained_bytes={}",
            run.provenance.retained_bytes(),
        );
        if bytes_per_invocation > 64 {
            return Err(format!(
                "{document} macro provenance retained {bytes_per_invocation} bytes/invocation (budget: 64)"
            ));
        }
        run.dvi
            .ok_or_else(|| "Umber did not produce DVI".to_owned())
    })
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
fn e2e_conformance_story() {
    assets::with_gate("story", |gate| run_plain_fixture_case("story.tex", gate));
}

#[test]
fn e2e_conformance_gentle() {
    assets::with_gate("gentle", |gate| run_plain_fixture_case("gentle.tex", gate));
}

/// Adds an extension inferred from the resource kind (`.tex` for input
/// requests, `.tfm` for font requests) when a canonical resource need names a
/// file without one, mirroring the legacy `InProcessInputResolver`/
/// `InProcessFontResolver`'s own default-extension behavior above.
fn with_default_extension(name: &str, extension: &str) -> PathBuf {
    let mut path = PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension(extension);
    }
    path
}

/// Resolves canonical resource suspensions (`\input`, font loads) directly
/// against the same staged fixture directory the legacy runner reads,
/// keeping the two engines' host-side fixture wiring identical so a DVI
/// difference between them reflects only engine behavior.
struct StagedDirResourceHost {
    base_dir: PathBuf,
}

impl CanonicalResourceHost for StagedDirResourceHost {
    fn fulfill(
        &mut self,
        world: &mut CanonicalResourceWorld<'_>,
        need: &CanonicalResourceNeed,
    ) -> Option<CanonicalResourceFulfillment> {
        match need {
            CanonicalResourceNeed::Input { name } => {
                let path = with_default_extension(name, "tex");
                world
                    .read_file(self.base_dir.join(path))
                    .ok()
                    .map(|content| CanonicalResourceFulfillment::world_input(name, content))
            }
            CanonicalResourceNeed::Font { request } => {
                let path = with_default_extension(&request.name, "tfm");
                world
                    .read_file(self.base_dir.join(path))
                    .ok()
                    .map(|metrics| CanonicalResourceFulfillment::Font {
                        request: request.clone(),
                        resource: Box::new(FontResource::Tfm {
                            metrics,
                            opentype: None,
                        }),
                    })
            }
            CanonicalResourceNeed::PdfImage { .. } => None,
        }
    }
}

/// Renders a canonical session failure the same way `first_failure_locator.rs`
/// does: an execution error gets its provenance-resolved TeX source context,
/// while every other `CanonicalSessionError` variant already carries enough
/// context through its own `Display` impl.
fn canonical_error_message(
    session: &CanonicalEngineSession<'_>,
    error: &CanonicalSessionError,
) -> String {
    match error {
        CanonicalSessionError::Execution(exec_error) => {
            exec_error.format_with_provenance(session.stores())
        }
        other => other.to_string(),
    }
}

/// Drives the same staged fixture job through `CanonicalEngineSession`
/// (the canonical `tex-command` architecture, not the legacy
/// `EngineSession`/`ExecutionContext` path above) and returns its assembled
/// DVI bytes. This is the production migration path's equivalent of
/// `run_file_in_process`, sharing the same staged directory contract so the
/// canonical Story and Gentle regression gates below are real byte-exact
/// checks on the `umber2-johp` canonical/reference DVI milestones.
#[allow(clippy::disallowed_methods)] // Host-side fixture loading; engine I/O still goes through World.
fn run_file_in_process_canonical(path: &Path) -> Result<Vec<u8>, String> {
    let (world, path) = staged_world(path)?;
    let base_dir = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?
        .to_owned();
    let job_name = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("texput")
        .to_owned();
    let job_bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;

    let mut stores = Universe::with_world(world);
    // These staged gates bootstrap plain.tex from source, so this phase is
    // INITEX rather than a cold job loaded from an already-built format.
    let mut session = CanonicalEngineSession::tex82_initex(&mut stores);
    session
        .register_authored_root(&job_name, Arc::from(job_bytes))
        .map_err(|error| format!("register canonical root {job_name}: {error}"))?;

    let mut host = StagedDirResourceHost { base_dir };
    let mut checkpoints: Vec<EngineCheckpoint> = Vec::new();
    let run = session
        .run(&mut host, &mut checkpoints)
        .map_err(|error| canonical_error_message(&session, &error))?;
    if run.dvi_pages.is_empty() {
        return Err("canonical Umber run did not produce DVI".to_owned());
    }
    dvi_from_page_plans(&run.dvi_pages).map_err(|error| error.to_string())
}

fn run_plain_fixture_case_canonical(document: &str, gate: &GateAssets) {
    run_named_fixture_document(
        &gate.repo_root,
        document,
        &gate.oracle,
        run_file_in_process_canonical,
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

/// Protects `umber2-johp`'s first canonical/reference byte-identical DVI
/// milestone (commit 5eed4dc3): the canonical `tex-command`
/// architecture's DVI for `story.tex` must remain byte-identical to real
/// pdfTeX's output after only the same preamble-comment normalization the
/// legacy `e2e_conformance_story` test above already tolerates. Kept
/// alongside (not replacing) the legacy test while the `umber2-johp`
/// migration is in progress; both reach the same registered `story` gate
/// through `assets::with_gate`, so neither can skip silently.
#[test]
fn e2e_conformance_story_canonical() {
    assets::with_gate("story", |gate| {
        run_plain_fixture_case_canonical("story.tex", gate);
    });
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_ligature_group_boundaries_match_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("dvi", "ligature_group_boundaries");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("dvi")
            .join("ligature_group_boundaries.expected.dvi"),
    )
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_rule_space_factor_reset_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("dvi", "rule_space_factor_reset");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("dvi")
            .join("rule_space_factor_reset.expected.dvi"),
    )
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_alignment_leading_tabskip_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "alignment_leading_tabskip");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("alignment_leading_tabskip.expected.dvi"),
    )
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_rule_follows_pending_characters_in_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "rule_character_order");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("rule_character_order.expected.dvi"),
    )
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_relax_breaks_ligatures_in_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "relax_ligature_boundary");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("relax_ligature_boundary.expected.dvi"),
    )
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_display_equation_number_preserves_formula_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "display_eqnos");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("display_eqnos.expected.dvi"),
    )
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_math_group_singleton_ord_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "mathopen_boxed_delimiter");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(
        test_support::corpus_root()
            .join("math")
            .join("mathopen_boxed_delimiter.expected.dvi"),
    )
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

/// Pins the canonical engine's Gentle DVI to the real-pdfTeX oracle. The
/// shared conformance comparator permits only the variable preamble comment;
/// every remaining byte, including list-setting geometry, must match.
#[test]
#[ignore = "explicit canonical Gentle byte-exact conformance gate"]
fn e2e_conformance_gentle_canonical() {
    assets::with_gate("gentle", |gate| {
        run_plain_fixture_case_canonical("gentle.tex", gate);
    });
}

#[allow(clippy::disallowed_methods)] // Host-side fixture staging and artifact comparison.
fn run_two_phase_fixture(source_name: &str, local_name: &str, etex: bool, gate: &GateAssets) {
    let root = &gate.repo_root;
    let fixture_name = gate.name;
    let fixture = &gate.oracle;
    let source = root.join("third_party/trip").join(source_name);

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
    fs::copy(
        root.join("third_party/trip/trip.tfm"),
        temp.path().join(format!("{fixture_name}.tfm")),
    )
    .expect("stage conformance TFM");

    let engine = if etex {
        EngineMode::ETex
    } else {
        EngineMode::Tex82
    };
    let initial = run_file_in_process(&input, None, engine)
        .unwrap_or_else(|error| panic!("{fixture_name} format creation failed: {error}"));
    let format = initial
        .format
        .unwrap_or_else(|| panic!("{fixture_name} did not dump a format"));
    let loaded = run_file_in_process(&input, Some(&format), engine)
        .unwrap_or_else(|error| panic!("{fixture_name} format-loaded run failed: {error}"));
    let dvi = loaded
        .dvi
        .unwrap_or_else(|| panic!("{fixture_name} did not produce DVI"));
    let actual = target_dir(root)
        .join("conformance-artifacts")
        .join(format!("{fixture_name}.dvi"));
    fs::create_dir_all(actual.parent().expect("artifact parent"))
        .expect("create conformance artifact directory");
    fs::write(&actual, dvi).expect("write conformance artifact");
    let expected_dvi = fs::read(fixture).expect("read conformance DVI oracle");
    let actual_dvi = fs::read(&actual).expect("read conformance DVI artifact");
    let expected_normalized =
        normalized_dvi_for_comparison(&expected_dvi).expect("normalize conformance DVI oracle");
    let expected_name = format!("{}/{fixture_name}.expected.dvi", assets::ORACLE_DIR);
    let expected_identity = format!("sha256:{:x}", Sha256::digest(&expected_normalized));
    let actual_identity = format!("sha256:{:x}", Sha256::digest(&format));
    write_trip_triage_artifact(
        &target_dir(root).join("conformance-triage"),
        TripTriageInput {
            label: fixture_name,
            phase: "format-loaded",
            expected_source: TripTriageSource {
                name: &expected_name,
                identity: &expected_identity,
            },
            actual_source: TripTriageSource {
                name: "umber in-process format-loaded run",
                identity: &actual_identity,
            },
            expected: TripTriageChannels {
                command_events: None,
                transcript: b"",
                log: b"",
                dvi: &expected_dvi,
            },
            actual: TripTriageChannels {
                command_events: None,
                transcript: b"",
                log: b"",
                dvi: &actual_dvi,
            },
        },
    )
    .expect("write bounded TRIP triage artifact");
    compare_dvi_files(
        fixture,
        &actual,
        &target_dir(root).join("conformance-triage"),
        fixture_name,
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
fn e2e_conformance_trip() {
    assets::with_gate("trip", |gate| {
        run_two_phase_fixture("trip.tex", "trip.tex", false, gate);
    });
}

#[test]
fn e2e_conformance_etrip() {
    assets::with_gate("etrip", |gate| {
        run_two_phase_fixture("etrip.tex", "etrip-local.tex", true, gate);
    });
}
