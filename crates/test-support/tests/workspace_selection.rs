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

#[path = "workspace_selection/source_audit.rs"]
mod source_audit;

/// Members deliberately absent from `default-members`, each naming the check
/// that does run it. This is not permission to leave a crate untested.
const OMITTED: &[(&str, &str)] = &[(
    "umber-wasm",
    "its tests are `#[wasm_bindgen_test]`, which registers no test on a host \
     target: selecting it would build a cdylib and run exactly zero tests. \
     `scripts/check-wasm.sh` runs them for real with \
     `wasm-pack test --headless --firefox crates/umber-wasm`.",
)];

fn repo_root() -> PathBuf {
    test_support::repository_root()
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
         check that runs it. A member reachable by neither is one the routine \
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
/// of the workspace must not quietly take its tests out of every check on the
/// way, so each excluded directory names the check that runs it.
#[test]
fn every_excluded_workspace_directory_names_its_check() {
    const EXCLUDED: &[(&str, &str)] = &[
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
         name the check that runs it."
    );
    for (path, _) in EXCLUDED {
        assert!(
            root.join(path).join("Cargo.toml").is_file(),
            "excluded directory {path} has no manifest; delete the entry"
        );
    }
}

/// A format-schema bump crosses workspace, excluded-tool, release-lock,
/// browser-fixture, benchmark, and documentation boundaries. Keep the
/// deliberately explicit inventory here so a new bump cannot leave a
/// consumer outside the crate that owns the version.
#[test]
fn current_format_schema_receipts_cover_every_release_surface() {
    struct HistoricalSchema10Use {
        path: &'static str,
        marker: &'static str,
        expected_count: usize,
        reason: &'static str,
    }

    const HISTORICAL_SCHEMA_10_USES: &[HistoricalSchema10Use] = &[
        HistoricalSchema10Use {
            path: "crates/umber-distribution/src/tests.rs",
            marker: "\"formatSchema\":10",
            expected_count: 3,
            reason: "schema-agnostic distribution parser fixtures",
        },
        HistoricalSchema10Use {
            path: "crates/tex-exec/src/shipout/transaction.rs",
            marker: "artifact_schema: 10,",
            expected_count: 1,
            reason: "memo artifact schema, not the frozen-format schema",
        },
        HistoricalSchema10Use {
            path: "docs/architecture.md",
            marker: "Schema 10 introduced authoritative fixed-width sections",
            expected_count: 1,
            reason: "frozen-format schema history",
        },
        HistoricalSchema10Use {
            path: "docs/architecture.md",
            marker: "Schemas 9 and 10 are rejected rather than guessed",
            expected_count: 1,
            reason: "frozen-format migration policy",
        },
        HistoricalSchema10Use {
            path: "docs/frozen_format.md",
            marker: "schema 10 to make token-parameter cell presence",
            expected_count: 1,
            reason: "schema-11 migration rationale",
        },
        HistoricalSchema10Use {
            path: "docs/frozen_format.md",
            marker: "## Migration from schemas 9 and 10",
            expected_count: 1,
            reason: "frozen-format migration heading",
        },
        HistoricalSchema10Use {
            path: "docs/frozen_format.md",
            marker: "Schema 10 introduced the",
            expected_count: 1,
            reason: "frozen-format schema history",
        },
        HistoricalSchema10Use {
            path: "docs/frozen_format.md",
            marker: "Schema 11 is therefore a clean boundary: the loader rejects schemas 9 and 10",
            expected_count: 1,
            reason: "frozen-format migration policy",
        },
        HistoricalSchema10Use {
            path: "docs/frozen_format.md",
            marker: "any schema other than 11, including schemas 9 and 10",
            expected_count: 1,
            reason: "frozen-format compatibility failure",
        },
        HistoricalSchema10Use {
            path: "docs/frozen_format.md",
            marker: "schema 10, or partially migrated images",
            expected_count: 1,
            reason: "frozen-format compatibility exclusion",
        },
        HistoricalSchema10Use {
            path: "docs/format_cache.md",
            marker: "historical 588,488-byte schema-10 image",
            expected_count: 1,
            reason: "historical Plain reproducibility result",
        },
        HistoricalSchema10Use {
            path: "docs/format_cache.md",
            marker: "historical minimal fixed-clock 38,304-byte schema-10 image",
            expected_count: 1,
            reason: "historical e-TeX reproducibility result",
        },
        HistoricalSchema10Use {
            path: "docs/format_cache.md",
            marker: "historical packaged Plain test loaded the same schema-10 bytes",
            expected_count: 1,
            reason: "historical browser compatibility result",
        },
    ];

    let root = repo_root();
    let schema_source =
        std::fs::read_to_string(root.join("crates/tex-state/src/format_container.rs"))
            .expect("read format schema owner");
    let schema = schema_source
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("pub(crate) const SCHEMA_VERSION: u32 = ")
                .and_then(|value| value.strip_suffix(';'))
                .and_then(|value| value.parse::<u32>().ok())
        })
        .expect("format schema owner declares a literal SCHEMA_VERSION");

    let receipts = [
        (
            "crates/umber-wasm/assets/plain-source.lock",
            format!("format_schema {schema}"),
        ),
        (
            "crates/umber-wasm/assets/plain-format.json",
            format!("\"formatSchema\": {schema}"),
        ),
        (
            "crates/umber-wasm/browser-tests/fixture.js",
            format!("formatSchemaVersion() === {schema}"),
        ),
        (
            "crates/umber-wasm/js/manifest-resolver.test.js",
            format!("formatSchema: {schema}"),
        ),
        ("tests/latex-source.lock", format!("format_schema {schema}")),
        (
            "tools/texlive-wasm-publish/src/tests.rs",
            format!("assert_eq!(format.format_schema, {schema})"),
        ),
        (
            "benchmarks/tex-state/src/bin/format_cache_profile.rs",
            format!("profile input must be schema-{schema}"),
        ),
        (
            "docs/architecture.md",
            format!("currently\nat schema {schema}"),
        ),
    ];

    for (path, expected) in receipts {
        let contents = std::fs::read_to_string(root.join(path))
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        assert!(
            contents.contains(&expected),
            "{path} must receipt current format schema {schema} with {expected:?}"
        );
    }

    let tracked = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&root)
        .output()
        .expect("list tracked release surfaces");
    assert!(
        tracked.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&tracked.stderr)
    );
    let mut classified_counts = vec![0_usize; HISTORICAL_SCHEMA_10_USES.len()];
    let mut unclassified = Vec::new();
    for path in tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = std::str::from_utf8(path).expect("tracked path is UTF-8");
        if path == "crates/test-support/tests/workspace_selection.rs" {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        for (line_index, line) in contents.lines().enumerate() {
            let lowercase = line.to_ascii_lowercase();
            let compact: String = lowercase
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .collect();
            let mentions_schema_10 =
                compact.contains("schema10") || compact.contains("schemas9and10");
            if !mentions_schema_10 {
                continue;
            }
            let matches: Vec<usize> = HISTORICAL_SCHEMA_10_USES
                .iter()
                .enumerate()
                .filter_map(|(index, classified)| {
                    (classified.path == path && line.contains(classified.marker)).then_some(index)
                })
                .collect();
            if matches.len() == 1 {
                classified_counts[matches[0]] += 1;
            } else {
                unclassified.push(format!("  {path}:{}: {line}", line_index + 1));
            }
        }
    }
    assert!(
        unclassified.is_empty(),
        "unclassified schema-10 text can be a stale current format receipt:\n{}\n\n\
         Replace stale receipts with schema {schema}, or classify legitimate \
         historical, migration, or schema-agnostic text explicitly.",
        unclassified.join("\n")
    );
    let stale_classifications: Vec<String> = HISTORICAL_SCHEMA_10_USES
        .iter()
        .zip(classified_counts)
        .filter(|(classified, count)| *count != classified.expected_count)
        .map(|(classified, count)| {
            format!(
                "  {} / {:?}: matched {count} lines ({})",
                classified.path, classified.marker, classified.reason
            )
        })
        .collect();
    assert!(
        stale_classifications.is_empty(),
        "schema-10 classifications must each match exactly once:\n{}",
        stale_classifications.join("\n")
    );

    let plain_format =
        std::fs::read(root.join("crates/umber-wasm/assets/plain.fmt")).expect("read Plain format");
    assert_eq!(
        plain_format.get(8..12),
        Some(schema.to_le_bytes().as_slice()),
        "packaged Plain format header must receipt current format schema {schema}"
    );
}
