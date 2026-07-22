use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::html::{
    FontManifestRecord, HTML_INDEX_SHARD_SCHEMA, HTML_SHARDED_ROOT_SCHEMA,
    LegacyMappingManifestRecord, parse_font_records, parse_legacy_mapping_records,
    write_font_records, write_legacy_mapping_records,
};
use crate::json::{self, Value};

pub const MANIFEST_SCHEMA: u32 = 1;
pub const LEGACY_SHARDED_ROOT_SCHEMA: u32 = 2;
pub const SHARDED_ROOT_SCHEMA: u32 = 3;
pub const INDEX_SHARD_SCHEMA: u32 = 1;
pub const MAX_SHARD_BITS: u8 = 16;
pub const FORMAT_INPUT_CLOSURE_SCHEMA: u32 = 1;
pub const MAX_FORMAT_INPUTS: usize = 256;
pub const MAX_REQUEST_KEY_BYTES: usize = 1024;
const MAX_OBJECT_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardedManifestRoot {
    pub schema: u32,
    pub distribution: String,
    pub objects_base_url: String,
    pub shard_bits: u8,
    pub shard_count: u32,
    pub shards: Vec<String>,
    pub formats: BTreeMap<String, ManifestFormat>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestShard {
    pub schema: u32,
    pub distribution: String,
    pub index: u32,
    pub files: BTreeMap<String, ShardFile>,
    pub fonts: BTreeMap<String, FontManifestRecord>,
    pub legacy_mappings: BTreeMap<String, LegacyMappingManifestRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardFile {
    pub virtual_path: String,
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
    pub dependencies: Vec<DependencyHint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyHint {
    pub key: String,
    pub virtual_path: String,
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Manifest {
    pub schema: u32,
    pub distribution: String,
    pub objects_base_url: String,
    pub files: BTreeMap<String, ManifestFile>,
    pub fonts: BTreeMap<String, ManifestFont>,
    pub formats: BTreeMap<String, ManifestFormat>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectEntry {
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestFile {
    pub virtual_path: String,
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestFont {
    pub object: String,
    pub sha256: String,
    pub bytes: u64,
    pub container: String,
    pub provenance: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub input_closure: Option<FormatInputClosure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatInputClosure {
    pub schema: u32,
    pub keys: Vec<String>,
}

impl ManifestFile {
    #[must_use]
    pub fn object_entry(&self) -> ObjectEntry {
        ObjectEntry {
            object: self.object.clone(),
            sha256: self.sha256.clone(),
            bytes: self.bytes,
        }
    }
}

impl ShardFile {
    #[must_use]
    pub fn object_entry(&self) -> ObjectEntry {
        ObjectEntry {
            object: self.object.clone(),
            sha256: self.sha256.clone(),
            bytes: self.bytes,
        }
    }
}

impl DependencyHint {
    #[must_use]
    pub fn object_entry(&self) -> ObjectEntry {
        ObjectEntry {
            object: self.object.clone(),
            sha256: self.sha256.clone(),
            bytes: self.bytes,
        }
    }
}

impl ManifestFont {
    #[must_use]
    pub fn object_entry(&self) -> ObjectEntry {
        ObjectEntry {
            object: self.object.clone(),
            sha256: self.sha256.clone(),
            bytes: self.bytes,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestParseError {
    message: String,
}

impl ManifestParseError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ManifestParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for ManifestParseError {}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, ManifestParseError> {
        let value =
            json::parse(text).map_err(|error| ManifestParseError::new(error.to_string()))?;
        let mut root = object(value, "manifest")?;
        let schema = u32_value(take(&mut root, "schema", "manifest")?, "schema")?;
        if schema != MANIFEST_SCHEMA {
            return Err(ManifestParseError::new(format!(
                "unsupported manifest schema {schema}; expected {MANIFEST_SCHEMA}"
            )));
        }
        let distribution = string(take(&mut root, "distribution", "manifest")?, "distribution")?;
        validate_distribution(&distribution)?;
        let objects_base_url = string(
            take(&mut root, "objectsBaseUrl", "manifest")?,
            "objectsBaseUrl",
        )?;
        validate_base_url(&objects_base_url)?;
        let files = parse_files(take(&mut root, "files", "manifest")?)?;
        let fonts = optional_object(&mut root, "fonts")?
            .map(parse_fonts)
            .transpose()?
            .unwrap_or_default();
        let formats = optional_object(&mut root, "formats")?
            .map(parse_formats)
            .transpose()?
            .unwrap_or_default();
        finish(root, "manifest")?;
        validate_cross_references(&files, &fonts, &formats)?;
        Ok(Self {
            schema,
            distribution,
            objects_base_url,
            files,
            fonts,
            formats,
        })
    }

    /// Canonical ordered JSON used by the deterministic publisher.
    #[must_use]
    pub fn to_json_pretty(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n  \"schema\": ");
        out.push_str(&self.schema.to_string());
        out.push_str(",\n  \"distribution\": ");
        json_string(&mut out, &self.distribution);
        out.push_str(",\n  \"objectsBaseUrl\": ");
        json_string(&mut out, &self.objects_base_url);
        out.push_str(",\n  \"files\": {");
        write_map(&mut out, &self.files, write_file);
        out.push_str("\n  }");
        if !self.fonts.is_empty() {
            out.push_str(",\n  \"fonts\": {");
            write_map(&mut out, &self.fonts, write_font);
            out.push_str("\n  }");
        }
        if !self.formats.is_empty() {
            out.push_str(",\n  \"formats\": {");
            write_map(&mut out, &self.formats, write_format);
            out.push_str("\n  }");
        }
        out.push_str("\n}\n");
        out
    }
}

impl ShardedManifestRoot {
    pub fn parse(text: &str) -> Result<Self, ManifestParseError> {
        let value =
            json::parse(text).map_err(|error| ManifestParseError::new(error.to_string()))?;
        let mut root = object(value, "root manifest")?;
        let schema = u32_value(take(&mut root, "schema", "root manifest")?, "schema")?;
        if !matches!(
            schema,
            LEGACY_SHARDED_ROOT_SCHEMA | SHARDED_ROOT_SCHEMA | HTML_SHARDED_ROOT_SCHEMA
        ) {
            return Err(ManifestParseError::new(format!(
                "unsupported root manifest schema {schema}; expected {LEGACY_SHARDED_ROOT_SCHEMA}, {SHARDED_ROOT_SCHEMA}, or {HTML_SHARDED_ROOT_SCHEMA}"
            )));
        }
        let distribution = string(
            take(&mut root, "distribution", "root manifest")?,
            "distribution",
        )?;
        validate_distribution(&distribution)?;
        let objects_base_url = string(
            take(&mut root, "objectsBaseUrl", "root manifest")?,
            "objectsBaseUrl",
        )?;
        validate_base_url(&objects_base_url)?;
        let shard_bits = u8::try_from(number(
            take(&mut root, "shardBits", "root manifest")?,
            "shardBits",
        )?)
        .map_err(|_| ManifestParseError::new("shardBits exceeds u8"))?;
        if shard_bits > MAX_SHARD_BITS {
            return Err(ManifestParseError::new(format!(
                "shardBits must be between 0 and {MAX_SHARD_BITS}"
            )));
        }
        let shard_count = u32_value(
            take(&mut root, "shardCount", "root manifest")?,
            "shardCount",
        )?;
        let shards = digest_array(take(&mut root, "shards", "root manifest")?, "shards")?;
        let expected_count = 1_u32 << shard_bits;
        if shard_count != expected_count || shards.len() != expected_count as usize {
            return Err(ManifestParseError::new(
                "root manifest shard metadata is inconsistent",
            ));
        }
        let formats = optional_object(&mut root, "formats")?
            .map(parse_formats)
            .transpose()?
            .unwrap_or_default();
        if schema == LEGACY_SHARDED_ROOT_SCHEMA
            && formats
                .values()
                .any(|format| format.input_closure.is_some())
        {
            return Err(ManifestParseError::new(
                "format input closures require root manifest schema 3",
            ));
        }
        finish(root, "root manifest")?;
        validate_cross_references(&BTreeMap::new(), &BTreeMap::new(), &formats)?;
        Ok(Self {
            schema,
            distribution,
            objects_base_url,
            shard_bits,
            shard_count,
            shards,
            formats,
        })
    }

    #[must_use]
    pub fn shard_digest(&self, index: u32) -> Option<&str> {
        self.shards.get(index as usize).map(String::as_str)
    }
}

impl ManifestShard {
    pub fn parse(text: &str) -> Result<Self, ManifestParseError> {
        let value =
            json::parse(text).map_err(|error| ManifestParseError::new(error.to_string()))?;
        let mut root = object(value, "index shard")?;
        let schema = u32_value(take(&mut root, "schema", "index shard")?, "schema")?;
        if !matches!(schema, INDEX_SHARD_SCHEMA | HTML_INDEX_SHARD_SCHEMA) {
            return Err(ManifestParseError::new(format!(
                "unsupported index shard schema {schema}; expected {INDEX_SHARD_SCHEMA} or {HTML_INDEX_SHARD_SCHEMA}"
            )));
        }
        let distribution = string(
            take(&mut root, "distribution", "index shard")?,
            "distribution",
        )?;
        validate_distribution(&distribution)?;
        let index = u32_value(take(&mut root, "index", "index shard")?, "index")?;
        let files = parse_shard_files(take(&mut root, "files", "index shard")?)?;
        let fonts = optional_object(&mut root, "fonts")?
            .map(parse_font_records)
            .transpose()?
            .unwrap_or_default();
        let legacy_mappings = optional_object(&mut root, "legacyMappings")?
            .map(parse_legacy_mapping_records)
            .transpose()?
            .unwrap_or_default();
        if schema == INDEX_SHARD_SCHEMA && (!fonts.is_empty() || !legacy_mappings.is_empty()) {
            return Err(ManifestParseError::new(
                "font and legacy mapping records require index shard schema 2",
            ));
        }
        validate_shard_object_conflicts(&files, &fonts, &legacy_mappings)?;
        finish(root, "index shard")?;
        Ok(Self {
            schema,
            distribution,
            index,
            files,
            fonts,
            legacy_mappings,
        })
    }

    pub fn validate_identity(
        &self,
        root: &ShardedManifestRoot,
        expected_index: u32,
    ) -> Result<(), ManifestParseError> {
        if self.distribution != root.distribution || self.index != expected_index {
            return Err(ManifestParseError::new(format!(
                "index shard {expected_index} identity does not match root manifest"
            )));
        }
        let expected_schema = if root.schema == HTML_SHARDED_ROOT_SCHEMA {
            HTML_INDEX_SHARD_SCHEMA
        } else {
            INDEX_SHARD_SCHEMA
        };
        if self.schema != expected_schema {
            return Err(ManifestParseError::new(format!(
                "index shard {expected_index} schema does not match root manifest"
            )));
        }
        Ok(())
    }

    /// Canonical compact JSON used for immutable shard hashing.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = format!("{{\"schema\":{},\"distribution\":", self.schema);
        json_string(&mut out, &self.distribution);
        out.push_str(",\"index\":");
        out.push_str(&self.index.to_string());
        out.push_str(",\"files\":{");
        for (position, (key, entry)) in self.files.iter().enumerate() {
            if position > 0 {
                out.push(',');
            }
            json_string(&mut out, key);
            out.push_str(":{");
            json_string(&mut out, "virtualPath");
            out.push(':');
            json_string(&mut out, &entry.virtual_path);
            out.push(',');
            json_string(&mut out, "object");
            out.push(':');
            json_string(&mut out, &entry.object);
            out.push(',');
            json_string(&mut out, "sha256");
            out.push(':');
            json_string(&mut out, &entry.sha256);
            out.push(',');
            json_string(&mut out, "bytes");
            out.push(':');
            out.push_str(&entry.bytes.to_string());
            if !entry.dependencies.is_empty() {
                out.push(',');
                json_string(&mut out, "dependencies");
                out.push_str(":[");
                for (dependency_index, dependency) in entry.dependencies.iter().enumerate() {
                    if dependency_index > 0 {
                        out.push(',');
                    }
                    out.push('{');
                    json_string(&mut out, "key");
                    out.push(':');
                    json_string(&mut out, &dependency.key);
                    out.push(',');
                    json_string(&mut out, "virtualPath");
                    out.push(':');
                    json_string(&mut out, &dependency.virtual_path);
                    out.push(',');
                    json_string(&mut out, "object");
                    out.push(':');
                    json_string(&mut out, &dependency.object);
                    out.push(',');
                    json_string(&mut out, "sha256");
                    out.push(':');
                    json_string(&mut out, &dependency.sha256);
                    out.push(',');
                    json_string(&mut out, "bytes");
                    out.push(':');
                    out.push_str(&dependency.bytes.to_string());
                    out.push('}');
                }
                out.push(']');
            }
            out.push('}');
        }
        out.push('}');
        write_font_records(&mut out, &self.fonts);
        write_legacy_mapping_records(&mut out, &self.legacy_mappings);
        out.push_str("}\n");
        out
    }
}

fn validate_shard_object_conflicts(
    files: &BTreeMap<String, ShardFile>,
    fonts: &BTreeMap<String, FontManifestRecord>,
    mappings: &BTreeMap<String, LegacyMappingManifestRecord>,
) -> Result<(), ManifestParseError> {
    let mut lengths = BTreeMap::new();
    for entry in files.values() {
        check_digest_length(&mut lengths, &entry.sha256, entry.bytes)?;
    }
    for entry in fonts.values() {
        check_digest_length(&mut lengths, &entry.object.sha256, entry.object.bytes)?;
        check_digest_length(
            &mut lengths,
            &entry.license.object.sha256,
            entry.license.object.bytes,
        )?;
    }
    for entry in mappings.values() {
        check_digest_length(&mut lengths, &entry.object.sha256, entry.object.bytes)?;
        check_digest_length(
            &mut lengths,
            &entry.license.object.sha256,
            entry.license.object.bytes,
        )?;
    }
    Ok(())
}

fn parse_shard_files(value: Value) -> Result<BTreeMap<String, ShardFile>, ManifestParseError> {
    let entries = object(value, "files")?;
    let mut files = BTreeMap::new();
    for (key, value) in entries {
        validate_file_key(&key)?;
        let mut entry = object(value, &format!("file {key}"))?;
        let virtual_path = string(take(&mut entry, "virtualPath", &key)?, "virtualPath")?;
        validate_path(&virtual_path, "/texlive/", "virtual path")?;
        let object_entry = parse_object_entry(&mut entry, &key)?;
        let dependencies = match entry.remove("dependencies") {
            Some(value) => parse_dependency_hints(value, &key)?,
            None => Vec::new(),
        };
        finish(entry, &format!("file {key}"))?;
        files.insert(
            key,
            ShardFile {
                virtual_path,
                object: object_entry.object,
                sha256: object_entry.sha256,
                bytes: object_entry.bytes,
                dependencies,
            },
        );
    }
    Ok(files)
}

fn parse_dependency_hints(
    value: Value,
    owner: &str,
) -> Result<Vec<DependencyHint>, ManifestParseError> {
    let Value::Array(values) = value else {
        return Err(ManifestParseError::new(format!(
            "dependencies for {owner} must be an array"
        )));
    };
    let mut output = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let mut entry = object(value, &format!("dependency for {owner}"))?;
        let key = string(take(&mut entry, "key", owner)?, "key")?;
        validate_file_key(&key)?;
        if !seen.insert(key.clone()) {
            return Err(ManifestParseError::new(format!(
                "dependencies for {owner} contains duplicate {key}"
            )));
        }
        let virtual_path = string(take(&mut entry, "virtualPath", &key)?, "virtualPath")?;
        validate_path(&virtual_path, "/texlive/", "virtual path")?;
        let object_entry = parse_object_entry(&mut entry, &key)?;
        finish(entry, &format!("dependency {key}"))?;
        output.push(DependencyHint {
            key,
            virtual_path,
            object: object_entry.object,
            sha256: object_entry.sha256,
            bytes: object_entry.bytes,
        });
    }
    Ok(output)
}

fn parse_files(value: Value) -> Result<BTreeMap<String, ManifestFile>, ManifestParseError> {
    let entries = object(value, "files")?;
    let mut files = BTreeMap::new();
    let mut paths = BTreeMap::<String, String>::new();
    for (key, value) in entries {
        validate_file_key(&key)?;
        let mut entry = object(value, &format!("file {key}"))?;
        let virtual_path = string(take(&mut entry, "virtualPath", &key)?, "virtualPath")?;
        validate_path(&virtual_path, "/texlive/", "virtual path")?;
        let object_entry = parse_object_entry(&mut entry, &key)?;
        let dependencies = match entry.remove("dependencies") {
            Some(value) => string_array(value, &format!("dependencies for {key}"))?,
            None => Vec::new(),
        };
        for dependency in &dependencies {
            validate_file_key(dependency)?;
        }
        finish(entry, &format!("file {key}"))?;
        if let Some(previous) = paths.insert(virtual_path.clone(), object_entry.sha256.clone())
            && previous != object_entry.sha256
        {
            return Err(ManifestParseError::new(format!(
                "virtual path {virtual_path} has conflicting objects"
            )));
        }
        files.insert(
            key,
            ManifestFile {
                virtual_path,
                object: object_entry.object,
                sha256: object_entry.sha256,
                bytes: object_entry.bytes,
                dependencies,
            },
        );
    }
    Ok(files)
}

fn parse_fonts(value: Value) -> Result<BTreeMap<String, ManifestFont>, ManifestParseError> {
    let entries = object(value, "fonts")?;
    let mut fonts = BTreeMap::new();
    for (name, value) in entries {
        validate_font_name(&name)?;
        let mut entry = object(value, &format!("font {name}"))?;
        let object_entry = parse_object_entry(&mut entry, &format!("font {name}"))?;
        let container = string(take(&mut entry, "container", &name)?, "container")?;
        if container != "woff2" {
            return Err(ManifestParseError::new(format!(
                "font {name} container must be woff2"
            )));
        }
        let provenance = entry
            .remove("provenance")
            .map(|value| string(value, "provenance"))
            .transpose()?;
        finish(entry, &format!("font {name}"))?;
        fonts.insert(
            name,
            ManifestFont {
                object: object_entry.object,
                sha256: object_entry.sha256,
                bytes: object_entry.bytes,
                container,
                provenance,
            },
        );
    }
    Ok(fonts)
}

fn parse_formats(value: Value) -> Result<BTreeMap<String, ManifestFormat>, ManifestParseError> {
    let entries = object(value, "formats")?;
    let mut formats = BTreeMap::new();
    for (name, value) in entries {
        validate_format_name(&name)?;
        let mut entry = object(value, &format!("format {name}"))?;
        let object_entry = parse_object_entry(&mut entry, &format!("format {name}"))?;
        let engine = string(take(&mut entry, "engine", &name)?, "engine")?;
        if engine != "umber" {
            return Err(ManifestParseError::new(format!(
                "format {name} engine must be umber"
            )));
        }
        let engine_version = nonempty_string(&mut entry, "engineVersion", &name)?;
        let format_schema = u32_value(take(&mut entry, "formatSchema", &name)?, "formatSchema")?;
        if format_schema == 0 {
            return Err(ManifestParseError::new("formatSchema must be positive"));
        }
        let source_distribution = nonempty_string(&mut entry, "sourceDistribution", &name)?;
        let source_manifest_sha256 = string(
            take(&mut entry, "sourceManifestSha256", &name)?,
            "sourceManifestSha256",
        )?;
        validate_digest(&source_manifest_sha256, "source manifest digest")?;
        let source_date_epoch = number(
            take(&mut entry, "sourceDateEpoch", &name)?,
            "sourceDateEpoch",
        )?;
        let input_closure = entry
            .remove("inputClosure")
            .map(|value| parse_format_input_closure(value, &name))
            .transpose()?;
        finish(entry, &format!("format {name}"))?;
        formats.insert(
            name,
            ManifestFormat {
                object: object_entry.object,
                sha256: object_entry.sha256,
                bytes: object_entry.bytes,
                engine,
                engine_version,
                format_schema,
                source_distribution,
                source_manifest_sha256,
                source_date_epoch,
                input_closure,
            },
        );
    }
    Ok(formats)
}

fn parse_format_input_closure(
    value: Value,
    format_name: &str,
) -> Result<FormatInputClosure, ManifestParseError> {
    let mut fields = object(value, &format!("input closure for format {format_name}"))?;
    let schema = u32_value(
        take(&mut fields, "schema", format_name)?,
        "input closure schema",
    )?;
    if schema != FORMAT_INPUT_CLOSURE_SCHEMA {
        return Err(ManifestParseError::new(format!(
            "unsupported format input closure schema {schema}; expected {FORMAT_INPUT_CLOSURE_SCHEMA}"
        )));
    }
    let keys = bounded_request_key_array(
        take(&mut fields, "keys", format_name)?,
        &format!("input closure for format {format_name}"),
    )?;
    finish(fields, &format!("input closure for format {format_name}"))?;
    Ok(FormatInputClosure { schema, keys })
}

pub(crate) fn parse_object_entry(
    fields: &mut BTreeMap<String, Value>,
    label: &str,
) -> Result<ObjectEntry, ManifestParseError> {
    let object = string(take(fields, "object", label)?, "object")?;
    let sha256 = string(take(fields, "sha256", label)?, "sha256")?;
    validate_digest(&sha256, label)?;
    if object != format!("sha256-{sha256}") {
        return Err(ManifestParseError::new(format!(
            "object name for {label} does not match its digest"
        )));
    }
    let bytes = number(take(fields, "bytes", label)?, "bytes")?;
    if bytes > MAX_OBJECT_BYTES {
        return Err(ManifestParseError::new(format!(
            "object {label} exceeds the {MAX_OBJECT_BYTES}-byte manifest limit"
        )));
    }
    Ok(ObjectEntry {
        object,
        sha256,
        bytes,
    })
}

fn validate_cross_references(
    files: &BTreeMap<String, ManifestFile>,
    fonts: &BTreeMap<String, ManifestFont>,
    formats: &BTreeMap<String, ManifestFormat>,
) -> Result<(), ManifestParseError> {
    let mut digest_lengths = BTreeMap::<&str, u64>::new();
    for (key, entry) in files {
        for dependency in &entry.dependencies {
            if !files.contains_key(dependency) {
                return Err(ManifestParseError::new(format!(
                    "dependency {dependency} from {key} is absent"
                )));
            }
        }
        check_digest_length(&mut digest_lengths, &entry.sha256, entry.bytes)?;
    }
    for entry in fonts.values() {
        check_digest_length(&mut digest_lengths, &entry.sha256, entry.bytes)?;
    }
    for entry in formats.values() {
        if let Some(closure) = &entry.input_closure {
            for key in &closure.keys {
                if !files.contains_key(key) && !files.is_empty() {
                    return Err(ManifestParseError::new(format!(
                        "format input {key} is absent"
                    )));
                }
            }
        }
        check_digest_length(&mut digest_lengths, &entry.sha256, entry.bytes)?;
    }
    Ok(())
}

fn check_digest_length<'a>(
    lengths: &mut BTreeMap<&'a str, u64>,
    digest: &'a str,
    bytes: u64,
) -> Result<(), ManifestParseError> {
    if let Some(previous) = lengths.insert(digest, bytes)
        && previous != bytes
    {
        return Err(ManifestParseError::new(format!(
            "inconsistent byte lengths for digest {digest}"
        )));
    }
    Ok(())
}

fn validate_distribution(value: &str) -> Result<(), ManifestParseError> {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return Err(ManifestParseError::new(
            "distribution must be a non-empty identifier without whitespace",
        ));
    }
    Ok(())
}

fn validate_base_url(value: &str) -> Result<(), ManifestParseError> {
    let Some((scheme, rest)) = value.split_once(':') else {
        return Err(ManifestParseError::new("objectsBaseUrl must be absolute"));
    };
    if scheme.is_empty()
        || !scheme.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || (index > 0 && matches!(byte, b'0'..=b'9' | b'+' | b'-' | b'.'))
        })
        || rest.is_empty()
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(ManifestParseError::new("objectsBaseUrl is invalid"));
    }
    if !value.ends_with('/') {
        return Err(ManifestParseError::new("objectsBaseUrl must end with '/'"));
    }
    Ok(())
}

pub(crate) fn validate_file_key(key: &str) -> Result<(), ManifestParseError> {
    if key.len() > MAX_REQUEST_KEY_BYTES {
        return Err(ManifestParseError::new(format!(
            "lookup key exceeds the {MAX_REQUEST_KEY_BYTES}-byte manifest limit"
        )));
    }
    let Some((kind, name)) = key.split_once(':') else {
        return Err(ManifestParseError::new(format!("invalid lookup key {key}")));
    };
    if !matches!(kind, "tex" | "tfm" | "bib-aux" | "classic-bib" | "bst") {
        return Err(ManifestParseError::new(format!("invalid lookup key {key}")));
    }
    validate_path(name, "", "lookup key")
}

pub(crate) fn validate_font_name(name: &str) -> Result<(), ManifestParseError> {
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(ManifestParseError::new(format!(
            "invalid font name {name:?}"
        )));
    }
    Ok(())
}

fn validate_format_name(name: &str) -> Result<(), ManifestParseError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ManifestParseError::new(format!(
            "invalid format name {name:?}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_digest(value: &str, label: &str) -> Result<(), ManifestParseError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ManifestParseError::new(format!(
            "{label} must use a 64-character lowercase SHA-256 digest"
        )));
    }
    Ok(())
}

