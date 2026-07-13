use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use parity_harness::{compare_dvi_files, run_named_fixture_document};

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
    run_named_fixture_document(
        &root,
        Path::new(env!("CARGO_BIN_EXE_umber")),
        document,
        &fixture,
    )
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

#[test]
#[allow(clippy::disallowed_methods)] // Explicit host-side conformance process.
fn e2e_conformance_trip() {
    let root = repo_root();
    let trip_dir = root.join("third_party/trip");
    let fixture = root.join("tests/corpus/e2e/trip.expected.dvi");
    if !trip_dir.join("trip.tex").is_file()
        || !trip_dir.join("trip.tfm").is_file()
        || !fixture.is_file()
    {
        eprintln!(
            "skipping TRIP end-to-end conformance: an external input or locally generated DVI oracle is absent; run scripts/setup-conformance-tests.sh"
        );
        return;
    }
    assert!(
        trip_dir.join("trip.dvi").is_file(),
        "TRIP inputs are present but canonical third_party/trip/trip.dvi is missing; run scripts/trip.sh fetch"
    );
    let target = target_dir(&root);
    let output = Command::new(root.join("scripts/trip.sh"))
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", &target)
        .env("UMBER_BIN", env!("CARGO_BIN_EXE_umber"))
        .arg("umber-artifacts")
        .output()
        .expect("run TRIP artifact producer");
    assert!(
        output.status.success(),
        "TRIP artifact production failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    compare_dvi_files(
        &fixture,
        &target.join("trip/umber/trip.dvi"),
        &target.join("conformance-triage"),
        "trip",
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
#[allow(clippy::disallowed_methods)] // Explicit host-side conformance process.
fn e2e_conformance_etrip() {
    let root = repo_root();
    let trip_dir = root.join("third_party/trip");
    let fixture = root.join("tests/corpus/e2e/etrip.expected.dvi");
    if !trip_dir.join("etrip.tex").is_file()
        || !trip_dir.join("trip.tfm").is_file()
        || !fixture.is_file()
    {
        eprintln!(
            "skipping e-TRIP conformance: an external input or locally generated DVI oracle is absent; run scripts/setup-conformance-tests.sh"
        );
        return;
    }

    let target = target_dir(&root);
    let output = Command::new(root.join("scripts/trip.sh"))
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", &target)
        .env("UMBER_BIN", env!("CARGO_BIN_EXE_umber"))
        .arg("etrip-umber-artifacts")
        .output()
        .expect("run e-TRIP artifact producer");
    assert!(
        output.status.success(),
        "e-TRIP artifact production failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    compare_dvi_files(
        &fixture,
        &target.join("etrip/umber/etrip.dvi"),
        &target.join("conformance-triage"),
        "etrip",
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}
