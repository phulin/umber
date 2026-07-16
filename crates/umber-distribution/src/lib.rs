//! Strict, host-neutral distribution manifest parsing and object selection.

mod json;
mod manifest;
mod selection;

pub use manifest::{
    MANIFEST_SCHEMA, Manifest, ManifestFile, ManifestFont, ManifestFormat, ManifestParseError,
    ObjectEntry,
};
pub use selection::{
    AcquisitionJob, FileKind, FileRequestKey, FontRequestKey, JobRequirement, ManifestLogicalKey,
    ManifestMiss, ManifestRequest, Selection, SelectionError, select,
};

#[cfg(test)]
mod tests;
