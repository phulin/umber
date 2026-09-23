//! Process entry, verifier, reserved worker route, and cache CLI contracts.

use super::*;

#[test]
fn exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .status()
        .expect("failed to run umber binary");

    assert!(status.success());
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the explicit verifier.
fn distribution_verifier_is_an_explicit_positive_and_negative_cache_control() {
    let directory = tempfile::tempdir().expect("cache verification fixture");
    let cache = directory.path().join("cache");
    let store = umber_fetch::BlobStore::new(&cache);
    let bytes = b"explicit verifier fixture";
    let digest = hex_ahash64(bytes);
    let spec = umber_fetch::VerifiedBlobSpec::content_addressed(
        "objects",
        &digest,
        bytes.len() as u64,
        bytes.len() as u64,
    )
    .expect("cache object specification");
    store.store(&spec, bytes).expect("cache object");

    let positive = Command::new(env!("CARGO_BIN_EXE_distribution-verify"))
        .args(["--cache", cache.to_str().expect("cache path")])
        .output()
        .expect("run explicit verifier");
    assert!(positive.status.success());
    assert_eq!(
        positive.stdout,
        format!(
            "cache blobs=1 objects=1 manifests=0 other=0 payload_bytes={}\n",
            bytes.len()
        )
        .as_bytes()
    );

    let path = store.entry_path(&spec);
    let mut encoded = fs::read(&path).expect("encoded cache object");
    *encoded.last_mut().expect("payload byte") ^= 1;
    fs::write(&path, encoded).expect("mutate cache object");
    let negative = Command::new(env!("CARGO_BIN_EXE_distribution-verify"))
        .args(["--cache", cache.to_str().expect("cache path")])
        .output()
        .expect("run explicit verifier against corruption");
    assert!(!negative.status.success());
    assert!(
        String::from_utf8_lossy(&negative.stderr).contains("envelope digest"),
        "{}",
        String::from_utf8_lossy(&negative.stderr)
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the built Umber binary.
fn reserved_format_worker_invocations_are_owned_before_application_dispatch() {
    let directory = tempfile::tempdir().expect("create isolated worker cache root");
    let cache_home = directory.path().join("cache");
    let run = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .args(arguments)
            .env("XDG_CACHE_HOME", &cache_home)
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .output()
            .expect("run production worker route")
    };

    let malformed = run(&["__format-worker", "trailing"]);
    assert_eq!(malformed.status.code(), Some(70));
    assert!(malformed.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&malformed.stderr),
        "umber format worker: reserved __format-worker invocation accepts no trailing arguments\n"
    );
    assert!(
        !cache_home.exists(),
        "malformed reserved invocation must not initialize the application cache"
    );

    let exact = run(&["__format-worker"]);
    assert_eq!(exact.status.code(), Some(70));
    assert!(String::from_utf8_lossy(&exact.stderr).starts_with("umber format worker: "));
    assert!(
        !cache_home.exists(),
        "exact worker dispatch without a request must not initialize the application cache"
    );

    let unrelated = run(&["__format-worker-unrelated"]);
    assert_eq!(unrelated.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unrelated.stderr).starts_with("umber: "));
    assert!(
        !cache_home.exists(),
        "unrelated ordinary parsing must not initialize the application cache"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the built Umber binary.
fn format_cache_cli_stores_restores_and_reports_misses() {
    let directory = tempfile::tempdir().expect("create format cache fixture");
    let closure = directory.path().join("closure.index");
    let source_lock = directory.path().join("source.lock");
    let build_configuration = directory.path().join("build.config");
    fs::write(&closure, b"tex:latex.ltx\n").expect("write closure identity");
    fs::write(&source_lock, b"pinned sources\n").expect("write source lock");
    fs::write(&build_configuration, b"profile=release\n").expect("write build config");
    let format_path = directory.path().join("generated.fmt");
    let format = umber::with_engine_universe(|stores| {
        umber::EngineMode::Tex82.prepare_initex(stores);
        let mut session =
            umber::EngineSession::prepared_initex(stores, tex_command::CommandProfile::TEX82);
        session
            .register_authored_job("format.tex", std::sync::Arc::from(&b"\\dump"[..]))
            .expect("format root registers");
        let mut host =
            umber::FileSessionResolvers::new(Path::new("format.tex"), Vec::new(), Vec::new());
        session
            .run(&mut host, &mut Vec::new())
            .expect("format construction")
            .format_dump
            .expect("schema-11 format")
            .image
            .into_bytes()
    })
    .expect("fresh CLI format universe");
    fs::write(&format_path, format).expect("write format image");
    let cache_root = directory.path().join("cache");

    let common = [
        "--engine",
        "latex",
        "--distribution",
        "texlive-test",
        "--closure",
        closure.to_str().expect("closure path"),
        "--source-lock",
        source_lock.to_str().expect("source lock path"),
        "--build-configuration",
        build_configuration.to_str().expect("build config path"),
        "--cache-root",
        cache_root.to_str().expect("cache root path"),
    ];
    let store = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "store"])
        .args(common)
        .args(["--format", format_path.to_str().expect("format path")])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .output()
        .expect("store generated format");
    assert!(store.status.success());
    assert_eq!(store.stdout, b"stored\n");
    assert!(String::from_utf8_lossy(&store.stderr).contains("published generated format"));

    let restored = directory.path().join("restored.fmt");
    let restore = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "restore"])
        .args(common)
        .args(["--format-out", restored.to_str().expect("restore path")])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .output()
        .expect("restore generated format");
    assert!(restore.status.success());
    assert_eq!(restore.stdout, b"hit\n");
    assert_eq!(
        fs::read(restored).expect("read restored format"),
        fs::read(format_path).expect("read source format")
    );

    let miss = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "restore"])
        .args(common)
        .arg("--format-out")
        .arg(directory.path().join("miss.fmt"))
        .env("SOURCE_DATE_EPOCH", "0")
        .output()
        .expect("probe changed-clock format");
    assert!(miss.status.success());
    assert_eq!(miss.stdout, b"miss\n");
    assert!(!directory.path().join("miss.fmt").exists());
}
