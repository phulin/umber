use std::error::Error;
use std::fmt;
use std::io::Read;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::FetchCancellation;
use crate::fetch::{agent, parse_transport_url};

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
    expected_sha256: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ManifestFetchError> {
    fetch_manifest_cancellable(url, expected_sha256, timeout, &FetchCancellation::new())
}

/// Downloads and verifies a manifest while observing cooperative cancellation.
pub fn fetch_manifest_cancellable(
    url: &str,
    expected_sha256: &str,
    timeout: Duration,
    cancellation: &FetchCancellation,
) -> Result<Vec<u8>, ManifestFetchError> {
    if cancellation.is_cancelled() {
        return Err(ManifestFetchError::Cancelled);
    }
    let url = parse_transport_url(url, "manifests").map_err(ManifestFetchError::InvalidUrl)?;
    let mut response = agent(timeout)
        .get(url)
        .call()
        .map_err(|error| ManifestFetchError::Transport(error.to_string()))?;
    if !response.status().is_success() {
        return Err(ManifestFetchError::HttpStatus(response.status().as_u16()));
    }
    if response
        .body()
        .content_length()
        .is_some_and(|length| length > MAX_MANIFEST_BYTES)
    {
        return Err(ManifestFetchError::TooLarge {
            limit: MAX_MANIFEST_BYTES,
        });
    }
    let mut bytes = Vec::new();
    let mut body = response.body_mut().as_reader();
    let mut reader = body.by_ref().take(MAX_MANIFEST_BYTES + 1);
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        if cancellation.is_cancelled() {
            return Err(ManifestFetchError::Cancelled);
        }
        let count = reader
            .read(&mut chunk)
            .map_err(|error| ManifestFetchError::Transport(error.to_string()))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if cancellation.is_cancelled() {
        return Err(ManifestFetchError::Cancelled);
    }
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ManifestFetchError::TooLarge {
            limit: MAX_MANIFEST_BYTES,
        });
    }
    let actual = hex_digest(&bytes);
    if actual != expected_sha256 {
        return Err(ManifestFetchError::DigestMismatch {
            expected: expected_sha256.to_owned(),
            actual,
        });
    }
    Ok(bytes)
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}
