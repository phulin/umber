use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use umber_distribution::{
    FontManifestRecord, HTML_SHARDED_ROOT_SCHEMA, LegacyMappingManifestRecord, Manifest,
    ObjectEntry, SHARDED_ROOT_SCHEMA, ShardedCatalog, ShardedManifestRoot, ValidatedPackedShard,
    assemble_sharded_catalog, pack_shard, unpack_shard,
};
use umber_hash::{AHash64, HashDomain};

pub const ROOT_SCHEMA: u32 = SHARDED_ROOT_SCHEMA;

type FetchEntry = ObjectEntry;

pub type ShardedPublication = ShardedCatalog;

pub fn shard_manifest(manifest: &Manifest, shard_bits: u8) -> Result<ShardedPublication> {
    umber_distribution::shard_manifest(manifest, shard_bits).map_err(Into::into)
}

fn shard_manifest_records(
    manifest: &Manifest,
    shard_bits: u8,
    root_schema: u32,
    fonts: &BTreeMap<String, FontManifestRecord>,
    legacy_mappings: &BTreeMap<String, LegacyMappingManifestRecord>,
) -> Result<ShardedPublication> {
    umber_distribution::shard_manifest_with_records(
        manifest,
        shard_bits,
        root_schema,
        fonts,
        legacy_mappings,
    )
    .map_err(Into::into)
}

pub fn write_sharded_manifest(
    manifest: &Manifest,
    shard_bits: u8,
    output: &Path,
) -> Result<ShardedPublication> {
    write_publication(shard_manifest(manifest, shard_bits)?, output)
}

pub fn write_html_sharded_manifest(
    manifest: &Manifest,
    shard_bits: u8,
    output: &Path,
    fonts: &BTreeMap<String, FontManifestRecord>,
    legacy_mappings: &BTreeMap<String, LegacyMappingManifestRecord>,
) -> Result<ShardedPublication> {
    let publication = shard_manifest_records(
        manifest,
        shard_bits,
        HTML_SHARDED_ROOT_SCHEMA,
        fonts,
        legacy_mappings,
    )?;
    write_publication(publication, output)
}

fn write_publication(publication: ShardedPublication, output: &Path) -> Result<ShardedPublication> {
    let objects = output.join("objects");
    fs::create_dir_all(&objects)
        .with_context(|| format!("create output directory {}", objects.display()))?;
    for (shard, digest) in publication.shards.iter().zip(&publication.root.shards) {
        let bytes = pack_shard(shard).context("encode packed index shard")?;
        let object = format!("ahash64-v1-{digest}");
        fs::write(objects.join(&object), &bytes)
            .with_context(|| format!("write index shard {object}"))?;
    }
    fs::write(output.join("manifest.json"), publication.root.to_json())
        .context("write root manifest")?;
    Ok(publication)
}

pub fn verify_sharded_snapshot(output: &Path) -> Result<ShardedPublication> {
    let publication = read_sharded_catalog(output)?;
    verify_catalog_objects(output, &publication)?;
    Ok(publication)
}

/// Authenticate a complete root and all of its shards without requiring the
/// payload objects. This is the trust boundary used when an immutable
/// content-addressed publication is succeeded in place: unchanged payloads
/// remain verified by their records, while the successor stages only
/// changed payloads and the newly derived index objects.
pub fn read_sharded_catalog(output: &Path) -> Result<ShardedPublication> {
    let root_bytes = fs::read(output.join("manifest.json")).context("read root manifest")?;
    let root_text = std::str::from_utf8(&root_bytes).context("root manifest is not UTF-8")?;
    let root = ShardedManifestRoot::parse(root_text).context("parse root manifest")?;
    if root.to_json().as_bytes() != root_bytes {
        bail!("root manifest is not canonically serialized");
    }
    let mut shards = Vec::with_capacity(root.shards.len());
    for (index, digest) in root.shards.iter().enumerate() {
        let object = format!("ahash64-v1-{digest}");
        let bytes = fs::read(output.join("objects").join(&object))
            .with_context(|| format!("read object for shard {index}"))?;
        if ahash64(&bytes) != *digest {
            bail!("object for shard {index} does not match its declared digest");
        }
        let packed = ValidatedPackedShard::new(bytes, &root, index as u32)
            .context("validate packed index shard")?;
        shards.push(unpack_shard(&packed).context("decode packed index shard")?);
    }
    assemble_sharded_catalog(root, shards).map_err(Into::into)
}

fn verify_catalog_objects(output: &Path, publication: &ShardedPublication) -> Result<()> {
    for (key, file) in &publication.files {
        read_verified_object(output, &file.object_entry(), key)?;
    }
    for (name, format) in &publication.formats {
        read_verified_object(output, &format.object_entry(), name)?;
    }
    for (key, record) in &publication.fonts {
        read_verified_object_entry(output, &record.object, key)?;
        read_verified_object_entry(
            output,
            &record.license.object,
            &format!("license for {key}"),
        )?;
    }
    for (key, record) in &publication.legacy_mappings {
        read_verified_object_entry(output, &record.object, key)?;
        read_verified_object_entry(
            output,
            &record.license.object,
            &format!("license for {key}"),
        )?;
    }
    Ok(())
}

pub fn shard_index(key: &str, shard_bits: u8) -> usize {
    umber_distribution::shard_index_for_key(key, shard_bits)
        .expect("publisher accepts canonical distribution keys") as usize
}

fn read_verified_object(output: &Path, entry: &FetchEntry, label: &str) -> Result<Vec<u8>> {
    let bytes = fs::read(output.join("objects").join(&entry.object))
        .with_context(|| format!("read object for {label}"))?;
    if bytes.len() as u64 != entry.bytes || ahash64(&bytes) != entry.ahash64 {
        bail!("object for {label} does not match declared digest and length");
    }
    Ok(bytes)
}

fn read_verified_object_entry(
    output: &Path,
    entry: &umber_distribution::ObjectEntry,
    label: &str,
) -> Result<Vec<u8>> {
    read_verified_object(
        output,
        &FetchEntry {
            object: entry.object.clone(),
            ahash64: entry.ahash64.clone(),
            bytes: entry.bytes,
        },
        label,
    )
}

fn ahash64(bytes: &[u8]) -> String {
    AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex()
}

pub fn referenced_objects(publication: &ShardedPublication) -> BTreeSet<String> {
    publication
        .files
        .values()
        .map(|entry| entry.object.clone())
        .chain(
            publication
                .formats
                .values()
                .map(|entry| entry.object.clone()),
        )
        .chain(
            publication
                .root
                .shards
                .iter()
                .map(|digest| format!("ahash64-v1-{digest}")),
        )
        .chain(publication.fonts.values().flat_map(|record| {
            [
                record.object.object.clone(),
                record.license.object.object.clone(),
            ]
        }))
        .chain(publication.legacy_mappings.values().flat_map(|record| {
            [
                record.object.object.clone(),
                record.license.object.object.clone(),
            ]
        }))
        .collect()
}

pub fn prune_unreferenced_objects(output: &Path, publication: &ShardedPublication) -> Result<()> {
    let expected = referenced_objects(publication);
    for entry in fs::read_dir(output.join("objects")).context("read staged object directory")? {
        let entry = entry.context("read staged object entry")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !expected.contains(&name) {
            fs::remove_file(entry.path()).with_context(|| format!("remove stale object {name}"))?;
        }
    }
    Ok(())
}
