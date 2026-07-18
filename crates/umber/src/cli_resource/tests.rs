#![allow(clippy::disallowed_methods)] // Host-side resource/cache integration fixtures.

use std::time::Instant;
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
        initial_prefetch_keys: Vec::new(),
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
        initial_prefetch_keys: Vec::new(),
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
        initial_prefetch_keys: Vec::new(),
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
            .resolve_generic_asset(
                &local_resolver(directory.path()),
                b"pdftex.map",
                &FetchCancellation::new(),
            )
            .expect("generic asset resolves"),
        bytes
    );
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
            &needs(vec![file_request("article.cls")]),
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
fn schema_three_format_closure_publishes_local_overrides_and_ignores_stale_hints() {
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
        "{{\"schema\":3,\"distribution\":\"closure\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"],\"formats\":{{\"latex\":{{\"object\":\"sha256-{format_digest}\",\"sha256\":\"{format_digest}\",\"bytes\":{},\"engine\":\"umber\",\"engineVersion\":\"{}\",\"formatSchema\":10,\"sourceDistribution\":\"closure\",\"sourceManifestSha256\":\"{}\",\"sourceDateEpoch\":0,\"inputClosure\":{{\"schema\":1,\"keys\":[\"tex:latex.ltx\",\"tex:stale.tex\"]}}}}}}}}\n",
        format_bytes.len(),
        crate::PACKAGE_VERSION,
        "1".repeat(64)
    );
    std::fs::write(directory.path().join("manifest-v3.json"), root).expect("root");
    let local_closure = b"local closure";
    std::fs::write(directory.path().join("latex.ltx"), local_closure).expect("local override");
    let cache = ObjectCache::new(directory.path().join("cache"));
    let mut resolver = DistributionResolver::new(
        cache.clone(),
        Some(directory.path().to_string_lossy().into_owned()),
        None,
        false,
    );
    let format = resolver
        .resolve_format(
            Path::new("latex.fmt"),
            EngineMode::Latex,
            &FetchCancellation::new(),
        )
        .expect("format resolution");
    assert_eq!(format.bytes, format_bytes);
    assert_eq!(format.prefetch_hints.len(), 2);
    let responses = resolver
        .resolve_batch(
            &local_resolver(directory.path()),
            &NeedResources {
                required: vec![file_request("article.cls")],
                probes: Vec::new(),
                prefetch_hints: format.prefetch_hints,
            },
            &FetchCancellation::new(),
        )
        .expect("closure batch");
    assert_eq!(responses.len(), 2);
    let closure = responses
        .iter()
        .find_map(|response| match response {
            ResourceResponse::File(file) if file.request.name() == "latex.ltx" => Some(file),
            _ => None,
        })
        .expect("positive closure response");
    assert_eq!(closure.bytes, local_closure);
    assert!(responses.iter().all(|response| !matches!(
        response,
        ResourceResponse::FileUnavailable(key) if key.name() == "stale.tex"
    )));
    assert_eq!(
        cache
            .load_object(&closure_digest, closure_bytes.len() as u64)
            .expect("closure cache"),
        None,
        "the local closure must take precedence over distribution speculation"
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
        html: false,
        distribution: Some(directory.path().to_string_lossy().into_owned()),
        distribution_sha256: None,
        offline: false,
        initial_prefetch_keys: Vec::new(),
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
    let incompatible_schema = Universe::FORMAT_SCHEMA_VERSION + 1;
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
    let error = match resolver.resolve_format(
        Path::new("latex.fmt"),
        EngineMode::Latex,
        &FetchCancellation::new(),
    ) {
        Ok(_) => panic!("incompatible format schema was accepted"),
        Err(error) => error,
    };

    assert_eq!(
        error.to_string(),
        format!(
            "format resource error: format latex uses schema {incompatible_schema}; this runtime requires schema {}",
            Universe::FORMAT_SCHEMA_VERSION
        )
    );
    assert_eq!(
        std::fs::read(cached_object).expect("untouched lookup sentinel"),
        lookup_sentinel,
        "schema preflight must run before the object cache removes corrupt entries"
    );
}

#[test]
fn format_closure_batch_is_installed_for_an_exactly_two_attempt_retry() {
    for (engine, closure_len) in [(EngineMode::Latex, 57), (EngineMode::PdfLatex, 60)] {
        let directory = TempDir::new().expect("distribution tempdir");
        let distribution = directory.path().join("distribution");
        let objects = distribution.join("objects");
        std::fs::create_dir_all(&objects).expect("objects directory");

        let mut initex = tex_state::Universe::with_world(World::memory());
        engine.prepare_fresh(&mut initex);
        let format = initex.dump_format().expect("schema-10 format");
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
            "{{\"schema\":3,\"distribution\":\"closure-attempts\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"],\"formats\":{{\"probe\":{{\"object\":\"sha256-{format_digest}\",\"sha256\":\"{format_digest}\",\"bytes\":{},\"engine\":\"umber\",\"engineVersion\":\"{}\",\"formatSchema\":10,\"sourceDistribution\":\"closure-attempts\",\"sourceManifestSha256\":\"{}\",\"sourceDateEpoch\":0,\"inputClosure\":{{\"schema\":1,\"keys\":[{}]}}}}}}}}\n",
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
                html: false,
                distribution: Some(distribution.to_string_lossy().into_owned()),
                distribution_sha256: None,
                offline: false,
            },
            &cancellation,
            cache.clone(),
        )
        .expect("native session");

        let CompileAttemptResult::NeedResources(first) = session.session.compile_attempt() else {
            panic!("first attempt must miss the closure head");
        };
        assert_eq!(first.required.len(), 1);
        assert_eq!(first.prefetch_hints.len(), closure_len - 1);
        let batch_started = Instant::now();
        let responses = session
            .distribution
            .resolve_batch(&session.local, &first, &cancellation)
            .expect("closure batch");
        let batch_elapsed = batch_started.elapsed();
        assert_eq!(responses.len(), closure_len);
        for (digest, bytes) in &closure_objects {
            assert!(
                cache
                    .load_object(digest, *bytes)
                    .expect("closure cache lookup")
                    .is_some(),
                "the complete closure must be cached by the first host batch"
            );
        }
        session
            .session
            .provide_resources(responses)
            .expect("provide closure head");

        let compile_started = Instant::now();
        session.compile(&cancellation).expect("complete chain");
        let compile_elapsed = compile_started.elapsed();

        assert_eq!(session.session.attempts(), 2);
        eprintln!(
            "format-prefetch-characterization engine={} closure={closure_len} first_batch_us={} attempts={} remaining_compile_us={}",
            engine.name(),
            batch_elapsed.as_micros(),
            session.session.attempts(),
            compile_elapsed.as_micros()
        );
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
    assert_eq!(shard_index("tex:article.cls", 8), 0x45);
    assert_eq!(shard_index("tfm:cmr10.tfm", 8), 0x91);
    assert_eq!(shard_index("tex:plain.tex", 0), 0);
}
