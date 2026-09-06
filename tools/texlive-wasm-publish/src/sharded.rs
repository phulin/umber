use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use umber_distribution::{
    FontManifestRecord, HTML_SHARDED_ROOT_SCHEMA, LegacyMappingManifestRecord, Manifest,
    ObjectEntry, SHARDED_ROOT_SCHEMA, ShardedCatalog, ShardedManifestRoot, ValidatedPackedShard,
    assemble_sharded_catalog, pack_shard, unpack_shard,
};
use umber_hash::{AHash64, AHash64Hasher, HashDomain};

pub const ROOT_SCHEMA: u32 = SHARDED_ROOT_SCHEMA;

type FetchEntry = ObjectEntry;

pub type ShardedPublication = ShardedCatalog;

/// The small publication boundary shared by payload, format, and shard
/// objects. Implementations retain only object identities; the byte buffer is
/// owned by the caller until this method returns.
pub(crate) trait ObjectSink {
    fn write_bytes(
        &mut self,
        object: &str,
        expected_ahash64: &str,
        expected_bytes: u64,
        bytes: Vec<u8>,
    ) -> Result<()>;

    fn inventory(&self) -> ObjectInventory;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ObjectInventory {
    pub(crate) objects: usize,
    pub(crate) bytes: u64,
}

/// Filesystem-backed object sink used by production publication. Existing
/// content-addressed objects are admitted only after their identity is
/// revalidated; duplicate writes do not retain another byte copy.
pub(crate) struct FilesystemObjectSink {
    objects: std::path::PathBuf,
    seen: BTreeSet<String>,
    inventory: ObjectInventory,
}

impl FilesystemObjectSink {
    pub(crate) fn new(output: &Path) -> Result<Self> {
        let objects = output.join("objects");
        fs::create_dir_all(&objects)
            .with_context(|| format!("create output directory {}", objects.display()))?;
        Ok(Self {
            objects,
            seen: BTreeSet::new(),
            inventory: ObjectInventory::default(),
        })
    }

    fn admit_identity(
        &mut self,
        object: &str,
        expected_ahash64: &str,
        expected_bytes: u64,
    ) -> Result<bool> {
        validate_object_name(object, expected_ahash64)?;
        if !self.seen.insert(object.to_owned()) {
            return Ok(false);
        }
        self.inventory.objects = self
            .inventory
            .objects
            .checked_add(1)
            .context("publication object count overflow")?;
        self.inventory.bytes = self
            .inventory
            .bytes
            .checked_add(expected_bytes)
            .context("publication object byte count overflow")?;
        Ok(true)
    }
}

impl ObjectSink for FilesystemObjectSink {
    fn write_bytes(
        &mut self,
        object: &str,
        expected_ahash64: &str,
        expected_bytes: u64,
        bytes: Vec<u8>,
    ) -> Result<()> {
        let actual_bytes = u64::try_from(bytes.len()).context("object length exceeds u64")?;
        let actual_ahash64 = ahash64(&bytes);
        if actual_bytes != expected_bytes || actual_ahash64 != expected_ahash64 {
            bail!(
                "object {object} does not match declared digest and length"
            );
        }
        let first_write = self.admit_identity(object, expected_ahash64, expected_bytes)?;
        if !first_write {
            return Ok(());
        }
        let path = self.objects.join(object);
        if path.exists() {
            verify_existing_object(&path, expected_ahash64, expected_bytes, object)?;
        } else {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .with_context(|| format!("create object {object}"))?;
            file.write_all(&bytes)
                .with_context(|| format!("write object {object}"))?;
            file.flush()
                .with_context(|| format!("flush object {object}"))?;
        }
        Ok(())
    }

