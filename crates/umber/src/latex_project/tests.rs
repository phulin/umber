use bib_engine::{BibOptionsBuilder, OutputFormat, OutputRequest};

use super::*;
use crate::EngineMode;

fn options() -> LatexProjectOptions {
    let control = VirtualPath::user("/job/main.bcf").expect("control path");
    let mut bib = BibOptionsBuilder::new();
    bib.output(OutputRequest::new(
        VirtualPath::user("/job/main.bbl").expect("output path"),
        OutputFormat::Bbl,
    ))
    .expect("output");
    LatexProjectOptions {
        tex: SessionOptions {
            engine: EngineMode::Tex82,
            ..SessionOptions::default()
        },
        bibliography: BibJob::new(control, bib.freeze()),
        bib_session: BibSessionOptions::default(),
        limits: LatexProjectLimits::default(),
    }
}

#[test]
fn converges_and_atomically_publishes_tex_and_bibliography_files() {
    let source = br#"\immediate\openout1=main.bcf
\immediate\write1{<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex"><bcf:section number="0"></bcf:section></bcf:controlfile>}
\immediate\closeout1
\shipout\hbox{X}\end
"#;
    let mut session = LatexProjectSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", source.to_vec())
        .expect("source");
    let output = match session.compile_attempt() {
        LatexProjectAttempt::Complete(output) => output,
        attempt => panic!("expected complete project, got {attempt:?}"),
    };
    assert!(output.passes >= 2);
    assert!(!output.tex.dvi.is_empty());
    assert!(
        output
            .generated_files
            .iter()
            .any(|file| file.path == std::path::Path::new("/job/main.bcf"))
    );
    assert!(
        output
            .generated_files
            .iter()
            .any(|file| file.path == std::path::Path::new("/job/main.bbl"))
    );
    assert_eq!(
        session.compile_attempt(),
        LatexProjectAttempt::Complete(output.clone())
    );

    let mut cold = LatexProjectSession::new(options()).expect("cold session");
    cold.add_user_file("/job/main.tex", source.to_vec())
        .expect("cold source");
    assert_eq!(
        cold.compile_attempt(),
        LatexProjectAttempt::Complete(output),
        "cold and retained project runs must agree byte-for-byte"
    );
}

#[test]
fn failed_patch_preserves_the_accepted_project() {
    let source =
        b"\\immediate\\openout1=state.aux \\immediate\\write1{old} \\immediate\\closeout1 \\end\n"
            .to_vec();
    let mut project_options = options();
    project_options.tex.limits.output_bytes = 512;
    let mut session = LatexProjectSession::new(project_options).expect("session");
    session
        .add_user_file("/job/main.tex", source.clone())
        .expect("source");
    let accepted = match session.compile_attempt() {
        LatexProjectAttempt::Complete(output) => output,
        attempt => panic!("expected complete project, got {attempt:?}"),
    };
    let start = source
        .windows(3)
        .position(|window| window == b"old")
        .expect("payload");
    session
        .apply_patch(SourcePatch {
            base_revision: accepted.revision,
            next_revision: tex_incr::RevisionId::new(2),
            expected_hash: accepted.content_hash,
            range: start..start + 3,
            replacement: "x".repeat(1_024),
        })
        .expect("patch");
    assert!(matches!(
        session.compile_attempt(),
        LatexProjectAttempt::Error(_)
    ));
    assert_eq!(session.accepted_output(), Some(&accepted));
    assert_eq!(session.revision(), Some(tex_incr::RevisionId::new(1)));
}

#[test]
fn repeated_resource_need_is_a_typed_no_progress_failure() {
    let mut session = LatexProjectSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", b"\\input remote \\end\n".to_vec())
        .expect("source");
    assert!(matches!(
        session.compile_attempt(),
        LatexProjectAttempt::NeedResources(_)
    ));
    assert_eq!(
        session.compile_attempt(),
        LatexProjectAttempt::Error(LatexProjectError::Compile(CompileError::NoProgress))
    );
    assert!(session.accepted_output().is_none());
}

fn only_file_request(needs: &NeedResources) -> umber_vfs::FileRequestKey {
    let [ResourceRequest::File(request)] = needs.required.as_slice() else {
        panic!("expected one required file, got {:?}", needs.required);
    };
    request.key().clone()
}

fn resolved(request: umber_vfs::FileRequestKey, path: &str, bytes: &[u8]) -> ResourceResponse {
    ResourceResponse::File(ResolvedFile {
        request,
        virtual_path: path.into(),
        bytes: bytes.to_vec(),
        expected_digest: None,
    })
}

#[test]
fn legacy_project_retains_one_tex_pass_across_positive_and_negative_resources() {
    let source = b"\\input first \\openin0=absent \\ifeof0\\closein0\\fi \\end\n";
    let mut session = LatexProjectSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", source.to_vec())
        .expect("source");

    let LatexProjectAttempt::NeedResources(first) = session.compile_attempt() else {
        panic!("first resource suspension");
    };
    assert_eq!(
        session
            .candidate
            .as_ref()
            .expect("retained project candidate")
            .tex
            .as_ref()
            .expect("retained TeX pass")
            .attempts(),
        1
    );
    session
        .provide_resources(vec![resolved(
            only_file_request(&first),
            "/texlive/first.tex",
            b"% first\n",
        )])
        .expect("first resource");

    let probe = match session.compile_attempt() {
        LatexProjectAttempt::NeedResources(needs) => needs,
        other => panic!("expected probe suspension, got {other:?}"),
    };
    assert_eq!(
        session
            .candidate
            .as_ref()
            .expect("retained project candidate")
            .tex
            .as_ref()
            .expect("retained TeX pass")
            .attempts(),
        2
    );
    let [ResourceRequest::File(probe)] = probe.probes.as_slice() else {
        panic!("expected one probe");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(probe.key().clone())])
        .expect("negative probe");

    assert!(matches!(
        session.compile_attempt(),
        LatexProjectAttempt::Complete(_)
    ));
}

