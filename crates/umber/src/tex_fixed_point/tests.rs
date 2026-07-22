use crate::{EngineMode, ResourceRequest, ResourceResponse};

use super::*;

const PRIMITIVE_GENERATED: &[u8] =
    include_bytes!("../../../../tests/corpus/stabilization/primitive-generated.tex");
const LATEX_REFERENCES: &[u8] =
    include_bytes!("../../../../tests/corpus/stabilization/latex-references.tex");
const CMR10_TFM: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

fn options() -> TexFixedPointOptions {
    TexFixedPointOptions {
        tex: SessionOptions {
            engine: EngineMode::Tex82,
            ..SessionOptions::default()
        },
        limits: FixedPointLimits::default(),
    }
}

fn finish(
    session: &mut TexFixedPointSession,
) -> Result<Box<TexFixedPointOutput>, TexFixedPointError> {
    loop {
        match session.compile_attempt() {
            TexFixedPointAttempt::Complete(output) => return Ok(output),
            TexFixedPointAttempt::Error(error) => return Err(error),
            TexFixedPointAttempt::NeedResources(needs) => {
                let responses = needs
                    .required
                    .into_iter()
                    .chain(needs.probes)
                    .map(|request| match request {
                        ResourceRequest::File(request) => {
                            ResourceResponse::FileUnavailable(request.key().clone())
                        }
                        request => panic!("unexpected non-file resource request: {request:?}"),
                    })
                    .collect();
                session.provide_resources(responses)?;
            }
        }
    }
}

fn generated<'a>(output: &'a TexFixedPointOutput, path: &str) -> &'a [u8] {
    output
        .generated_files
        .iter()
        .find(|file| file.path == std::path::Path::new(path))
        .unwrap_or_else(|| panic!("missing generated file {path}"))
        .bytes
        .as_slice()
}

#[test]
fn primitive_and_latex_reference_fixtures_reach_cold_identical_fixed_points() {
    for (engine, source, expected_files) in [
        (
            EngineMode::Tex82,
            PRIMITIVE_GENERATED,
            &["/job/optional.aux", "/job/state.aux", "/job/unused.aux"][..],
        ),
        (
            EngineMode::Tex82,
            LATEX_REFERENCES,
            &["/job/main.aux", "/job/main.toc"][..],
        ),
    ] {
        let case_options = TexFixedPointOptions {
            tex: SessionOptions {
                engine,
                ..SessionOptions::default()
            },
            limits: FixedPointLimits::default(),
        };
        let run = |options: TexFixedPointOptions| {
            let mut session = TexFixedPointSession::new(options).expect("session");
            session
                .add_user_file("/job/main.tex", source.to_vec())
                .expect("source");
            if std::ptr::eq(source, LATEX_REFERENCES) {
                session
                    .add_user_file("/job/cmr10.tfm", CMR10_TFM.to_vec())
                    .expect("TFM");
            }
            let output = finish(&mut session)
                .unwrap_or_else(|error| panic!("{engine:?} fixed point failed: {error:?}"));
            (session, output)
        };
        let (first_session, first) = run(case_options.clone());
        let (_, cold) = run(case_options);

        assert_eq!(first.as_ref(), cold.as_ref());
        assert_eq!(first.passes, 2);
        assert_eq!(
            first
                .generated_files
                .iter()
                .map(|file| file.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            expected_files
        );
        assert_eq!(first.generated_fingerprint.len(), expected_files.len());
        assert!(!first.tex.dvi.is_empty());

        let ledger = first_session
            .accepted_input_observations()
            .expect("accepted observation ledger");
        for path in expected_files
            .iter()
            .filter(|path| !path.ends_with("unused.aux"))
        {
            assert!(ledger.observations().iter().any(|item| {
                item.path().as_str() == *path
                    && item.outcome() == crate::InputObservationOutcome::Missing
            }));
            assert!(ledger.observations().iter().any(|item| {
                item.path().as_str() == *path
                    && matches!(item.outcome(), crate::InputObservationOutcome::Present(_))
            }));
        }
    }
}

#[test]
fn changed_consumed_input_matches_cold_and_changed_unused_output_is_selected() {
    let mut session = TexFixedPointSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", PRIMITIVE_GENERATED.to_vec())
        .expect("source");
    let accepted = finish(&mut session).expect("initial fixed point");

    let mut changed = PRIMITIVE_GENERATED.to_vec();
    let stable = changed
        .windows(b"stable".len())
        .position(|window| window == b"stable")
        .expect("consumed generated value");
    changed.splice(stable..stable + b"stable".len(), b"updated".iter().copied());
    let unused = changed
        .windows(b"unused-v1".len())
        .position(|window| window == b"unused-v1")
        .expect("unused generated value");
    changed.splice(
        unused..unused + b"unused-v1".len(),
        b"unused-v2".iter().copied(),
    );
    session
        .apply_patch(SourcePatch {
            base_revision: accepted.revision,
            next_revision: tex_incr::RevisionId::new(2),
            expected_hash: accepted.content_hash,
            range: 0..PRIMITIVE_GENERATED.len(),
            replacement: String::from_utf8(changed.clone()).expect("ASCII fixture"),
        })
        .expect("patch");
    let changed_output = finish(&mut session).expect("changed fixed point");

    let mut cold = TexFixedPointSession::new(options()).expect("cold session");
    cold.add_user_file("/job/main.tex", changed)
        .expect("cold source");
    let cold_output = finish(&mut cold).expect("cold fixed point");
    assert_eq!(changed_output.tex, cold_output.tex);
    assert_eq!(changed_output.generated_files, cold_output.generated_files);
    assert_eq!(
        changed_output.generated_fingerprint,
        cold_output.generated_fingerprint
    );
    assert_eq!(changed_output.passes, cold_output.passes);
    assert!(
        String::from_utf8_lossy(generated(&changed_output, "/job/state.aux")).contains("updated")
    );
    assert_eq!(
        generated(&changed_output, "/job/unused.aux"),
        b"unused-v2\n"
    );
}

#[test]
fn tex_only_generated_files_converge_without_bibliography() {
    let source = br#"\openin0=state.aux
\ifeof0 \def\value{first}\else \read0 to \value \closein0 \fi
\immediate\openout1=state.aux
\immediate\write1{stable}
\immediate\closeout1
\shipout\hbox{\value}\end
"#;
    let mut session = TexFixedPointSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", source.to_vec())
        .expect("source");
    let TexFixedPointAttempt::NeedResources(needs) = session.compile_attempt() else {
        panic!("initial missing generated input probe");
    };
    let [ResourceRequest::File(probe)] = needs.probes.as_slice() else {
        panic!("one generated input probe");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(probe.key().clone())])
        .expect("negative generated input probe");
    let output = match session.compile_attempt() {
        TexFixedPointAttempt::Complete(output) => output,
        other => panic!("expected complete fixed point, got {other:?}"),
    };
    assert_eq!(output.passes, 2);
    assert!(output.bibliography_free());
    assert_eq!(session.accepted_output(), Some(output.as_ref()));
}

