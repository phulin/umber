mod fixture;

use std::io::Write;
use std::num::NonZeroUsize;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use umber_distribution::ObjectEntry;

use super::*;
use crate::cache::{BLOB_DIRECTORY, hex_digest};
use crate::manifest::fetch_manifest_with_test_agent;

use self::fixture::{FixtureServer, Reply};

fn request(key: &str, bytes: &[u8], limit: u64) -> FetchRequest {
    let digest = hex_digest(bytes);
    FetchRequest {
        request_key: key.into(),
        object: ObjectEntry {
            object: format!("ahash64-v1-{digest}"),
            ahash64: digest,
            bytes: bytes.len() as u64,
        },
        max_bytes: limit,
    }
}

fn cancel_after_request(
    requests: Arc<AtomicUsize>,
    cancellation: FetchCancellation,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for _ in 0..500 {
            if requests.load(Ordering::SeqCst) != 0 {
                cancellation.cancel();
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        cancellation.cancel();
        panic!("fixture did not accept the cancellation request within five seconds");
    })
}

fn client(
    server: &FixtureServer,
    concurrency: usize,
    timeout: Duration,
    retries: usize,
) -> FetchClient {
    let config = FetchClientConfig {
        concurrency: NonZeroUsize::new(concurrency).expect("nonzero concurrency"),
        timeout,
        retries,
    };
    FetchClient::with_agent(config, server.agent(timeout))
}

#[test]
fn fetches_then_reuses_verified_object_cache() {
    let bytes = b"fixture object";
    let server = FixtureServer::new(vec![Reply::ok(bytes)]);
    let temp = TempDir::new().expect("cache tempdir");
    let cache = ObjectCache::new(temp.path());
    let fetcher = client(&server, 2, Duration::from_secs(1), 0);
    let requests = vec![request("tex:plain.tex", bytes, 1024)];

    let cold = fetcher
        .fetch_batch(&cache, &server.base_url, &requests)
        .expect("cold fetch");
    let warm = fetcher
        .fetch_batch(&cache, &server.base_url, &requests)
        .expect("warm fetch");

    assert_eq!(cold[0].bytes, bytes);
    assert!(!cold[0].cache_hit);
    assert!(warm[0].cache_hit);
    assert_eq!(server.finish().0, 1);
}

#[test]
fn fetches_a_manifest_only_when_it_matches_the_trust_pin() {
    let bytes = br#"{"schema":1}"#;
    let server = FixtureServer::new(vec![Reply::ok(bytes)]);
    let fetched = fetch_manifest_with_test_agent(
        &format!("{}manifest.json", server.base_url),
        &hex_digest(bytes),
        &FetchCancellation::new(),
        &server.agent(Duration::from_secs(1)),
    )
    .expect("verified manifest");
    assert_eq!(fetched, bytes);
    server.finish();

    let server = FixtureServer::new(vec![Reply::ok(bytes)]);
    let error = fetch_manifest_with_test_agent(
        &format!("{}manifest.json", server.base_url),
        &"a".repeat(16),
        &FetchCancellation::new(),
        &server.agent(Duration::from_secs(1)),
    )
    .expect_err("mismatched manifest pin");
    assert!(matches!(error, ManifestFetchError::DigestMismatch { .. }));
    server.finish();
}

#[test]
fn distribution_client_persists_manifest_through_shared_blob_store() {
    let bytes = br#"{"schema":1}"#;
    let server = FixtureServer::new(vec![Reply::ok(bytes)]);
    let temp = TempDir::new().expect("cache tempdir");
    let client = DistributionClient::with_agent(
        BlobStore::new(temp.path()),
        FetchClientConfig {
            timeout: Duration::from_secs(1),
            ..FetchClientConfig::default()
        },
        server.agent(Duration::from_secs(1)),
    );
    let url = format!("{}manifest.json", server.base_url);
    let digest = hex_digest(bytes);

    let cold = client
        .acquire_manifest(&url, &digest, &FetchCancellation::new())
        .expect("cold manifest acquisition");
    let warm = client
        .acquire_manifest(&url, &digest, &FetchCancellation::new())
        .expect("warm manifest acquisition");

    assert_eq!(cold.bytes, bytes);
    assert!(!cold.cache_hit);
    assert_eq!(warm.bytes, bytes);
    assert!(warm.cache_hit);
    assert_eq!(server.finish().0, 1);
}

