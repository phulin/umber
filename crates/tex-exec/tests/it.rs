use std::path::Path;

use test_support::{CompileFailDependency, assert_compile_fail};

#[test]
fn engine_checkpoint_cannot_be_forged_by_callers() {
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
        "engine-checkpoint-forgery-forbidden",
        &manifest_dir.join("tests/ui/engine_checkpoint_forgery_forbidden.rs"),
        &dependencies,
        &["cannot construct `EngineCheckpoint`", "private fields"],
    );
}
