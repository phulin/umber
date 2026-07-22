#![allow(clippy::disallowed_methods)] // Host release tooling intentionally owns filesystem I/O.

use std::env;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use texlive_wasm_publish::{PublishConfig, publish, tree_sha256, verify_sharded_snapshot};
use umber_distribution::Manifest;

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
    if config_path == "--shard-existing" {
        let Some(staging) = arguments.next() else {
            bail!("missing STAGING after --shard-existing");
        };
        let Some(flag) = arguments.next() else {
            bail!("missing --shard-bits after --shard-existing STAGING");
        };
        if flag != "--shard-bits" {
            bail!("expected --shard-bits after --shard-existing STAGING");
        }
        let Some(bits) = arguments.next() else {
            bail!("missing BITS after --shard-bits");
        };
        if arguments.next().is_some() {
            bail!("unexpected argument after --shard-bits BITS");
        }
        let bits = bits
            .to_string_lossy()
            .parse::<u8>()
            .context("parse shard bits")?;
        let staging = Path::new(&staging);
        let text = fs::read_to_string(staging.join("manifest.json"))
            .context("read existing monolithic manifest")?;
        let manifest = Manifest::parse(&text).context("parse existing monolithic manifest")?;
        let publication = texlive_wasm_publish::write_sharded_manifest(&manifest, bits, staging)?;
        texlive_wasm_publish::prune_unreferenced_objects(staging, &publication)?;
        verify_sharded_snapshot(staging)?;
        return Ok(());
    }
    if config_path == "--verify-sharded" {
        let Some(staging) = arguments.next() else {
            bail!("missing STAGING after --verify-sharded");
        };
        if arguments.next().is_some() {
            bail!("unexpected argument after --verify-sharded STAGING");
        }
        verify_sharded_snapshot(Path::new(&staging))?;
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
    if let Some(html) = &mut config.html {
        if html.catalog.is_relative() {
            html.catalog = parent.join(&html.catalog);
        }
        for source in html.object_sources.values_mut() {
            if source.is_relative() {
                *source = parent.join(&*source);
            }
        }
    }
    if let Some(package_database) = &mut config.package_database
        && package_database.is_relative()
    {
        *package_database = parent.join(&*package_database);
    }
    publish(&config, Path::new(&output_path))?;
    Ok(())
}