#[test]
fn manifest_policy_does_not_inherit_object_retries() {
    let bytes = br#"{"schema":1}"#;
    let server = FixtureServer::new(vec![Reply::ok(bytes)]);
    let temp = TempDir::new().expect("cache tempdir");
    let client = DistributionClient::with_agent(
        BlobStore::new(temp.path()),
        FetchClientConfig {
            timeout: Duration::from_secs(1),
            retries: 3,
            ..FetchClientConfig::default()
        },
        server.agent(Duration::from_secs(1)),
    );

    let error = client
        .acquire_manifest(
            &format!("{}manifest.json", server.base_url),
            &"a".repeat(16),
            &FetchCancellation::new(),
        )
        .expect_err("a bad manifest trust pin must fail without object retries");

    assert!(matches!(
        error,
        DistributionClientError::Manifest(ManifestFetchError::DigestMismatch { .. })
    ));
    assert_eq!(server.finish().0, 1);
}

#[test]
fn read_only_store_miss_does_not_create_cache_paths() {
    let temp = TempDir::new().expect("cache tempdir");
    let store = BlobStore::new(temp.path());
    let spec = VerifiedBlobSpec::new("formats-v2", "missing", 1024).expect("blob specification");

    assert_eq!(store.load(&spec).expect("read-only miss"), None);
    assert!(!temp.path().join(BLOB_DIRECTORY).exists());
}

#[test]
fn explicit_cache_verifier_checks_every_current_blob() {
    let temp = TempDir::new().expect("cache tempdir");
    let store = BlobStore::new(temp.path());
    let object = b"object payload";
    let object_digest = hex_digest(object);
    store
        .store_object(&object_digest, object.len() as u64, object)
        .expect("store object");
    let manifest = br#"{"schema":2}"#;
    let manifest_digest = hex_digest(manifest);
    store
        .store_manifest(&manifest_digest, manifest)
        .expect("store manifest");

    assert_eq!(
        store.verify_all().expect("complete cache audit"),
        CacheVerificationReport {
            blobs: 2,
            object_blobs: 1,
            manifest_blobs: 1,
            other_blobs: 0,
            payload_bytes: (object.len() + manifest.len()) as u64,
        }
    );
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the corruption control mutates an encoded native cache entry"
)]
fn explicit_cache_verifier_rejects_mutation_without_quarantining() {
    let temp = TempDir::new().expect("cache tempdir");
    let store = BlobStore::new(temp.path());
    let bytes = b"cache mutation control";
    let digest = hex_digest(bytes);
    let spec = VerifiedBlobSpec::content_addressed(
        "objects",
        &digest,
        bytes.len() as u64,
        bytes.len() as u64,
    )
    .expect("object specification");
    store.store(&spec, bytes).expect("store object");
    let path = store.entry_path(&spec);
    let mut encoded = std::fs::read(&path).expect("encoded cache entry");
    *encoded.last_mut().expect("payload byte") ^= 1;
    std::fs::write(&path, &encoded).expect("mutate cache entry");

    let error = store
        .verify_all()
        .expect_err("mutation must fail explicit audit");
    assert!(error.to_string().contains("envelope digest"), "{error}");
    assert_eq!(
        std::fs::read(&path).expect("audit is read-only"),
        encoded,
        "the explicit verifier must not quarantine or rewrite cache bytes"
    );
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the compatibility test writes the previous native cache layout"
)]
fn legacy_object_layout_is_verified_and_migrated() {
    let temp = TempDir::new().expect("cache tempdir");
    let bytes = b"legacy cached object";
    let digest = hex_digest(bytes);
    let legacy = temp
        .path()
        .join("objects")
        .join(format!("ahash64-v1-{digest}"));
    std::fs::create_dir_all(legacy.parent().expect("legacy parent")).expect("legacy directory");
    std::fs::write(&legacy, bytes).expect("legacy object");
    let store = BlobStore::new(temp.path());
    let spec = VerifiedBlobSpec::content_addressed(
        "objects",
        &digest,
        bytes.len() as u64,
        bytes.len() as u64,
    )
    .expect("object specification");

    assert_eq!(
        store.load(&spec).expect("legacy load"),
        Some(bytes.to_vec())
    );
    assert!(store.entry_path(&spec).is_file(), "entry migrated in place");
}

