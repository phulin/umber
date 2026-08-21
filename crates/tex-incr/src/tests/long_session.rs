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
        assert_eq!(accepted.effects, expected.effects);
        assert_eq!(accepted.artifacts, expected.artifacts);
        assert_eq!(accepted.dvi_pages, expected.dvi_pages);
        source = next;
    }
}
