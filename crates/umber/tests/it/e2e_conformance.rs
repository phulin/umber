use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use parity_harness::{compare_dvi_files, run_named_external_document};

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

#[test]
#[ignore = "requires the fetched external corpus and a live reference TeX"]
fn e2e_conformance_story() {
    run_named_external_document(
        &repo_root(),
        Path::new(env!("CARGO_BIN_EXE_umber")),
        "story.tex",
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
#[ignore = "requires the fetched external corpus and a live reference TeX"]
fn e2e_conformance_gentle() {
    run_named_external_document(
        &repo_root(),
        Path::new(env!("CARGO_BIN_EXE_umber")),
        "gentle.tex",
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
#[ignore = "requires pinned TRIP inputs and TeXware tools"]
#[allow(clippy::disallowed_methods)] // Explicit host-side conformance process.
fn e2e_conformance_trip() {
    let root = repo_root();
    let target = target_dir(&root);
    let output = Command::new(root.join("scripts/trip.sh"))
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", &target)
        .env("UMBER_BIN", env!("CARGO_BIN_EXE_umber"))
        .arg("umber-artifacts")
        .arg("--offline")
        .output()
        .expect("run TRIP artifact producer");
    assert!(
        output.status.success(),
        "TRIP artifact production failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    compare_dvi_files(
        &root.join("third_party/trip/trip.dvi"),
        &target.join("trip/umber/trip.dvi"),
        &target.join("conformance-triage"),
        "trip",
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}
