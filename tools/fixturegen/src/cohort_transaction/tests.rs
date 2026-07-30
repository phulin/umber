use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Result, bail};
use tempfile::TempDir;

use super::{CohortCase, validate_staged_case};
use crate::layout_migration::{
    MigrationFs, Mode, RealFs, run_staged_cohort, run_staged_cohort_with_fs,
};

fn repository() -> TempDir {
    let temp = tempfile::tempdir().expect("temp");
    Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(temp.path())
        .status()
        .expect("git init");
    Command::new("git")
        .arg("-C")
        .arg(temp.path())
        .args(["config", "user.email", "fixture@example.test"])
        .status()
        .expect("email");
    Command::new("git")
        .arg("-C")
        .arg(temp.path())
        .args(["config", "user.name", "Fixture Test"])
        .status()
        .expect("name");
    fs::create_dir(temp.path().join("cases")).expect("destination parent");
    for name in ["one", "two"] {
        fs::create_dir_all(temp.path().join(format!("old/{name}"))).expect("old");
        fs::write(temp.path().join(format!("old/{name}/old.txt")), name).expect("old payload");
        stage(temp.path(), name, format!("new-{name}").as_bytes());
    }
    Command::new("git")
        .arg("-C")
        .arg(temp.path())
        .args(["add", "old"])
        .status()
        .expect("add");
    Command::new("git")
        .arg("-C")
        .arg(temp.path())
        .args(["commit", "-qm", "fixture"])
        .status()
        .expect("commit");
    temp
}

fn stage(repository: &Path, name: &str, payload: &[u8]) {
    let root = repository.join(format!("stage/{name}"));
    fs::create_dir_all(&root).expect("stage");
    fs::write(root.join("case.inventory"), "closed-case-v1\npayload.txt\n").expect("inventory");
    fs::write(root.join("payload.txt"), payload).expect("payload");
}

fn cases() -> Vec<CohortCase> {
    ["one", "two"]
        .into_iter()
        .map(|name| CohortCase {
            staged: format!("stage/{name}"),
            destination: format!("cases/{name}"),
            authorities: vec![format!("old/{name}")],
        })
        .collect()
}

#[test]
fn rejects_invalid_staged_case_before_authority_mutation() {
    let repo = repository();
    fs::write(repo.path().join("stage/two/extra"), b"extra").expect("extra");
    let error = run_staged_cohort(repo.path(), &cases(), Mode::Apply).expect_err("invalid");
    assert!(format!("{error:#}").contains("closed inventory mismatch"));
    assert!(repo.path().join("old/one/old.txt").is_file());
}

#[test]
fn rejects_destination_collision_before_authority_mutation() {
    let repo = repository();
    let mut plan = cases();
    plan[1].destination = plan[0].destination.clone();
    let error = run_staged_cohort(repo.path(), &plan, Mode::Apply).expect_err("collision");
    assert!(format!("{error:#}").contains("duplicate cohort destination"));
    assert!(repo.path().join("old/one/old.txt").is_file());
}

#[test]
fn two_case_commit_is_atomic_and_completed_retry_is_idempotent() {
    let repo = repository();
    run_staged_cohort(repo.path(), &cases(), Mode::Apply).expect("apply");
    assert_eq!(
        fs::read(repo.path().join("cases/one/payload.txt")).expect("one"),
        b"new-one"
    );
    assert_eq!(
        fs::read(repo.path().join("cases/two/payload.txt")).expect("two"),
        b"new-two"
    );
    run_staged_cohort(repo.path(), &cases(), Mode::Apply).expect("retry");
}

#[derive(Clone, Copy)]
enum Failure {
    Write(usize),
    Marker,
    Rename(usize),
    RenameFrom(usize),
    Remove(usize),
    PartialRemove,
}

struct InjectedFs {
    failure: Failure,
    writes: Cell<usize>,
    renames: Cell<usize>,
    removes: Cell<usize>,
}
impl InjectedFs {
    fn new(failure: Failure) -> Self {
        Self {
            failure,
            writes: Cell::new(0),
            renames: Cell::new(0),
            removes: Cell::new(0),
        }
    }
}
impl MigrationFs for InjectedFs {
    fn create_dir(&self, path: &Path) -> Result<()> {
        RealFs.create_dir(path)
    }
    fn create_dir_all(&self, path: &Path) -> Result<()> {
        RealFs.create_dir_all(path)
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let n = self.writes.get() + 1;
        self.writes.set(n);
        if matches!(self.failure, Failure::Marker)
            && path.file_name().is_some_and(|v| v == "committed")
        {
            bail!("injected marker failure");
        }
        if matches!(self.failure, Failure::Write(at) if at == n) {
            bail!("injected write failure");
        }
        RealFs.write(path, bytes)
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let n = self.renames.get() + 1;
        self.renames.set(n);
        if matches!(self.failure, Failure::Rename(at) if at == n)
            || matches!(self.failure, Failure::RenameFrom(at) if n >= at)
        {
            bail!("injected rename failure {n}");
        }
        RealFs.rename(from, to)
    }
    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let n = self.removes.get() + 1;
        self.removes.set(n);
        if matches!(self.failure, Failure::Remove(at) if at == n) {
            bail!("injected remove failure");
        }
        if matches!(self.failure, Failure::PartialRemove) && n == 1 {
            let victim = fs::read_dir(path)?
                .next()
                .expect("backup")
                .expect("entry")
                .path();
            if victim.is_dir() {
                fs::remove_dir_all(victim)?;
            } else {
                fs::remove_file(victim)?;
            }
            bail!("injected partial remove failure");
        }
        RealFs.remove_dir_all(path)
    }
    fn remove_file(&self, path: &Path) -> Result<()> {
        RealFs.remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> Result<()> {
        RealFs.remove_dir(path)
    }
}

