//! Native content-addressed cache and HTTPS acquisition for distribution objects.

mod cache;
mod fetch;

pub use cache::{CacheError, ObjectCache};
pub use fetch::{
    BatchFetchError, FetchClient, FetchClientConfig, FetchDiagnostic, FetchFailure, FetchRequest,
    FetchedObject,
};

#[cfg(test)]
mod tests;
