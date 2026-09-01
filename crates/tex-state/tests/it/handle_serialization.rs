use test_support::{CompileFailDependency, assert_compile_fail};

#[test]
fn downstream_serde_cannot_mint_or_construct_live_handles() {
    let manifest_dir = test_support::repository_root().join("crates/tex-state");
    let dependencies = [
        CompileFailDependency::path("tex-state", &manifest_dir),
        CompileFailDependency::registry("serde", "1"),
    ];

    assert_compile_fail(
        "handle-serialization-forbidden",
        &manifest_dir.join("tests/ui/handle_serialization_forbidden.rs"),
        &dependencies,
        &[
            "DefinitionRef",
            "TokenListId",
            "GlueId",
            "ProvenanceId",
            "PageListId",
            "Deserialize",
            "Serialize",
        ],
    );
}
