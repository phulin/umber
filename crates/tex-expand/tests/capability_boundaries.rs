#[test]
fn scanner_helpers_cannot_open_input() {
    use std::path::Path;

    use test_support::{CompileFailDependency, assert_compile_fail};

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [CompileFailDependency::path("tex-state", &tex_state_dir)];

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

#[test]
fn lexer_input_stack_cannot_resolve_meanings() {
    use std::path::Path;

    use test_support::{CompileFailDependency, assert_compile_fail};

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tex_lex_dir = manifest_dir.join("../tex-lex");
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [
        CompileFailDependency::path("tex-lex", &tex_lex_dir),
        CompileFailDependency::path("tex-state", &tex_state_dir),
    ];

    assert_compile_fail(
        "lexer-meaning-resolution-forbidden",
        &manifest_dir.join("tests/ui/lexer_meaning_resolution_forbidden.rs"),
        &dependencies,
        &[
            "E0599",
            "no method named `resolve_expansion_meaning`",
            "InputStack",
        ],
    );
}
