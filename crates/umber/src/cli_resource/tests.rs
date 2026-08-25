#![allow(clippy::disallowed_methods)] // Host-side resource/cache integration fixtures.

use tempfile::TempDir;
use tex_fonts::{FontFeaturePolicy, FontPurposes, FontRequest, FontRequestKey, VariationSelection};
use tex_incr::RevisionId;

use super::*;

#[test]
fn pdf_font_closure_receipt_preserves_typed_outcomes_and_manifest_keys() {
    let vf = FileRequestKey::new(FileKind::VirtualFont, "root.vf").expect("VF request");
    let program =
        FileRequestKey::new(FileKind::PdfFontProgram, "leaf.pfb").expect("program request");
    let receipt = crate::PdfFontClosureReceipt {
        entries: vec![
            crate::PdfFontClosureReceiptEntry::File {
                request: vf,
                outcome: crate::PdfFontClosureResourceOutcome::Unavailable,
            },
            crate::PdfFontClosureReceiptEntry::File {
                request: program,
                outcome: crate::PdfFontClosureResourceOutcome::Resolved {
                    virtual_path: "/texlive/fonts/type1/leaf.pfb".to_owned(),
                    bytes: 3,
                    sha256: [0xab; 32],
                },
            },
        ],
    };

    assert_eq!(
        pdf_font_closure_receipt_bytes(&receipt).expect("render receipt"),
        concat!(
            "umber-pdf-font-closure-v1\n",
            "unavailable\tvf\troot.vf\ttex:root.vf\n",
            "resolved\tfont-program\tleaf.pfb\ttex:leaf.pfb\t",
            "/texlive/fonts/type1/leaf.pfb\t3\t",
            "abababababababababababababababababababababababababababababababab\n",
        )
        .as_bytes()
    );
}

#[test]
fn native_session_allows_the_hard_bounded_resource_attempt_count() {
    let directory = TempDir::new().expect("temporary project");
    let input = directory.path().join("main.tex");
    std::fs::write(&input, b"\\end").expect("main input");
    let options = NativeRunOptions {
        input,
        format: None,
        initial_prefetch_keys: Vec::new(),
        engine: EngineMode::Tex82,
        outputs: OutputCapabilitySet::DVI,
        html_asset_directory: None,
        distribution: None,
        distribution_sha256: None,
        offline: true,
        expansion_fuel: None,
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
        initial_prefetch_keys: Vec::new(),
        engine: EngineMode::Tex82,
        outputs: OutputCapabilitySet::DVI,
        html_asset_directory: None,
        distribution: Some(distribution.to_string_lossy().into_owned()),
        distribution_sha256: None,
        offline: false,
        expansion_fuel: None,
    };
    let cache_root = directory.path().join("cache");
    let cache = ObjectCache::new(&cache_root);
    let cancellation = FetchCancellation::new();
    let mut session = NativeCompileSession::new_with_cache(&options, &cancellation, cache.clone())
        .expect("session");
    let cold = session.compile(&cancellation).expect("cold compile");

    std::fs::remove_file(objects.join(object)).expect("remove source object");
    let spec = umber_fetch::VerifiedBlobSpec::content_addressed(
        "objects",
        &digest,
        package.len() as u64,
        package.len() as u64,
    )
    .expect("object blob specification");
    std::fs::remove_file(cache.entry_path(&spec)).expect("remove cached object");
    session
        .apply_source(RevisionId::new(2), edited)
        .expect("apply edit");
    let incremental = session.compile(&cancellation).expect("incremental compile");

    assert_ne!(incremental.dvi, cold.dvi);
    assert_eq!(session.source(), edited);
}

#[test]
fn bounded_distribution_owner_reuses_authenticated_state_and_preserves_detection_boundaries() {
    let directory = TempDir::new().expect("temporary project");
    let distribution = directory.path().join("distribution");
    std::fs::create_dir_all(&distribution).expect("distribution directory");
    let package = b"\\def\\sharedmanifeststate{1}";
    write_single_file_distribution(
        &distribution,
        "shared-owner",
        "tex:package.sty",
        "/texlive/tex/package.sty",
        package,
    );
    let root_path = distribution.join("manifest-v2.json");
    let root = std::fs::read(&root_path).expect("root manifest");
    let input = directory.path().join("main.tex");
    std::fs::write(&input, b"\\input package.sty \\end").expect("main input");
    let options = NativeRunOptions {
        input,
        format: None,
        initial_prefetch_keys: Vec::new(),
        engine: EngineMode::Tex82,
        outputs: OutputCapabilitySet::DVI,
        html_asset_directory: None,
        distribution: Some(distribution.to_string_lossy().into_owned()),
        distribution_sha256: Some(hex_digest(&root)),
        offline: true,
        expansion_fuel: None,
    };
    let cache = ObjectCache::new(directory.path().join("cache"));
    let owner = NativeDistributionOwner::with_cache(&options, cache);

    let cancellation = FetchCancellation::new();
    let mut cold =
        NativeCompileSession::new_with_distribution_owner(&options, &cancellation, &owner)
            .expect("cold session");
    let cold_output = cold.compile(&cancellation).expect("cold compile");
    let cold_counters = cold.host_telemetry().resolver;
    assert_eq!(cold_counters.manifest_reads, 2);
    assert_eq!(cold_counters.manifest_parses, 2);
    assert_eq!(cold_counters.manifest_authentications, 2);
    assert_eq!(cold_counters.shard_loads, 1);
    assert_eq!(cold_counters.object_hashes, 1);
    let cache_before = regular_file_inventory(directory.path().join("cache").as_path());

    let mut warm =
        NativeCompileSession::new_with_distribution_owner(&options, &cancellation, &owner)
            .expect("same-owner session");
    let warm_output = warm.compile(&cancellation).expect("same-owner compile");
    let warm_counters = warm.host_telemetry().resolver;
    assert_eq!(warm_output, cold_output, "shared state must be zero-loss");
    assert_eq!(warm_counters.manifest_reads, 0);
    assert_eq!(warm_counters.manifest_parses, 0);
    assert_eq!(warm_counters.manifest_authentications, 0);
    assert_eq!(warm_counters.shard_loads, 0);
    assert!(warm_counters.authenticated_manifest_hits >= 2);
    assert_eq!(warm_counters.object_hashes, 1);
    assert_eq!(warm_counters.object_cache_hits, 1);
    assert_eq!(
        regular_file_inventory(directory.path().join("cache").as_path()),
        cache_before,
        "same-owner reuse must not rewrite cache bytes"
    );

    std::fs::write(&root_path, b"mutated root").expect("mutate pinned root");
    let mut retained =
        NativeCompileSession::new_with_distribution_owner(&options, &cancellation, &owner)
            .expect("retained authenticated owner");
    assert_eq!(
        retained.compile(&cancellation).expect("immutable snapshot"),
        cold_output,
        "source mutation cannot change an already authenticated owner"
    );

    let fresh_owner = NativeDistributionOwner::with_cache(
        &options,
        ObjectCache::new(directory.path().join("fresh-cache")),
    );
    let mut fresh =
        NativeCompileSession::new_with_distribution_owner(&options, &cancellation, &fresh_owner)
            .expect("fresh session setup");
    assert!(matches!(
        fresh.compile(&cancellation),
        Err(NativeRunError::ManifestDigestMismatch { .. })
    ));

    let mut mismatched = options.clone();
    mismatched.offline = false;
    assert!(matches!(
        NativeCompileSession::new_with_distribution_owner(&mismatched, &cancellation, &owner),
        Err(NativeRunError::Selection(_))
    ));
}

