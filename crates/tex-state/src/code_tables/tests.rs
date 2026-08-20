use super::CodeTables;
use crate::token::Catcode;
use proptest::prelude::*;
use std::hash::Hasher;

#[test]
fn initex_catcode_defaults_match_tex82_ascii() {
    let tables = CodeTables::new();

    assert_eq!(tables.catcode('\0'), Catcode::Ignored);
    assert_eq!(tables.catcode('\r'), Catcode::EndLine);
    assert_eq!(tables.catcode(' '), Catcode::Space);
    assert_eq!(tables.catcode('\\'), Catcode::Escape);
    assert_eq!(tables.catcode('%'), Catcode::Comment);
    // tex.web §232 assigns no other category codes: `{ } $ & # ^ _` stay
    // `other_char` until a format assigns them.
    assert_eq!(tables.catcode('{'), Catcode::Other);
    assert_eq!(tables.catcode('}'), Catcode::Other);
    assert_eq!(tables.catcode('$'), Catcode::Other);
    assert_eq!(tables.catcode('&'), Catcode::Other);
    assert_eq!(tables.catcode('#'), Catcode::Other);
    assert_eq!(tables.catcode('^'), Catcode::Other);
    assert_eq!(tables.catcode('_'), Catcode::Other);
    assert_eq!(tables.catcode('~'), Catcode::Other);
    assert_eq!(tables.catcode('\u{7f}'), Catcode::Invalid);
    assert_eq!(tables.catcode('A'), Catcode::Letter);
    assert_eq!(tables.catcode('z'), Catcode::Letter);
    assert_eq!(tables.catcode('@'), Catcode::Other);
    assert_eq!(tables.catcode('é'), Catcode::Other);
}

#[test]
fn initex_case_space_math_and_delimiter_defaults() {
    let tables = CodeTables::new();

    assert_eq!(tables.lccode('A'), u32::from('a'));
    assert_eq!(tables.lccode('a'), u32::from('a'));
    assert_eq!(tables.lccode('@'), 0);
    assert_eq!(tables.uccode('A'), u32::from('A'));
    assert_eq!(tables.uccode('a'), u32::from('A'));
    assert_eq!(tables.uccode('@'), 0);
    assert_eq!(tables.sfcode('A'), 999);
    assert_eq!(tables.sfcode('a'), 1000);
    assert_eq!(tables.sfcode('é'), 1000);
    assert_eq!(tables.mathcode('0'), 0x7030);
    assert_eq!(tables.mathcode('A'), 0x7141);
    assert_eq!(tables.mathcode('a'), 0x7161);
    assert_eq!(tables.mathcode('@'), u32::from('@'));
    assert_eq!(tables.mathcode('é'), u32::from('é'));
    // tex.web §240: every `del_code` is -1 except the null delimiter `.`.
    assert_eq!(tables.delcode('A'), -1);
    assert_eq!(tables.delcode('.'), 0);
}

#[test]
fn snapshot_restores_every_code_table_value() {
    let mut tables = CodeTables::new();
    let snapshot = tables.checkpoint();

    tables.set_catcode('@', Catcode::Letter);
    tables.set_lccode('@', u32::from('a'));
    tables.set_uccode('@', u32::from('A'));
    tables.set_sfcode('A', 1000);
    tables.set_mathcode('∑', 0x1350);
    tables.set_delcode('[', 0x45);

    tables.rollback_to(snapshot);

    assert_eq!(tables.catcode('@'), Catcode::Other);
    assert_eq!(tables.lccode('@'), 0);
    assert_eq!(tables.uccode('@'), 0);
    assert_eq!(tables.sfcode('A'), 999);
    assert_eq!(tables.mathcode('∑'), u32::from('∑'));
    assert_eq!(tables.delcode('['), -1);
}

#[test]
fn save_stack_projection_counts_local_code_table_runs() {
    // TeX82 §§240/275 initialize code-table cells at level_one and preserve
    // each first local assignment as a two-word restore_old_value record.
    // Reassignment in one local run is free; a global write retains the old
    // physical record but lets the next local run allocate another.
    let mut tables = CodeTables::new();
    tables.enter_group();
    tables.set_catcode_at(1, '@', Catcode::Letter);
    assert_eq!(tables.canonical_save_stack_words(), 2);
    assert_eq!(tables.canonical_save_stack_projection().1, Some((1, 2)));
    tables.set_catcode_at(2, '@', Catcode::Active);
    assert_eq!(tables.canonical_save_stack_words(), 2);

    tables.set_catcode_global('@', Catcode::Other);
    assert_eq!(tables.canonical_save_stack_words(), 2);
    tables.set_catcode_at(3, '@', Catcode::Letter);
    assert_eq!(tables.canonical_save_stack_words(), 4);
    assert_eq!(tables.canonical_save_stack_projection().1, Some((3, 2)));

    tables.enter_group();
    tables.set_lccode_at(4, '@', u32::from('a'));
    assert_eq!(tables.canonical_save_stack_words(), 6);
    let _ = tables.leave_group();
    assert_eq!(tables.canonical_save_stack_words(), 4);
    assert_eq!(tables.canonical_save_stack_projection().1, Some((3, 2)));
}

