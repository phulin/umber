use super::*;

mod long_session;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

fn page_source(width: usize) -> String {
    format!("\\shipout\\vbox{{\\hrule height1pt width{width}pt}}\\end")
}

fn session(revision: RevisionId, source: &str) -> Session {
    Session::start((), "incremental-test", revision, source, 4096).expect("session starts")
}

fn edit(session: &Session, range: std::ops::Range<usize>, text: &str) -> Edit {
    Edit {
        base_revision: session.revision(),
        expected_hash: session.content_hash(),
        range,
        replacement: text.to_owned(),
    }
}

#[test]
fn cold_execution_publishes_detached_artifact_and_boundary_history() {
    let source = page_source(10);
    let mut session = session(RevisionId::new(1), &source);
    let output = session.cold().expect("cold execution");
    assert_eq!(output.artifacts.len(), 1);
    assert_eq!(output.dvi_pages.len(), 1);
    assert_eq!(
        session.history()[0].key().boundary,
        EngineBoundary::JobStart
    );
    assert!(
        session
            .history()
            .iter()
            .any(|record| record.key().boundary == EngineBoundary::ShipoutComplete)
    );
}

#[test]
fn semantic_edit_matches_a_fresh_cold_execution() {
    let original = page_source(10);
    let replacement = page_source(24);
    let mut incremental = session(RevisionId::new(1), &original);
    incremental.cold().expect("baseline");
    let accepted = incremental
        .advance(
            RevisionId::new(2),
            edit(&incremental, 0..original.len(), &replacement),
        )
        .expect("edited revision");
    let mut cold = session(RevisionId::new(2), &replacement);
    let expected = cold.cold().expect("fresh comparison");
    assert_eq!(accepted.effects, expected.effects);
    assert_eq!(accepted.artifacts, expected.artifacts);
    assert_eq!(accepted.dvi_pages, expected.dvi_pages);
    assert_eq!(
        accepted.reuse.execution_path,
        RevisionExecutionPath::SlowEdit
    );
    assert_eq!(accepted.reuse.pages_reused, 0);
}

#[test]
fn no_op_edit_reports_detached_history_convergence() {
    let source = page_source(12);
    let mut session = session(RevisionId::new(1), &source);
    session.cold().expect("baseline");
    let output = session
        .advance(RevisionId::new(2), edit(&session, 0..0, ""))
        .expect("no-op revision");
    assert_eq!(output.reuse.same_history_stop, SameHistoryStop::Matched);
    assert!(output.reuse.convergence_boundary.is_some());
    assert!(output.reuse.same_history_attempts > 0);
}

#[test]
fn semantic_change_does_not_claim_suffix_adoption() {
    let original = page_source(10);
    let mut session = session(RevisionId::new(1), &original);
    session.cold().expect("baseline");
    let output = session
        .advance(
            RevisionId::new(2),
            edit(&session, 0..original.len(), &page_source(11)),
        )
        .expect("changed revision");
    assert_eq!(output.reuse.pages_reused, 0);
    assert_eq!(output.reuse.suffixes_adopted, 0);
    assert_eq!(output.reuse.convergence_boundary, None);
}

#[test]
fn dropping_prepared_revision_keeps_accepted_session_unchanged() {
    let original = page_source(10);
    let mut session = session(RevisionId::new(1), &original);
    let before = session.cold().expect("baseline");
    let transaction = session
        .prepare_revision_with_resolvers(
            RevisionId::new(2),
            edit(&session, 0..original.len(), &page_source(20)),
            &mut DirectResourceHost,
        )
        .expect("prepared revision");
    assert_ne!(transaction.artifacts(), before.artifacts);
    drop(transaction);
    assert_eq!(session.revision(), RevisionId::new(1));
    assert_eq!(session.source(), original);
    assert_eq!(session.content_hash(), before.content_hash);
}

#[test]
fn stale_revision_and_hash_rejections_do_not_mutate_state() {
    let source = page_source(10);
    let mut session = session(RevisionId::new(2), &source);
    session.cold().expect("baseline");
    let before = session.content_hash();
    let stale = Edit {
        base_revision: RevisionId::new(1),
        expected_hash: before,
        range: 0..0,
        replacement: String::new(),
    };
    assert!(matches!(
        session.advance(RevisionId::new(3), stale),
        Err(SessionError::StaleRevision { .. })
    ));
    let wrong_hash = Edit {
        base_revision: RevisionId::new(2),
        expected_hash: ContentHash::from_bytes(b"foreign"),
        range: 0..0,
        replacement: String::new(),
    };
    assert!(matches!(
        session.advance(RevisionId::new(3), wrong_hash),
        Err(SessionError::ContentHashMismatch)
    ));
    assert_eq!(session.content_hash(), before);
}

