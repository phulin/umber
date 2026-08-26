use umber_distribution::{
    JobRequirement, ManifestMiss, ManifestRequest, ShardedManifestRoot, prepare_batch, verify_batch,
};
use wasm_bindgen::{JsCast as _, prelude::*};

use crate::{
    JsCatalogBatchPlan, JsCatalogKeys, JsCatalogPreparedBatch, JsCatalogRawShards, JsNamedFormat,
    wire,
};

fn boundary_error(error: impl ToString) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn requests(keys: &JsCatalogKeys) -> Result<Vec<ManifestRequest>, JsValue> {
    serde_wasm_bindgen::from_value::<Vec<String>>(keys.into())
        .map_err(boundary_error)?
        .into_iter()
        .map(|key| ManifestRequest::from_manifest_key(&key).map_err(boundary_error))
        .collect()
}

/// Parses every supported root and returns its canonical value plus the unique
/// verified shard objects required for one ordered request batch.
#[wasm_bindgen(js_name = catalogPrepareBatch)]
pub fn prepare(root_text: &str, keys: &JsCatalogKeys) -> Result<JsCatalogPreparedBatch, JsValue> {
    let requests = requests(keys)?;
    let (root, indexes) = prepare_batch(root_text, &requests).map_err(boundary_error)?;
    let shards = indexes
        .into_iter()
        .map(|index| {
            let ahash64 = root
                .shard_digest(index)
                .expect("prepared shard index exists")
                .to_owned();
            wire::CatalogShardDto {
                index,
                object: format!("ahash64-v1-{ahash64}"),
                ahash64,
            }
        })
        .collect();
    Ok(wire::to_js_value(&wire::CatalogPreparedBatchDto {
        root: root.to_json(),
        shards,
    })?
    .unchecked_into())
}

/// Verifies exact shard bytes against the root and returns the complete
/// required-before-hint transport plan. JavaScript owns only fetching.
#[wasm_bindgen(js_name = catalogPlanBatch)]
pub fn plan(
    root_text: &str,
    raw_shards: &JsCatalogRawShards,
    keys: &JsCatalogKeys,
) -> Result<JsCatalogBatchPlan, JsValue> {
    let requests = requests(keys)?;
    let raw: Vec<wire::CatalogRawShardDto> =
        serde_wasm_bindgen::from_value(raw_shards.into()).map_err(boundary_error)?;
    let borrowed = raw
        .iter()
        .map(|shard| (shard.index, shard.text.as_str()))
        .collect::<Vec<_>>();
    let batch = verify_batch(root_text, &borrowed, &requests).map_err(boundary_error)?;

    let jobs = batch
        .selection
        .jobs
        .iter()
        .map(|job| {
            let key = job.manifest_key.as_str();
            let index = umber_distribution::shard_index_for_key(key, batch.root.shard_bits)
                .expect("verified job key is canonical");
            let shard = &batch.shards[&index];
            let mut entry = wire::CatalogJobEntryDto {
                object: job.object.object.clone(),
                ahash64: job.object.ahash64.clone(),
                bytes: wire::SafeInteger::new(job.object.bytes).map_err(boundary_error)?,
                virtual_path: None,
                container: None,
                program_identity: None,
                provenance: None,
                font_key: None,
                unicode_map: None,
                fallback: None,
            };
            let (kind, request_index) = match &job.request {
                ManifestRequest::File(_) => {
                    entry.virtual_path = job.virtual_path.clone();
                    (
                        wire::CatalogJobKindDto::File,
                        request_position(&requests, &job.request),
                    )
                }
                ManifestRequest::Font(_) => {
                    let record = &shard.fonts[key];
                    entry.container = Some(container(&record.container)?);
                    entry.program_identity = record.declared_program_identity.clone();
                    entry.provenance = Some(record.provenance.identity.clone());
                    (
                        wire::CatalogJobKindDto::Font,
                        request_position(&requests, &job.request),
                    )
                }
                ManifestRequest::LegacyMapping(_) => {
                    let record = &shard.legacy_mappings[key];
                    entry.container = Some(container(&record.container)?);
                    entry.program_identity = record.declared_program_identity.clone();
                    entry.provenance = Some(record.provenance.identity.clone());
                    entry.font_key = Some(record.font_request.manifest_key().to_string());
                    entry.unicode_map = Some(record.unicode_map.clone());
                    entry.fallback = Some(match record.fallback.as_str() {
                        "error" => wire::FontMappingFallbackDto::Error,
                        "classic-tfm-exact" => wire::FontMappingFallbackDto::ClassicTfmExact,
                        _ => unreachable!("authenticated catalogue fallback is canonical"),
                    });
                    (
                        wire::CatalogJobKindDto::LegacyFontMapping,
                        request_position(&requests, &job.request),
                    )
                }
            };
            Ok(wire::CatalogJobDto {
                manifest_key: key.to_owned(),
                requirement: match job.requirement {
                    JobRequirement::Required => wire::CatalogJobRequirementDto::Required,
                    JobRequirement::DependencyHint => wire::CatalogJobRequirementDto::Hint,
                },
                kind,
                request_index,
                entry,
            })
        })
        .collect::<Result<Vec<_>, JsValue>>()?;
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
        .collect();
    Ok(wire::to_js_value(&wire::CatalogBatchPlanDto { jobs, misses })?.unchecked_into())
}

fn request_position(requests: &[ManifestRequest], request: &ManifestRequest) -> Option<u32> {
    requests
        .iter()
        .position(|candidate| candidate == request)
        .map(|index| u32::try_from(index).expect("catalog batch length is bounded"))
}

fn container(value: &str) -> Result<wire::FontContainerDto, JsValue> {
    match value {
        "woff2" => Ok(wire::FontContainerDto::Woff2),
        _ => Err(boundary_error(
            "authenticated catalogue container is unsupported",
        )),
    }
}

/// Selects one named format from a strictly parsed root. This keeps format
/// name, compatibility, and closure interpretation out of JavaScript.
#[wasm_bindgen(js_name = catalogSelectFormat)]
pub fn select_format(root_text: &str, name: &str) -> Result<JsNamedFormat, JsValue> {
    let root = ShardedManifestRoot::parse(root_text).map_err(boundary_error)?;
    let format = root
        .formats
        .get(name)
        .ok_or_else(|| boundary_error(format!("manifest has no format named {name}")))?;
    let dto = wire::NamedFormatDto {
        name: name.to_owned(),
        object: format.object.clone(),
        ahash64: format.ahash64.clone(),
        bytes: wire::SafeInteger::new(format.bytes).map_err(boundary_error)?,
        engine: format.engine.clone(),
        engine_version: format.engine_version.clone(),
        format_schema: format.format_schema,
        source_distribution: format.source_distribution.clone(),
        source_manifest_ahash64: format.source_manifest_ahash64.clone(),
        source_date_epoch: wire::SafeInteger::new(format.source_date_epoch)
            .map_err(boundary_error)?,
        input_closure: format
            .input_closure
            .as_ref()
            .map(|closure| wire::FormatInputClosureDto {
                schema: closure.schema,
                keys: closure.keys.clone(),
            }),
    };
    Ok(wire::to_js_value(&dto)?.unchecked_into())
}