#[test]
fn save_stack_projection_rolls_back_with_code_table_roots() {
    let mut tables = CodeTables::new();
    tables.enter_group();
    tables.set_catcode_at(1, '@', Catcode::Letter);
    let snapshot = tables.checkpoint();

    tables.set_lccode_at(2, '@', u32::from('a'));
    assert_eq!(tables.canonical_save_stack_projection(), (4, Some((2, 2))));

    tables.rollback_to(snapshot);
    assert_eq!(tables.canonical_save_stack_projection(), (2, Some((1, 2))));
}

#[test]
fn testing_hash_is_independent_of_sparse_update_order() {
    let mut left = CodeTables::new();
    left.set_catcode('🦀', Catcode::Letter);
    left.set_catcode('λ', Catcode::Active);
    let mut right = CodeTables::new();
    right.set_catcode('λ', Catcode::Active);
    right.set_catcode('🦀', Catcode::Letter);

    let mut left_hash = ahash::AHasher::default();
    left.testing_hash_content(&mut left_hash);
    let mut right_hash = ahash::AHasher::default();
    right.testing_hash_content(&mut right_hash);

    assert_eq!(left_hash.finish(), right_hash.finish());
}

#[test]
fn no_op_write_preserves_the_observable_value() {
    let mut tables = CodeTables::new();

    tables.set_catcode('@', Catcode::Other);

    assert_eq!(tables.catcode('@'), Catcode::Other);
}

#[test]
fn global_writes_override_interleaved_locals_at_each_group_exit() {
    let mut tables = CodeTables::new();
    tables.enter_group();
    tables.set_catcode('@', Catcode::Letter);
    tables.set_lccode('@', u32::from('a'));
    tables.enter_group();
    tables.set_catcode_global('@', Catcode::Active);
    tables.set_lccode_global('@', u32::from('z'));
    tables.set_catcode('@', Catcode::Comment);
    tables.set_lccode('@', u32::from('x'));

    tables.leave_group();
    assert_eq!(tables.catcode('@'), Catcode::Active);
    assert_eq!(tables.lccode('@'), u32::from('z'));
    tables.set_catcode('@', Catcode::Letter);
    tables.set_lccode('@', u32::from('y'));

    tables.leave_group();
    assert_eq!(tables.catcode('@'), Catcode::Active);
    assert_eq!(tables.lccode('@'), u32::from('z'));
}

#[test]
fn same_value_global_assignment_survives_group_exit() {
    let mut tables = CodeTables::new();
    tables.enter_group();

    tables.set_catcode_global('@', Catcode::Other);

    tables.leave_group();
    assert_eq!(tables.catcode('@'), Catcode::Other);
}

#[test]
fn rollback_restores_the_global_write_history_inside_groups() {
    let mut tables = CodeTables::new();
    tables.enter_group();
    let snapshot = tables.checkpoint();
    tables.set_catcode_global('@', Catcode::Letter);
    assert_eq!(tables.global_writes.len(), 1);

    tables.rollback_to(snapshot);

    assert_eq!(tables.global_writes.len(), 0);
    tables.leave_group();
    assert_eq!(tables.catcode('@'), Catcode::Other);
}

proptest! {
    #[test]
    fn snapshots_restore_arbitrary_catcode_values(
        ch in any::<char>(),
        replacement in 0_u8..=15,
    ) {
        let replacement = catcode_from_u8(replacement);
        let mut tables = CodeTables::new();
        let before = tables.catcode(ch);
        let snapshot = tables.checkpoint();

        tables.set_catcode(ch, replacement);
        prop_assert_eq!(tables.catcode(ch), replacement);

        tables.rollback_to(snapshot);
        prop_assert_eq!(tables.catcode(ch), before);
    }
}

fn catcode_from_u8(value: u8) -> Catcode {
    match value {
        0 => Catcode::Escape,
        1 => Catcode::BeginGroup,
        2 => Catcode::EndGroup,
        3 => Catcode::MathShift,
        4 => Catcode::AlignmentTab,
        5 => Catcode::EndLine,
        6 => Catcode::Parameter,
        7 => Catcode::Superscript,
        8 => Catcode::Subscript,
        9 => Catcode::Ignored,
        10 => Catcode::Space,
        11 => Catcode::Letter,
        12 => Catcode::Other,
        13 => Catcode::Active,
        14 => Catcode::Comment,
        15 => Catcode::Invalid,
        _ => unreachable!("strategy bounds catcodes"),
    }
}
