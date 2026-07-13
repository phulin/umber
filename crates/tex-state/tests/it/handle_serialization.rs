use std::path::Path;

use test_support::{CompileFailDependency, assert_compile_fail};

#[test]
fn downstream_serde_cannot_mint_or_construct_live_handles() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [
        CompileFailDependency::path("tex-state", manifest_dir),
        CompileFailDependency::registry("serde", "1"),
    ];

    assert_compile_fail(
        "handle-serialization-forbidden",
        &manifest_dir.join("tests/ui/handle_serialization_forbidden.rs"),
        &dependencies,
        &[
            "TokenListId",
            "OriginListId",
            "MacroDefinitionId",
            "GlueId",
            "FontId",
            "SnapshotId",
            "SurvivorRootId",
            "ArenaRef",
            "NodeListId",
            "Node",
            "MathField",
            "Deserialize",
            "Serialize",
            "associated function `new` is private",
            "associated function `frozen_end_template` is private",
            "cannot initialize a tuple struct which contains private fields",
        ],
    );
}
