use test_support::{CompileFailDependency, assert_compile_fail};

fn assert_live_boundary(test_name: &str, expected_stderr: &[&str]) {
    let manifest_dir = test_support::repository_root().join("crates/tex-state");
    let dependencies = [CompileFailDependency::path("tex-state", &manifest_dir)];
    assert_compile_fail(
        test_name,
        &manifest_dir.join(format!("tests/ui/{test_name}.rs")),
        &dependencies,
        expected_stderr,
    );
}

#[test]
fn downstream_crate_cannot_import_private_stores() {
    assert_live_boundary("stores-boundary-forbidden", &["module `stores` is private"]);
}

#[test]
fn downstream_crate_cannot_construct_or_mutate_raw_env() {
    assert_live_boundary(
        "env-boundary-forbidden",
        &["struct `DenseState` is private"],
    );
}

#[test]
fn downstream_crate_cannot_construct_or_mutate_raw_interner_or_code_tables() {
    assert_live_boundary(
        "raw-table-boundary-forbidden",
        &["E0624", "associated function `from_packed_slot` is private"],
    );
}

#[test]
fn downstream_crate_cannot_construct_or_mutate_raw_content_stores() {
    assert_live_boundary(
        "content-store-boundary-forbidden",
        &[
            "module `definition_arena` is private",
            "module `durable_arena` is private",
        ],
    );
}

#[test]
fn downstream_crate_cannot_construct_or_mutate_raw_source_map() {
    assert_live_boundary(
        "source-map-boundary-forbidden",
        &["struct `SourceMap` is private"],
    );
}

#[test]
fn downstream_crate_cannot_construct_raw_origin_or_traced_words() {
    assert_live_boundary(
        "token-boundary-forbidden",
        &["OriginId::from_raw", "TracedTokenWord::from_raw", "raw"],
    );
}

#[test]
fn downstream_crate_cannot_commit_world_effects_without_universe_boundary() {
    assert_live_boundary(
        "world-boundary-forbidden",
        &[
            "E0624",
            "method `commit_effects` is private",
            "no method named `record_deferred_write` found",
            "no method named `rollback_generation_fork` found",
        ],
    );
}

#[test]
fn downstream_crate_cannot_bypass_universe_facade_through_raw_env_ref() {
    assert_live_boundary(
        "universe-env-boundary-forbidden",
        &[
            "method `live_state` is private",
            "method `admitted` is private",
        ],
    );
}
