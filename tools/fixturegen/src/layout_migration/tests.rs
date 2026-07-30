use std::cell::Cell;
use std::path::Path;

use anyhow::{Result, bail};

use super::{
    CaseFile, CaseOwnedFile, FamilySpec, FileRole, MigrationFs, Mode, RealFs, SelectedSharedFile,
    SharedFile, run, run_with_fs,
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
const SELECTED_SHARED: &[SelectedSharedFile] = &[SelectedSharedFile {
    cases: &["first"],
    source: "first-only.data",
    destination: "inputs/first-only",
    role: FileRole::Input,
}];
const SPEC: &[FamilySpec] = &[FamilySpec {
    area: "sample",
    case_discovery_suffix: ".src",
    case_files: FILES,
    case_owned_files: OWNED,
    shared_files: SHARED,
    selected_shared_files: SELECTED_SHARED,
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
    std::fs::write(area.join("first-only.data"), b"first only").expect("selected shared");
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
    assert_eq!(
        std::fs::read(temp.path().join("sample/first/inputs/first-only")).expect("selected shared"),
        b"first only"
    );
    assert!(!temp.path().join("sample/second/inputs/first-only").exists());
    assert_eq!(run(temp.path(), SPEC, Mode::Apply).expect("retry"), applied);
}

#[test]
fn declarative_family_area_may_be_a_normalized_nested_path() {
    const NESTED: &[FamilySpec] = &[FamilySpec {
        area: "group/sample",
        case_discovery_suffix: ".src",
        case_files: FILES,
        case_owned_files: &[],
        shared_files: &[],
        selected_shared_files: &[],
    }];
    let temp = tempfile::tempdir().expect("temp");
    let area = temp.path().join("group/sample");
    std::fs::create_dir_all(&area).expect("nested area");
    std::fs::write(area.join("only.src"), b"nested").expect("source");

    run(temp.path(), NESTED, Mode::Apply).expect("nested migration");
    assert_eq!(
        std::fs::read(area.join("only/program.input")).expect("migrated source"),
        b"nested"
    );
}

#[derive(Clone, Copy)]
enum Failure {
    Write(usize),
    WriteCommittedMarker,
    Rename(usize),
    RenameFrom(usize),
    Remove(usize),
    PartialRemove,
    RenameAndRemove(usize),
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
        if matches!(self.failure, Failure::WriteCommittedMarker)
            && path.file_name().is_some_and(|name| name == "committed")
        {
            bail!("injected committed-marker write failure");
        }
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
            Failure::RenameAndRemove(at) if current == at => {
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
            Failure::RenameAndRemove(_) => {
                bail!("injected persistent remove failure at operation {current}")
            }
            Failure::PartialRemove if current == 1 => {
                let removed = path.join("sample/first.src");
                std::fs::remove_file(&removed).expect("partially remove backed-up authority");
                std::fs::remove_dir(path.join("sample"))
                    .expect_err("other backups keep directory nonempty");
                bail!(
                    "injected partial remove failure after deleting {}",
                    removed.display()
                )
            }
            _ => {}
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
fn staging_failure_precedes_every_authority_mutation_and_retry_succeeds() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::Write(3)),
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
    // Six unique authorities are backed up, then first is installed, then failure.
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::Rename(8)),
    )
    .expect_err("failure");
    assert!(format!("{error:#}").contains("every authority was restored"));
    assert!(!temp.path().join("sample/first").exists());
    assert!(temp.path().join("sample/first.src").is_file());
    assert!(temp.path().join("sample/common.data").is_file());
}

#[test]
fn committed_marker_write_failure_is_pre_commit_and_rolls_back_every_authority() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::WriteCommittedMarker),
    )
    .expect_err("committed-marker write failure");
    let message = format!("{error:#}");
    assert!(message.contains("injected committed-marker write failure"));
    assert!(message.contains("every authority was restored"));

    assert_eq!(
        std::fs::read(temp.path().join("sample/first.src")).expect("first source"),
        b"first"
    );
    assert_eq!(
        std::fs::read(temp.path().join("sample/first.oracle-log")).expect("first output"),
        b"log"
    );
    assert_eq!(
        std::fs::read(temp.path().join("sample/second.src")).expect("second source"),
        b"second"
    );
    assert_eq!(
        std::fs::read(temp.path().join("sample/odd metadata")).expect("metadata"),
        b"meta"
    );
    assert_eq!(
        std::fs::read(temp.path().join("sample/common.data")).expect("shared input"),
        b"common"
    );
    assert!(!temp.path().join("sample/first").exists());
    assert!(!temp.path().join("sample/second").exists());
    assert!(transaction_roots(temp.path()).is_empty());

    run(temp.path(), SPEC, Mode::Apply).expect("clean retry");
    assert!(temp.path().join("sample/first/program.input").is_file());
    assert!(temp.path().join("sample/second/program.input").is_file());
}

