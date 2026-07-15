use std::path::Path;

use test_support::{CompileFailDependency, assert_compile_fail};

fn assert_live_boundary(test_name: &str, expected_stderr: &[&str]) {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [CompileFailDependency::path("tex-state", manifest_dir)];
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
    assert_live_boundary("env-boundary-forbidden", &["Env::new", "Env::default"]);
}

#[test]
fn downstream_crate_cannot_construct_or_mutate_raw_interner_or_code_tables() {
    assert_live_boundary(
        "raw-table-boundary-forbidden",
        &[
            "E0624",
            "Interner::new",
            "method `intern` is private",
            "CodeTables::new",
            "method `set_catcode` is private",
            "method `set_lccode` is private",
            "method `set_uccode` is private",
            "method `set_sfcode` is private",
            "method `set_mathcode` is private",
            "method `set_delcode` is private",
        ],
    );
}

#[test]
fn downstream_crate_cannot_construct_or_mutate_raw_content_stores() {
    assert_live_boundary(
        "content-store-boundary-forbidden",
        &[
            "E0624",
            "TokenStore::new",
            "TokenListBuilder::new",
            "GlueStore::new",
            "NodeArena::new",
            "NodeListBuilder::new",
            "SurvivorArena::new",
            "method `intern` is private",
            "method `finish` is private",
            "method `get` is private",
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
            "method `store_artifact` is private",
            "method `record_deferred_write` is private",
        ],
    );
}

#[test]
fn downstream_crate_cannot_bypass_universe_facade_through_raw_env_ref() {
    assert_live_boundary(
        "universe-env-boundary-forbidden",
        &["E0599", "no method named `env`"],
    );
}

#[test]
fn downstream_crate_cannot_install_fragments_without_the_paired_layout() {
    assert_live_boundary(
        "editor-fragment-install-boundary-forbidden",
        &["E0061", "argument #2 of type `&EditorLayout` is missing"],
    );
}
