use js_sys::Array;
use serde_json::json;
use umber_distribution::{
    FileRequestKey, FontRequestKey, JobRequirement, LegacyMappingRequestKey, ManifestRequest,
    ManifestShard, ShardedManifestRoot, select_shard, shard_index_for_key,
};
use wasm_bindgen::prelude::*;

fn boundary_error(error: impl ToString) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Parses every supported published root schema and returns its canonical wire value.
#[wasm_bindgen(js_name = catalogValidateRoot)]
pub fn validate_root(text: &str) -> Result<String, JsValue> {
    ShardedManifestRoot::parse(text)
        .map(|root| root.to_json())
        .map_err(boundary_error)
}

/// Parses and authenticates one shard against its already validated root identity.
#[wasm_bindgen(js_name = catalogValidateShard)]
pub fn validate_shard(root_text: &str, shard_text: &str, index: u32) -> Result<String, JsValue> {
    let root = ShardedManifestRoot::parse(root_text).map_err(boundary_error)?;
    let shard = ManifestShard::parse(shard_text).map_err(boundary_error)?;
    shard
        .validate_identity(&root, index)
        .map_err(boundary_error)?;
    for key in shard
        .files
        .keys()
        .chain(shard.fonts.keys())
        .chain(shard.legacy_mappings.keys())
    {
        if shard_index_for_key(key, root.shard_bits).map_err(boundary_error)? != index {
            return Err(boundary_error(format!(
                "lookup key {key} is not in canonical shard {index}"
            )));
        }
    }
    Ok(shard.to_json())
}

#[wasm_bindgen(js_name = catalogShardIndex)]
pub fn shard_index(key: &str, shard_bits: u8) -> Result<u32, JsValue> {
    shard_index_for_key(key, shard_bits).map_err(boundary_error)
}

/// Returns the shared ordered required/hint/miss plan for canonical request keys.
#[wasm_bindgen(js_name = catalogSelectShard)]
pub fn select(shard_text: &str, keys: &Array) -> Result<String, JsValue> {
    let shard = ManifestShard::parse(shard_text).map_err(boundary_error)?;
    let requests = keys
        .iter()
        .map(|value| {
            let key = value
                .as_string()
                .ok_or_else(|| boundary_error("catalog request keys must be strings"))?;
            if key.starts_with("font:") {
                FontRequestKey::from_manifest_key(&key).map(ManifestRequest::Font)
            } else if key.starts_with("legacy-mapping:") {
                LegacyMappingRequestKey::from_manifest_key(&key).map(ManifestRequest::LegacyMapping)
            } else {
                FileRequestKey::from_manifest_key(&key).map(ManifestRequest::File)
            }
            .map_err(boundary_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let selection = select_shard(&shard, &requests);
    let jobs = selection
        .jobs
        .into_iter()
        .map(|job| {
            json!({
                "manifestKey": job.manifest_key.to_string(),
                "requirement": match job.requirement {
                    JobRequirement::Required => "required",
                    JobRequirement::DependencyHint => "hint",
                },
                "object": job.object.object,
                "sha256": job.object.sha256,
                "bytes": job.object.bytes,
                "virtualPath": job.virtual_path,
            })
        })
        .collect::<Vec<_>>();
    let misses = selection
        .misses
        .into_iter()
        .map(|miss| {
            let key = match miss {
                umber_distribution::ManifestMiss::File(key) => key.manifest_key(),
                umber_distribution::ManifestMiss::Font(key) => key.manifest_key(),
                umber_distribution::ManifestMiss::LegacyMapping(key) => key.manifest_key(),
            };
            key.to_string()
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({ "jobs": jobs, "misses": misses })).map_err(boundary_error)
}