fn validate_path(value: &str, prefix: &str, label: &str) -> Result<(), ManifestParseError> {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(ManifestParseError::new(format!(
            "invalid {label} {value:?}"
        )));
    };
    if suffix.is_empty()
        || suffix.contains(['\\', '\0', ':'])
        || suffix
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(ManifestParseError::new(format!(
            "invalid {label} {value:?}"
        )));
    }
    Ok(())
}

pub(crate) fn object(
    value: Value,
    label: &str,
) -> Result<BTreeMap<String, Value>, ManifestParseError> {
    match value {
        Value::Object(fields) => Ok(fields),
        _ => Err(ManifestParseError::new(format!(
            "{label} must be an object"
        ))),
    }
}

fn optional_object(
    fields: &mut BTreeMap<String, Value>,
    name: &str,
) -> Result<Option<Value>, ManifestParseError> {
    match fields.remove(name) {
        Some(value @ Value::Object(_)) => Ok(Some(value)),
        Some(_) => Err(ManifestParseError::new(format!("{name} must be an object"))),
        None => Ok(None),
    }
}

pub(crate) fn take(
    fields: &mut BTreeMap<String, Value>,
    name: &str,
    label: &str,
) -> Result<Value, ManifestParseError> {
    fields
        .remove(name)
        .ok_or_else(|| ManifestParseError::new(format!("{label} is missing required field {name}")))
}

