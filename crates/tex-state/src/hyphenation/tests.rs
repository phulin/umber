use super::*;

#[test]
fn operationless_pattern_does_not_occupy_its_trie_path() {
    // TeX82 §965 computes min_trie_op when every effective hyphenation value
    // is zero; §963 then installs that replacement at the path boundary.
    let mut table = HyphenationTable::new();
    assert!(
        !table
            .add_pattern_for_language(
                0,
                PatternSpec {
                    letters: vec!['b', 'b'],
                    values: vec![0, 0, 0],
                },
            )
            .expect("pattern fits the default trie capacity")
    );
    assert!(!table.contains_pattern_for_language(0, &['b', 'b']));
    assert!(
        !table
            .add_pattern_for_language(
                0,
                PatternSpec {
                    letters: vec!['b', 'b'],
                    values: vec![0, 0, 1],
                },
            )
            .expect("pattern fits the default trie capacity")
    );
    assert!(table.contains_pattern_for_language(0, &['b', 'b']));
    assert!(
        table
            .add_pattern_for_language(
                0,
                PatternSpec {
                    letters: vec!['b', 'b'],
                    values: vec![0, 2, 0],
                },
            )
            .expect("pattern fits the default trie capacity")
    );
    assert!(
        !table
            .add_pattern_for_language(
                0,
                PatternSpec {
                    letters: vec!['.', 'x', '.'],
                    values: vec![1, 0, 0, 2],
                },
            )
            .expect("pattern fits the default trie capacity")
    );
    assert!(!table.contains_pattern_for_language(0, &['.', 'x', '.']));
}

#[test]
fn trie_capacity_is_checked_before_mutating_the_pattern_table() {
    let mut table = HyphenationTable::new();
    table.set_trie_capacity(3);
    table
        .add_pattern(PatternSpec {
            letters: vec!['a', 'b'],
            values: vec![0, 1, 0],
        })
        .expect("language root and two letters exactly fill capacity");

    let before = table.clone();
    assert_eq!(
        table.add_pattern(PatternSpec {
            letters: vec!['a', 'c'],
            values: vec![0, 1, 0],
        }),
        Err(HyphenationCapacityError { capacity: 3 })
    );
    assert_eq!(table, before, "rejected insertion is atomic");
}

