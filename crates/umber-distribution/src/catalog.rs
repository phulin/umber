use std::collections::BTreeMap;

use crate::{
    DependencyHint, FontManifestRecord, HTML_INDEX_SHARD_SCHEMA, HTML_SHARDED_ROOT_SCHEMA,
    INDEX_SHARD_SCHEMA, LegacyMappingManifestRecord, Manifest, ManifestFile, ManifestFormat,
    ManifestShard, SHARDED_ROOT_SCHEMA, SelectionError, ShardFile, ShardedManifestRoot,
    ValidatedPackedShard, pack_shard, select_packed_shards, shard_index_for_key,
};

/// A complete browser acquisition plan whose shard bytes were verified
/// against the supplied root before any record became selectable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedBatchPlan {
    pub root: ShardedManifestRoot,
    pub shards: BTreeMap<u32, ValidatedPackedShard>,
    pub selection: crate::Selection,
}

/// Strictly parses a root and returns the unique shard indexes needed by an
/// ordered canonical request batch.
pub fn prepare_batch(
    root_text: &str,
    requests: &[crate::ManifestRequest],
) -> Result<(ShardedManifestRoot, Vec<u32>), SelectionError> {
    let root = ShardedManifestRoot::parse(root_text).map_err(SelectionError::from_manifest)?;
    let mut indexes = requests
        .iter()
        .map(|request| shard_index_for_key(request.manifest_key().as_str(), root.shard_bits))
        .collect::<Result<Vec<_>, _>>()?;
    indexes.sort_unstable();
    indexes.dedup();
    Ok((root, indexes))
}

/// Authenticates the exact selected shard bytes and produces the shared
/// required-before-hint acquisition plan.
pub fn verify_batch(
    root_text: &str,
    raw_shards: &[(u32, &[u8])],
    requests: &[crate::ManifestRequest],
) -> Result<VerifiedBatchPlan, SelectionError> {
    let (root, expected_indexes) = prepare_batch(root_text, requests)?;
    let mut shards = BTreeMap::new();
    for &(index, bytes) in raw_shards {
        if !expected_indexes.contains(&index) {
            return Err(SelectionError::new(format!(
                "unexpected index shard {index} in acquisition batch"
            )));
        }
        let expected_digest = root
            .shard_digest(index)
            .ok_or_else(|| SelectionError::new(format!("invalid index shard {index}")))?;
        if ahash64_hex(bytes) != expected_digest {
            return Err(SelectionError::new(format!(
                "index shard {index} does not match its verified root digest"
            )));
        }
        let shard = ValidatedPackedShard::new(bytes.to_vec(), &root, index)?;
        if shards.insert(index, shard).is_some() {
            return Err(SelectionError::new(format!(
                "duplicate index shard {index} in acquisition batch"
            )));
        }
    }
    if shards.keys().copied().collect::<Vec<_>>() != expected_indexes {
        return Err(SelectionError::new(
            "acquisition batch does not contain every required index shard",
        ));
    }
    let selection = select_packed_shards(&shards, root.shard_bits, requests);
    Ok(VerifiedBatchPlan {
        root,
        shards,
        selection,
    })
}

fn ahash64_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    crate::ahash64::digest(bytes)
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect()
}

/// Canonical, I/O-free representation of a complete sharded publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardedCatalog {
    pub root: ShardedManifestRoot,
    pub shards: Vec<ManifestShard>,
    pub files: BTreeMap<String, ManifestFile>,
    pub formats: BTreeMap<String, ManifestFormat>,
    pub fonts: BTreeMap<String, FontManifestRecord>,
    pub legacy_mappings: BTreeMap<String, LegacyMappingManifestRecord>,
}