pub(crate) fn finish(
    fields: BTreeMap<String, Value>,
    label: &str,
) -> Result<(), ManifestParseError> {
    if fields.is_empty() {
        Ok(())
    } else {
        Err(ManifestParseError::new(format!(
            "unknown field {:?} in {label}",
            fields.keys().next().expect("nonempty map")
        )))
    }
}

pub(crate) fn string(value: Value, label: &str) -> Result<String, ManifestParseError> {
    match value {
        Value::String(value) => Ok(value),
        _ => Err(ManifestParseError::new(format!("{label} must be a string"))),
    }
}

fn nonempty_string(
    fields: &mut BTreeMap<String, Value>,
    name: &str,
    label: &str,
) -> Result<String, ManifestParseError> {
    let value = string(take(fields, name, label)?, name)?;
    if value.is_empty() {
        Err(ManifestParseError::new(format!("{name} must not be empty")))
    } else {
        Ok(value)
    }
}

pub(crate) fn number(value: Value, label: &str) -> Result<u64, ManifestParseError> {
    match value {
        Value::Number(value) => Ok(value),
        _ => Err(ManifestParseError::new(format!(
            "{label} must be an unsigned integer"
        ))),
    }
}

pub(crate) fn u32_value(value: Value, label: &str) -> Result<u32, ManifestParseError> {
    u32::try_from(number(value, label)?)
        .map_err(|_| ManifestParseError::new(format!("{label} exceeds u32")))
}

