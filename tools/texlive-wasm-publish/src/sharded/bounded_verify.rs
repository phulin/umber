use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use umber_distribution::{
    ObjectEntry, PackedRecordKind, ShardedManifestRoot, ValidatedPackedShard,
};
use umber_hash::{AHash64Hasher, HashDomain};

/// Authenticate a complete sharded publication without assembling a catalog.
pub(super) fn verify(output: &Path) -> Result<()> {
    let root = read_canonical_root(output)?;
    let mut keys = BTreeMap::new();
    let mut objects = ObjectTable::default();

    verify_shard_records_first_pass(output, &root, &mut keys, &mut objects)?;
    verify_root_format_closures(&root, &keys)?;
    verify_shard_references_second_pass(output, &root, &keys, &objects)?;
    for (name, format) in &root.formats {
        objects.intern(&format.object_entry(), name)?;
    }
    verify_payload_objects(output, &objects)
}

/// Compact metadata retained by the bounded verifier for one published key.
/// Dependency vectors and decoded catalogue records are intentionally not
/// retained after the shard that contains them is released.
#[derive(Clone, Debug, Eq, PartialEq)]
enum KeyMeta {
    File {
        virtual_path: String,
        object: usize,
    },
    Font {
        object: usize,
        license: LicenseMeta,
    },
    LegacyMapping {
        font_key: String,
        object: usize,
        license: LicenseMeta,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LicenseMeta {
    identity: String,
    object: usize,
    spdx: String,
    embeddable: bool,
    redistributable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObjectMeta {
    digest: String,
    bytes: u64,
    first_reference: String,
}

#[derive(Default)]
struct ObjectTable {
    by_digest: BTreeMap<String, usize>,
    entries: Vec<ObjectMeta>,
    first_order: Vec<usize>,
}

impl ObjectTable {
    fn intern(&mut self, entry: &ObjectEntry, first_reference: &str) -> Result<usize> {
        if let Some(&index) = self.by_digest.get(&entry.ahash64) {
            let previous = &self.entries[index];
            if previous.bytes != entry.bytes {
                bail!("packed object digest has conflicting lengths");
            }
            return Ok(index);
        }

        let index = self.entries.len();
        self.by_digest.insert(entry.ahash64.clone(), index);
        self.entries.push(ObjectMeta {
            digest: entry.ahash64.clone(),
            bytes: entry.bytes,
            first_reference: first_reference.to_owned(),
        });
        self.first_order.push(index);
        Ok(index)
    }

    fn matches(&self, index: usize, entry: &ObjectEntry) -> bool {
        self.entries.get(index).is_some_and(|expected| {
            expected.digest == entry.ahash64 && expected.bytes == entry.bytes
        })
    }
}

fn read_canonical_root(output: &Path) -> Result<ShardedManifestRoot> {
    let root_bytes = fs::read(output.join("manifest.json")).context("read root manifest")?;
    let root_text = std::str::from_utf8(&root_bytes).context("root manifest is not UTF-8")?;
    let root = ShardedManifestRoot::parse(root_text).context("parse root manifest")?;
    if root.to_json().as_bytes() != root_bytes {
        bail!("root manifest is not canonically serialized");
    }
    Ok(root)
}

fn read_validated_shard(
    output: &Path,
    root: &ShardedManifestRoot,
    index: usize,
) -> Result<ValidatedPackedShard> {
    let digest = &root.shards[index];
    let object = format!("ahash64-v1-{digest}");
    let bytes = fs::read(output.join("objects").join(&object))
        .with_context(|| format!("read object for shard {index}"))?;
    if super::ahash64(&bytes) != *digest {
        bail!("object for shard {index} does not match its declared digest");
    }
    ValidatedPackedShard::new(bytes, root, index as u32).context("validate packed index shard")
}

fn verify_shard_records_first_pass(
    output: &Path,
    root: &ShardedManifestRoot,
    keys: &mut BTreeMap<String, KeyMeta>,
    objects: &mut ObjectTable,
) -> Result<()> {
    for index in 0..root.shards.len() {
        let packed = read_validated_shard(output, root, index)?;
        for record in packed.records() {
            let key = record.key();
            let metadata = match record.kind() {
                PackedRecordKind::File => {
                    let file = record.file().expect("validated file record");
                    let object_entry = file.object();
                    let object = objects.intern(&object_entry, key)?;
                    for dependency in file.dependencies() {
                        let dependency_object = dependency.object();
                        objects.intern(&dependency_object, key)?;
                    }
                    KeyMeta::File {
                        virtual_path: file.virtual_path().to_owned(),
                        object,
                    }
                }
                PackedRecordKind::Font => {
                    let font = record
                        .font()
                        .context("decode packed font record")?
                        .expect("validated font record");
                    let object = objects.intern(&font.object, key)?;
                    let license_reference = format!("license for {key}");
                    let license = intern_license(objects, &font.license, &license_reference)?;
                    KeyMeta::Font { object, license }
                }
                PackedRecordKind::LegacyMapping => {
                    let mapping = record
                        .legacy_mapping()
                        .context("decode packed legacy mapping record")?
                        .expect("validated legacy mapping record");
                    let object = objects.intern(&mapping.object, key)?;
                    let license_reference = format!("license for {key}");
                    let license = intern_license(objects, &mapping.license, &license_reference)?;
                    KeyMeta::LegacyMapping {
                        font_key: mapping.font_request.manifest_key().to_string(),
                        object,
                        license,
                    }
                }
            };
            if keys.insert(key.to_owned(), metadata).is_some() {
                bail!("duplicate lookup key across shards");
            }
        }
    }
    Ok(())
}

fn intern_license(
    objects: &mut ObjectTable,
    license: &umber_distribution::LicenseRecord,
    first_reference: &str,
) -> Result<LicenseMeta> {
    let object = objects.intern(&license.object, first_reference)?;
    Ok(LicenseMeta {
        identity: license.identity.clone(),
        object,
        spdx: license.spdx.clone(),
        embeddable: license.embeddable,
        redistributable: license.redistributable,
    })
}

fn verify_root_format_closures(
    root: &ShardedManifestRoot,
    keys: &BTreeMap<String, KeyMeta>,
) -> Result<()> {
    for (name, format) in &root.formats {
        let Some(closure) = &format.input_closure else {
            continue;
        };
        for key in &closure.keys {
            if !matches!(keys.get(key), Some(KeyMeta::File { .. })) {
                bail!("input closure key {key} for format {name} is absent");
            }
        }
    }
    Ok(())
}

fn verify_shard_references_second_pass(
    output: &Path,
    root: &ShardedManifestRoot,
    keys: &BTreeMap<String, KeyMeta>,
    objects: &ObjectTable,
) -> Result<()> {
    for index in 0..root.shards.len() {
        let packed = read_validated_shard(output, root, index)?;
        for record in packed.records() {
            match record.kind() {
                PackedRecordKind::File => {
                    let file = record.file().expect("validated file record");
                    let owner = record.key();
                    let Some(KeyMeta::File { .. }) = keys.get(owner) else {
                        bail!("lookup key {owner} is absent from its first verification pass");
                    };
                    for dependency in file.dependencies() {
                        let dependency_key = dependency.key();
                        let Some(KeyMeta::File {
                            virtual_path,
                            object,
                        }) = keys.get(dependency_key)
                        else {
                            bail!("dependency {dependency_key} from {owner} is absent");
                        };
                        let dependency_object = dependency.object();
                        if dependency.virtual_path() != virtual_path
                            || !objects.matches(*object, &dependency_object)
                        {
                            bail!(
                                "dependency {dependency_key} from {owner} has stale inline metadata"
                            );
                        }
                    }
                }
                PackedRecordKind::Font => {
                    let owner = record.key();
                    if !matches!(keys.get(owner), Some(KeyMeta::Font { .. })) {
                        bail!("lookup key {owner} is absent from its first verification pass");
                    }
                }
                PackedRecordKind::LegacyMapping => {
                    let owner = record.key();
                    let Some(KeyMeta::LegacyMapping {
                        font_key,
                        object: mapping_object,
                        license: mapping_license,
                    }) = keys.get(owner)
                    else {
                        bail!("lookup key {owner} is absent from its first verification pass");
                    };
                    let Some(KeyMeta::Font {
                        object: font_object,
                        license: font_license,
                    }) = keys.get(font_key)
                    else {
                        bail!("legacy mapping {owner} references absent font {font_key}");
                    };
                    if font_object != mapping_object || font_license != mapping_license {
                        bail!(
                            "legacy mapping {owner} does not match its declared font and license objects"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn verify_payload_objects(output: &Path, objects: &ObjectTable) -> Result<()> {
    let mut buffer = [0_u8; 64 * 1024];
    for &index in &objects.first_order {
        let object = &objects.entries[index];
        let path = output
            .join("objects")
            .join(format!("ahash64-v1-{}", object.digest));
        let metadata = fs::metadata(&path)
            .with_context(|| format!("read object for {}", object.first_reference))?;
        if !metadata.is_file() {
            bail!(
                "object for {} is not a regular file",
                object.first_reference
            );
        }
        if metadata.len() != object.bytes {
            bail!(
                "object for {} does not match declared digest and length",
                object.first_reference
            );
        }
        let mut file = File::open(&path)
            .with_context(|| format!("read object for {}", object.first_reference))?;
        let mut hasher = AHash64Hasher::new(HashDomain::DistributionContent);
        let mut actual_bytes = 0_u64;
        loop {
            let read = file
                .read(&mut buffer)
                .with_context(|| format!("read object for {}", object.first_reference))?;
            if read == 0 {
                break;
            }
            actual_bytes = actual_bytes
                .checked_add(read as u64)
                .context("object length overflows")?;
            hasher.write(&buffer[..read]);
        }
        if actual_bytes != object.bytes || hasher.finish().hex() != object.digest {
            bail!(
                "object for {} does not match declared digest and length",
                object.first_reference
            );
        }
    }
    Ok(())
}
