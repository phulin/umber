use crate::{EngineMode, ResourceRequest, ResourceResponse};

use super::*;

fn options() -> TexFixedPointOptions {
    TexFixedPointOptions {
        tex: SessionOptions {
            engine: EngineMode::Tex82,
            ..SessionOptions::default()
        },
        limits: FixedPointLimits::default(),
    }
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