struct DeclineOnceInput(bool);

impl ResourceHost for DeclineOnceInput {
    fn fulfill(&mut self, _world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        let ResourceNeed::Input { name, .. } = need else {
            return ResourceOutcome::Unavailable;
        };
        if !self.0 {
            self.0 = true;
            return ResourceOutcome::Declined;
        }
        ResourceOutcome::Fulfilled(ResourceFulfillment::input(
            name,
            RegisteredSourceKind::Generated,
            Arc::from(b"\\relax".as_slice()),
        ))
    }
}

#[test]
fn resource_suspension_replays_from_detached_plan_and_accepts_once() {
    let mut session = session(RevisionId::new(1), "\\input child \\end");
    let mut candidate = session.start_cold_candidate().expect("candidate");
    let mut host = DeclineOnceInput(false);
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(&mut host, &Cancellation::new())
            .expect("first drive"),
        RevisionCandidateResult::AwaitingResources(ResourceNeed::Input { .. })
    ));
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(&mut host, &Cancellation::new())
            .expect("replay"),
        RevisionCandidateResult::Complete
    ));
    let output = session
        .accept_cold_candidate(candidate)
        .expect("accept replay");
    assert_eq!(output.revision, RevisionId::new(1));
}

#[test]
fn registered_font_resource_survives_each_fresh_generation() {
    let source = "\\font\\tenrm=cmr10\\relax\\tenrm\\shipout\\hbox{A}\\end";
    let mut session = session(RevisionId::new(1), source);
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("register font");
    let first = session.cold().expect("font run");
    let second = session
        .advance(RevisionId::new(2), edit(&session, 0..0, ""))
        .expect("font rerun");
    assert_eq!(first.artifacts, second.artifacts);
}

#[test]
fn history_budget_keeps_job_start_and_newest_observation() {
    let mut source = String::new();
    for width in 1..=8 {
        source.push_str(&format!(
            "\\shipout\\vbox{{\\hrule height1pt width{width}pt}}"
        ));
    }
    source.push_str("\\end");
    let mut session = Session::start((), "budget", RevisionId::new(1), source, 0).expect("session");
    session.cold().expect("cold run");
    assert_eq!(session.history().len(), 2);
    assert_eq!(
        session.history()[0].key().boundary,
        EngineBoundary::JobStart
    );
    assert_eq!(
        session.history()[1].key().boundary,
        EngineBoundary::ShipoutComplete
    );
}

#[test]
fn repeated_revisions_match_fresh_cold_output() {
    let mut source = page_source(1);
    let mut incremental = session(RevisionId::new(1), &source);
    incremental.cold().expect("baseline");
    for revision in 2..=12 {
        let next = page_source(revision as usize);
        let output = incremental
            .advance(
                RevisionId::new(revision),
                edit(&incremental, 0..source.len(), &next),
            )
            .expect("accepted edit");
        let mut cold = session(RevisionId::new(revision), &next);
        let expected = cold.cold().expect("cold comparison");
        assert_eq!(output.artifacts, expected.artifacts);
        assert_eq!(output.effects, expected.effects);
        assert_eq!(output.dvi_pages, expected.dvi_pages);
        source = next;
    }
}

#[test]
fn byte_projection_round_trips_invalid_utf8_source() {
    let bytes = vec![b'\\', b'e', b'n', b'd', 0xff];
    let session = Session::start_with_source_bytes(
        (),
        "bytes",
        "bytes.tex",
        RevisionId::new(1),
        bytes.clone(),
        1024,
    )
    .expect("byte session");
    assert_eq!(session.source_file_bytes(session.source()), bytes);
}

#[test]
fn rendered_query_rejects_foreign_output_before_artifact_lookup() {
    let source = page_source(10);
    let mut first = session(RevisionId::new(1), &source);
    first.cold().expect("first");
    let second = session(RevisionId::new(1), &source);
    assert_eq!(
        first
            .rendered_source_location(1, 0, None, second.output_id(), first.revision())
            .expect("query"),
        Some(RenderedSourceResult::OutputMismatch {
            accepted: first.output_id()
        })
    );
}
