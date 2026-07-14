use super::*;

fn template() -> Universe {
    let mut universe = Universe::with_world(tex_state::World::memory());
    tex_exec::install_unexpandable_primitives(&mut universe);
    tex_expand_for_tests(&mut universe);
    universe
}

fn tex_expand_for_tests(universe: &mut Universe) {
    // tex-incr intentionally does not own primitive installation policy. The
    // compact synthetic corpus below needs only stomach primitives.
    let _ = universe;
}

fn source(label: &str) -> String {
    format!(
        "\\shipout\\vbox{{\\hrule height 1pt width {}pt}}\\shipout\\vbox{{\\hrule height 2pt}}\\end",
        label.len() + 1
    )
}

#[test]
fn cold_history_contains_only_named_restartable_boundaries() {
    let text = source("a");
    let mut session = Session::start(template(), "test", RevisionId::new(1), text, usize::MAX)
        .expect("session starts");
    let output = session.cold().expect("cold execution succeeds");
    assert_eq!(output.history[0].key().boundary, EngineBoundary::JobStart);
    assert!(session.substrate.is_some());
    assert_eq!(output.artifacts.len(), 2);
}

#[test]
fn no_op_revision_converges_at_first_eligible_boundary() {
    let text = source("a");
    let mut session = Session::start(
        template(),
        "test",
        RevisionId::new(1),
        text.clone(),
        usize::MAX,
    )
    .expect("session starts");
    let cold = session.cold().expect("cold execution succeeds");
    let output = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(text.as_bytes()),
                range: 0..0,
                replacement: String::new(),
            },
        )
        .expect("no-op revision succeeds");
    assert_eq!(
        output.reuse.convergence_boundary,
        cold.history.get(1).map(BoundaryRecord::key)
    );
    assert!(output.reuse.pages_reused > 0);
    assert_eq!(
        output.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn adopted_old_suffix_remains_restartable_on_the_next_edit() {
    let text = source("a");
    let mut session = Session::start(
        template(),
        "test",
        RevisionId::new(1),
        text.clone(),
        usize::MAX,
    )
    .expect("session starts");
    let cold = session.cold().expect("cold execution succeeds");
    for revision in 2..=3 {
        let output = session
            .advance(
                RevisionId::new(revision),
                Edit {
                    base_revision: RevisionId::new(revision - 1),
                    expected_hash: ContentHash::from_bytes(text.as_bytes()),
                    range: 0..0,
                    replacement: String::new(),
                },
            )
            .expect("adopted history remains restartable");
        assert!(output.reuse.convergence_boundary.is_some());
        assert_eq!(
            output.dvi_bytes().expect("incremental DVI"),
            cold.dvi_bytes().expect("cold DVI")
        );
    }
}

#[test]
fn edited_output_is_byte_identical_to_a_fresh_cold_session() {
    let original = source("a");
    let replacement = source("longer");
    let mut incremental = Session::start(
        template(),
        "test",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    incremental.cold().expect("initial run");
    let edited = incremental
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 0..original.len(),
                replacement: replacement.clone(),
            },
        )
        .expect("edit succeeds");

    let mut cold = Session::start(
        template(),
        "test",
        RevisionId::new(2),
        replacement,
        usize::MAX,
    )
    .expect("cold session starts");
    let cold = cold.cold().expect("cold run");
    assert_eq!(
        edited.dvi_bytes().expect("edited DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn promoted_prefix_records_remain_restartable_on_the_next_edit() {
    let first = source("a");
    let second = source("longer");
    let third = source("longest");
    let mut session = Session::start(
        template(),
        "test",
        RevisionId::new(1),
        first.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("initial run");
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(first.as_bytes()),
                range: 0..first.len(),
                replacement: second.clone(),
            },
        )
        .expect("first promotion succeeds");
    let incrementally_edited = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(second.as_bytes()),
                range: 0..second.len(),
                replacement: third.clone(),
            },
        )
        .expect("retargeted prefix restores on the next edit");

    let mut cold = Session::start(template(), "test", RevisionId::new(3), third, usize::MAX)
        .expect("cold session starts");
    let cold = cold.cold().expect("cold run");
    assert_eq!(
        incrementally_edited.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn pruning_protects_job_start_and_newest_and_reports_overage() {
    let text = source("a");
    let mut session =
        Session::start(template(), "test", RevisionId::new(1), text, 0).expect("session starts");
    let output = session.cold().expect("cold execution succeeds");
    assert_eq!(
        output.history.first().expect("job start").key().boundary,
        EngineBoundary::JobStart
    );
    assert!(output.history.len() <= 2);
    assert!(output.retention.protected_overage_bytes > 0);
    assert!(output.retention.output_bytes > 0);
}

#[test]
fn stale_revision_and_hash_are_actionable_errors() {
    let text = source("a");
    let mut session = Session::start(
        template(),
        "test",
        RevisionId::new(4),
        text.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold run");
    let error = session
        .advance(
            RevisionId::new(5),
            Edit {
                base_revision: RevisionId::new(3),
                expected_hash: ContentHash::from_bytes(text.as_bytes()),
                range: 0..0,
                replacement: String::new(),
            },
        )
        .expect_err("stale edit rejected");
    assert!(matches!(error, SessionError::StaleRevision { .. }));
}
