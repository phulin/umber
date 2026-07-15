use super::*;

fn key(kind: FileKind, name: &str) -> FileRequestKey {
    FileRequestKey::new(kind, name).expect("request key")
}

fn request(kind: FileKind, name: &str) -> FileRequest {
    FileRequest::new(key(kind, name), name)
}

fn response(kind: FileKind, name: &str, path: &str, bytes: &[u8]) -> ResolvedFile {
    ResolvedFile {
        request: key(kind, name),
        virtual_path: path.to_owned(),
        bytes: bytes.to_vec(),
        expected_digest: None,
    }
}

fn state(registry: &FileProvisioner) -> Vec<(FileRequestKey, String, Vec<u8>)> {
    registry
        .files()
        .map(|(key, file)| (key.clone(), file.path().to_string(), file.bytes().to_vec()))
        .collect()
}

#[test]
fn keys_include_domain_and_reject_cross_domain_kinds() {
    let tex = key(FileKind::TexInput, "shared.dat");
    let bib = FileRequestKey::for_domain(
        ResourceDomain::Bibliography,
        FileKind::BibData,
        "shared.dat",
    )
    .expect("bib key");
    assert_ne!(tex, bib);
    assert_eq!(tex.domain(), ResourceDomain::Tex);
    assert_eq!(bib.domain(), ResourceDomain::Bibliography);
    assert_eq!(tex.name(), "shared.dat");
    assert!(matches!(
        FileRequestKey::for_domain(ResourceDomain::Tex, FileKind::BibData, "refs.bib"),
        Err(RequestKeyError::KindMismatch { .. })
    ));
}

#[test]
fn native_wire_names_round_trip_every_wasm_value() {
    for domain in [
        ResourceDomain::Tex,
        ResourceDomain::Bibliography,
        ResourceDomain::Generic,
    ] {
        assert_eq!(
            ResourceDomain::from_wire_name(domain.wire_name()),
            Some(domain)
        );
    }
    for kind in [
        FileKind::TexInput,
        FileKind::Tfm,
        FileKind::FormatImage,
        FileKind::BibControl,
        FileKind::BibData,
        FileKind::BibConfiguration,
        FileKind::XmlSchema,
        FileKind::GenericAsset,
    ] {
        assert_eq!(FileKind::from_wire_name(kind.wire_name()), Some(kind));
    }
}

#[test]
fn batches_are_sorted_deduplicated_and_required_wins_over_hint() {
    let batch = FileRequestBatch::new(
        [
            FileRequest::new(key(FileKind::Tfm, "z.tfm"), "z"),
            FileRequest::new(key(FileKind::TexInput, "a.tex"), "./a"),
            FileRequest::new(key(FileKind::TexInput, "a.tex"), "a"),
        ],
        [
            request(FileKind::TexInput, "a.tex"),
            request(FileKind::BibData, "refs.bib"),
        ],
    );
    assert_eq!(
        batch
            .required
            .iter()
            .map(|request| (request.key().kind(), request.original_name()))
            .collect::<Vec<_>>(),
        vec![(FileKind::TexInput, "./a"), (FileKind::Tfm, "z")]
    );
    assert_eq!(
        batch.prefetch_hints,
        vec![request(FileKind::BibData, "refs.bib")]
    );
}

#[test]
fn partial_permuted_and_chunked_responses_are_equivalent() {
    let requests = FileRequestBatch::new(
        [
            request(FileKind::TexInput, "one.tex"),
            request(FileKind::BibData, "refs.bib"),
        ],
        [],
    );
    let one = response(
        FileKind::TexInput,
        "one.tex",
        "/texlive/tex/one.tex",
        b"one",
    );
    let refs = response(
        FileKind::BibData,
        "refs.bib",
        "/texlive/bib/refs.bib",
        b"refs",
    );

    let mut together = FileProvisioner::new(VfsLimits::default()).expect("registry");
    together.expect(&requests);
    together
        .provision_batch([refs.clone(), one.clone()])
        .expect("permuted batch");

    let mut chunked = FileProvisioner::new(VfsLimits::default()).expect("registry");
    chunked.expect(&requests);
    chunked.provision(one).expect("first chunk");
    chunked.retry().expect("required progress");
    chunked.provision(refs).expect("second chunk");

    assert_eq!(state(&together), state(&chunked));
}

