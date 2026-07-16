use std::error::Error;
use std::fmt;
use std::io::Read;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use umber_distribution::ObjectEntry;

use crate::cache::hex_digest;
use crate::{CacheError, ObjectCache};

#[derive(Clone, Debug)]
pub struct FetchClientConfig {
    pub concurrency: NonZeroUsize,
    pub timeout: Duration,
    /// Number of retries after the first attempt.
    pub retries: usize,
}

impl Default for FetchClientConfig {
    fn default() -> Self {
        Self {
            concurrency: NonZeroUsize::new(4).expect("four is nonzero"),
            timeout: Duration::from_secs(30),
            retries: 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FetchRequest {
    pub request_key: String,
    pub object: ObjectEntry,
    pub max_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchedObject {
    pub request_key: String,
    pub object_digest: String,
    pub bytes: Vec<u8>,
    pub cache_hit: bool,
}

/// Shared cooperative cancellation for one native acquisition operation.
#[derive(Clone, Debug, Default)]
pub struct FetchCancellation {
    state: Arc<FetchCancellationState>,
}

#[derive(Debug, Default)]
struct FetchCancellationState {
    cancelled: AtomicBool,
    publication: Mutex<()>,
}

impl FetchCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        let _publication = self
            .state
            .publication
            .lock()
            .expect("cancellation publication mutex poisoned");
        self.state.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FetchFailure {
    Oversized { declared: u64, limit: u64 },
    InvalidUrl(String),
    HttpStatus(u16),
    Transport(String),
    LengthMismatch { expected: u64, actual: u64 },
    DigestMismatch { actual: String },
    Cache(String),
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchDiagnostic {
    pub request_key: String,
    pub object_digest: String,
    pub failure: FetchFailure,
}

impl fmt::Display for FetchDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fetch {} (object {}): ",
            self.request_key, self.object_digest
        )?;
        match &self.failure {
            FetchFailure::Oversized { declared, limit } => {
                write!(f, "declared size {declared} exceeds limit {limit}")
            }
            FetchFailure::InvalidUrl(message) => write!(f, "invalid object URL: {message}"),
            FetchFailure::HttpStatus(status) => write!(f, "HTTP status {status}"),
            FetchFailure::Transport(message) => write!(f, "transport failure: {message}"),
            FetchFailure::LengthMismatch { expected, actual } => {
                write!(f, "expected {expected} bytes, received {actual}")
            }
            FetchFailure::DigestMismatch { actual } => {
                write!(f, "digest mismatch (received {actual})")
            }
            FetchFailure::Cache(message) => write!(f, "cache failure: {message}"),
            FetchFailure::Cancelled => f.write_str("cancelled"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchFetchError {
    pub diagnostics: Vec<FetchDiagnostic>,
}

impl fmt::Display for BatchFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to acquire {} distribution object(s)",
            self.diagnostics.len()
        )
    }
}

impl Error for BatchFetchError {}

#[derive(Clone, Debug)]
pub struct FetchClient {
    client: Client,
    config: FetchClientConfig,
}

impl FetchClient {
    pub fn new(config: FetchClientConfig) -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .connect_timeout(config.timeout)
            .timeout(config.timeout)
            .build()?;
        Ok(Self { client, config })
    }

    /// Acquires a complete batch in input order. On any failure no bytes are
    /// returned to the caller, though verified objects remain safely cached.
    pub fn fetch_batch(
        &self,
        cache: &ObjectCache,
        objects_base_url: &str,
        requests: &[FetchRequest],
    ) -> Result<Vec<FetchedObject>, BatchFetchError> {
        self.fetch_batch_cancellable(cache, objects_base_url, requests, &FetchCancellation::new())
    }

    /// Acquires a complete batch while observing `cancellation`. Cancelled
    /// bytes are never published to the cache or returned to the caller.
    pub fn fetch_batch_cancellable(
        &self,
        cache: &ObjectCache,
        objects_base_url: &str,
        requests: &[FetchRequest],
        cancellation: &FetchCancellation,
    ) -> Result<Vec<FetchedObject>, BatchFetchError> {
        if cancellation.is_cancelled() {
            return Err(cancelled_batch(requests));
        }
        let base_url = match reqwest::Url::parse(objects_base_url)
            .map_err(|error| error.to_string())
            .and_then(validate_transport)
        {
            Ok(url) => url,
            Err(message) => {
                return Err(BatchFetchError {
                    diagnostics: requests
                        .iter()
                        .map(|request| {
                            diagnostic(request, FetchFailure::InvalidUrl(message.clone()))
                        })
                        .collect(),
                });
            }
        };
        let results = Arc::new(Mutex::new(
            (0..requests.len())
                .map(|_| None)
                .collect::<Vec<Option<Result<FetchedObject, FetchDiagnostic>>>>(),
        ));
        let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_count = self.config.concurrency.get().min(requests.len());
        std::thread::scope(|scope| {
            for _ in 0..worker_count {
                let results = Arc::clone(&results);
                let next = Arc::clone(&next);
                let base_url = base_url.clone();
                scope.spawn(move || {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(request) = requests.get(index) else {
                            break;
                        };
                        let result = self.fetch_one(cache, &base_url, request, cancellation);
                        results.lock().expect("result mutex poisoned")[index] = Some(result);
                    }
                });
            }
        });
        let mut successes = Vec::with_capacity(requests.len());
        let mut diagnostics = Vec::new();
        for result in Arc::try_unwrap(results)
            .expect("workers released results")
            .into_inner()
            .expect("result mutex poisoned")
        {
            match result.expect("each request assigned to a worker") {
                Ok(object) => successes.push(object),
                Err(error) => diagnostics.push(error),
            }
        }
        if cancellation.is_cancelled() {
            Err(cancelled_batch(requests))
        } else if diagnostics.is_empty() {
            let _publication = cancellation
                .state
                .publication
                .lock()
                .expect("cancellation publication mutex poisoned");
            if cancellation.is_cancelled() {
                return Err(cancelled_batch(requests));
            }
            for object in &successes {
                if object.cache_hit {
                    continue;
                }
                let request = requests
                    .iter()
                    .find(|request| request.request_key == object.request_key)
                    .expect("fetched object came from the input batch");
                if let Err(error) =
                    cache.store_object(&request.object.sha256, request.object.bytes, &object.bytes)
                {
                    return Err(BatchFetchError {
                        diagnostics: vec![cache_diagnostic(request, error)],
                    });
                }
            }
            Ok(successes)
        } else {
            Err(BatchFetchError { diagnostics })
        }
    }

    fn fetch_one(
        &self,
        cache: &ObjectCache,
        base_url: &reqwest::Url,
        request: &FetchRequest,
        cancellation: &FetchCancellation,
    ) -> Result<FetchedObject, FetchDiagnostic> {
        check_cancelled(request, cancellation)?;
        if request.object.bytes > request.max_bytes {
            return Err(diagnostic(
                request,
                FetchFailure::Oversized {
                    declared: request.object.bytes,
                    limit: request.max_bytes,
                },
            ));
        }
        if request.object.object != format!("sha256-{}", request.object.sha256) {
            return Err(diagnostic(
                request,
                FetchFailure::InvalidUrl("object name does not match its digest".into()),
            ));
        }
        match cache.load_object(&request.object.sha256, request.object.bytes) {
            Ok(Some(bytes)) => {
                check_cancelled(request, cancellation)?;
                return Ok(fetched(request, bytes, true));
            }
            Ok(None) => {}
            Err(error) => return Err(cache_diagnostic(request, error)),
        }
        let url = base_url
            .join(&request.object.object)
            .map_err(|error| diagnostic(request, FetchFailure::InvalidUrl(error.to_string())))?;
        let url = validate_transport(url)
            .map_err(|error| diagnostic(request, FetchFailure::InvalidUrl(error)))?;
        let mut last_failure = None;
        for attempt in 0..=self.config.retries {
            check_cancelled(request, cancellation)?;
            match self.download(&url, request, cancellation) {
                Ok(bytes) => {
                    check_cancelled(request, cancellation)?;
                    return Ok(fetched(request, bytes, false));
                }
                Err(failure) => {
                    let retry = retryable(&failure) && attempt < self.config.retries;
                    last_failure = Some(failure);
                    if !retry {
                        break;
                    }
                }
            }
        }
        Err(diagnostic(
            request,
            last_failure.expect("at least one download attempt"),
        ))
    }

    fn download(
        &self,
        url: &reqwest::Url,
        request: &FetchRequest,
        cancellation: &FetchCancellation,
    ) -> Result<Vec<u8>, FetchFailure> {
        let response = self
            .client
            .get(url.clone())
            .send()
            .map_err(|error| FetchFailure::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchFailure::HttpStatus(status.as_u16()));
        }
        if let Some(length) = response.content_length()
            && (length > request.max_bytes || length > request.object.bytes)
        {
            return Err(FetchFailure::LengthMismatch {
                expected: request.object.bytes,
                actual: length,
            });
        }
        read_and_verify(response, request, cancellation)
    }
}

