//! Native content-addressed cache and HTTPS acquisition for distribution objects.

mod cache;
mod fetch;
mod format_cache;
mod manifest;

pub use cache::{CacheError, ObjectCache};
pub use fetch::{
    BatchFetchError, FetchCancellation, FetchClient, FetchClientConfig, FetchDiagnostic,
    FetchFailure, FetchRequest, FetchedObject,
};
pub use format_cache::{
    FormatCacheClock, FormatCacheError, FormatCacheIdentity, FormatCacheStore, FormatEngineMode,
    FormatFingerprint, ValidatedFormatImage,
};
pub use manifest::{ManifestFetchError, fetch_manifest, fetch_manifest_cancellable};

#[cfg(test)]
mod tests;
