use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::layout_migration::{self, Mode};

const PLAN_SCHEMA: &str = "umber-fixture-cohort-plan-v1";
const INVENTORY_SCHEMA: &str = "closed-case-v1";

#[derive(Debug, Deserialize)]
struct CohortPlan {
    schema: String,
    repository: String,
    cases: Vec<CohortCase>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CohortCase {
    pub staged: String,
    pub destination: String,
    pub authorities: Vec<String>,
}

#[derive(Serialize)]
struct CliResult<'a> {
    schema: &'static str,
    status: &'a str,
    mode: &'a str,
    cases: usize,
    report: String,
}

pub(crate) fn run_cli(args: Vec<String>) -> Result<()> {
    let [mode, plan_path] = args.as_slice() else {
        bail!("--cohort-transaction requires (--plan|--apply) PLAN.json");
    };
    let mode = match mode.as_str() {
        "--plan" => Mode::Plan,
        "--apply" => Mode::Apply,
        _ => bail!("--cohort-transaction requires --plan or --apply"),
    };
    let bytes = fs::read(plan_path).with_context(|| format!("read cohort plan {plan_path}"))?;
    let plan: CohortPlan = serde_json::from_slice(&bytes).context("parse cohort plan JSON")?;
    ensure!(plan.schema == PLAN_SCHEMA, "unsupported cohort plan schema");
    let repository = Path::new(&plan.repository)
        .canonicalize()
        .context("canonicalize cohort repository")?;
    validate_repository(&repository)?;
    let report = layout_migration::run_staged_cohort(&repository, &plan.cases, mode)?;
    println!(
        "{}",
        serde_json::to_string(&CliResult {
            schema: "umber-fixture-cohort-result-v1",
            status: "ok",
            mode: if mode == Mode::Plan {
                "plan"
            } else {
                "committed"
            },
            cases: plan.cases.len(),
            report,
        })?
    );
    Ok(())
}

fn validate_repository(repository: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("resolve cohort Git authority")?;
    ensure!(
        output.status.success(),
        "cohort repository is not a Git checkout"
    );
    let root = Path::new(std::str::from_utf8(&output.stdout)?.trim()).canonicalize()?;
    ensure!(
        root == repository,
        "cohort repository must be the Git checkout root"
    );
    Ok(())
}

pub(crate) fn validate_git_authority(repository: &Path, relative: &str) -> Result<()> {
    let path = repository.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("inspect cohort authority {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "symlink authority is forbidden"
    );
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "authority is not a regular file or directory"
    );
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "-z", "--"])
        .arg(relative)
        .output()
        .context("inventory cohort Git authority")?;
    ensure!(
        output.status.success(),
        "git ls-files failed for {relative}"
    );
    let tracked: BTreeSet<_> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8(entry.to_vec()))
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        !tracked.is_empty(),
        "cohort authority is not tracked by Git: {relative}"
    );
    let status = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["status", "--porcelain=v1", "--untracked-files=all", "--"])
        .arg(relative)
        .output()
        .context("check cohort Git authority status")?;
    ensure!(status.status.success(), "git status failed for {relative}");
    ensure!(
        status.stdout.is_empty(),
        "cohort authority is not clean in Git: {relative}"
    );
    Ok(())
}

pub(crate) fn validate_staged_case(
    root: &Path,
) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("inspect staged case {}", root.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "staged case is not a directory"
    );
    let inventory_path = root.join("case.inventory");
    let declared = if inventory_path.is_file() {
        let text = fs::read_to_string(&inventory_path)
            .with_context(|| format!("read {}", inventory_path.display()))?;
        let mut lines = text.lines();
        ensure!(
            lines.next() == Some(INVENTORY_SCHEMA),
            "{} must begin with {INVENTORY_SCHEMA}",
            inventory_path.display()
        );
        let mut declared = BTreeSet::new();
        for name in lines.filter(|line| !line.is_empty()) {
            let path = Path::new(name);
            ensure!(
                path.components().count() == 1
                    && path
                        .components()
                        .all(|part| matches!(part, Component::Normal(_))),
                "unsafe staged inventory entry {name:?}"
            );
            ensure!(
                name != "case.inventory",
                "case.inventory is metadata, not a payload"
            );
            ensure!(
                declared.insert(name.to_owned()),
                "duplicate staged inventory entry {name}"
            );
        }
        ensure!(!declared.is_empty(), "staged case inventory is empty");
        Some(declared)
    } else {
        None
    };
    let mut actual = BTreeSet::new();
    let mut bytes = std::collections::BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            kind.is_file() && !kind.is_symlink(),
            "staged case contains a non-regular entry"
        );
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF-8 staged filename"))?;
        actual.insert(name.clone());
        bytes.insert(name, fs::read(entry.path())?);
    }
    if let Some(declared) = declared {
        let expected = declared
            .iter()
            .cloned()
            .chain(std::iter::once("case.inventory".to_owned()))
            .collect();
        ensure!(
            actual == expected,
            "staged closed inventory mismatch: declared={expected:?}, present={actual:?}"
        );
    } else {
        ensure!(!actual.is_empty(), "staged case inventory is empty");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