fn validate_transport(url: reqwest::Url) -> Result<reqwest::Url, String> {
    if url.scheme() == "https" {
        return Ok(url);
    }
    if url.scheme() == "http"
        && url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback())
    {
        return Ok(url);
    }
    Err("distribution objects must use HTTPS (HTTP is allowed only for loopback tests)".into())
}

fn read_and_verify(
    mut response: Response,
    request: &FetchRequest,
    cancellation: &FetchCancellation,
) -> Result<Vec<u8>, FetchFailure> {
    let bound = request.object.bytes.saturating_add(1);
    let mut bytes = Vec::with_capacity(
        usize::try_from(request.object.bytes.min(1024 * 1024)).unwrap_or(1024 * 1024),
    );
    let mut reader = response.by_ref().take(bound);
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        if cancellation.is_cancelled() {
            return Err(FetchFailure::Cancelled);
        }
        let count = reader
            .read(&mut chunk)
            .map_err(|error| FetchFailure::Transport(error.to_string()))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if cancellation.is_cancelled() {
        return Err(FetchFailure::Cancelled);
    }
    if bytes.len() as u64 != request.object.bytes {
        return Err(FetchFailure::LengthMismatch {
            expected: request.object.bytes,
            actual: bytes.len() as u64,
        });
    }
    let actual = hex_digest(&bytes);
    if actual != request.object.sha256 {
        return Err(FetchFailure::DigestMismatch { actual });
    }
    Ok(bytes)
}

