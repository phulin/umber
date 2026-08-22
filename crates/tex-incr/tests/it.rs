use test_support::{CompileFailDependency, assert_compile_fail};

fn assert_generation_boundary(test_name: &str, expected: &[&str]) {
    let manifest_dir = test_support::repository_root().join("crates/tex-incr");
    let tex_exec_dir = manifest_dir.join("../tex-exec");
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [
        CompileFailDependency::path("tex-incr", &manifest_dir),
        CompileFailDependency::path("tex-exec", &tex_exec_dir),
        CompileFailDependency::path("tex-state", &tex_state_dir),
    ];
    assert_compile_fail(
        test_name,
        &manifest_dir.join(format!("tests/ui/{test_name}.rs")),
        &dependencies,
        expected,
    );
}

#[test]
fn prior_and_current_generation_ids_cannot_cross() {
    assert_generation_boundary(
        "prior_current_id_crossing_forbidden",
        &[
            "expected `TokenListId<Current>`",
            "found `TokenListId<Prior>`",
        ],
    );
}

#[test]
fn admitted_generation_brand_cannot_escape() {
    assert_generation_boundary(
        "admitted_generation_escape_forbidden",
        &["expected `TokenListId<()>`, found `TokenListId<G>`"],
    );
}

#[test]
fn detached_history_cannot_store_a_generation_owner() {
    assert_generation_boundary(
        "history_generation_owner_forbidden",
        &["struct `BoundaryRecord` has no field named `owner`"],
    );
}
