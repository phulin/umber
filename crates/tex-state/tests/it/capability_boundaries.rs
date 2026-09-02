#[test]
fn restricted_capabilities_reject_privileged_apis() {
    use test_support::{CompileFailDependency, assert_compile_fail};

    let manifest_dir = test_support::repository_root().join("crates/tex-state");
    let dependencies = [CompileFailDependency::path("tex-state", &manifest_dir)];

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
            "cannot borrow `*universe` as mutable more than once at a time",
        ],
    );
    assert_compile_fail(
        "fork-arena-pool-mutation-forbidden",
        &manifest_dir.join("tests/ui/fork_arena_pool_mutation_forbidden.rs"),
        &dependencies,
        &[
            "E0502",
            "cannot borrow `pool` as mutable because it is also borrowed as immutable",
        ],
    );
    assert_compile_fail(
        "fork-arena-builder-reuse-forbidden",
        &manifest_dir.join("tests/ui/fork_arena_builder_reuse_forbidden.rs"),
        &dependencies,
        &["E0382", "use of moved value: `builder`"],
    );
    assert_compile_fail(
        "fork-arena-single-publication-forbidden",
        &manifest_dir.join("tests/ui/fork_arena_single_publication_forbidden.rs"),
        &dependencies,
        &["E0382", "use of moved value: `unique`"],
    );
    assert_compile_fail(
        "durable-token-boundary-forbidden",
        &manifest_dir.join("tests/ui/durable_token_boundary_forbidden.rs"),
        &dependencies,
        &["mismatched types"],
    );
    assert_compile_fail(
        "durable-token-constructor-forbidden",
        &manifest_dir.join("tests/ui/durable_token_constructor_forbidden.rs"),
        &dependencies,
        &["fields `slot`, `serial` and `_brand` of struct `TokenListBuilder` are private"],
    );
    assert_compile_fail(
        "shipout-scratch-checkpoint-forbidden",
        &manifest_dir.join("tests/ui/shipout_scratch_checkpoint_forbidden.rs"),
        &dependencies,
        &["expected `PageListId`", "found `ShipoutScratchListId`"],
    );
}
