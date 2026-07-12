use std::path::Path;

use test_support::{CompileFailDependency, assert_compile_fail};

#[test]
fn hash_only_observation_cannot_be_passed_to_restore() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tex_lex_dir = manifest_dir.join("../tex-lex");
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [
        CompileFailDependency {
            name: "tex-exec",
            path: manifest_dir,
        },
        CompileFailDependency {
            name: "tex-lex",
            path: &tex_lex_dir,
        },
        CompileFailDependency {
            name: "tex-state",
            path: &tex_state_dir,
        },
    ];
    assert_compile_fail(
        "hash-only-engine-restore-forbidden",
        &manifest_dir.join("tests/ui/hash_only_engine_restore_forbidden.rs"),
        &dependencies,
        &[
            "E0308",
            "expected `&ResumeValidCheckpoint`",
            "found `&HashOnlyObservation`",
        ],
    );
}
