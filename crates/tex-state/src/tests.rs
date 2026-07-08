use crate::Universe;
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::TokenListId;
use crate::page::PageMark;
use crate::token::{Catcode, Token};

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

#[test]
fn page_mark_slots_roll_back_with_snapshots() {
    let mut universe = Universe::new();
    let before = universe.intern_token_list(&[Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    }]);
    universe.set_page_mark(PageMark::Bot, before);
    let snapshot = universe.snapshot();

    let after = universe.intern_token_list(&[Token::Char {
        ch: 'b',
        cat: Catcode::Letter,
    }]);
    universe.set_page_mark(PageMark::Top, after);
    universe.set_page_mark(PageMark::First, after);
    universe.set_page_mark(PageMark::Bot, after);
    universe.set_page_mark(PageMark::SplitFirst, after);
    universe.set_page_mark(PageMark::SplitBot, after);

    universe.rollback(&snapshot);

    assert_eq!(universe.page_mark(PageMark::Top), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::First), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::Bot), before);
    assert_eq!(universe.page_mark(PageMark::SplitFirst), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::SplitBot), TokenListId::EMPTY);
}
