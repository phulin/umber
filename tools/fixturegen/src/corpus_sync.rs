use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use corpus_manifest::{Entry, parse_manifest_file};
use sha2::{Digest, Sha256};

use crate::layout_migration::publish_case_inventory;

#[derive(Debug, Clone)]
pub(crate) struct SyncOptions {
    pub manifest_path: PathBuf,
    pub destination: PathBuf,
    pub offline: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            manifest_path: PathBuf::from("tests/corpus-manifest.txt"),
            destination: PathBuf::from("third_party/corpus"),
            offline: false,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum EntryStatus {
    Verified { name: String, path: PathBuf },
    Fetched { name: String, path: PathBuf },
}

impl fmt::Display for EntryStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verified { name, path } => {
                write!(formatter, "verified {name}: {}", path.display())
            }
            Self::Fetched { name, path } => write!(formatter, "fetched {name}: {}", path.display()),
        }
    }
}

pub(crate) fn run(options: &SyncOptions) -> Result<Vec<EntryStatus>> {
    let manifest = parse_manifest_file(&options.manifest_path)
        .with_context(|| format!("failed to parse {}", options.manifest_path.display()))?;
    let mut inventory = BTreeMap::new();
    let mut statuses = Vec::with_capacity(manifest.entries.len());
    let mut changed = false;
    for entry in &manifest.entries {
        let path = options.destination.join(&entry.name);
        let (bytes, status) = if path.exists() {
            let bytes =
                fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
            verify(entry, &bytes).with_context(|| {
                format!(
                    "sha256 mismatch for cached {} at {}; remove the file and rerun to refetch",
                    entry.name,
                    path.display()
                )
            })?;
            (
                bytes,
                EntryStatus::Verified {
                    name: entry.name.clone(),
                    path,
                },
            )
        } else {
            if options.offline {
                bail!(
                    "missing corpus document {} at {} while running --offline",
                    entry.name,
                    path.display()
                );
            }
            changed = true;
            (
                fetch_verified(entry, &path)?,
                EntryStatus::Fetched {
                    name: entry.name.clone(),
                    path,
                },
            )
        };
        inventory.insert(entry.name.clone(), bytes);
        statuses.push(status);
    }

    if changed {
        let authority_root = options
            .destination
            .parent()
            .context("corpus destination has no parent")?;
        fs::create_dir_all(authority_root)?;
        publish_case_inventory(authority_root, &options.destination, inventory)?;
    }
    Ok(statuses)
}

fn fetch_verified(entry: &Entry, path: &Path) -> Result<Vec<u8>> {
    let mut failures = Vec::with_capacity(entry.urls.len());
    for url in &entry.urls {
        match fetch_url(url) {
            Ok(bytes) => match verify(entry, &bytes) {
                Ok(()) => return Ok(bytes),
                Err(error) => failures.push(format!("{url}: {error}")),
            },
            Err(error) => failures.push(format!("{url}: {error:#}")),
        }
    }
    bail!(
        "all {} locators failed for corpus document {}: {}; not writing {}",
        entry.urls.len(),
        entry.name,
        failures.join("; "),
        path.display()
    )
}

fn verify(entry: &Entry, bytes: &[u8]) -> Result<()> {
    let actual = sha256_hex(bytes);
    if actual != entry.sha256 {
        bail!("sha256 mismatch (expected {}, got {actual})", entry.sha256);
    }
    Ok(())
}

fn fetch_url(url: &str) -> Result<Vec<u8>> {
    let mut response = reqwest::blocking::get(url)?.error_for_status()?;
    let mut bytes = Vec::new();
    response.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests;
