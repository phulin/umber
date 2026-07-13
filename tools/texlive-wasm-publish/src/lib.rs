//! Deterministic publisher for a pinned browser TeX Live subset.

#![allow(clippy::disallowed_methods)] // Host release tooling intentionally owns filesystem I/O.

mod scan;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub use scan::tree_sha256;
use scan::{Candidate, scan_roots};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishConfig {
    pub schema: u32,
    pub distribution: String,
    pub objects_base_url: String,
    pub roots: Vec<RootConfig>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootConfig {
    pub name: String,
    pub path: PathBuf,
    pub tree_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub distribution: String,
    pub objects_base_url: String,
    pub files: BTreeMap<String, ManifestFile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    pub virtual_path: String,
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

pub fn publish(config: &PublishConfig, output: &Path) -> Result<Manifest> {
    validate_config(config)?;
    let candidates = scan_roots(&config.roots)?;
    let winners = flatten_candidates(candidates)?;
    validate_dependencies(&config.dependencies, &winners)?;

    let objects = output.join("objects");
    fs::create_dir_all(&objects)
        .with_context(|| format!("create output directory {}", objects.display()))?;

    let mut files = BTreeMap::new();
    let mut expected_objects = BTreeSet::new();
    for (key, candidate) in winners {
        let bytes = fs::read(&candidate.source)
            .with_context(|| format!("read {}", candidate.source.display()))?;
        let object = format!("sha256-{}", candidate.sha256);
        let object_path = objects.join(&object);
        fs::write(&object_path, &bytes)
            .with_context(|| format!("write {}", object_path.display()))?;
        expected_objects.insert(object.clone());
        files.insert(
            key.clone(),
            ManifestFile {
                virtual_path: format!("/texlive/{}", candidate.relative),
                object,
                sha256: candidate.sha256,
                bytes: u64::try_from(bytes.len()).context("file length exceeds u64")?,
                dependencies: config.dependencies.get(&key).cloned().unwrap_or_default(),
            },
        );
    }
    remove_stale_objects(&objects, &expected_objects)?;

    let manifest = Manifest {
        schema: config.schema,
        distribution: config.distribution.clone(),
        objects_base_url: config.objects_base_url.clone(),
        files,
    };
    let mut encoded = serde_json::to_vec_pretty(&manifest).context("serialize manifest")?;
    encoded.push(b'\n');
    fs::write(output.join("manifest.json"), encoded).context("write manifest")?;
    Ok(manifest)
}

fn remove_stale_objects(objects: &Path, expected: &BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(objects)
        .with_context(|| format!("read object directory {}", objects.display()))?
    {
        let entry = entry.context("read object directory entry")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !expected.contains(&name) {
            let path = entry.path();
            let metadata = entry.metadata().context("inspect stale object")?;
            if metadata.is_dir() {
                fs::remove_dir_all(&path)
                    .with_context(|| format!("remove stale directory {}", path.display()))?;
            } else {
                fs::remove_file(&path)
                    .with_context(|| format!("remove stale object {}", path.display()))?;
            }
        }
    }
    Ok(())
}

fn validate_config(config: &PublishConfig) -> Result<()> {
    if config.schema != 1 {
        bail!("unsupported manifest schema {}; expected 1", config.schema);
    }
    if config.distribution.is_empty() || config.distribution.contains(char::is_whitespace) {
        bail!("distribution must be a non-empty identifier without whitespace");
    }
    if config.roots.is_empty() {
        bail!("at least one pinned TEXMF root is required");
    }
    if !config.objects_base_url.ends_with('/') {
        bail!("objectsBaseUrl must end with '/'");
    }
    Ok(())
}

fn flatten_candidates(candidates: Vec<Candidate>) -> Result<BTreeMap<String, Candidate>> {
    let mut winners = BTreeMap::new();
    let mut folded = BTreeMap::<String, String>::new();
    for candidate in candidates {
        for name in candidate.logical_names() {
            let key = format!("{}:{name}", candidate.kind);
            let fold = key.to_lowercase();
            if let Some(previous) = folded.get(&fold)
                && previous != &key
            {
                bail!("case-fold lookup collision between {previous:?} and {key:?}");
            }
            folded.insert(fold, key.clone());
            winners.entry(key).or_insert_with(|| candidate.clone());
        }
    }
    Ok(winners)
}

fn validate_dependencies(
    dependencies: &BTreeMap<String, Vec<String>>,
    files: &BTreeMap<String, Candidate>,
) -> Result<()> {
    for (owner, hints) in dependencies {
        if !files.contains_key(owner) {
            bail!("dependency owner {owner:?} is not a published lookup key");
        }
        for hint in hints {
            if !files.contains_key(hint) {
                bail!("dependency hint {hint:?} from {owner:?} is not published");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
