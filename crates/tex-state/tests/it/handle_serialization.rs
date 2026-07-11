use std::fs;
use std::process::Command;

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_serde_cannot_mint_or_construct_live_handles() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create handle serde probe workspace");
    let probe_dir = probe_workspace.path().join("handle-serde-probe");
    let src_dir = probe_dir.join("src");
    fs::create_dir_all(&src_dir).expect("create handle serde probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "handle-serde-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
bincode = "1"
serde = "1"
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write handle serde probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use serde::de::DeserializeOwned;
use serde::Serialize;
use tex_state::ids::{
    ArenaRef, FontId, GlueId, MacroDefinitionId, NodeListId, OriginListId, SnapshotId,
    SurvivorRootId, TokenListId,
};
use tex_state::math::MathField;
use tex_state::node::Node;

fn require_deserialize<T: DeserializeOwned>() {}
fn require_serialize<T: Serialize>() {}

fn main() {
    require_deserialize::<TokenListId>();
    require_deserialize::<OriginListId>();
    require_deserialize::<MacroDefinitionId>();
    require_deserialize::<GlueId>();
    require_deserialize::<FontId>();
    require_deserialize::<SnapshotId>();
    require_deserialize::<SurvivorRootId>();
    require_deserialize::<ArenaRef>();
    require_deserialize::<NodeListId>();
    require_deserialize::<Node>();
    require_deserialize::<MathField>();

    require_serialize::<TokenListId>();
    require_serialize::<NodeListId>();
    require_serialize::<Node>();

    let _ = TokenListId::new(1);
    let _ = SurvivorRootId::new(1);
}
"#,
    )
    .expect("write handle serde probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run handle serde probe");

    assert!(
        !output.status.success(),
        "downstream handle serde probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for handle in [
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
    ] {
        assert!(
            stderr.contains(handle),
            "probe did not reject {handle} as expected:\n{stderr}"
        );
    }
    assert!(
        stderr.contains("Deserialize") && stderr.contains("Serialize"),
        "probe failed without serde trait rejection:\n{stderr}"
    );
    assert!(
        stderr.contains("associated function `new` is private"),
        "probe did not reject private constructors:\n{stderr}"
    );
}
