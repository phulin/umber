use std::{fs, path::Path};

use test_support::{CompileFailDependency, assert_compile_fail};

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn command_crate_has_no_executor_dependency() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .unwrap_or_else(|error| panic!("failed to read tex-command manifest: {error}"));

    assert!(
        !manifest.contains("tex-exec"),
        "tex-command must not depend on tex-exec"
    );
}

#[test]
fn command_state_machines_are_private() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [CompileFailDependency::path("tex-command", manifest_dir)];

    assert_compile_fail(
        "command-private-modules",
        &manifest_dir.join("tests/ui/private_modules.rs"),
        &dependencies,
        &[
            "E0603",
            "module `input` is private",
            "module `processor` is private",
        ],
    );
}

#[test]
fn host_context_cannot_be_serialized() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [
        CompileFailDependency::path("tex-command", manifest_dir),
        CompileFailDependency::registry("serde", "1"),
    ];

    assert_compile_fail(
        "command-host-serialization",
        &manifest_dir.join("tests/ui/host_serialization.rs"),
        &dependencies,
        &[
            "CommandHostContext",
            "Serialize",
            "DeserializeOwned",
            "Clone",
        ],
    );
}
