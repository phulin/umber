use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::layout_migration::{self, Mode};

const PLAN_SCHEMA: &str = "umber-fixture-cohort-plan-v1";

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
    Ok(test_support::closed_case::StagedCase::validate(root)?
        .inventory()
        .clone())
}

#[cfg(test)]
mod tests;
