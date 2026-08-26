//! Explicit complete verification for a local immutable distribution.

#![allow(
    clippy::disallowed_methods,
    reason = "this module is an explicit native host-side maintenance tool"
)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use umber_distribution::{
    ObjectEntry, ShardedManifestRoot, ValidatedPackedShard, assemble_sharded_catalog, unpack_shard,
};
use umber_hash::{AHash64, AHash64Hasher, HashDomain};

const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;

/// Complete work performed by [`verify_distribution`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DistributionVerificationReport {
    pub roots: u64,
    pub shards: u64,
    pub objects: u64,
    pub hashed_bytes: u64,
}

#[derive(Debug)]
pub struct DistributionVerificationError {
    message: String,
}

impl DistributionVerificationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn at(path: &Path, operation: &str, error: impl fmt::Display) -> Self {
        Self::new(format!(
            "failed to {operation} distribution path {}: {error}",
            path.display()
        ))
    }
}

impl fmt::Display for DistributionVerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for DistributionVerificationError {}

/// Authenticates a pinned root, every shard, and every referenced object.
///
/// This function is deliberately separate from compilation. It performs a
/// complete immutable-graph audit and never populates or rewrites a cache.
pub fn verify_distribution(
    source: &Path,
    expected_root_ahash64: &str,
) -> Result<DistributionVerificationReport, DistributionVerificationError> {
    if !is_digest(expected_root_ahash64) {
        return Err(DistributionVerificationError::new(
            "distribution root pin must be a lowercase aHash64 digest",
        ));
    }
    let root_path = select_root_path(source);
    let root_bytes = read_bounded(&root_path, MAX_MANIFEST_BYTES, "root manifest")?;
    verify_identity(&root_bytes, expected_root_ahash64, "root manifest")?;
    let root_text = std::str::from_utf8(&root_bytes)
        .map_err(|error| DistributionVerificationError::at(&root_path, "decode", error))?;
    let root = ShardedManifestRoot::parse(root_text)
        .map_err(|error| DistributionVerificationError::at(&root_path, "parse", error))?;
    if root.to_json().as_bytes() != root_bytes {
        return Err(DistributionVerificationError::at(
            &root_path,
            "verify",
            "root manifest is not canonically serialized",
        ));
    }
    let distribution_root = root_path.parent().ok_or_else(|| {
        DistributionVerificationError::at(&root_path, "resolve", "root has no parent directory")
    })?;
    let mut report = DistributionVerificationReport {
        roots: 1,
        hashed_bytes: root_bytes.len() as u64,
        ..DistributionVerificationReport::default()
    };
    let mut shards = Vec::with_capacity(root.shards.len());
    for (index, digest) in root.shards.iter().enumerate() {
        let path = local_object_path(distribution_root, &format!("ahash64-v1-{digest}"));
        let bytes = read_bounded(&path, MAX_MANIFEST_BYTES, "index shard")?;
        let shard_bytes = bytes.len() as u64;
        verify_identity(&bytes, digest, &format!("index shard {index}"))?;
        let packed = ValidatedPackedShard::new(bytes, &root, index as u32)
            .map_err(|error| DistributionVerificationError::at(&path, "validate", error))?;
        let shard = unpack_shard(&packed)
            .map_err(|error| DistributionVerificationError::at(&path, "decode", error))?;
        report.shards = report.shards.saturating_add(1);
        report.hashed_bytes = report.hashed_bytes.saturating_add(shard_bytes);
        shards.push(shard);
    }
    let catalog = assemble_sharded_catalog(root, shards)
        .map_err(|error| DistributionVerificationError::new(error.to_string()))?;
    let mut objects = BTreeMap::<String, (ObjectEntry, String)>::new();
    for (key, entry) in &catalog.files {
        insert_object(&mut objects, entry.object_entry(), format!("file {key}"))?;
    }
    for (name, entry) in &catalog.formats {
        insert_object(&mut objects, entry.object_entry(), format!("format {name}"))?;
    }
    for (key, record) in &catalog.fonts {
        insert_object(&mut objects, record.object.clone(), format!("font {key}"))?;
        insert_object(
            &mut objects,
            record.license.object.clone(),
            format!("font license {key}"),
        )?;
    }
    for (key, record) in &catalog.legacy_mappings {
        insert_object(
            &mut objects,
            record.object.clone(),
            format!("legacy mapping {key}"),
        )?;
        insert_object(
            &mut objects,
            record.license.object.clone(),
            format!("legacy mapping license {key}"),
        )?;
    }
    for (name, (entry, label)) in objects {
        if name != format!("ahash64-v1-{}", entry.ahash64) {
            return Err(DistributionVerificationError::new(format!(
                "object name for {label} does not match its declared digest"
            )));
        }
        let path = local_object_path(distribution_root, &name);
        verify_object(&path, &entry, &label)?;
        report.objects = report.objects.saturating_add(1);
        report.hashed_bytes = report.hashed_bytes.saturating_add(entry.bytes);
    }
    Ok(report)
}