#[test]
fn authenticated_owner_retains_only_selected_records_and_replays_unseen_keys_offline() {
    let directory = TempDir::new().expect("distribution tempdir");
    let objects = directory.path().join("objects");
    std::fs::create_dir_all(&objects).expect("objects directory");
    let first = b"first selected object";
    let later = b"later selected object";
    let first_digest = hex_digest(first);
    let later_digest = hex_digest(later);
    std::fs::write(objects.join(format!("sha256-{first_digest}")), first).expect("first object");
    std::fs::write(objects.join(format!("sha256-{later_digest}")), later).expect("later object");
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"compact-owner\",\"index\":0,\"files\":{{\"tex:first.sty\":{{\"virtualPath\":\"/texlive/tex/first.sty\",\"object\":\"sha256-{first_digest}\",\"sha256\":\"{first_digest}\",\"bytes\":{}}},\"tex:later.sty\":{{\"virtualPath\":\"/texlive/tex/later.sty\",\"object\":\"sha256-{later_digest}\",\"sha256\":\"{later_digest}\",\"bytes\":{}}}}}}}\n",
        first.len(),
        later.len(),
    );
    let (_, digests) = write_sharded_root(
        directory.path(),
        "compact-owner",
        0,
        &[(shard.as_str(), true)],
    );
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        true,
    );
    let project = TempDir::new().expect("isolated project");
    let local = local_resolver(project.path());
    let cancellation = FetchCancellation::new();
    let mut cold = ResolverTelemetry::default();
    let responses = resolver
        .resolve_batch_with_prefetch(
            &local,
            &needs(vec![file_request("first.sty"), image_request("absent.pdf")]),
            &cancellation,
            &mut cold,
        )
        .expect("cold authenticated selection")
        .responses;
    assert!(responses.iter().any(|response| matches!(
        response,
        ResourceResponse::File(file) if file.request.name() == "first.sty"
    )));
    assert!(responses.iter().any(|response| matches!(
        response,
        ResourceResponse::FileUnavailable(key) if key.name() == "absent.pdf"
    )));
    assert_eq!(cold.manifest_parses, 2);
    assert_eq!(cold.manifest_parse_peak_bytes, shard.len() as u64);
    assert_eq!(cold.retained_manifest_records, 1);
    assert_eq!(cold.retained_manifest_misses, 1);
    assert!(cold.retained_manifest_requested_bytes > 0);
    assert!(cold.retained_manifest_requested_bytes < 2_048);

    {
        let authenticated = resolver
            .authenticated
            .lock()
            .expect("authenticated distribution state");
        let loaded = authenticated.loaded.as_ref().expect("loaded distribution");
        assert!(matches!(
            loaded.selected_record("tex:first.sty"),
            Some(Some(_))
        ));
        assert!(matches!(
            loaded.selected_record("tex:absent.pdf"),
            Some(None)
        ));
        assert_eq!(loaded.selected_record("tex:later.sty"), None);
    }

    std::fs::remove_file(objects.join(format!("sha256-{}", digests[0])))
        .expect("remove local shard after persistent verified-cache publication");
    let mut extended = ResolverTelemetry::default();
    let responses = resolver
        .resolve_batch_with_prefetch(
            &local,
            &needs(vec![file_request("later.sty")]),
            &cancellation,
            &mut extended,
        )
        .expect("unseen key reparses the verified cached shard offline")
        .responses;
    assert!(matches!(
        responses.as_slice(),
        [ResourceResponse::File(file)] if file.request.name() == "later.sty" && file.bytes == later
    ));
    assert_eq!(extended.manifest_reads, 1);
    assert_eq!(extended.manifest_parses, 1);
    assert_eq!(extended.manifest_authentications, 1);
    assert_eq!(extended.manifest_cache_hits, 1);
    assert_eq!(extended.shard_loads, 1);
    assert_eq!(extended.manifest_parse_peak_bytes, shard.len() as u64);
    assert_eq!(extended.retained_manifest_records, 2);
    assert_eq!(extended.retained_manifest_misses, 1);

    let mut warm = ResolverTelemetry::default();
    resolver
        .resolve_batch_with_prefetch(
            &local,
            &needs(vec![
                file_request("first.sty"),
                file_request("later.sty"),
                image_request("absent.pdf"),
            ]),
            &cancellation,
            &mut warm,
        )
        .expect("all compact records replay without a shard parse");
    assert_eq!(warm.manifest_reads, 0);
    assert_eq!(warm.manifest_parses, 0);
    assert_eq!(warm.manifest_authentications, 0);
    assert_eq!(warm.shard_loads, 0);
    assert!(warm.authenticated_manifest_hits >= 2);
    assert_eq!(warm.manifest_parse_peak_bytes, 0);
    assert_eq!(warm.retained_manifest_records, 2);
    assert_eq!(warm.retained_manifest_misses, 1);
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
        initial_prefetch_keys: Vec::new(),
        engine: EngineMode::Tex82,
        outputs: OutputCapabilitySet::DVI,
        html_asset_directory: None,
        distribution: None,
        distribution_sha256: None,
        offline: true,
        expansion_fuel: None,
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

