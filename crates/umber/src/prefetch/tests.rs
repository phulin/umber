use super::*;

fn planner() -> PrefetchPlanner {
    PrefetchPlanner::new(
        PrefetchIdentity::new("pdftex", "latex", "dvi", "root", PREFETCH_POLICY_VERSION)
            .expect("identity"),
        PrefetchBudget::default(),
    )
}

#[test]
fn startup_policy_combines_prior_manifest_and_literal_hints() {
    let identity = PrefetchIdentity::new("pdftex", "latex", "dvi", "root", PREFETCH_POLICY_VERSION)
        .expect("identity");
    let mut prior = LookupManifest::new(identity.clone());
    let resolved = ResolvedIdentity::new(
        "tex:prior.sty",
        Some("/texlive/prior.sty".to_owned()),
        "object",
        "0123456789abcdef",
        10,
    )
    .expect("resolved");
    prior
        .record(
            LookupRecord::new(
                "prior.sty",
                "tex:prior.sty",
                "tex",
                "distribution",
                LookupRole::Required,
                LookupOutcome::Resolved(resolved),
            )
            .expect("record"),
        )
        .expect("manifest");
    let mut planner = PrefetchPlanner::with_prior(identity, PrefetchBudget::default(), Some(prior));
    let hints = planner.startup_hints("\\usepackage{newpkg}");
    let names = hints
        .iter()
        .map(|request| match request {
            ResourceRequest::File(request) => request.key().name(),
            _ => "other",
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["prior.sty", "newpkg.sty"]);
    assert_eq!(planner.metrics().startup_candidates, 1);
}

#[test]
fn source_edits_do_not_change_identity_or_publish_predictions() {
    let mut planner = planner();
    let first = planner.startup_hints("\\input{first}");
    let second = planner.startup_hints("\\input{second}");
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert!(matches!(
        &first[0],
        ResourceRequest::File(request) if request.key().name() == "first.tex"
    ));
    assert!(matches!(
        &second[0],
        ResourceRequest::File(request) if request.key().name() == "second.tex"
    ));
    assert!(planner.into_manifest().records().is_empty());
}

#[test]
fn planner_scans_only_after_admission_and_keeps_spelling() {
    let mut planner = planner();
    let root = FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, "root.sty").expect("key"),
        "./root.sty",
    );
    planner.enqueue_escalation([root.clone()]);
    let startup = planner.drain_followups();
    assert_eq!(startup.len(), 1);
    assert!(planner.drain_followups().is_empty());
    planner.admit_file(&root, br#"\input{child.tex}"#);
    let closure = planner.drain_followups();
    assert_eq!(closure.len(), 1);
    let ResourceRequest::File(child) = &closure[0] else {
        panic!("expected file closure request");
    };
    assert_eq!(child.original_name(), "child.tex");
}

#[test]
fn planner_admission_queues_authenticated_dependency_hints() {
    let mut planner = planner();
    let root = FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, "root.sty").expect("key"),
        "root.sty",
    );
    let dependency = FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, "companion.sty").expect("key"),
        "./companion.sty",
    );
    planner.admit_file_with_metadata(
        &root,
        "/texlive/root.sty",
        br#"% no literal child"#,
        [dependency],
    );
    let closure = planner.drain_followups();
    assert!(closure.iter().any(|request| {
        matches!(request, ResourceRequest::File(file) if file.original_name() == "./companion.sty")
    }));
}

