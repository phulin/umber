//! The coverage guard that lets `cargo test --tests` be the routine gate.
//!
//! `cargo test --tests` selects the workspace's `default-members`. When that
//! list was a hand-picked subset, thirteen members -- the nine `bib-*` crates,
//! `umber-interrupt`, `refexec`, `profile-analyzer`, and `umber-wasm` -- were
//! executed by no routine command at all, and `bib-engine`'s integration
//! binary alone held roughly 1295 tests that nothing ran (`umber2-johp.211`).
//! Nothing had rotted when the gap was measured, which is the danger rather
//! than the reassurance: `tools/tex-command-stream` had rotted out of
//! compiling through exactly the same gap (`umber2-johp.121`) and no test run
//! reported it.
//!
//! `scripts/run-native-tests.py` used to close that gap by wrapping Cargo and
//! selecting `--workspace` minus a declared exclusion list. This test closes
//! it instead, which is strictly better: the invariant is now enforced by the
//! suite under the command everyone already runs, rather than by remembering
//! to run a particular wrapper.
//!
//! An omission is legal only if it appears in `OMITTED` with a reason and the
//! gate that does run it. A member missing from both `default-members` and
//! `OMITTED` fails here.

#![allow(clippy::disallowed_methods)] // host-only workspace-manifest audit

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Members deliberately absent from `default-members`, each naming the tier
/// that does run it. This is not permission to leave a crate untested.
const OMITTED: &[(&str, &str)] = &[(
    "umber-wasm",
    "its tests are `#[wasm_bindgen_test]`, which registers no test on a host \
     target: selecting it would build a cdylib and run exactly zero tests. \
     `scripts/check-wasm.sh` runs them for real with \
     `wasm-pack test --headless --firefox crates/umber-wasm`.",
)];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above test-support")
        .to_path_buf()
}

fn metadata(root: &Path) -> Value {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse cargo metadata")
}

/// Package name to manifest directory, relative to the workspace root.
fn members(root: &Path, meta: &Value) -> BTreeMap<String, String> {
    meta["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .map(|package| {
            let name = package["name"].as_str().expect("package name").to_owned();
            let manifest = Path::new(package["manifest_path"].as_str().expect("manifest path"));
            let directory = manifest
                .parent()
                .expect("manifest has a directory")
                .strip_prefix(root)
                .expect("member lies inside the workspace")
                .to_string_lossy()
                .into_owned();
            (name, directory)
        })
        .collect()
}

fn default_member_directories(root: &Path, meta: &Value) -> BTreeSet<String> {
    let by_id: BTreeMap<&str, &Value> = meta["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .map(|package| (package["id"].as_str().expect("package id"), package))
        .collect();
    meta["workspace_default_members"]
        .as_array()
        .expect("workspace_default_members array")
        .iter()
        .map(|id| {
            let package = by_id[id.as_str().expect("default member id")];
            Path::new(package["manifest_path"].as_str().expect("manifest path"))
                .parent()
                .expect("manifest has a directory")
                .strip_prefix(root)
                .expect("member lies inside the workspace")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[test]
fn default_members_cover_every_host_testable_crate() {
    let root = repo_root();
    let meta = metadata(&root);
    let members = members(&root, &meta);
    let selected = default_member_directories(&root, &meta);
    let omitted: BTreeMap<&str, &str> = OMITTED.iter().copied().collect();

    let mut unreached = Vec::new();
    for (name, directory) in &members {
        if selected.contains(directory) || omitted.contains_key(name.as_str()) {
            continue;
        }
        unreached.push(format!("  {name} ({directory})"));
    }
    assert!(
        unreached.is_empty(),
        "these workspace members are not selected by `cargo test --tests`, and \
         no omission declares where they do run:\n{}\n\nAdd each to \
         `default-members` in Cargo.toml, or to OMITTED in this file with the \
         tier that runs it. A member reachable by neither is one the routine \
         gate silently skips.",
        unreached.join("\n")
    );

    let stale: Vec<&str> = omitted
        .keys()
        .copied()
        .filter(|name| !members.contains_key(*name))
        .collect();
    assert!(
        stale.is_empty(),
        "OMITTED names packages the workspace no longer has: {stale:?}. A stale \
         excuse must not outlive the thing it excused."
    );

    let contradictory: Vec<&str> = omitted
        .keys()
        .copied()
        .filter(|name| members.get(*name).is_some_and(|dir| selected.contains(dir)))
        .collect();
    assert!(
        contradictory.is_empty(),
        "OMITTED claims these run elsewhere, but `default-members` selects them \
         here too: {contradictory:?}. Delete the omission."
    );
}

/// `[workspace] exclude` directories are not members at all, so the test above
/// cannot see them: `cargo metadata` never reports them. Pushing a crate out
/// of the workspace must not quietly take its tests out of every gate on the
/// way, so each excluded directory names the tier that runs it.
#[test]
fn every_excluded_workspace_directory_names_its_tier() {
    const EXCLUDED: &[(&str, &str)] = &[
        ("tools/corpus-sync", "check-tools.sh"),
        ("tools/fixturegen", "check-tools.sh"),
        ("tools/texlive-wasm-publish", "check-tools.sh"),
    ];

    let root = repo_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read root manifest");
    let declared: BTreeSet<&str> = manifest
        .split_once("exclude = [")
        .map(|(_, rest)| rest.split_once(']').expect("exclude list closes").0)
        .expect("root manifest declares an exclude list")
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').strip_prefix('"'))
        .filter_map(|line| line.strip_suffix('"'))
        .collect();
    let accounted: BTreeSet<&str> = EXCLUDED.iter().map(|(path, _)| *path).collect();

    assert_eq!(
        declared, accounted,
        "the root manifest's `[workspace] exclude` list and this test's \
         declaration disagree. Every excluded directory is its own workspace \
         with its own lockfile, unreachable from `--workspace`, so each must \
         name the tier that runs it."
    );
    for (path, _) in EXCLUDED {
        assert!(
            root.join(path).join("Cargo.toml").is_file(),
            "excluded directory {path} has no manifest; delete the entry"
        );
    }
}
