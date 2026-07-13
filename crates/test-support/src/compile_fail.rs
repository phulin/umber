use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Dependency exposed to a standalone compile-fail fixture crate.
#[derive(Clone, Copy, Debug)]
pub struct CompileFailDependency<'a> {
    name: &'a str,
    source: DependencySource<'a>,
}

#[derive(Clone, Copy, Debug)]
enum DependencySource<'a> {
    Path(&'a Path),
    Registry(&'a str),
}

impl<'a> CompileFailDependency<'a> {
    /// Creates a dependency on a local crate.
    #[must_use]
    pub const fn path(name: &'a str, path: &'a Path) -> Self {
        Self {
            name,
            source: DependencySource::Path(path),
        }
    }

    /// Creates a dependency from the configured Cargo registry.
    #[must_use]
    pub const fn registry(name: &'a str, version: &'a str) -> Self {
        Self {
            name,
            source: DependencySource::Registry(version),
        }
    }
}

/// Runs `cargo check` against a standalone fixture crate and asserts that it
/// fails with all expected stderr substrings.
///
/// Every fixture gets an independent temporary crate, while all fixture crates
/// share one workspace-local Cargo target directory. Dropping the temporary
/// directory removes the installed fixture source and manifest after the check;
/// Cargo retains only reusable dependency artifacts in the shared target.
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

    fs::copy(fixture, src_dir.join("main.rs")).unwrap_or_else(|error| {
        panic!(
            "failed to install compile-fail fixture {} for {test_name}: {error}",
            fixture.display()
        )
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
        .arg(shared_target_dir())
        .output()
        .unwrap_or_else(|error| panic!("failed to run compile-fail check: {error}"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "compile-fail fixture {test_name} unexpectedly compiled"
    );

    let missing: Vec<_> = expected_stderr
        .iter()
        .copied()
        .filter(|expected| !stderr.contains(expected))
        .collect();

    assert!(
        missing.is_empty(),
        "compile-fail fixture {test_name} missed expected stderr substrings:\n{}\n\nstderr:\n{stderr}",
        missing.join("\n")
    );
}

fn shared_target_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/compile-fail")
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
        match dependency.source {
            DependencySource::Path(path) => manifest.push_str(&format!(
                "{} = {{ path = \"{}\" }}\n",
                dependency.name,
                toml_string(&path.to_string_lossy())
            )),
            DependencySource::Registry(version) => manifest.push_str(&format!(
                "{} = \"{}\"\n",
                dependency.name,
                toml_string(version)
            )),
        }
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

fn toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
