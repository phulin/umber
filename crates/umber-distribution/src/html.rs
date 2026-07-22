use std::collections::BTreeMap;

use crate::json::Value;
use crate::manifest::{
    ManifestParseError, ObjectEntry, finish, json_string, object, parse_object_entry, string, take,
    u32_value, validate_digest,
};
use crate::selection::{FontRequestKey, LegacyMappingRequestKey};

pub const HTML_SHARDED_ROOT_SCHEMA: u32 = 4;
pub const HTML_INDEX_SHARD_SCHEMA: u32 = 2;
pub const FONT_RECORD_SCHEMA: u32 = 1;
pub const LEGACY_MAPPING_RECORD_SCHEMA: u32 = 1;
const POLICY_VERSION: u32 = 1;
const MAX_METADATA_BYTES: usize = 4096;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
const MAX_UNICODE_MAPPING_BYTES: usize = 64;
const MAX_RECORDS_PER_SHARD: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvenanceRecord {
    pub identity: String,
    pub upstream: String,
    pub upstream_version: String,
    pub source_url: String,
    pub conversion_tool: String,
    pub conversion_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LicenseRecord {
    pub identity: String,
    pub object: ObjectEntry,
    pub spdx: String,
    pub embeddable: bool,
    pub redistributable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontManifestRecord {
    pub schema: u32,
    pub request: FontRequestKey,
    pub object: ObjectEntry,
    pub container: String,
    pub declared_program_identity: Option<String>,
    pub feature_policy_version: u32,
    pub provenance: ProvenanceRecord,
    pub license: LicenseRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyMappingManifestRecord {
    pub schema: u32,
    pub request: LegacyMappingRequestKey,
    pub font_request: FontRequestKey,
    pub object: ObjectEntry,
    pub container: String,
    pub declared_program_identity: Option<String>,
    pub unicode_map: Vec<Option<String>>,
    pub mapping_version: u32,
    pub fontdimen_version: u32,
    pub feature_policy_version: u32,
    pub fallback: String,
    pub provenance: ProvenanceRecord,
    pub license: LicenseRecord,
}

pub(crate) fn parse_font_records(
    value: Value,
) -> Result<BTreeMap<String, FontManifestRecord>, ManifestParseError> {
    let values = object(value, "fonts")?;
    if values.len() > MAX_RECORDS_PER_SHARD {
        return Err(ManifestParseError::new(
            "font shard contains too many records",
        ));
    }
    let mut records = BTreeMap::new();
    for (key, value) in values {
        let request = FontRequestKey::from_manifest_key(&key)
            .map_err(|error| ManifestParseError::new(error.to_string()))?;
        let mut fields = object(value, &format!("font record {key}"))?;
        let schema = record_schema(&mut fields, &key, FONT_RECORD_SCHEMA, "font record")?;
        let object = parse_object_entry(&mut fields, &format!("font record {key}"))?;
        let container = woff2(&mut fields, &key)?;
        let declared_program_identity = optional_digest(&mut fields, "programIdentity", &key)?;
        let feature_policy_version = policy_version(&mut fields, "featurePolicyVersion", &key)?;
        let provenance = parse_provenance(take(&mut fields, "provenance", &key)?, &key)?;
        let license = parse_license(take(&mut fields, "license", &key)?, &key)?;
        finish(fields, &format!("font record {key}"))?;
        records.insert(
            key,
            FontManifestRecord {
                schema,
                request,
                object,
                container,
                declared_program_identity,
                feature_policy_version,
                provenance,
                license,
            },
        );
    }
    Ok(records)
}

pub(crate) fn parse_legacy_mapping_records(
    value: Value,
) -> Result<BTreeMap<String, LegacyMappingManifestRecord>, ManifestParseError> {
    let values = object(value, "legacyMappings")?;
    if values.len() > MAX_RECORDS_PER_SHARD {
        return Err(ManifestParseError::new(
            "legacy mapping shard contains too many records",
        ));
    }
    let mut records = BTreeMap::new();
    for (key, value) in values {
        let request = LegacyMappingRequestKey::from_manifest_key(&key)
            .map_err(|error| ManifestParseError::new(error.to_string()))?;
        let mut fields = object(value, &format!("legacy mapping {key}"))?;
        let schema = record_schema(
            &mut fields,
            &key,
            LEGACY_MAPPING_RECORD_SCHEMA,
            "legacy mapping record",
        )?;
        let tfm_sha256 = string(take(&mut fields, "tfmSha256", &key)?, "tfmSha256")?;
        validate_digest(&tfm_sha256, "TFM digest")?;
        if tfm_sha256 != request.tfm_sha256() {
            return Err(ManifestParseError::new(format!(
                "legacy mapping {key} TFM digest does not match its request key"
            )));
        }
        let font_key = string(take(&mut fields, "fontKey", &key)?, "fontKey")?;
        let font_request = FontRequestKey::from_manifest_key(&font_key)
            .map_err(|error| ManifestParseError::new(error.to_string()))?;
        let object = parse_object_entry(&mut fields, &format!("legacy mapping {key}"))?;
        let container = woff2(&mut fields, &key)?;
        let declared_program_identity = optional_digest(&mut fields, "programIdentity", &key)?;
        let unicode_map = parse_unicode_map(take(&mut fields, "unicodeMap", &key)?, &key)?;
        let mapping_version = policy_version(&mut fields, "mappingVersion", &key)?;
        let fontdimen_version = policy_version(&mut fields, "fontdimenVersion", &key)?;
        let feature_policy_version = policy_version(&mut fields, "featurePolicyVersion", &key)?;
        let fallback = string(take(&mut fields, "fallback", &key)?, "fallback")?;
        if !matches!(fallback.as_str(), "classic-tfm-exact" | "error") {
            return Err(ManifestParseError::new(format!(
                "legacy mapping {key} has unsupported fallback {fallback:?}"
            )));
        }
        let provenance = parse_provenance(take(&mut fields, "provenance", &key)?, &key)?;
        let license = parse_license(take(&mut fields, "license", &key)?, &key)?;
        finish(fields, &format!("legacy mapping {key}"))?;
        records.insert(
            key,
            LegacyMappingManifestRecord {
                schema,
                request,
                font_request,
                object,
                container,
                declared_program_identity,
                unicode_map,
                mapping_version,
                fontdimen_version,
                feature_policy_version,
                fallback,
                provenance,
                license,
            },
        );
    }
    Ok(records)
}

fn record_schema(
    fields: &mut BTreeMap<String, Value>,
    key: &str,
    expected: u32,
    family: &str,
) -> Result<u32, ManifestParseError> {
    let schema = u32_value(take(fields, "schema", key)?, "record schema")?;
    if schema != expected {
        return Err(ManifestParseError::new(format!(
            "unsupported {family} schema {schema}; expected {expected}"
        )));
    }
    Ok(schema)
}

fn woff2(fields: &mut BTreeMap<String, Value>, key: &str) -> Result<String, ManifestParseError> {
    let container = string(take(fields, "container", key)?, "container")?;
    if container != "woff2" {
        return Err(ManifestParseError::new(format!(
            "record {key} container must be woff2"
        )));
    }
    Ok(container)
}

fn optional_digest(
    fields: &mut BTreeMap<String, Value>,
    name: &str,
    key: &str,
) -> Result<Option<String>, ManifestParseError> {
    fields
        .remove(name)
        .map(|value| {
            let digest = string(value, name)?;
            validate_digest(&digest, name)?;
            Ok(digest)
        })
        .transpose()
        .map_err(|error: ManifestParseError| ManifestParseError::new(format!("{key}: {error}")))
}

fn policy_version(
    fields: &mut BTreeMap<String, Value>,
    name: &str,
    key: &str,
) -> Result<u32, ManifestParseError> {
    let version = u32_value(take(fields, name, key)?, name)?;
    if version != POLICY_VERSION {
        return Err(ManifestParseError::new(format!(
            "unsupported {name} {version}; expected {POLICY_VERSION}"
        )));
    }
    Ok(version)
}

fn parse_provenance(value: Value, key: &str) -> Result<ProvenanceRecord, ManifestParseError> {
    let mut fields = object(value, &format!("provenance for {key}"))?;
    let identity = digest_field(&mut fields, "identity", key)?;
    let upstream = metadata_field(&mut fields, "upstream", key)?;
    let upstream_version = metadata_field(&mut fields, "upstreamVersion", key)?;
    let source_url = metadata_field(&mut fields, "sourceUrl", key)?;
    if !source_url.contains("://") {
        return Err(ManifestParseError::new(format!(
            "invalid provenance source URL for {key}"
        )));
    }
    let conversion_tool = metadata_field(&mut fields, "conversionTool", key)?;
    let conversion_version = metadata_field(&mut fields, "conversionVersion", key)?;
    finish(fields, &format!("provenance for {key}"))?;
    Ok(ProvenanceRecord {
        identity,
        upstream,
        upstream_version,
        source_url,
        conversion_tool,
        conversion_version,
    })
}

fn parse_license(value: Value, key: &str) -> Result<LicenseRecord, ManifestParseError> {
    let mut fields = object(value, &format!("license for {key}"))?;
    let identity = digest_field(&mut fields, "identity", key)?;
    let object = parse_object_entry(&mut fields, &format!("license for {key}"))?;
    if object.bytes == 0 || object.bytes > MAX_LICENSE_BYTES {
        return Err(ManifestParseError::new(format!(
            "invalid license length for {key}"
        )));
    }
    let spdx = metadata_field(&mut fields, "spdx", key)?;
    let embeddable = bool_field(&mut fields, "embeddable", key)?;
    let redistributable = bool_field(&mut fields, "redistributable", key)?;
    if !embeddable || !redistributable {
        return Err(ManifestParseError::new(format!(
            "record {key} lacks affirmative embedding and redistribution authority"
        )));
    }
    finish(fields, &format!("license for {key}"))?;
    Ok(LicenseRecord {
        identity,
        object,
        spdx,
        embeddable,
        redistributable,
    })
}

fn digest_field(
    fields: &mut BTreeMap<String, Value>,
    name: &str,
    key: &str,
) -> Result<String, ManifestParseError> {
    let value = string(take(fields, name, key)?, name)?;
    validate_digest(&value, name)?;
    Ok(value)
}

fn metadata_field(
    fields: &mut BTreeMap<String, Value>,
    name: &str,
    key: &str,
) -> Result<String, ManifestParseError> {
    let value = string(take(fields, name, key)?, name)?;
    if value.is_empty() || value.len() > MAX_METADATA_BYTES || value.chars().any(char::is_control) {
        return Err(ManifestParseError::new(format!("invalid {name} for {key}")));
    }
    Ok(value)
}

fn bool_field(
    fields: &mut BTreeMap<String, Value>,
    name: &str,
    key: &str,
) -> Result<bool, ManifestParseError> {
    match take(fields, name, key)? {
        Value::Bool(value) => Ok(value),
        _ => Err(ManifestParseError::new(format!(
            "{name} for {key} must be boolean"
        ))),
    }
}

fn parse_unicode_map(value: Value, key: &str) -> Result<Vec<Option<String>>, ManifestParseError> {
    let Value::Array(values) = value else {
        return Err(ManifestParseError::new(format!(
            "Unicode map for {key} must be an array"
        )));
    };
    if values.len() != 256 {
        return Err(ManifestParseError::new(format!(
            "Unicode map for {key} must contain exactly 256 entries"
        )));
    }
    values
        .into_iter()
        .map(|value| match value {
            Value::Null => Ok(None),
            Value::String(value)
                if !value.is_empty()
                    && value.len() <= MAX_UNICODE_MAPPING_BYTES
                    && !value.chars().any(char::is_control) =>
            {
                Ok(Some(value))
            }
            _ => Err(ManifestParseError::new(format!(
                "invalid Unicode map entry for {key}"
            ))),
        })
        .collect()
}

pub(crate) fn write_font_records(out: &mut String, records: &BTreeMap<String, FontManifestRecord>) {
    if records.is_empty() {
        return;
    }
    out.push_str(",\"fonts\":{");
    for (index, (key, record)) in records.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        json_string(out, key);
        out.push_str(":{");
        write_record_prefix(
            out,
            record.schema,
            &record.object,
            &record.container,
            record.declared_program_identity.as_deref(),
        );
        out.push_str(",\"featurePolicyVersion\":");
        out.push_str(&record.feature_policy_version.to_string());
        write_provenance(out, &record.provenance);
        write_license(out, &record.license);
        out.push('}');
    }
    out.push('}');
}

pub(crate) fn write_legacy_mapping_records(
    out: &mut String,
    records: &BTreeMap<String, LegacyMappingManifestRecord>,
) {
    if records.is_empty() {
        return;
    }
    out.push_str(",\"legacyMappings\":{");
    for (index, (key, record)) in records.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        json_string(out, key);
        out.push_str(":{");
        out.push_str("\"schema\":");
        out.push_str(&record.schema.to_string());
        out.push_str(",\"tfmSha256\":");
        json_string(out, record.request.tfm_sha256());
        out.push_str(",\"fontKey\":");
        json_string(out, &record.font_request.manifest_key().to_string());
        write_object(out, &record.object);
        out.push_str(",\"container\":");
        json_string(out, &record.container);
        if let Some(identity) = &record.declared_program_identity {
            out.push_str(",\"programIdentity\":");
            json_string(out, identity);
        }
        out.push_str(",\"unicodeMap\":[");
        for (slot, value) in record.unicode_map.iter().enumerate() {
            if slot > 0 {
                out.push(',');
            }
            if let Some(value) = value {
                json_string(out, value);
            } else {
                out.push_str("null");
            }
        }
        out.push(']');
        out.push_str(",\"mappingVersion\":");
        out.push_str(&record.mapping_version.to_string());
        out.push_str(",\"fontdimenVersion\":");
        out.push_str(&record.fontdimen_version.to_string());
        out.push_str(",\"featurePolicyVersion\":");
        out.push_str(&record.feature_policy_version.to_string());
        out.push_str(",\"fallback\":");
        json_string(out, &record.fallback);
        write_provenance(out, &record.provenance);
        write_license(out, &record.license);
        out.push('}');
    }
    out.push('}');
}

fn write_record_prefix(
    out: &mut String,
    schema: u32,
    object: &ObjectEntry,
    container: &str,
    program: Option<&str>,
) {
    out.push_str("\"schema\":");
    out.push_str(&schema.to_string());
    write_object(out, object);
    out.push_str(",\"container\":");
    json_string(out, container);
    if let Some(program) = program {
        out.push_str(",\"programIdentity\":");
        json_string(out, program);
    }
}

fn write_object(out: &mut String, object: &ObjectEntry) {
    out.push_str(",\"object\":");
    json_string(out, &object.object);
    out.push_str(",\"sha256\":");
    json_string(out, &object.sha256);
    out.push_str(",\"bytes\":");
    out.push_str(&object.bytes.to_string());
}

fn write_provenance(out: &mut String, value: &ProvenanceRecord) {
    out.push_str(",\"provenance\":{");
    for (index, (name, field)) in [
        ("identity", value.identity.as_str()),
        ("upstream", value.upstream.as_str()),
        ("upstreamVersion", value.upstream_version.as_str()),
        ("sourceUrl", value.source_url.as_str()),
        ("conversionTool", value.conversion_tool.as_str()),
        ("conversionVersion", value.conversion_version.as_str()),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            out.push(',');
        }
        json_string(out, name);
        out.push(':');
        json_string(out, field);
    }
    out.push('}');
}

fn write_license(out: &mut String, value: &LicenseRecord) {
    out.push_str(",\"license\":{\"identity\":");
    json_string(out, &value.identity);
    write_object(out, &value.object);
    out.push_str(",\"spdx\":");
    json_string(out, &value.spdx);
    out.push_str(",\"embeddable\":");
    out.push_str(if value.embeddable { "true" } else { "false" });
    out.push_str(",\"redistributable\":");
    out.push_str(if value.redistributable {
        "true"
    } else {
        "false"
    });
    out.push('}');
}
