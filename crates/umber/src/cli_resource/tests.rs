#![allow(clippy::disallowed_methods)] // Host-side resource/cache integration fixtures.

use tempfile::TempDir;
use tex_fonts::{FontFeaturePolicy, FontPurposes, FontRequest, FontRequestKey, VariationSelection};
use tex_incr::RevisionId;

use super::*;

#[test]
fn native_session_allows_the_hard_bounded_resource_attempt_count() {
    let directory = TempDir::new().expect("temporary project");
    let input = directory.path().join("main.tex");
    std::fs::write(&input, b"\\end").expect("main input");
    let options = NativeRunOptions {
        input,
        format: None,
        engine: EngineMode::Tex82,
        html: false,
        distribution: None,
        distribution_sha256: None,
        offline: true,
    };

    let session = NativeCompileSession::new_with_cache(
        &options,
        &FetchCancellation::new(),
        ObjectCache::new(directory.path().join("cache")),
    )
    .expect("native session");

    assert_eq!(
        session.session.attempt_limit(),
        SessionLimits::HARD_MAX.attempts
    );
}

#[test]
fn retained_revision_does_not_refetch_resolved_distribution_file() {
    let directory = TempDir::new().expect("temporary project");
    let distribution = directory.path().join("distribution");
    let objects = distribution.join("objects");
    std::fs::create_dir_all(&objects).expect("distribution objects directory");
    let package = b"\\def\\packagewasloaded{1}";
    let digest = hex_digest(package);
    let object = format!("sha256-{digest}");
    std::fs::write(objects.join(&object), package).expect("distribution object");
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"watch-test\",\"index\":0,\"files\":{{\"tex:package.sty\":{{\"virtualPath\":\"/texlive/tex/package.sty\",\"object\":\"{object}\",\"sha256\":\"{digest}\",\"bytes\":{}}}}}}}\n",
        package.len()
    );
    let shard_digest = hex_digest(shard.as_bytes());
    std::fs::write(objects.join(format!("sha256-{shard_digest}")), shard).expect("index shard");
    let root = format!(
        "{{\"schema\":2,\"distribution\":\"watch-test\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
    );
    std::fs::write(distribution.join("manifest-v2.json"), root).expect("root manifest");
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

    std::fs::remove_file(objects.join(object)).expect("remove source object");
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

fn file_request(name: &str) -> ResourceRequest {
    ResourceRequest::File(FileRequest::new(
        crate::FileRequestKey::new(FileKind::TexInput, name).expect("file request key"),
        name,
    ))
}

fn local_resolver(root: &Path) -> LocalResolver {
    LocalResolver {
        base: root.to_owned(),
        input: TexInputSearchPath::new(root, Vec::new()),
        font: TexFontSearchPath::new(root.to_owned(), Vec::new()),
        input_paths: RefCell::new(BTreeMap::new()),
        resolved_inputs: RefCell::new(Vec::new()),
    }
}

#[test]
fn local_resolver_handles_each_classic_bibliography_kind() {
    let directory = tempfile::tempdir().expect("temporary directory");
    std::fs::write(directory.path().join("child.aux"), b"aux").expect("AUX");
    std::fs::write(directory.path().join("refs.bib"), b"bib").expect("BIB");
    std::fs::write(directory.path().join("plain.bst"), b"bst").expect("BST");
    let resolver = local_resolver(directory.path());
    for (kind, name, bytes) in [
        (FileKind::BibAux, "child", b"aux".as_slice()),
        (FileKind::ClassicBibData, "refs", b"bib".as_slice()),
        (FileKind::BibStyle, "plain", b"bst".as_slice()),
    ] {
        let request = FileRequest::new(
            crate::FileRequestKey::new(kind, name).expect("classic request"),
            name,
        );
        assert_eq!(resolver.resolve(&request).expect("resolved").bytes, bytes);
    }
}

