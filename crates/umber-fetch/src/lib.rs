//! Native content-addressed cache and HTTPS acquisition for distribution objects.

mod cache;
mod fetch;
mod manifest;

pub use cache::{CacheError, ObjectCache};
pub use fetch::{
    BatchFetchError, FetchClient, FetchClientConfig, FetchDiagnostic, FetchFailure, FetchRequest,
    FetchedObject,
};
pub use manifest::{ManifestFetchError, fetch_manifest};

#[cfg(test)]
mod tests;
