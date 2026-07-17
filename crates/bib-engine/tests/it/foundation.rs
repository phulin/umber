use std::sync::Arc;

use bib_engine::{
    BibAttempt, BibConfigurationBuilder, BibExitStatus, BibJob, BibOptionsBuilder,
    BibResultBuilder, BibSession, BibliographyAttempt, BibliographyBackend, BibliographyDetection,
    BibliographyDetector, BibliographyDocument, BibliographyFailure, BibliographyHistory,
    BibliographyJob, BibliographyMode, BibliographyResult, BibliographyResultError,
    BibliographySession, BibliographyStats, ClassicBibCommand, ClassicBibFailure, ClassicBibJob,
    ClassicBibOptions, CompatibilityVersion, FileKind, FileProvisioner, GeneratedFile,
    OutputFormat, OutputRequest, ProcessedBibliographyBuilder, ResolvedFile, VfsLimits,
    VirtualPath,
};

#[test]
fn public_result_is_detached_and_preserves_output_order() {
    let configuration =
        BibConfigurationBuilder::new(CompatibilityVersion::BIBER_2_22_BETA).freeze();
    let document = Arc::new(ProcessedBibliographyBuilder::new(configuration).freeze());
    let first_path = VirtualPath::user("main.bbl").expect("valid output path");
    let second_path = VirtualPath::user("main.blg").expect("valid output path");
    let mut result = BibResultBuilder::new(document);
    result
        .file(GeneratedFile::new(
            first_path,
            Arc::<[u8]>::from(&b"bbl"[..]),
        ))
        .expect("unique path");
    result
        .file(GeneratedFile::new(
            second_path,
            Arc::<[u8]>::from(&b"log"[..]),
        ))
        .expect("unique path");
    let result = result.freeze();
    assert_eq!(
        result
            .files()
            .map(|file| file.path().as_str())
            .collect::<Vec<_>>(),
        ["/job/main.bbl", "/job/main.blg"]
    );
    assert_eq!(result.stats().generated_bytes(), 6);
}

#[test]
fn public_options_reject_duplicate_output_bindings() {
    let path = VirtualPath::user("main.bbl").expect("valid output path");
    let request = OutputRequest::new(path.clone(), OutputFormat::Bbl);
    let mut options = BibOptionsBuilder::new();
    options
        .output(request.clone())
        .expect("first path is unique");
    assert!(options.output(request).is_err());
}

#[test]
fn wrapped_biblatex_session_preserves_legacy_output_bytes() {
    let control = VirtualPath::user("main.bcf").expect("control path");
    let output = VirtualPath::user("main.bbl").expect("output path");
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("limits");
    provisioner
        .register_user(
            control.clone(),
            br#"<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex"><bcf:section number="0"></bcf:section></bcf:controlfile>"#.to_vec(),
        )
        .expect("control");
    let mut options = BibOptionsBuilder::new();
    options
        .output(OutputRequest::new(output, OutputFormat::Bbl))
        .expect("output");
    let job = BibJob::new(control, options.freeze());
    let legacy = match BibSession::default().process(&job, &provisioner.snapshot()) {
        BibAttempt::Complete(result) => result,
        attempt => panic!("expected legacy completion, got {attempt:?}"),
    };
    let mut session = BibliographySession::biblatex(Default::default()).expect("session");
    let wrapped = match session.process(&BibliographyJob::Biblatex(job), &provisioner.snapshot()) {
        BibliographyAttempt::Finished(result) => result,
        attempt => panic!("expected wrapped completion, got {attempt:?}"),
    };
    assert_eq!(wrapped.backend(), BibliographyBackend::Biblatex);
    assert_eq!(wrapped.history(), BibliographyHistory::Spotless);
    assert_eq!(
        wrapped
            .files()
            .map(GeneratedFile::bytes)
            .collect::<Vec<_>>(),
        legacy.files().map(GeneratedFile::bytes).collect::<Vec<_>>(),
    );
}

