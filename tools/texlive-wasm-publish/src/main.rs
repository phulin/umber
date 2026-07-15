#![allow(clippy::disallowed_methods)] // Host release tooling intentionally owns filesystem I/O.

use std::env;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use texlive_wasm_publish::{PublishConfig, publish, tree_sha256};

fn main() -> Result<()> {
    let mut arguments = env::args_os().skip(1);
    let Some(config_path) = arguments.next() else {
        bail!("usage: texlive-wasm-publish CONFIG.json OUTPUT-DIRECTORY | --tree-sha256 ROOT");
    };
    if config_path == "--tree-sha256" {
        let Some(root) = arguments.next() else {
            bail!("missing ROOT after --tree-sha256");
        };
        if arguments.next().is_some() {
            bail!("unexpected argument after --tree-sha256 ROOT");
        }
        println!("{}", tree_sha256(Path::new(&root))?);
        return Ok(());
    }
    let Some(output_path) = arguments.next() else {
        bail!("usage: texlive-wasm-publish CONFIG.json OUTPUT-DIRECTORY");
    };
    if arguments.next().is_some() {
        bail!("usage: texlive-wasm-publish CONFIG.json OUTPUT-DIRECTORY");
    }
    let config_path = Path::new(&config_path);
    let bytes = fs::read(config_path)
        .with_context(|| format!("read publisher config {}", config_path.display()))?;
    let mut config: PublishConfig = serde_json::from_slice(&bytes).context("parse config JSON")?;
    let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
    for root in &mut config.roots {
        if root.path.is_relative() {
            root.path = parent.join(&root.path);
        }
    }
    for format in &mut config.formats {
        if format.path.is_relative() {
            format.path = parent.join(&format.path);
        }
        if format.metadata.is_relative() {
            format.metadata = parent.join(&format.metadata);
        }
    }
    publish(&config, Path::new(&output_path))?;
    Ok(())
}
