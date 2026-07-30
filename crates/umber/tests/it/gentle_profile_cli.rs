use std::process::Command;

#[test]
#[allow(clippy::disallowed_methods)] // CLI regression launches the built profiling binary.
fn explicit_repo_root_is_parsed_outside_a_git_checkout() {
    let repository = test_support::repository_root();
    let outside = tempfile::tempdir().expect("create non-Git working directory");
    let output = Command::new(env!("CARGO_BIN_EXE_gentle-profile"))
        .args([
            "--repo-root",
            repository.to_str().expect("repository path is UTF-8"),
            "--help",
        ])
        .current_dir(outside.path())
        .output()
        .expect("run gentle-profile");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI regression launches the built profiling binary.
fn omitted_repo_root_reports_non_git_working_directory() {
    let outside = tempfile::tempdir().expect("create non-Git working directory");
    let output = Command::new(env!("CARGO_BIN_EXE_gentle-profile"))
        .args(["--iterations", "1", "--warmups", "1"])
        .current_dir(outside.path())
        .output()
        .expect("run gentle-profile");
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
