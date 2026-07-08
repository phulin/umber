use crate::Universe;
use crate::hyphenation::{ExceptionSpec, PatternSpec};

mod live_boundary;
#[cfg(feature = "testing")]
mod replay;
#[cfg(feature = "testing")]
mod replay_common;

#[test]
fn smoke() {
    assert!(!env!("CARGO_PKG_NAME").is_empty());
}

#[test]
fn hyphenation_state_rolls_back_with_snapshots() {
    let mut universe = Universe::new();
    universe.add_hyphenation_exception(ExceptionSpec {
        word: "before".to_owned(),
        positions: vec![2],
    });
    let snapshot = universe.snapshot();
    universe.add_hyphenation_exception(ExceptionSpec {
        word: "after".to_owned(),
        positions: vec![3],
    });
    universe.add_hyphenation_pattern(PatternSpec {
        letters: "after".chars().collect(),
        values: vec![0, 0, 1, 0, 0, 0],
    });

    assert_eq!(universe.hyphen_positions("after", 1, 1), vec![3]);
    universe.rollback(&snapshot);
    assert_eq!(universe.hyphen_positions("before", 1, 1), vec![2]);
    assert!(universe.hyphen_positions("after", 1, 1).is_empty());
}
