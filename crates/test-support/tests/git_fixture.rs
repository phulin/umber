use std::fs;
use std::path::Path;
use std::process::Command;

use test_support::git_fixture::ClosedCase;

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    git(temp.path(), &["init", "-q"]);
    let case = temp.path().join("tests/corpus/example/only");
    fs::create_dir_all(&case).expect("case directory");
    fs::write(case.join("case.inventory"), "closed-case-v1\ninput.txt\n").expect("inventory");
    fs::write(case.join("input.txt"), "exact bytes\n").expect("payload");
    git(temp.path(), &["add", "tests/corpus/example/only"]);
    temp
}

fn reject(temp: &tempfile::TempDir, needle: &str) {
    let error = ClosedCase::discover_at(temp.path(), "tests/corpus/example/only")
        .expect_err("invalid fixture accepted");
    assert!(
        format!("{error:#}").contains(needle),
        "expected {needle:?}, got {error:#}"
    );
}

#[test]
fn accepts_a_closed_tracked_case() {
    let temp = fixture();
    let case = ClosedCase::discover_at(temp.path(), "tests/corpus/example/only").expect("case");
    assert_eq!(case.read("input.txt").expect("payload"), b"exact bytes\n");
}

#[test]
fn rejects_missing_extra_duplicate_untracked_and_ignored_entries() {
    let temp = fixture();
    fs::remove_file(temp.path().join("tests/corpus/example/only/input.txt")).expect("remove");
    reject(&temp, "filesystem inventory mismatch");

    let temp = fixture();
    let case = temp.path().join("tests/corpus/example/only");
    fs::write(case.join("extra.txt"), "extra").expect("extra");
    git(temp.path(), &["add", "tests/corpus/example/only/extra.txt"]);
    reject(&temp, "Git inventory mismatch");

    let temp = fixture();
    fs::write(
        temp.path().join("tests/corpus/example/only/case.inventory"),
        "closed-case-v1\ninput.txt\ninput.txt\n",
    )
    .expect("duplicate");
    reject(&temp, "duplicate inventory entry");

    let temp = fixture();
    fs::write(
        temp.path().join("tests/corpus/example/only/untracked.txt"),
        "untracked",
    )
    .expect("untracked");
    reject(&temp, "filesystem inventory mismatch");

    let temp = fixture();
    fs::write(temp.path().join(".gitignore"), "*.ignored\n").expect("ignore");
    fs::write(
        temp.path().join("tests/corpus/example/only/extra.ignored"),
        "ignored",
    )
    .expect("ignored");
    reject(&temp, "filesystem inventory mismatch");
}

#[cfg(unix)]
#[test]
fn rejects_symlink_and_non_regular_entries() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let temp = fixture();
    let case = temp.path().join("tests/corpus/example/only");
    fs::remove_file(case.join("input.txt")).expect("remove");
    symlink("case.inventory", case.join("input.txt")).expect("symlink");
    git(temp.path(), &["add", "tests/corpus/example/only/input.txt"]);
    reject(&temp, "not a regular file");

    let temp = fixture();
    let case = temp.path().join("tests/corpus/example/only");
    fs::set_permissions(case.join("input.txt"), fs::Permissions::from_mode(0o755))
        .expect("executable regular file");
    git(temp.path(), &["add", "tests/corpus/example/only/input.txt"]);
    assert!(ClosedCase::discover_at(temp.path(), "tests/corpus/example/only").is_ok());
    fs::create_dir(case.join("directory")).expect("directory");
    reject(&temp, "not a regular file");
}

#[test]
fn rejects_out_of_directory_target_backed_and_other_checkout_authority() {
    let temp = fixture();
    for path in ["../only", "/tmp/only"] {
        let error = ClosedCase::discover_at(temp.path(), path).expect_err("escape accepted");
        assert!(format!("{error:#}").contains("repository-relative"));
    }
    let error = ClosedCase::discover_at(temp.path(), "target/case").expect_err("target accepted");
    assert!(format!("{error:#}").contains("target-backed"));

    let other = fixture();
    let error =
        ClosedCase::discover_at(temp.path(), other.path().join("tests/corpus/example/only"))
            .expect_err("other checkout accepted");
    assert!(format!("{error:#}").contains("repository-relative"));
}
