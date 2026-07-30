use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};
use test_support::{
    CorpusCase, assert_matches_fixture, corpus_cases, dvi, git_fixture::ClosedCase, normalize,
    read_binary_fixture,
};
use tex_lex::{Lexer, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::{Catcode, Token};
use tex_state::{Universe, World};

const PINNED_SOURCE_DATE_EPOCH: &str = "1783604160";

#[test]
fn exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .status()
        .expect("failed to run umber binary");

    assert!(status.success());
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the built Umber binary.
fn format_cache_cli_stores_restores_and_reports_misses() {
    let directory = tempfile::tempdir().expect("create format cache fixture");
    let closure = directory.path().join("closure.index");
    let source_lock = directory.path().join("source.lock");
    let build_configuration = directory.path().join("build.config");
    fs::write(&closure, b"tex:latex.ltx\n").expect("write closure identity");
    fs::write(&source_lock, b"pinned sources\n").expect("write source lock");
    fs::write(&build_configuration, b"profile=release\n").expect("write build config");
    let format_path = directory.path().join("generated.fmt");
    fs::write(
        &format_path,
        Universe::new().dump_format().expect("schema-11 format"),
    )
    .expect("write format image");
    let cache_root = directory.path().join("cache");

    let common = [
        "--engine",
        "latex",
        "--distribution",
        "texlive-test",
        "--closure",
        closure.to_str().expect("closure path"),
        "--source-lock",
        source_lock.to_str().expect("source lock path"),
        "--build-configuration",
        build_configuration.to_str().expect("build config path"),
        "--cache-root",
        cache_root.to_str().expect("cache root path"),
    ];
    let store = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "store"])
        .args(common)
        .args(["--format", format_path.to_str().expect("format path")])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .output()
        .expect("store generated format");
    assert!(store.status.success());
    assert_eq!(store.stdout, b"stored\n");
    assert!(String::from_utf8_lossy(&store.stderr).contains("published generated format"));

    let restored = directory.path().join("restored.fmt");
    let restore = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "restore"])
        .args(common)
        .args(["--format-out", restored.to_str().expect("restore path")])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .output()
        .expect("restore generated format");
    assert!(restore.status.success());
    assert_eq!(restore.stdout, b"hit\n");
    assert_eq!(
        fs::read(restored).expect("read restored format"),
        fs::read(format_path).expect("read source format")
    );

    let miss = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "restore"])
        .args(common)
        .arg("--format-out")
        .arg(directory.path().join("miss.fmt"))
        .env("SOURCE_DATE_EPOCH", "0")
        .output()
        .expect("probe changed-clock format");
    assert!(miss.status.success());
    assert_eq!(miss.stdout, b"miss\n");
    assert!(!directory.path().join("miss.fmt").exists());
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the built Umber binary.
fn bib_command_has_exact_native_invocation_outputs_and_statuses() {
    let repository =
        test_support::repository_root_at(&std::env::current_dir().expect("current directory"))
            .expect("runtime repository");
    let area_relative = PathBuf::from("tests/corpus/bib/invocation");
    let area = repository.join(&area_relative);
    let mut names = fs::read_dir(&area)
        .expect("read bibliography invocation cases")
        .map(|entry| {
            let entry = entry.expect("read bibliography invocation case");
            assert!(
                entry.file_type().expect("case file type").is_dir(),
                "bibliography invocation area contains a non-case entry: {}",
                entry.path().display()
            );
            entry.file_name().into_string().expect("UTF-8 case name")
        })
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["bcf-success", "invalid-output-format", "tool-mode",]
    );

    for name in names {
        let case_relative = area_relative.join(&name);
        let case = ClosedCase::discover(&case_relative).expect("closed bibliography case");
        let invocation =
            BibInvocationCase::parse(&case.read_to_string("invocation.case").expect("metadata"));
        let temp = tempfile::tempdir().expect("create isolated bibliography output directory");
        let resolved = invocation
            .resolve(&case, temp.path())
            .expect("validate bibliography invocation roles");
        let mut command = Command::new(env!("CARGO_BIN_EXE_umber"));
        command.arg("bib");
        for argument in &invocation.argv {
            if argument == "{output}" {
                command.arg(
                    resolved
                        .artifact
                        .as_ref()
                        .expect("output placeholder artifact"),
                );
            } else if let Some(input) = resolved.inputs.get(argument) {
                command.arg(input);
            } else {
                command.arg(argument);
            }
        }
        let output = command.output().expect("run native bibliography case");
        assert_eq!(output.status.code(), Some(invocation.status), "{name}");
        assert_eq!(
            output.stdout,
            invocation.expected_channel(&case, &invocation.stdout),
            "{name} stdout"
        );
        assert_eq!(
            output.stderr,
            invocation.expected_channel(&case, &invocation.stderr),
            "{name} stderr"
        );
        if let Some((_, expected)) = &invocation.artifact {
            assert_eq!(
                fs::read(resolved.artifact.as_ref().expect("artifact path"))
                    .expect("generated bibliography artifact"),
                case.read(expected).expect("expected bibliography artifact"),
                "{name} artifact"
            );
        }
        ClosedCase::discover(&case_relative)
            .expect("invocation must not publish ambient case outputs");
    }
}

struct BibInvocationCase {
    argv: Vec<String>,
    status: i32,
    inputs: BTreeSet<String>,
    stdout: String,
    stderr: String,
    artifact: Option<(String, String)>,
}

struct ResolvedBibInvocation {
    inputs: BTreeMap<String, PathBuf>,
    artifact: Option<PathBuf>,
}

