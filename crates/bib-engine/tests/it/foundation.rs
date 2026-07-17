use std::sync::Arc;

use bib_engine::{
    BibAttempt, BibConfigurationBuilder, BibJob, BibOptionsBuilder, BibResultBuilder, BibSession,
    BibliographyAttempt, BibliographyBackend, BibliographyDocument, BibliographyHistory,
    BibliographyJob, BibliographyResult, BibliographyResultError, BibliographySession,
    BibliographyStats, ClassicBibJob, ClassicBibOptions, CompatibilityVersion, FileProvisioner,
    GeneratedFile, OutputFormat, OutputRequest, ProcessedBibliographyBuilder, VfsLimits,
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
fn classic_noop_uses_the_same_typed_result_boundary() {
    let job = ClassicBibJob::new(
        VirtualPath::user("main.aux").expect("AUX path"),
        ClassicBibOptions::default(),
    );
    let snapshot = FileProvisioner::new(VfsLimits::default())
        .expect("limits")
        .snapshot();
    let mut session = BibliographySession::classic();
    let result = match session.process(&BibliographyJob::Classic(job), &snapshot) {
        BibliographyAttempt::Finished(result) => result,
        attempt => panic!("expected no-op completion, got {attempt:?}"),
    };
    assert_eq!(result.backend(), BibliographyBackend::Classic);
    assert!(result.is_publishable());
    assert!(matches!(
        result.document(),
        BibliographyDocument::Classic(_)
    ));
}
