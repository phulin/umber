//! Native content-addressed cache and HTTPS acquisition for distribution objects.

mod cache;
mod distribution_client;
mod downloader;
mod fetch;
mod manifest;

pub use cache::{BlobStore, CacheError, CacheVerificationReport, ObjectCache, VerifiedBlobSpec};
pub use distribution_client::{AcquiredManifest, DistributionClient, DistributionClientError};
pub use fetch::{
    BatchFetchError, FetchCancellation, FetchClient, FetchClientConfig, FetchDiagnostic,
    FetchFailure, FetchRequest, FetchedObject,
};
pub use manifest::{ManifestFetchError, fetch_manifest, fetch_manifest_cancellable};

#[cfg(test)]
mod tests;
