use super::{CodeTableGenerations, CodeTables, location};
use crate::token::Catcode;
use proptest::prelude::*;
use std::hash::Hasher;
use std::sync::Arc;

#[test]
fn initex_catcode_defaults_match_tex82_ascii() {
    let tables = CodeTables::new();

    assert_eq!(tables.catcode('\0'), Catcode::Ignored);
    assert_eq!(tables.catcode('\r'), Catcode::EndLine);
    assert_eq!(tables.catcode(' '), Catcode::Space);
    assert_eq!(tables.catcode('\\'), Catcode::Escape);
    assert_eq!(tables.catcode('{'), Catcode::BeginGroup);
    assert_eq!(tables.catcode('}'), Catcode::EndGroup);
    assert_eq!(tables.catcode('$'), Catcode::MathShift);
    assert_eq!(tables.catcode('&'), Catcode::AlignmentTab);
    assert_eq!(tables.catcode('#'), Catcode::Parameter);
    assert_eq!(tables.catcode('^'), Catcode::Superscript);
    assert_eq!(tables.catcode('_'), Catcode::Subscript);
    assert_eq!(tables.catcode('%'), Catcode::Comment);
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
    assert_eq!(tables.delcode('A'), -1);
}

#[test]
fn snapshot_restores_roots_and_generations() {
    let mut tables = CodeTables::new();
    let snapshot = tables.checkpoint();
    let generation = tables.generations();

    tables.set_catcode('@', Catcode::Letter);
    tables.set_lccode('@', u32::from('a'));
    tables.set_uccode('@', u32::from('A'));
    tables.set_sfcode('A', 1000);
    tables.set_mathcode('∑', 0x1350);
    tables.set_delcode('[', 0x45);

    assert_ne!(tables.generations(), generation);
    tables.rollback_to(snapshot);

    assert_eq!(tables.generations(), generation);
    assert_eq!(tables.catcode('@'), Catcode::Other);
    assert_eq!(tables.lccode('@'), 0);
    assert_eq!(tables.uccode('@'), 0);
    assert_eq!(tables.sfcode('A'), 999);
    assert_eq!(tables.mathcode('∑'), u32::from('∑'));
    assert_eq!(tables.delcode('['), -1);
}

#[test]
fn snapshots_keep_old_shared_pages_after_copy_on_write() {
    let mut tables = CodeTables::new();
    let snapshot = tables.checkpoint();

    tables.set_catcode('@', Catcode::Letter);
    assert_eq!(tables.catcode('@'), Catcode::Letter);
    let (page, _) = location('@');
    assert!(snapshot.catcodes.root.page(page).is_none());
}

#[test]
fn new_tables_share_canonical_default_roots_and_pages() {
    let first = CodeTables::new();
    let second = CodeTables::new();

    assert!(Arc::ptr_eq(&first.catcodes.root, &second.catcodes.root));
    assert_eq!(first.catcodes.root.materialized_page_count(), 0);
    assert_eq!(second.catcodes.root.materialized_page_count(), 0);
    assert!(Arc::ptr_eq(&first.lccodes.root, &second.lccodes.root));
    assert!(Arc::ptr_eq(&first.uccodes.root, &second.uccodes.root));
    assert!(Arc::ptr_eq(&first.sfcodes.root, &second.sfcodes.root));
    assert!(Arc::ptr_eq(&first.mathcodes.root, &second.mathcodes.root));
    assert!(Arc::ptr_eq(&first.delcodes.root, &second.delcodes.root));
}

#[test]
fn checkpoint_captures_root_pointers_without_cloning_root_arrays() {
    let mut tables = CodeTables::new();
    tables.set_catcode('@', Catcode::Letter);
    let snapshot = tables.checkpoint();

    assert!(Arc::ptr_eq(&tables.catcodes.root, &snapshot.catcodes.root));
    let old_root = Arc::clone(&snapshot.catcodes.root);

    tables.set_catcode('!', Catcode::Letter);

    assert!(!Arc::ptr_eq(&tables.catcodes.root, &old_root));
    assert_eq!(tables.catcode('!'), Catcode::Letter);
    let (page, _) = location('!');
    assert_eq!(
        snapshot.catcodes.root.page(page).expect("old page").values[usize::from(b'@')],
        Catcode::Letter
    );
    assert_eq!(snapshot.catcodes.root.materialized_page_count(), 1);
    assert_eq!(tables.catcodes.root.materialized_page_count(), 1);
}

