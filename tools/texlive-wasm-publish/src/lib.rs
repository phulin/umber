//! Deterministic publisher for a pinned browser TeX Live subset.

#![allow(clippy::disallowed_methods)] // Host release tooling intentionally owns filesystem I/O.

mod scan;
mod sharded;
mod tlpdb;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use umber_distribution::{
    FileRequestKey, FontManifestRecord, HTML_INDEX_SHARD_SCHEMA, HTML_SHARDED_ROOT_SCHEMA,
    LegacyMappingManifestRecord, ManifestFile, ManifestFormat, ManifestShard, NamedFormat,
};
use umber_hash::{AHash64, HashDomain};

pub use sharded::{
    ShardedPublication, prune_unreferenced_objects, read_sharded_catalog, shard_index,
    verify_sharded_snapshot, write_html_sharded_manifest, write_sharded_manifest,
};

pub use scan::tree_ahash64;
use scan::{Candidate, scan_roots};
use tlpdb::PackageDatabase;
pub use umber_distribution::Manifest;

/// Hash one published object with the repository-owned content-identity domain.
pub fn file_ahash64(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(AHash64::for_bytes(HashDomain::DistributionContent, &bytes).hex())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishConfig {
    pub schema: u32,
    pub distribution: String,
    pub objects_base_url: String,
    pub shard_bits: u8,
    pub roots: Vec<RootConfig>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub formats: Vec<FormatConfig>,
    #[serde(default)]
    pub package_database: Option<PathBuf>,
    #[serde(default)]
    pub inventory: Option<InventoryConfig>,
    #[serde(default)]
    pub profile: PublicationProfile,
    #[serde(default)]
    pub html: Option<HtmlProfileConfig>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PublicationProfile {
    #[default]
    Full,
    Html,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HtmlProfileConfig {
    #[serde(default)]
    pub runtime_file_keys: Vec<String>,
    pub catalog: PathBuf,
    pub object_sources: BTreeMap<String, PathBuf>,
    pub inventory: HtmlInventoryConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HtmlInventoryConfig {
    pub maximum_logical_files: usize,
    pub maximum_objects: usize,
    pub maximum_bytes: u64,
    pub maximum_fonts: usize,
    pub maximum_legacy_mappings: usize,
    pub maximum_licenses: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryConfig {
    pub minimum_logical_files: usize,
    pub minimum_objects: usize,
    pub minimum_bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootConfig {
    pub name: String,
    pub path: PathBuf,
    pub tree_ahash64: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormatConfig {
    pub path: PathBuf,
    pub metadata: PathBuf,
    #[serde(default)]
    pub input_identities: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FormatInputIdentities {
    schema: u32,
    inputs: Vec<FormatInputIdentity>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FormatInputIdentity {
    key: String,
    ahash64: String,
    bytes: u64,
}

struct PreparedPublication {
    manifest: Manifest,
    objects: BTreeMap<String, Vec<u8>>,
    fonts: BTreeMap<String, FontManifestRecord>,
    legacy_mappings: BTreeMap<String, LegacyMappingManifestRecord>,
}

pub fn publish(config: &PublishConfig, output: &Path) -> Result<ShardedPublication> {
    validate_config(config)?;
    let prepared = match config.profile {
        PublicationProfile::Full => prepare_full(config)?,
        PublicationProfile::Html => prepare_html(config)?,
    };
    let objects = output.join("objects");
    fs::create_dir_all(&objects)
        .with_context(|| format!("create output directory {}", objects.display()))?;
    for (object, bytes) in &prepared.objects {
        fs::write(objects.join(object), bytes)
            .with_context(|| format!("write prepared object {object}"))?;
    }
    let publication = if config.profile == PublicationProfile::Html {
        write_html_sharded_manifest(
            &prepared.manifest,
            config.shard_bits,
            output,
            &prepared.fonts,
            &prepared.legacy_mappings,
        )?
    } else {
        write_sharded_manifest(&prepared.manifest, config.shard_bits, output)?
    };
    remove_stale_objects(&objects, &sharded::referenced_objects(&publication))?;
    if let Some(html) = &config.html {
        validate_html_inventory(&html.inventory, output, &publication)?;
    }
    verify_sharded_snapshot(output).context("verify staged sharded snapshot")
}

/// Publish a sparse successor to an verified complete sharded catalog.
///
/// The base directory must contain the canonical root and every verified
/// shard, but need not duplicate unchanged content-addressed payloads. Roots
/// in `config` are an ordered overlay and may replace or add logical keys. The
/// output contains every successor shard plus exactly the changed payload and
/// format objects.
pub fn publish_successor(
    base: &Path,
    base_ahash64: &str,
    config: &PublishConfig,
    output: &Path,
) -> Result<ShardedPublication> {
    validate_config(config)?;
    if config.profile != PublicationProfile::Full {
        bail!("sparse successors support only the full publication profile");
    }
    if config.package_database.is_some() || !config.dependencies.is_empty() {
        bail!("sparse successors preserve the base dependency graph");
    }
    verify_base_root(base, base_ahash64)?;
    let base = read_sharded_catalog(base).context("verify successor base catalog")?;
    if config.distribution != base.root.distribution
        || config.objects_base_url != base.root.objects_base_url
        || config.shard_bits != base.root.shard_bits
    {
        bail!("successor distribution, object base URL, and shard policy must match the base");
    }
    let replacements = flatten_candidates(scan_roots(&config.roots)?)?;
    let mut files = base.files.clone();
    let mut winners = base
        .files
        .iter()
        .map(|(key, file)| {
            (
                key.clone(),
                Candidate {
                    kind: if key.starts_with("tfm:") {
                        "tfm"
                    } else {
                        "tex"
                    },
                    relative: file
                        .virtual_path
                        .strip_prefix("/texlive/")
                        .unwrap_or(&file.virtual_path)
                        .to_owned(),
                    source: PathBuf::new(),
                    ahash64: file.ahash64.clone(),
                    bytes: file.bytes,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut objects = BTreeMap::new();
    for (key, replacement) in replacements {
        let dependencies = files
            .get(&key)
            .map(|prior| prior.dependencies.clone())
            .unwrap_or_default();
        let bytes = fs::read(&replacement.source)
            .with_context(|| format!("read successor object {}", replacement.source.display()))?;
        let object = format!("ahash64-v1-{}", replacement.ahash64);
        files.insert(
            key.clone(),
            ManifestFile {
                virtual_path: format!("/texlive/{}", replacement.relative),
                object: object.clone(),
                ahash64: replacement.ahash64.clone(),
                bytes: replacement.bytes,
                dependencies,
            },
        );
        winners.insert(key, replacement);
        objects.entry(object).or_insert(bytes);
    }

    let mut formats = BTreeMap::new();
    for format in &config.formats {
        let (name, metadata, bytes) = load_format(format, &winners)?;
        if formats.insert(name.clone(), metadata.clone()).is_some() {
            bail!("duplicate published format name {name:?}");
        }
        objects.entry(metadata.object).or_insert(bytes);
    }
    if formats.keys().collect::<Vec<_>>() != base.formats.keys().collect::<Vec<_>>() {
        bail!("sparse successor must replace exactly the base format names");
    }
    let manifest = Manifest {
        schema: umber_distribution::MANIFEST_SCHEMA,
        distribution: config.distribution.clone(),
        objects_base_url: config.objects_base_url.clone(),
        files,
        fonts: BTreeMap::new(),
        formats,
    };
    let publication = write_sharded_manifest(&manifest, config.shard_bits, output)?;
    let referenced_payloads = objects.keys().cloned().collect::<BTreeSet<_>>();
    let object_dir = output.join("objects");
    for (object, bytes) in objects {
        fs::write(object_dir.join(&object), bytes)
            .with_context(|| format!("write successor payload {object}"))?;
    }
    verify_sparse_successor(&base, &publication, output, &referenced_payloads)?;
    Ok(publication)
}

/// Verify a sparse successor staging directory against its verified base.
pub fn verify_successor(
    base: &Path,
    base_ahash64: &str,
    output: &Path,
) -> Result<ShardedPublication> {
    verify_base_root(base, base_ahash64)?;
    let base = read_sharded_catalog(base).context("verify successor base catalog")?;
    let successor = read_sharded_catalog(output).context("verify successor catalog")?;
    if successor.root.distribution != base.root.distribution
        || successor.root.objects_base_url != base.root.objects_base_url
        || successor.root.shard_bits != base.root.shard_bits
    {
        bail!("successor distribution, object base URL, and shard policy differ from the base");
    }
    if !base
        .files
        .keys()
        .all(|key| successor.files.contains_key(key))
    {
        bail!("successor removed a base logical key");
    }
    if successor.formats.keys().collect::<Vec<_>>() != base.formats.keys().collect::<Vec<_>>() {
        bail!("successor must replace exactly the base format names");
    }
    let changed_payloads = successor
        .files
        .iter()
        .filter(|(key, file)| base.files.get(*key) != Some(*file))
        .map(|(_, file)| file.object.clone())
        .chain(
            successor
                .formats
                .values()
                .map(|format| format.object.clone()),
        )
        .collect::<BTreeSet<_>>();
    verify_sparse_successor(&base, &successor, output, &changed_payloads)?;
    Ok(successor)
}

fn verify_base_root(base: &Path, expected: &str) -> Result<()> {
    if expected.len() != 16
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        || expected.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        bail!("successor base aHash64 must contain 16 lowercase hexadecimal characters");
    }
    let bytes = fs::read(base.join("manifest.json")).context("read successor base root")?;
    if distribution_ahash64(&bytes) != expected {
        bail!("successor base root does not match its pinned aHash64");
    }
    Ok(())
}

fn verify_sparse_successor(
    base: &ShardedPublication,
    successor: &ShardedPublication,
    output: &Path,
    changed_payloads: &BTreeSet<String>,
) -> Result<()> {
    let reread = read_sharded_catalog(output).context("verify successor root and shards")?;
    if &reread != successor {
        bail!("successor catalog changed after serialization");
    }
    for (key, file) in &successor.files {
        if base.files.get(key).is_some_and(|prior| file == prior) {
            continue;
        }
        if !changed_payloads.contains(&file.object) {
            bail!("changed successor key {key:?} has no staged payload");
        }
        read_verified_successor_object(output, &file.object_entry(), key)?;
    }
    for (name, format) in &successor.formats {
        if !changed_payloads.contains(&format.object) {
            bail!("successor format {name:?} has no staged payload");
        }
        read_verified_successor_object(output, &format.object_entry(), name)?;
    }
    Ok(())
}

fn read_verified_successor_object(
    output: &Path,
    entry: &umber_distribution::ObjectEntry,
    label: &str,
) -> Result<()> {
    let bytes = fs::read(output.join("objects").join(&entry.object))
        .with_context(|| format!("read successor object for {label}"))?;
    if bytes.len() as u64 != entry.bytes || distribution_ahash64(&bytes) != entry.ahash64 {
        bail!("successor object for {label} does not match declared digest and length");
    }
    Ok(())
}

fn prepare_full(config: &PublishConfig) -> Result<PreparedPublication> {
    let candidates = scan_roots(&config.roots)?;
    let winners = flatten_candidates(candidates)?;
    let dependencies = publication_dependencies(config, &winners)?;
    validate_dependencies(&dependencies, &winners)?;

    let mut objects = BTreeMap::new();
    let mut formats = BTreeMap::new();
    for format in &config.formats {
        let (name, manifest_format, bytes) = load_format(format, &winners)?;
        if formats
            .insert(name.clone(), manifest_format.clone())
            .is_some()
        {
            bail!("duplicate published format name {name:?}");
        }
        objects.entry(manifest_format.object).or_insert(bytes);
    }

    let mut files = BTreeMap::new();
    for (key, candidate) in winners {
        let bytes = fs::read(&candidate.source)
            .with_context(|| format!("read {}", candidate.source.display()))?;
        let object = format!("ahash64-v1-{}", candidate.ahash64);
        objects.entry(object.clone()).or_insert(bytes.clone());
        files.insert(
            key.clone(),
            ManifestFile {
                virtual_path: format!("/texlive/{}", candidate.relative),
                object,
                ahash64: candidate.ahash64,
                bytes: u64::try_from(bytes.len()).context("file length exceeds u64")?,
                dependencies: dependencies.get(&key).cloned().unwrap_or_default(),
            },
        );
    }
    let published_bytes = objects.values().try_fold(0_u64, |total, bytes| {
        total
            .checked_add(u64::try_from(bytes.len()).context("object length exceeds u64")?)
            .context("publication byte count overflow")
    })?;
    validate_inventory(
        config.inventory.as_ref(),
        files.len(),
        objects.len(),
        published_bytes,
    )?;
    Ok(PreparedPublication {
        manifest: publication_manifest(config, files, formats),
        objects,
        fonts: BTreeMap::new(),
        legacy_mappings: BTreeMap::new(),
    })
}

fn prepare_html(config: &PublishConfig) -> Result<PreparedPublication> {
    let html = config
        .html
        .as_ref()
        .context("HTML publication profile requires html configuration")?;
    let candidates = scan_roots(&config.roots)?;
    let mut winners = flatten_candidates(candidates)?;

    let mut formats = BTreeMap::new();
    let mut objects = BTreeMap::new();
    let mut selected_keys = html
        .runtime_file_keys
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if selected_keys.len() != html.runtime_file_keys.len() {
        bail!("HTML runtimeFileKeys contains duplicate keys");
    }
    for format in &config.formats {
        let (name, manifest_format, bytes) = load_format(format, &winners)?;
        let closure = manifest_format.input_closure.as_ref().with_context(|| {
            format!("HTML format {name:?} must carry an verified input closure")
        })?;
        selected_keys.extend(closure.keys.iter().cloned());
        if formats
            .insert(name.clone(), manifest_format.clone())
            .is_some()
        {
            bail!("duplicate published format name {name:?}");
        }
        objects
            .entry(manifest_format.object.clone())
            .or_insert(bytes);
    }
    if formats.is_empty() {
        bail!("HTML publication profile requires at least one selected format");
    }

    let dependencies = publication_dependencies(config, &winners)?;
    let mut selected = BTreeMap::new();
    for key in selected_keys {
        FileRequestKey::from_manifest_key(&key)
            .with_context(|| format!("invalid HTML runtime file key {key:?}"))?;
        let candidate = winners.remove(&key).with_context(|| {
            format!("HTML runtime file key {key:?} is absent from pinned roots")
        })?;
        validate_html_candidate(&key, &candidate)?;
        selected.insert(key, candidate);
    }
    let selected_dependencies = dependencies
        .into_iter()
        .filter(|(owner, _)| selected.contains_key(owner))
        .map(|(owner, hints)| {
            let hints = hints
                .into_iter()
                .filter(|hint| selected.contains_key(hint))
                .collect();
            (owner, hints)
        })
        .collect::<BTreeMap<_, _>>();
    validate_dependencies(&selected_dependencies, &selected)?;

    let catalog_text = fs::read_to_string(&html.catalog)
        .with_context(|| format!("read HTML catalog {}", html.catalog.display()))?;
    let catalog = ManifestShard::parse(&catalog_text).context("parse HTML catalog")?;
    if catalog.schema != HTML_INDEX_SHARD_SCHEMA
        || catalog.distribution != config.distribution
        || catalog.index != 0
        || !catalog.files.is_empty()
    {
        bail!("HTML catalog must be a schema-2, file-free shard zero for this distribution");
    }
    validate_html_catalog(&catalog.fonts, &catalog.legacy_mappings, &selected)?;

    let mut files = BTreeMap::new();
    for (key, candidate) in selected {
        let bytes = fs::read(&candidate.source)
            .with_context(|| format!("read {}", candidate.source.display()))?;
        let object = format!("ahash64-v1-{}", candidate.ahash64);
        objects.entry(object.clone()).or_insert(bytes.clone());
        files.insert(
            key.clone(),
            ManifestFile {
                virtual_path: format!("/texlive/{}", candidate.relative),
                object,
                ahash64: candidate.ahash64,
                bytes: u64::try_from(bytes.len()).context("file length exceeds u64")?,
                dependencies: selected_dependencies.get(&key).cloned().unwrap_or_default(),
            },
        );
    }
    prepare_html_catalog_objects(html, &catalog, &mut objects)?;
    Ok(PreparedPublication {
        manifest: publication_manifest(config, files, formats),
        objects,
        fonts: catalog.fonts,
        legacy_mappings: catalog.legacy_mappings,
    })
}

fn publication_manifest(
    config: &PublishConfig,
    files: BTreeMap<String, ManifestFile>,
    formats: BTreeMap<String, ManifestFormat>,
) -> Manifest {
    Manifest {
        schema: umber_distribution::MANIFEST_SCHEMA,
        distribution: config.distribution.clone(),
        objects_base_url: config.objects_base_url.clone(),
        files,
        fonts: BTreeMap::new(),
        formats,
    }
}

fn validate_html_candidate(key: &str, candidate: &Candidate) -> Result<()> {
    let allowed = candidate.relative.starts_with("tex/")
        || (candidate.relative.starts_with("fonts/tfm/")
            && candidate.relative.to_ascii_lowercase().ends_with(".tfm"));
    if !allowed {
        bail!(
            "HTML profile rejects PDF/DVI-only runtime class for {key}: {}",
            candidate.relative
        );
    }
    Ok(())
}

fn validate_html_catalog(
    fonts: &BTreeMap<String, FontManifestRecord>,
    mappings: &BTreeMap<String, LegacyMappingManifestRecord>,
    files: &BTreeMap<String, Candidate>,
) -> Result<()> {
    if fonts.is_empty() || mappings.is_empty() {
        bail!("HTML catalog must declare font and legacy mapping records");
    }
    for (key, mapping) in mappings {
        if !files.iter().any(|(file_key, candidate)| {
            file_key.starts_with("tfm:") && candidate.ahash64 == mapping.request.tfm_ahash64()
        }) {
            bail!("legacy mapping {key} does not reference a selected exact TFM object");
        }
        let font_key = mapping.font_request.manifest_key().to_string();
        let font = fonts
            .get(&font_key)
            .with_context(|| format!("legacy mapping {key} references absent font {font_key}"))?;
        if font.object != mapping.object || font.license != mapping.license {
            bail!("legacy mapping {key} does not match its font and license objects");
        }
    }
    Ok(())
}

fn prepare_html_catalog_objects(
    html: &HtmlProfileConfig,
    catalog: &ManifestShard,
    objects: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let expected = catalog
        .fonts
        .values()
        .flat_map(|record| [&record.object, &record.license.object])
        .chain(
            catalog
                .legacy_mappings
                .values()
                .flat_map(|record| [&record.object, &record.license.object]),
        )
        .map(|entry| (entry.ahash64.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    if html.object_sources.keys().collect::<BTreeSet<_>>() != expected.keys().collect() {
        bail!("HTML objectSources must exactly cover the catalog font and license digests");
    }
    for (digest, entry) in expected {
        let source = &html.object_sources[&digest];
        let bytes = fs::read(source)
            .with_context(|| format!("read HTML catalog object {}", source.display()))?;
        if distribution_ahash64(&bytes) != digest
            || bytes.len() as u64 != entry.bytes
            || entry.object != format!("ahash64-v1-{digest}")
        {
            bail!("HTML catalog object {digest} does not match its declared digest and length");
        }
        objects.entry(entry.object).or_insert(bytes);
    }
    Ok(())
}

fn validate_html_inventory(
    limits: &HtmlInventoryConfig,
    output: &Path,
    publication: &ShardedPublication,
) -> Result<()> {
    let mut objects = 0_usize;
    let mut bytes = 0_u64;
    for entry in fs::read_dir(output.join("objects")).context("read HTML object inventory")? {
        let entry = entry.context("read HTML object inventory entry")?;
        let metadata = entry
            .metadata()
            .context("inspect HTML object inventory entry")?;
        if metadata.is_file() {
            objects += 1;
            bytes = bytes
                .checked_add(metadata.len())
                .context("HTML object inventory byte count overflow")?;
        }
    }
    let licenses = publication
        .fonts
        .values()
        .map(|record| &record.license.identity)
        .chain(
            publication
                .legacy_mappings
                .values()
                .map(|record| &record.license.identity),
        )
        .collect::<BTreeSet<_>>()
        .len();
    if publication.files.len() > limits.maximum_logical_files
        || objects > limits.maximum_objects
        || bytes > limits.maximum_bytes
        || publication.fonts.len() > limits.maximum_fonts
        || publication.legacy_mappings.len() > limits.maximum_legacy_mappings
        || licenses > limits.maximum_licenses
    {
        bail!(
            "HTML publication inventory exceeds ceiling: files {} (max {}), objects {} (max {}), bytes {} (max {}), fonts {} (max {}), mappings {} (max {}), licenses {} (max {})",
            publication.files.len(),
            limits.maximum_logical_files,
            objects,
            limits.maximum_objects,
            bytes,
            limits.maximum_bytes,
            publication.fonts.len(),
            limits.maximum_fonts,
            publication.legacy_mappings.len(),
            limits.maximum_legacy_mappings,
            licenses,
            limits.maximum_licenses,
        );
    }
    Ok(())
}

fn publication_dependencies(
    config: &PublishConfig,
    files: &BTreeMap<String, Candidate>,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut dependencies = if let Some(path) = &config.package_database {
        let text = fs::read_to_string(path)
            .with_context(|| format!("read TeX Live package database {}", path.display()))?;
        PackageDatabase::parse(&text)?.hints(files)
    } else {
        BTreeMap::new()
    };
    for (owner, hints) in &config.dependencies {
        let entry = dependencies.entry(owner.clone()).or_default();
        entry.extend(hints.iter().cloned());
        entry.sort();
        entry.dedup();
    }
    Ok(dependencies)
}

fn validate_inventory(
    expected: Option<&InventoryConfig>,
    logical_files: usize,
    objects: usize,
    bytes: u64,
) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if logical_files < expected.minimum_logical_files
        || objects < expected.minimum_objects
        || bytes < expected.minimum_bytes
    {
        bail!(
            "publication inventory is incomplete: logical files {logical_files} (minimum {}), objects {objects} (minimum {}), bytes {bytes} (minimum {})",
            expected.minimum_logical_files,
            expected.minimum_objects,
            expected.minimum_bytes
        );
    }
    Ok(())
}

fn load_format(
    config: &FormatConfig,
    winners: &BTreeMap<String, Candidate>,
) -> Result<(String, ManifestFormat, Vec<u8>)> {
    let metadata_text = fs::read_to_string(&config.metadata)
        .with_context(|| format!("read format metadata {}", config.metadata.display()))?;
    let named = NamedFormat::parse(&metadata_text).context("parse format metadata")?;
    let metadata = named.format;
    let bytes = fs::read(&config.path)
        .with_context(|| format!("read format image {}", config.path.display()))?;
    let actual = distribution_ahash64(&bytes);
    if actual != metadata.ahash64 || bytes.len() as u64 != metadata.bytes {
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
    if let Some(closure) = &metadata.input_closure {
        validate_format_input_identities(&named.name, config, &closure.keys, winners)?;
    } else if config.input_identities.is_some() {
        bail!(
            "format {:?} supplies input identities without an verified input closure",
            named.name
        );
    }
    Ok((named.name, metadata, bytes))
}

fn distribution_ahash64(bytes: &[u8]) -> String {
    AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex()
}

fn validate_format_input_identities(
    format_name: &str,
    config: &FormatConfig,
    closure_keys: &[String],
    winners: &BTreeMap<String, Candidate>,
) -> Result<()> {
    let path = config.input_identities.as_ref().with_context(|| {
        format!("format {format_name:?} must pin the identities of its construction inputs")
    })?;
    let text = fs::read_to_string(path)
        .with_context(|| format!("read format input identities {}", path.display()))?;
    let receipt: FormatInputIdentities = serde_json::from_str(&text)
        .with_context(|| format!("parse format input identities {}", path.display()))?;
    if receipt.schema != 1 {
        bail!(
            "unsupported format input identity schema {}; expected 1",
            receipt.schema
        );
    }
    let mut identities = BTreeMap::new();
    for input in receipt.inputs {
        let key = input.key.clone();
        FileRequestKey::from_manifest_key(&input.key)
            .with_context(|| format!("invalid format input identity key {:?}", input.key))?;
        if input.ahash64.len() != 16
            || !input.ahash64.bytes().all(|byte| byte.is_ascii_hexdigit())
            || input.ahash64.bytes().any(|byte| byte.is_ascii_uppercase())
        {
            bail!(
                "format input identity {:?} must use a lowercase aHash64 digest",
                input.key
            );
        }
        if identities.insert(key.clone(), input).is_some() {
            bail!("duplicate format input identity key {key:?}");
        }
    }
    let identity_keys = identities.keys().cloned().collect::<Vec<_>>();
    if identity_keys != closure_keys {
        bail!(
            "format {format_name:?} input identity keys do not exactly match its verified closure"
        );
    }
    for (key, expected) in identities {
        let winner = winners.get(&key).with_context(|| {
            format!("format {format_name:?} input {key:?} is absent from pinned roots")
        })?;
        if winner.ahash64 != expected.ahash64 || winner.bytes != expected.bytes {
            bail!(
                "format {format_name:?} was constructed from {key:?} ahash64={} bytes={}, but the published runtime winner {} has ahash64={} bytes={}",
                expected.ahash64,
                expected.bytes,
                winner.relative,
                winner.ahash64,
                winner.bytes
            );
        }
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
    let expected_schema = match config.profile {
        PublicationProfile::Full => sharded::ROOT_SCHEMA,
        PublicationProfile::Html => HTML_SHARDED_ROOT_SCHEMA,
    };
    if config.schema != expected_schema {
        bail!(
            "unsupported root manifest schema {}; expected {}",
            config.schema,
            expected_schema
        );
    }
    if config.profile == PublicationProfile::Full && config.html.is_some() {
        bail!("full publication profile cannot contain html configuration");
    }
    if config.profile == PublicationProfile::Html && config.inventory.is_some() {
        bail!("HTML publication uses its independent html.inventory ceilings");
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
    for candidate in candidates {
        for name in candidate.logical_names() {
            let key = format!("{}:{name}", candidate.kind);
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