#[test]
fn cancelled_manifest_is_not_returned() {
    let bytes = br#"{"schema":1}"#;
    let cancellation = FetchCancellation::new();
    let server = FixtureServer::new(vec![Reply {
        wait_for_cancellation: Some(cancellation.clone()),
        ..Reply::ok(bytes)
    }]);
    let canceller = cancel_after_request(Arc::clone(&server.requests), cancellation.clone());

    let error = fetch_manifest_with_test_agent(
        &format!("{}manifest.json", server.base_url),
        &hex_digest(bytes),
        &cancellation,
        &server.agent(Duration::from_secs(1)),
    )
    .expect_err("cancelled manifest must not be returned");

    canceller.join().expect("canceller");
    assert_eq!(error, ManifestFetchError::Cancelled);
    assert_eq!(server.finish().0, 1);
}

#[test]
fn returns_typed_404_with_key_and_digest() {
    let bytes = b"absent";
    let server = FixtureServer::new(vec![Reply {
        status: 404,
        body: Vec::new(),
        content_length: Some(0),
        delay: Duration::ZERO,
        wait_for_cancellation: None,
    }]);
    let cache_dir = TempDir::new().expect("cache tempdir");
    let request = request("tfm:missing.tfm", bytes, 1024);
    let expected_digest = request.object.ahash64.clone();

    let error = client(&server, 1, Duration::from_secs(1), 2)
        .fetch_batch(
            &ObjectCache::new(cache_dir.path()),
            &server.base_url,
            &[request],
        )
        .expect_err("404 must fail");

    assert_eq!(error.diagnostics[0].request_key, "tfm:missing.tfm");
    assert_eq!(error.diagnostics[0].object_digest, expected_digest);
    assert_eq!(error.diagnostics[0].failure, FetchFailure::HttpStatus(404));
    assert_eq!(server.finish().0, 1, "404 is not retried");
}

#[test]
fn rejects_corruption_and_truncation_without_caching() {
    let expected = b"correct object";
    let corrupt = b"wrong!! object";
    assert_eq!(expected.len(), corrupt.len());
    let truncated = &expected[..5];
    let server = FixtureServer::new(vec![
        Reply::ok(corrupt),
        Reply {
            status: 200,
            body: truncated.to_vec(),
            content_length: Some(expected.len() as u64),
            delay: Duration::ZERO,
            wait_for_cancellation: None,
        },
    ]);
    let temp = TempDir::new().expect("cache tempdir");
    let cache = ObjectCache::new(temp.path());
    let requests = vec![
        request("tex:corrupt.sty", expected, 1024),
        request("tex:truncated.sty", expected, 1024),
    ];

    let error = client(&server, 2, Duration::from_secs(1), 0)
        .fetch_batch(&cache, &server.base_url, &requests)
        .expect_err("invalid bodies must fail atomically");

    assert_eq!(error.diagnostics.len(), 2);
    assert!(
        error
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.failure, FetchFailure::DigestMismatch { .. }))
    );
    assert!(error.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.failure,
        FetchFailure::Transport(_) | FetchFailure::LengthMismatch { .. }
    )));
    assert!(
        cache
            .load_object(&requests[0].object.ahash64, expected.len() as u64)
            .expect("load cache")
            .is_none()
    );
    server.finish();
}