/// Partitions a validated monolithic manifest into the canonical sharded model.
pub fn shard_manifest(
    manifest: &Manifest,
    shard_bits: u8,
) -> Result<ShardedCatalog, SelectionError> {
    shard_manifest_with_records(
        manifest,
        shard_bits,
        SHARDED_ROOT_SCHEMA,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

/// Partitions a manifest and optional HTML records using the requested root schema.
pub fn shard_manifest_with_records(
    manifest: &Manifest,
    shard_bits: u8,
    root_schema: u32,
    fonts: &BTreeMap<String, FontManifestRecord>,
    legacy_mappings: &BTreeMap<String, LegacyMappingManifestRecord>,
) -> Result<ShardedCatalog, SelectionError> {
    let shard_count = 1_usize
        .checked_shl(u32::from(shard_bits))
        .filter(|_| shard_bits <= crate::MAX_SHARD_BITS)
        .ok_or_else(|| SelectionError::new("invalid shard bit count"))?;
    if !matches!(root_schema, SHARDED_ROOT_SCHEMA | HTML_SHARDED_ROOT_SCHEMA) {
        return Err(SelectionError::new("invalid sharded root schema"));
    }
    let mut shard_files = vec![BTreeMap::new(); shard_count];
    for (key, file) in &manifest.files {
        let dependencies = file
            .dependencies
            .iter()
            .map(|dependency| {
                let target = manifest
                    .files
                    .get(dependency)
                    .expect("validated manifest dependency exists");
                DependencyHint {
                    key: dependency.clone(),
                    virtual_path: target.virtual_path.clone(),
                    object: target.object.clone(),
                    ahash64: target.ahash64.clone(),
                    bytes: target.bytes,
                }
            })
            .collect();
        let index = shard_index_for_key(key, shard_bits)? as usize;
        shard_files[index].insert(
            key.clone(),
            ShardFile {
                virtual_path: file.virtual_path.clone(),
                object: file.object.clone(),
                ahash64: file.ahash64.clone(),
                bytes: file.bytes,
                dependencies,
            },
        );
    }
    let mut shard_fonts = vec![BTreeMap::new(); shard_count];
    for (key, record) in fonts {
        let index = shard_index_for_key(key, shard_bits)? as usize;
        shard_fonts[index].insert(key.clone(), record.clone());
    }
    let mut shard_mappings = vec![BTreeMap::new(); shard_count];
    for (key, record) in legacy_mappings {
        let index = shard_index_for_key(key, shard_bits)? as usize;
        shard_mappings[index].insert(key.clone(), record.clone());
    }
    let shard_schema = if root_schema == HTML_SHARDED_ROOT_SCHEMA {
        HTML_INDEX_SHARD_SCHEMA
    } else {
        INDEX_SHARD_SCHEMA
    };
    let shards = shard_files
        .into_iter()
        .zip(shard_fonts)
        .zip(shard_mappings)
        .enumerate()
        .map(|(index, ((files, fonts), legacy_mappings))| ManifestShard {
            schema: shard_schema,
            distribution: manifest.distribution.clone(),
            index: index as u32,
            files,
            fonts,
            legacy_mappings,
        })
        .collect::<Vec<_>>();
    let shard_digests = shards
        .iter()
        .map(pack_shard)
        .map(|result| result.map(|bytes| ahash64_hex(&bytes)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ShardedCatalog {
        root: ShardedManifestRoot {
            schema: root_schema,
            distribution: manifest.distribution.clone(),
            objects_base_url: manifest.objects_base_url.clone(),
            shard_bits,
            shard_count: shard_count as u32,
            shards: shard_digests,
            formats: manifest.formats.clone(),
        },
        shards,
        files: manifest.files.clone(),
        formats: manifest.formats.clone(),
        fonts: fonts.clone(),
        legacy_mappings: legacy_mappings.clone(),
    })
}

/// Validates and assembles independently verified root and shard values.
pub fn assemble_sharded_catalog(
    root: ShardedManifestRoot,
    shards: Vec<ManifestShard>,
) -> Result<ShardedCatalog, SelectionError> {
    if shards.len() != root.shard_count as usize {
        return Err(SelectionError::new(
            "root manifest shard metadata is inconsistent",
        ));
    }
    let mut files = BTreeMap::new();
    let mut fonts = BTreeMap::new();
    let mut legacy_mappings = BTreeMap::new();
    for (index, shard) in shards.iter().enumerate() {
        shard
            .validate_identity(&root, index as u32)
            .map_err(SelectionError::from_manifest)?;
        for (key, file) in &shard.files {
            if shard_index_for_key(key, root.shard_bits)? != index as u32 {
                return Err(SelectionError::new(format!(
                    "lookup key {key} is not in its canonical shard"
                )));
            }
            if files.insert(key.clone(), file.clone()).is_some() {
                return Err(SelectionError::new(format!(
                    "duplicate lookup key {key} across shards"
                )));
            }
        }
        for (key, record) in &shard.fonts {
            if shard_index_for_key(key, root.shard_bits)? != index as u32
                || fonts.insert(key.clone(), record.clone()).is_some()
            {
                return Err(SelectionError::new(format!(
                    "invalid or duplicate font key {key} across shards"
                )));
            }
        }
        for (key, record) in &shard.legacy_mappings {
            if shard_index_for_key(key, root.shard_bits)? != index as u32
                || legacy_mappings
                    .insert(key.clone(), record.clone())
                    .is_some()
            {
                return Err(SelectionError::new(format!(
                    "invalid or duplicate legacy mapping key {key} across shards"
                )));
            }
        }
    }
    for (key, file) in &files {
        let mut previous: Option<&str> = None;
        for dependency in &file.dependencies {
            if previous.is_some_and(|value| value >= dependency.key.as_str()) {
                return Err(SelectionError::new(format!(
                    "dependencies for {key} are not strictly sorted"
                )));
            }
            let Some(target) = files.get(&dependency.key) else {
                return Err(SelectionError::new(format!(
                    "dependency {} from {key} is absent",
                    dependency.key
                )));
            };
            if dependency.virtual_path != target.virtual_path
                || dependency.object != target.object
                || dependency.ahash64 != target.ahash64
                || dependency.bytes != target.bytes
            {
                return Err(SelectionError::new(format!(
                    "dependency {} from {key} has stale inline metadata",
                    dependency.key
                )));
            }
            previous = Some(&dependency.key);
        }
    }
    for (name, format) in &root.formats {
        if let Some(closure) = &format.input_closure {
            for key in &closure.keys {
                if !files.contains_key(key) {
                    return Err(SelectionError::new(format!(
                        "input closure key {key} for format {name} is absent"
                    )));
                }
            }
        }
    }
    for (key, mapping) in &legacy_mappings {
        let font_key = mapping.font_request.manifest_key().to_string();
        let Some(font) = fonts.get(&font_key) else {
            return Err(SelectionError::new(format!(
                "legacy mapping {key} references absent font {font_key}"
            )));
        };
        if font.object != mapping.object || font.license != mapping.license {
            return Err(SelectionError::new(format!(
                "legacy mapping {key} does not match its declared font and license objects"
            )));
        }
    }
    Ok(ShardedCatalog {
        formats: root.formats.clone(),
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
                        ahash64: file.ahash64,
                        bytes: file.bytes,
                        dependencies,
                    },
                )
            })
            .collect(),
        fonts,
        legacy_mappings,
    })
}
