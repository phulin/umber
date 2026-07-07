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