#[test]
fn fatal_artifacts_remain_detached_from_publishable_files() {
    let configuration =
        BibConfigurationBuilder::new(CompatibilityVersion::BIBER_2_22_BETA).freeze();
    let document = BibliographyDocument::Biblatex(Arc::new(
        ProcessedBibliographyBuilder::new(configuration).freeze(),
    ));
    let partial = GeneratedFile::new(
        VirtualPath::user("main.bbl").expect("partial path"),
        Arc::<[u8]>::from(&b"partial"[..]),
    );
    let fatal = BibliographyResult::new(
        BibliographyHistory::Fatal,
        document.clone(),
        [],
        [partial.clone()],
        [],
        BibliographyStats::Biblatex(Default::default()),
    )
    .expect("fatal partial result");
    assert!(!fatal.is_publishable());
    assert!(fatal.files().next().is_none());
    assert_eq!(fatal.partial_files().collect::<Vec<_>>(), [&partial]);
    assert_eq!(
        BibliographyResult::new(
            BibliographyHistory::Fatal,
            document.clone(),
            [partial],
            [],
            [],
            BibliographyStats::Biblatex(Default::default()),
        ),
        Err(BibliographyResultError::FatalHistoryHasPublishedFiles)
    );
    assert_eq!(
        BibliographyResult::new(
            BibliographyHistory::Spotless,
            document,
            [],
            [GeneratedFile::new(
                VirtualPath::user("main.blg").expect("partial log path"),
                Arc::<[u8]>::from(&b"partial log"[..]),
            )],
            [],
            BibliographyStats::Biblatex(Default::default()),
        ),
        Err(BibliographyResultError::PartialArtifactsRequireFatalHistory)
    );
}

#[test]
fn classic_control_resolves_aux_bst_and_datasource_resources() {
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("limits");
    let aux = VirtualPath::user("main.aux").expect("AUX path");
    provisioner
        .register_user(
            aux.clone(),
            b"\\citation{one}\n\\@input{chapter.aux}\n".to_vec(),
        )
        .expect("root AUX");
    let job = ClassicBibJob::new(aux, ClassicBibOptions::default());
    let mut session = BibliographySession::classic();
    let included = match session.process(
        &BibliographyJob::Classic(job.clone()),
        &provisioner.snapshot(),
    ) {
        BibliographyAttempt::NeedResources(needs) => needs,
        attempt => panic!("expected included AUX request, got {attempt:?}"),
    };
    assert_eq!(included.required[0].key().kind(), FileKind::BibAux);
    provisioner.expect(&included);
    provisioner
        .provision(ResolvedFile {
            request: included.required[0].key().clone(),
            virtual_path: "/texlive/classic/chapter.aux".into(),
            bytes: b"\\bibstyle{plain}\n\\bibdata{refs}\n".to_vec(),
            expected_digest: None,
        })
        .expect("included AUX");
    let resources = match session.process(
        &BibliographyJob::Classic(job.clone()),
        &provisioner.snapshot(),
    ) {
        BibliographyAttempt::NeedResources(needs) => needs,
        attempt => panic!("expected BST and classic BIB requests, got {attempt:?}"),
    };
    assert_eq!(
        resources
            .required
            .iter()
            .map(|request| request.key().kind())
            .collect::<Vec<_>>(),
        [FileKind::ClassicBibData, FileKind::BibStyle]
    );
    provisioner.expect(&resources);
    for request in &resources.required {
        let bytes = match request.key().kind() {
            FileKind::BibStyle => b"ENTRY { } { } { } READ".to_vec(),
            FileKind::ClassicBibData => b"@book{one}".to_vec(),
            kind => panic!("unexpected classic resource kind: {kind:?}"),
        };
        provisioner
            .provision(ResolvedFile {
                request: request.key().clone(),
                virtual_path: format!("/texlive/classic/{}", request.key().name()),
                bytes,
                expected_digest: None,
            })
            .expect("classic resource");
    }
    let result = match session.process(&BibliographyJob::Classic(job), &provisioner.snapshot()) {
        BibliographyAttempt::Finished(result) => result,
        attempt => panic!("expected classic control completion, got {attempt:?}"),
    };
    assert_eq!(result.backend(), BibliographyBackend::Classic);
    assert!(result.is_publishable());
    assert!(matches!(
        result.document(),
        BibliographyDocument::Classic(_)
    ));
}

