//! Deterministic publisher for a pinned browser TeX Live subset.

#![allow(clippy::disallowed_methods)] // Host release tooling intentionally owns filesystem I/O.

mod mvp_catalog;
mod scan;
mod sharded;
mod tlpdb;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use umber_distribution::{
    FORMAT_INPUT_CLOSURE_SCHEMA, FileRequestKey, FontManifestRecord, FormatInputClosure,
    HTML_INDEX_SHARD_SCHEMA, HTML_SHARDED_ROOT_SCHEMA, LegacyMappingManifestRecord,
    MAX_FORMAT_INPUTS, ManifestFile, ManifestFormat, ManifestShard,
};

pub use sharded::{
    IndexShard, RootManifest, ShardedPublication, prune_unreferenced_objects, shard_index,
    verify_sharded_snapshot, write_html_sharded_manifest, write_sharded_manifest,
};

pub use mvp_catalog::write_html_mvp_catalog;
pub use scan::tree_sha256;
use scan::{Candidate, scan_roots};
use tlpdb::PackageDatabase;
pub use umber_distribution::Manifest;

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
    #[serde(default)]
    input_closure: Option<FormatInputClosureMetadata>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FormatInputClosureMetadata {
    schema: u32,
    keys: Vec<String>,
}

pub fn publish(config: &PublishConfig, output: &Path) -> Result<ShardedPublication> {
    validate_config(config)?;
    if config.profile == PublicationProfile::Html {
        return publish_html(config, output);
    }
    let candidates = scan_roots(&config.roots)?;
    let winners = flatten_candidates(candidates)?;
    let dependencies = publication_dependencies(config, &winners)?;
    validate_dependencies(&dependencies, &winners)?;

    let objects = output.join("objects");
    fs::create_dir_all(&objects)
        .with_context(|| format!("create output directory {}", objects.display()))?;

    let mut files = BTreeMap::new();
    let mut expected_objects = BTreeSet::new();
    let mut published_bytes = 0_u64;
    for (key, candidate) in winners {
        let bytes = fs::read(&candidate.source)
            .with_context(|| format!("read {}", candidate.source.display()))?;
        let object = format!("sha256-{}", candidate.sha256);
        let object_path = objects.join(&object);
        fs::write(&object_path, &bytes)
            .with_context(|| format!("write {}", object_path.display()))?;
        if expected_objects.insert(object.clone()) {
            published_bytes += u64::try_from(bytes.len()).context("file length exceeds u64")?;
        }
        files.insert(
            key.clone(),
            ManifestFile {
                virtual_path: format!("/texlive/{}", candidate.relative),
                object,
                sha256: candidate.sha256,
                bytes: u64::try_from(bytes.len()).context("file length exceeds u64")?,
                dependencies: dependencies.get(&key).cloned().unwrap_or_default(),
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
        fs::write(objects.join(&manifest_format.object), &bytes)
            .with_context(|| format!("write format object {}", manifest_format.object))?;
        if expected_objects.insert(manifest_format.object) {
            published_bytes += u64::try_from(bytes.len()).context("format length exceeds u64")?;
        }
    }
    validate_inventory(
        config.inventory.as_ref(),
        files.len(),
        expected_objects.len(),
        published_bytes,
    )?;
    remove_stale_objects(&objects, &expected_objects)?;

    let manifest = Manifest {
        schema: umber_distribution::MANIFEST_SCHEMA,
        distribution: config.distribution.clone(),
        objects_base_url: config.objects_base_url.clone(),
        files,
        fonts: BTreeMap::new(),
        formats,
    };
    let encoded = manifest.to_json_pretty();
    Manifest::parse(&encoded).context("validate publication entries")?;
    let publication = sharded::write_sharded_manifest(&manifest, config.shard_bits, output)?;
    remove_stale_objects(&objects, &sharded::referenced_objects(&publication))?;
    sharded::verify_sharded_snapshot(output).context("verify staged sharded snapshot")
}

fn publish_html(config: &PublishConfig, output: &Path) -> Result<ShardedPublication> {
    let html = config
        .html
        .as_ref()
        .context("HTML publication profile requires html configuration")?;
    let candidates = scan_roots(&config.roots)?;
    let mut winners = flatten_candidates(candidates)?;

    let mut formats = BTreeMap::new();
    let mut format_objects = Vec::new();
    let mut selected_keys = html
        .runtime_file_keys
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if selected_keys.len() != html.runtime_file_keys.len() {
        bail!("HTML runtimeFileKeys contains duplicate keys");
    }
    for format in &config.formats {
        let (name, manifest_format, bytes) = load_format(format)?;
        let closure = manifest_format.input_closure.as_ref().with_context(|| {
            format!("HTML format {name:?} must carry an authenticated input closure")
        })?;
        selected_keys.extend(closure.keys.iter().cloned());
        if formats
            .insert(name.clone(), manifest_format.clone())
            .is_some()
        {
            bail!("duplicate published format name {name:?}");
        }
        format_objects.push((manifest_format.object.clone(), bytes));
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

    let objects = output.join("objects");
    fs::create_dir_all(&objects)
        .with_context(|| format!("create output directory {}", objects.display()))?;
    let mut files = BTreeMap::new();
    for (key, candidate) in selected {
        let bytes = fs::read(&candidate.source)
            .with_context(|| format!("read {}", candidate.source.display()))?;
        let object = format!("sha256-{}", candidate.sha256);
        fs::write(objects.join(&object), &bytes).context("write HTML runtime object")?;
        files.insert(
            key.clone(),
            ManifestFile {
                virtual_path: format!("/texlive/{}", candidate.relative),
                object,
                sha256: candidate.sha256,
                bytes: u64::try_from(bytes.len()).context("file length exceeds u64")?,
                dependencies: selected_dependencies.get(&key).cloned().unwrap_or_default(),
            },
        );
    }
    for (object, bytes) in format_objects {
        fs::write(objects.join(object), bytes).context("write HTML format object")?;
    }
    stage_html_catalog_objects(html, &catalog, &objects)?;

    let manifest = Manifest {
        schema: umber_distribution::MANIFEST_SCHEMA,
        distribution: config.distribution.clone(),
        objects_base_url: config.objects_base_url.clone(),
        files,
        fonts: BTreeMap::new(),
        formats,
    };
    let publication = write_html_sharded_manifest(
        &manifest,
        config.shard_bits,
        output,
        &catalog.fonts,
        &catalog.legacy_mappings,
    )?;
    remove_stale_objects(&objects, &sharded::referenced_objects(&publication))?;
    validate_html_inventory(&html.inventory, output, &publication)?;
    verify_sharded_snapshot(output).context("verify staged HTML sharded snapshot")
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
            file_key.starts_with("tfm:") && candidate.sha256 == mapping.request.tfm_sha256()
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

fn stage_html_catalog_objects(
    html: &HtmlProfileConfig,
    catalog: &ManifestShard,
    objects: &Path,
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
        .map(|entry| (entry.sha256.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    if html.object_sources.keys().collect::<BTreeSet<_>>() != expected.keys().collect() {
        bail!("HTML objectSources must exactly cover the catalog font and license digests");
    }
    for (digest, entry) in expected {
        let source = &html.object_sources[&digest];
        let bytes = fs::read(source)
            .with_context(|| format!("read HTML catalog object {}", source.display()))?;
        if format!("{:x}", Sha256::digest(&bytes)) != digest
            || bytes.len() as u64 != entry.bytes
            || entry.object != format!("sha256-{digest}")
        {
            bail!("HTML catalog object {digest} does not match its declared digest and length");
        }
        fs::write(objects.join(&entry.object), bytes).context("write HTML catalog object")?;
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

fn load_format(config: &FormatConfig) -> Result<(String, ManifestFormat, Vec<u8>)> {
    let metadata_bytes = fs::read(&config.metadata)
        .with_context(|| format!("read format metadata {}", config.metadata.display()))?;
    let metadata: FormatMetadata =
        serde_json::from_slice(&metadata_bytes).context("parse format metadata")?;
    if !matches!(metadata.schema, 1 | 2) || metadata.engine != "umber" {
        bail!("format metadata must describe schema 1 or 2 for engine umber");
    }
    if metadata.schema == 1 && metadata.input_closure.is_some() {
        bail!("format metadata schema 1 cannot contain an input closure");
    }
    if metadata.schema == 2 && metadata.input_closure.is_none() {
        bail!("format metadata schema 2 requires an input closure");
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
    let input_closure = metadata
        .input_closure
        .map(|closure| canonicalize_format_input_closure(closure, &metadata.name))
        .transpose()?;
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
        input_closure,
    };
    Ok((metadata.name, published, bytes))
}

fn canonicalize_format_input_closure(
    mut closure: FormatInputClosureMetadata,
    format_name: &str,
) -> Result<FormatInputClosure> {
    if closure.schema != FORMAT_INPUT_CLOSURE_SCHEMA {
        bail!(
            "unsupported input closure schema {} for format {format_name}; expected {FORMAT_INPUT_CLOSURE_SCHEMA}",
            closure.schema
        );
    }
    if closure.keys.is_empty() || closure.keys.len() > MAX_FORMAT_INPUTS {
        bail!(
            "input closure for format {format_name} must contain between 1 and {MAX_FORMAT_INPUTS} keys"
        );
    }
    for key in &closure.keys {
        FileRequestKey::from_manifest_key(key).with_context(|| {
            format!("invalid input closure key {key:?} for format {format_name}")
        })?;
    }
    let original_len = closure.keys.len();
    closure.keys.sort();
    closure.keys.dedup();
    if closure.keys.len() != original_len {
        bail!("input closure for format {format_name} contains duplicate keys");
    }
    Ok(FormatInputClosure {
        schema: closure.schema,
        keys: closure.keys,
    })
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
