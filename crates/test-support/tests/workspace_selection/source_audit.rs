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

/// Temporary migration debt inventoried by
/// `docs/tex_exec_dormant_test_ledger.md` (`umber2-vgjr.15.1`). Exact source
/// coordinates make additions and movement fail; deleting a site also requires
/// deleting its now-stale exception.
const REVIEWED_EXCEPTIONS: &[&str] = &[
    "crates/tex-exec/src/align/packaging.rs:2:cfg(any())",
    "crates/tex-exec/src/align/widths.rs:5:cfg(any())",
    "crates/tex-exec/src/align/widths/resolution.rs:2:cfg(any())",
    "crates/tex-exec/src/align/widths/set.rs:2:cfg(any())",
    "crates/tex-exec/src/assignments/admissibility.rs:182:cfg(any())",
    "crates/tex-exec/src/assignments/mod.rs:6:cfg(any())",
    "crates/tex-exec/src/assignments/mod.rs:10:cfg(any())",
    "crates/tex-exec/src/assignments/mod.rs:13:cfg(any())",
    "crates/tex-exec/src/assignments/mod.rs:15:cfg(any())",
    "crates/tex-exec/src/assignments/mod.rs:18:cfg(any())",
    "crates/tex-exec/src/assignments/mod.rs:24:cfg(any())",
    "crates/tex-exec/src/assignments/mod.rs:27:cfg(any())",
    "crates/tex-exec/src/box_runtime/hmode.rs:30:cfg(any())",
    "crates/tex-exec/src/box_runtime/hmode.rs:355:cfg(any())",
    "crates/tex-exec/src/box_runtime/hmode.rs:372:cfg(any())",
    "crates/tex-exec/src/box_runtime/hmode.rs:439:cfg(any())",
    "crates/tex-exec/src/box_runtime/hmode.rs:684:cfg(any())",
    "crates/tex-exec/src/box_runtime/mod.rs:12:cfg(any())",
    "crates/tex-exec/src/effective_tail.rs:61:cfg(any())",
    "crates/tex-exec/src/job.rs:302:cfg(any())",
    "crates/tex-exec/src/job.rs:846:cfg(any())",
    "crates/tex-exec/src/job_output.rs:52:cfg(any())",
    "crates/tex-exec/src/job_output.rs:125:cfg(any())",
    "crates/tex-exec/src/main_control.rs:754:cfg(any())",
    "crates/tex-exec/src/main_control.rs:864:cfg(any())",
    "crates/tex-exec/src/main_control.rs:1702:cfg(any())",
    "crates/tex-exec/src/main_control.rs:1714:cfg(any())",
    "crates/tex-exec/src/main_control.rs:1724:cfg(any())",
    "crates/tex-exec/src/main_control.rs:1731:cfg(any())",
    "crates/tex-exec/src/main_control.rs:1750:cfg(any())",
    "crates/tex-exec/src/main_control.rs:4705:cfg(any())",
    "crates/tex-exec/src/main_control.rs:9993:cfg(any())",
    "crates/tex-exec/src/main_control.rs:12083:cfg(any())",
    "crates/tex-exec/src/main_control.rs:18131:cfg(any())",
    "crates/tex-exec/src/math/mod.rs:92:cfg(any())",
    "crates/tex-exec/src/mode.rs:169:cfg(any())",
    "crates/tex-exec/src/mode.rs:177:cfg(any())",
    "crates/tex-exec/src/mode.rs:428:cfg(any())",
    "crates/tex-exec/src/mode.rs:485:cfg(any())",
    "crates/tex-exec/src/mode.rs:494:cfg(any())",
    "crates/tex-exec/src/mode.rs:1285:cfg(any())",
    "crates/tex-exec/src/mode/journal.rs:196:cfg(any())",
    "crates/tex-exec/src/mode/journal.rs:240:cfg(any())",
    "crates/tex-exec/src/node_dump.rs:1191:cfg(any())",
    "crates/tex-exec/src/pack_report.rs:26:cfg(any())",
    "crates/tex-exec/src/page_builder.rs:908:cfg(any())",
    "crates/tex-exec/src/paragraph_end.rs:9:cfg(any())",
    "crates/tex-exec/src/paragraph_end.rs:24:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:60:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:229:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:241:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:255:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:278:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:298:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:828:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:848:cfg(any())",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:893:cfg(any())",
    "crates/tex-exec/src/paragraph_end/runtime.rs:259:cfg(any())",
    "crates/tex-exec/src/paragraph_end/runtime.rs:284:cfg(any())",
    "crates/tex-exec/src/paragraph_end/runtime.rs:459:cfg(any())",
    "crates/tex-exec/src/paragraph_end/runtime.rs:490:cfg(any())",
    "crates/tex-exec/src/paragraph_end/runtime.rs:501:cfg(any())",
    "crates/tex-exec/src/paragraph_end/runtime.rs:808:cfg(any())",
    "crates/tex-exec/src/splitting.rs:104:cfg(any())",
    "crates/tex-exec/src/main_control.rs:18029:cfg(test) module under a library with `test = false`",
    "crates/tex-exec/src/node_dump.rs:1275:cfg(test) module under a library with `test = false`",
    "crates/tex-exec/src/packing_params.rs:12:cfg(test) module under a library with `test = false`",
    "crates/tex-exec/src/page_output.rs:570:cfg(test) module under a library with `test = false`",
    "crates/tex-exec/src/paragraph_end/hyphenation.rs:995:cfg(test) module under a library with `test = false`",
    "crates/tex-exec/src/shipout/transaction.rs:16:cfg(test) module under a library with `test = false`",
];

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
