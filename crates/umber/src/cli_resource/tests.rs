#![allow(clippy::disallowed_methods)] // Host-side resource/cache integration fixtures.

use std::collections::BTreeMap;

use tempfile::TempDir;
use tex_incr::RevisionId;
use umber_distribution::{MANIFEST_SCHEMA, ManifestFile};

use super::*;

#[test]
fn retained_revision_does_not_refetch_resolved_distribution_file() {
    let directory = TempDir::new().expect("temporary project");
    let distribution = directory.path().join("distribution");
    std::fs::create_dir(&distribution).expect("distribution directory");
    let package = b"\\def\\packagewasloaded{1}";
    let digest = hex_digest(package);
    let object = format!("sha256-{digest}");
    std::fs::write(distribution.join(&object), package).expect("distribution object");
    let manifest = Manifest {
        schema: MANIFEST_SCHEMA,
        distribution: "watch-test".into(),
        objects_base_url: "https://example.invalid/objects/".into(),
        files: BTreeMap::from([(
            "tex:package.sty".into(),
            ManifestFile {
                virtual_path: "/texlive/tex/package.sty".into(),
                object: object.clone(),
                sha256: digest.clone(),
                bytes: package.len() as u64,
                dependencies: Vec::new(),
            },
        )]),
        fonts: BTreeMap::new(),
        formats: BTreeMap::new(),
    };
    std::fs::write(
        distribution.join("manifest.json"),
        manifest.to_json_pretty(),
    )
    .expect("manifest");
    let input = directory.path().join("watch.tex");
    let original = "\\input package.sty \\shipout\\vbox{\\hrule height 1pt}\\end";
    let edited = "\\input package.sty \\shipout\\vbox{\\hrule height 2pt}\\end";
    std::fs::write(&input, original).expect("main input");
    let options = NativeRunOptions {
        input,
        format: None,
        engine: EngineMode::Tex82,
        html: false,
        distribution: Some(distribution.to_string_lossy().into_owned()),
        distribution_sha256: None,
        offline: false,
    };
    let cache_root = directory.path().join("cache");
    let cancellation = FetchCancellation::new();
    let mut session = NativeCompileSession::new_with_cache(
        &options,
        &cancellation,
        ObjectCache::new(&cache_root),
    )
    .expect("session");
    let cold = session.compile(&cancellation).expect("cold compile");

    std::fs::remove_file(distribution.join(object)).expect("remove source object");
    std::fs::remove_file(cache_root.join("objects").join(format!("sha256-{digest}")))
        .expect("remove cached object");
    session
        .apply_source(RevisionId::new(2), edited)
        .expect("apply edit");
    let incremental = session.compile(&cancellation).expect("incremental compile");

    assert_ne!(incremental.dvi, cold.dvi);
    assert_eq!(session.source(), edited);
}

#[test]
fn cancelled_pending_revision_can_be_superseded() {
    let directory = TempDir::new().expect("temporary project");
    let input = directory.path().join("watch.tex");
    let original = "\\shipout\\vbox{\\hrule height 1pt}\\end";
    let edited = "\\shipout\\vbox{\\hrule height 2pt}\\end";
    std::fs::write(&input, original).expect("main input");
    let options = NativeRunOptions {
        input,
        format: None,
        engine: EngineMode::Tex82,
        html: false,
        distribution: None,
        distribution_sha256: None,
        offline: true,
    };
    let cancellation = FetchCancellation::new();
    let mut session = NativeCompileSession::new_with_cache(
        &options,
        &cancellation,
        ObjectCache::new(directory.path().join("cache")),
    )
    .expect("session");
    session.compile(&cancellation).expect("cold compile");
    session
        .apply_source(RevisionId::new(2), "\\input missing.sty \\end")
        .expect("first edit");
    let cancelled = FetchCancellation::new();
    cancelled.cancel();
    assert!(matches!(
        session.compile(&cancelled),
        Err(NativeRunError::Cancelled)
    ));
    assert!(session.cancel_pending_revision());

    session
        .apply_source(RevisionId::new(3), edited)
        .expect("superseding edit");
    session.compile(&cancellation).expect("superseding compile");
    assert_eq!(session.source(), edited);
}
