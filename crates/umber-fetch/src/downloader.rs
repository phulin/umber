//! Policy-parameterized bounded HTTPS download and verification.

use std::io::Read;
use std::str::FromStr;
use std::time::Duration;

use ureq::http::Uri;

use crate::FetchCancellation;
use crate::cache::hex_digest;

#[derive(Clone, Debug)]
pub(crate) struct VerifiedDownloader {
    agent: ureq::Agent,
}

#[derive(Clone, Debug)]
pub(crate) struct DownloadPolicy<'a> {
    pub(crate) subject: &'static str,
    pub(crate) length: LengthPolicy,
    pub(crate) expected_ahash64: &'a str,
    pub(crate) retries: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum LengthPolicy {
    Exact(u64),
    AtMost(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DownloadFailure {
    InvalidUrl(String),
    HttpStatus(u16),
    Transport(String),
    TooLarge { limit: u64 },
    LengthMismatch { expected: u64, actual: u64 },
    DigestMismatch { actual: String },
    Cancelled,
}

impl VerifiedDownloader {
    pub(crate) fn new(timeout: Duration) -> Self {
        Self::with_agent(agent(timeout))
    }

    pub(crate) fn with_agent(agent: ureq::Agent) -> Self {
        Self { agent }
    }

    pub(crate) fn download(
        &self,
        url: &str,
        policy: &DownloadPolicy<'_>,
        cancellation: &FetchCancellation,
    ) -> Result<Vec<u8>, DownloadFailure> {
        let url = parse_transport_url(url, policy.subject).map_err(DownloadFailure::InvalidUrl)?;
        self.download_uri(&url, policy, cancellation)
    }

    pub(crate) fn download_uri(
        &self,
        url: &Uri,
        policy: &DownloadPolicy<'_>,
        cancellation: &FetchCancellation,
    ) -> Result<Vec<u8>, DownloadFailure> {
        let mut last_failure = None;
        for attempt in 0..=policy.retries {
            check_cancelled(cancellation)?;
            match self.download_once(url, policy, cancellation) {
                Ok(bytes) => return Ok(bytes),
                Err(failure) => {
                    let retry = retryable(&failure) && attempt < policy.retries;
                    last_failure = Some(failure);
                    if !retry {
                        break;
                    }
                }
            }
        }
        Err(last_failure.expect("a verified download always makes one attempt"))
    }

    fn download_once(
        &self,
        url: &Uri,
        policy: &DownloadPolicy<'_>,
        cancellation: &FetchCancellation,
    ) -> Result<Vec<u8>, DownloadFailure> {
        let mut response = self
            .agent
            .get(url.clone())
            .call()
            .map_err(|error| DownloadFailure::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(DownloadFailure::HttpStatus(status.as_u16()));
        }
        let limit = policy.length.limit();
        if let Some(actual) = response.body().content_length() {
            match policy.length {
                LengthPolicy::Exact(expected) if actual > expected => {
                    return Err(DownloadFailure::LengthMismatch { expected, actual });
                }
                LengthPolicy::AtMost(_) if actual > limit => {
                    return Err(DownloadFailure::TooLarge { limit });
                }
                _ => {}
            }
        }

        let mut bytes =
            Vec::with_capacity(usize::try_from(limit.min(1024 * 1024)).unwrap_or(1024 * 1024));
        let mut body = response.body_mut().as_reader();
        let mut reader = body.by_ref().take(limit.saturating_add(1));
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            check_cancelled(cancellation)?;
            let count = reader
                .read(&mut chunk)
                .map_err(|error| DownloadFailure::Transport(error.to_string()))?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        check_cancelled(cancellation)?;
        match policy.length {
            LengthPolicy::Exact(expected) if bytes.len() as u64 != expected => {
                return Err(DownloadFailure::LengthMismatch {
                    expected,
                    actual: bytes.len() as u64,
                });
            }
            LengthPolicy::AtMost(limit) if bytes.len() as u64 > limit => {
                return Err(DownloadFailure::TooLarge { limit });
            }
            _ => {}
        }
        let actual = hex_digest(&bytes);
        if actual != policy.expected_ahash64 {
            return Err(DownloadFailure::DigestMismatch { actual });
        }
        Ok(bytes)
    }
}

impl LengthPolicy {
    fn limit(self) -> u64 {
        match self {
            Self::Exact(bytes) | Self::AtMost(bytes) => bytes,
        }
    }
}

pub(crate) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::new_with_config(agent_config(timeout))
}

pub(crate) fn agent_config(timeout: Duration) -> ureq::config::Config {
    ureq::Agent::config_builder()
        .timeout_connect(Some(timeout))
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
}

pub(crate) fn parse_transport_url(value: &str, subject: &str) -> Result<Uri, String> {
    let url = Uri::from_str(value).map_err(|error| error.to_string())?;
    let Some(scheme) = url.scheme_str() else {
        return Err("URL must be absolute".into());
    };
    let Some(host) = url.host() else {
        return Err("URL must include a host".into());
    };
    if scheme == "https" {
        return Ok(url);
    }
    if scheme == "http"
        && host
            .parse::<std::net::IpAddr>()
            .ok()
            .is_some_and(|address| address.is_loopback())
    {
        return Ok(url);
    }
    Err(format!(
        "{subject} must use HTTPS (HTTP is allowed only for loopback tests)"
    ))
}

fn check_cancelled(cancellation: &FetchCancellation) -> Result<(), DownloadFailure> {
    if cancellation.is_cancelled() {
        Err(DownloadFailure::Cancelled)
    } else {
        Ok(())
    }
}

fn retryable(failure: &DownloadFailure) -> bool {
    match failure {
        DownloadFailure::Transport(_)
        | DownloadFailure::LengthMismatch { .. }
        | DownloadFailure::DigestMismatch { .. } => true,
        DownloadFailure::HttpStatus(status) => matches!(*status, 408 | 429 | 500..=599),
        DownloadFailure::InvalidUrl(_)
        | DownloadFailure::TooLarge { .. }
        | DownloadFailure::Cancelled => false,
    }
}