    fn inventory(&self) -> ObjectInventory {
        self.inventory
    }
}

fn validate_object_name(object: &str, expected_ahash64: &str) -> Result<()> {
    let expected = format!("ahash64-v1-{expected_ahash64}");
    if object != expected {
        bail!("object name {object:?} does not match declared digest");
    }
    Ok(())
}

fn verify_existing_object(
    path: &Path,
    expected_ahash64: &str,
    expected_bytes: u64,
    object: &str,
) -> Result<()> {
    let metadata = fs::metadata(path).with_context(|| format!("inspect object {object}"))?;
    if !metadata.is_file() {
        bail!("object {object} is not a regular file");
    }
    if metadata.len() != expected_bytes {
        bail!("object {object} does not match declared digest and length");
    }
    let mut file = File::open(path).with_context(|| format!("read object {object}"))?;
    let mut buffer = [0_u8; 1024 * 1024];
    let mut hasher = AHash64Hasher::new(HashDomain::DistributionContent);
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("read object {object}"))?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
    }
    if hasher.finish().hex() != expected_ahash64 {
        bail!("object {object} does not match declared digest and length");
    }
    Ok(())
}

pub fn shard_manifest(manifest: &Manifest, shard_bits: u8) -> Result<ShardedPublication> {
    umber_distribution::shard_manifest(manifest, shard_bits).map_err(Into::into)
}

pub(crate) fn shard_manifest_with_records(
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
    let mut sink = FilesystemObjectSink::new(output)?;
    let publication = shard_manifest(manifest, shard_bits)?;
    write_shard_objects(&publication, &mut sink)?;
    verify_staged_objects(output, &publication)?;
    write_root_manifest(&publication, output)?;
    Ok(publication)
}

pub fn write_html_sharded_manifest(
    manifest: &Manifest,
    shard_bits: u8,
    output: &Path,
    fonts: &BTreeMap<String, FontManifestRecord>,
    legacy_mappings: &BTreeMap<String, LegacyMappingManifestRecord>,
) -> Result<ShardedPublication> {
    let mut sink = FilesystemObjectSink::new(output)?;
    let publication = shard_manifest_with_records(
        manifest,
        shard_bits,
        HTML_SHARDED_ROOT_SCHEMA,
        fonts,
        legacy_mappings,
    )?;
    write_shard_objects(&publication, &mut sink)?;
    verify_staged_objects(output, &publication)?;
    write_root_manifest(&publication, output)?;
    Ok(publication)
}

pub(crate) fn write_shard_objects<S: ObjectSink>(
    publication: &ShardedPublication,
    sink: &mut S,
) -> Result<()> {
    for (shard, digest) in publication.shards.iter().zip(&publication.root.shards) {
        let bytes = pack_shard(shard).context("encode packed index shard")?;
        let object = format!("ahash64-v1-{digest}");
        let length = u64::try_from(bytes.len()).context("packed shard length exceeds u64")?;
        sink.write_bytes(&object, digest, length, bytes)?;
    }
    Ok(())
}

pub(crate) fn write_root_manifest(publication: &ShardedPublication, output: &Path) -> Result<()> {
    let temporary = output.join("manifest.json.tmp");
    fs::write(&temporary, publication.root.to_json()).context("write staged root manifest")?;
    fs::rename(&temporary, output.join("manifest.json")).context("commit root manifest")?;
    Ok(())
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

/// Validate all staged payload and packed-shard objects before the root is
/// committed. This is intentionally separate from `read_sharded_catalog`,
/// whose root file is the post-commit trust boundary.
pub(crate) fn verify_staged_objects(
    output: &Path,
    publication: &ShardedPublication,
) -> Result<()> {
    verify_catalog_objects(output, publication)?;
    verify_staged_shard_objects(output, publication)
}

pub(crate) fn verify_staged_shard_objects(
    output: &Path,
    publication: &ShardedPublication,
) -> Result<()> {
    for (index, digest) in publication.root.shards.iter().enumerate() {
        let object = format!("ahash64-v1-{digest}");
        let bytes = fs::read(output.join("objects").join(&object))
            .with_context(|| format!("read object for shard {index}"))?;
        if ahash64(&bytes) != *digest {
            bail!("object for shard {index} does not match its declared digest");
        }
        let packed = ValidatedPackedShard::new(bytes, &publication.root, index as u32)
            .context("validate packed index shard")?;
        unpack_shard(&packed).context("decode packed index shard")?;
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
