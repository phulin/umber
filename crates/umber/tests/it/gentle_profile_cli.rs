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

#[cfg(feature = "profiling")]
#[test]
#[allow(clippy::disallowed_methods)] // CLI regression launches the built profiling binary.
fn direct_profile_preserves_gentle_output_and_provenance_stats() {
    let repository = test_support::repository_root();
    let output = Command::new(env!("CARGO_BIN_EXE_gentle-profile"))
        .args([
            "--repo-root",
            repository.to_str().expect("repository path is UTF-8"),
            "--iterations",
            "1",
            "--warmups",
            "1",
        ])
        .output()
        .expect("run canonical gentle profile");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let stdout = String::from_utf8(output.stdout).expect("profile output is UTF-8");
    assert!(stdout.contains("97 pages, 263424 DVI bytes, 0 checkpoints"));
    assert!(stdout.contains(
        "gentle-profile expansion: token_frame_steps=236260 provenance_resolutions=98089 \
character_tokens=164153 character_fraction=0.694798 meaning_lookups=236260 \
meaning_cache_hits=0 meaning_cache_misses=0 literal_spans=23116 literal_tokens=164153 \
mean_literal_run=7.101272 segmentation_cache_hits=0 segmentation_cache_misses=0 \
builder_appends=0 source_text_span_attempts=236260 source_text_spans=5775 \
source_text_tokens=84306 mean_source_text_run=14.598442"
    ));
    assert!(!stdout.contains("gentle-profile meaning invalidations:"));
}