fn image_request(name: &str) -> ResourceRequest {
    ResourceRequest::File(FileRequest::new(
        crate::FileRequestKey::new(FileKind::Image, name).expect("image request key"),
        name,
    ))
}

fn needs(required: Vec<ResourceRequest>) -> NeedResources {
    NeedResources {
        required,
        probes: Vec::new(),
        prefetch_hints: Vec::new(),
    }
}

fn local_resolver(root: &Path) -> LocalResolver {
    LocalResolver {
        base: root.to_owned(),
        roots: vec![root.to_owned()],
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

#[test]
fn explicit_local_distribution_preserves_typed_pk_key_path_and_digest() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let bytes = include_bytes!("../../../../tests/corpus/pdf/pk_bitmap_600/cmr10.600pk");
    std::fs::write(directory.path().join("cmr10.600pk"), bytes).expect("PK fixture");
    let request = tex_fonts::PdfPkFontRequest::new(b"cmr10".to_vec(), 600, b"ljfour".to_vec());
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        None,
        None,
        true,
    );
    let responses = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &needs(vec![ResourceRequest::PkFont(request.clone())]),
            &FetchCancellation::new(),
        )
        .expect("offline local PK resolution");
    let [ResourceResponse::PkFont(resolved)] = responses.as_slice() else {
        panic!("typed PK response");
    };
    assert_eq!(resolved.request, request);
    assert_eq!(resolved.virtual_path, "/texlive/local/asset/cmr10.600pk");
    assert_eq!(resolved.bytes, bytes);
    assert_eq!(resolved.expected_sha256, Some(Sha256::digest(bytes).into()));
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

fn write_single_file_distribution(
    directory: &Path,
    distribution: &str,
    manifest_key: &str,
    virtual_path: &str,
    bytes: &[u8],
) {
    let objects = directory.join("objects");
    std::fs::create_dir_all(&objects).expect("distribution objects directory");
    let digest = hex_digest(bytes);
    let object = format!("sha256-{digest}");
    std::fs::write(objects.join(&object), bytes).expect("distribution object");
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"{distribution}\",\"index\":0,\"files\":{{\"{manifest_key}\":{{\"virtualPath\":\"{virtual_path}\",\"object\":\"{object}\",\"sha256\":\"{digest}\",\"bytes\":{}}}}}}}\n",
        bytes.len()
    );
    write_sharded_root(directory, distribution, 0, &[(shard.as_str(), true)]);
}

fn regular_file_inventory(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(base: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = std::fs::read_dir(directory)
            .expect("read cache directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("read cache entries");
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().expect("cache entry type");
            if file_type.is_dir() {
                visit(base, &path, files);
            } else if file_type.is_file() {
                files.insert(
                    path.strip_prefix(base)
                        .expect("cache-relative path")
                        .to_owned(),
                    std::fs::read(path).expect("cache entry bytes"),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn native_distribution_image_preserves_typed_identity_and_authenticated_bytes() {
    let directory = TempDir::new().expect("distribution tempdir");
    let bytes = b"authenticated image payload";
    write_single_file_distribution(
        directory.path(),
        "image",
        "tex:figure.pdf",
        "/texlive/tex/images/figure.pdf",
        bytes,
    );
    let project = TempDir::new().expect("isolated project");
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        true,
    );

    let responses = resolver
        .resolve_batch(
            &local_resolver(project.path()),
            &needs(vec![image_request("figure.pdf")]),
            &FetchCancellation::new(),
        )
        .expect("offline image resolution");
    let [ResourceResponse::File(file)] = responses.as_slice() else {
        panic!("typed image response: {responses:?}");
    };
    assert_eq!(file.request.kind(), FileKind::Image);
    assert_eq!(file.request.name(), "figure.pdf");
    assert_eq!(file.virtual_path, "/texlive/tex/images/figure.pdf");
    assert_eq!(file.bytes, bytes);
    assert_eq!(file.expected_digest, Some(FileContentId::for_bytes(bytes)));
}

#[test]
fn native_image_resolution_preserves_local_precedence() {
    let directory = TempDir::new().expect("distribution tempdir");
    write_single_file_distribution(
        directory.path(),
        "image-shadow",
        "tex:figure.pdf",
        "/texlive/tex/images/figure.pdf",
        b"distribution image",
    );
    let project = TempDir::new().expect("isolated project");
    std::fs::write(project.path().join("figure.pdf"), b"local image").expect("local image");
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        true,
    );

    let responses = resolver
        .resolve_batch(
            &local_resolver(project.path()),
            &needs(vec![image_request("figure.pdf")]),
            &FetchCancellation::new(),
        )
        .expect("local image resolution");
    let [ResourceResponse::File(file)] = responses.as_slice() else {
        panic!("typed local image response: {responses:?}");
    };
    assert_eq!(file.request.kind(), FileKind::Image);
    assert_eq!(file.bytes, b"local image");
    assert_eq!(file.virtual_path, "/texlive/local/image/figure.pdf");
    assert!(
        resolver
            .authenticated
            .lock()
            .expect("authenticated distribution state")
            .loaded
            .is_none(),
        "local hit must not load distribution"
    );
}

#[test]
fn native_distribution_non_image_payload_reaches_malformed_image_diagnostic() {
    let directory = TempDir::new().expect("distribution tempdir");
    write_single_file_distribution(
        directory.path(),
        "malformed-image",
        "tex:figure.pdf",
        "/texlive/tex/images/figure.pdf",
        b"not an image",
    );
    let project = TempDir::new().expect("isolated project");
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        true,
    );
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        outputs: OutputCapabilitySet::PDF,
        ..SessionOptions::default()
    })
    .expect("PDF session");
    session
        .add_user_file(
            "main.tex",
            b"\\pdfoutput=1 \\pdfximage figure.pdf \\end".to_vec(),
        )
        .expect("main file");
    let CompileAttemptResult::NeedResources(batch) = session.compile_attempt() else {
        panic!("image request");
    };

    let responses = resolver
        .resolve_batch(
            &local_resolver(project.path()),
            &batch,
            &FetchCancellation::new(),
        )
        .expect("malformed image bytes remain an available resource");
    assert!(matches!(
        responses.as_slice(),
        [ResourceResponse::File(file)] if file.request.kind() == FileKind::Image
    ));
    session
        .provide_resources(responses)
        .expect("provision authenticated image bytes");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Error(CompileError::Diagnostic(diagnostic))
            if diagnostic.message.contains("image type is not PDF, PNG, or JPEG")
                && !diagnostic.message.contains("image is unavailable")
    ));
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
        true,
    );
    let responses = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &needs(vec![file_request("missing.sty")]),
            &FetchCancellation::new(),
        )
        .expect("authoritative absence");
    assert!(matches!(
        responses.as_slice(),
        [ResourceResponse::FileUnavailable(key)] if key.name() == "missing.sty"
    ));
}