fn string_array(value: Value, label: &str) -> Result<Vec<String>, ManifestParseError> {
    let Value::Array(values) = value else {
        return Err(ManifestParseError::new(format!("{label} must be an array")));
    };
    let mut output = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let value = string(value, label)?;
        if !seen.insert(value.clone()) {
            return Err(ManifestParseError::new(format!(
                "{label} contains duplicate {value}"
            )));
        }
        output.push(value);
    }
    Ok(output)
}

fn bounded_request_key_array(value: Value, label: &str) -> Result<Vec<String>, ManifestParseError> {
    let Value::Array(values) = value else {
        return Err(ManifestParseError::new(format!("{label} must be an array")));
    };
    if values.is_empty() || values.len() > MAX_FORMAT_INPUTS {
        return Err(ManifestParseError::new(format!(
            "{label} must contain between 1 and {MAX_FORMAT_INPUTS} keys"
        )));
    }
    let mut output = Vec::with_capacity(values.len());
    let mut previous: Option<String> = None;
    for value in values {
        let key = string(value, label)?;
        validate_file_key(&key)?;
        if previous.as_ref().is_some_and(|value| value >= &key) {
            return Err(ManifestParseError::new(format!(
                "{label} keys must be unique and strictly sorted"
            )));
        }
        previous = Some(key.clone());
        output.push(key);
    }
    Ok(output)
}

