//! Deterministic publisher for a pinned browser TeX Live subset.

#![allow(clippy::disallowed_methods)] // Host release tooling intentionally owns filesystem I/O.

mod scan;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    #[serde(default)]
    pub formats: Vec<FormatConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootConfig {
    pub name: String,
    pub path: PathBuf,
    pub tree_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormatConfig {
    pub path: PathBuf,
    pub metadata: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FormatMetadata {
    schema: u32,
    name: String,
    object: String,
    sha256: String,
    bytes: u64,
    engine: String,
    engine_version: String,
    format_schema: u32,
    source_distribution: String,
    source_manifest_sha256: String,
    source_date_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub distribution: String,
    pub objects_base_url: String,
    pub files: BTreeMap<String, ManifestFile>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub formats: BTreeMap<String, ManifestFormat>,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFormat {
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
    pub engine: String,
    pub engine_version: String,
    pub format_schema: u32,
    pub source_distribution: String,
    pub source_manifest_sha256: String,
    pub source_date_epoch: u64,
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
    let mut formats = BTreeMap::new();
    for format in &config.formats {
        let (name, manifest_format, bytes) = load_format(format)?;
        if formats
            .insert(name.clone(), manifest_format.clone())
            .is_some()
        {
            bail!("duplicate published format name {name:?}");
        }
        fs::write(objects.join(&manifest_format.object), bytes)
            .with_context(|| format!("write format object {}", manifest_format.object))?;
        expected_objects.insert(manifest_format.object);
    }
    remove_stale_objects(&objects, &expected_objects)?;

    let manifest = Manifest {
        schema: config.schema,
        distribution: config.distribution.clone(),
        objects_base_url: config.objects_base_url.clone(),
        files,
        formats,
    };
    let mut encoded = serde_json::to_vec_pretty(&manifest).context("serialize manifest")?;
    encoded.push(b'\n');
    fs::write(output.join("manifest.json"), encoded).context("write manifest")?;
    Ok(manifest)
}

fn load_format(config: &FormatConfig) -> Result<(String, ManifestFormat, Vec<u8>)> {
    let metadata_bytes = fs::read(&config.metadata)
        .with_context(|| format!("read format metadata {}", config.metadata.display()))?;
    let metadata: FormatMetadata =
        serde_json::from_slice(&metadata_bytes).context("parse format metadata")?;
    if metadata.schema != 1 || metadata.engine != "umber" {
        bail!("format metadata must describe schema 1 for engine umber");
    }
	if metadata.engine_version.is_empty()
		|| metadata.format_schema == 0
		|| metadata.source_distribution.is_empty()
	{
		bail!("format compatibility metadata is incomplete");
	}
    if metadata.name.is_empty()
        || !metadata
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("invalid published format name {:?}", metadata.name);
    }
    validate_sha256(&metadata.sha256, "format sha256")?;
    validate_sha256(&metadata.source_manifest_sha256, "source manifest sha256")?;
    if metadata.object != format!("sha256-{}", metadata.sha256) {
        bail!("format object name does not match its digest");
    }
    let bytes = fs::read(&config.path)
        .with_context(|| format!("read format image {}", config.path.display()))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != metadata.sha256 || bytes.len() as u64 != metadata.bytes {
        bail!("format image digest or length does not match its metadata");
    }
    if bytes.get(..8) != Some(b"UMBRFMT\0") {
        bail!("published format is not an Umber format image");
    }
    let schema = u32::from_le_bytes(
        bytes
            .get(8..12)
            .context("published format header is truncated")?
            .try_into()
            .context("format schema header width")?,
    );
    if schema != metadata.format_schema {
        bail!("format image schema does not match its metadata");
    }
    let published = ManifestFormat {
        object: metadata.object,
        sha256: metadata.sha256,
        bytes: metadata.bytes,
        engine: metadata.engine,
        engine_version: metadata.engine_version,
        format_schema: metadata.format_schema,
        source_distribution: metadata.source_distribution,
        source_manifest_sha256: metadata.source_manifest_sha256,
        source_date_epoch: metadata.source_date_epoch,
    };
    Ok((metadata.name, published, bytes))
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        bail!("{label} must be 64 lowercase hexadecimal characters");
    }
    Ok(())
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
