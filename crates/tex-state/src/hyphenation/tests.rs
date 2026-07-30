use super::*;

#[test]
fn operationless_pattern_does_not_occupy_its_trie_path() {
    // TeX82 §965 computes min_trie_op when every effective hyphenation value
    // is zero; §963 then installs that replacement at the path boundary.
    let mut table = HyphenationTable::new();
    assert!(!table.add_pattern_for_language(
        0,
        PatternSpec {
            letters: vec!['b', 'b'],
            values: vec![0, 0, 0],
        },
    ));
    assert!(!table.contains_pattern_for_language(0, &['b', 'b']));
    assert!(!table.add_pattern_for_language(
        0,
        PatternSpec {
            letters: vec!['b', 'b'],
            values: vec![0, 0, 1],
        },
    ));
    assert!(table.contains_pattern_for_language(0, &['b', 'b']));
    assert!(table.add_pattern_for_language(
        0,
        PatternSpec {
            letters: vec!['b', 'b'],
            values: vec![0, 2, 0],
        },
    ));
    assert!(!table.add_pattern_for_language(
        0,
        PatternSpec {
            letters: vec!['.', 'x', '.'],
            values: vec![1, 0, 0, 2],
        },
    ));
    assert!(!table.contains_pattern_for_language(0, &['.', 'x', '.']));
}

#[test]
fn pattern_values_apply_liang_odd_positions() {
    let mut table = HyphenationTable::new();
    table.add_pattern(PatternSpec {
        letters: "hyphen".chars().collect(),
        values: vec![0, 2, 0, 3, 0, 0, 0],
    });
    assert_eq!(table.hyphen_positions("hyphen", 2, 2), vec![3]);
}

#[test]
fn exceptions_override_patterns() {
    let mut table = HyphenationTable::new();
    table.add_pattern(PatternSpec {
        letters: "testing".chars().collect(),
        values: vec![0, 0, 1, 0, 1, 0, 0, 0],
    });
    table.add_exception(ExceptionSpec {
        word: "testing".to_owned(),
        positions: vec![4],
    });
    assert_eq!(table.hyphen_positions("testing", 2, 2), vec![4]);
}

#[test]
fn dependency_fingerprints_follow_snapshot_roots_and_invalidate_on_write() {
    let mut table = HyphenationTable::new();
    table.add_pattern(PatternSpec {
        letters: "hyphen".chars().collect(),
        values: vec![0, 2, 0, 3, 0, 0, 0],
    });

    let before = table.dependency_fingerprint(0, 0);
    assert!(table.dependency_fingerprints.get().is_some());
    let snapshot = table.clone();
    assert_eq!(snapshot.dependency_fingerprint(0, 0), before);

    table.add_pattern(PatternSpec {
        letters: "ation".chars().collect(),
        values: vec![0, 0, 1, 0, 0, 0],
    });
    assert!(table.dependency_fingerprints.get().is_none());
    assert_ne!(table.dependency_fingerprint(0, 0), before);
    assert_eq!(snapshot.dependency_fingerprint(0, 0), before);
}