fn write_sharded_root(
    directory: &Path,
    distribution: &str,
    shard_bits: u8,
    shards: &[(&str, bool)],
) -> (Vec<u8>, Vec<String>) {
    let objects = directory.join("objects");
    std::fs::create_dir_all(&objects).expect("objects directory");
    let mut digests = Vec::new();
    for (body, publish) in shards {
        let digest = hex_digest(body.as_bytes());
        if *publish {
            std::fs::write(objects.join(format!("sha256-{digest}")), body).expect("shard object");
        }
        digests.push(digest);
    }
    let quoted = digests
        .iter()
        .map(|digest| format!("\"{digest}\""))
        .collect::<Vec<_>>()
        .join(",");
    let root = format!(
        "{{\"schema\":2,\"distribution\":\"{distribution}\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":{shard_bits},\"shardCount\":{},\"shards\":[{quoted}]}}\n",
        shards.len()
    )
    .into_bytes();
    std::fs::write(directory.join("manifest-v2.json"), &root).expect("root manifest");
    (root, digests)
}

#[test]
fn verified_shard_absence_returns_typed_unavailable() {
    let directory = TempDir::new().expect("distribution tempdir");
    let shard = "{\"schema\":1,\"distribution\":\"absence\",\"index\":0,\"files\":{}}\n";
    write_sharded_root(directory.path(), "absence", 0, &[(shard, true)]);
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    let responses = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &[file_request("missing.sty")],
            &FetchCancellation::new(),
        )
        .expect("authoritative absence");
    assert!(matches!(
        responses.as_slice(),
        [ResourceResponse::FileUnavailable(key)] if key.name() == "missing.sty"
    ));
}

#[test]
fn verified_schema_v2_root_returns_typed_font_unavailable() {
    let directory = TempDir::new().expect("distribution tempdir");
    let shard = "{\"schema\":1,\"distribution\":\"absence\",\"index\":0,\"files\":{}}\n";
    write_sharded_root(directory.path(), "absence", 0, &[(shard, true)]);
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    let key = FontRequestKey::new(
        "missing-font",
        0,
        VariationSelection::default(),
        FontFeaturePolicy::default(),
    )
    .expect("font request key");
    let responses = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &[ResourceRequest::Font(FontRequest {
                key: key.clone(),
                accepted_containers: AcceptedFontContainers::NATIVE_WITH_COLLECTIONS,
                purposes: FontPurposes::LAYOUT,
            })],
            &FetchCancellation::new(),
        )
        .expect("authoritative font absence");
    assert_eq!(responses, vec![ResourceResponse::FontUnavailable(key)]);
}

#[test]
fn inline_hint_fetches_without_loading_the_dependency_shard() {
    let directory = TempDir::new().expect("distribution tempdir");
    let required_bytes = b"required";
    let required_digest = hex_digest(required_bytes);
    let dependency_bytes = b"dependency";
    let dependency_digest = hex_digest(dependency_bytes);
    let required_object = format!("sha256-{required_digest}");
    let dependency_object = format!("sha256-{dependency_digest}");
    let shard_zero = format!(
        "{{\"schema\":1,\"distribution\":\"hints\",\"index\":0,\"files\":{{\"tex:article.cls\":{{\"virtualPath\":\"/texlive/tex/article.cls\",\"object\":\"{required_object}\",\"sha256\":\"{required_digest}\",\"bytes\":{},\"dependencies\":[{{\"key\":\"tfm:cmr10.tfm\",\"virtualPath\":\"/texlive/fonts/cmr10.tfm\",\"object\":\"{dependency_object}\",\"sha256\":\"{dependency_digest}\",\"bytes\":{}}}]}}}}}}\n",
        required_bytes.len(),
        dependency_bytes.len()
    );
    let shard_one = "{\"schema\":1,\"distribution\":\"hints\",\"index\":1,\"files\":{}}\n";
    write_sharded_root(
        directory.path(),
        "hints",
        1,
        &[(&shard_zero, true), (shard_one, false)],
    );
    let objects = directory.path().join("objects");
    std::fs::write(objects.join(required_object), required_bytes).expect("required object");
    std::fs::write(objects.join(dependency_object), dependency_bytes).expect("dependency object");
    let cache = ObjectCache::new(directory.path().join("cache"));
    let mut resolver = DistributionResolver::new(
        cache.clone(),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    let responses = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &[file_request("article.cls")],
            &FetchCancellation::new(),
        )
        .expect("inline dependency hint");
    assert!(matches!(responses.as_slice(), [ResourceResponse::File(_)]));
    assert_eq!(
        cache
            .load_object(&dependency_digest, dependency_bytes.len() as u64)
            .expect("hint cache read"),
        Some(dependency_bytes.to_vec())
    );
}