fn classic_options(mode: bib_engine::BibliographyMode) -> LatexProjectOptionsV2 {
    LatexProjectOptionsV2 {
        tex: SessionOptions {
            engine: EngineMode::Tex82,
            ..SessionOptions::default()
        },
        bibliography: BibliographyProjectOptions {
            mode,
            biblatex: bib_engine::BibOptions::default(),
            bib_session: BibSessionOptions::default(),
            classic: bib_engine::ClassicBibOptions::default(),
            detector: bib_engine::BibliographyDetectorOptions::default(),
        },
        limits: LatexProjectLimits::default(),
    }
}

fn classic_project_source() -> &'static [u8] {
    br#"\immediate\openout1=main.aux
\immediate\write1{\relax}
\immediate\write1{\string\citation{knuth}}
\immediate\write1{\string\bibdata{smoke}}
\immediate\write1{\string\bibstyle{smoke}}
\immediate\closeout1
\shipout\hbox{X}\end
"#
}

#[test]
fn v2_project_retains_one_tex_pass_across_positive_and_negative_resources() {
    let source = b"\\input first \\openin0=absent \\ifeof0\\closein0\\fi \\end\n";
    let mut session =
        LatexProjectSessionV2::new(classic_options(bib_engine::BibliographyMode::Auto {
            job_path: VirtualPath::user("/job/main").expect("job"),
        }))
        .expect("session");
    session
        .add_user_file("/job/main.tex", source.to_vec())
        .expect("source");

    let LatexProjectAttemptV2::NeedResources(first) = session.compile_attempt() else {
        panic!("first resource suspension");
    };
    assert_eq!(
        session
            .candidate
            .as_ref()
            .expect("retained project candidate")
            .tex
            .as_ref()
            .expect("retained TeX pass")
            .attempts(),
        1
    );
    session
        .provide_resources(vec![resolved(
            only_file_request(&first),
            "/texlive/first.tex",
            b"% first\n",
        )])
        .expect("first resource");

    let probe = match session.compile_attempt() {
        LatexProjectAttemptV2::NeedResources(needs) => needs,
        other => panic!("expected probe suspension, got {other:?}"),
    };
    assert_eq!(
        session
            .candidate
            .as_ref()
            .expect("retained project candidate")
            .tex
            .as_ref()
            .expect("retained TeX pass")
            .attempts(),
        2
    );
    let [ResourceRequest::File(probe)] = probe.probes.as_slice() else {
        panic!("expected one probe");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(probe.key().clone())])
        .expect("negative probe");
    assert!(matches!(
        session.compile_attempt(),
        LatexProjectAttemptV2::Complete(_)
    ));
}

fn finish_classic_project(session: &mut LatexProjectSessionV2) -> LatexProjectOutputV2 {
    loop {
        match session.compile_attempt() {
            LatexProjectAttemptV2::Complete(output) => return *output,
            LatexProjectAttemptV2::NeedResources(needs) => {
                let responses = needs
                    .required
                    .into_iter()
                    .map(|request| match request {
                        ResourceRequest::File(file) => {
                            let (path, bytes) = match file.key().name() {
                                "smoke.bib" => (
                                    "/texlive/bib/smoke.bib",
                                    include_bytes!(
                                        "../../../../tests/corpus/bibtex/cases/smoke/smoke.bib"
                                    )
                                    .to_vec(),
                                ),
                                "smoke.bst" => (
                                    "/texlive/bib/smoke.bst",
                                    include_bytes!(
                                        "../../../../tests/corpus/bibtex/cases/smoke/smoke.bst"
                                    )
                                    .to_vec(),
                                ),
                                "fatal.bst" => ("/texlive/bib/fatal.bst", b"ENTRY {".to_vec()),
                                name => panic!("unexpected resource {name}"),
                            };
                            ResourceResponse::File(ResolvedFile {
                                request: file.key().clone(),
                                virtual_path: path.into(),
                                bytes,
                                expected_digest: None,
                            })
                        }
                        ResourceRequest::Font(_) => panic!("unexpected font request"),
                    })
                    .collect();
                session
                    .provide_resources(responses)
                    .expect("provide classic resource");
            }
            LatexProjectAttemptV2::Error(error) => panic!("classic project failed: {error}"),
        }
    }
}