#[test]
fn tex_only_resource_wait_resumes_the_same_pass() {
    let mut session = TexFixedPointSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", b"\\input remote \\end\n".to_vec())
        .expect("source");
    let TexFixedPointAttempt::NeedResources(needs) = session.compile_attempt() else {
        panic!("resource suspension");
    };
    let [ResourceRequest::File(request)] = needs.required.as_slice() else {
        panic!("one file request");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request.key().clone(),
        )])
        .expect("negative response");
    assert!(matches!(
        session.compile_attempt(),
        TexFixedPointAttempt::Error(TexFixedPointError::Compile(_))
    ));
    assert!(session.accepted_output().is_none());
}

#[test]
fn tex_only_no_progress_and_pass_limit_are_typed_and_atomic() {
    let mut waiting = TexFixedPointSession::new(options()).expect("session");
    waiting
        .add_user_file("/job/main.tex", b"\\input remote \\end\n".to_vec())
        .expect("source");
    assert!(matches!(
        waiting.compile_attempt(),
        TexFixedPointAttempt::NeedResources(_)
    ));
    assert_eq!(
        waiting.compile_attempt(),
        TexFixedPointAttempt::Error(TexFixedPointError::Compile(CompileError::NoProgress))
    );
    assert!(waiting.accepted_output().is_none());

    let mut limited_options = options();
    limited_options.limits.passes = 1;
    let mut limited = TexFixedPointSession::new(limited_options).expect("session");
    limited
        .add_user_file(
            "/job/main.tex",
            b"\\immediate\\openout1=state.aux \\immediate\\write1{changed} \\immediate\\closeout1 \\end\n"
                .to_vec(),
        )
        .expect("source");
    assert_eq!(
        limited.compile_attempt(),
        TexFixedPointAttempt::Error(TexFixedPointError::PassLimit { limit: 1 })
    );
    assert!(limited.accepted_output().is_none());
}

#[test]
fn tex_only_oscillation_rolls_back_the_pending_generation() {
    let stable = b"\\immediate\\openout1=state.aux \\immediate\\write1{stable} \\immediate\\closeout1 \\end\n".to_vec();
    let mut session = TexFixedPointSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", stable.clone())
        .expect("source");
    let accepted = match session.compile_attempt() {
        TexFixedPointAttempt::Complete(output) => output,
        other => panic!("stable initial generation, got {other:?}"),
    };
    let oscillating = r#"\openin0=flip.aux
\ifeof0 \def\state{A}\else \closein0 \input flip.aux \fi
\def\statea{A}
\immediate\openout1=flip.aux
\ifx\state\statea
  \immediate\write1{\string\def\string\state{B}}
\else
  \immediate\write1{\string\def\string\state{A}}
\fi
\immediate\closeout1
\end
"#;
    session
        .apply_patch(SourcePatch {
            base_revision: accepted.revision,
            next_revision: tex_incr::RevisionId::new(2),
            expected_hash: accepted.content_hash,
            range: 0..stable.len(),
            replacement: oscillating.into(),
        })
        .expect("patch");
    let TexFixedPointAttempt::NeedResources(needs) = session.compile_attempt() else {
        panic!("missing flip probe");
    };
    let [ResourceRequest::File(probe)] = needs.probes.as_slice() else {
        panic!("one flip probe");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(probe.key().clone())])
        .expect("negative flip probe");
    assert!(matches!(
        session.compile_attempt(),
        TexFixedPointAttempt::Error(TexFixedPointError::Oscillation { .. })
    ));
    assert_eq!(session.accepted_output(), Some(accepted.as_ref()));
    assert_eq!(session.revision(), Some(tex_incr::RevisionId::new(1)));
}

impl TexFixedPointOutput {
    fn bibliography_free(&self) -> bool {
        self.generated_files.iter().all(|file| {
            file.path
                .extension()
                .is_none_or(|extension| extension != "bbl")
        })
    }
}
