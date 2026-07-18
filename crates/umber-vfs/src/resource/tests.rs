use super::*;
use proptest::prelude::*;

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
fn user_registration_is_atomic_and_snapshots_retain_exact_generations() {
    let limits = VfsLimits {
        user_files: 1,
        user_bytes: 4,
        one_file_bytes: 4,
        ..VfsLimits::default()
    };
    let mut registry = FileProvisioner::new(limits).expect("registry");
    let main = VirtualPath::user("main.tex").expect("main path");
    assert_eq!(
        registry
            .register_user(main.clone(), b"old".to_vec())
            .expect("initial user file"),
        ProvisionOutcome::Inserted
    );
    let retained = registry.snapshot();

    registry
        .register_user(main.clone(), b"new!".to_vec())
        .expect("replacement at limit");
    assert_eq!(registry.user_file_count(), 1);
    assert_eq!(registry.user_bytes(), 4);
    assert_eq!(
        retained
            .get(&main)
            .expect("retained read")
            .expect("retained main file")
            .bytes(),
        b"old"
    );
    assert_eq!(
        registry
            .snapshot()
            .get(&main)
            .expect("current read")
            .expect("current main file")
            .bytes(),
        b"new!"
    );

    let other = VirtualPath::user("other.tex").expect("other path");
    assert!(matches!(
        registry.register_user(other.clone(), Vec::new()),
        Err(UserRegistrationError::Limit(VfsLimitError::LimitExceeded {
            kind: VfsLimitKind::UserFiles,
            ..
        }))
    ));
    assert!(!registry.snapshot().contains(&other).expect("current read"));
}

#[test]
fn resolved_registration_and_clear_are_reflected_in_vfs_snapshots() {
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    let user = VirtualPath::user("main.tex").expect("user path");
    registry
        .register_user(user.clone(), b"main".to_vec())
        .expect("user file");
    registry
        .preload(response(
            FileKind::TexInput,
            "plain.tex",
            "/texlive/plain.tex",
            b"plain",
        ))
        .expect("resolved file");
    let resolved = VirtualPath::distribution("/texlive/plain.tex").expect("resolved path");
    let retained = registry.snapshot();
    assert!(retained.contains(&user).expect("user read"));
    assert!(retained.contains(&resolved).expect("resolved read"));

    registry.clear();
    let current = registry.snapshot();
    assert!(current.contains(&user).expect("user read"));
    assert!(!current.contains(&resolved).expect("resolved read"));
    assert!(
        retained
            .contains(&resolved)
            .expect("retained resolved read")
    );
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
        FileKind::Image,
        FileKind::BibAux,
        FileKind::ClassicBibData,
        FileKind::BibStyle,
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

proptest! {
    #[test]
    fn arbitrary_response_permutations_and_chunking_are_equivalent(
        entries in prop::collection::btree_map(
            "[a-z]{1,8}\\.tex",
            (any::<u32>(), prop::collection::vec(any::<u8>(), 0..64)),
            1..32,
        ),
        chunk_size in 1usize..8,
    ) {
        let requests = FileRequestBatch::new(
            entries
                .keys()
                .map(|name| request(FileKind::TexInput, name)),
            [],
        );
        let responses = entries
            .iter()
            .map(|(name, (order, bytes))| {
                (
                    *order,
                    response(
                        FileKind::TexInput,
                        name,
                        &format!("/texlive/tex/{name}"),
                        bytes,
                    ),
                )
            })
            .collect::<Vec<_>>();

        let mut together = FileProvisioner::new(VfsLimits::default()).expect("registry");
        together.expect(&requests);
        together
            .provision_batch(responses.iter().map(|(_, response)| response.clone()))
            .expect("canonical batch");

        let mut permuted = responses;
        permuted.sort_by_key(|(order, response)| (*order, response.request.clone()));
        let chunk_count = permuted.len().div_ceil(chunk_size);
        let mut chunked = FileProvisioner::new(VfsLimits::default()).expect("registry");
        chunked.expect(&requests);
        for (index, chunk) in permuted.chunks(chunk_size).enumerate() {
            chunked
                .provision_batch(chunk.iter().map(|(_, response)| response.clone()))
                .expect("permuted partial batch");
            if index + 1 < chunk_count {
                chunked.retry().expect("partial batch made progress");
            }
        }

        prop_assert_eq!(state(&together), state(&chunked));
    }
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
fn prefetch_hints_accept_positive_files_but_reject_unavailable_bindings() {
    let required = key(FileKind::TexInput, "required.tex");
    let hint = key(FileKind::TexInput, "hint.tex");
    let batch = FileRequestBatch::new(
        [FileRequest::new(required, "required.tex")],
        [FileRequest::new(hint.clone(), "hint.tex")],
    );
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    registry.expect(&batch);

    assert!(matches!(
        registry.provision_unavailable(hint.clone()),
        Err(ProvisionError::UnexpectedRequest(request)) if request == hint
    ));
    assert!(!registry.is_unavailable(&hint));
    registry
        .provision(response(
            FileKind::TexInput,
            "hint.tex",
            "/texlive/hint.tex",
            b"hint",
        ))
        .expect("positive hint response");
}

#[test]
fn unavailable_bindings_are_progress_idempotent_and_immutable() {
    let required = key(FileKind::TexInput, "optional.cfg");
    let batch = FileRequestBatch::new([FileRequest::new(required.clone(), "optional.cfg")], []);
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    registry.expect(&batch);

    assert_eq!(
        registry
            .provision_unavailable(required.clone())
            .expect("negative response"),
        ProvisionOutcome::Inserted
    );
    assert!(registry.is_unavailable(&required));
    assert_eq!(registry.retry(), Ok(()));
    assert_eq!(
        registry
            .provision_unavailable(required.clone())
            .expect("duplicate negative response"),
        ProvisionOutcome::AlreadyPresent
    );
    assert!(matches!(
        registry.provision(response(
            FileKind::TexInput,
            "optional.cfg",
            "/texlive/optional.cfg",
            b"later",
        )),
        Err(ProvisionError::AvailabilityConflict { .. })
    ));
    assert_eq!(registry.resolved_bytes(), 0);
}

#[test]
fn blocking_probe_authorizes_positive_or_negative_progress() {
    let probe = key(FileKind::TexInput, "optional.cfg");
    let batch =
        FileRequestBatch::with_probes([], [FileRequest::new(probe.clone(), "optional.cfg")], []);
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    registry.expect(&batch);

    assert_eq!(
        registry
            .provision_unavailable(probe.clone())
            .expect("authoritative negative probe response"),
        ProvisionOutcome::Inserted
    );
    assert!(registry.is_unavailable(&probe));
    assert_eq!(registry.retry(), Ok(()));
}

#[test]
fn available_binding_rejects_later_unavailable_answer() {
    let required = key(FileKind::TexInput, "present.tex");
    let batch = FileRequestBatch::new([FileRequest::new(required.clone(), "present.tex")], []);
    let mut registry = FileProvisioner::new(VfsLimits::default()).expect("registry");
    registry.expect(&batch);
    registry
        .provision(response(
            FileKind::TexInput,
            "present.tex",
            "/texlive/present.tex",
            b"present",
        ))
        .expect("positive response");
    assert!(matches!(
        registry.provision_unavailable(required),
        Err(ProvisionError::AvailabilityConflict { .. })
    ));
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
