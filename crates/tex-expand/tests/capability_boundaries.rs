#[test]
fn scanner_helpers_cannot_open_input() {
    use test_support::{CompileFailDependency, assert_compile_fail};

    let manifest_dir = test_support::repository_root().join("crates/tex-expand");
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
    use test_support::{CompileFailDependency, assert_compile_fail};

    let manifest_dir = test_support::repository_root().join("crates/tex-expand");
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

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn macro_arguments_stay_on_the_state_owner() {
    let root = test_support::repository_root();
    let expand = root.join("crates/tex-expand/src");
    for entry in std::fs::read_dir(expand).expect("read expansion source") {
        let path = entry.expect("read source entry").path();
        if path.extension().is_some_and(|extension| extension == "rs")
            && path.file_name().is_none_or(|name| name != "tests.rs")
        {
            let source = std::fs::read_to_string(&path).expect("read expansion source file");
            assert!(
                !source.contains("tex_lex::MacroArguments")
                    && !source.contains("use tex_lex::{InputStack, MacroArguments}")
                    && !source.contains("tex_lex::MacroReplaySite")
                    && !source.contains("tex_lex::TracedExpansionToken")
                    && !source.contains("MacroReplaySite, TokenListReplayKind")
                    && !source.contains("TokenListReplayKind, TracedExpansionToken"),
                "{} must consume immutable replay payloads from tex-state",
                path.display()
            );
        }
    }
    let lexer =
        std::fs::read_to_string(root.join("crates/tex-lex/src/lib.rs")).expect("read lexer source");
    assert!(!lexer.contains("pub struct MacroArguments"));
    assert!(!lexer.contains("pub struct MacroReplaySite"));
    assert!(!lexer.contains("pub struct TracedExpansionToken"));
}
