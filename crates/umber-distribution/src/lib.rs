//! Strict, host-neutral distribution manifest parsing and object selection.

mod ahash64;
mod catalog;
mod html;
mod json;
mod manifest;
mod packed;
mod prefetch;
mod readiness;
mod selection;

pub use html::{
    FONT_RECORD_SCHEMA, FontManifestRecord, HTML_INDEX_SHARD_SCHEMA, HTML_SHARDED_ROOT_SCHEMA,
    LEGACY_MAPPING_RECORD_SCHEMA, LegacyMappingManifestRecord, LicenseRecord, ProvenanceRecord,
};
pub use manifest::{
    DependencyHint, FORMAT_INPUT_CLOSURE_SCHEMA, FormatInputClosure, INDEX_SHARD_SCHEMA,
    LEGACY_SHARDED_ROOT_SCHEMA, MANIFEST_SCHEMA, MAX_FORMAT_INPUTS, MAX_REQUEST_KEY_BYTES,
    MAX_SHARD_BITS, Manifest, ManifestFile, ManifestFont, ManifestFormat, ManifestParseError,
    ManifestShard, NamedFormat, ObjectEntry, SHARDED_ROOT_SCHEMA, ShardFile, ShardedManifestRoot,
};
pub use packed::{
    LEGACY_PACKED_SHARD_SCHEMA, PACKED_SHARD_SCHEMA, PackedDependency, PackedFileRecord,
    PackedRecord, PackedRecordKind, PackedShardError, ValidatedPackedShard, pack_shard,
    unpack_shard,
};
pub use prefetch::{
    LiteralHint, LiteralHintKind, LiteralHintLimits, PrefetchBudget, PrefetchCandidate,
    PrefetchClass, PrefetchEscalation, PrefetchFileKey, PrefetchPolicy, PrefetchPolicyMetrics,
    PrefetchRegionKey, PrefetchRequest, PrefetchSelection, extract_literal_hints,
    normalize_literal_hint_name, select_prefetch_group,
};
pub use readiness::{
    LookupManifest, LookupOutcome, LookupRecord, LookupRole, ManifestError, NegativeScope,
    PREFETCH_MANIFEST_SCHEMA, PrefetchIdentity, Readiness, ResolvedIdentity,
};
pub use selection::{
    AcquisitionJob, FeatureSetting, FileKind, FileRequestKey, FontRequestContext, FontRequestKey,
    JobRequirement, LegacyMappingRequestKey, ManifestLogicalKey, ManifestMiss, ManifestRequest,
    Selection, SelectionError, VariationCoordinate, VariationInstance, WritingDirection,
    select_packed_shards, select_shard, select_shards, shard_index, shard_index_for_key,
};

#[cfg(test)]
mod tests;
pub use catalog::{
    ShardedCatalog, VerifiedBatchPlan, assemble_sharded_catalog, prepare_batch, shard_manifest,
    shard_manifest_with_records, verify_batch,
};