impl BibInvocationCase {
    fn parse(metadata: &str) -> Self {
        let mut lines = metadata.lines();
        assert_eq!(lines.next(), Some("bib-invocation-v1"));
        let mut argv = Vec::new();
        let mut status = None;
        let mut inputs = BTreeSet::new();
        let mut stdout = None;
        let mut stderr = None;
        let mut artifact = None;
        for line in lines {
            let (key, value) = line.split_once('=').expect("keyed invocation metadata");
            match key {
                "argv" => argv.push(value.to_owned()),
                "status" => {
                    assert!(status.is_none(), "duplicate status role");
                    status = Some(value.parse().expect("numeric invocation status"));
                }
                "input" => assert!(inputs.insert(value.to_owned()), "duplicate input role"),
                "stdout" => {
                    assert!(stdout.is_none(), "duplicate stdout role");
                    stdout = Some(value.to_owned());
                }
                "stderr" => {
                    assert!(stderr.is_none(), "duplicate stderr role");
                    stderr = Some(value.to_owned());
                }
                "artifact" if value == "none" => {
                    assert!(artifact.is_none(), "duplicate artifact role");
                    artifact = Some(None);
                }
                "artifact" => {
                    assert!(artifact.is_none(), "duplicate artifact role");
                    let (actual, expected) = value
                        .split_once(':')
                        .expect("artifact actual:expected roles");
                    artifact = Some(Some((actual.to_owned(), expected.to_owned())));
                }
                _ => panic!("unknown invocation metadata field: {line}"),
            }
        }
        Self {
            argv,
            status: status.expect("status role"),
            inputs,
            stdout: stdout.expect("stdout role"),
            stderr: stderr.expect("stderr role"),
            artifact: artifact.expect("artifact role"),
        }
    }

    fn expected_channel(&self, case: &ClosedCase, authority: &str) -> Vec<u8> {
        if authority == "empty" {
            Vec::new()
        } else {
            case.read(authority).expect("declared channel authority")
        }
    }

    fn resolve(
        &self,
        case: &ClosedCase,
        output_root: &Path,
    ) -> Result<ResolvedBibInvocation, String> {
        let inputs = self
            .inputs
            .iter()
            .map(|role| {
                case.payload_path(role)
                    .map(|path| (role.clone(), path))
                    .map_err(|error| format!("invalid input role {role:?}: {error:#}"))
            })
            .collect::<Result<_, _>>()?;
        for (channel, role) in [("stdout", &self.stdout), ("stderr", &self.stderr)] {
            if role != "empty" {
                case.payload_path(role)
                    .map_err(|error| format!("invalid {channel} role {role:?}: {error:#}"))?;
            }
        }
        let output_placeholders = self
            .argv
            .iter()
            .filter(|argument| argument.as_str() == "{output}")
            .count();
        let artifact = match &self.artifact {
            Some((actual, expected)) => {
                if output_placeholders != 1 {
                    return Err(format!(
                        "artifact invocation requires exactly one output placeholder, found {output_placeholders}"
                    ));
                }
                case.payload_path(expected).map_err(|error| {
                    format!("invalid expected artifact role {expected:?}: {error:#}")
                })?;
                Some(safe_artifact_path(output_root, actual)?)
            }
            None => {
                if output_placeholders != 0 {
                    return Err("artifact-free invocation contains an output placeholder".into());
                }
                None
            }
        };
        Ok(ResolvedBibInvocation { inputs, artifact })
    }
}

fn safe_artifact_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let relative = Path::new(name);
    let mut components = relative.components();
    let Some(Component::Normal(file_name)) = components.next() else {
        return Err(format!(
            "artifact name must be a normalized relative filename: {name:?}"
        ));
    };
    if components.next().is_some()
        || matches!(name, "case.inventory" | "invocation.case" | "{output}")
    {
        return Err(format!(
            "artifact name must be a non-reserved relative filename: {name:?}"
        ));
    }
    let metadata =
        fs::symlink_metadata(root).map_err(|error| format!("inspect output root: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("artifact output root is not a non-symlink directory".into());
    }
    let root = root
        .canonicalize()
        .map_err(|error| format!("canonicalize output root: {error}"))?;
    let output = root.join(file_name);
    if output.parent() != Some(root.as_path()) {
        return Err(format!("artifact output escapes isolated root: {name:?}"));
    }
    if fs::symlink_metadata(&output).is_ok() {
        return Err(format!(
            "artifact output collides with an existing path: {}",
            output.display()
        ));
    }
    Ok(output)
}

#[test]
#[allow(clippy::disallowed_methods)] // Hermetic adversarial output-root construction.
fn bibliography_artifact_names_reject_authority_escapes_and_collisions() {
    let temp = tempfile::tempdir().expect("artifact output root");
    for name in [
        "/tmp/ambient.bbl",
        "../ambient.bbl",
        ".",
        "./result.bbl",
        "nested/result.bbl",
        "case.inventory",
        "invocation.case",
        "{output}",
    ] {
        assert!(
            safe_artifact_path(temp.path(), name).is_err(),
            "unsafe artifact name accepted: {name}"
        );
    }
    fs::write(temp.path().join("result.bbl"), "occupied").expect("collision");
    assert!(safe_artifact_path(temp.path(), "result.bbl").is_err());
}

#[cfg(unix)]
#[test]
fn bibliography_artifact_names_reject_symlink_outputs_and_roots() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("artifact output root");
    symlink(
        "/tmp/ambient-bibliography-output",
        temp.path().join("result.bbl"),
    )
    .expect("artifact symlink");
    assert!(safe_artifact_path(temp.path(), "result.bbl").is_err());

    let parent = tempfile::tempdir().expect("symlink root parent");
    symlink(temp.path(), parent.path().join("output")).expect("output root symlink");
    assert!(safe_artifact_path(&parent.path().join("output"), "result.bbl").is_err());
}