fn retryable(failure: &FetchFailure) -> bool {
    match failure {
        FetchFailure::Transport(_)
        | FetchFailure::LengthMismatch { .. }
        | FetchFailure::DigestMismatch { .. } => true,
        FetchFailure::HttpStatus(status) => matches!(*status, 408 | 429 | 500..=599),
        FetchFailure::Cancelled => false,
        _ => false,
    }
}

fn check_cancelled(
    request: &FetchRequest,
    cancellation: &FetchCancellation,
) -> Result<(), FetchDiagnostic> {
    if cancellation.is_cancelled() {
        Err(diagnostic(request, FetchFailure::Cancelled))
    } else {
        Ok(())
    }
}

fn cancelled_batch(requests: &[FetchRequest]) -> BatchFetchError {
    BatchFetchError {
        diagnostics: requests
            .iter()
            .map(|request| diagnostic(request, FetchFailure::Cancelled))
            .collect(),
    }
}

fn diagnostic(request: &FetchRequest, failure: FetchFailure) -> FetchDiagnostic {
    FetchDiagnostic {
        request_key: request.request_key.clone(),
        object_digest: request.object.sha256.clone(),
        failure,
    }
}

fn cache_diagnostic(request: &FetchRequest, error: CacheError) -> FetchDiagnostic {
    diagnostic(request, FetchFailure::Cache(error.to_string()))
}

fn fetched(request: &FetchRequest, bytes: Vec<u8>, cache_hit: bool) -> FetchedObject {
    FetchedObject {
        request_key: request.request_key.clone(),
        object_digest: request.object.sha256.clone(),
        bytes,
        cache_hit,
    }
}
