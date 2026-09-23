//! Native content-addressed cache and HTTPS acquisition for distribution objects.

mod cache;
mod distribution_client;
mod downloader;
mod fetch;
mod manifest;

pub use cache::{BlobStore, CacheError, CacheVerificationReport, VerifiedBlobSpec};
pub use distribution_client::{AcquiredManifest, DistributionClient, DistributionClientError};
pub use fetch::{
    BatchFetchError, FetchCancellation, FetchClient, FetchClientConfig, FetchDiagnostic,
    FetchFailure, FetchRequest, FetchedObject,
};
pub use manifest::ManifestFetchError;

#[cfg(test)]
mod tests;
