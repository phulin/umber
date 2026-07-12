#[test]
fn expansion_capability_rejects_privileged_apis() {
    use std::path::Path;

    use test_support::{CompileFailDependency, assert_compile_fail};

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [CompileFailDependency {
        name: "tex-state",
        path: manifest_dir,
    }];

    assert_compile_fail(
        "expansion-state-input-forbidden",
        &manifest_dir.join("tests/ui/expansion_state_input_forbidden.rs"),
        &dependencies,
        &[
            "E0599",
            "no method named `input_open_context`",
            "ExpansionState + InputOpenState",
        ],
    );
    assert_compile_fail(
        "expansion-context-forbidden",
        &manifest_dir.join("tests/ui/expansion_context_forbidden.rs"),
        &dependencies,
        &[
            "no method named `world_mut`",
            "no method named `snapshot`",
            "no method named `rollback`",
            "no method named `set_count`",
            "no method named `set_catcode`",
            "no method named `set_current_font`",
        ],
    );
    assert_compile_fail(
        "input-open-context-forbidden",
        &manifest_dir.join("tests/ui/input_open_context_forbidden.rs"),
        &dependencies,
        &[
            "no method named `world_mut`",
            "no method named `meaning`",
            "no method named `symbol`",
            "no method named `set_count`",
        ],
    );
    assert_compile_fail(
        "arena-transaction-exclusive",
        &manifest_dir.join("tests/ui/arena_transaction_exclusive.rs"),
        &dependencies,
        &[
            "E0499",
            "cannot borrow `universe` as mutable more than once at a time",
        ],
    );
}
