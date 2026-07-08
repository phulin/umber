use tex_state::meaning::Meaning;
use tex_state::stores::Stores;

use std::fs;
use std::process::Command;

#[test]
#[should_panic(expected = "symbol is not live in this Stores timeline")]
fn stale_rolled_back_symbol_cannot_mutate_reused_meaning_cell() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern("rolled-back");

    stores.rollback(snapshot);
    stores.set_meaning(stale, Meaning::Relax);
}

#[test]
fn rollback_reuse_starts_with_undefined_meaning() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern("rolled-back");

    stores.rollback(snapshot);
    let reused = stores.intern("reused");

    assert_eq!(reused.raw(), stale.raw());
    assert_eq!(stores.meaning(reused), Meaning::Undefined);
}

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_construct_or_mutate_raw_env() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_dir = std::path::Path::new(manifest_dir)
        .join("target")
        .join("live-boundary-probe");
    let src_dir = probe_dir.join("src");

    let _ = fs::remove_dir_all(&probe_dir);
    fs::create_dir_all(&src_dir).expect("create live boundary probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "live-boundary-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write live boundary probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use tex_state::env::Env;
use tex_state::env::banks::IntParam;

fn main() {
    let mut env = Env::new();
    env.bump_epoch();
    env.enter_group();
    env.push_aftergroup(1);
    let _ = env.leave_group();
    env.set_count(0, 1);
    env.set_int_param(IntParam::new(0), 1);
}
"#,
    )
    .expect("write live boundary probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run live boundary probe");

    assert!(
        !output.status.success(),
        "downstream raw Env probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E0624") && stderr.contains("Env::new"),
        "probe failed for an unexpected reason:\n{stderr}"
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_construct_or_mutate_raw_content_stores() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_dir = std::path::Path::new(manifest_dir)
        .join("target")
        .join("content-boundary-probe");
    let src_dir = probe_dir.join("src");

    let _ = fs::remove_dir_all(&probe_dir);
    fs::create_dir_all(&src_dir).expect("create content boundary probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "content-boundary-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write content boundary probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use tex_state::glue::{GlueSpec, GlueStore};
use tex_state::node::Node;
use tex_state::node_arena::{NodeArena, NodeListBuilder};
use tex_state::survivor::SurvivorArena;
use tex_state::token::Token;
use tex_state::token_store::{TokenListBuilder, TokenStore};

fn main() {
    let mut tokens = TokenStore::new();
    let _ = tokens.intern(&[Token::param(1)]);
    let mut token_builder = TokenListBuilder::new();
    let _ = token_builder.finish(&mut tokens);
    let _ = tokens.get(TokenStore::empty_id());

    let mut glue = GlueStore::new();
    let zero = glue.intern(GlueSpec::ZERO);
    let _ = glue.get(zero);

    let mut nodes = NodeArena::new();
    let survivors = SurvivorArena::new();
    let mut node_builder = NodeListBuilder::new();
    node_builder.push(Node::MathOn);
    let id = node_builder.finish(&mut nodes);
    let _ = nodes.get(id, &survivors);
}
"#,
    )
    .expect("write content boundary probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run content boundary probe");

    assert!(
        !output.status.success(),
        "downstream raw content-store probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "TokenStore::new",
        "TokenListBuilder::new",
        "GlueStore::new",
        "NodeArena::new",
        "NodeListBuilder::new",
        "SurvivorArena::new",
        "method `intern` is private",
        "method `finish` is private",
        "method `get` is private",
    ] {
        assert!(
            stderr.contains("E0624") && stderr.contains(expected),
            "probe failed for an unexpected reason while checking {expected}:\n{stderr}"
        );
    }
}
