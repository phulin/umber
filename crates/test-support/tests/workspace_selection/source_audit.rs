//! Reject source constructs that make apparent test evidence unreachable.
//!
//! This module stays under `workspace_selection` so the same routine test
//! executable proves both that the audit runs and that its crate remains in
//! the workspace's default selection.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::{metadata, repo_root};

const CFG_ANY: &str = "cfg(any())";
const DORMANT_TEST_MODULE: &str = "cfg(test) module under a library with `test = false`";

// Keep this empty: source-level test authority debt is not accepted. The
// parameter remains explicit so the scanner's stale-exception behavior stays
// covered by its positive tests below.
const REVIEWED_EXCEPTIONS: &[&str] = &[];

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Site {
    path: String,
    line: usize,
    problem: &'static str,
}

impl Site {
    fn key(&self) -> String {
        format!("{}:{}:{}", self.path, self.line, self.problem)
    }

    fn diagnostic(&self) -> String {
        let remedy = if self.problem == CFG_ANY {
            "delete the dead conditional or replace it with an active configuration"
        } else {
            "enable the library test target or move the evidence to an active integration test"
        };
        format!(
            "{}:{}: {} makes test evidence inactive; {remedy}",
            self.path, self.line, self.problem
        )
    }
}

fn compact(line: &str) -> String {
    line.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn begins_module(compact_line: &str) -> bool {
    let line = compact_line
        .strip_prefix("pub(crate)")
        .or_else(|| compact_line.strip_prefix("pub(super)"))
        .or_else(|| compact_line.strip_prefix("pub(self)"))
        .or_else(|| compact_line.strip_prefix("pub"))
        .unwrap_or(compact_line);
    line.starts_with("mod")
        && line[3..]
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
}

fn scan_source(path: &str, source: &str, disabled_library_test: bool) -> Vec<Site> {
    let lines: Vec<&str> = source.lines().collect();
    let mut sites = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let line = compact(line);
        if line.starts_with("#[cfg(any())]") {
            sites.push(Site {
                path: path.to_owned(),
                line: index + 1,
                problem: CFG_ANY,
            });
        }
        if !disabled_library_test || !line.starts_with("#[cfg(test)]") {
            continue;
        }

        let suffix = line.strip_prefix("#[cfg(test)]").expect("marker exists");
        let module_line = if suffix.is_empty() {
            lines[index + 1..]
                .iter()
                .map(|line| compact(line))
                .find(|line| !line.is_empty() && !line.starts_with("#["))
                .unwrap_or_default()
        } else {
            suffix.to_owned()
        };
        if begins_module(&module_line) {
            sites.push(Site {
                path: path.to_owned(),
                line: index + 1,
                problem: DORMANT_TEST_MODULE,
            });
        }
    }
    sites
}

fn disabled_library_roots(root: &Path, meta: &Value) -> Vec<PathBuf> {
    meta["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .flat_map(|package| package["targets"].as_array().expect("targets array"))
        .filter(|target| {
            !target["test"].as_bool().expect("target test flag")
                && target["kind"]
                    .as_array()
                    .expect("target kinds")
                    .iter()
                    .any(|kind| matches!(kind.as_str(), Some("lib" | "proc-macro")))
        })
        .map(|target| {
            Path::new(target["src_path"].as_str().expect("target source path"))
                .parent()
                .expect("library source has a parent")
                .strip_prefix(root)
                .expect("library source lies under repository root")
                .to_owned()
        })
        .collect()
}

fn tracked_production_rust(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .args(["ls-files", "--", "*.rs"])
        .current_dir(root)
        .output()
        .expect("list tracked Rust sources");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git paths are UTF-8")
        .lines()
        .map(PathBuf::from)
        .filter(|path| {
            path.components()
                .any(|component| component.as_os_str() == "src")
                || path.file_name().is_some_and(|name| name == "build.rs")
        })
        .collect()
}

fn audit(sites: Vec<Site>, exceptions: &[&str]) -> Result<(), String> {
    let found: BTreeSet<String> = sites.iter().map(Site::key).collect();
    let reviewed: BTreeSet<String> = exceptions.iter().map(|site| (*site).to_owned()).collect();
    let violations: Vec<String> = sites
        .iter()
        .filter(|site| !reviewed.contains(&site.key()))
        .map(Site::diagnostic)
        .collect();
    let stale: Vec<&String> = reviewed.difference(&found).collect();
    if violations.is_empty() && stale.is_empty() {
        return Ok(());
    }

    let mut message = String::from("inactive test-authority source audit failed");
    if !violations.is_empty() {
        message.push_str("\n\nnew violations:\n  ");
        message.push_str(&violations.join("\n  "));
    }
    if !stale.is_empty() {
        message
            .push_str("\n\nstale reviewed exceptions (delete them after removing the debt):\n  ");
        message.push_str(&stale.into_iter().cloned().collect::<Vec<_>>().join("\n  "));
    }
    Err(message)
}

#[test]
fn production_sources_have_active_test_authorities() {
    let root = repo_root();
    let disabled_roots = disabled_library_roots(&root, &metadata(&root));
    let sites = tracked_production_rust(&root)
        .into_iter()
        .flat_map(|path| {
            let disabled = disabled_roots.iter().any(|source| path.starts_with(source));
            let source = std::fs::read_to_string(root.join(&path))
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            scan_source(&path.to_string_lossy(), &source, disabled)
        })
        .collect();
    if let Err(message) = audit(sites, REVIEWED_EXCEPTIONS) {
        panic!("{message}");
    }
}

#[test]
fn source_audit_accepts_active_test_modules() {
    let source = "// `#[cfg(any())]` is text, not an attribute.\n\
                  const DESCRIPTION: &str = \"#[cfg(any())]\";\n\
                  #[cfg(any(test, feature = \"extra\"))]\n\
                  fn helper() {}\n\n\
                  #[cfg(test)]\n\
                  mod tests;\n";
    audit(scan_source("src/lib.rs", source, false), &[]).expect("active library tests pass");
}

#[test]
fn source_audit_rejects_cfg_any_actionably() {
    let error = audit(
        scan_source("src/lib.rs", "#[cfg(any())]\nmod evidence;\n", false),
        &[],
    )
    .expect_err("unconditionally dead configuration must fail");
    assert!(error.contains("src/lib.rs:1: cfg(any()) makes test evidence inactive"));
    assert!(error.contains("replace it with an active configuration"));
}

#[test]
fn source_audit_rejects_tests_disabled_by_manifest_actionably() {
    let error = audit(
        scan_source("src/lib.rs", "#[cfg(test)]\nmod tests;\n", true),
        &[],
    )
    .expect_err("a disabled library test target must make its modules fail");
    assert!(error.contains("src/lib.rs:1: cfg(test) module under a library with `test = false`"));
    assert!(error.contains("move the evidence to an active integration test"));
}

#[test]
fn source_audit_rejects_stale_reviewed_exceptions() {
    let error = audit(Vec::new(), &["src/lib.rs:7:cfg(any())"])
        .expect_err("an exception must not outlive its debt");
    assert!(error.contains("stale reviewed exceptions"));
    assert!(error.contains("src/lib.rs:7:cfg(any())"));
}