#[test]
fn pattern_values_apply_liang_odd_positions() {
    let mut table = HyphenationTable::new();
    table
        .add_pattern(PatternSpec {
            letters: "hyphen".chars().collect(),
            values: vec![0, 2, 0, 3, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");
    assert_eq!(table.hyphen_positions("hyphen", 2, 2), vec![3]);
}

#[test]
fn exceptions_override_patterns() {
    let mut table = HyphenationTable::new();
    table
        .add_pattern(PatternSpec {
            letters: "testing".chars().collect(),
            values: vec![0, 0, 1, 0, 1, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");
    table.add_exception(ExceptionSpec {
        word: "testing".to_owned(),
        positions: vec![4],
    });
    assert_eq!(table.hyphen_positions("testing", 2, 2), vec![4]);
}

#[test]
fn pattern_overlay_and_language_exception_matrix() {
    // TeX82 §§923-933 overlays every matching pattern by taking the maximum
    // value at each interletter position. Sections 951-966 qualify tries by
    // language, while §§934-941 consult that language's exception first.
    let mut table = HyphenationTable::new();

    table
        .add_pattern_for_language(
            1,
            PatternSpec {
                letters: "ab".chars().collect(),
                values: vec![0, 1, 0],
            },
        )
        .expect("pattern fits the default trie capacity");
    table
        .add_pattern_for_language(
            1,
            PatternSpec {
                letters: "abc".chars().collect(),
                values: vec![0, 2, 0, 0],
            },
        )
        .expect("pattern fits the default trie capacity");
    table
        .add_pattern_for_language(
            1,
            PatternSpec {
                letters: "bc".chars().collect(),
                values: vec![0, 0, 3],
            },
        )
        .expect("pattern fits the default trie capacity");
    table
        .add_pattern_for_language(
            2,
            PatternSpec {
                letters: "ab".chars().collect(),
                values: vec![0, 1, 0],
            },
        )
        .expect("pattern fits the default trie capacity");

    assert_eq!(
        table.hyphen_positions_for_language(1, "abcd", 0, 0),
        vec![3]
    );
    assert_eq!(
        table.hyphen_positions_for_language(2, "abcd", 0, 0),
        vec![1]
    );
    assert!(
        table
            .hyphen_positions_for_language(3, "abcd", 0, 0)
            .is_empty()
    );

    table.add_exception_for_language(
        1,
        ExceptionSpec {
            word: "abcd".to_owned(),
            positions: vec![2],
        },
    );
    assert_eq!(
        table.hyphen_positions_for_language(1, "abcd", 0, 0),
        vec![2]
    );
    assert_eq!(
        table.hyphen_positions_for_language(2, "abcd", 0, 0),
        vec![1]
    );
}

#[test]
fn dependency_fingerprints_follow_snapshot_roots_and_invalidate_on_write() {
    let mut table = HyphenationTable::new();
    table
        .add_pattern(PatternSpec {
            letters: "hyphen".chars().collect(),
            values: vec![0, 2, 0, 3, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");

    let before = table.dependency_fingerprint(0, 0);
    assert!(table.dependency_fingerprints.get().is_some());
    let snapshot = table.clone();
    assert_eq!(snapshot.dependency_fingerprint(0, 0), before);

    table
        .add_pattern(PatternSpec {
            letters: "ation".chars().collect(),
            values: vec![0, 0, 1, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");
    assert!(table.dependency_fingerprints.get().is_none());
    assert_ne!(table.dependency_fingerprint(0, 0), before);
    assert_eq!(snapshot.dependency_fingerprint(0, 0), before);
}

#[test]
fn checkpoint_restore_rewinds_mutable_hyphenation_without_copying_patterns() {
    let mut table = HyphenationTable::new();
    table
        .add_pattern_for_language(
            3,
            PatternSpec {
                letters: "checkpoint".chars().collect(),
                values: vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
            },
        )
        .expect("pattern fits");
    table.add_exception_for_language(
        3,
        ExceptionSpec {
            word: "checkpoint".to_owned(),
            positions: vec![5],
        },
    );
    table.save_hyphen_codes(3, [('a', 'a')]);
    table.set_exception_capacity(1_024);
    table.close_patterns();
    assert!(table.enable_reachable_state_identity());
    let identity = table.reachable_state_identity_root();
    let checkpoint = table.checkpoint();

    table.add_exception_for_language(
        3,
        ExceptionSpec {
            word: "checkpoint".to_owned(),
            positions: vec![2, 7],
        },
    );
    table.add_exception_for_language(
        3,
        ExceptionSpec {
            word: "temporary".to_owned(),
            positions: vec![4],
        },
    );
    table.save_hyphen_codes(3, [('a', 'b'), ('z', 'z')]);
    table.set_exception_capacity(2_048);

    table.restore_checkpoint(&checkpoint);
    assert_eq!(
        table.exception_for_language(3, "checkpoint"),
        Some(&[5][..])
    );
    assert_eq!(table.exception_for_language(3, "temporary"), None);
    assert_eq!(table.saved_hyphen_code(3, 'a'), Some(Some('a')));
    assert_eq!(table.saved_hyphen_code(3, 'z'), Some(None));
    assert_eq!(table.runtime.exception_capacity, 1_024);
    assert_eq!(table.reachable_state_identity_root(), identity);
    assert_eq!(
        table.hyphen_positions_for_language(3, "checkpoint", 0, 0),
        vec![5]
    );
}

#[test]
fn checkpoint_candidate_settlement_preserves_exact_accepted_or_candidate_state() {
    let mut table = HyphenationTable::new();
    table.add_exception(ExceptionSpec {
        word: "word".to_owned(),
        positions: vec![1],
    });
    table.save_hyphen_codes(0, [('a', 'a')]);
    let checkpoint = table.checkpoint();

    table.add_exception(ExceptionSpec {
        word: "word".to_owned(),
        positions: vec![2],
    });
    table.save_hyphen_codes(0, [('a', 'b')]);
    let candidate = table.begin_checkpoint_candidate(&checkpoint);
    table.add_exception(ExceptionSpec {
        word: "word".to_owned(),
        positions: vec![3],
    });
    table.save_hyphen_codes(0, [('a', 'c')]);
    table.reject_checkpoint_candidate(candidate);
    assert_eq!(table.exception("word"), Some(&[2][..]));
    assert_eq!(table.saved_hyphen_code(0, 'a'), Some(Some('b')));

    let candidate = table.begin_checkpoint_candidate(&checkpoint);
    table.add_exception(ExceptionSpec {
        word: "word".to_owned(),
        positions: vec![4],
    });
    table.save_hyphen_codes(0, [('a', 'd')]);
    table.accept_checkpoint_candidate(candidate);
    assert_eq!(table.exception("word"), Some(&[4][..]));
    assert_eq!(table.saved_hyphen_code(0, 'a'), Some(Some('d')));
}
