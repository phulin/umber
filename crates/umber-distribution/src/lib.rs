//! Strict, host-neutral distribution manifest parsing and object selection.

mod json;
mod manifest;
mod selection;

pub use manifest::{
    DependencyHint, INDEX_SHARD_SCHEMA, MANIFEST_SCHEMA, MAX_SHARD_BITS, Manifest, ManifestFile,
    ManifestFont, ManifestFormat, ManifestParseError, ManifestShard, ObjectEntry,
    SHARDED_ROOT_SCHEMA, ShardFile, ShardedManifestRoot,
};
pub use selection::{
    AcquisitionJob, FileKind, FileRequestKey, FontRequestKey, JobRequirement, ManifestLogicalKey,
    ManifestMiss, ManifestRequest, Selection, SelectionError, select,
};

#[cfg(test)]
mod tests;