#[test]
fn missing_local_shard_diagnostic_names_identity_and_bounds_request_keys() {
    let directory = TempDir::new().expect("distribution tempdir");
    let shard = "{\"schema\":1,\"distribution\":\"missing-shard\",\"index\":0,\"files\":{}}\n";
    let (_, digests) = write_sharded_root(directory.path(), "missing-shard", 0, &[(shard, false)]);
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        true,
    );
    let requests = (0..6)
        .map(|index| file_request(&format!("missing-{index}.sty")))
        .collect();

    let error = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &needs(requests),
            &FetchCancellation::new(),
        )
        .expect_err("missing shard must fail before claiming absence");
    let message = error.to_string();
    assert!(matches!(
        error,
        NativeRunError::DistributionShardUnavailable { index: 0, .. }
    ));
    assert!(message.contains("index=0"));
    assert!(message.contains(&format!("digest={}", digests[0])));
    for index in 0..4 {
        assert!(message.contains(&format!("tex:missing-{index}.sty")));
    }
    assert!(message.contains("(+2 more)"));
    assert!(!message.contains("tex:missing-4.sty"));
    assert!(!message.contains("tex:missing-5.sty"));
}

#[test]
fn distribution_tex_input_uses_web2c_ordered_appended_tex_fallback() {
    let directory = TempDir::new().expect("distribution tempdir");
    let objects = directory.path().join("objects");
    std::fs::create_dir_all(&objects).expect("objects directory");
    let exact = b"exact file wins";
    let shadow = b"appended shadow loses";
    let fallback = b"appended fallback";
    let wrong_kind = b"not a font-program fallback";
    let entries = [
        ("tex:exact.ltd", "/texlive/exact.ltd", exact.as_slice()),
        (
            "tex:exact.ltd.tex",
            "/texlive/exact.ltd.tex",
            shadow.as_slice(),
        ),
        (
            "tex:fallback.ltd.tex",
            "/texlive/fallback.ltd.tex",
            fallback.as_slice(),
        ),
        (
            "tex:program.pfb.tex",
            "/texlive/program.pfb.tex",
            wrong_kind.as_slice(),
        ),
    ];
    let mut files = Vec::new();
    for (key, virtual_path, bytes) in entries {
        let digest = hex_digest(bytes);
        std::fs::write(objects.join(format!("sha256-{digest}")), bytes)
            .expect("distribution object");
        files.push(format!(
            "\"{key}\":{{\"virtualPath\":\"{virtual_path}\",\"object\":\"sha256-{digest}\",\"sha256\":\"{digest}\",\"bytes\":{}}}",
            bytes.len()
        ));
    }
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"appended-tex\",\"index\":0,\"files\":{{{}}}}}\n",
        files.join(",")
    );
    write_sharded_root(
        directory.path(),
        "appended-tex",
        0,
        &[(shard.as_str(), true)],
    );
    let program = FileRequest::new(
        crate::FileRequestKey::new(FileKind::PdfFontProgram, "program.pfb")
            .expect("program request"),
        "program.pfb",
    );
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        true,
    );

    let responses = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &needs(vec![
                file_request("exact.ltd"),
                file_request("fallback.ltd"),
                ResourceRequest::File(program),
            ]),
            &FetchCancellation::new(),
        )
        .expect("ordered distribution lookup");
    assert!(responses.iter().any(|response| matches!(
        response,
        ResourceResponse::File(file)
            if file.request.name() == "exact.ltd" && file.bytes == exact
    )));
    assert!(responses.iter().any(|response| matches!(
        response,
        ResourceResponse::File(file)
            if file.request.name() == "fallback.ltd"
                && file.virtual_path == "/texlive/fallback.ltd.tex"
                && file.bytes == fallback
    )));
    assert!(responses.iter().any(|response| matches!(
        response,
        ResourceResponse::FileUnavailable(key)
            if key.kind() == FileKind::PdfFontProgram && key.name() == "program.pfb"
    )));
}

#[test]
fn offline_local_distribution_reports_a_missing_object_distinctly_from_a_missing_key() {
    let directory = TempDir::new().expect("distribution tempdir");
    let bytes = b"object deliberately absent from mirror";
    let digest = hex_digest(bytes);
    let mut shard = format!(
        "{{\"schema\":1,\"distribution\":\"missing-object\",\"index\":0,\"files\":{{\"tex:present.sty\":{{\"virtualPath\":\"/texlive/tex/present.sty\",\"object\":\"sha256-{digest}\",\"sha256\":\"{digest}\",\"bytes\":{}",
        bytes.len()
    );
    shard.push_str("}}}\n");
    write_sharded_root(
        directory.path(),
        "missing-object",
        0,
        &[(shard.as_str(), true)],
    );
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        true,
    );

    let error = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &needs(vec![file_request("present.sty")]),
            &FetchCancellation::new(),
        )
        .expect_err("missing mirror object must fail");
    assert!(matches!(error, NativeRunError::Io { .. }));
}

