use std::cell::Cell;
use std::path::Path;

use anyhow::{Result, bail};

use super::{
    CaseFile, CaseOwnedFile, FamilySpec, FileRole, MigrationFs, Mode, RealFs, SharedFile, run,
    run_with_fs,
};

const FILES: &[CaseFile] = &[
    CaseFile {
        source_suffix: ".src",
        destination_suffix: "program.input",
        destination_keeps_case: false,
        captures_tail: false,
        role: FileRole::Source,
        required: true,
    },
    CaseFile {
        source_suffix: ".oracle-",
        destination_suffix: "answer-",
        destination_keeps_case: false,
        captures_tail: true,
        role: FileRole::Output,
        required: false,
    },
];
const OWNED: &[CaseOwnedFile] = &[CaseOwnedFile {
    case: "second",
    source: "odd metadata",
    destination: "meta/info",
    role: FileRole::Metadata,
}];
const SHARED: &[SharedFile] = &[SharedFile {
    source: "common.data",
    destination: "inputs/common",
    role: FileRole::Input,
}];
const SPEC: &[FamilySpec] = &[FamilySpec {
    area: "sample",
    case_discovery_suffix: ".src",
    case_files: FILES,
    case_owned_files: OWNED,
    shared_files: SHARED,
}];

fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("temp");
    let area = temp.path().join("sample");
    std::fs::create_dir(&area).expect("area");
    std::fs::write(area.join("first.src"), b"first").expect("first");
    std::fs::write(area.join("first.oracle-log"), b"log").expect("output");
    std::fs::write(area.join("second.src"), b"second").expect("second");
    std::fs::write(area.join("odd metadata"), b"meta").expect("metadata");
    std::fs::write(area.join("common.data"), b"common").expect("shared");
    temp
}

#[test]
fn declarative_non_tex_nonconventional_names_apply_and_repeat() {
    let temp = fixture();
    let plan = run(temp.path(), SPEC, Mode::Plan).expect("plan");
    let applied = run(temp.path(), SPEC, Mode::Apply).expect("apply");
    assert_eq!(plan, applied);
    assert_eq!(
        std::fs::read(temp.path().join("sample/first/program.input")).expect("source"),
        b"first"
    );
    assert_eq!(
        std::fs::read(temp.path().join("sample/first/answer-log")).expect("output"),
        b"log"
    );
    assert_eq!(
        std::fs::read(temp.path().join("sample/second/meta/info")).expect("metadata"),
        b"meta"
    );
    assert_eq!(run(temp.path(), SPEC, Mode::Apply).expect("retry"), applied);
}

#[derive(Clone, Copy)]
enum Failure {
    Write(usize),
    Rename(usize),
    RenameFrom(usize),
    Remove(usize),
    RemoveFrom(usize),
}

struct InjectedFs {
    failure: Failure,
    writes: Cell<usize>,
    renames: Cell<usize>,
    removals: Cell<usize>,
}

impl InjectedFs {
    fn new(failure: Failure) -> Self {
        Self {
            failure,
            writes: Cell::new(0),
            renames: Cell::new(0),
            removals: Cell::new(0),
        }
    }

    fn fail(counter: &Cell<usize>, wanted: usize) -> Result<()> {
        let current = counter.get() + 1;
        counter.set(current);
        if current == wanted {
            bail!("injected failure at operation {current}");
        }
        Ok(())
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
        if let Failure::Write(at) = self.failure {
            Self::fail(&self.writes, at)?;
        }
        RealFs.write(path, bytes)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let current = self.renames.get() + 1;
        self.renames.set(current);
        match self.failure {
            Failure::Rename(at) if current == at => {
                bail!("injected rename failure at operation {current}")
            }
            Failure::RenameFrom(at) if current >= at => {
                bail!("injected persistent rename failure at operation {current}")
            }
            _ => RealFs.rename(from, to),
        }
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let current = self.removals.get() + 1;
        self.removals.set(current);
        match self.failure {
            Failure::Remove(at) if current == at => {
                bail!("injected remove failure at operation {current}")
            }
            Failure::RemoveFrom(at) if current >= at => {
                bail!("injected persistent remove failure at operation {current}")
            }
            _ => {}
        }
        RealFs.remove_dir_all(path)
    }
}

#[test]
fn staging_failure_precedes_every_authority_mutation_and_retry_succeeds() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::Write(2)),
    )
    .expect_err("failure");
    assert!(format!("{error:#}").contains("injected failure"));
    assert_eq!(
        std::fs::read(temp.path().join("sample/first.src")).expect("authority"),
        b"first"
    );
    assert!(!temp.path().join("sample/first").exists());
    run(temp.path(), SPEC, Mode::Apply).expect("retry");
}

#[test]
fn authority_commit_failure_rolls_back_and_retry_succeeds() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::Rename(3)),
    )
    .expect_err("failure");
    assert!(format!("{error:#}").contains("every authority was restored"));
    assert!(temp.path().join("sample/first.src").is_file());
    assert!(temp.path().join("sample/second.src").is_file());
    run(temp.path(), SPEC, Mode::Apply).expect("retry");
}

#[test]
fn install_failure_after_prior_swap_rolls_back_everything() {
    let temp = fixture();
    // Five unique authorities are backed up, then first is installed, then failure.
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::Rename(7)),
    )
    .expect_err("failure");
    assert!(format!("{error:#}").contains("every authority was restored"));
    assert!(!temp.path().join("sample/first").exists());
    assert!(temp.path().join("sample/first.src").is_file());
    assert!(temp.path().join("sample/common.data").is_file());
}

#[test]
fn restoration_failure_reports_both_errors_and_preserves_backups() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::RenameFrom(7)),
    )
    .expect_err("dual failure");
    let message = format!("{error:#}");
    assert!(message.contains("migration commit failed"));
    assert!(message.contains("rollback failures"));
    let transaction = std::fs::read_dir(temp.path())
        .expect("corpus")
        .map(|entry| entry.expect("entry").path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".fixture-layout-transaction-"))
        })
        .expect("retained transaction");
    assert!(transaction.join("backup/sample/first.src").is_file());
    assert!(temp.path().join("sample/first").is_dir());
}

#[test]
fn post_commit_cleanup_failure_restores_authorities_and_retry_succeeds() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::Remove(1)),
    )
    .expect_err("cleanup failure");
    let message = format!("{error:#}");
    assert!(message.contains("cleanup failed after swaps committed"));
    assert!(message.contains("every authority was restored"));
    assert!(temp.path().join("sample/first.src").is_file());
    assert!(!temp.path().join("sample/first").exists());
    run(temp.path(), SPEC, Mode::Apply).expect("retry");
}

#[test]
fn rollback_cleanup_failure_reports_retained_unique_root_and_retry_succeeds() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::RemoveFrom(1)),
    )
    .expect_err("cleanup failure");
    let message = format!("{error:#}");
    assert!(message.contains("recoverable transaction retained at"));
    assert!(message.contains("remove restored transaction root"));
    assert!(temp.path().join("sample/first.src").is_file());
    let retained = std::fs::read_dir(temp.path())
        .expect("corpus")
        .map(|entry| entry.expect("entry").path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".fixture-layout-transaction-"))
        })
        .expect("retained unique root");
    assert!(retained.is_dir());
    run(temp.path(), SPEC, Mode::Apply).expect("retry beside retained root");
}