#[test]
fn staging_write_failure_mutates_no_authority() {
    let repo = repository();
    run_staged_cohort_with_fs(
        repo.path(),
        &cases(),
        Mode::Apply,
        &InjectedFs::new(Failure::Write(3)),
    )
    .expect_err("write");
    assert!(repo.path().join("old/one/old.txt").is_file());
}

#[test]
fn first_and_later_install_failures_restore_complete_cohort() {
    for at in [3, 4] {
        let repo = repository();
        let error = run_staged_cohort_with_fs(
            repo.path(),
            &cases(),
            Mode::Apply,
            &InjectedFs::new(Failure::Rename(at)),
        )
        .expect_err("rename");
        assert!(format!("{error:#}").contains("every authority was restored"));
        assert!(repo.path().join("old/one/old.txt").is_file());
        assert!(repo.path().join("old/two/old.txt").is_file());
        assert!(!repo.path().join("cases/one").exists());
    }
}

#[test]
fn commit_marker_failure_rolls_back_complete_cohort() {
    let repo = repository();
    let error = run_staged_cohort_with_fs(
        repo.path(),
        &cases(),
        Mode::Apply,
        &InjectedFs::new(Failure::Marker),
    )
    .expect_err("marker");
    assert!(format!("{error:#}").contains("every authority was restored"));
    assert!(repo.path().join("old/one/old.txt").is_file());
}

#[test]
fn dual_install_restore_failure_retains_named_backups() {
    let repo = repository();
    let error = run_staged_cohort_with_fs(
        repo.path(),
        &cases(),
        Mode::Apply,
        &InjectedFs::new(Failure::RenameFrom(4)),
    )
    .expect_err("dual");
    let message = format!("{error:#}");
    assert!(message.contains("rollback failures"));
    assert!(message.contains("recoverable transaction retained"));
}

#[test]
fn zero_progress_gc_and_same_plan_retry_keep_new_cohort() {
    let repo = repository();
    let error = run_staged_cohort_with_fs(
        repo.path(),
        &cases(),
        Mode::Apply,
        &InjectedFs::new(Failure::Remove(1)),
    )
    .expect_err("gc");
    assert!(format!("{error:#}").contains("committed=true"));
    assert!(repo.path().join("cases/two/payload.txt").is_file());
    run_staged_cohort(repo.path(), &cases(), Mode::Apply).expect("retry");
}

#[test]
fn genuine_partial_gc_and_same_plan_retry_keep_new_cohort() {
    let repo = repository();
    let error = run_staged_cohort_with_fs(
        repo.path(),
        &cases(),
        Mode::Apply,
        &InjectedFs::new(Failure::PartialRemove),
    )
    .expect_err("gc");
    assert!(format!("{error:#}").contains("committed=true"));
    assert!(repo.path().join("cases/one/payload.txt").is_file());
    run_staged_cohort(repo.path(), &cases(), Mode::Apply).expect("retry");
}

#[test]
fn unknown_and_mismatched_roots_are_refused() {
    for marker in [
        None,
        Some("umber-fixture-layout-transaction-v1\nplan-sha256=wrong\n"),
    ] {
        let repo = repository();
        let root = repo.path().join(".fixture-layout-transaction-foreign");
        fs::create_dir(&root).expect("root");
        if let Some(marker) = marker {
            fs::write(root.join("owner"), marker).expect("owner");
        }
        let error = run_staged_cohort(repo.path(), &cases(), Mode::Apply).expect_err("refused");
        assert!(format!("{error:#}").contains(if marker.is_some() {
            "mismatched"
        } else {
            "unknown"
        }));
        assert!(root.is_dir());
    }
}

#[test]
fn staged_validator_accepts_closed_untracked_case_without_git_authority() {
    let repo = repository();
    assert_eq!(
        validate_staged_case(&repo.path().join("stage/one"))
            .expect("closed")
            .len(),
        2
    );
}