fn insert_object(
    objects: &mut BTreeMap<String, (ObjectEntry, String)>,
    entry: ObjectEntry,
    label: String,
) -> Result<(), DistributionVerificationError> {
    if let Some((existing, existing_label)) = objects.get(&entry.object) {
        if existing != &entry {
            return Err(DistributionVerificationError::new(format!(
                "object {} has conflicting declarations for {existing_label} and {label}",
                entry.object
            )));
        }
    } else {
        objects.insert(entry.object.clone(), (entry, label));
    }
    Ok(())
}

fn verify_object(
    path: &Path,
    entry: &ObjectEntry,
    label: &str,
) -> Result<(), DistributionVerificationError> {
    let metadata = regular_file_metadata(path)?;
    if metadata.len() != entry.bytes {
        return Err(DistributionVerificationError::at(
            path,
            "verify",
            format!("{label} length does not match its declaration"),
        ));
    }
    let mut file =
        File::open(path).map_err(|error| DistributionVerificationError::at(path, "open", error))?;
    let mut digest = AHash64Hasher::new(HashDomain::DistributionContent);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|error| DistributionVerificationError::at(path, "read", error))?;
        if length == 0 {
            break;
        }
        digest.write(&buffer[..length]);
    }
    if digest.finish().hex() != entry.ahash64 {
        return Err(DistributionVerificationError::at(
            path,
            "verify",
            format!("{label} digest does not match its declaration"),
        ));
    }
    Ok(())
}

fn read_bounded(
    path: &Path,
    limit: u64,
    label: &str,
) -> Result<Vec<u8>, DistributionVerificationError> {
    let metadata = regular_file_metadata(path)?;
    if metadata.len() > limit {
        return Err(DistributionVerificationError::at(
            path,
            "verify",
            format!("{label} exceeds the {limit}-byte limit"),
        ));
    }
    fs::read(path).map_err(|error| DistributionVerificationError::at(path, "read", error))
}

fn regular_file_metadata(path: &Path) -> Result<fs::Metadata, DistributionVerificationError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| DistributionVerificationError::at(path, "inspect", error))?;
    if !metadata.file_type().is_file() {
        return Err(DistributionVerificationError::at(
            path,
            "verify",
            io::Error::new(io::ErrorKind::InvalidData, "path is not a regular file"),
        ));
    }
    Ok(metadata)
}

fn verify_identity(
    bytes: &[u8],
    expected: &str,
    label: &str,
) -> Result<(), DistributionVerificationError> {
    let actual = AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex();
    if actual == expected {
        Ok(())
    } else {
        Err(DistributionVerificationError::new(format!(
            "{label} digest mismatch: expected {expected}, received {actual}"
        )))
    }
}

fn select_root_path(source: &Path) -> PathBuf {
    if !source.is_dir() {
        return source.to_owned();
    }
    for name in ["manifest-v9.json", "manifest-v8.json"] {
        let candidate = source.join(name);
        if candidate.exists() {
            return candidate;
        }
    }
    source.join("manifest.json")
}

fn local_object_path(root: &Path, object: &str) -> PathBuf {
    let nested = root.join("objects").join(object);
    if nested.exists() {
        nested
    } else {
        root.join(object)
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests;