#[test]
fn auto_detection_waits_for_included_aux_before_reporting_ambiguity() {
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("limits");
    provisioner
        .register_user(VirtualPath::user("main.bcf").expect("BCF"), b"bcf".to_vec())
        .expect("BCF");
    provisioner
        .register_user(
            VirtualPath::user("main.aux").expect("AUX"),
            b"\\@input{included.aux}\n".to_vec(),
        )
        .expect("AUX");
    let mode = BibliographyMode::Auto {
        job_path: VirtualPath::user("main.tex").expect("job"),
    };
    let mut detector = BibliographyDetector::default();
    let needs = match detector.detect(&mode, &provisioner.snapshot()) {
        BibliographyDetection::NeedResources(needs) => needs,
        result => panic!("expected included AUX request, got {result:?}"),
    };
    provisioner.expect(&needs);
    provisioner
        .provision(ResolvedFile {
            request: needs.required[0].key().clone(),
            virtual_path: "/texlive/classic/included.aux".into(),
            bytes: b"\\bibstyle{plain}\n\\bibdata{refs}\n".to_vec(),
            expected_digest: None,
        })
        .expect("included AUX");
    assert!(matches!(
        detector.detect(&mode, &provisioner.snapshot()),
        BibliographyDetection::Failed(BibliographyFailure::Classic(
            ClassicBibFailure::AmbiguousProtocol
        ))
    ));
}

#[test]
fn classic_resource_retry_rejects_an_unchanged_missing_batch() {
    let job = ClassicBibJob::new(
        VirtualPath::user("missing.aux").expect("AUX"),
        ClassicBibOptions::default(),
    );
    let snapshot = FileProvisioner::new(VfsLimits::default())
        .expect("limits")
        .snapshot();
    let mut session = BibliographySession::classic();
    assert!(matches!(
        session.process(&BibliographyJob::Classic(job.clone()), &snapshot),
        BibliographyAttempt::NeedResources(_)
    ));
    assert!(matches!(
        session.process(&BibliographyJob::Classic(job), &snapshot),
        BibliographyAttempt::Failed(BibliographyFailure::Classic(ClassicBibFailure::NoProgress))
    ));
}

#[test]
fn classic_smoke_executes_through_the_public_session_with_cold_and_cached_bytes() {
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("limits");
    provisioner
        .register_user(
            VirtualPath::user("smoke.aux").expect("fixture path"),
            include_bytes!("../../../../tests/corpus/bibtex/cases/smoke/smoke.aux").to_vec(),
        )
        .expect("fixture input");
    let job = ClassicBibJob::new(
        VirtualPath::user("smoke.aux").expect("AUX path"),
        ClassicBibOptions::default(),
    );
    let mut session = BibliographySession::classic();
    let needs = match session.process(
        &BibliographyJob::Classic(job.clone()),
        &provisioner.snapshot(),
    ) {
        BibliographyAttempt::NeedResources(needs) => needs,
        attempt => panic!("expected classic resource requests, got {attempt:?}"),
    };
    provisioner.expect(&needs);
    for request in &needs.required {
        let bytes = match request.key().kind() {
            FileKind::ClassicBibData => {
                include_bytes!("../../../../tests/corpus/bibtex/cases/smoke/smoke.bib").to_vec()
            }
            FileKind::BibStyle => {
                include_bytes!("../../../../tests/corpus/bibtex/cases/smoke/smoke.bst").to_vec()
            }
            kind => panic!("unexpected resource kind: {kind:?}"),
        };
        provisioner
            .provision(ResolvedFile {
                request: request.key().clone(),
                virtual_path: format!("/texlive/classic/{}", request.key().name()),
                bytes,
                expected_digest: None,
            })
            .expect("fixture resource");
    }
    let first = match session.process(
        &BibliographyJob::Classic(job.clone()),
        &provisioner.snapshot(),
    ) {
        BibliographyAttempt::Finished(result) => result,
        attempt => panic!("expected classic execution, got {attempt:?}"),
    };
    assert_eq!(first.history(), BibliographyHistory::Warning);
    assert_eq!(
        first
            .files()
            .find(|file| file.path().as_str() == "/job/smoke.bbl")
            .expect("BBL")
            .bytes(),
        include_bytes!("../../../../tests/corpus/bibtex/cases/smoke/smoke.bbl"),
    );
    assert_eq!(
        first
            .files()
            .find(|file| file.path().as_str() == "/job/smoke.blg")
            .expect("BLG")
            .bytes(),
        include_bytes!("../../../../tests/corpus/bibtex/cases/smoke/smoke.blg"),
    );
    let second = match session.process(&BibliographyJob::Classic(job), &provisioner.snapshot()) {
        BibliographyAttempt::Finished(result) => result,
        attempt => panic!("expected cached classic execution, got {attempt:?}"),
    };
    assert_eq!(
        first.files().map(GeneratedFile::bytes).collect::<Vec<_>>(),
        second.files().map(GeneratedFile::bytes).collect::<Vec<_>>(),
    );
}

