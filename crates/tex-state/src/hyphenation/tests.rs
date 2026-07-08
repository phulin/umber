use super::*;

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
