use std::{collections::BTreeSet, fs, path::Path};

use test_support::{CompileFailDependency, assert_compile_fail};

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn crate_dependency_direction_is_command_toward_state_only() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let command_manifest = fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .unwrap_or_else(|error| panic!("failed to read tex-command manifest: {error}"));
    let state_manifest = fs::read_to_string(manifest_dir.join("../tex-state/Cargo.toml"))
        .unwrap_or_else(|error| panic!("failed to read tex-state manifest: {error}"));

    let command_dependencies = dependency_names(&command_manifest);
    let state_dependencies = dependency_names(&state_manifest);
    assert!(
        command_dependencies.contains("tex-state"),
        "tex-command must depend on the aggregate state boundary"
    );
    for forbidden in ["tex-exec", "tex-expand", "tex-lex"] {
        assert!(
            !command_dependencies.contains(forbidden),
            "tex-command must not depend on {forbidden}"
        );
    }
    assert!(
        !state_dependencies.contains("tex-command"),
        "tex-state must remain unaware of command interpretation"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn hot_character_delivery_has_no_host_lookup_surface() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "src/state.rs",
        "src/input/source.rs",
        "src/input/lines.rs",
        "src/input/tokenizer.rs",
    ] {
        let source = fs::read_to_string(manifest_dir.join(relative))
            .unwrap_or_else(|error| panic!("failed to read {relative}: {error}"));
        for forbidden in [
            "std::fs",
            "std::net",
            "File::open",
            "CommandHostContext",
            "CommandHostCapabilities",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} must not acquire host resources through {forbidden}"
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn raw_delivery_keeps_one_profile_shared_input_path_and_semantic_free_levels() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let next = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery implementation");
    let levels = fs::read_to_string(manifest_dir.join("src/input/levels.rs"))
        .expect("read input-level representation");

    assert_eq!(
        next.matches("fn take_input_token(").count(),
        1,
        "the command core must have exactly one scalar input-level delivery loop"
    );
    assert_eq!(
        next.matches("fn next_source_step(").count(),
        1,
        "exact and Unicode profiles must select beneath the shared delivery loop"
    );
    assert!(next.contains("CharacterMode::EightBitExact"));
    assert!(next.contains("CharacterMode::UnicodeExtended"));
    assert!(next.contains("self.get_next_with_control_sequence_creation(true)"));
    assert!(next.contains("self.get_next_with_control_sequence_creation(false)"));
    assert!(
        !next.contains(".trace"),
        "diagnostic replay explanations must not select raw delivery semantics"
    );

    let level_definition = levels
        .split("pub(crate) enum InputLevel")
        .nth(1)
        .and_then(|tail| tail.split("/// One registered-source level").next())
        .expect("locate InputLevel definition");
    for forbidden in ["Condition", "Cache", "Scanner", "Expansion", "Paragraph"] {
        assert!(
            !level_definition.contains(forbidden),
            "input levels must not retain {forbidden} state"
        );
    }
    assert!(levels.contains("This value is diagnostic/provenance state."));
    assert!(levels.contains("cannot select expansion"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn outer_validity_and_runaway_recovery_have_one_raw_delivery_owner() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let next = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery implementation");

    assert_eq!(
        next.matches("fn check_outer_validity_entry(").count(),
        1,
        "outer-command detection must have one raw-delivery entry point"
    );
    assert_eq!(
        next.matches("fn recover_runaway_eof(").count(),
        1,
        "EOF legality must have one raw-delivery entry point"
    );
    assert_eq!(
        next.matches("fn install_outer_recovery(").count(),
        1,
        "outer commands and runaway EOF must share one recovery table"
    );
    assert_eq!(
        next.matches("self.install_outer_recovery(recovery)?;")
            .count(),
        2,
        "only outer-command and runaway-EOF entry points may install recovery"
    );
    assert!(next.contains("self.back_input(command.copy_for_backup())?;"));
    assert!(next.contains("self.command.scanner.clear_for_recovery();"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn scalar_macro_call_keeps_one_raw_fallback_matcher() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let matcher = fs::read_to_string(manifest_dir.join("src/macro_call.rs"))
        .expect("read scalar macro matcher implementation");

    assert_eq!(
        matcher.matches("fn macro_call_scalar(").count(),
        1,
        "macro calls must retain one scalar semantic fallback"
    );
    assert_eq!(
        matcher.matches("fn scan_undelimited_argument(").count(),
        1,
        "undelimited matching must have one canonical scanner"
    );
    assert_eq!(
        matcher.matches("fn scan_delimited_argument(").count(),
        1,
        "delimited matching must have one canonical scanner"
    );
    assert!(
        matcher.contains("self.get_token()?"),
        "the scalar matcher must consume raw tokens through get_token"
    );
    for forbidden in ["CompiledMacroMatcher", "MacroBytecode", "FastMacroMatcher"] {
        assert!(
            !matcher.contains(forbidden),
            "alternate macro matcher {forbidden} would bypass the scalar fallback"
        );
    }
}

fn dependency_names(manifest: &str) -> BTreeSet<&str> {
    let mut in_dependencies = false;
    let mut names = BTreeSet::new();

    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]";
            continue;
        }
        if in_dependencies
            && !line.is_empty()
            && !line.starts_with('#')
            && let Some((name, _)) = line.split_once('=')
        {
            names.insert(name.trim());
        }
    }

    names
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
            "module `conditionals` is private",
            "module `input` is private",
            "module `macro_call` is private",
            "module `primitives` is private",
            "module `processor` is private",
            "module `scan_toks` is private",
            "module `scanners` is private",
        ],
    );
}

#[test]
fn semantic_and_runtime_fields_are_opaque() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [CompileFailDependency::path("tex-command", manifest_dir)];

    assert_compile_fail(
        "command-opaque-state",
        &manifest_dir.join("tests/ui/opaque_state.rs"),
        &dependencies,
        &[
            "E0616",
            "field `input`",
            "field `state`",
            "field `meaning_cache`",
        ],
    );
}

#[test]
fn command_profile_and_installed_mode_are_immutable() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [CompileFailDependency::path("tex-command", manifest_dir)];

    assert_compile_fail(
        "command-immutable-profile",
        &manifest_dir.join("tests/ui/immutable_profile.rs"),
        &dependencies,
        &[
            "E0616",
            "field `dialect`",
            "field `characters`",
            "field `expansion`",
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
            "CommandHostCapabilities",
            "CommandHostContext",
            "Serialize",
            "DeserializeOwned",
            "Clone",
        ],
    );
}

#[test]
fn runtime_cannot_become_semantic_or_serialized_by_convenience() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [
        CompileFailDependency::path("tex-command", manifest_dir),
        CompileFailDependency::registry("serde", "1"),
    ];

    assert_compile_fail(
        "command-runtime-traits",
        &manifest_dir.join("tests/ui/runtime_traits.rs"),
        &dependencies,
        &[
            "CommandRuntime",
            "Clone",
            "DeserializeOwned",
            "Eq",
            "Hash",
            "Serialize",
        ],
    );
}

#[test]
fn ephemeral_command_types_cannot_be_serialized() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [
        CompileFailDependency::path("tex-command", manifest_dir),
        CompileFailDependency::registry("serde", "1"),
    ];

    assert_compile_fail(
        "command-ephemeral-serialization",
        &manifest_dir.join("tests/ui/ephemeral_serialization.rs"),
        &dependencies,
        &[
            "CurrentCommand",
            "CommandProcessor",
            "Serialize",
            "DeserializeOwned",
        ],
    );
}