#[test]
fn classic_plain_executes_through_the_public_session() {
    execute_standard_style(
        "plain",
        include_bytes!("../../../../tests/corpus/bibtex/cases/plain/plain.aux"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/plain/references.bib"),
        include_bytes!("../../../../tests/corpus/bibtex/styles/plain.bst"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/plain/plain.bbl"),
        BibliographyHistory::Spotless,
    );
}

#[test]
fn classic_apalike_executes_through_the_public_session() {
    execute_standard_style(
        "apalike",
        include_bytes!("../../../../tests/corpus/bibtex/cases/apalike/apalike.aux"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/apalike/references.bib"),
        include_bytes!("../../../../tests/corpus/bibtex/styles/apalike.bst"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/apalike/apalike.bbl"),
        BibliographyHistory::Spotless,
    );
}

#[test]
fn classic_tex_live_xampl_executes_through_the_public_session() {
    execute_standard_style(
        "exampl",
        include_bytes!("../../../../tests/corpus/bibtex/cases/xampl/exampl.aux"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/xampl/xampl.bib"),
        include_bytes!("../../../../tests/corpus/bibtex/styles/apalike.bst"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/xampl/exampl.bbl"),
        BibliographyHistory::Warning,
    );
}

#[test]
fn classic_real_world_elsarticle_book_has_exact_public_command_parity() {
    execute_real_world_command(
        "elsarticle-book",
        include_bytes!("../../../../tests/corpus/bibtex/cases/elsarticle-book/elsarticle-book.aux"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/elsarticle-book/references.bib"),
        include_bytes!("../../../../tests/corpus/bibtex/styles/elsarticle-num.bst"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/elsarticle-book/elsarticle-book.bbl"),
        include_bytes!("../../../../tests/corpus/bibtex/cases/elsarticle-book/elsarticle-book.blg"),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-book/elsarticle-book.terminal"
        ),
    );
}

#[test]
fn classic_real_world_elsarticle_article_has_exact_public_command_parity() {
    execute_real_world_command(
        "elsarticle-article",
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-article/elsarticle-article.aux"
        ),
        include_bytes!("../../../../tests/corpus/bibtex/cases/elsarticle-article/references.bib"),
        include_bytes!("../../../../tests/corpus/bibtex/styles/elsarticle-num.bst"),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-article/elsarticle-article.bbl"
        ),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-article/elsarticle-article.blg"
        ),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-article/elsarticle-article.terminal"
        ),
    );
}

#[test]
fn classic_real_world_elsarticle_names_has_exact_public_command_parity() {
    execute_real_world_command(
        "elsarticle-names",
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-names/elsarticle-names.aux"
        ),
        include_bytes!("../../../../tests/corpus/bibtex/cases/elsarticle-names/references.bib"),
        include_bytes!("../../../../tests/corpus/bibtex/styles/elsarticle-num.bst"),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-names/elsarticle-names.bbl"
        ),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-names/elsarticle-names.blg"
        ),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-names/elsarticle-names.terminal"
        ),
    );
}

