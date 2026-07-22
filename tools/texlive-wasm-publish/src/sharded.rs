use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use umber_distribution::{
    DependencyHint, FORMAT_INPUT_CLOSURE_SCHEMA, FileRequestKey, FontManifestRecord,
    FormatInputClosure, HTML_INDEX_SHARD_SCHEMA, HTML_SHARDED_ROOT_SCHEMA,
    LegacyMappingManifestRecord, MAX_FORMAT_INPUTS, Manifest, ManifestFile, ManifestFormat,
    ManifestShard, ShardFile,
};

pub const ROOT_SCHEMA: u32 = 3;
pub const SHARD_SCHEMA: u32 = 1;
pub const MAX_SHARD_BITS: u8 = 16;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootManifest {
    pub schema: u32,
    pub distribution: String,
    pub objects_base_url: String,
    pub shard_bits: u8,
    pub shard_count: u32,
    pub shards: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub formats: BTreeMap<String, RootFormat>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexShard {
    pub schema: u32,
    pub distribution: String,
    pub index: u32,
    pub files: BTreeMap<String, ShardFile>,
    pub fonts: BTreeMap<String, FontManifestRecord>,
    pub legacy_mappings: BTreeMap<String, LegacyMappingManifestRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchEntry {
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootFormat {
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
    pub engine: String,
    pub engine_version: String,
    pub format_schema: u32,
    pub source_distribution: String,
    pub source_manifest_sha256: String,
    pub source_date_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_closure: Option<RootFormatInputClosure>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootFormatInputClosure {
    pub schema: u32,
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardedPublication {
    pub root: RootManifest,
    pub shards: Vec<IndexShard>,
    pub files: BTreeMap<String, ManifestFile>,
    pub formats: BTreeMap<String, ManifestFormat>,
    pub fonts: BTreeMap<String, FontManifestRecord>,
    pub legacy_mappings: BTreeMap<String, LegacyMappingManifestRecord>,
}

pub fn shard_manifest(manifest: &Manifest, shard_bits: u8) -> Result<ShardedPublication> {
    shard_manifest_records(
        manifest,
        shard_bits,
        ROOT_SCHEMA,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

fn shard_manifest_records(
    manifest: &Manifest,
    shard_bits: u8,
    root_schema: u32,
    fonts: &BTreeMap<String, FontManifestRecord>,
    legacy_mappings: &BTreeMap<String, LegacyMappingManifestRecord>,
) -> Result<ShardedPublication> {
    validate_shard_bits(shard_bits)?;
    let shard_count = 1_usize << shard_bits;
    let mut shard_files = vec![BTreeMap::new(); shard_count];
    for (key, file) in &manifest.files {
        let dependencies = file
            .dependencies
            .iter()
            .map(|dependency| {
                let target = manifest
                    .files
                    .get(dependency)
                    .expect("monolithic manifest validates dependency references");
                DependencyHint {
                    key: dependency.clone(),
                    virtual_path: target.virtual_path.clone(),
                    object: target.object.clone(),
                    sha256: target.sha256.clone(),
                    bytes: target.bytes,
                }
            })
            .collect();
        shard_files[shard_index(key, shard_bits)].insert(
            key.clone(),
            ShardFile {
                virtual_path: file.virtual_path.clone(),
                object: file.object.clone(),
                sha256: file.sha256.clone(),
                bytes: file.bytes,
                dependencies,
            },
        );
    }
    let mut shard_fonts = vec![BTreeMap::new(); shard_count];
    for (key, record) in fonts {
        shard_fonts[shard_index(key, shard_bits)].insert(key.clone(), record.clone());
    }
    let mut shard_mappings = vec![BTreeMap::new(); shard_count];
    for (key, record) in legacy_mappings {
        shard_mappings[shard_index(key, shard_bits)].insert(key.clone(), record.clone());
    }
    let shard_schema = if root_schema == HTML_SHARDED_ROOT_SCHEMA {
        HTML_INDEX_SHARD_SCHEMA
    } else {
        SHARD_SCHEMA
    };
    let shards = shard_files
        .into_iter()
        .zip(shard_fonts)
        .zip(shard_mappings)
        .enumerate()
        .map(|(index, ((files, fonts), legacy_mappings))| IndexShard {
            schema: shard_schema,
            distribution: manifest.distribution.clone(),
            index: u32::try_from(index).expect("bounded shard index fits u32"),
            files,
            fonts,
            legacy_mappings,
        })
        .collect();
    Ok(ShardedPublication {
        root: RootManifest {
            schema: root_schema,
            distribution: manifest.distribution.clone(),
            objects_base_url: manifest.objects_base_url.clone(),
            shard_bits,
            shard_count: u32::try_from(shard_count).context("shard count exceeds u32")?,
            shards: Vec::new(),
            formats: manifest
                .formats
                .iter()
                .map(|(name, format)| (name.clone(), RootFormat::from(format)))
                .collect(),
        },
        shards,
        files: manifest.files.clone(),
        formats: manifest.formats.clone(),
        fonts: fonts.clone(),
        legacy_mappings: legacy_mappings.clone(),
    })
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

fn write_publication(
    mut publication: ShardedPublication,
    output: &Path,
) -> Result<ShardedPublication> {
    let objects = output.join("objects");
    fs::create_dir_all(&objects)
        .with_context(|| format!("create output directory {}", objects.display()))?;
    for shard in &publication.shards {
        let bytes = canonical_shard(shard).into_bytes();
        let digest = sha256(&bytes);
        let object = format!("sha256-{digest}");
        fs::write(objects.join(&object), &bytes)
            .with_context(|| format!("write index shard {object}"))?;
        publication.root.shards.push(digest);
    }
    fs::write(
        output.join("manifest.json"),
        canonical_json(&publication.root)?,
    )
    .context("write root manifest")?;
    Ok(publication)
}

pub fn verify_sharded_snapshot(output: &Path) -> Result<ShardedPublication> {
    let root_bytes = fs::read(output.join("manifest.json")).context("read root manifest")?;
    let root: RootManifest = parse_canonical(&root_bytes, "root manifest")?;
    validate_root(&root)?;
    let mut shards = Vec::with_capacity(root.shards.len());
    let mut files = BTreeMap::new();
    for (index, digest) in root.shards.iter().enumerate() {
        validate_digest(digest, "shard")?;
        let object = format!("sha256-{digest}");
        let bytes = fs::read(output.join("objects").join(&object))
            .with_context(|| format!("read object for shard {index}"))?;
        if sha256(&bytes) != *digest {
            bail!("object for shard {index} does not match its declared digest");
        }
        let text = std::str::from_utf8(&bytes).context("index shard is not UTF-8")?;
        let parsed = ManifestShard::parse(text).context("parse index shard")?;
        if parsed.to_json().as_bytes() != bytes {
            bail!("index shard is not canonically serialized");
        }
        let shard = IndexShard {
            schema: parsed.schema,
            distribution: parsed.distribution,
            index: parsed.index,
            files: parsed.files,
            fonts: parsed.fonts,
            legacy_mappings: parsed.legacy_mappings,
        };
        let expected_shard_schema = if root.schema == HTML_SHARDED_ROOT_SCHEMA {
            HTML_INDEX_SHARD_SCHEMA
        } else {
            SHARD_SCHEMA
        };
        if shard.schema != expected_shard_schema
            || shard.distribution != root.distribution
            || shard.index != index as u32
        {
            bail!("index shard {index} identity does not match root manifest");
        }
        for (key, file) in &shard.files {
            if shard_index(key, root.shard_bits) != index {
                bail!("lookup key {key} is in shard {index}, not its canonical shard");
            }
            validate_shard_file(key, file)?;
            if files.insert(key.clone(), file.clone()).is_some() {
                bail!("duplicate lookup key {key} across shards");
            }
        }
        shards.push(shard);
    }
    for (key, file) in &files {
        read_verified_object(
            output,
            &FetchEntry {
                object: file.object.clone(),
                sha256: file.sha256.clone(),
                bytes: file.bytes,
            },
            key,
        )?;
        let mut previous = None;
        for dependency in &file.dependencies {
            if previous
                .as_ref()
                .is_some_and(|value| value >= &dependency.key)
            {
                bail!("dependencies for {key} are not strictly sorted");
            }
            previous = Some(dependency.key.clone());
            let Some(target) = files.get(&dependency.key) else {
                bail!("dependency {} from {key} is absent", dependency.key);
            };
            if dependency.virtual_path != target.virtual_path
                || dependency.object != target.object
                || dependency.sha256 != target.sha256
                || dependency.bytes != target.bytes
            {
                bail!(
                    "dependency {} from {key} has stale inline metadata",
                    dependency.key
                );
            }
        }
    }
    let formats = root
        .formats
        .iter()
        .map(|(name, format)| {
            validate_fetch_entry(&format.fetch_entry(), name)?;
            read_verified_object(output, &format.fetch_entry(), name)?;
            Ok((name.clone(), ManifestFormat::from(format)))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    for (name, format) in &formats {
        if let Some(closure) = &format.input_closure {
            validate_format_input_closure(name, closure, &files)?;
        }
    }
    let fonts = shards
        .iter()
        .flat_map(|shard| {
            shard
                .fonts
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let legacy_mappings = shards
        .iter()
        .flat_map(|shard| {
            shard
                .legacy_mappings
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    for (key, record) in &fonts {
        read_verified_object_entry(output, &record.object, key)?;
        read_verified_object_entry(
            output,
            &record.license.object,
            &format!("license for {key}"),
        )?;
    }
    for (key, record) in &legacy_mappings {
        read_verified_object_entry(output, &record.object, key)?;
        read_verified_object_entry(
            output,
            &record.license.object,
            &format!("license for {key}"),
        )?;
        let font_key = record.font_request.manifest_key().to_string();
        let font = fonts
            .get(&font_key)
            .with_context(|| format!("legacy mapping {key} references absent font {font_key}"))?;
        if font.object != record.object || font.license != record.license {
            bail!("legacy mapping {key} does not match its declared font and license objects");
        }
    }
    Ok(ShardedPublication {
        root,
        shards,
        files: files
            .into_iter()
            .map(|(key, file)| {
                let dependencies = file
                    .dependencies
                    .iter()
                    .map(|hint| hint.key.clone())
                    .collect();
                (
                    key,
                    ManifestFile {
                        virtual_path: file.virtual_path,
                        object: file.object,
                        sha256: file.sha256,
                        bytes: file.bytes,
                        dependencies,
                    },
                )
            })
            .collect(),
        formats,
        fonts,
        legacy_mappings,
    })
}

pub fn shard_index(key: &str, shard_bits: u8) -> usize {
    debug_assert!(shard_bits <= MAX_SHARD_BITS);
    if shard_bits == 0 {
        return 0;
    }
    let digest = Sha256::digest(key.as_bytes());
    let prefix = u16::from_be_bytes([digest[0], digest[1]]);
    usize::from(prefix >> (16 - shard_bits))
}

fn validate_root(root: &RootManifest) -> Result<()> {
    validate_shard_bits(root.shard_bits)?;
    let expected = 1_u32 << root.shard_bits;
    if !matches!(root.schema, ROOT_SCHEMA | HTML_SHARDED_ROOT_SCHEMA)
        || root.shard_count != expected
        || root.shards.len() != expected as usize
    {
        bail!("root manifest shard metadata is inconsistent");
    }
    if root.shards.iter().collect::<BTreeSet<_>>().len() != root.shards.len() {
        bail!("root manifest contains duplicate shard digests");
    }
    Ok(())
}

fn validate_shard_bits(bits: u8) -> Result<()> {
    if bits > MAX_SHARD_BITS {
        bail!("shardBits must be between 0 and {MAX_SHARD_BITS}");
    }
    Ok(())
}

fn validate_shard_file(key: &str, file: &ShardFile) -> Result<()> {
    if !matches!(key.split_once(':'), Some(("tex" | "tfm", name)) if !name.is_empty()) {
        bail!("invalid lookup key {key}");
    }
    validate_fetch_entry(
        &FetchEntry {
            object: file.object.clone(),
            sha256: file.sha256.clone(),
            bytes: file.bytes,
        },
        key,
    )
}

fn validate_fetch_entry(entry: &FetchEntry, label: &str) -> Result<()> {
    validate_digest(&entry.sha256, label)?;
    if entry.object != format!("sha256-{}", entry.sha256) {
        bail!("invalid content-addressed object metadata for {label}");
    }
    Ok(())
}

fn validate_format_input_closure(
    format_name: &str,
    closure: &FormatInputClosure,
    files: &BTreeMap<String, ShardFile>,
) -> Result<()> {
    if closure.schema != FORMAT_INPUT_CLOSURE_SCHEMA
        || closure.keys.is_empty()
        || closure.keys.len() > MAX_FORMAT_INPUTS
    {
        bail!("invalid input closure metadata for format {format_name}");
    }
    let mut previous: Option<&str> = None;
    for key in &closure.keys {
        FileRequestKey::from_manifest_key(key).with_context(|| {
            format!("invalid input closure key {key:?} for format {format_name}")
        })?;
        if previous.is_some_and(|value| value >= key) {
            bail!("input closure keys for format {format_name} are not strictly sorted");
        }
        if !files.contains_key(key) {
            bail!("input closure key {key} for format {format_name} is absent");
        }
        previous = Some(key.as_str());
    }
    Ok(())
}

fn validate_digest(digest: &str, label: &str) -> Result<()> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        bail!("invalid SHA-256 digest for {label}");
    }
    Ok(())
}

fn read_verified_object(output: &Path, entry: &FetchEntry, label: &str) -> Result<Vec<u8>> {
    let bytes = fs::read(output.join("objects").join(&entry.object))
        .with_context(|| format!("read object for {label}"))?;
    if bytes.len() as u64 != entry.bytes || sha256(&bytes) != entry.sha256 {
        bail!("object for {label} does not match declared digest and length");
    }
    Ok(bytes)
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).context("serialize canonical JSON")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_shard(shard: &IndexShard) -> String {
    ManifestShard {
        schema: shard.schema,
        distribution: shard.distribution.clone(),
        index: shard.index,
        files: shard.files.clone(),
        fonts: shard.fonts.clone(),
        legacy_mappings: shard.legacy_mappings.clone(),
    }
    .to_json()
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
            sha256: entry.sha256.clone(),
            bytes: entry.bytes,
        },
        label,
    )
}

fn parse_canonical<T>(bytes: &[u8], label: &str) -> Result<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let parsed = serde_json::from_slice(bytes).with_context(|| format!("parse {label}"))?;
    if canonical_json(&parsed)? != bytes {
        bail!("{label} is not canonically serialized");
    }
    Ok(parsed)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl From<&ManifestFormat> for RootFormat {
    fn from(value: &ManifestFormat) -> Self {
        Self {
            object: value.object.clone(),
            sha256: value.sha256.clone(),
            bytes: value.bytes,
            engine: value.engine.clone(),
            engine_version: value.engine_version.clone(),
            format_schema: value.format_schema,
            source_distribution: value.source_distribution.clone(),
            source_manifest_sha256: value.source_manifest_sha256.clone(),
            source_date_epoch: value.source_date_epoch,
            input_closure: value
                .input_closure
                .as_ref()
                .map(|closure| RootFormatInputClosure {
                    schema: closure.schema,
                    keys: closure.keys.clone(),
                }),
        }
    }
}

impl From<&RootFormat> for ManifestFormat {
    fn from(value: &RootFormat) -> Self {
        Self {
            object: value.object.clone(),
            sha256: value.sha256.clone(),
            bytes: value.bytes,
            engine: value.engine.clone(),
            engine_version: value.engine_version.clone(),
            format_schema: value.format_schema,
            source_distribution: value.source_distribution.clone(),
            source_manifest_sha256: value.source_manifest_sha256.clone(),
            source_date_epoch: value.source_date_epoch,
            input_closure: value
                .input_closure
                .as_ref()
                .map(|closure| FormatInputClosure {
                    schema: closure.schema,
                    keys: closure.keys.clone(),
                }),
        }
    }
}

impl RootFormat {
    fn fetch_entry(&self) -> FetchEntry {
        FetchEntry {
            object: self.object.clone(),
            sha256: self.sha256.clone(),
            bytes: self.bytes,
        }
    }
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
                .map(|digest| format!("sha256-{digest}")),
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
