//! Strict, host-neutral distribution manifest parsing and object selection.

mod html;
mod json;
mod manifest;
mod selection;
mod sha256;

pub use html::{
    FONT_RECORD_SCHEMA, FontManifestRecord, HTML_INDEX_SHARD_SCHEMA, HTML_SHARDED_ROOT_SCHEMA,
    LEGACY_MAPPING_RECORD_SCHEMA, LegacyMappingManifestRecord, LicenseRecord, ProvenanceRecord,
};
pub use manifest::{
    DependencyHint, FORMAT_INPUT_CLOSURE_SCHEMA, FormatInputClosure, INDEX_SHARD_SCHEMA,
    LEGACY_SHARDED_ROOT_SCHEMA, MANIFEST_SCHEMA, MAX_FORMAT_INPUTS, MAX_REQUEST_KEY_BYTES,
    MAX_SHARD_BITS, Manifest, ManifestFile, ManifestFont, ManifestFormat, ManifestParseError,
    ManifestShard, ObjectEntry, SHARDED_ROOT_SCHEMA, ShardFile, ShardedManifestRoot,
};
pub use selection::{
    AcquisitionJob, FeatureSetting, FileKind, FileRequestKey, FontRequestContext, FontRequestKey,
    JobRequirement, LegacyMappingRequestKey, ManifestLogicalKey, ManifestMiss, ManifestRequest,
    Selection, SelectionError, VariationCoordinate, VariationInstance, WritingDirection, select,
    select_shard, shard_index,
};

#[cfg(test)]
mod tests;