#[test]
fn exact_snapshot_delivers_corpus_tex_tfm_type1_and_vf_requests_offline() {
    let snapshot = test_support::repository_root()
        .join("crates/umber")
        .join("../..")
        .join("target/texlive-snapshot");
    if !snapshot.join("manifest.json").is_file() {
        return;
    }
    let cache = TempDir::new().expect("isolated cold cache");
    let requests = [
        (FileKind::TexInput, "physics.sty", "tex:physics.sty"),
        (FileKind::Tfm, "ecrm1728.tfm", "tfm:ecrm1728.tfm"),
        (FileKind::PdfFontProgram, "cmmib7.pfb", "tex:cmmib7.pfb"),
        (FileKind::VirtualFont, "ptmbc8t.vf", "tex:ptmbc8t.vf"),
    ];
    let batch = needs(
        requests
            .iter()
            .map(|(kind, name, _)| {
                ResourceRequest::File(FileRequest::new(
                    crate::FileRequestKey::new(*kind, name).expect("snapshot request key"),
                    *name,
                ))
            })
            .collect(),
    );
    let project = TempDir::new().expect("isolated project");
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(cache.path()),
        Some(snapshot.to_string_lossy().into_owned()),
        None,
        true,
    );

    let responses = resolver
        .resolve_batch(
            &local_resolver(project.path()),
            &batch,
            &FetchCancellation::new(),
        )
        .expect("cold explicit snapshot resolution must work offline");
    assert_eq!(responses.len(), requests.len());
    for (kind, name, manifest_key) in requests {
        let file = responses
            .iter()
            .find_map(|response| match response {
                ResourceResponse::File(file)
                    if file.request.kind() == kind && file.request.name() == name =>
                {
                    Some(file)
                }
                _ => None,
            })
            .expect("typed snapshot response");
        let authenticated = resolver
            .authenticated
            .lock()
            .expect("authenticated distribution state");
        let entry = authenticated
            .loaded
            .as_ref()
            .expect("loaded snapshot")
            .selected_record(manifest_key)
            .expect("selected-key evidence")
            .as_ref()
            .expect("compact record from canonical authenticated shard");
        assert_eq!(hex_digest(&file.bytes), entry.object.sha256);
        assert_eq!(
            file.expected_digest,
            Some(FileContentId::for_bytes(&file.bytes))
        );
    }
}

#[test]
fn native_virtual_font_resolution_preserves_typed_identity_and_reuses_cache() {
    let directory = TempDir::new().expect("distribution tempdir");
    let vf = b"typed-vf-object";
    let digest = hex_digest(vf);
    let object = format!("sha256-{digest}");
    let objects = directory.path().join("objects");
    std::fs::create_dir_all(&objects).expect("objects directory");
    std::fs::write(objects.join(&object), vf).expect("VF object");
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"vf-cache\",\"index\":0,\"files\":{{\"tex:root.vf\":{{\"virtualPath\":\"/texlive/fonts/vf/root.vf\",\"object\":\"{object}\",\"sha256\":\"{digest}\",\"bytes\":{}}}}}}}\n",
        vf.len()
    );
    write_sharded_root(directory.path(), "vf-cache", 0, &[(shard.as_str(), true)]);
    let cache = directory.path().join("cache");
    let key = crate::FileRequestKey::new(FileKind::VirtualFont, "root.vf").expect("VF key");
    let batch = needs(vec![ResourceRequest::File(FileRequest::new(
        key.clone(),
        "root",
    ))]);
    let cancellation = FetchCancellation::new();
    let mut cold = DistributionResolver::new(
        ObjectCache::new(&cache),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    let responses = cold
        .resolve_batch(&local_resolver(directory.path()), &batch, &cancellation)
        .expect("cold VF acquisition");
    assert!(matches!(
        responses.as_slice(),
        [ResourceResponse::File(file)]
            if file.request == key && file.bytes == vf
    ));

    std::fs::remove_file(objects.join(object)).expect("remove distribution VF object");
    let mut warm = DistributionResolver::new(
        ObjectCache::new(&cache),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        true,
    );
    let responses = warm
        .resolve_batch(&local_resolver(directory.path()), &batch, &cancellation)
        .expect("warm offline VF acquisition");
    assert!(matches!(
        responses.as_slice(),
        [ResourceResponse::File(file)]
            if file.request == key && file.bytes == vf
    ));
}

#[test]
fn explicit_local_distribution_resolves_nested_ec_tfm_record() {
    let directory = TempDir::new().expect("distribution tempdir");
    let metric = b"EC typewriter metric";
    let digest = hex_digest(metric);
    let object = format!("sha256-{digest}");
    let mut shard = format!(
        "{{\"schema\":1,\"distribution\":\"ec-tfm\",\"index\":0,\"files\":{{\"tfm:ectt0800.tfm\":{{\"virtualPath\":\"/texlive/fonts/tfm/jknappen/ec/ectt0800.tfm\",\"object\":\"{object}\",\"sha256\":\"{digest}\",\"bytes\":{}",
        metric.len()
    );
    shard.push_str("}}}\n");
    write_sharded_root(directory.path(), "ec-tfm", 0, &[(shard.as_str(), true)]);
    std::fs::write(directory.path().join("objects").join(object), metric).expect("TFM object");
    let key = crate::FileRequestKey::new(FileKind::Tfm, "ectt0800.tfm").expect("TFM key");
    let request = FileRequest::new(key.clone(), "ectt0800");
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );

    let responses = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &needs(vec![ResourceRequest::File(request)]),
            &FetchCancellation::new(),
        )
        .expect("EC TFM acquisition");
    let [ResourceResponse::File(resolved)] = responses.as_slice() else {
        panic!("typed TFM response");
    };
    assert_eq!(resolved.request, key);
    assert_eq!(
        resolved.virtual_path,
        "/texlive/fonts/tfm/jknappen/ec/ectt0800.tfm"
    );
    assert_eq!(resolved.bytes, metric);
    assert_eq!(
        resolved.expected_digest,
        Some(FileContentId::for_bytes(metric))
    );
}

#[test]
fn local_resolution_owns_virtual_and_request_path_receipt_aliases() {
    let directory = TempDir::new().expect("local resolution tempdir");
    let path = directory.path().join("owned.ltx");
    std::fs::write(&path, b"owned bytes").expect("write local input");
    let resolver = local_resolver(directory.path());
    let request = FileRequest::new(
        crate::FileRequestKey::new(FileKind::TexInput, "owned.ltx").expect("local request key"),
        "owned.ltx",
    );

    let resolved = resolver.resolve(&request).expect("resolve local input");
    let path_map = resolver.input_path_map();

    assert_eq!(path_map.get(Path::new("owned.ltx")), Some(&path));
    assert_eq!(path_map.get(Path::new(&resolved.virtual_path)), Some(&path));
    assert_eq!(resolver.resolved_inputs(), vec![(path, 11)]);
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
            &needs(vec![ResourceRequest::Font(FontRequest {
                key: key.clone(),
                accepted_containers: AcceptedFontContainers::NATIVE_WITH_COLLECTIONS,
                purposes: FontPurposes::LAYOUT,
            })]),
            &FetchCancellation::new(),
        )
        .expect("authoritative font absence");
    assert_eq!(responses, vec![ResourceResponse::FontUnavailable(key)]);
}

