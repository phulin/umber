use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::manifest::{ManifestParseError, ManifestShard, ObjectEntry, validate_file_key};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FileKind {
    Tex,
    Tfm,
    BibAux,
    ClassicBib,
    BibStyle,
}

impl FileKind {
    #[must_use]
    pub const fn manifest_name(self) -> &'static str {
        match self {
            Self::Tex => "tex",
            Self::Tfm => "tfm",
            Self::BibAux => "bib-aux",
            Self::ClassicBib => "classic-bib",
            Self::BibStyle => "bst",
        }
    }

    pub fn from_manifest_name(value: &str) -> Result<Self, SelectionError> {
        match value {
            "tex" => Ok(Self::Tex),
            "tfm" => Ok(Self::Tfm),
            "bib-aux" => Ok(Self::BibAux),
            "classic-bib" => Ok(Self::ClassicBib),
            "bst" => Ok(Self::BibStyle),
            _ => Err(SelectionError::new(format!(
                "unsupported distribution file kind {value:?}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileRequestKey {
    kind: FileKind,
    normalized_name: String,
}

impl FileRequestKey {
    pub fn new(kind: FileKind, normalized_name: impl Into<String>) -> Result<Self, SelectionError> {
        let key = Self {
            kind,
            normalized_name: normalized_name.into(),
        };
        validate_file_key(&key.manifest_key().0).map_err(SelectionError::from_manifest)?;
        Ok(key)
    }

    #[must_use]
    pub const fn kind(&self) -> FileKind {
        self.kind
    }

    #[must_use]
    pub fn normalized_name(&self) -> &str {
        &self.normalized_name
    }

    #[must_use]
    pub fn manifest_key(&self) -> ManifestLogicalKey {
        ManifestLogicalKey(format!(
            "{}:{}",
            self.kind.manifest_name(),
            self.normalized_name
        ))
    }

    pub fn from_manifest_key(value: &str) -> Result<Self, SelectionError> {
        validate_file_key(value).map_err(SelectionError::from_manifest)?;
        let (kind, name) = value
            .split_once(':')
            .expect("validated manifest file keys contain a colon");
        Self::new(FileKind::from_manifest_name(kind)?, name)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontRequestKey {
    logical_name: String,
    pub face_index: u32,
    pub variation_instance: VariationInstance,
    pub variations: Vec<VariationCoordinate>,
    pub features: Vec<FeatureSetting>,
    pub direction: WritingDirection,
    pub script: Option<[u8; 4]>,
    pub language: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontRequestContext {
    pub face_index: u32,
    pub variation_instance: VariationInstance,
    pub variations: Vec<VariationCoordinate>,
    pub features: Vec<FeatureSetting>,
    pub direction: WritingDirection,
    pub script: Option<[u8; 4]>,
    pub language: Option<String>,
}

impl FontRequestKey {
    pub fn new(logical_name: impl Into<String>) -> Result<Self, SelectionError> {
        let logical_name = logical_name.into();
        validate_logical_name(&logical_name)?;
        Ok(Self {
            logical_name,
            face_index: 0,
            variation_instance: VariationInstance::Default,
            variations: Vec::new(),
            features: Vec::new(),
            direction: WritingDirection::LeftToRight,
            script: None,
            language: None,
        })
    }

    pub fn with_context(mut self, context: FontRequestContext) -> Result<Self, SelectionError> {
        let FontRequestContext {
            face_index,
            variation_instance,
            mut variations,
            mut features,
            direction,
            script,
            language,
        } = context;
        if face_index >= 64 {
            return Err(SelectionError::new("font face index must be below 64"));
        }
        variations.sort_unstable();
        features.sort_unstable();
        if variations.len() > 64 || variations.windows(2).any(|pair| pair[0].tag == pair[1].tag) {
            return Err(SelectionError::new(
                "font variations contain duplicate or excessive axes",
            ));
        }
        if features.len() > 64 || features.windows(2).any(|pair| pair[0].tag == pair[1].tag) {
            return Err(SelectionError::new(
                "font features contain duplicate or excessive tags",
            ));
        }
        if variations.iter().any(|item| !valid_tag(item.tag))
            || features.iter().any(|item| !valid_tag(item.tag))
            || script.is_some_and(|tag| !valid_tag(tag))
        {
            return Err(SelectionError::new(
                "OpenType tags must be four printable ASCII bytes",
            ));
        }
        if matches!(
            variation_instance,
            VariationInstance::Default | VariationInstance::Named(_)
        ) && !variations.is_empty()
        {
            return Err(SelectionError::new(
                "only coordinate variation instances may carry axes",
            ));
        }
        let language = language.map(|value| value.to_ascii_lowercase());
        if language.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.len() > 63
                || value.starts_with('-')
                || value.ends_with('-')
                || value
                    .bytes()
                    .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-')
        }) {
            return Err(SelectionError::new("invalid font language"));
        }
        self.face_index = face_index;
        self.variation_instance = variation_instance;
        self.variations = variations;
        self.features = features;
        self.direction = direction;
        self.script = script;
        self.language = language;
        Ok(self)
    }

    #[must_use]
    pub fn logical_name(&self) -> &str {
        &self.logical_name
    }

    #[must_use]
    pub fn manifest_key(&self) -> ManifestLogicalKey {
        let instance = match self.variation_instance {
            VariationInstance::Default => "d".to_owned(),
            VariationInstance::Named(value) => format!("n{value}"),
            VariationInstance::Coordinates => "c".to_owned(),
        };
        let variations = self
            .variations
            .iter()
            .map(|item| format!("{}={:08x}", hex(&item.tag), item.value as u32))
            .collect::<Vec<_>>()
            .join(",");
        let features = self
            .features
            .iter()
            .map(|item| format!("{}={:08x}", hex(&item.tag), item.value))
            .collect::<Vec<_>>()
            .join(",");
        ManifestLogicalKey(format!(
            "font:1:{}:{}:{instance}:{variations}:{features}:{}:{}:{}",
            hex(self.logical_name.as_bytes()),
            self.face_index,
            match self.direction {
                WritingDirection::LeftToRight => "ltr",
                WritingDirection::RightToLeft => "rtl",
            },
            self.script
                .map_or_else(|| "-".to_owned(), |value| hex(&value)),
            self.language
                .as_ref()
                .map_or_else(|| "-".to_owned(), |value| hex(value.as_bytes())),
        ))
    }

    pub fn from_manifest_key(value: &str) -> Result<Self, SelectionError> {
        if value.len() > 4096 {
            return Err(SelectionError::new("font request key is too long"));
        }
        let fields = value.split(':').collect::<Vec<_>>();
        let [
            "font",
            "1",
            logical,
            face,
            instance,
            variations,
            features,
            direction,
            script,
            language,
        ] = fields.as_slice()
        else {
            return Err(SelectionError::new("invalid canonical font request key"));
        };
        let logical_name = String::from_utf8(unhex(logical)?)
            .map_err(|_| SelectionError::new("font logical name is not UTF-8"))?;
        let face_index = face
            .parse()
            .map_err(|_| SelectionError::new("invalid font face index"))?;
        let variation_instance = if *instance == "d" {
            VariationInstance::Default
        } else if *instance == "c" {
            VariationInstance::Coordinates
        } else if let Some(value) = instance.strip_prefix('n') {
            VariationInstance::Named(
                value
                    .parse()
                    .map_err(|_| SelectionError::new("invalid named variation"))?,
            )
        } else {
            return Err(SelectionError::new("invalid variation instance"));
        };
        let variations = parse_settings(variations, true)?
            .into_iter()
            .map(|(tag, value)| VariationCoordinate {
                tag,
                value: value as i32,
            })
            .collect();
        let features = parse_settings(features, false)?
            .into_iter()
            .map(|(tag, value)| FeatureSetting { tag, value })
            .collect();
        let direction = match *direction {
            "ltr" => WritingDirection::LeftToRight,
            "rtl" => WritingDirection::RightToLeft,
            _ => return Err(SelectionError::new("invalid writing direction")),
        };
        let script = if *script == "-" {
            None
        } else {
            Some(unhex_array(script)?)
        };
        let language = if *language == "-" {
            None
        } else {
            Some(
                String::from_utf8(unhex(language)?)
                    .map_err(|_| SelectionError::new("font language is not UTF-8"))?,
            )
        };
        Self::new(logical_name)?.with_context(FontRequestContext {
            face_index,
            variation_instance,
            variations,
            features,
            direction,
            script,
            language,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum VariationInstance {
    Default,
    Named(u16),
    Coordinates,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VariationCoordinate {
    pub tag: [u8; 4],
    pub value: i32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FeatureSetting {
    pub tag: [u8; 4],
    pub value: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum WritingDirection {
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LegacyMappingRequestKey {
    mapping_schema: u32,
    tfm_ahash64: String,
    layout_policy_version: u32,
    purpose: String,
    encoding_catalog: Option<String>,
}

impl LegacyMappingRequestKey {
    pub fn new(
        tfm_ahash64: impl Into<String>,
        layout_policy_version: u32,
        purpose: impl Into<String>,
        encoding_catalog: Option<String>,
    ) -> Result<Self, SelectionError> {
        let tfm_ahash64 = tfm_ahash64.into();
        if tfm_ahash64.len() != 16
            || !tfm_ahash64
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(SelectionError::new(
                "legacy mapping TFM digest must be lowercase aHash64",
            ));
        }
        if layout_policy_version != 1 {
            return Err(SelectionError::new("unsupported layout policy version"));
        }
        let purpose = purpose.into();
        if !matches!(purpose.as_str(), "html-layout" | "html-paint") {
            return Err(SelectionError::new("unsupported legacy mapping purpose"));
        }
        if encoding_catalog.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
        }) {
            return Err(SelectionError::new("invalid encoding catalog identifier"));
        }
        Ok(Self {
            mapping_schema: 2,
            tfm_ahash64,
            layout_policy_version,
            purpose,
            encoding_catalog,
        })
    }

    #[must_use]
    pub fn tfm_ahash64(&self) -> &str {
        &self.tfm_ahash64
    }

    #[must_use]
    pub fn manifest_key(&self) -> ManifestLogicalKey {
        ManifestLogicalKey(format!(
            "legacy-mapping:{}:{}:{}:{}:{}",
            self.mapping_schema,
            self.tfm_ahash64,
            self.layout_policy_version,
            self.purpose,
            self.encoding_catalog
                .as_ref()
                .map_or_else(|| "-".to_owned(), |value| hex(value.as_bytes()))
        ))
    }

    pub fn from_manifest_key(value: &str) -> Result<Self, SelectionError> {
        if value.len() > 4096 {
            return Err(SelectionError::new(
                "legacy mapping request key is too long",
            ));
        }
        let fields = value.split(':').collect::<Vec<_>>();
        let ["legacy-mapping", schema, digest, layout, purpose, encoding] = fields.as_slice()
        else {
            return Err(SelectionError::new(
                "invalid canonical legacy mapping request key",
            ));
        };
        if *schema != "2" {
            return Err(SelectionError::new(format!(
                "unsupported legacy mapping request schema {schema}"
            )));
        }
        let encoding_catalog = if *encoding == "-" {
            None
        } else {
            Some(
                String::from_utf8(unhex(encoding)?)
                    .map_err(|_| SelectionError::new("encoding catalog is not UTF-8"))?,
            )
        };
        Self::new(
            *digest,
            layout
                .parse()
                .map_err(|_| SelectionError::new("invalid layout policy version"))?,
            *purpose,
            encoding_catalog,
        )
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManifestLogicalKey(String);

impl ManifestLogicalKey {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ManifestLogicalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn shard_index(key: &ManifestLogicalKey, shard_bits: u8) -> Result<u32, SelectionError> {
    if shard_bits > crate::MAX_SHARD_BITS {
        return Err(SelectionError::new(format!(
            "shard bits must not exceed {}",
            crate::MAX_SHARD_BITS
        )));
    }
    if shard_bits == 0 {
        return Ok(0);
    }
    let hash = crate::ahash64::shard_key(key.as_str().as_bytes());
    Ok((hash >> (64 - shard_bits)) as u32)
}

/// Validates any canonical catalog request key and returns its shard index.
pub fn shard_index_for_key(value: &str, shard_bits: u8) -> Result<u32, SelectionError> {
    if value.starts_with("font:") {
        let canonical = FontRequestKey::from_manifest_key(value)?.manifest_key();
        if canonical.as_str() != value {
            return Err(SelectionError::new("noncanonical distribution request key"));
        }
    } else if value.starts_with("legacy-mapping:") {
        let canonical = LegacyMappingRequestKey::from_manifest_key(value)?.manifest_key();
        if canonical.as_str() != value {
            return Err(SelectionError::new("noncanonical distribution request key"));
        }
    } else {
        validate_file_key(value).map_err(SelectionError::from_manifest)?;
    }
    if shard_bits > crate::MAX_SHARD_BITS {
        return Err(SelectionError::new(format!(
            "shard bits must not exceed {}",
            crate::MAX_SHARD_BITS
        )));
    }
    if shard_bits == 0 {
        return Ok(0);
    }
    let hash = crate::ahash64::shard_key(value.as_bytes());
    Ok((hash >> (64 - shard_bits)) as u32)
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ManifestRequest {
    File(FileRequestKey),
    Font(FontRequestKey),
    LegacyMapping(LegacyMappingRequestKey),
}

impl ManifestRequest {
    #[must_use]
    pub fn manifest_key(&self) -> ManifestLogicalKey {
        match self {
            Self::File(key) => key.manifest_key(),
            Self::Font(key) => key.manifest_key(),
            Self::LegacyMapping(key) => key.manifest_key(),
        }
    }

    pub fn from_manifest_key(value: &str) -> Result<Self, SelectionError> {
        if value.starts_with("font:") {
            FontRequestKey::from_manifest_key(value).map(Self::Font)
        } else if value.starts_with("legacy-mapping:") {
            LegacyMappingRequestKey::from_manifest_key(value).map(Self::LegacyMapping)
        } else {
            FileRequestKey::from_manifest_key(value).map(Self::File)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobRequirement {
    Required,
    DependencyHint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquisitionJob {
    pub request: ManifestRequest,
    pub manifest_key: ManifestLogicalKey,
    pub requirement: JobRequirement,
    pub object: ObjectEntry,
    pub virtual_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestMiss {
    File(FileRequestKey),
    Font(FontRequestKey),
    LegacyMapping(LegacyMappingRequestKey),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    pub jobs: Vec<AcquisitionJob>,
    pub misses: Vec<ManifestMiss>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionError {
    message: String,
}

impl SelectionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn from_manifest(error: ManifestParseError) -> Self {
        Self::new(error.to_string())
    }
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for SelectionError {}

/// Selects typed records from one already-verified canonical shard.
#[must_use]
pub fn select_shard(shard: &ManifestShard, requests: &[ManifestRequest]) -> Selection {
    select_from_shards(requests, |_| shard)
}

fn select_from_shards<'a>(
    requests: &[ManifestRequest],
    mut shard_for: impl FnMut(&ManifestLogicalKey) -> &'a ManifestShard,
) -> Selection {
    let mut selection = Selection::default();
    let mut seen = BTreeSet::new();
    let mut dependencies = Vec::new();
    for request in requests {
        let key = request.manifest_key();
        if !seen.insert(key.0.clone()) {
            continue;
        }
        let shard = shard_for(&key);
        let object = match request {
            ManifestRequest::File(request_key) => shard
                .files
                .get(key.as_str())
                .map(|entry| {
                    dependencies.extend(entry.dependencies.iter().cloned());
                    (entry.object_entry(), Some(entry.virtual_path.clone()))
                })
                .ok_or_else(|| ManifestMiss::File(request_key.clone())),
            ManifestRequest::Font(request_key) => shard
                .fonts
                .get(key.as_str())
                .map(|entry| (entry.object.clone(), None))
                .ok_or_else(|| ManifestMiss::Font(request_key.clone())),
            ManifestRequest::LegacyMapping(request_key) => shard
                .legacy_mappings
                .get(key.as_str())
                .map(|entry| (entry.object.clone(), None))
                .ok_or_else(|| ManifestMiss::LegacyMapping(request_key.clone())),
        };
        match object {
            Ok((object, virtual_path)) => selection.jobs.push(AcquisitionJob {
                request: request.clone(),
                manifest_key: key,
                requirement: JobRequirement::Required,
                object,
                virtual_path,
            }),
            Err(miss) => selection.misses.push(miss),
        }
    }
    for dependency in dependencies {
        if !seen.insert(dependency.key.clone()) {
            continue;
        }
        let Ok(key) = FileRequestKey::from_manifest_key(&dependency.key) else {
            continue;
        };
        let object = dependency.object_entry();
        selection.jobs.push(AcquisitionJob {
            request: ManifestRequest::File(key),
            manifest_key: ManifestLogicalKey(dependency.key),
            requirement: JobRequirement::DependencyHint,
            object,
            virtual_path: Some(dependency.virtual_path),
        });
    }
    selection
}

/// Selects one ordered request batch from its verified canonical shards.
/// Required jobs retain first-request order; their inline dependency hints
/// follow in discovery order after every required job.
#[must_use]
pub fn select_shards(
    shards: &BTreeMap<u32, ManifestShard>,
    shard_bits: u8,
    requests: &[ManifestRequest],
) -> Selection {
    select_from_shards(requests, |key| {
        let index = shard_index(key, shard_bits).expect("validated request key has a shard index");
        shards
            .get(&index)
            .expect("verified batch contains every requested shard")
    })
}

/// Selects one ordered request batch directly from already-validated packed
/// shards. Required jobs retain first-request order and inline dependency
/// hints follow in discovery order.
#[must_use]
pub fn select_packed_shards(
    shards: &BTreeMap<u32, crate::ValidatedPackedShard>,
    shard_bits: u8,
    requests: &[ManifestRequest],
) -> Selection {
    let mut selection = Selection::default();
    let mut seen = BTreeSet::new();
    let mut dependencies = Vec::new();
    for request in requests {
        let key = request.manifest_key();
        if !seen.insert(key.0.clone()) {
            continue;
        }
        let index = shard_index(&key, shard_bits).expect("validated request shard index");
        let shard = &shards[&index];
        let Some(record) = shard.lookup(key.as_str()) else {
            selection.misses.push(match request {
                ManifestRequest::File(key) => ManifestMiss::File(key.clone()),
                ManifestRequest::Font(key) => ManifestMiss::Font(key.clone()),
                ManifestRequest::LegacyMapping(key) => ManifestMiss::LegacyMapping(key.clone()),
            });
            continue;
        };
        let (object, virtual_path) = match request {
            ManifestRequest::File(_) => {
                let file = record
                    .file()
                    .expect("validated key kind matches request kind");
                dependencies.extend(file.dependencies().map(|dependency| {
                    (
                        dependency.key().to_owned(),
                        dependency.virtual_path().to_owned(),
                        dependency.object(),
                    )
                }));
                (file.object(), Some(file.virtual_path().to_owned()))
            }
            ManifestRequest::Font(_) => (
                record
                    .font()
                    .expect("validated font metadata")
                    .expect("validated key kind matches request kind")
                    .object,
                None,
            ),
            ManifestRequest::LegacyMapping(_) => (
                record
                    .legacy_mapping()
                    .expect("validated mapping metadata")
                    .expect("validated key kind matches request kind")
                    .object,
                None,
            ),
        };
        selection.jobs.push(AcquisitionJob {
            request: request.clone(),
            manifest_key: key,
            requirement: JobRequirement::Required,
            object,
            virtual_path,
        });
    }
    for (key, virtual_path, object) in dependencies {
        if !seen.insert(key.clone()) {
            continue;
        }
        let Ok(request) = FileRequestKey::from_manifest_key(&key) else {
            continue;
        };
        selection.jobs.push(AcquisitionJob {
            request: ManifestRequest::File(request),
            manifest_key: ManifestLogicalKey(key),
            requirement: JobRequirement::DependencyHint,
            object,
            virtual_path: Some(virtual_path),
        });
    }
    selection
}

fn validate_logical_name(value: &str) -> Result<(), SelectionError> {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        Err(SelectionError::new("invalid font logical name"))
    } else {
        Ok(())
    }
}

fn valid_tag(value: [u8; 4]) -> bool {
    value.iter().all(|byte| matches!(byte, 0x20..=0x7e))
}

fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}

fn unhex(value: &str) -> Result<Vec<u8>, SelectionError> {
    if !value.len().is_multiple_of(2) {
        return Err(SelectionError::new("invalid hexadecimal request-key field"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte| match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                _ => Err(SelectionError::new("invalid hexadecimal request-key field")),
            };
            Ok((digit(pair[0])? << 4) | digit(pair[1])?)
        })
        .collect()
}

fn unhex_array(value: &str) -> Result<[u8; 4], SelectionError> {
    unhex(value)?
        .try_into()
        .map_err(|_| SelectionError::new("OpenType tag must contain four bytes"))
}

fn parse_settings(value: &str, signed: bool) -> Result<Vec<([u8; 4], u32)>, SelectionError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|item| {
            let (tag, number) = item
                .split_once('=')
                .ok_or_else(|| SelectionError::new("invalid request-key setting"))?;
            let tag = unhex_array(tag)?;
            if number.len() != 8
                || !number
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            {
                return Err(SelectionError::new("invalid request-key setting value"));
            }
            let number = u32::from_str_radix(number, 16)
                .map_err(|_| SelectionError::new("invalid request-key setting value"))?;
            let _ = signed;
            Ok((tag, number))
        })
        .collect()
}