#[test]
fn planner_drains_admitted_literals_before_metadata_peers() {
    let mut planner = planner();
    let root = FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, "root.sty").expect("key"),
        "root.sty",
    );
    let dependency = FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, "companion.sty").expect("key"),
        "companion.sty",
    );
    planner.admit_file_with_metadata(
        &root,
        "/texlive/root.sty",
        br#"\input{literal-child.tex}"#,
        [dependency],
    );
    let first = planner.drain_followups();
    assert_eq!(
        first
            .iter()
            .map(|request| match request {
                ResourceRequest::File(file) => file.key().name(),
                ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => "font",
            })
            .collect::<Vec<_>>(),
        ["literal-child.tex"]
    );
    let second = planner.drain_followups();
    assert_eq!(
        second
            .iter()
            .map(|request| match request {
                ResourceRequest::File(file) => file.key().name(),
                ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => "font",
            })
            .collect::<Vec<_>>(),
        ["companion.sty"]
    );
}

#[test]
fn planner_resets_replay_and_admission_state_for_new_context() {
    let mut planner = planner();
    let root = FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, "root.sty").expect("key"),
        "root.sty",
    );
    planner.enqueue_escalation([root.clone()]);
    assert_eq!(planner.drain_followups().len(), 1);
    planner.admit_file(&root, br#"\input{old-child.tex}"#);
    assert_eq!(planner.drain_followups().len(), 1);
    planner.reset_for_context("\\input{new-root.sty}");
    let startup = planner.drain_followups();
    assert!(startup.iter().any(|request| {
        matches!(request, ResourceRequest::File(file) if file.key().name() == "new-root.sty")
    }));
    planner.admit_file(&root, br#"\input{new-child.tex}"#);
    let closure = planner.drain_followups();
    assert!(closure.iter().any(|request| {
        matches!(request, ResourceRequest::File(file) if file.key().name() == "new-child.tex")
    }));
}

#[test]
fn planner_preserves_shared_transport_names_for_distinct_file_kinds() {
    let requests = [
        FileRequest::new(
            FileRequestKey::new(FileKind::VirtualFont, "shared").expect("vf key"),
            "shared",
        ),
        FileRequest::new(
            FileRequestKey::new(FileKind::PdfFontProgram, "shared").expect("pdf key"),
            "shared",
        ),
        FileRequest::new(
            FileRequestKey::new(FileKind::GenericAsset, "shared").expect("asset key"),
            "shared",
        ),
    ];
    let mut planner = planner();
    planner.enqueue_escalation(requests);
    let mut drained = planner.drain_followups();
    drained.sort_by_key(|request| match request {
        ResourceRequest::File(request) => request.key().kind(),
        ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => FileKind::TexInput,
    });
    assert_eq!(
        drained
            .iter()
            .map(|request| match request {
                ResourceRequest::File(request) => (request.key().kind(), request.key().name()),
                ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => {
                    (FileKind::TexInput, "")
                }
            })
            .collect::<Vec<_>>(),
        vec![
            (FileKind::GenericAsset, "shared"),
            (FileKind::VirtualFont, "shared"),
            (FileKind::PdfFontProgram, "shared"),
        ]
    );
}

#[test]
fn opt_in_diagnostics_are_bounded_at_planner_seams() {
    let mut planner = planner();
    assert!(!planner.diagnostics_enabled());
    planner.enable_diagnostics();
    let startup = planner.startup_hints("\\usepackage{telemetry-test}");
    let request = startup
        .iter()
        .find_map(|request| match request {
            ResourceRequest::File(request) => Some(request.clone()),
            ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("literal package hint");
    planner.note_catalog_result(&request, true);
    planner.admit_file(&request, b"% runtime text");
    planner.admit_file(&request, b"% already resident");

    for index in 0..(MAX_PREFETCH_DIAGNOSTIC_DECISIONS + 4) {
        let name = format!("diagnostic-{index}.tex");
        let request = FileRequest::new(
            FileRequestKey::new(FileKind::TexInput, &name).expect("diagnostic key"),
            "diagnostic",
        );
        planner.note_catalog_result(&request, false);
    }
    let (emitted, dropped) = planner.diagnostic_counts().expect("diagnostics enabled");
    assert_eq!(emitted, MAX_PREFETCH_DIAGNOSTIC_DECISIONS as u64);
    assert!(dropped > 0);
}