#[test]
fn detached_write_copies_only_one_bounded_radix_path() {
    let mut tables = CodeTables::new();
    let first = char::from_u32(0x0100).expect("scalar");
    let second = char::from_u32(0x0200).expect("scalar");
    let distant = char::from_u32(0x1_0100).expect("scalar");
    tables.set_catcode(first, Catcode::Letter);
    tables.set_catcode(distant, Catcode::Letter);
    let snapshot = tables.checkpoint();

    let (first_page, _) = location(first);
    let (second_page, _) = location(second);
    let (distant_page, _) = location(distant);
    let (near_chunk, _) = super::page_location(first_page);
    let (distant_chunk, _) = super::page_location(distant_page);
    let old_near = Arc::clone(
        snapshot.catcodes.root.chunks[near_chunk]
            .as_ref()
            .expect("near chunk"),
    );
    let old_distant = Arc::clone(
        snapshot.catcodes.root.chunks[distant_chunk]
            .as_ref()
            .expect("distant chunk"),
    );
    let old_first_page = Arc::clone(
        old_near.pages[super::page_location(first_page).1]
            .as_ref()
            .expect("first page"),
    );

    tables.set_catcode(second, Catcode::Letter);

    let new_near = tables.catcodes.root.chunks[near_chunk]
        .as_ref()
        .expect("new near chunk");
    assert!(!Arc::ptr_eq(new_near, &old_near));
    assert!(Arc::ptr_eq(
        tables.catcodes.root.chunks[distant_chunk]
            .as_ref()
            .expect("new distant chunk"),
        &old_distant
    ));
    assert!(Arc::ptr_eq(
        new_near.pages[super::page_location(first_page).1]
            .as_ref()
            .expect("new first page"),
        &old_first_page
    ));
    assert!(tables.catcodes.root.page(second_page).is_some());
    assert_eq!(tables.catcodes.root.materialized_page_count(), 3);
}

#[test]
fn restoring_a_page_to_defaults_reuses_the_canonical_empty_root() {
    let mut tables = CodeTables::new();
    let default_root = Arc::clone(&tables.catcodes.root);
    let ch = '🦀';

    tables.set_catcode(ch, Catcode::Letter);
    assert_eq!(tables.catcodes.root.materialized_page_count(), 1);
    tables.set_catcode(ch, Catcode::Other);

    assert_eq!(tables.catcodes.root.materialized_page_count(), 0);
    assert!(Arc::ptr_eq(&tables.catcodes.root, &default_root));
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

    assert_eq!(left.generations(), right.generations());
    assert_eq!(left_hash.finish(), right_hash.finish());
}

#[test]
fn no_op_write_bumps_generation_without_copying_root() {
    let mut tables = CodeTables::new();
    let generation = tables.generations();
    let snapshot = tables.checkpoint();

    tables.set_catcode('@', Catcode::Other);

    assert_eq!(tables.generations().catcode, generation.catcode + 1);
    assert_eq!(tables.catcode('@'), Catcode::Other);
    assert!(Arc::ptr_eq(&tables.catcodes.root, &snapshot.catcodes.root));
}

#[test]
fn deep_group_global_assignment_appends_one_write_without_rewriting_frames() {
    const DEPTH: usize = 4_096;
    let mut tables = CodeTables::new();
    for _ in 0..DEPTH {
        tables.enter_group();
    }
    let saved_frames = Arc::clone(&tables.group_roots);
    let generation = tables.generations().catcode;

    tables.set_catcode_global('􏿽', Catcode::Active);

    assert!(Arc::ptr_eq(&tables.group_roots, &saved_frames));
    assert_eq!(tables.global_writes.len(), 1);
    assert_eq!(tables.generations().catcode, generation + 1);
    assert!(
        saved_frames
            .iter()
            .all(|frame| frame.catcodes.page(location('􏿽').0).is_none())
    );
    drop(saved_frames);

    for _ in 0..DEPTH {
        tables.leave_group();
    }
    assert_eq!(tables.catcode('􏿽'), Catcode::Active);
    assert_eq!(tables.global_writes.len(), 0);
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
fn same_value_global_assignment_records_activity_without_copying_roots() {
    let mut tables = CodeTables::new();
    tables.enter_group();
    let root = Arc::clone(&tables.catcodes.root);
    let generation = tables.generations().catcode;

    tables.set_catcode_global('@', Catcode::Other);

    assert_eq!(tables.global_writes.len(), 1);
    assert_eq!(tables.generations().catcode, generation + 1);
    assert!(Arc::ptr_eq(&tables.catcodes.root, &root));
    tables.leave_group();
    assert_eq!(tables.catcode('@'), Catcode::Other);
    assert_eq!(tables.generations().catcode, generation + 1);
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
    fn structural_persistence_restores_catcode_roots(
        ch in any::<char>(),
        replacement in 0_u8..=15,
    ) {
        let replacement = catcode_from_u8(replacement);
        let mut tables = CodeTables::new();
        let before = tables.catcode(ch);
        let generation = tables.generations();
        let snapshot = tables.checkpoint();

        tables.set_catcode(ch, replacement);
        prop_assert_eq!(
            tables.generations().catcode,
            generation.catcode + 1
        );

        tables.rollback_to(snapshot);
        prop_assert_eq!(tables.catcode(ch), before);
        prop_assert_eq!(tables.generations(), generation);
    }

    #[test]
    fn generation_bumps_once_per_code_table_write(
        ch in any::<char>(),
        lc in 0_u32..0x11_0000,
        uc in 0_u32..0x11_0000,
        sf in any::<u16>(),
        math in 0_u32..0x80_0000,
        del in -1_i32..0x80_0000,
    ) {
        let mut tables = CodeTables::new();
        let before = tables.generations();
        let expected = CodeTableGenerations {
            catcode: before.catcode,
            lccode: before.lccode + 1,
            uccode: before.uccode + 1,
            sfcode: before.sfcode + 1,
            mathcode: before.mathcode + 1,
            delcode: before.delcode + 1,
        };

        tables.set_lccode(ch, lc);
        tables.set_uccode(ch, uc);
        tables.set_sfcode(ch, sf);
        tables.set_mathcode(ch, math);
        tables.set_delcode(ch, del);

        prop_assert_eq!(tables.generations(), expected);
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