#[test]
fn exact_duplicate_is_idempotent_but_request_and_path_conflicts_are_typed() {
    let batch = FileRequestBatch::new([request(FileKind::TexInput, "one.tex")], []);
    let exact = response(
        FileKind::TexInput,
        "one.tex",
        "/texlive/tex/one.tex",
        b"one",
    );
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    registry.expect(&batch);
    assert_eq!(
        registry.provision(exact.clone()),
        Ok(ProvisionOutcome::Inserted)
    );
    assert_eq!(
        registry.provision(exact),
        Ok(ProvisionOutcome::AlreadyPresent)
    );
    assert!(matches!(
        registry.preload(response(
            FileKind::TexInput,
            "one.tex",
            "/texlive/tex/other.tex",
            b"one"
        )),
        Err(ProvisionError::Conflict { .. })
    ));
    assert!(matches!(
        registry.preload(response(
            FileKind::TexInput,
            "alias.tex",
            "/texlive/tex/one.tex",
            b"different"
        )),
        Err(ProvisionError::PathConflict { .. })
    ));
}

#[test]
fn digest_path_kind_unexpected_and_limit_failures_are_typed_and_atomic() {
    let expected = request(FileKind::TexInput, "one.tex");
    let batch = FileRequestBatch::new([expected.clone()], []);
    let limits = VfsLimits {
        resolved_files: 1,
        one_file_bytes: 3,
        resolved_bytes: 3,
        ..VfsLimits::default()
    };
    let mut registry = FileProvisioner::new(limits).expect("registry");
    registry.expect(&batch);

    let mut corrupt = response(
        FileKind::TexInput,
        "one.tex",
        "/texlive/tex/one.tex",
        b"one",
    );
    corrupt.expected_digest = Some(FileContentId::for_bytes(b"different"));
    assert!(matches!(
        registry.provision(corrupt),
        Err(ProvisionError::DigestMismatch { .. })
    ));
    assert!(matches!(
        registry.provision(response(
            FileKind::TexInput,
            "one.tex",
            "/job/one.tex",
            b"one"
        )),
        Err(ProvisionError::InvalidPath { .. })
    ));
    assert!(matches!(
        registry.provision(response(
            FileKind::Tfm,
            "one.tex",
            "/texlive/one.tfm",
            b"one"
        )),
        Err(ProvisionError::KindMismatch { .. })
    ));
    assert!(matches!(
        registry.provision(response(
            FileKind::TexInput,
            "other.tex",
            "/texlive/other.tex",
            b"one"
        )),
        Err(ProvisionError::UnexpectedRequest(_))
    ));
    assert!(matches!(
        registry.provision(response(
            FileKind::TexInput,
            "one.tex",
            "/texlive/one.tex",
            b"four"
        )),
        Err(ProvisionError::Limit(VfsLimitError::LimitExceeded { .. }))
    ));
    assert!(registry.is_empty());
}

#[test]
fn retry_requires_progress_on_required_requests_not_hints() {
    let batch = FileRequestBatch::new(
        [request(FileKind::TexInput, "required.tex")],
        [request(FileKind::BibData, "hint.bib")],
    );
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    registry.expect(&batch);
    assert_eq!(registry.retry(), Err(RetryError::NoProgress));
    registry
        .provision(response(
            FileKind::BibData,
            "hint.bib",
            "/texlive/bib/hint.bib",
            b"hint",
        ))
        .expect("hint");
    assert_eq!(registry.retry(), Err(RetryError::NoProgress));
    registry
        .provision(response(
            FileKind::TexInput,
            "required.tex",
            "/texlive/tex/required.tex",
            b"required",
        ))
        .expect("required");
    assert_eq!(registry.retry(), Ok(()));

    let two_required = FileRequestBatch::new(
        [
            request(FileKind::TexInput, "one.tex"),
            request(FileKind::TexInput, "two.tex"),
        ],
        [],
    );
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    registry.expect(&two_required);
    registry
        .provision(response(
            FileKind::TexInput,
            "one.tex",
            "/texlive/one.tex",
            b"one",
        ))
        .expect("one required");
    assert_eq!(registry.retry(), Ok(()));
    assert_eq!(registry.retry(), Err(RetryError::NoProgress));
}

#[test]
fn an_invalid_batch_publishes_nothing() {
    let batch = FileRequestBatch::new(
        [
            request(FileKind::TexInput, "one.tex"),
            request(FileKind::TexInput, "two.tex"),
        ],
        [],
    );
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    registry.expect(&batch);
    let result = registry.provision_batch([
        response(FileKind::TexInput, "one.tex", "/texlive/one.tex", b"one"),
        response(FileKind::TexInput, "two.tex", "/job/two.tex", b"two"),
    ]);
    assert!(matches!(result, Err(ProvisionError::InvalidPath { .. })));
    assert!(registry.is_empty());
    assert_eq!(registry.resolved_bytes(), 0);
}