#[test]
fn refuses_oversized_declaration_before_network_access() {
    let bytes = b"too large";
    let server = FixtureServer::new(Vec::new());
    let temp = TempDir::new().expect("cache tempdir");
    let request = request("tex:large.sty", bytes, 3);

    let error = client(&server, 1, Duration::from_millis(50), 0)
        .fetch_batch(
            &ObjectCache::new(temp.path()),
            "http://127.0.0.1:1/objects/",
            &[request],
        )
        .expect_err("declared size exceeds limit");

    assert_eq!(
        error.diagnostics[0].failure,
        FetchFailure::Oversized {
            declared: bytes.len() as u64,
            limit: 3
        }
    );
    assert_eq!(server.finish().0, 0);
}

#[test]
fn refuses_oversized_content_length_before_reading_body() {
    let bytes = b"small";
    let server = FixtureServer::new(vec![Reply {
        status: 200,
        body: vec![b'x'; 20],
        content_length: Some(20),
        delay: Duration::ZERO,
        wait_for_cancellation: None,
    }]);
    let temp = TempDir::new().expect("cache tempdir");

    let error = client(&server, 1, Duration::from_secs(1), 0)
        .fetch_batch(
            &ObjectCache::new(temp.path()),
            &server.base_url,
            &[request("tex:small.sty", bytes, 10)],
        )
        .expect_err("content length exceeds declaration");

    assert_eq!(
        error.diagnostics[0].failure,
        FetchFailure::LengthMismatch {
            expected: 5,
            actual: 20
        }
    );
    server.finish();
}

#[test]
fn retries_timeout_and_succeeds() {
    let bytes = b"eventual object";
    let server = FixtureServer::new(vec![
        Reply {
            delay: Duration::from_millis(250),
            ..Reply::ok(bytes)
        },
        Reply::ok(bytes),
    ]);
    let temp = TempDir::new().expect("cache tempdir");

    let fetched = client(&server, 1, Duration::from_millis(80), 1)
        .fetch_batch(
            &ObjectCache::new(temp.path()),
            &server.base_url,
            &[request("tex:retry.sty", bytes, 1024)],
        )
        .expect("retry succeeds");

    assert_eq!(fetched[0].bytes, bytes);
    assert_eq!(server.finish().0, 2);
}

#[test]
fn cancellation_after_download_does_not_publish_or_return_bytes() {
    let bytes = b"cancelled object";
    let cancellation = FetchCancellation::new();
    let server = FixtureServer::new(vec![Reply {
        wait_for_cancellation: Some(cancellation.clone()),
        ..Reply::ok(bytes)
    }]);
    let temp = TempDir::new().expect("cache tempdir");
    let cache = ObjectCache::new(temp.path());
    let request = request("tex:cancelled.sty", bytes, 1024);
    let canceller = cancel_after_request(Arc::clone(&server.requests), cancellation.clone());

    let error = client(&server, 1, Duration::from_secs(1), 0)
        .fetch_batch_cancellable(
            &cache,
            &server.base_url,
            std::slice::from_ref(&request),
            &cancellation,
        )
        .expect_err("cancelled fetch must not return bytes");

    canceller.join().expect("canceller");
    assert!(
        error
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.failure == FetchFailure::Cancelled)
    );
    assert!(
        cache
            .load_object(&request.object.ahash64, request.object.bytes)
            .expect("load cache")
            .is_none(),
        "cancelled download must not be published"
    );
    assert_eq!(server.finish().0, 1);
}

