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