#[test]
fn classic_projects_converge_transactionally_with_explicit_and_auto_modes() {
    for mode in [
        bib_engine::BibliographyMode::Classic {
            aux_path: VirtualPath::user("/job/main.aux").expect("aux"),
        },
        bib_engine::BibliographyMode::Auto {
            job_path: VirtualPath::user("/job/main").expect("job"),
        },
    ] {
        let mut session =
            LatexProjectSessionV2::new(classic_options(mode.clone())).expect("project");
        session
            .add_user_file("/job/main.tex", classic_project_source().to_vec())
            .expect("source");
        let output = finish_classic_project(&mut session);
        assert!(output.passes >= 2);
        assert_eq!(
            output.fingerprint.backend,
            Some(bib_engine::BibliographyBackend::Classic),
            "mode: {mode:?}, files: {:?}",
            output
                .generated_files
                .iter()
                .map(|file| &file.path)
                .collect::<Vec<_>>()
        );
        assert!(
            output
                .generated_files
                .iter()
                .any(|file| file.path == std::path::Path::new("/job/main.bbl"))
        );
        assert!(
            output
                .generated_files
                .iter()
                .any(|file| file.path == std::path::Path::new("/job/main.blg"))
        );
    }
}

#[test]
fn v2_backend_switch_discards_incompatible_bibliography_artifacts() {
    let mut session =
        LatexProjectSessionV2::new(classic_options(bib_engine::BibliographyMode::Classic {
            aux_path: VirtualPath::user("/job/main.aux").expect("aux"),
        }))
        .expect("project");
    session
        .add_user_file("/job/main.tex", classic_project_source().to_vec())
        .expect("source");
    let accepted = finish_classic_project(&mut session);
    session
        .set_bibliography(BibliographyProjectOptions::auto(
            VirtualPath::user("/job/no-bibliography").expect("job"),
        ))
        .expect("switch");
    let marker = classic_project_source()
        .windows(1)
        .position(|_| true)
        .expect("source");
    session
        .apply_patch(SourcePatch {
            base_revision: accepted.revision,
            next_revision: tex_incr::RevisionId::new(2),
            expected_hash: accepted.content_hash,
            range: marker..marker,
            replacement: "% switched\n".into(),
        })
        .expect("patch");
    let output = finish_classic_project(&mut session);
    assert_eq!(output.fingerprint.backend, None);
    assert!(
        !output
            .generated_files
            .iter()
            .any(|file| file.path == std::path::Path::new("/job/main.bbl"))
    );
    assert_eq!(session.revision(), Some(tex_incr::RevisionId::new(2)));
}

#[test]
fn fatal_classic_execution_rolls_back_to_the_last_accepted_project() {
    let mut session =
        LatexProjectSessionV2::new(classic_options(bib_engine::BibliographyMode::Classic {
            aux_path: VirtualPath::user("/job/main.aux").expect("aux"),
        }))
        .expect("project");
    let source = classic_project_source().to_vec();
    session
        .add_user_file("/job/main.tex", source.clone())
        .expect("source");
    let accepted = finish_classic_project(&mut session);
    let style = source
        .windows(b"smoke}".len())
        .rposition(|window| window == b"smoke}")
        .expect("style name");
    session
        .apply_patch(SourcePatch {
            base_revision: accepted.revision,
            next_revision: tex_incr::RevisionId::new(2),
            expected_hash: accepted.content_hash,
            range: style..style + b"smoke".len(),
            replacement: "fatal".into(),
        })
        .expect("patch");
    loop {
        match session.compile_attempt() {
            LatexProjectAttemptV2::NeedResources(needs) => {
                let responses = needs
                    .required
                    .into_iter()
                    .map(|request| match request {
                        ResourceRequest::File(file) => {
                            let (path, bytes) = match file.key().name() {
                                "smoke.bib" => (
                                    "/texlive/bib/smoke.bib",
                                    include_bytes!(
                                        "../../../../tests/corpus/bibtex/cases/smoke/smoke.bib"
                                    )
                                    .to_vec(),
                                ),
                                "fatal.bst" => ("/texlive/bib/fatal.bst", b"ENTRY {".to_vec()),
                                name => panic!("unexpected resource {name}"),
                            };
                            ResourceResponse::File(ResolvedFile {
                                request: file.key().clone(),
                                virtual_path: path.into(),
                                bytes,
                                expected_digest: None,
                            })
                        }
                        ResourceRequest::Font(_) => panic!("unexpected font request"),
                    })
                    .collect();
                session.provide_resources(responses).expect("resources");
            }
            LatexProjectAttemptV2::Error(LatexProjectError::BibliographyFatal {
                backend: bib_engine::BibliographyBackend::Classic,
            }) => break,
            other => panic!("expected fatal classic rollback, got {other:?}"),
        }
    }
    assert_eq!(session.accepted_output(), Some(&accepted));
    assert_eq!(session.revision(), Some(tex_incr::RevisionId::new(1)));
}
