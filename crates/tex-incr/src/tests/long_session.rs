use super::*;

#[test]
fn accepted_and_rejected_revisions_remain_cold_equivalent_over_a_long_session() {
    let mut source = page_source(1);
    let mut incremental = session(RevisionId::new(1), &source);
    incremental.cold().expect("initial revision");

    for revision in 2_u64..=64 {
        let rejected = Edit {
            base_revision: RevisionId::new(revision.saturating_sub(2)),
            expected_hash: incremental.content_hash(),
            range: 0..0,
            replacement: String::new(),
        };
        assert!(matches!(
            incremental.advance(RevisionId::new(revision), rejected),
            Err(SessionError::StaleRevision { .. })
        ));

        let next = page_source(revision as usize);
        let accepted = incremental
            .advance(
                RevisionId::new(revision),
                edit(&incremental, 0..source.len(), &next),
            )
            .expect("accepted revision");
        let mut cold = session(RevisionId::new(revision), &next);
        let expected = cold.cold().expect("cold comparison");
        assert_detached_output_eq(&accepted, &expected);
        source = next;
    }
}

/// Explicit daemon-style semantic stress. Equal-work milestones compare the
/// exact detached output with a fresh run; every intervening edit still
/// exercises acceptance and history pruning.
#[test]
#[ignore = "explicit 2,048-cycle incremental semantic stress tier"]
fn long_session_thousands_match_clean_at_equal_work_milestones() {
    let mut source = page_source(1);
    let mut incremental = Session::start(
        Box::leak(Box::new(new_reachability_store())),
        "stress",
        RevisionId::new(1),
        &source,
        256,
    )
    .expect("stress session");
    incremental.cold().expect("initial revision");

    for revision in 2_u64..=2_049 {
        let width = usize::try_from((revision * 97) % 8_191 + 1).expect("bounded width");
        let next = page_source(width);
        let accepted = incremental
            .advance(
                RevisionId::new(revision),
                edit(&incremental, 0..source.len(), &next),
            )
            .expect("stress edit accepts");
        if revision.is_power_of_two() || revision == 2_049 {
            let mut cold = session(RevisionId::new(revision), &next);
            let expected = cold.cold().expect("milestone cold comparison");
            assert_detached_output_eq(&accepted, &expected);
        }
        source = next;
    }
}
