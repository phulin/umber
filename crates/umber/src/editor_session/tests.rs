use crate::{EngineMode, ResourceRequest};

use super::*;

fn options() -> EditorSessionOptions {
    EditorSessionOptions {
        tex: SessionOptions {
            engine: EngineMode::Tex82,
            ..SessionOptions::default()
        },
        stabilization: FixedPointLimits::default(),
    }
}

#[test]
fn one_pass_is_provisional_and_stabilization_keeps_revision() {
    let source = br#"\openin0=state.aux
\ifeof0 \def\value{first}\else \read0 to \value \closein0 \fi
\immediate\openout1=state.aux
\immediate\write1{stable}
\immediate\closeout1
\shipout\hbox{\value}\end
"#;
    let mut session = EditorCompileSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", source.to_vec())
        .expect("source");
    let CompileAttemptResult::NeedResources(needs) = session.advance() else {
        panic!("initial generated-input probe");
    };
    let [ResourceRequest::File(probe)] = needs.probes.as_slice() else {
        panic!("one probe");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(probe.key().clone())])
        .expect("negative probe");
    assert!(matches!(
        session.advance(),
        CompileAttemptResult::Complete(_)
    ));
    assert_eq!(
        session.status(),
        Some(EditorSessionStatus::Provisional {
            revision: tex_incr::RevisionId::new(1),
            stabilization_required: true,
        })
    );
    assert!(session.stable_output().is_none());

    let completed = match session.stabilize_attempt() {
        EditorStabilizationAttempt::Complete(output) => output,
        other => panic!("stable output, got {other:?}"),
    };
    assert_eq!(completed.revision, tex_incr::RevisionId::new(1));
    assert_eq!(completed.passes, 2);
    assert_eq!(session.display_output(), Some(completed.as_ref()));
    assert_eq!(session.stable_output(), Some(completed.as_ref()));
    assert_eq!(
        session.status(),
        Some(EditorSessionStatus::Stable {
            revision: tex_incr::RevisionId::new(1),
            passes: 2,
            stabilization_required: false,
        })
    );

    session
        .apply_patch(SourcePatch {
            base_revision: tex_incr::RevisionId::new(1),
            next_revision: tex_incr::RevisionId::new(2),
            expected_hash: completed.content_hash,
            range: 0..source.len(),
            replacement: "\\end\n".into(),
        })
        .expect("the next public revision follows the root edit, not internal passes");
    assert!(matches!(
        session.advance(),
        CompileAttemptResult::Complete(_)
    ));
    assert!(matches!(
        session.status(),
        Some(EditorSessionStatus::Provisional {
            revision,
            stabilization_required: true,
        }) if revision == tex_incr::RevisionId::new(2)
    ));
}

#[test]
fn failed_stabilization_preserves_display_and_prior_stable_output() {
    let original = br"\shipout\hbox{old}\end
"
    .to_vec();
    let mut session = EditorCompileSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", original.clone())
        .expect("source");
    assert!(matches!(
        session.advance(),
        CompileAttemptResult::Complete(_)
    ));
    let stable_revision_one = session.stable_output().expect("initial stable").clone();

    let edited = r#"\openin0=state.aux
\ifeof0 \else \closein0 \input state.aux \fi
\immediate\openout1=state.aux
\immediate\write1{\noexpand\input remote}
\immediate\closeout1
\shipout\hbox{new}\end
"#;
    session
        .apply_patch(SourcePatch {
            base_revision: tex_incr::RevisionId::new(1),
            next_revision: tex_incr::RevisionId::new(2),
            expected_hash: ContentHash::from_bytes(&original),
            range: 0..original.len(),
            replacement: edited.into(),
        })
        .expect("patch");
    let CompileAttemptResult::NeedResources(needs) = session.advance() else {
        panic!("missing state probe");
    };
    let [ResourceRequest::File(probe)] = needs.probes.as_slice() else {
        panic!("one state probe");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(probe.key().clone())])
        .expect("negative state probe");
    assert!(matches!(
        session.advance(),
        CompileAttemptResult::Complete(_)
    ));
    let provisional = session.display_output().expect("provisional").clone();

    let attempt = session.stabilize_attempt();
    let EditorStabilizationAttempt::NeedResources(needs) = attempt else {
        panic!("second pass requests remote input, got {attempt:?}");
    };
    assert_eq!(
        session.status(),
        Some(EditorSessionStatus::Stabilizing {
            revision: tex_incr::RevisionId::new(2),
            completed_passes: 1,
            stabilization_required: true,
        })
    );
    let [ResourceRequest::File(request)] = needs.required.as_slice() else {
        panic!("one remote request");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request.key().clone(),
        )])
        .expect("negative remote response");
    assert!(matches!(
        session.stabilize_attempt(),
        EditorStabilizationAttempt::Error(TexFixedPointError::Compile(_))
    ));
    assert_eq!(session.display_output(), Some(&provisional));
    assert_eq!(session.stable_output(), Some(&stable_revision_one));
    assert!(matches!(
        session.status(),
        Some(EditorSessionStatus::Provisional {
            revision,
            stabilization_required: true,
        }) if revision == tex_incr::RevisionId::new(2)
    ));
}