#[test]
fn generic_pdf_asset_uses_the_snapshot_tex_vocabulary() {
    let directory = TempDir::new().expect("distribution tempdir");
    let bytes = b"cmr10 CMR10 <cmr10.pfb\n";
    let digest = hex_digest(bytes);
    let object = format!("sha256-{digest}");
    let mut shard = format!(
        "{{\"schema\":1,\"distribution\":\"pdf-assets\",\"index\":0,\"files\":{{\"tex:pdftex.map\":{{\"virtualPath\":\"/texlive/fonts/map/pdftex.map\",\"object\":\"{object}\",\"sha256\":\"{digest}\",\"bytes\":{}",
        bytes.len()
    );
    shard.push_str("}}}\n");
    write_sharded_root(directory.path(), "pdf-assets", 0, &[(&shard, true)]);
    std::fs::write(directory.path().join("objects").join(object), bytes).expect("map object");
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    assert_eq!(
        resolver
            .resolve_generic_file(
                &local_resolver(directory.path()),
                b"pdftex.map",
                &FetchCancellation::new(),
            )
            .expect("generic asset resolves")
            .bytes,
        bytes
    );
}

#[test]
fn live_lookup_does_not_hash_an_unrequested_inline_hint() {
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
    std::fs::write(objects.join(&dependency_object), b"corrupt-unrequested")
        .expect("corrupt dependency object");
    let cache = ObjectCache::new(directory.path().join("cache"));
    let mut resolver = DistributionResolver::new(
        cache.clone(),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    let mut telemetry = ResolverTelemetry::default();
    let resolved = resolver
        .resolve_batch_with_prefetch(
            &local_resolver(directory.path()),
            &needs(vec![file_request("article.cls")]),
            &FetchCancellation::new(),
            &mut telemetry,
        )
        .expect("required object resolution");
    assert!(matches!(
        resolved.responses.as_slice(),
        [ResourceResponse::File(_)]
    ));
    assert_eq!(telemetry.object_requests, 1);
    assert_eq!(telemetry.object_hashes, 1);
    assert_eq!(telemetry.object_cache_hits, 0);
    assert!(telemetry.local_lookups > 0);
    assert!(telemetry.manifest_lookups > 0);
    let dependency_spec = umber_fetch::VerifiedBlobSpec::content_addressed(
        "objects",
        dependency_digest,
        dependency_bytes.len() as u64,
        dependency_bytes.len() as u64,
    )
    .expect("dependency specification");
    assert!(
        !cache.entry_path(&dependency_spec).exists(),
        "an unrequested dependency hint must not enter the live cache"
    );
}

#[test]
fn schema_three_format_closure_does_not_drive_native_live_lookup() {
    let directory = TempDir::new().expect("distribution tempdir");
    let objects = directory.path().join("objects");
    std::fs::create_dir_all(&objects).expect("objects directory");
    let format_bytes = b"format";
    let required_bytes = b"required";
    let closure_bytes = b"closure";
    let format_digest = hex_digest(format_bytes);
    let required_digest = hex_digest(required_bytes);
    let closure_digest = hex_digest(closure_bytes);
    for (digest, bytes) in [
        (&format_digest, format_bytes.as_slice()),
        (&required_digest, required_bytes.as_slice()),
        (&closure_digest, closure_bytes.as_slice()),
    ] {
        std::fs::write(objects.join(format!("sha256-{digest}")), bytes).expect("object");
    }
    let required_entry = format!(
        "{{\"virtualPath\":\"/texlive/article.cls\",\"object\":\"sha256-{required_digest}\",\"sha256\":\"{required_digest}\",\"bytes\":{}}}",
        required_bytes.len()
    );
    let closure_entry = format!(
        "{{\"virtualPath\":\"/texlive/latex.ltx\",\"object\":\"sha256-{closure_digest}\",\"sha256\":\"{closure_digest}\",\"bytes\":{}}}",
        closure_bytes.len()
    );
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"closure\",\"index\":0,\"files\":{{\"tex:article.cls\":{required_entry},\"tex:latex.ltx\":{closure_entry}}}}}\n"
    );
    let shard_digest = hex_digest(shard.as_bytes());
    std::fs::write(objects.join(format!("sha256-{shard_digest}")), shard).expect("shard");
    let root = format!(
        "{{\"schema\":3,\"distribution\":\"closure\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"],\"formats\":{{\"latex\":{{\"object\":\"sha256-{format_digest}\",\"sha256\":\"{format_digest}\",\"bytes\":{},\"engine\":\"umber\",\"engineVersion\":\"{}\",\"formatSchema\":11,\"sourceDistribution\":\"closure\",\"sourceManifestSha256\":\"{}\",\"sourceDateEpoch\":0,\"inputClosure\":{{\"schema\":1,\"keys\":[\"tex:latex.ltx\",\"tex:stale.tex\"]}}}}}}}}\n",
        format_bytes.len(),
        crate::PACKAGE_VERSION,
        "1".repeat(64)
    );
    std::fs::write(directory.path().join("manifest-v3.json"), root).expect("root");
    let cache = ObjectCache::new(directory.path().join("cache"));
    let mut resolver = DistributionResolver::new(
        cache.clone(),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    let mut telemetry = ResolverTelemetry::default();
    let format = resolver
        .resolve_format(
            Path::new("latex.fmt"),
            EngineMode::Latex,
            &FetchCancellation::new(),
            &mut telemetry,
        )
        .expect("format resolution");
    assert_eq!(format.bytes, format_bytes);
    let responses = resolver
        .resolve_batch_with_prefetch(
            &local_resolver(directory.path()),
            &NeedResources {
                required: vec![file_request("article.cls")],
                probes: Vec::new(),
                prefetch_hints: Vec::new(),
            },
            &FetchCancellation::new(),
            &mut telemetry,
        )
        .expect("required batch");
    assert!(matches!(
        responses.responses.as_slice(),
        [ResourceResponse::File(file)] if file.request.name() == "article.cls"
    ));
    assert_eq!(telemetry.object_requests, 2, "format plus required file");
    assert_eq!(telemetry.object_hashes, 2, "format plus required file");
    let closure_spec = umber_fetch::VerifiedBlobSpec::content_addressed(
        "objects",
        closure_digest,
        closure_bytes.len() as u64,
        closure_bytes.len() as u64,
    )
    .expect("closure specification");
    assert!(
        !cache.entry_path(&closure_spec).exists(),
        "format closure metadata must not cause live object work"
    );
}