#[test]
fn bibliography_artifact_name_accepts_a_fresh_safe_filename() {
    let temp = tempfile::tempdir().expect("artifact output root");
    assert_eq!(
        safe_artifact_path(temp.path(), "result.bbl").expect("safe artifact"),
        temp.path()
            .canonicalize()
            .expect("canonical output root")
            .join("result.bbl")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Regression exercises the native command with pinned files.
fn bibtex_command_runs_the_pinned_classic_smoke_case_in_process() {
    let fixture = test_support::repository_root()
        .join("crates/umber")
        .join("../../tests/corpus/bibtex/cases/smoke");
    let temp_dir = tempfile::tempdir().expect("create classic output directory");
    for extension in ["aux", "bib", "bst"] {
        fs::copy(
            fixture.join(format!("smoke.{extension}")),
            temp_dir.path().join(format!("smoke.{extension}")),
        )
        .expect("stage classic fixture");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("bibtex")
        .arg(temp_dir.path().join("smoke"))
        .output()
        .expect("run native classic BibTeX command");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(temp_dir.path().join("smoke.bbl")).expect("generated BBL"),
        fs::read(fixture.join("smoke.bbl")).expect("pinned BBL")
    );
    assert_eq!(
        output.stdout,
        fs::read(fixture.join("smoke.terminal")).expect("pinned terminal output")
    );
    assert_eq!(
        fs::read(temp_dir.path().join("smoke.blg")).expect("generated BLG"),
        fs::read(fixture.join("smoke.blg")).expect("pinned BLG")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Regression exercises the native command with pinned files.
fn bib_command_processes_pinned_full_bibtex_unicode_names() {
    let fixture = test_support::repository_root()
        .join("crates/umber")
        .join("../../tests/corpus/bib/upstream-2.22/tdata");
    let temp_dir = tempfile::tempdir().expect("create full BibTeX output directory");
    let output_path = temp_dir.path().join("full.bib");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("bib")
        .arg("--noconf")
        .arg("--nolog")
        .arg("--output-format=bibtex")
        .arg("--output-align")
        .arg("--output-file")
        .arg(&output_path)
        .arg(fixture.join("full-bibtex.bcf"))
        .output()
        .expect("run pinned full BibTeX command");

    assert_eq!(
        output.status.code(),
        Some(0),
        "native bib command failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let generated = fs::read_to_string(output_path).expect("generated full BibTeX output");
    assert!(generated.contains("H{ü}nenberger, Philippe H."));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_publishes_a_dumped_format_from_the_resource_session() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("format.tex");
    let format = temp_dir.path().join("format.fmt");
    fs::write(&source, "\\catcode`@=11 \\dump\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out"])
        .arg(&format)
        .arg(&source)
        .output()
        .expect("run format dump");

    assert!(
        output.status.success(),
        "format dump failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fs::read(format).expect("read dumped format").is_empty());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn format_output_rejects_a_successful_run_that_did_not_dump() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("format.tex");
    let format = temp_dir.path().join("format.fmt");
    fs::write(&source, "\\message{NO-DUMP}\\endinput\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out"])
        .arg(&format)
        .arg(&source)
        .output()
        .expect("run missing format dump");

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "umber: --format-out requires the input to execute \\dump\n"
    );
    assert!(!format.exists());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_rule_page_is_published_only_to_an_explicit_distinct_pdf_path() {
    let temp_dir = tempfile::tempdir().expect("create PDF output temp dir");
    let source = temp_dir.path().join("rule.tex");
    let pdf = temp_dir.path().join("rule.pdf");
    let dvi = temp_dir.path().join("rule.dvi");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfcompresslevel=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end\n",
    )
    .expect("write PDF rule fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("UMBER_RESOURCE_TELEMETRY", "1")
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg("--dvi")
        .arg(&dvi)
        .arg(&source)
        .output()
        .expect("run pdfTeX PDF fixture");

    assert!(
        output.status.success(),
        "pdfTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pdf_bytes = fs::read(&pdf).expect("read published PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF"));
    assert!(fs::metadata(&dvi).expect("published DVI").len() > 0);
    let telemetry = String::from_utf8_lossy(&output.stderr);
    for marker in [
        "RESOURCE_STARTUP_TELEMETRY",
        "RESOURCE_ENGINE_ACCEPTED",
        "RESOURCE_HOST_TELEMETRY",
        "PDF_TELEMETRY",
        "PDF_DRIVER_BUILD",
        "PDF_DRIVER_TELEMETRY",
    ] {
        assert!(telemetry.contains(marker), "missing {marker}:\n{telemetry}");
    }

    let rejected = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdf")
        .arg(temp_dir.path().join("wrong-mode.pdf"))
        .arg(&source)
        .output()
        .expect("reject PDF without pdfTeX mode");
    assert!(!rejected.status.success());
    assert_eq!(
        String::from_utf8(rejected.stderr).expect("stderr is utf-8"),
        "umber: --pdf requires --pdftex or --pdflatex\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdflatex_mode_composes_latex_compatibility_with_pdf_output() {
    let temp_dir = tempfile::tempdir().expect("create pdfLaTeX output temp dir");
    let source = temp_dir.path().join("composed.tex");
    let pdf = temp_dir.path().join("composed.pdf");
    fs::write(
        &source,
        "\\catcode123=1\\catcode125=2\\pdfoutput=1\\ifnum\\strcmp{same}{same}=0\\shipout\\vbox{\\hrule width10pt height5pt}\\fi\\end\n",
    )
    .expect("write composed pdfLaTeX fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdflatex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run composed pdfLaTeX fixture");

    assert!(
        output.status.success(),
        "pdfLaTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pdf_bytes = fs::read(&pdf).expect("read composed pdfLaTeX PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdfdraftmode_does_not_replace_the_requested_pdf_output() {
    let temp_dir = tempfile::tempdir().expect("create draft-mode output temp dir");
    let source = temp_dir.path().join("draft.tex");
    let pdf = temp_dir.path().join("draft.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfdraftmode=1\\shipout\\vbox{\\hrule width10pt height5pt}\\end\n",
    )
    .expect("write draft-mode fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run draft-mode fixture");

    assert!(
        output.status.success(),
        "draft-mode run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is utf-8"),
        "pdfTeX warning: \\pdfdraftmode enabled, not changing output pdf\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read unchanged output"),
        b"existing output\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdf_lowering_omits_dvi_special_and_publishes_all_driver_output() {
    let temp_dir = tempfile::tempdir().expect("create DVI-special temp dir");
    let source = temp_dir.path().join("text.tex");
    let pdf = temp_dir.path().join("text.pdf");
    let dvi = temp_dir.path().join("text.dvi");
    fs::write(
        &source,
        "\\pdfoutput=1\\shipout\\vbox{\\special{dvi-only-payload}}\\end\n",
    )
    .expect("write DVI-special fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("UMBER_RESOURCE_TELEMETRY", "1")
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg("--dvi")
        .arg(&dvi)
        .arg(&source)
        .output()
        .expect("run DVI-special PDF fixture");

    assert!(
        output.status.success(),
        "DVI-special PDF run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("RESOURCE_ENGINE_ACCEPTED"),
        "accepted-engine telemetry must precede detached finalization"
    );
    let pdf_bytes = fs::read(&pdf).expect("PDF output was published");
    assert!(
        !pdf_bytes
            .windows(b"dvi-only-payload".len())
            .any(|window| window == b"dvi-only-payload"),
        "DVI-only special leaked into PDF output"
    );
    let dvi_bytes = fs::read(&dvi).expect("DVI peer output was published");
    assert!(
        dvi_bytes
            .windows(b"dvi-only-payload".len())
            .any(|window| window == b"dvi-only-payload"),
        "DVI peer output lost its special payload"
    );
}

#[cfg(feature = "profiling")]
#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary file and command execution.
fn profiling_stats_are_reported_only_when_requested() {
    let temp_dir = tempfile::tempdir().expect("create profiling stats temp dir");
    let source = temp_dir.path().join("stats.tex");
    fs::write(&source, "\\end\n").expect("write profiling stats fixture");

    let quiet = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run instrumented umber without reporting");
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty());

    let reported = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--profiling-stats")
        .arg(&source)
        .output()
        .expect("run instrumented umber with reporting");
    assert!(reported.status.success());
    let stderr = String::from_utf8(reported.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("EXPANSION_STATS "));
    assert!(stderr.contains("NODE_MEMORY_TOTAL "));
    assert!(stderr.contains("ALLOC_NODE_APPEND "));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn lex_dump_prints_stable_token_format_for_corpus() {
    for case in corpus_cases("lexer") {
        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("lex-dump")
            .arg(case.source_path())
            .output()
            .expect("run umber lex-dump");

        assert!(
            output.status.success(),
            "lex-dump failed for {}:\n{}",
            case.source_path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("lex-dump output is utf-8");
        assert_matches_fixture("lexer", case.name(), "tokens", &actual);
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn expand_dump_prints_stable_token_format_for_corpus() {
    for case in corpus_cases("expand") {
        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("expand-dump")
            .arg(case.source_path())
            .output()
            .expect("run umber expand-dump");

        assert!(
            output.status.success(),
            "expand-dump failed for {}:\n{}",
            case.source_path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("expand-dump output is utf-8");
        assert_matches_fixture("expand", case.name(), "tokens", &actual);
    }
}

#[test]
fn expand_dump_usage_errors_follow_lex_dump_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .output()
        .expect("run umber expand-dump without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for expand-dump\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber expand-dump with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: expand-dump accepts exactly one input path\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_expansion_error_renders_primary_source_context() {
    let temp_dir = tempfile::tempdir().expect("create diagnostic temp dir");
    let source = temp_dir.path().join("undefined.tex");
    fs::write(&source, "\\undefined\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump diagnostic fixture");

    assert!(
        !output.status.success(),
        "undefined expand-dump should fail"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("undefined.tex:1:1"));
    assert!(stderr.contains("  1 | \\undefined"));
    assert!(stderr.contains("    | ^"));
    assert!(!stderr.contains("unknown origin"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_macro_error_renders_bounded_expansion_trace() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\undefined X}\\a\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump macro diagnostic fixture");

    assert!(!output.status.success(), "macro expand-dump should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("macro.tex:1:8"));
    assert!(stderr.contains("expansion trace:"));
    assert!(stderr.contains("invoked at"));
    assert!(stderr.contains("defined at"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_recovered_execution_error_exits_successfully() {
    let temp_dir = tempfile::tempdir().expect("create execution diagnostic temp dir");
    let source = temp_dir.path().join("prefix.tex");
    fs::write(&source, "\\global X\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump execution diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered prefix error should succeed"
    );
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary fixture setup and command execution.
fn run_recovered_diagnostic_after_tfm_load_exits_successfully() {
    let temp_dir = tempfile::tempdir().expect("create font provenance temp dir");
    let source = temp_dir.path().join("after-font.tex");
    let child = temp_dir.path().join("child.tex");
    let tfm = temp_dir.path().join("cmr10.tfm");
    fs::write(&source, "\\font\\f=cmr10 \\relax\n\\input child\n").expect("write main fixture");
    fs::write(&child, "\\global X\n").expect("write diagnostic fixture");
    fs::copy(
        test_support::repository_root().join("crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"),
        &tfm,
    )
    .expect("copy TFM fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run font provenance fixture");

    assert!(
        output.status.success(),
        "recovered prefix error should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("You can't use a prefix"), "{stdout}");
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_exec_corpus_matches_committed_diagnostics() {
    // The excluded cases are excluded for a harness reason, not an engine
    // one: this test compares `umber run`'s *terminal* (stdout) capture
    // against each `tests/corpus/exec/<case>/expected.log`, but that fixture is
    // generated from the reference engine's *log* (transcript) file
    // (`fixturegen` reruns pdftex and captures its `.log`, independent of
    // `\tracingonline`). The two channels agree byte-for-byte for most
    // fixtures here, but tex.web §90's `error`/`help_on_transcript` tail
    // (crates/tex-state/src/print.rs) deliberately writes one more
    // `print_ln` to the log than to the terminal around a `\show` family
    // completion (`decr(selector); ...; incr(selector); print_ln`), so two
    // back-to-back `\show`-family completions -- each excluded fixture's
    // shape -- leave a blank line between them on the log that the terminal
    // never gets. `\tracingonline=1` (needed so `\showbox`/`\showlists`
    // reach the terminal at all post-umber2-alfh.9) does not change that
    // tail, so these fixtures cannot match on both channels at once through
    // this harness. See umber2-gn1p (the channel-comparison gap itself) and
    // umber2-sob4 (found alongside it: `umber run` never shows §310
    // context at all, independent of channel).
    run_corpus_matches_committed_log_fixtures(
        "exec",
        false,
        &[
            "hmode_ligkern",
            "hmode_material_primitives",
            "hmode_space_factor",
            "math_component_recovery",
            "paragraph_line_shape",
            "showbox_simple",
            "vbox_baseline_spacing",
        ],
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_etex_exec_corpus_matches_committed_diagnostics() {
    for case in corpus_cases("etex_exec") {
        assert_log_case_matches_committed_fixture("etex_exec", &case, false, true);
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_typeset_corpus_matches_committed_box_dumps() {
    // See `run_exec_corpus_matches_committed_diagnostics`'s comment: same
    // terminal-vs-log channel mismatch (umber2-gn1p), this area's own set of
    // fixtures with two or more back-to-back `\show`-family completions.
    run_corpus_matches_committed_log_fixtures(
        "typeset",
        true,
        &[
            "alignment_math_group_balance",
            "alignment_widths_spans",
            "display_math_machinery",
            "ligkern_words",
            "material_primitives",
            "paragraph_line_shape",
            "space_factor",
            "vbox_baseline_spacing",
            "vsplit_split_marks",
        ],
    );
}

#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_corpus_matches_committed_log_fixtures(
    area: &str,
    show_fixtures: bool,
    ignored_cases: &[&str],
) {
    for case in corpus_cases(area) {
        if !ignored_cases.contains(&case.name()) {
            assert_log_case_matches_committed_fixture(area, &case, show_fixtures, false);
        }
    }
}

#[allow(clippy::disallowed_methods)] // host-side command execution and expected-output reads.
fn assert_log_case_matches_committed_fixture(
    area: &str,
    case: &CorpusCase,
    show_fixtures: bool,
    etex: bool,
) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_umber"));
    command.env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH);
    if etex {
        command.current_dir(
            case.source_path()
                .parent()
                .expect("corpus source has a parent directory"),
        );
    }
    command.arg("run");
    if etex {
        command.arg("--etex");
    }
    if show_fixtures {
        command.arg("--show-fixtures");
    }
    let output = command
        .arg(case.source_path())
        .output()
        .expect("run umber run");
    assert!(
        output.status.success(),
        "umber run failed for {}:\n{}",
        case.source_path().display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let actual_stdout = String::from_utf8(output.stdout).expect("umber run output is utf-8");
    let actual = if show_fixtures {
        normalize::box_dump(&actual_stdout)
    } else {
        normalize::exec_log(&actual_stdout)
    };
    assert_matches_fixture(area, case.name(), "log", &actual);
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_math_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("math");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_align_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("align");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side command execution.
fn removed_html_font_directory_names_the_typed_replacement() {
    // Any small committed source will do: this asserts an argument-parsing
    // rejection that never reaches the engine. `dvi`/`page` were retired
    // into the minifixture corpus, so it names a surviving area.
    let setup = dvi::DviCaseSetup::new("math", "accents");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .current_dir(setup.run_dir())
        .args([
            "run",
            setup.source_file_name(),
            "--html",
            "actual.html",
            "--html-font-dir",
            "web-fonts",
        ])
        .output()
        .expect("run removed HTML font option");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("--html-font-dir was removed"));
    assert!(error.contains("typed resource resolver API"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_initializes_clock_parameters_from_source_date_epoch() {
    let temp_dir = tempfile::tempdir().expect("create clock temp dir");
    let source = temp_dir.path().join("clock.tex");
    fs::write(
        &source,
        "\\message{clock=\\the\\time/\\the\\day/\\the\\month/\\the\\year}\\end\n",
    )
    .expect("write clock fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run umber clock fixture");

    assert!(
        output.status.success(),
        "clock run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("clock=816/9/7/2026"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn latex_creationdate_uses_the_source_date_epoch_job_clock() {
    let temp_dir = tempfile::tempdir().expect("create creation-date temp dir");
    let source = temp_dir.path().join("creationdate.tex");
    fs::write(
        &source,
        "\\catcode123=1 \\catcode125=2 \\message{created=\\creationdate}\\end\n",
    )
    .expect("write creation-date fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--latex")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run Umber LaTeX creation-date fixture");

    assert!(
        output.status.success(),
        "creation-date run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("created=D:20260709133600Z"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_mode_reports_the_pinned_engine_identity() {
    let temp_dir = tempfile::tempdir().expect("create pdfTeX identity temp dir");
    let source = temp_dir.path().join("identity.tex");
    fs::write(
        &source,
        "\\message{engine=\\the\\pdftexversion.\\pdftexrevision}\\end\n",
    )
    .expect("write pdfTeX identity fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg("--pdftex")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run Umber pdfTeX identity fixture");

    assert!(
        output.status.success(),
        "pdfTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .expect("stdout is utf-8")
            .contains("engine=140.27")
    );
}

#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn assert_dvi_area_matches_committed_fixture(area: &str) {
    for case in corpus_cases(area) {
        assert_dvi_case_matches_committed_fixture(area, case.name());
    }
}

#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn assert_dvi_case_matches_committed_fixture(area: &str, case: &str) {
    let setup = dvi::DviCaseSetup::new(area, case);

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(setup.run_dir())
        .arg("run")
        .arg(setup.source_file_name())
        .arg("--dvi")
        .arg(setup.actual_dvi_file_name())
        .output()
        .expect("run umber DVI smoke");
    assert!(
        output.status.success(),
        "umber DVI smoke failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = fs::read(setup.actual_dvi_path()).expect("read umber DVI");
    let expected = read_binary_fixture(area, case, "dvi");
    dvi::assert_dvi_matches(&expected, &actual, &format!("{area}/{case}"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_deadcycles_overflow() {
    let temp_dir = tempfile::tempdir().expect("create deadcycles temp dir");
    let source = temp_dir.path().join("deadcycles.tex");
    fs::write(
        &source,
        "\\maxdeadcycles=1 \\output={\\setbox1=\\box255}\n\
         \\topskip=0pt \\setbox0=\\hbox{}\n\
         \\copy0 \\penalty-10000\n\
         \\copy0 \\penalty-10000\n",
    )
    .expect("write deadcycles fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber deadcycles fixture");

    assert!(
        output.status.success(),
        "recovered deadcycles overflow should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Output loop---1 consecutive dead cycles"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_extra_right_brace() {
    let temp_dir = tempfile::tempdir().expect("create diagnostic temp dir");
    let source = temp_dir.path().join("brace.tex");
    fs::write(&source, "}\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered extra brace should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Too many }'s."));
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_undefined_control_sequence() {
    let temp_dir = tempfile::tempdir().expect("create expansion diagnostic temp dir");
    let source = temp_dir.path().join("undefined.tex");
    fs::write(&source, "\\undefined\n").expect("write expansion diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber expansion diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered undefined control sequence should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Undefined control sequence \\undefined"));
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_extra_endgroup_in_macro() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\endgroup}\\a\n").expect("write macro diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber macro diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered extra endgroup should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Extra \\endgroup."));
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
fn run_usage_errors_follow_existing_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .output()
        .expect("run umber run without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for run\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber run with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: run accepts one input path with optional --show-fixtures and --dvi <path>\n"
    );

    let removed_plain_format = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("--plain-format")
        .output()
        .expect("run umber run with removed --plain-format flag");
    assert!(!removed_plain_format.status.success());
    assert_eq!(
        String::from_utf8(removed_plain_format.stderr).expect("stderr is utf-8"),
        "umber: run accepts one input path with optional --show-fixtures and --dvi <path>\n"
    );

    let missing_show_fixtures = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--show-fixtures")
        .output()
        .expect("run umber run with show-fixtures but without path");
    assert!(!missing_show_fixtures.status.success());
    assert_eq!(
        String::from_utf8(missing_show_fixtures.stderr).expect("stderr is utf-8"),
        "umber: missing input path for run\n"
    );

    let missing_dvi_path = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("--dvi")
        .output()
        .expect("run umber run with --dvi but without output path");
    assert!(!missing_dvi_path.status.success());
    assert_eq!(
        String::from_utf8(missing_dvi_path.stderr).expect("stderr is utf-8"),
        "umber: missing output path for --dvi\n"
    );

    let conflicting_outputs = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args([
            "run",
            "one.tex",
            "--dvi",
            "same.out",
            "--format-out",
            "same.out",
        ])
        .output()
        .expect("run umber with conflicting output paths");
    assert!(!conflicting_outputs.status.success());
    assert_eq!(
        String::from_utf8(conflicting_outputs.stderr).expect("stderr is utf-8"),
        "umber: --dvi and --format-out must use different output paths\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_area_less_input_through_texinputs_and_advances() {
    let temp_dir = tempfile::tempdir().expect("create TeX input search temp dir");
    let job_dir = temp_dir.path().join("plain/base");
    let search_dir = temp_dir.path().join("generic/hyphen");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&search_dir).expect("create TeX input search directory");
    let source = job_dir.join("plain.tex");
    fs::write(&source, "\\input hyphen \\message{after-hyphen}\\end\n")
        .expect("write principal input");
    fs::write(search_dir.join("hyphen.tex"), "\\message{loaded-hyphen}\n")
        .expect("write searched input");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXINPUTS", &search_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run input search smoke");

    assert!(
        output.status.success(),
        "input search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("loaded-hyphen"));
    assert!(stdout.contains("after-hyphen"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_acquires_from_a_local_distribution_then_reuses_cache_offline() {
    let temp_dir = tempfile::tempdir().expect("create distribution temp dir");
    let source = temp_dir.path().join("main.tex");
    let distribution = temp_dir.path().join("distribution");
    let cache = temp_dir.path().join("cache");
    let objects = distribution.join("objects");
    fs::create_dir_all(&objects).expect("create distribution");
    fs::write(&source, "\\input remote \\message{after-remote}\\end\n").expect("write source");
    let remote = b"\\message{from-distribution}\n";
    let object_digest = hex_sha256(remote);
    let object = format!("sha256-{object_digest}");
    fs::write(objects.join(&object), remote).expect("write object");
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"test-snapshot\",\"index\":0,\"files\":{{\"tex:remote.tex\":{{\"virtualPath\":\"/texlive/tex/remote.tex\",\"object\":\"{object}\",\"sha256\":\"{object_digest}\",\"bytes\":{}}}}}}}\n",
        remote.len()
    );
    let shard_digest = hex_sha256(shard.as_bytes());
    fs::write(objects.join(format!("sha256-{shard_digest}")), shard).expect("write shard");
    let manifest = format!(
        "{{\"schema\":2,\"distribution\":\"test-snapshot\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
    );
    fs::write(distribution.join("manifest-v2.json"), &manifest).expect("write manifest");

    let first = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("XDG_CACHE_HOME", &cache)
        .args(["run", "--show-fixtures", "--distribution"])
        .arg(&distribution)
        .arg(&source)
        .output()
        .expect("run cold local distribution");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("from-distribution"));
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr UTF-8"),
        "umber: acquired 1 distribution resource(s)\n"
    );

    fs::remove_file(objects.join(object)).expect("remove source object after warming");
    let second = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("XDG_CACHE_HOME", &cache)
        .env("UMBER_OFFLINE", "1")
        .args(["run", "--show-fixtures", "--distribution"])
        .arg(&distribution)
        .arg(&source)
        .output()
        .expect("run warm offline distribution");
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(second.stderr.is_empty());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_rejects_a_manifest_that_mismatches_its_pin() {
    let temp_dir = tempfile::tempdir().expect("create manifest mismatch temp dir");
    let source = temp_dir.path().join("main.tex");
    let manifest = temp_dir.path().join("manifest.json");
    fs::write(&source, "\\input absent \\end\n").expect("write source");
    fs::write(
        &manifest,
        "{\"schema\":1,\"distribution\":\"test\",\"objectsBaseUrl\":\"https://example.invalid/\",\"files\":{}}",
    )
    .expect("write manifest");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("XDG_CACHE_HOME", temp_dir.path().join("cache"))
        .args([
            "run",
            "--distribution",
            manifest.to_str().expect("UTF-8 path"),
            "--distribution-sha256",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ])
        .arg(&source)
        .output()
        .expect("run mismatched manifest");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("distribution manifest digest mismatch")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_offline_local_mirror_miss_names_the_exact_object_digest() {
    let temp_dir = tempfile::tempdir().expect("create offline miss temp dir");
    let source = temp_dir.path().join("main.tex");
    let distribution = temp_dir.path().join("distribution");
    let objects = distribution.join("objects");
    fs::create_dir_all(&objects).expect("create distribution");
    fs::write(&source, "\\input remote \\end\n").expect("write source");
    let bytes = b"\\relax\n";
    let digest = hex_sha256(bytes);
    let entry = format!(
        "\"tex:remote.tex\":{{\"virtualPath\":\"/texlive/remote.tex\",\"object\":\"sha256-{digest}\",\"sha256\":\"{digest}\",\"bytes\":{}}}",
        bytes.len()
    );
    let shard =
        format!("{{\"schema\":1,\"distribution\":\"test\",\"index\":0,\"files\":{{{entry}}}}}\n");
    let shard_digest = hex_sha256(shard.as_bytes());
    fs::write(objects.join(format!("sha256-{shard_digest}")), shard).expect("write shard");
    fs::write(
        distribution.join("manifest-v2.json"),
        format!(
            "{{\"schema\":2,\"distribution\":\"test\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
        ),
    )
    .expect("write manifest");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("XDG_CACHE_HOME", temp_dir.path().join("empty-cache"))
        .args(["run", "--offline", "--distribution"])
        .arg(&distribution)
        .arg(&source)
        .output()
        .expect("run offline miss");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to read ")
            && stderr.contains(&format!("sha256-{digest}"))
            && !stderr.contains("tex:remote.tex"),
        "{stderr}"
    );
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_writes_a_sorted_deduplicated_input_record_receipt() {
    let temp_dir = tempfile::tempdir().expect("create input receipt temp dir");
    let source = temp_dir.path().join("main.tex");
    let helper = temp_dir.path().join("helper.tex");
    let nested = temp_dir.path().join("nested.tex");
    let receipt = temp_dir.path().join("inputs.tsv");
    let source_bytes = b"\\input helper \\input helper \\end\n";
    let helper_bytes = b"\\input nested \\relax\n";
    let nested_bytes = b"\\relax\n";
    fs::write(&source, source_bytes).expect("write principal input");
    fs::write(&helper, helper_bytes).expect("write included input");
    fs::write(&nested, nested_bytes).expect("write nested input");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .arg("--input-records-out")
        .arg(&receipt)
        .output()
        .expect("run input receipt smoke");

    assert!(
        output.status.success(),
        "input receipt run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = format!(
        "{}\t{}\n{}\t{}\n{}\t{}\n",
        helper_bytes.len(),
        helper.display(),
        source_bytes.len(),
        source.display(),
        nested_bytes.len(),
        nested.display()
    );
    assert_eq!(
        fs::read_to_string(receipt).expect("read input receipt"),
        expected
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_quoted_openin_through_texinputs() {
    let temp_dir = tempfile::tempdir().expect("create TeX stream search temp dir");
    let job_dir = temp_dir.path().join("latex/base");
    let search_dir = temp_dir.path().join("latex/l3kernel");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&search_dir).expect("create TeX stream search directory");
    let source = job_dir.join("stream-search.tex");
    fs::write(
        &source,
        "\\openin1=\"probe.tex\" \\ifeof1 \\errmessage{missing-probe}\\else \\message{found-probe}\\fi \\end\n",
    )
    .expect("write stream search input");
    fs::write(search_dir.join("probe.tex"), "found\n").expect("write searched stream");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXINPUTS", &search_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run stream search smoke");

    assert!(
        output.status.success(),
        "stream search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("found-probe"));
    assert!(!stdout.contains("missing-probe"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_area_less_tfm_through_texfonts_and_advances() {
    let temp_dir = tempfile::tempdir().expect("create TeX font search temp dir");
    let job_dir = temp_dir.path().join("plain/base");
    let font_dir = temp_dir.path().join("fonts/tfm/public/cm");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&font_dir).expect("create TeX font search directory");
    let source = job_dir.join("font-search.tex");
    fs::write(
        &source,
        "\\font\\tenrm=cmr10 \\relax \\message{after-font}\\end\n",
    )
    .expect("write font search input");
    let cmr10 = test_support::repository_root()
        .join("crates/umber")
        .join("../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    fs::copy(cmr10, font_dir.join("cmr10.tfm")).expect("copy searched TFM");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXFONTS", &font_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run font search smoke");

    assert!(
        output.status.success(),
        "font search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("after-font"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture command execution and file checks.
fn run_show_fixtures_harvests_without_committing_immediate_stream_effects() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let normal_dir = temp_dir.path().join("normal");
    let fixture_dir = temp_dir.path().join("fixture");
    fs::create_dir_all(&normal_dir).expect("create normal dir");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let input = temp_dir.path().join("stream_effect.tex");
    fs::write(
        &input,
        "\\immediate\\openout0=side-effect.txt\n\
         \\immediate\\write0{immediate-effect}\n\
         \\immediate\\closeout0\n\\end\n",
    )
    .expect("write input");

    let normal = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(&normal_dir)
        .arg("run")
        .arg(&input)
        .output()
        .expect("run ordinary umber run");
    assert!(
        normal.status.success(),
        "ordinary run failed:\n{}",
        String::from_utf8_lossy(&normal.stderr)
    );
    assert!(
        normal_dir.join("side-effect.txt").exists(),
        "ordinary run should commit immediate stream effects at final commit"
    );

    let fixture = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(&fixture_dir)
        .arg("run")
        .arg("--show-fixtures")
        .arg(&input)
        .output()
        .expect("run umber fixture harvest");
    assert!(
        fixture.status.success(),
        "fixture run failed:\n{}",
        String::from_utf8_lossy(&fixture.stderr)
    );
    assert!(
        !fixture_dir.join("side-effect.txt").exists(),
        "--show-fixtures must not run the final commit for pending immediate effects"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus files, not engine I/O.
fn lexer_dynamic_corpus_covers_mutable_input_state() {
    assert_matches_fixture(
        "lexer_dynamic",
        "catcode_mutation",
        "tokens",
        &lex_catcode_mutation_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "endlinechar_mutation",
        "tokens",
        &lex_endlinechar_mutation_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "ignored_character",
        "tokens",
        &lex_ignored_character_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "invalid_character",
        "tokens",
        &lex_invalid_character_fixture(),
    );
}

fn lex_catcode_mutation_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("catcode_mutation");
    let mut actual = String::new();

    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_catcode('@', Catcode::Letter);
    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_endlinechar_mutation_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("endlinechar_mutation");
    stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);
    let mut actual = String::new();

    push_next_token(&mut actual, &mut lexer, &mut stores);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, b'?' as i32);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_ignored_character_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("ignored_character");
    stores.set_catcode('!', Catcode::Ignored);
    let mut actual = String::new();

    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_invalid_character_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("invalid_character");
    stores.set_catcode('?', Catcode::Invalid);
    let mut actual = String::new();

    loop {
        match lexer.next_token(&mut stores) {
            Ok(Some(token)) => push_token(&mut actual, token, &stores),
            Ok(None) => break,
            Err(err) => {
                actual.push_str(&format!("error:{err}\n"));
                break;
            }
        }
    }

    actual
}

fn lexer_fixture(case: &str) -> (Lexer, Universe) {
    let path = test_support::repository_root()
        .join("tests/corpus/lexer_dynamic")
        .join(case)
        .join("source.tex");
    let mut stores = Universe::with_world(World::real());
    let content = stores
        .world_mut()
        .read_file(&path)
        .expect("open dynamic lexer fixture");
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    (Lexer::new(WorldInput::from_content(content)), stores)
}

fn push_remaining_tokens(actual: &mut String, lexer: &mut Lexer, stores: &mut Universe) {
    while let Some(token) = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
    {
        push_token(actual, token, stores);
    }
}

fn push_next_token(actual: &mut String, lexer: &mut Lexer, stores: &mut Universe) {
    let token = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
        .expect("dynamic lexer fixture ended early");
    push_token(actual, token, stores);
}

fn push_token(actual: &mut String, token: Token, stores: &Universe) {
    let line = match token {
        Token::Char { ch, cat } => format!("char:{}:{}", ch as u32, cat as u8),
        Token::Cs(symbol) => format!("cs:{}", stores.resolve(symbol)),
        Token::Param(slot) => format!("param:{slot}"),
        token if token.is_frozen_end_template() => "frozen:endtemplate".to_owned(),
        token if token.is_frozen_endv() => "frozen:endv".to_owned(),
        Token::Frozen(_) => unreachable!("invalid frozen token payload"),
    };
    actual.push_str(&line);
    actual.push('\n');
}
