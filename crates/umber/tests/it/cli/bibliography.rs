//! Native bibliography command staging, argument authority, and output contracts.

use super::*;

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
        let case = FixtureCase::discover(&case_relative, "invocation.case", "bib-invocation-v2")
            .expect("typed closed bibliography case");
        let invocation =
            BibInvocationCase::parse(&case.read_to_string("invocation.case").expect("metadata"))
                .expect("valid invocation metadata");
        let temp = tempfile::tempdir().expect("create isolated bibliography output directory");
        let resolved = invocation
            .resolve(&case, temp.path())
            .expect("validate bibliography invocation roles");
        let mut command = Command::new(env!("CARGO_BIN_EXE_umber"));
        command.arg("bib");
        command.args(&resolved.argv);
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
        if let Some((artifact, expected)) = &resolved.artifact {
            assert_eq!(
                fs::read(artifact).expect("generated bibliography artifact"),
                case.read(expected).expect("expected bibliography artifact"),
                "{name} artifact"
            );
        }
        FixtureCase::discover(&case_relative, "invocation.case", "bib-invocation-v2")
            .expect("invocation must not publish ambient case outputs");
    }
}

#[test]
fn bibliography_typed_arguments_reject_path_authority_and_undeclared_roles() {
    for literal in [
        "ambient.bib",
        "/tmp/ambient.bib",
        "../ambient.bib",
        "./ambient.bib",
        "nested/ambient.bib",
        ".",
    ] {
        let metadata = format!(
            "bib-invocation-v2\narg=literal:{literal}\nstatus=0\nstdout=empty\nstderr=empty\n"
        );
        assert!(
            BibInvocationCase::parse(&metadata).is_err(),
            "path-bearing literal accepted: {literal:?}"
        );
    }

    assert!(
        BibInvocationCase::parse(
            "bib-invocation-v2\n\
         arg=input:ambient\n\
         status=0\n\
         stdout=empty\n\
         stderr=empty\n",
        )
        .is_err()
    );
}

#[test]
fn bibliography_typed_roles_have_one_global_namespace_and_exact_cardinality() {
    for (name, body) in [
        (
            "duplicate input definition",
            "arg=input:source\ninput=source:basic.bcf\ninput=source:basic.bcf\n",
        ),
        (
            "duplicate output definition",
            "arg=output:result\noutput=result:one.bbl:expected.bbl\noutput=result:two.bbl:expected.bbl\n",
        ),
        (
            "cross-kind definition",
            "arg=input:shared\narg=output:shared\ninput=shared:basic.bcf\noutput=shared:result.bbl:expected.bbl\n",
        ),
        (
            "repeated input use",
            "arg=input:source\narg=input:source\ninput=source:basic.bcf\n",
        ),
        (
            "repeated output use",
            "arg=output:result\narg=output:result\noutput=result:result.bbl:expected.bbl\n",
        ),
        (
            "unused input",
            "arg=literal:ordinary\ninput=source:basic.bcf\n",
        ),
        (
            "unused output",
            "arg=literal:ordinary\noutput=result:result.bbl:expected.bbl\n",
        ),
        (
            "output used as input",
            "arg=input:result\noutput=result:result.bbl:expected.bbl\n",
        ),
        (
            "input used as output",
            "arg=output:source\ninput=source:basic.bcf\n",
        ),
        (
            "conflicting output artifact",
            "arg=output:first\narg=output:second\noutput=first:result.bbl:expected.bbl\noutput=second:result.bbl:expected.bbl\n",
        ),
    ] {
        let metadata = format!("bib-invocation-v2\n{body}status=0\nstdout=empty\nstderr=empty\n");
        assert!(
            BibInvocationCase::parse(&metadata).is_err(),
            "{name} accepted"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-only isolated staging assertion.
fn bibliography_typed_arguments_materialize_literals_and_declared_roles() {
    let case = FixtureCase::discover(
        "tests/corpus/bib/invocation/bcf-success",
        "invocation.case",
        "bib-invocation-v2",
    )
    .expect("typed closed bibliography case");
    let invocation = BibInvocationCase::parse(
        "bib-invocation-v2\n\
         arg=literal:ordinary\n\
         arg=input:control\n\
         arg=output:artifact\n\
         status=0\n\
         input=control:basic.bcf\n\
         stdout=empty\n\
         stderr=empty\n\
         output=artifact:result.bbl:expected.bbl\n",
    )
    .expect("typed invocation");
    let workspace = tempfile::tempdir().expect("isolated workspace");
    let resolved = invocation
        .resolve(&case, workspace.path())
        .expect("materialized invocation");
    assert_eq!(resolved.argv[0], "ordinary");
    assert_eq!(
        Path::new(&resolved.argv[1]).parent(),
        Some(
            workspace
                .path()
                .canonicalize()
                .expect("workspace")
                .as_path()
        )
    );
    assert_eq!(
        fs::read(&resolved.argv[1]).expect("staged declared input"),
        case.read("basic.bcf").expect("authority input")
    );
    assert_eq!(
        Path::new(&resolved.argv[2]).parent(),
        Some(
            workspace
                .path()
                .canonicalize()
                .expect("workspace")
                .as_path()
        )
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-only adversarial sentinel construction.
fn failing_path_literal_cannot_touch_an_ambient_sentinel() {
    let ambient = tempfile::tempdir().expect("ambient directory");
    let sentinel = ambient.path().join("ambient.bib");
    fs::write(&sentinel, b"untouched sentinel\n").expect("ambient sentinel");
    let metadata = format!(
        "bib-invocation-v2\narg=literal:{}\nstatus=0\nstdout=empty\nstderr=empty\n",
        sentinel.display()
    );

    assert!(BibInvocationCase::parse(&metadata).is_err());
    assert_eq!(
        fs::read(&sentinel).expect("ambient sentinel"),
        b"untouched sentinel\n"
    );
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