fn write_locally_shadowed_hint_distribution(directory: &Path) {
    let required_bytes = b"\\message{DIST-ARTICLE}";
    let required_digest = hex_digest(required_bytes);
    let shadowed_bytes = b"\\message{DIST-REVTEX}";
    let shadowed_digest = hex_digest(shadowed_bytes);
    let required_object = format!("sha256-{required_digest}");
    let shadowed_object = format!("sha256-{shadowed_digest}");
    let shard = "{\"schema\":1,\"distribution\":\"shadowing\",\"index\":0,\"files\":{\"tex:article.cls\":{\"virtualPath\":\"/texlive/tex/article.cls\",\"object\":\"$REQUIRED_OBJECT\",\"sha256\":\"$REQUIRED_DIGEST\",\"bytes\":$REQUIRED_BYTES,\"dependencies\":[{\"key\":\"tex:revtex4-1.cls\",\"virtualPath\":\"/texlive/tex/revtex4-1.cls\",\"object\":\"$SHADOWED_OBJECT\",\"sha256\":\"$SHADOWED_DIGEST\",\"bytes\":$SHADOWED_BYTES}]},\"tex:revtex4-1.cls\":{\"virtualPath\":\"/texlive/tex/revtex4-1.cls\",\"object\":\"$SHADOWED_OBJECT\",\"sha256\":\"$SHADOWED_DIGEST\",\"bytes\":$SHADOWED_BYTES}}}\n"
        .replace("$REQUIRED_OBJECT", &required_object)
        .replace("$REQUIRED_DIGEST", &required_digest)
        .replace("$REQUIRED_BYTES", &required_bytes.len().to_string())
        .replace("$SHADOWED_OBJECT", &shadowed_object)
        .replace("$SHADOWED_DIGEST", &shadowed_digest)
        .replace("$SHADOWED_BYTES", &shadowed_bytes.len().to_string());
    write_sharded_root(directory, "shadowing", 0, &[(&shard, true)]);
    let objects = directory.join("objects");
    std::fs::write(objects.join(required_object), required_bytes).expect("required object");
    std::fs::write(objects.join(shadowed_object), shadowed_bytes).expect("shadowed object");
}

#[test]
fn distribution_prefetch_does_not_claim_a_locally_shadowed_alias() {
    let directory = TempDir::new().expect("distribution tempdir");
    write_locally_shadowed_hint_distribution(directory.path());
    std::fs::write(
        directory.path().join("revtex4-1.cls"),
        b"\\message{LOCAL-REVTEX}",
    )
    .expect("local class");
    let mut resolver = DistributionResolver::new(
        ObjectCache::new(directory.path().join("cache")),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );

    let resolved = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &NeedResources {
                required: vec![file_request("article.cls")],
                probes: Vec::new(),
                prefetch_hints: Vec::new(),
            },
            &FetchCancellation::new(),
        )
        .expect("required distribution file");

    assert!(matches!(
        resolved.as_slice(),
        [ResourceResponse::File(file)] if file.request.name() == "article.cls"
    ));
}

#[test]
fn native_compile_uses_local_file_after_shadowed_distribution_hint() {
    let directory = TempDir::new().expect("distribution tempdir");
    write_locally_shadowed_hint_distribution(directory.path());
    std::fs::write(
        directory.path().join("revtex4-1.cls"),
        b"\\message{LOCAL-REVTEX}",
    )
    .expect("local class");
    let input = directory.path().join("main.tex");
    std::fs::write(&input, b"\\input article.cls \\input revtex4-1.cls \\end").expect("main input");
    let options = NativeRunOptions {
        input,
        format: None,
        engine: EngineMode::Tex82,
        outputs: OutputCapabilitySet::DVI,
        html_asset_directory: None,
        distribution: Some(directory.path().to_string_lossy().into_owned()),
        distribution_sha256: None,
        offline: false,
        initial_prefetch_keys: Vec::new(),
        expansion_fuel: None,
    };
    let cancellation = FetchCancellation::new();
    let mut session = NativeCompileSession::new_with_cache(
        &options,
        &cancellation,
        ObjectCache::new(directory.path().join("cache")),
    )
    .expect("native session");

    let output = session
        .compile(&cancellation)
        .expect("local shadowing compile");
    let terminal = String::from_utf8_lossy(&output.terminal);
    assert!(terminal.contains("DIST-ARTICLE"), "{terminal}");
    assert!(terminal.contains("LOCAL-REVTEX"), "{terminal}");
    assert!(!terminal.contains("DIST-REVTEX"), "{terminal}");
}

#[test]
fn incompatible_format_schema_is_rejected_before_cache_lookup_or_acquisition() {
    let directory = TempDir::new().expect("distribution tempdir");
    let format_bytes = b"format that must not be acquired";
    let format_digest = hex_digest(format_bytes);
    let shard_digest = "0".repeat(64);
    let incompatible_schema = tex_state::FORMAT_SCHEMA_VERSION + 1;
    let root = format!(
        "{{\"schema\":3,\"distribution\":\"schema-preflight\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"],\"formats\":{{\"latex\":{{\"object\":\"sha256-{format_digest}\",\"sha256\":\"{format_digest}\",\"bytes\":{},\"engine\":\"umber\",\"engineVersion\":\"{}\",\"formatSchema\":{incompatible_schema},\"sourceDistribution\":\"schema-preflight\",\"sourceManifestSha256\":\"{}\",\"sourceDateEpoch\":0}}}}}}\n",
        format_bytes.len(),
        crate::PACKAGE_VERSION,
        "1".repeat(64)
    );
    std::fs::write(directory.path().join("manifest-v3.json"), root).expect("root manifest");

    let cache_root = directory.path().join("cache");
    let cached_object = cache_root
        .join("objects")
        .join(format!("sha256-{format_digest}"));
    std::fs::create_dir_all(cached_object.parent().expect("cache objects directory"))
        .expect("cache objects directory");
    let lookup_sentinel = b"corrupt cache sentinel";
    std::fs::write(&cached_object, lookup_sentinel).expect("cache lookup sentinel");

    let mut resolver = DistributionResolver::new(
        ObjectCache::new(&cache_root),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    let mut telemetry = ResolverTelemetry::default();
    let error = match resolver.resolve_format(
        Path::new("latex.fmt"),
        EngineMode::Latex,
        &FetchCancellation::new(),
        &mut telemetry,
    ) {
        Ok(_) => panic!("incompatible format schema was accepted"),
        Err(error) => error,
    };

    assert_eq!(
        error.to_string(),
        format!(
            "format resource error: format latex uses schema {incompatible_schema}; this runtime requires schema {}",
            tex_state::FORMAT_SCHEMA_VERSION
        )
    );
    assert_eq!(
        std::fs::read(cached_object).expect("untouched lookup sentinel"),
        lookup_sentinel,
        "schema preflight must run before the object cache removes corrupt entries"
    );
}

