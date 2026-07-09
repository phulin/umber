use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Path dependency exposed to a compile-fail fixture crate.
#[derive(Clone, Copy, Debug)]
pub struct CompileFailDependency<'a> {
    pub name: &'a str,
    pub path: &'a Path,
}

/// Runs `cargo check` against a standalone fixture crate and asserts that it
/// fails with all expected stderr substrings.
#[allow(clippy::disallowed_methods)] // host-side test harness code
pub fn assert_compile_fail(
    test_name: &str,
    fixture: &Path,
    dependencies: &[CompileFailDependency<'_>],
    expected_stderr: &[&str],
) {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| {
        panic!("failed to create compile-fail workspace for {test_name}: {error}")
    });
    let crate_dir = workspace.path().join(sanitize_package_name(test_name));
    let src_dir = crate_dir.join("src");

    fs::create_dir_all(&src_dir).unwrap_or_else(|error| {
        panic!(
            "failed to create compile-fail source directory {}: {error}",
            src_dir.display()
        )
    });

    let source = fs::read_to_string(fixture).unwrap_or_else(|error| {
        panic!(
            "failed to read compile-fail fixture {}: {error}",
            fixture.display()
        )
    });
    fs::write(src_dir.join("main.rs"), source).unwrap_or_else(|error| {
        panic!("failed to write compile-fail fixture for {test_name}: {error}")
    });
    fs::write(
        crate_dir.join("Cargo.toml"),
        manifest(test_name, dependencies),
    )
    .unwrap_or_else(|error| panic!("failed to write compile-fail manifest: {error}"));

    let output = Command::new(cargo_command())
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(crate_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(crate_dir.join("target"))
        .output()
        .unwrap_or_else(|error| panic!("failed to run compile-fail check: {error}"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "compile-fail fixture {test_name} unexpectedly compiled"
    );

    let mut missing = Vec::new();
    for expected in expected_stderr {
        if !stderr.contains(expected) {
            missing.push(*expected);
        }
    }

    assert!(
        missing.is_empty(),
        "compile-fail fixture {test_name} missed expected stderr substrings:\n{}\n\nstderr:\n{stderr}",
        missing.join("\n")
    );
}

fn cargo_command() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn manifest(test_name: &str, dependencies: &[CompileFailDependency<'_>]) -> String {
    let mut manifest = format!(
        r#"[package]
name = "{}"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
"#,
        sanitize_package_name(test_name)
    );

    for dependency in dependencies {
        manifest.push_str(&format!(
            "{} = {{ path = \"{}\" }}\n",
            dependency.name,
            toml_string(dependency.path)
        ));
    }

    manifest
}

fn sanitize_package_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            sanitized.push(ch.to_ascii_lowercase());
        } else {
            sanitized.push('-');
        }
    }

    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "compile-fail-probe".to_owned()
    } else {
        sanitized.to_owned()
    }
}

fn toml_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}
