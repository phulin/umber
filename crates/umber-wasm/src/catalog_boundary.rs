use js_sys::Array;
use serde::Deserialize;
use serde_json::json;
use umber_distribution::{
    JobRequirement, ManifestMiss, ManifestRequest, ShardedManifestRoot, authenticate_batch,
    prepare_batch,
};
use wasm_bindgen::prelude::*;

fn boundary_error(error: impl ToString) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn requests(keys: &Array) -> Result<Vec<ManifestRequest>, JsValue> {
    keys.iter()
        .map(|value| {
            let key = value
                .as_string()
                .ok_or_else(|| boundary_error("catalog request keys must be strings"))?;
            ManifestRequest::from_manifest_key(&key).map_err(boundary_error)
        })
        .collect()
}

/// Parses every supported root and returns its canonical value plus the unique
/// authenticated shard objects required for one ordered request batch.
#[wasm_bindgen(js_name = catalogPrepareBatch)]
pub fn prepare(root_text: &str, keys: &Array) -> Result<String, JsValue> {
    let requests = requests(keys)?;
    let (root, indexes) = prepare_batch(root_text, &requests).map_err(boundary_error)?;
    let shards = indexes
        .into_iter()
        .map(|index| {
            let sha256 = root
                .shard_digest(index)
                .expect("prepared shard index exists");
            json!({
                "index": index,
                "object": format!("sha256-{sha256}"),
                "sha256": sha256,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({
        "root": root.to_json(),
        "shards": shards,
    }))
    .map_err(boundary_error)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawShard {
    index: u32,
    text: String,
}

/// Authenticates exact shard bytes against the root and returns the complete
/// required-before-hint transport plan. JavaScript owns only fetching.
#[wasm_bindgen(js_name = catalogPlanBatch)]
pub fn plan(root_text: &str, raw_shards_json: &str, keys: &Array) -> Result<String, JsValue> {
    let requests = requests(keys)?;
    let raw: Vec<RawShard> = serde_json::from_str(raw_shards_json).map_err(boundary_error)?;
    let borrowed = raw
        .iter()
        .map(|shard| (shard.index, shard.text.as_str()))
        .collect::<Vec<_>>();
    let batch = authenticate_batch(root_text, &borrowed, &requests).map_err(boundary_error)?;

    let jobs = batch
        .selection
        .jobs
        .iter()
        .map(|job| {
            let key = job.manifest_key.as_str();
            let index = umber_distribution::shard_index_for_key(key, batch.root.shard_bits)
                .expect("authenticated job key is canonical");
            let shard = &batch.shards[&index];
            let mut entry = json!({
                "object": job.object.object,
                "sha256": job.object.sha256,
                "bytes": job.object.bytes,
            });
            let (kind, request_index) = match &job.request {
                ManifestRequest::File(_) => {
                    entry["virtualPath"] = json!(job.virtual_path);
                    ("file", request_position(&requests, &job.request))
                }
                ManifestRequest::Font(_) => {
                    let record = &shard.fonts[key];
                    entry["container"] = json!(record.container);
                    if let Some(identity) = &record.declared_program_identity {
                        entry["programIdentity"] = json!(identity);
                    }
                    entry["provenance"] = json!(record.provenance.identity);
                    ("font", request_position(&requests, &job.request))
                }
                ManifestRequest::LegacyMapping(_) => {
                    let record = &shard.legacy_mappings[key];
                    entry["container"] = json!(record.container);
                    if let Some(identity) = &record.declared_program_identity {
                        entry["programIdentity"] = json!(identity);
                    }
                    entry["provenance"] = json!(record.provenance.identity);
                    entry["fontKey"] = json!(record.font_request.manifest_key().to_string());
                    entry["unicodeMap"] = json!(record.unicode_map);
                    entry["fallback"] = json!(record.fallback);
                    (
                        "legacy-font-mapping",
                        request_position(&requests, &job.request),
                    )
                }
            };
            json!({
                "manifestKey": key,
                "requirement": match job.requirement {
                    JobRequirement::Required => "required",
                    JobRequirement::DependencyHint => "hint",
                },
                "kind": kind,
                "requestIndex": request_index,
                "entry": entry,
            })
        })
        .collect::<Vec<_>>();
    let misses = batch
        .selection
        .misses
        .iter()
        .map(|miss| {
            let request = match miss {
                ManifestMiss::File(key) => ManifestRequest::File(key.clone()),
                ManifestMiss::Font(key) => ManifestRequest::Font(key.clone()),
                ManifestMiss::LegacyMapping(key) => ManifestRequest::LegacyMapping(key.clone()),
            };
            request_position(&requests, &request).expect("a miss originates from the request batch")
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({ "jobs": jobs, "misses": misses })).map_err(boundary_error)
}

fn request_position(requests: &[ManifestRequest], request: &ManifestRequest) -> Option<usize> {
    requests.iter().position(|candidate| candidate == request)
}

/// Selects one named format from a strictly parsed root. This keeps format
/// name, compatibility, and closure interpretation out of JavaScript.
#[wasm_bindgen(js_name = catalogSelectFormat)]
pub fn select_format(root_text: &str, name: &str) -> Result<String, JsValue> {
    let root = ShardedManifestRoot::parse(root_text).map_err(boundary_error)?;
    let format = root
        .formats
        .get(name)
        .ok_or_else(|| boundary_error(format!("manifest has no format named {name}")))?;
    serde_json::to_string(&json!({
        "name": name,
        "object": format.object,
        "sha256": format.sha256,
        "bytes": format.bytes,
        "engine": format.engine,
        "engineVersion": format.engine_version,
        "formatSchema": format.format_schema,
        "sourceDistribution": format.source_distribution,
        "sourceManifestSha256": format.source_manifest_sha256,
        "sourceDateEpoch": format.source_date_epoch,
        "inputClosure": format.input_closure.as_ref().map(|closure| json!({
            "schema": closure.schema,
            "keys": closure.keys,
        })),
    }))
    .map_err(boundary_error)
}