#[test]
fn classic_real_world_elsarticle_month_has_exact_public_command_parity() {
    execute_real_world_command(
        "elsarticle-month",
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-month/elsarticle-month.aux"
        ),
        include_bytes!("../../../../tests/corpus/bibtex/cases/elsarticle-month/references.bib"),
        include_bytes!("../../../../tests/corpus/bibtex/styles/elsarticle-num.bst"),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-month/elsarticle-month.bbl"
        ),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-month/elsarticle-month.blg"
        ),
        include_bytes!(
            "../../../../tests/corpus/bibtex/cases/elsarticle-month/elsarticle-month.terminal"
        ),
    );
}

fn execute_standard_style(
    name: &str,
    aux_bytes: &[u8],
    database_bytes: &[u8],
    style_bytes: &[u8],
    expected_bbl: &[u8],
    expected_history: BibliographyHistory,
) {
    let aux = VirtualPath::user(&format!("{name}.aux")).expect("fixture path");
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("VFS");
    provisioner
        .register_user(aux.clone(), aux_bytes.to_vec())
        .expect("fixture AUX");
    let job = ClassicBibJob::new(aux, ClassicBibOptions::default());
    let mut session = BibliographySession::classic();
    let needs = match session.process(
        &BibliographyJob::Classic(job.clone()),
        &provisioner.snapshot(),
    ) {
        BibliographyAttempt::NeedResources(needs) => needs,
        attempt => panic!("expected classic resource requests, got {attempt:?}"),
    };
    provisioner.expect(&needs);
    for request in &needs.required {
        let bytes = match request.key().kind() {
            FileKind::ClassicBibData => database_bytes.to_vec(),
            FileKind::BibStyle => style_bytes.to_vec(),
            kind => panic!("unexpected classic resource kind: {kind:?}"),
        };
        provisioner
            .provision(ResolvedFile {
                request: request.key().clone(),
                virtual_path: format!("/texlive/classic/{}", request.key().name()),
                bytes,
                expected_digest: None,
            })
            .expect("fixture resource");
    }
    let result = match session.process(&BibliographyJob::Classic(job), &provisioner.snapshot()) {
        BibliographyAttempt::Finished(result) => result,
        attempt => panic!("expected classic execution, got {attempt:?}"),
    };
    assert_eq!(result.history(), expected_history, "{result:?}");
    assert_eq!(
        result
            .files()
            .find(|file| file.path().as_str() == format!("/job/{name}.bbl"))
            .expect("BBL")
            .bytes(),
        expected_bbl,
    );
}

fn execute_real_world_command(
    name: &str,
    aux_bytes: &[u8],
    database_bytes: &[u8],
    style_bytes: &[u8],
    expected_bbl: &[u8],
    expected_blg: &[u8],
    expected_terminal: &[u8],
) {
    let command = ClassicBibCommand::parse([name]).expect("fixture command");
    let mut files = FileProvisioner::new(VfsLimits::default()).expect("VFS");
    files
        .register_user(command.aux_path().clone(), aux_bytes.to_vec())
        .expect("fixture AUX");
    let output = command.execute_provisioned(&mut files, |request| {
        let bytes = match request.key().kind() {
            FileKind::ClassicBibData => database_bytes.to_vec(),
            FileKind::BibStyle => style_bytes.to_vec(),
            kind => panic!("unexpected fixture resource kind: {kind:?}"),
        };
        Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: format!("/texlive/classic/{}", request.key().name()),
            bytes,
            expected_digest: None,
        })
    });
    assert_eq!(output.status(), BibExitStatus::Success, "{output:?}");
    assert_eq!(output.terminal(), expected_terminal);
    let result = output.result().expect("publishable classic result");
    assert_eq!(
        result.history(),
        BibliographyHistory::Spotless,
        "{result:?}"
    );
    for (extension, expected) in [("bbl", expected_bbl), ("blg", expected_blg)] {
        assert_eq!(
            result
                .files()
                .find(|file| file.path().as_str() == format!("/job/{name}.{extension}"))
                .expect("expected classic artifact")
                .bytes(),
            expected,
            "{extension} parity for {name}",
        );
    }
}
