#![allow(clippy::disallowed_methods)] // host-side audit of committed fixtures

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const PINNED_COMMIT: &str = "74252e608e5f8115375c532eb25416430a9f52eb";
const IMPORTED_FILE_COUNT: usize = 113;
const UPSTREAM_MODULE_COUNT: usize = 51;
const UPSTREAM_ASSERTION_COUNT: usize = 1_275;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    schema: u32,
    upstream_repository: String,
    upstream_commit: String,
    compatibility_version: String,
    license: String,
    files: Vec<FixtureFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureFile {
    path: String,
    upstream_path: String,
    bytes: u64,
    sha256: String,
}

#[test]
fn fixture_manifest_is_complete_and_pinned() {
    let root = fixture_root();
    let manifest_bytes = fs::read(root.join("manifest.json"))
        .unwrap_or_else(|error| panic!("failed to read bibliography fixture manifest: {error}"));
    let manifest: FixtureManifest = serde_json::from_slice(&manifest_bytes)
        .unwrap_or_else(|error| panic!("invalid bibliography fixture manifest: {error}"));

    assert_eq!(manifest.schema, 1);
    assert_eq!(
        manifest.upstream_repository,
        "https://github.com/plk/biber.git"
    );
    assert_eq!(manifest.upstream_commit, PINNED_COMMIT);
    assert_eq!(manifest.compatibility_version, "2.22 beta");
    assert_eq!(manifest.license, "Artistic-2.0");
    assert_eq!(manifest.files.len(), IMPORTED_FILE_COUNT);

    let mut declared = BTreeSet::new();
    for fixture in &manifest.files {
        assert!(
            declared.insert(fixture.path.clone()),
            "duplicate manifest path: {}",
            fixture.path
        );
        let expected_upstream_path = if fixture.path == "LICENSE.Artistic-2.0" {
            "LICENSE".to_owned()
        } else {
            format!("t/{}", fixture.path)
        };
        assert_eq!(fixture.upstream_path, expected_upstream_path);
        let path = root.join(&fixture.path);
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!("failed to read pinned fixture {}: {error}", path.display())
        });
        assert_eq!(
            bytes.len() as u64,
            fixture.bytes,
            "byte length drift for {}",
            fixture.path
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            fixture.sha256,
            "SHA-256 drift for {}",
            fixture.path
        );
    }

    let present = imported_paths(&root);
    assert_eq!(
        declared, present,
        "manifest must name every imported file and no absent files"
    );
}

#[test]
fn translated_suite_has_no_compatibility_allowances() {
    let upstream = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/upstream");
    let mut modules = 0;
    let mut assertions = 0;
    for entry in fs::read_dir(&upstream)
        .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", upstream.display()))
    {
        let path = entry.expect("valid upstream directory entry").path();
        if path.extension().is_none_or(|extension| extension != "rs")
            || path.file_name().is_some_and(|name| name == "mod.rs")
        {
            continue;
        }
        modules += 1;
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        assertions += source.matches("#[test]").count();
        let expected_failure_marker = ["x", "fail"].concat();
        let unexpected_pass_marker = ["X", "PASS"].concat();
        let ignored_test_marker = ["#[", "ignore", "]"].concat();
        let expected_panic_marker = ["#[", "should_panic", "]"].concat();
        for forbidden in [
            ignored_test_marker.as_str(),
            expected_panic_marker.as_str(),
            expected_failure_marker.as_str(),
            unexpected_pass_marker.as_str(),
        ] {
            assert!(
                !source.contains(forbidden),
                "compatibility allowance `{forbidden}` remains in {}",
                path.display()
            );
        }
    }
    assert_eq!(modules, UPSTREAM_MODULE_COUNT);
    assert_eq!(assertions, UPSTREAM_ASSERTION_COUNT);
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/bib/upstream-2.22")
}

fn imported_paths(root: &Path) -> BTreeSet<String> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", directory.display()));
        for entry in entries {
            let entry =
                entry.unwrap_or_else(|error| panic!("failed to enumerate fixture: {error}"));
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().is_some_and(|name| name != "manifest.json") {
                let relative = path
                    .strip_prefix(root)
                    .expect("fixture must be below corpus root");
                paths.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    paths
}