fn digest_array(value: Value, label: &str) -> Result<Vec<String>, ManifestParseError> {
    let values = string_array(value, label)?;
    for digest in &values {
        validate_digest(digest, label)?;
    }
    Ok(values)
}

fn write_map<T>(out: &mut String, values: &BTreeMap<String, T>, write: fn(&mut String, &T, usize)) {
    for (index, (key, value)) in values.iter().enumerate() {
        out.push_str(if index == 0 { "\n" } else { ",\n" });
        out.push_str("    ");
        json_string(out, key);
        out.push_str(": {");
        write(out, value, 6);
        out.push_str("\n    }");
    }
}

fn write_file(out: &mut String, entry: &ManifestFile, indent: usize) {
    field_string(out, "virtualPath", &entry.virtual_path, indent, true);
    write_object_fields(out, &entry.object_entry(), indent, false);
    if !entry.dependencies.is_empty() {
        out.push_str(",\n      \"dependencies\": [");
        for (index, dependency) in entry.dependencies.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            json_string(out, dependency);
        }
        out.push(']');
    }
}

fn write_font(out: &mut String, entry: &ManifestFont, indent: usize) {
    write_object_fields(out, &entry.object_entry(), indent, true);
    field_string(out, "container", &entry.container, indent, false);
    if let Some(provenance) = &entry.provenance {
        field_string(out, "provenance", provenance, indent, false);
    }
}

