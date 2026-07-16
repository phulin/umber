use std::error::Error;
use std::fmt;
use std::io::Read;
use std::time::Duration;

use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestFetchError {
    InvalidUrl(String),
    HttpStatus(u16),
    Transport(String),
    TooLarge { limit: u64 },
    DigestMismatch { expected: String, actual: String },
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
    let url = reqwest::Url::parse(url)
        .map_err(|error| ManifestFetchError::InvalidUrl(error.to_string()))?;
    if url.scheme() != "https"
        && !(url.scheme() == "http"
            && url
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|address| address.is_loopback()))
    {
        return Err(ManifestFetchError::InvalidUrl(
            "manifests must use HTTPS (HTTP is allowed only for loopback tests)".into(),
        ));
    }
    let response = Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .map_err(|error| ManifestFetchError::Transport(error.to_string()))?
        .get(url)
        .send()
        .map_err(|error| ManifestFetchError::Transport(error.to_string()))?;
    if !response.status().is_success() {
        return Err(ManifestFetchError::HttpStatus(response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANIFEST_BYTES)
    {
        return Err(ManifestFetchError::TooLarge {
            limit: MAX_MANIFEST_BYTES,
        });
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| ManifestFetchError::Transport(error.to_string()))?;
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
