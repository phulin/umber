use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use tempfile::TempDir;

use super::*;

#[test]
fn verifies_a_complete_cache_without_publication() {
    let temp = TempDir::new().expect("temporary directory");
    let destination = temp.path().join("corpus");
    fs::create_dir(&destination).expect("create corpus");
    fs::write(destination.join("plain.tex"), b"plain").expect("write support");
    fs::write(destination.join("story.tex"), b"story").expect("write document");
    let manifest = write_manifest(temp.path());

    let status = run(&SyncOptions {
        manifest_path: manifest,
        destination: destination.clone(),
        offline: true,
    })
    .expect("verify cache");

    assert_eq!(status.len(), 2);
    assert!(
        status
            .iter()
            .all(|status| matches!(status, EntryStatus::Verified { .. }))
    );
}

#[test]
fn offline_mode_reports_the_first_missing_entry_without_mutation() {
    let temp = TempDir::new().expect("temporary directory");
    let destination = temp.path().join("corpus");
    let error = run(&SyncOptions {
        manifest_path: write_manifest(temp.path()),
        destination: destination.clone(),
        offline: true,
    })
    .expect_err("missing offline cache must fail");

    assert!(
        error
            .to_string()
            .contains("missing corpus document plain.tex")
    );
    assert!(!destination.exists());
}

#[test]
fn locator_fallback_publishes_the_complete_tree_atomically() {
    let temp = TempDir::new().expect("temporary directory");
    let destination = temp.path().join("corpus");
    fs::create_dir(&destination).expect("create corpus");
    fs::write(destination.join("plain.tex"), b"plain").expect("write support");
    let good = serve_once(b"story");
    let manifest = temp.path().join("manifest.txt");
    fs::write(
        &manifest,
        format!(
            "support plain.tex\nurl https://example.invalid/plain.tex\nsha256 {}\nlicense MIT\nredistributable true\nnotes support\n\ndoc story.tex\nurl http://127.0.0.1:9/unavailable\nurl {good}\nsha256 {}\nlicense MIT\nredistributable true\nformat_source plain.tex\nexpected_ref_dvi_sha256 {}\nnotes document\n",
            sha256_hex(b"plain"),
            sha256_hex(b"story"),
            sha256_hex(b"dvi")
        ),
    )
    .expect("write manifest");

    let status = run(&SyncOptions {
        manifest_path: manifest,
        destination: destination.clone(),
        offline: false,
    })
    .expect("fallback fetch");

    assert!(matches!(status[0], EntryStatus::Verified { .. }));
    assert!(matches!(status[1], EntryStatus::Fetched { .. }));
    assert_eq!(
        fs::read(destination.join("story.tex")).expect("published document"),
        b"story"
    );
}

fn write_manifest(root: &Path) -> PathBuf {
    let manifest = root.join("manifest.txt");
    fs::write(
        &manifest,
        format!(
            "support plain.tex\nurl https://example.invalid/plain.tex\nsha256 {}\nlicense MIT\nredistributable true\nnotes support\n\ndoc story.tex\nurl https://example.invalid/story.tex\nsha256 {}\nlicense MIT\nredistributable true\nformat_source plain.tex\nexpected_ref_dvi_sha256 {}\nnotes document\n",
            sha256_hex(b"plain"),
            sha256_hex(b"story"),
            sha256_hex(b"dvi")
        ),
    )
    .expect("write manifest");
    manifest
}

fn serve_once(body: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture server address");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fixture request");
        let mut request = [0; 1024];
        let _ = stream.read(&mut request);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .expect("write response headers");
        stream.write_all(body).expect("write response body");
    });
    format!("http://{address}/story.tex")
}