fn write_format(out: &mut String, entry: &ManifestFormat, indent: usize) {
    write_object_fields(
        out,
        &ObjectEntry {
            object: entry.object.clone(),
            sha256: entry.sha256.clone(),
            bytes: entry.bytes,
        },
        indent,
        true,
    );
    field_string(out, "engine", &entry.engine, indent, false);
    field_string(out, "engineVersion", &entry.engine_version, indent, false);
    field_number(out, "formatSchema", u64::from(entry.format_schema), indent);
    field_string(
        out,
        "sourceDistribution",
        &entry.source_distribution,
        indent,
        false,
    );
    field_string(
        out,
        "sourceManifestSha256",
        &entry.source_manifest_sha256,
        indent,
        false,
    );
    field_number(out, "sourceDateEpoch", entry.source_date_epoch, indent);
    if let Some(closure) = &entry.input_closure {
        out.push_str(",\n");
        out.push_str(&" ".repeat(indent));
        json_string(out, "inputClosure");
        out.push_str(": {");
        out.push('\n');
        out.push_str(&" ".repeat(indent + 2));
        json_string(out, "schema");
        out.push_str(": ");
        out.push_str(&closure.schema.to_string());
        out.push_str(",\n");
        out.push_str(&" ".repeat(indent + 2));
        json_string(out, "keys");
        out.push_str(": [");
        for (index, key) in closure.keys.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            json_string(out, key);
        }
        out.push_str("]\n");
        out.push_str(&" ".repeat(indent));
        out.push('}');
    }
}

fn write_object_fields(out: &mut String, entry: &ObjectEntry, indent: usize, first: bool) {
    field_string(out, "object", &entry.object, indent, first);
    field_string(out, "sha256", &entry.sha256, indent, false);
    field_number(out, "bytes", entry.bytes, indent);
}

fn field_string(out: &mut String, name: &str, value: &str, indent: usize, first: bool) {
    out.push_str(if first { "\n" } else { ",\n" });
    out.push_str(&" ".repeat(indent));
    json_string(out, name);
    out.push_str(": ");
    json_string(out, value);
}

fn field_number(out: &mut String, name: &str, value: u64, indent: usize) {
    out.push_str(",\n");
    out.push_str(&" ".repeat(indent));
    json_string(out, name);
    out.push_str(": ");
    out.push_str(&value.to_string());
}

pub(crate) fn json_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                write!(out, "\\u{:04x}", u32::from(character))
                    .expect("writing to String cannot fail");
            }
            character => out.push(character),
        }
    }
    out.push('"');
}
