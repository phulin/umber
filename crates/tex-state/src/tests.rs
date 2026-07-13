use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::TokenListId;
use crate::page::PageMark;
use crate::scaled::Scaled;
use crate::token::{Catcode, Token};
use crate::{ParagraphShapeLine, Universe, World};

mod handle_matrix;
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
fn paragraph_shape_is_grouped_checkpointed_and_format_stable() {
    let outer = [ParagraphShapeLine {
        indent: Scaled::from_raw(3),
        width: Scaled::from_raw(40),
    }];
    let inner = [ParagraphShapeLine {
        indent: Scaled::from_raw(-7),
        width: Scaled::from_raw(90),
    }];
    let mut universe = Universe::new();
    universe.set_paragraph_shape(&outer, false);
    let snapshot = universe.snapshot();

    universe.enter_group();
    universe.set_paragraph_shape(&inner, false);
    assert_eq!(universe.paragraph_shape(), inner);
    let _ = universe.leave_group();
    assert_eq!(universe.paragraph_shape(), outer);

    universe.set_paragraph_shape(&inner, false);
    universe.rollback(&snapshot);
    assert_eq!(universe.paragraph_shape(), outer);

    let format = universe.dump_format().expect("paragraph shape format");
    let loaded = Universe::from_format(World::default(), &format).expect("load paragraph shape");
    assert_eq!(loaded.paragraph_shape(), outer);
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
    universe.set_page_mark_class(PageMark::Bot, 27, before);
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
    universe.set_page_mark_class(PageMark::Top, 27, after);
    universe.set_page_mark_class(PageMark::First, 27, after);
    universe.set_page_mark_class(PageMark::Bot, 27, after);

    universe.rollback(&snapshot);

    assert_eq!(universe.page_mark(PageMark::Top), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::First), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::Bot), before);
    assert_eq!(universe.page_mark(PageMark::SplitFirst), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::SplitBot), TokenListId::EMPTY);
    assert_eq!(
        universe.page_mark_class(PageMark::Top, 27),
        TokenListId::EMPTY
    );
    assert_eq!(universe.page_mark_class(PageMark::Bot, 27), before);
}

#[test]
fn frozen_alignment_token_kinds_have_distinct_semantic_hashes() {
    let mut universe = Universe::new();
    let checkpoint = universe.snapshot();
    let end_template = universe.intern_token_list(&[Token::frozen_end_template()]);
    universe.set_toks(0, end_template);
    let end_template_hash = universe.snapshot().state_hash();

    universe.rollback(&checkpoint);
    let endv = universe.intern_token_list(&[Token::frozen_endv()]);
    universe.set_toks(0, endv);
    let endv_hash = universe.snapshot().state_hash();

    assert_ne!(end_template_hash, endv_hash);
}
