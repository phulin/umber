#[test]
fn scanner_helpers_cannot_open_input() {
    use std::path::Path;

    use test_support::{CompileFailDependency, assert_compile_fail};

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [CompileFailDependency {
        name: "tex-state",
        path: &tex_state_dir,
    }];

    assert_compile_fail(
        "scanner-helper-input-open-forbidden",
        &manifest_dir.join("tests/ui/scanner_helper_input_open_forbidden.rs"),
        &dependencies,
        &[
            "E0599",
            "no method named `input_open_context`",
            "ExpansionState + InputOpenState",
        ],
    );
}