#[test]
fn warm_root_shard_and_object_cache_resolve_offline() {
    let directory = TempDir::new().expect("distribution tempdir");
    let bytes = b"cached";
    let digest = hex_digest(bytes);
    let object = format!("sha256-{digest}");
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"offline\",\"index\":0,\"files\":{{\"tex:cached.sty\":{{\"virtualPath\":\"/texlive/tex/cached.sty\",\"object\":\"{object}\",\"sha256\":\"{digest}\",\"bytes\":{}}}}}}}\n",
        bytes.len()
    );
    let (root, _) = write_sharded_root(directory.path(), "offline", 0, &[(&shard, true)]);
    std::fs::write(directory.path().join("objects").join(object), bytes).expect("file object");
    let cache = ObjectCache::new(directory.path().join("cache"));
    let root_digest = hex_digest(&root);
    cache
        .store_manifest(&root_digest, &root)
        .expect("cache root manifest");
    let mut online = DistributionResolver::new(
        cache.clone(),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    online
        .resolve_batch(
            &local_resolver(directory.path()),
            &[file_request("cached.sty")],
            &FetchCancellation::new(),
        )
        .expect("warm caches");
    let mut offline = DistributionResolver::new(
        cache,
        Some("https://example.invalid/manifest-v2.json".into()),
        Some(root_digest),
        true,
    );
    let responses = offline
        .resolve_batch(
            &local_resolver(directory.path()),
            &[file_request("cached.sty")],
            &FetchCancellation::new(),
        )
        .expect("offline cache resolution");
    assert!(matches!(responses.as_slice(), [ResourceResponse::File(_)]));
}

#[test]
fn rejects_tampered_shard_and_observes_cancellation() {
    let directory = TempDir::new().expect("distribution tempdir");
    let shard = "{\"schema\":1,\"distribution\":\"tamper\",\"index\":0,\"files\":{}}\n";
    let (_, digests) = write_sharded_root(directory.path(), "tamper", 0, &[(shard, true)]);
    std::fs::write(
        directory
            .path()
            .join("objects")
            .join(format!("sha256-{}", digests[0])),
        b"tampered",
    )
    .expect("tamper shard");
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    assert!(matches!(
        resolver.resolve_batch(
            &local_resolver(directory.path()),
            &[file_request("missing.sty")],
            &FetchCancellation::new(),
        ),
        Err(NativeRunError::ManifestDigestMismatch { .. })
    ));

    let cancellation = FetchCancellation::new();
    cancellation.cancel();
    assert!(matches!(
        resolver.resolve_batch(
            &local_resolver(directory.path()),
            &[file_request("missing.sty")],
            &cancellation,
        ),
        Err(NativeRunError::Cancelled)
    ));
}

#[test]
fn shard_partition_uses_sha256_network_prefix_bits() {
    assert_eq!(shard_index("tex:article.cls", 8), 0x45);
    assert_eq!(shard_index("tfm:cmr10.tfm", 8), 0x91);
    assert_eq!(shard_index("tex:plain.tex", 0), 0);
}
