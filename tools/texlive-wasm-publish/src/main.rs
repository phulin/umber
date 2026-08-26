#![allow(clippy::disallowed_methods)] // Host release tooling intentionally owns filesystem I/O.

use std::env;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use texlive_wasm_publish::{
    PublishConfig, file_ahash64, publish, publish_successor, tree_ahash64, verify_sharded_snapshot,
    verify_successor,
};
use umber_distribution::Manifest;

fn main() -> Result<()> {
    let mut arguments = env::args_os().skip(1);
    let Some(config_path) = arguments.next() else {
        bail!(
            "usage: texlive-wasm-publish CONFIG.json OUTPUT-DIRECTORY | --tree-ahash64 ROOT | --file-ahash64 FILE"
        );
    };
    if config_path == "--tree-ahash64" {
        let Some(root) = arguments.next() else {
            bail!("missing ROOT after --tree-ahash64");
        };
        if arguments.next().is_some() {
            bail!("unexpected argument after --tree-ahash64 ROOT");
        }
        println!("{}", tree_ahash64(Path::new(&root))?);
        return Ok(());
    }
    if config_path == "--file-ahash64" {
        let Some(path) = arguments.next() else {
            bail!("missing FILE after --file-ahash64");
        };
        if arguments.next().is_some() {
            bail!("unexpected argument after --file-ahash64 FILE");
        }
        println!("{}", file_ahash64(Path::new(&path))?);
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
    if config_path == "--verify-successor" {
        let Some(base) = arguments.next() else {
            bail!("missing BASE after --verify-successor");
        };
        let Some(flag) = arguments.next() else {
            bail!("missing --base-ahash64 after --verify-successor BASE");
        };
        if flag != "--base-ahash64" {
            bail!("expected --base-ahash64 after --verify-successor BASE");
        }
        let Some(base_ahash64) = arguments.next() else {
            bail!("missing AHASH64 after --base-ahash64");
        };
        let Some(staging) = arguments.next() else {
            bail!("missing STAGING after --base-ahash64 AHASH64");
        };
        if arguments.next().is_some() {
            bail!("unexpected argument after --verify-successor BASE STAGING");
        }
        verify_successor(
            Path::new(&base),
            &base_ahash64.to_string_lossy(),
            Path::new(&staging),
        )?;
        return Ok(());
    }
    let successor_base = if config_path == "--successor" {
        let Some(base) = arguments.next() else {
            bail!("missing BASE after --successor");
        };
        let Some(flag) = arguments.next() else {
            bail!("missing --base-ahash64 after --successor BASE");
        };
        if flag != "--base-ahash64" {
            bail!("expected --base-ahash64 after --successor BASE");
        }
        let Some(base_ahash64) = arguments.next() else {
            bail!("missing AHASH64 after --base-ahash64");
        };
        Some((base, base_ahash64))
    } else {
        None
    };
    let config_path = if successor_base.is_some() {
        let Some(config) = arguments.next() else {
            bail!("missing CONFIG.json after --successor BASE");
        };
        config
    } else {
        config_path
    };
    let Some(output_path) = arguments.next() else {
        bail!("usage: texlive-wasm-publish [--successor BASE] CONFIG.json OUTPUT-DIRECTORY");
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
        if let Some(input_identities) = &mut format.input_identities
            && input_identities.is_relative()
        {
            *input_identities = parent.join(&*input_identities);
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
    if let Some((base, base_ahash64)) = successor_base {
        publish_successor(
            Path::new(&base),
            &base_ahash64.to_string_lossy(),
            &config,
            Path::new(&output_path),
        )?;
    } else {
        publish(&config, Path::new(&output_path))?;
    }
    Ok(())
}
