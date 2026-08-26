use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::FetchCancellation;
use crate::downloader::{DownloadFailure, DownloadPolicy, LengthPolicy, VerifiedDownloader};

const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestFetchError {
    InvalidUrl(String),
    HttpStatus(u16),
    Transport(String),
    TooLarge { limit: u64 },
    DigestMismatch { expected: String, actual: String },
    Cancelled,
}

impl fmt::Display for ManifestFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(message) => write!(f, "invalid manifest URL: {message}"),
            Self::HttpStatus(status) => write!(f, "manifest request returned HTTP {status}"),
            Self::Transport(message) => write!(f, "manifest transport failure: {message}"),
            Self::TooLarge { limit } => write!(f, "manifest exceeds the {limit}-byte limit"),
            Self::DigestMismatch { expected, actual } => write!(
                f,
                "manifest digest mismatch: expected {expected}, received {actual}"
            ),
            Self::Cancelled => f.write_str("manifest acquisition cancelled"),
        }
    }
}

impl Error for ManifestFetchError {}

/// Downloads one bounded HTTPS manifest and verifies the caller's trust pin.
pub fn fetch_manifest(
    url: &str,
    expected_ahash64: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ManifestFetchError> {
    fetch_manifest_cancellable(url, expected_ahash64, timeout, &FetchCancellation::new())
}

/// Downloads and verifies a manifest while observing cooperative cancellation.
pub fn fetch_manifest_cancellable(
    url: &str,
    expected_ahash64: &str,
    timeout: Duration,
    cancellation: &FetchCancellation,
) -> Result<Vec<u8>, ManifestFetchError> {
    fetch_manifest_with_downloader(
        url,
        expected_ahash64,
        cancellation,
        &VerifiedDownloader::new(timeout),
    )
}

pub(crate) fn fetch_manifest_with_downloader(
    url: &str,
    expected_ahash64: &str,
    cancellation: &FetchCancellation,
    downloader: &VerifiedDownloader,
) -> Result<Vec<u8>, ManifestFetchError> {
    downloader
        .download(
            url,
            &DownloadPolicy {
                subject: "manifests",
                length: LengthPolicy::AtMost(MAX_MANIFEST_BYTES),
                expected_ahash64,
                retries: 0,
            },
            cancellation,
        )
        .map_err(|failure| map_download_failure(failure, expected_ahash64))
}

#[cfg(test)]
pub(crate) fn fetch_manifest_with_test_agent(
    url: &str,
    expected_ahash64: &str,
    cancellation: &FetchCancellation,
    agent: &ureq::Agent,
) -> Result<Vec<u8>, ManifestFetchError> {
    fetch_manifest_with_downloader(
        url,
        expected_ahash64,
        cancellation,
        &VerifiedDownloader::with_agent(agent.clone()),
    )
}

fn map_download_failure(failure: DownloadFailure, expected_ahash64: &str) -> ManifestFetchError {
    match failure {
        DownloadFailure::InvalidUrl(message) => ManifestFetchError::InvalidUrl(message),
        DownloadFailure::HttpStatus(status) => ManifestFetchError::HttpStatus(status),
        DownloadFailure::Transport(message) => ManifestFetchError::Transport(message),
        DownloadFailure::TooLarge { limit } => ManifestFetchError::TooLarge { limit },
        DownloadFailure::LengthMismatch { expected, actual } => {
            debug_assert!(actual > expected);
            ManifestFetchError::TooLarge { limit: expected }
        }
        DownloadFailure::DigestMismatch { actual } => ManifestFetchError::DigestMismatch {
            expected: expected_ahash64.to_owned(),
            actual,
        },
        DownloadFailure::Cancelled => ManifestFetchError::Cancelled,
    }
}