#[test]
fn format_closure_is_loaded_only_as_each_input_is_requested() {
    for (engine, closure_len) in [(EngineMode::Latex, 57), (EngineMode::PdfLatex, 60)] {
        let directory = TempDir::new().expect("distribution tempdir");
        let distribution = directory.path().join("distribution");
        let objects = distribution.join("objects");
        std::fs::create_dir_all(&objects).expect("objects directory");

        let mut recipe = if engine == EngineMode::Latex {
            crate::FormatRecipe::raw_etex26()
        } else {
            crate::FormatRecipe::production_pdftex14029()
        };
        recipe.engine = engine;
        recipe.format_name = format!("closure-{engine:?}");
        recipe.format_ident_name = recipe.format_name.clone();
        recipe.distribution_identity = format!("closure-test-{engine:?}").into_bytes();
        let format = crate::format_fixture::construct_format_in_worker(&recipe)
            .expect("schema-11 format")
            .image;
        let format_digest = hex_digest(&format);
        std::fs::write(objects.join(format!("sha256-{format_digest}")), &format)
            .expect("format object");

        let mut closure_keys = Vec::new();
        let mut shard_entries = Vec::new();
        let mut closure_objects = Vec::new();
        for index in 0..closure_len {
            let name = format!("closure-{index:02}.tex");
            let key = format!("tex:{name}");
            let bytes = if index + 1 == closure_len {
                b"\\end".to_vec()
            } else {
                format!("\\input closure-{:02}\n", index + 1).into_bytes()
            };
            let digest = hex_digest(&bytes);
            std::fs::write(objects.join(format!("sha256-{digest}")), &bytes)
                .expect("closure object");
            closure_keys.push(format!("\"{key}\""));
            shard_entries.push(format!(
            "\"{key}\":{{\"virtualPath\":\"/texlive/{name}\",\"object\":\"sha256-{digest}\",\"sha256\":\"{digest}\",\"bytes\":{}}}",
            bytes.len()
        ));
            closure_objects.push((digest, bytes.len() as u64));
        }
        let shard = format!(
            "{{\"schema\":1,\"distribution\":\"closure-attempts\",\"index\":0,\"files\":{{{}}}}}\n",
            shard_entries.join(",")
        );
        let shard_digest = hex_digest(shard.as_bytes());
        std::fs::write(objects.join(format!("sha256-{shard_digest}")), shard)
            .expect("shard object");
        let root = format!(
            "{{\"schema\":3,\"distribution\":\"closure-attempts\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"],\"formats\":{{\"probe\":{{\"object\":\"sha256-{format_digest}\",\"sha256\":\"{format_digest}\",\"bytes\":{},\"engine\":\"umber\",\"engineVersion\":\"{}\",\"formatSchema\":11,\"sourceDistribution\":\"closure-attempts\",\"sourceManifestSha256\":\"{}\",\"sourceDateEpoch\":0,\"inputClosure\":{{\"schema\":1,\"keys\":[{}]}}}}}}}}\n",
            format.len(),
            crate::PACKAGE_VERSION,
            "1".repeat(64),
            closure_keys.join(",")
        );
        std::fs::write(distribution.join("manifest-v3.json"), root).expect("root manifest");

        let input = directory.path().join("main.tex");
        std::fs::write(&input, b"\\input closure-00\n").expect("main input");
        let cache = ObjectCache::new(directory.path().join("cache"));
        let cancellation = FetchCancellation::new();
        let mut session = NativeCompileSession::new_with_cache(
            &NativeRunOptions {
                input,
                format: Some(PathBuf::from("probe.fmt")),
                initial_prefetch_keys: Vec::new(),
                engine,
                outputs: OutputCapabilitySet::DVI,
                html_asset_directory: None,
                distribution: Some(distribution.to_string_lossy().into_owned()),
                distribution_sha256: None,
                offline: false,
                expansion_fuel: None,
            },
            &cancellation,
            cache.clone(),
        )
        .expect("native session");

        let CompileAttemptResult::NeedResources(first) = session.session.compile_attempt() else {
            panic!("first attempt must miss the closure head");
        };
        assert_eq!(first.required.len(), 1);
        assert!(first.prefetch_hints.is_empty());
        let responses = session
            .distribution
            .resolve_batch(&session.local, &first, &cancellation)
            .expect("first required input");
        assert_eq!(responses.len(), 1);
        for (index, (digest, bytes)) in closure_objects.iter().enumerate() {
            let spec =
                umber_fetch::VerifiedBlobSpec::content_addressed("objects", digest, *bytes, *bytes)
                    .expect("closure object specification");
            assert!(
                cache.entry_path(&spec).exists() == (index == 0),
                "only the first requested closure object may be cached"
            );
        }
        session
            .session
            .provide_resources(responses)
            .expect("provide closure head");

        session.compile(&cancellation).expect("complete chain");
        assert_eq!(session.session.attempts(), closure_len + 1);
    }
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
            &needs(vec![file_request("cached.sty")]),
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
            &needs(vec![file_request("cached.sty")]),
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
            &needs(vec![file_request("missing.sty")]),
            &FetchCancellation::new(),
        ),
        Err(NativeRunError::ManifestDigestMismatch { .. })
    ));

    let cancellation = FetchCancellation::new();
    cancellation.cancel();
    assert!(matches!(
        resolver.resolve_batch(
            &local_resolver(directory.path()),
            &needs(vec![file_request("missing.sty")]),
            &cancellation,
        ),
        Err(NativeRunError::Cancelled)
    ));
}

#[test]
fn shard_partition_uses_sha256_network_prefix_bits() {
    assert_eq!(shard_index_for_key("tex:article.cls", 8), Ok(0x45));
    assert_eq!(shard_index_for_key("tfm:cmr10.tfm", 8), Ok(0x91));
    assert_eq!(shard_index_for_key("tex:plain.tex", 0), Ok(0));
}