#[test]
fn restoration_failure_reports_both_errors_and_preserves_backups() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::RenameFrom(8)),
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
fn zero_progress_post_commit_gc_failure_keeps_new_authority_and_retry_succeeds() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::Remove(1)),
    )
    .expect_err("cleanup failure");
    let message = format!("{error:#}");
    assert!(message.contains("fixture layout committed and revalidated"));
    assert!(message.contains("garbage collection failed"));
    assert!(message.contains("committed=true"));
    assert!(message.contains("retained owned transaction="));
    assert!(!temp.path().join("sample/first.src").exists());
    assert!(temp.path().join("sample/first").is_dir());
    let retained = transaction_roots(temp.path());
    assert_eq!(retained.len(), 1);
    assert!(
        retained[0].join("committed").is_file(),
        "a successful marker write must cross into GC-only recovery"
    );
    run(temp.path(), SPEC, Mode::Apply).expect("retry");
    assert!(transaction_roots(temp.path()).is_empty());
}

#[test]
fn rollback_cleanup_failure_reports_retained_unique_root_and_retry_succeeds() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::RenameAndRemove(8)),
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

#[test]
fn partial_post_commit_gc_failure_never_rolls_back_and_matching_retry_finishes() {
    let temp = fixture();
    let error = run_with_fs(
        temp.path(),
        SPEC,
        Mode::Apply,
        &InjectedFs::new(Failure::PartialRemove),
    )
    .expect_err("partial gc failure");
    let message = format!("{error:#}");
    assert!(message.contains("committed=true"));
    assert!(message.contains("retained owned transaction="));
    assert!(temp.path().join("sample/first/program.input").is_file());
    assert!(temp.path().join("sample/second/program.input").is_file());
    assert!(!temp.path().join("sample/first.src").exists());
    let retained = transaction_roots(temp.path());
    assert_eq!(retained.len(), 1);
    assert!(!retained[0].join("backup/sample/first.src").exists());
    assert!(retained[0].join("owner").is_file());
    assert!(retained[0].join("committed").is_file());

    run(temp.path(), SPEC, Mode::Apply).expect("matching committed retry");
    assert!(transaction_roots(temp.path()).is_empty());
    assert!(temp.path().join("sample/first/program.input").is_file());
}

#[test]
fn unknown_retained_root_is_refused_and_preserved() {
    let temp = fixture();
    let root = temp.path().join(".fixture-layout-transaction-user-data");
    std::fs::create_dir(&root).expect("unknown root");
    std::fs::write(root.join("notes"), b"user").expect("unknown data");
    let error = run(temp.path(), SPEC, Mode::Apply).expect_err("unknown root");
    assert!(format!("{error:#}").contains("refusing unknown transaction root"));
    assert_eq!(
        std::fs::read(root.join("notes")).expect("preserved"),
        b"user"
    );
    assert!(temp.path().join("sample/first.src").is_file());
}

#[test]
fn mismatched_retained_root_is_refused_and_preserved() {
    let temp = fixture();
    let root = temp.path().join(".fixture-layout-transaction-other-plan");
    std::fs::create_dir(&root).expect("mismatched root");
    std::fs::write(
        root.join("owner"),
        b"umber-fixture-layout-transaction-v1\nplan-sha256=wrong\n",
    )
    .expect("owner");
    let error = run(temp.path(), SPEC, Mode::Apply).expect_err("mismatched root");
    assert!(format!("{error:#}").contains("refusing mismatched transaction root"));
    assert!(root.join("owner").is_file());
    assert!(temp.path().join("sample/first.src").is_file());
}

fn transaction_roots(corpus: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(corpus)
        .expect("corpus")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".fixture-layout-transaction-"))
        })
        .collect()
}
