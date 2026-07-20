use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::num::NonZeroUsize;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use umber_distribution::ObjectEntry;

use super::*;
use crate::cache::hex_digest;

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
    content_length: Option<u64>,
    delay: Duration,
}

impl Reply {
    fn ok(body: &[u8]) -> Self {
        Self {
            status: 200,
            body: body.to_vec(),
            content_length: Some(body.len() as u64),
            delay: Duration::ZERO,
        }
    }
}

struct FixtureServer {
    base_url: String,
    requests: Arc<AtomicUsize>,
    maximum_active: Arc<AtomicUsize>,
    join: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let base_url = format!(
            "http://{}/objects/",
            listener.local_addr().expect("address")
        );
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_thread = Arc::clone(&requests);
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let maximum_for_thread = Arc::clone(&maximum_active);
        let active = Arc::new(AtomicUsize::new(0));
        let join = thread::spawn(move || {
            let mut handlers = Vec::new();
            for reply in replies {
                let (stream, _) = listener.accept().expect("accept fixture request");
                requests_for_thread.fetch_add(1, Ordering::SeqCst);
                let maximum = Arc::clone(&maximum_for_thread);
                let active = Arc::clone(&active);
                handlers.push(thread::spawn(move || serve(stream, reply, active, maximum)));
            }
            for handler in handlers {
                handler.join().expect("fixture connection handler");
            }
        });
        Self {
            base_url,
            requests,
            maximum_active,
            join: Some(join),
        }
    }

    fn routed(replies: BTreeMap<String, Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let base_url = format!(
            "http://{}/objects/",
            listener.local_addr().expect("address")
        );
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_thread = Arc::clone(&requests);
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let maximum_for_thread = Arc::clone(&maximum_active);
        let active = Arc::new(AtomicUsize::new(0));
        let replies = Arc::new(replies);
        let expected = replies.len();
        let join = thread::spawn(move || {
            let mut handlers = Vec::new();
            for _ in 0..expected {
                let (stream, _) = listener.accept().expect("accept fixture request");
                requests_for_thread.fetch_add(1, Ordering::SeqCst);
                let maximum = Arc::clone(&maximum_for_thread);
                let active = Arc::clone(&active);
                let replies = Arc::clone(&replies);
                handlers.push(thread::spawn(move || {
                    serve_routed(stream, &replies, active, maximum)
                }));
            }
            for handler in handlers {
                handler.join().expect("fixture connection handler");
            }
        });
        Self {
            base_url,
            requests,
            maximum_active,
            join: Some(join),
        }
    }

    fn finish(mut self) -> (usize, usize) {
        self.join
            .take()
            .expect("server thread")
            .join()
            .expect("server");
        (
            self.requests.load(Ordering::SeqCst),
            self.maximum_active.load(Ordering::SeqCst),
        )
    }
}

fn serve(
    mut stream: TcpStream,
    reply: Reply,
    active_connections: Arc<AtomicUsize>,
    maximum_active: Arc<AtomicUsize>,
) {
    let active = active_connections.fetch_add(1, Ordering::SeqCst) + 1;
    maximum_active.fetch_max(active, Ordering::SeqCst);
    let mut request = [0_u8; 2048];
    let _ = stream.read(&mut request);
    thread::sleep(reply.delay);
    let reason = if reply.status == 200 {
        "OK"
    } else {
        "Not Found"
    };
    let length = reply.content_length.unwrap_or(reply.body.len() as u64);
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status, reason, length
    );
    write_fixture_response(&mut stream, response.as_bytes(), &reply.body);
    active_connections.fetch_sub(1, Ordering::SeqCst);
}

fn write_fixture_response(stream: &mut TcpStream, headers: &[u8], body: &[u8]) {
    if let Err(error) = stream
        .write_all(headers)
        .and_then(|()| stream.write_all(body))
    {
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
            ),
            "write fixture response: {error}"
        );
    }
}

fn serve_routed(
    mut stream: TcpStream,
    replies: &BTreeMap<String, Reply>,
    active_connections: Arc<AtomicUsize>,
    maximum_active: Arc<AtomicUsize>,
) {
    let active = active_connections.fetch_add(1, Ordering::SeqCst) + 1;
    maximum_active.fetch_max(active, Ordering::SeqCst);
    let mut request = [0_u8; 2048];
    let count = stream.read(&mut request).expect("read fixture request");
    let request = String::from_utf8_lossy(&request[..count]);
    let path = request
        .split_whitespace()
        .nth(1)
        .expect("HTTP request path");
    let object = path.rsplit('/').next().expect("object path");
    let reply = replies.get(object).expect("routed fixture object").clone();
    thread::sleep(reply.delay);
    let length = reply.content_length.unwrap_or(reply.body.len() as u64);
    let response = format!(
        "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status, length
    );
    write_fixture_response(&mut stream, response.as_bytes(), &reply.body);
    active_connections.fetch_sub(1, Ordering::SeqCst);
}