#[test]
fn bounds_parallel_batch_downloads() {
    let bodies = [b"one".as_slice(), b"two", b"three", b"four"];
    let requests: Vec<_> = bodies
        .iter()
        .enumerate()
        .map(|(index, body)| request(&format!("tex:{index}.sty"), body, 1024))
        .collect();
    let replies = requests
        .iter()
        .zip(bodies)
        .map(|(request, body)| {
            (
                request.object.object.clone(),
                Reply {
                    delay: Duration::from_millis(80),
                    ..Reply::ok(body)
                },
            )
        })
        .collect();
    let server = FixtureServer::routed(replies);
    let temp = TempDir::new().expect("cache tempdir");

    let fetched = client(&server, 2, Duration::from_secs(1), 0)
        .fetch_batch(&ObjectCache::new(temp.path()), &server.base_url, &requests)
        .expect("bounded fetch");

    assert_eq!(fetched.len(), 4);
    let (_, maximum_active) = server.finish();
    assert!((1..=2).contains(&maximum_active));
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the test deliberately corrupts a native cache file"
)]
fn manifest_cache_is_digest_keyed_and_reverified() {
    let temp = TempDir::new().expect("cache tempdir");
    let cache = ObjectCache::new(temp.path());
    let bytes = br#"{"schema":1}"#;
    let digest = hex_digest(bytes);
    cache
        .store_manifest(&digest, bytes)
        .expect("store manifest");
    assert_eq!(
        cache.load_manifest(&digest).expect("load manifest"),
        Some(bytes.to_vec())
    );
    let path = cache.entry_path(
        &VerifiedBlobSpec::content_addressed(
            "manifests",
            &digest,
            bytes.len() as u64,
            32 * 1024 * 1024,
        )
        .expect("manifest blob specification"),
    );
    let mut file = std::fs::File::create(path).expect("open cached manifest");
    file.write_all(b"corrupt").expect("corrupt cached manifest");
    assert_eq!(
        cache.load_manifest(&digest).expect("reverify manifest"),
        None
    );
}

#[test]
fn manifest_cache_rejects_oversized_entries() {
    let temp = TempDir::new().expect("cache tempdir");
    let cache = ObjectCache::new(temp.path());
    let bytes = vec![0_u8; 32 * 1024 * 1024 + 1];
    let digest = hex_digest(&bytes);
    assert!(cache.store_manifest(&digest, &bytes).is_err());
    assert_eq!(cache.load_manifest(&digest).expect("load manifest"), None);
}

const RACE_BYTES: &[u8] = b"concurrent process cache object";

#[test]
fn cache_race_child() {
    let Some(root) = std::env::var_os("UMBER_FETCH_RACE_CHILD") else {
        return;
    };
    let digest = hex_digest(RACE_BYTES);
    ObjectCache::new(root)
        .store_object(&digest, RACE_BYTES.len() as u64, RACE_BYTES)
        .expect("race child stores object");
}

#[test]
fn concurrent_processes_publish_one_verified_cache_object() {
    let temp = TempDir::new().expect("cache tempdir");
    let executable = std::env::current_exe().expect("test executable");
    let mut children = Vec::new();
    for _ in 0..6 {
        children.push(
            Command::new(&executable)
                .args(["--exact", "tests::cache_race_child"])
                .env("UMBER_FETCH_RACE_CHILD", temp.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn cache writer"),
        );
    }
    for mut child in children {
        assert!(child.wait().expect("wait cache writer").success());
    }
    let digest = hex_digest(RACE_BYTES);
    assert_eq!(
        ObjectCache::new(temp.path())
            .load_object(&digest, RACE_BYTES.len() as u64)
            .expect("load raced object"),
        Some(RACE_BYTES.to_vec())
    );
    let entries = std::fs::read_dir(temp.path().join(BLOB_DIRECTORY))
        .expect("blob directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("object entries");
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("ahash64-v1-"))
            .count(),
        1,
        "temporary files are cleaned up"
    );
}
