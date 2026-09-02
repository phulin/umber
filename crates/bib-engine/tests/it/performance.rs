use std::time::{Duration, Instant};

use bib_engine::{
    BibliographyAttempt, ClassicBibJob, ClassicBibOptions, ClassicBibSession, FileKind,
    ProjectWorkspace, ResolvedFile, VfsLimits, VirtualPath,
};

fn plain_payload(name: &str) -> Vec<u8> {
    test_support::closed_case::FixtureCase::discover_classic_bibtex(
        "tests/corpus/bibtex/cases/plain",
    )
    .expect("closed plain case")
    .read(name)
    .expect("plain payload")
}

#[test]
#[ignore = "explicit classic BST performance tier"]
// A performance budget is measured against the host clock by definition, so
// this is one of the few places outside `tex-state::World` that may read one.
#[allow(clippy::disallowed_methods)]
fn classic_native_session_performance_budget() {
    const RUNS: usize = 16;
    const SESSION_BUDGET: Duration = Duration::from_secs(5);
    const CACHE_BYTES: usize = 8 * 1024 * 1024;

    let mut provisioner = ProjectWorkspace::new(VfsLimits::default()).expect("VFS limits");
    provisioner
        .register_user(
            VirtualPath::user("plain.aux").expect("AUX path"),
            plain_payload("plain.aux"),
        )
        .expect("AUX");
    let job = ClassicBibJob::new(
        VirtualPath::user("plain.aux").expect("AUX path"),
        ClassicBibOptions::default()
            .with_cache_entries(8)
            .with_cache_bytes(CACHE_BYTES),
    );
    let mut session = ClassicBibSession::new();
    let resources = match session.process(&job, &provisioner.snapshot()) {
        BibliographyAttempt::NeedResources(resources) => resources,
        attempt => panic!("expected plain resources, got {attempt:?}"),
    };
    provisioner.expect(&resources);
    for request in &resources.required {
        let bytes = match request.key().kind() {
            FileKind::ClassicBibData => plain_payload("references.bib"),
            FileKind::BibStyle => plain_payload("plain.bst"),
            kind => panic!("unexpected classic resource kind: {kind:?}"),
        };
        provisioner
            .provision(ResolvedFile {
                request: request.key().clone(),
                virtual_path: format!("/texlive/classic/{}", request.key().name()),
                bytes: bytes.into(),
                expected_digest: None,
            })
            .expect("plain resource");
    }

    assert!(matches!(
        session.process(&job, &provisioner.snapshot()),
        BibliographyAttempt::Finished(_)
    ));
    let started = Instant::now();
    for _ in 0..RUNS {
        assert!(matches!(
            session.process(&job, &provisioner.snapshot()),
            BibliographyAttempt::Finished(_)
        ));
    }
    assert!(
        started.elapsed() <= SESSION_BUDGET,
        "{RUNS} native classic sessions exceeded {SESSION_BUDGET:?}"
    );
    let usage = session.cache_usage();
    assert!(
        usage.compiled_styles <= CACHE_BYTES,
        "style cache: {usage:?}"
    );
    assert!(
        usage.prepared_databases <= CACHE_BYTES,
        "database cache: {usage:?}"
    );
}