fn request(key: &str, bytes: &[u8], limit: u64) -> FetchRequest {
    let digest = hex_digest(bytes);
    FetchRequest {
        request_key: key.into(),
        object: ObjectEntry {
            object: format!("sha256-{digest}"),
            sha256: digest,
            bytes: bytes.len() as u64,
        },
        max_bytes: limit,
    }
}

fn client(concurrency: usize, timeout: Duration, retries: usize) -> FetchClient {
    FetchClient::new(FetchClientConfig {
        concurrency: NonZeroUsize::new(concurrency).expect("nonzero concurrency"),
        timeout,
        retries,
    })
    .expect("fetch client")
}

#[test]
fn fetches_then_reuses_verified_object_cache() {
    let bytes = b"fixture object";
    let server = FixtureServer::new(vec![Reply::ok(bytes)]);
    let temp = TempDir::new().expect("cache tempdir");
    let cache = ObjectCache::new(temp.path());
    let fetcher = client(2, Duration::from_secs(1), 0);
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
    let fetched = fetch_manifest(
        &format!("{}manifest.json", server.base_url),
        &hex_digest(bytes),
        Duration::from_secs(1),
    )
    .expect("verified manifest");
    assert_eq!(fetched, bytes);
    server.finish();

    let server = FixtureServer::new(vec![Reply::ok(bytes)]);
    let error = fetch_manifest(
        &format!("{}manifest.json", server.base_url),
        &"0".repeat(64),
        Duration::from_secs(1),
    )
    .expect_err("mismatched manifest pin");
    assert!(matches!(error, ManifestFetchError::DigestMismatch { .. }));
    server.finish();
}

#[test]
fn cancelled_manifest_is_not_returned() {
    let bytes = br#"{"schema":1}"#;
    let server = FixtureServer::new(vec![Reply {
        delay: Duration::from_millis(120),
        ..Reply::ok(bytes)
    }]);
    let cancellation = FetchCancellation::new();
    let cancel_from_thread = cancellation.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        cancel_from_thread.cancel();
    });

    let error = fetch_manifest_cancellable(
        &format!("{}manifest.json", server.base_url),
        &hex_digest(bytes),
        Duration::from_secs(1),
        &cancellation,
    )
    .expect_err("cancelled manifest must not be returned");

    canceller.join().expect("canceller");
    assert_eq!(error, ManifestFetchError::Cancelled);
    server.finish();
}

#[test]
fn returns_typed_404_with_key_and_digest() {
    let bytes = b"absent";
    let server = FixtureServer::new(vec![Reply {
        status: 404,
        body: Vec::new(),
        content_length: Some(0),
        delay: Duration::ZERO,
    }]);
    let cache_dir = TempDir::new().expect("cache tempdir");
    let request = request("tfm:missing.tfm", bytes, 1024);
    let expected_digest = request.object.sha256.clone();

    let error = client(1, Duration::from_secs(1), 2)
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
        },
    ]);
    let temp = TempDir::new().expect("cache tempdir");
    let cache = ObjectCache::new(temp.path());
    let requests = vec![
        request("tex:corrupt.sty", expected, 1024),
        request("tex:truncated.sty", expected, 1024),
    ];

    let error = client(2, Duration::from_secs(1), 0)
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
            .load_object(&requests[0].object.sha256, expected.len() as u64)
            .expect("load cache")
            .is_none()
    );
    server.finish();
}

#[test]
fn refuses_oversized_declaration_before_network_access() {
    let bytes = b"too large";
    let temp = TempDir::new().expect("cache tempdir");
    let request = request("tex:large.sty", bytes, 3);

    let error = client(1, Duration::from_millis(50), 0)
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
}

#[test]
fn refuses_oversized_content_length_before_reading_body() {
    let bytes = b"small";
    let server = FixtureServer::new(vec![Reply {
        status: 200,
        body: vec![b'x'; 20],
        content_length: Some(20),
        delay: Duration::ZERO,
    }]);
    let temp = TempDir::new().expect("cache tempdir");

    let error = client(1, Duration::from_secs(1), 0)
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

    let fetched = client(1, Duration::from_millis(80), 1)
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
    let server = FixtureServer::new(vec![Reply {
        delay: Duration::from_millis(120),
        ..Reply::ok(bytes)
    }]);
    let temp = TempDir::new().expect("cache tempdir");
    let cache = ObjectCache::new(temp.path());
    let request = request("tex:cancelled.sty", bytes, 1024);
    let cancellation = FetchCancellation::new();
    let cancel_from_thread = cancellation.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        cancel_from_thread.cancel();
    });

    let error = client(1, Duration::from_secs(1), 0)
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
            .load_object(&request.object.sha256, request.object.bytes)
            .expect("load cache")
            .is_none(),
        "cancelled download must not be published"
    );
    server.finish();
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

    let fetched = client(2, Duration::from_secs(1), 0)
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
    let path = temp
        .path()
        .join("manifests")
        .join(format!("sha256-{digest}"));
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
    let entries = std::fs::read_dir(temp.path().join("objects"))
        .expect("object directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("object entries");
    assert_eq!(entries.len(), 1, "temporary files are cleaned up");
}
