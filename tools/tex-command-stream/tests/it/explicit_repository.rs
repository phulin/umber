use std::process::Command;

#[test]
#[allow(clippy::disallowed_methods)] // CLI regression launches the built comparison binary.
fn explicit_repository_works_outside_a_git_checkout() {
    let repository = test_support::repository_root();
    let outside = tempfile::tempdir().expect("create non-Git working directory");
    let output = Command::new(env!("CARGO_BIN_EXE_tex-command-stream"))
        .args([
            "--repository",
            repository.to_str().expect("repository path is UTF-8"),
            "--max-divergences",
            "1",
        ])
        .current_dir(outside.path())
        .output()
        .expect("run tex-command-stream");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("VERDICT:"),
        "explicit repository should run comparison, stderr:\n{stderr}"
    );
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI regression launches the built comparison binary.
fn omitted_repository_reports_non_git_working_directory() {
    let outside = tempfile::tempdir().expect("create non-Git working directory");
    let output = Command::new(env!("CARGO_BIN_EXE_tex-command-stream"))
        .current_dir(outside.path())
        .output()
        .expect("run tex-command-stream");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("resolve repository root"), "{stderr}");
    assert!(
        stderr.contains(
            outside
                .path()
                .to_str()
                .expect("temporary directory path is UTF-8")
        ),
        "{stderr}"
    );
    assert!(!stderr.contains("panicked at"), "{stderr}");
}
