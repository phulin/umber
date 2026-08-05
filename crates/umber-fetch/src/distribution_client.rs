//! Native distribution acquisition over the shared verified blob store.

use crate::fetch::agent;
use crate::manifest::fetch_manifest_with_agent;
use crate::{
    BatchFetchError, BlobStore, CacheError, FetchCancellation, FetchClient, FetchClientConfig,
    FetchRequest, FetchedObject, ManifestFetchError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquiredManifest {
    pub bytes: Vec<u8>,
    pub cache_hit: bool,
}

#[derive(Debug)]
pub enum DistributionClientError {
    Cache(CacheError),
    Manifest(ManifestFetchError),
}

impl std::fmt::Display for DistributionClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cache(error) => write!(formatter, "distribution cache failure: {error}"),
            Self::Manifest(error) => write!(formatter, "distribution manifest failure: {error}"),
        }
    }
}

impl std::error::Error for DistributionClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cache(error) => Some(error),
            Self::Manifest(error) => Some(error),
        }
    }
}

/// Acquisition façade that binds network retry to one verified persistent store.
#[derive(Clone, Debug)]
pub struct DistributionClient {
    store: BlobStore,
    fetch: FetchClient,
    manifest_agent: ureq::Agent,
}

impl DistributionClient {
    #[must_use]
    pub fn new(store: BlobStore, config: FetchClientConfig) -> Self {
        let manifest_agent = agent(config.timeout);
        Self {
            store,
            fetch: FetchClient::new(config),
            manifest_agent,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_agent(
        store: BlobStore,
        config: FetchClientConfig,
        transport: ureq::Agent,
    ) -> Self {
        Self {
            store,
            fetch: FetchClient::with_agent(config, transport.clone()),
            manifest_agent: transport,
        }
    }

    pub fn from_environment(config: FetchClientConfig) -> Result<Self, CacheError> {
        Ok(Self::new(BlobStore::from_environment()?, config))
    }

    #[must_use]
    pub fn store(&self) -> &BlobStore {
        &self.store
    }

    pub fn acquire_batch(
        &self,
        objects_base_url: &str,
        requests: &[FetchRequest],
        cancellation: &FetchCancellation,
    ) -> Result<Vec<FetchedObject>, BatchFetchError> {
        self.fetch
            .fetch_batch_cancellable(&self.store, objects_base_url, requests, cancellation)
    }

    pub fn load_manifest(&self, digest: &str) -> Result<Option<Vec<u8>>, CacheError> {
        self.store.load_manifest(digest)
    }

    pub fn persist_manifest(&self, digest: &str, bytes: &[u8]) -> Result<(), CacheError> {
        self.store.store_manifest(digest, bytes)
    }

    pub fn acquire_manifest(
        &self,
        url: &str,
        digest: &str,
        cancellation: &FetchCancellation,
    ) -> Result<AcquiredManifest, DistributionClientError> {
        if let Some(bytes) = self
            .store
            .load_manifest(digest)
            .map_err(DistributionClientError::Cache)?
        {
            return Ok(AcquiredManifest {
                bytes,
                cache_hit: true,
            });
        }
        let bytes = fetch_manifest_with_agent(url, digest, cancellation, &self.manifest_agent)
            .map_err(DistributionClientError::Manifest)?;
        self.store
            .store_manifest(digest, &bytes)
            .map_err(DistributionClientError::Cache)?;
        Ok(AcquiredManifest {
            bytes,
            cache_hit: false,
        })
    }
}
