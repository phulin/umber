use std::fs;
use std::process::Command;

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_import_private_stores() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create stores boundary probe workspace");
    let probe_dir = probe_workspace.path().join("stores-boundary-probe");
    let src_dir = probe_dir.join("src");

    fs::create_dir_all(&src_dir).expect("create stores boundary probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "stores-boundary-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write stores boundary probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use tex_state::stores::Stores;

fn main() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    stores.rollback(snapshot);
}
"#,
    )
    .expect("write stores boundary probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run stores boundary probe");

    assert!(
        !output.status.success(),
        "downstream Stores probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("module `stores` is private"),
        "probe failed for an unexpected reason:\n{stderr}"
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_construct_or_mutate_raw_env() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create live boundary probe workspace");
    let probe_dir = probe_workspace.path().join("live-boundary-probe");
    let src_dir = probe_dir.join("src");

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
    let _default_env = Env::default();
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
    for expected in ["Env::new", "Env::default"] {
        assert!(
            stderr.contains("E0624") && stderr.contains(expected),
            "probe failed for an unexpected reason while checking {expected}:\n{stderr}"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_construct_or_mutate_raw_interner_or_code_tables() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create raw table boundary probe workspace");
    let probe_dir = probe_workspace.path().join("raw-table-boundary-probe");
    let src_dir = probe_dir.join("src");

    fs::create_dir_all(&src_dir).expect("create raw table boundary probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "raw-table-boundary-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write raw table boundary probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use tex_state::code_tables::CodeTables;
use tex_state::interner::Interner;
use tex_state::token::Catcode;

fn main() {
    let mut interner = Interner::new();
    let _symbol = interner.intern("rogue");

    let mut tables = CodeTables::new();
    tables.set_catcode('@', Catcode::Letter);
    tables.set_lccode('@', u32::from('a'));
    tables.set_uccode('@', u32::from('A'));
    tables.set_sfcode('@', 1000);
    tables.set_mathcode('@', u32::from('@'));
    tables.set_delcode('@', -1);
}
"#,
    )
    .expect("write raw table boundary probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run raw table boundary probe");

    assert!(
        !output.status.success(),
        "downstream raw table/interner probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "Interner::new",
        "method `intern` is private",
        "CodeTables::new",
        "method `set_catcode` is private",
        "method `set_lccode` is private",
        "method `set_uccode` is private",
        "method `set_sfcode` is private",
        "method `set_mathcode` is private",
        "method `set_delcode` is private",
    ] {
        assert!(
            stderr.contains("E0624") && stderr.contains(expected),
            "probe failed for an unexpected reason while checking {expected}:\n{stderr}"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_construct_or_mutate_raw_content_stores() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create content boundary probe workspace");
    let probe_dir = probe_workspace.path().join("content-boundary-probe");
    let src_dir = probe_dir.join("src");

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
use tex_state::scaled::Scaled;
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
    node_builder.push(Node::MathOn(Scaled::from_raw(0)));
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

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_construct_or_mutate_raw_source_map() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create source-map boundary probe workspace");
    let probe_dir = probe_workspace.path().join("source-map-boundary-probe");
    let src_dir = probe_dir.join("src");

    fs::create_dir_all(&src_dir).expect("create source-map boundary probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "source-map-boundary-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write source-map boundary probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use tex_state::source_map::SourceMap;

fn main() {
    let _map = SourceMap::default();
}
"#,
    )
    .expect("write source-map boundary probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run source-map boundary probe");

    assert!(!output.status.success(), "raw source-map probe compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("struct `SourceMap` is private"),
        "probe failed for an unexpected reason:\n{stderr}"
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_construct_raw_origin_or_traced_words() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create token boundary probe workspace");
    let probe_dir = probe_workspace.path().join("token-boundary-probe");
    let src_dir = probe_dir.join("src");

    fs::create_dir_all(&src_dir).expect("create token boundary probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "token-boundary-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write token boundary probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use tex_state::token::{OriginId, TracedTokenWord};

fn main() {
    let origin = OriginId::from_raw(123);
    let word = TracedTokenWord::from_raw(456);
    let _origin_raw = origin.raw();
    let _word_raw = word.raw();
}
"#,
    )
    .expect("write token boundary probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run token boundary probe");

    assert!(
        !output.status.success(),
        "downstream raw token probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E0624") && stderr.contains("OriginId::from_raw"),
        "probe failed for an unexpected reason while checking OriginId::from_raw:\n{stderr}"
    );
    assert!(
        (stderr.contains("E0624") && stderr.contains("TracedTokenWord::from_raw"))
            || (stderr.contains("E0599") && stderr.contains("from_raw")),
        "probe failed for an unexpected reason while checking TracedTokenWord::from_raw:\n{stderr}"
    );
    assert!(
        stderr.contains("raw") && (stderr.contains("E0624") || stderr.contains("E0599")),
        "probe failed for an unexpected reason while checking raw accessors:\n{stderr}"
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_commit_world_effects_without_universe_boundary() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create world boundary probe workspace");
    let probe_dir = probe_workspace.path().join("world-boundary-probe");
    let src_dir = probe_dir.join("src");

    fs::create_dir_all(&src_dir).expect("create world boundary probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "world-boundary-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write world boundary probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use tex_state::{StreamSlot, Universe};

fn main() {
    let mut universe = Universe::new();
    let effect_pos = universe.world().effect_pos();
    universe.world_mut().commit_effects(effect_pos).unwrap();
    let _ = universe.world_mut().store_artifact(b"page").unwrap();
    let tokens = universe.intern_token_list(&[]);
    universe
        .world_mut()
        .record_deferred_write(StreamSlot::new(0), tokens);
}
"#,
    )
    .expect("write world boundary probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run world boundary probe");

    assert!(
        !output.status.success(),
        "downstream raw World commit probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "method `commit_effects` is private",
        "method `store_artifact` is private",
        "method `record_deferred_write` is private",
    ] {
        assert!(
            stderr.contains("E0624") && stderr.contains(expected),
            "probe failed for an unexpected reason while checking {expected}:\n{stderr}"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)]
fn downstream_crate_cannot_bypass_universe_facade_through_raw_env_ref() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let probe_workspace = tempfile::tempdir().expect("create universe env probe workspace");
    let probe_dir = probe_workspace.path().join("universe-env-probe");
    let src_dir = probe_dir.join("src");

    fs::create_dir_all(&src_dir).expect("create universe env probe src dir");
    fs::write(
        probe_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "universe-env-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
tex-state = {{ path = "{manifest_dir}" }}
"#
        ),
    )
    .expect("write universe env probe manifest");
    fs::write(
        src_dir.join("main.rs"),
        r#"use tex_state::Universe;

fn main() {
    let universe = Universe::new();
    let _ = universe.env();
}
"#,
    )
    .expect("write universe env probe main");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(probe_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(probe_dir.join("target"))
        .output()
        .expect("run universe env probe");

    assert!(
        !output.status.success(),
        "downstream raw Universe env probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E0599") && stderr.contains("no method named `env`"),
        "probe failed for an unexpected reason:\n{stderr}"
    );
}
