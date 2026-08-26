use std::collections::BTreeMap;

use umber_distribution::{
    JobRequirement, ManifestMiss, ManifestRequest, ShardedManifestRoot, ValidatedPackedShard,
    prepare_batch, select_packed_shards,
};
use umber_hash::{AHash64, HashDomain};
use wasm_bindgen::{JsCast as _, prelude::*};

use crate::{JsCatalogBatchPlan, JsCatalogKeys, JsCatalogPreparedBatch, JsNamedFormat, wire};

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

/// One browser distribution catalogue. Touched packed shard bytes stay in
/// Rust and pass complete validation exactly once before any lookup.
#[wasm_bindgen]
pub struct CatalogSession {
    root: ShardedManifestRoot,
    root_json: String,
    shards: BTreeMap<u32, ValidatedPackedShard>,
}

#[wasm_bindgen(js_name = catalogCreateSession)]
pub fn create_session(root_text: &str) -> Result<CatalogSession, JsValue> {
    let root = ShardedManifestRoot::parse(root_text).map_err(boundary_error)?;
    let root_json = root.to_json();
    Ok(CatalogSession {
        root,
        root_json,
        shards: BTreeMap::new(),
    })
}

#[wasm_bindgen]
impl CatalogSession {
    #[wasm_bindgen(js_name = prepareBatch)]
    pub fn prepare(&self, keys: &JsCatalogKeys) -> Result<JsCatalogPreparedBatch, JsValue> {
        let requests = requests(keys)?;
        let (_, indexes) = prepare_batch(&self.root_json, &requests).map_err(boundary_error)?;
        let shards = indexes
            .into_iter()
            .filter(|index| !self.shards.contains_key(index))
            .map(|index| {
                let ahash64 = self
                    .root
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
            root: self.root_json.clone(),
            shards,
        })?
        .unchecked_into())
    }

    #[wasm_bindgen(js_name = provideShard)]
    pub fn provide_shard(&mut self, index: u32, bytes: &[u8]) -> Result<(), JsValue> {
        let expected = self
            .root
            .shard_digest(index)
            .ok_or_else(|| boundary_error(format!("invalid index shard {index}")))?;
        let actual = AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex();
        if actual != expected {
            return Err(boundary_error(format!(
                "index shard {index} does not match its verified root digest"
            )));
        }
        if let Some(existing) = self.shards.get(&index) {
            if existing.bytes() == bytes {
                return Ok(());
            }
            return Err(boundary_error(format!(
                "index shard {index} conflicts with retained bytes"
            )));
        }
        let shard =
            ValidatedPackedShard::new(bytes.to_vec(), &self.root, index).map_err(boundary_error)?;
        self.shards.insert(index, shard);
        Ok(())
    }

    #[wasm_bindgen(js_name = planBatch)]
    pub fn plan(&self, keys: &JsCatalogKeys) -> Result<JsCatalogBatchPlan, JsValue> {
        let requests = requests(keys)?;
        let (_, indexes) = prepare_batch(&self.root_json, &requests).map_err(boundary_error)?;
        if let Some(index) = indexes
            .into_iter()
            .find(|index| !self.shards.contains_key(index))
        {
            return Err(boundary_error(format!(
                "index shard {index} has not been provided"
            )));
        }
        let selection = select_packed_shards(&self.shards, self.root.shard_bits, &requests);
        batch_plan(&self.root, &self.shards, &requests, &selection)
    }

    #[wasm_bindgen(js_name = selectFormat)]
    pub fn select_format(&self, name: &str) -> Result<JsNamedFormat, JsValue> {
        named_format(&self.root, name)
    }
}

fn batch_plan(
    root: &ShardedManifestRoot,
    shards: &BTreeMap<u32, ValidatedPackedShard>,
    requests: &[ManifestRequest],
    selection: &umber_distribution::Selection,
) -> Result<JsCatalogBatchPlan, JsValue> {
    let jobs = selection
        .jobs
        .iter()
        .map(|job| {
            let key = job.manifest_key.as_str();
            let index = umber_distribution::shard_index_for_key(key, root.shard_bits)
                .expect("verified job key is canonical");
            let packed = shards[&index]
                .lookup(key)
                .expect("selected packed record exists");
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
                        request_position(requests, &job.request),
                    )
                }
                ManifestRequest::Font(_) => {
                    let record = packed
                        .font()
                        .map_err(boundary_error)?
                        .expect("selected font record");
                    entry.container = Some(container(&record.container)?);
                    entry.program_identity = record.declared_program_identity;
                    entry.provenance = Some(record.provenance.identity);
                    (
                        wire::CatalogJobKindDto::Font,
                        request_position(requests, &job.request),
                    )
                }
                ManifestRequest::LegacyMapping(_) => {
                    let record = packed
                        .legacy_mapping()
                        .map_err(boundary_error)?
                        .expect("selected mapping record");
                    entry.container = Some(container(&record.container)?);
                    entry.program_identity = record.declared_program_identity;
                    entry.provenance = Some(record.provenance.identity);
                    entry.font_key = Some(record.font_request.manifest_key().to_string());
                    entry.unicode_map = Some(record.unicode_map);
                    entry.fallback = Some(match record.fallback.as_str() {
                        "error" => wire::FontMappingFallbackDto::Error,
                        "classic-tfm-exact" => wire::FontMappingFallbackDto::ClassicTfmExact,
                        _ => unreachable!("validated catalogue fallback is canonical"),
                    });
                    (
                        wire::CatalogJobKindDto::LegacyFontMapping,
                        request_position(requests, &job.request),
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
    let misses = selection
        .misses
        .iter()
        .map(|miss| {
            let request = match miss {
                ManifestMiss::File(key) => ManifestRequest::File(key.clone()),
                ManifestMiss::Font(key) => ManifestRequest::Font(key.clone()),
                ManifestMiss::LegacyMapping(key) => ManifestRequest::LegacyMapping(key.clone()),
            };
            request_position(requests, &request).expect("miss originates from request batch")
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
            "validated catalogue container is unsupported",
        )),
    }
}

fn named_format(root: &ShardedManifestRoot, name: &str) -> Result<JsNamedFormat, JsValue> {
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
