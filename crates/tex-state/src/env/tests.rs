use super::{Env, SEGMENT_LEN, font_dimen_index};
use crate::GroupKind;
use crate::cell::{BankTag, CellId};
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
use crate::interner::Symbol;
use crate::journal::{Entry, UndoRec};
use crate::meaning::Meaning;
use crate::scaled::Scaled;
use crate::token::{Catcode, Token};
use ahash::AHashMap;

#[test]
fn default_get_before_any_set_is_undefined() {
    let env = Env::new();

    assert_eq!(env.get(Symbol::new(10)), Meaning::Undefined);
}

#[test]
fn fontdimen_key_codec_is_injective_at_both_field_boundaries() {
    use crate::font::{MAX_FONT_DIMEN, MAX_FONT_DIMEN_FONT_ID};
    use crate::stores::FontParameterError;

    let first = font_dimen_index(FontId::new(0), 1).expect("first fontdimen key");
    let last_slot =
        font_dimen_index(FontId::new(0), MAX_FONT_DIMEN).expect("last slot of first font");
    let next_font = font_dimen_index(FontId::new(1), 1).expect("first slot of next font");
    let last = font_dimen_index(FontId::new(MAX_FONT_DIMEN_FONT_ID), MAX_FONT_DIMEN)
        .expect("last representable fontdimen key");

    assert_eq!(first, 0);
    assert_eq!(last_slot + 1, next_font);
    assert_eq!(last, u32::MAX);
    assert!(matches!(
        font_dimen_index(FontId::new(0), MAX_FONT_DIMEN + 1),
        Err(FontParameterError::NumberOutOfRange { .. })
    ));
    assert!(matches!(
        font_dimen_index(FontId::new(MAX_FONT_DIMEN_FONT_ID + 1), 1),
        Err(FontParameterError::FontOutOfRange { .. })
    ));
}

#[test]
fn first_write_per_epoch_coalesces_and_keeps_first_new_value() {
    let mut env = Env::new();
    let symbol = Symbol::new(3);
    let start = env.journal_pos();

    env.set(symbol, Meaning::Relax);
    env.set(symbol, Meaning::CharGiven('x'));

    assert_eq!(env.get(symbol), Meaning::CharGiven('x'));
    let entries = env.journal_entries_since(start);
    assert_eq!(
        entries,
        &[Entry::Undo(UndoRec::new(
            CellId::new(BankTag::Meaning, 3),
            Meaning::Undefined.encode(),
            Meaning::Relax.encode(),
        ))]
    );
}

#[test]
fn write_in_later_epoch_records_again() {
    let mut env = Env::new();
    let symbol = Symbol::new(8);
    let start = env.journal_pos();

    env.set(symbol, Meaning::Relax);
    env.bump_epoch();
    env.set(symbol, Meaning::CharGiven('y'));

    let entries = env.journal_entries_since(start);
    assert_eq!(
        entries,
        &[
            Entry::Undo(UndoRec::new(
                CellId::new(BankTag::Meaning, 8),
                Meaning::Undefined.encode(),
                Meaning::Relax.encode(),
            )),
            Entry::Undo(UndoRec::new(
                CellId::new(BankTag::Meaning, 8),
                Meaning::Relax.encode(),
                Meaning::CharGiven('y').encode(),
            )),
        ]
    );
}

#[test]
fn global_set_tags_cell_id_in_journal() {
    let mut env = Env::new();
    let symbol = Symbol::new(4);
    let start = env.journal_pos();

    env.set_global(symbol, Meaning::Relax);

    assert_eq!(
        env.journal_entries_since(start),
        &[Entry::Undo(UndoRec::new(
            CellId::new_global(BankTag::Meaning, 4),
            Meaning::Undefined.encode(),
            Meaning::Relax.encode(),
        ))]
    );
}

#[test]
fn segment_growth_keeps_earlier_segment_addresses_stable() {
    let mut env = Env::new();
    let first = Symbol::new(0);
    let second_segment = Symbol::new(SEGMENT_LEN as u32);

    env.set(first, Meaning::Relax);
    let cells_ptr = env.meaning_cells[0]
        .as_ref()
        .expect("first meaning segment")
        .as_ptr();
    let stamps_ptr = env.meaning_stamps[0]
        .as_ref()
        .expect("first stamp segment")
        .as_ptr();

    env.set(second_segment, Meaning::CharGiven('z'));

    assert_eq!(
        env.meaning_cells[0]
            .as_ref()
            .expect("first meaning segment")
            .as_ptr(),
        cells_ptr
    );
    assert_eq!(
        env.meaning_stamps[0]
            .as_ref()
            .expect("first stamp segment")
            .as_ptr(),
        stamps_ptr
    );
    assert_eq!(env.get(first), Meaning::Relax);
    assert_eq!(env.get(second_segment), Meaning::CharGiven('z'));
}

#[test]
fn meaning_boundaries_above_26_bits_preserve_journal_group_and_hash_semantics() {
    for index in [1 << 26, (1 << 30) - 1] {
        let mut env = Env::new();
        let symbol = Symbol::new(index);
        let initial_hash = env.testing_state_hash();

        env.enter_group();
        let group_start = env.journal_pos();
        env.set(symbol, Meaning::Relax);
        assert_eq!(
            env.journal_entries_since(group_start),
            &[Entry::Undo(UndoRec::new(
                CellId::new(BankTag::Meaning, index),
                Meaning::Undefined.encode(),
                Meaning::Relax.encode(),
            ))]
        );
        assert_ne!(env.testing_state_hash(), initial_hash);
        assert_eq!(env.leave_group(), Vec::<Token>::new());
        assert_eq!(env.get(symbol), Meaning::Undefined);
        assert_eq!(env.testing_state_hash(), initial_hash);

        let checkpoint = env.checkpoint();
        env.enter_group();
        env.set_global(symbol, Meaning::CharGiven('x'));
        assert_eq!(env.leave_group(), Vec::<Token>::new());
        assert_eq!(env.get(symbol), Meaning::CharGiven('x'));
        assert_ne!(env.testing_state_hash(), initial_hash);
        env.rollback_to(checkpoint);
        assert_eq!(env.get(symbol), Meaning::Undefined);
        assert_eq!(env.testing_state_hash(), initial_hash);

        let cell = CellId::new(BankTag::Meaning, index);
        env.restore_raw(cell, Meaning::Relax.encode());
        assert_eq!(env.get(symbol), Meaning::Relax);
        assert_ne!(env.testing_state_hash(), initial_hash);
        env.restore_raw(cell, Meaning::Undefined.encode());
        assert_eq!(env.testing_state_hash(), initial_hash);
    }
}

#[test]
fn cached_group_boundaries_survive_deep_journals_clone_and_rollback() {
    let mut env = Env::new();
    env.enter_group_with_kind(GroupKind::Simple);
    let outer_marker = env.last_group_marker_pos();
    for index in 0..10_000 {
        env.set(Symbol::new(index), Meaning::Relax);
    }
    assert_eq!(env.innermost_group_kind(), Some(GroupKind::Simple));
    assert_eq!(env.last_group_marker_pos(), outer_marker);

    let checkpoint = env.checkpoint();
    env.enter_group_with_kind(GroupKind::Align);
    env.set(Symbol::new(20_000), Meaning::CharGiven('x'));
    assert_eq!(env.innermost_group_kind(), Some(GroupKind::Align));

    let mut fork = env.clone();
    assert_eq!(fork.leave_group_with_kind(GroupKind::Align), Ok(Vec::new()));
    assert_eq!(fork.innermost_group_kind(), Some(GroupKind::Simple));
    assert_eq!(env.innermost_group_kind(), Some(GroupKind::Align));

    env.rollback_to(checkpoint);
    assert_eq!(env.innermost_group_kind(), Some(GroupKind::Simple));
    assert_eq!(env.last_group_marker_pos(), outer_marker);
    assert_eq!(env.group_boundaries.len(), env.group_depth as usize);
    assert_eq!(env.leave_group_with_kind(GroupKind::Simple), Ok(Vec::new()));
    assert_eq!(env.innermost_group_kind(), None);
    assert!(env.group_boundaries.is_empty());
}

#[test]
fn box_slots_restore_owner_and_value_across_nested_local_and_global_writes() {
    for index in [7, 300] {
        let root = NodeListId::testing_survivor(1, 1, 0);
        let outer = NodeListId::testing_survivor(2, 1, 0);
        let inner = NodeListId::testing_survivor(3, 1, 0);
        let global = NodeListId::testing_survivor(4, 1, 0);
        let mut env = Env::new();
        env.set_box_reg_global(index, Some(root));

        env.enter_group();
        env.set_box_reg(index, Some(outer));
        assert!(env.box_reg_is_local_to_current_group(index));
        env.enter_group();
        env.set_box_reg(index, Some(inner));
        assert!(env.box_reg_is_local_to_current_group(index));
        let _ = env.leave_group();
        assert_eq!(env.box_reg(index), Some(outer));
        assert!(env.box_reg_is_local_to_current_group(index));

        env.enter_group();
        env.set_box_reg_global(index, Some(global));
        assert!(!env.box_reg_is_local_to_current_group(index));
        let _ = env.leave_group();
        assert_eq!(env.box_reg(index), Some(global));
        let _ = env.leave_group();
        assert_eq!(env.box_reg(index), Some(global));
    }
}

#[test]
fn box_checkpoint_rollback_restores_coalescing_cursor_for_same_depth_reuse() {
    let mut env = Env::new();
    let first = NodeListId::testing_survivor(1, 1, 0);
    let discarded = NodeListId::testing_survivor(2, 1, 0);
    let replacement = NodeListId::testing_survivor(3, 1, 0);
    env.enter_group();
    env.set_box_reg(9, Some(first));
    let snapshot = env.checkpoint();
    env.set_box_reg(9, Some(discarded));
    env.rollback_to(snapshot);
    assert_eq!(env.box_reg(9), Some(first));
    assert!(env.box_reg_is_local_to_current_group(9));
    env.set_box_reg(9, Some(replacement));
    assert_eq!(env.box_reg(9), Some(replacement));
    let _ = env.leave_group();
    assert_eq!(env.box_reg(9), None);
}

#[test]
fn cloned_box_slots_and_journals_diverge_without_cross_timeline_cursors() {
    let mut env = Env::new();
    env.enter_group();
    env.set_box_reg(300, Some(NodeListId::testing_survivor(1, 1, 0)));
    let mut fork = env.clone();
    fork.set_box_reg(300, Some(NodeListId::testing_survivor(2, 1, 0)));
    env.set_box_reg(300, Some(NodeListId::testing_survivor(3, 1, 0)));
    assert_ne!(env.box_reg(300), fork.box_reg(300));
    let _ = env.leave_group();
    let _ = fork.leave_group();
    assert_eq!(env.box_reg(300), None);
    assert_eq!(fork.box_reg(300), None);
}

#[test]
fn dense_register_typed_api_round_trips_boundary_and_signed_values() {
    let mut env = Env::new();

    env.set_count(255, i32::MIN);
    env.set_dimen(255, Scaled::MIN);
    env.set_skip(255, GlueId::new(u32::MAX));
    env.set_muskip(255, GlueId::new(u32::MAX - 3));
    env.set_toks(255, TokenListId::new(u32::MAX - 1));
    env.set_box_reg(255, Some(NodeListId::testing_survivor(7, 3, 0)));

    assert_eq!(env.count(255), i32::MIN);
    assert_eq!(env.dimen(255), Scaled::MIN);
    assert_eq!(env.skip(255), GlueId::new(u32::MAX));
    assert_eq!(env.muskip(255), GlueId::new(u32::MAX - 3));
    assert_eq!(env.toks(255), TokenListId::new(u32::MAX - 1));
    assert_eq!(
        env.box_reg(255),
        Some(NodeListId::testing_survivor(7, 3, 0))
    );
}

#[test]
fn dense_register_journal_records_use_bank_tags_and_encoded_words() {
    let mut env = Env::new();
    let start = env.journal_pos();

    env.set_count(1, -1);
    env.set_dimen(2, Scaled::from_raw(-2));
    env.set_skip(3, GlueId::new(33));
    env.set_muskip(4, GlueId::new(34));
    env.set_toks(5, TokenListId::new(44));
    env.set_box_reg(6, Some(NodeListId::testing_survivor(8, 55, 0)));

    let entries = env.journal_entries_since(start);
    assert_eq!(
        &entries[..5],
        &[
            undo(BankTag::Count, 1, 0, u64::from((-1_i32) as u32)),
            undo(BankTag::Dimen, 2, 0, u64::from((-2_i32) as u32)),
            undo(BankTag::Skip, 3, 0, 33),
            undo(BankTag::Muskip, 4, 0, 34),
            undo(BankTag::Toks, 5, 0, 44),
        ]
    );
    let Entry::BoxUndo(id) = entries[5] else {
        panic!("box write must use the specialized journal arena");
    };
    let rec = env.box_undo(id);
    assert_eq!(rec.index(), 6);
    assert!(!rec.is_global());
    assert_eq!(rec.old().value(), u64::MAX);
    assert_eq!(
        rec.new_value().value(),
        NodeListId::encode_box_word(Some(NodeListId::testing_survivor(8, 55, 0)))
    );
}

#[test]
fn dense_register_global_sets_tag_journal_records() {
    let mut env = Env::new();
    let start = env.journal_pos();

    env.set_count_global(255, 7);

    assert_eq!(
        env.journal_entries_since(start),
        &[Entry::Undo(UndoRec::new(
            CellId::new_global(BankTag::Count, 255),
            0,
            7,
        ))]
    );
}

#[test]
fn parameter_typed_api_round_trips_values() {
    let mut env = Env::new();

    env.set_int_param(IntParam::new(127), i32::MIN);
    env.set_dimen_param(DimenParam::new(127), Scaled::MIN);
    env.set_glue_param(GlueParam::new(127), GlueId::new(77));
    env.set_tok_param(TokParam::new(127), TokenListId::new(88));

    assert_eq!(env.int_param(IntParam::new(127)), i32::MIN);
    assert_eq!(env.dimen_param(DimenParam::new(127)), Scaled::MIN);
    assert_eq!(env.glue_param(GlueParam::new(127)), GlueId::new(77));
    assert_eq!(env.tok_param(TokParam::new(127)), TokenListId::new(88));
}

#[test]
fn parameter_journal_records_use_parameter_bank_tags() {
    let mut env = Env::new();
    let start = env.journal_pos();

    env.set_int_param(IntParam::new(1), -9);
    env.set_dimen_param(DimenParam::new(2), Scaled::from_raw(-10));
    env.set_glue_param(GlueParam::new(3), GlueId::new(90));
    env.set_tok_param(TokParam::new(4), TokenListId::new(100));

    assert_eq!(
        env.journal_entries_since(start),
        &[
            undo(BankTag::IntParam, 1, 0, u64::from((-9_i32) as u32)),
            undo(BankTag::DimenParam, 2, 0, u64::from((-10_i32) as u32)),
            undo(BankTag::GlueParam, 3, 0, 90),
            undo(BankTag::TokParam, 4, 0, 100),
        ]
    );
}

#[test]
fn parameter_global_sets_tag_journal_records() {
    let mut env = Env::new();
    let start = env.journal_pos();

    env.set_tok_param_global(TokParam::new(7), TokenListId::new(11));

    assert_eq!(
        env.journal_entries_since(start),
        &[Entry::Undo(UndoRec::new(
            CellId::new_global(BankTag::TokParam, 7),
            0,
            11,
        ))]
    );
}

#[test]
fn sparse_read_before_write_returns_default_without_allocating_page() {
    let env = Env::new();

    assert_eq!(env.count(300), 0);
    assert!(!env.overflow_counts.has_page_for(300));
    assert_eq!(env.box_reg(300), None);
    assert!(!env.boxes.has_page_for(300));
}

#[test]
fn sparse_registers_round_trip_boundaries() {
    let mut env = Env::new();

    env.set_count(256, 1);
    env.set_count(511, 2);
    env.set_count(512, 3);
    env.set_count(32_767, 4);

    assert_eq!(env.count(256), 1);
    assert_eq!(env.count(511), 2);
    assert_eq!(env.count(512), 3);
    assert_eq!(env.count(32_767), 4);
}

#[test]
fn sparse_journal_records_use_absolute_register_indices() {
    let mut env = Env::new();
    let start = env.journal_pos();

    env.set_count(256, -1);
    env.set_dimen(32_767, Scaled::from_raw(-2));
    env.set_muskip(300, GlueId::new(99));

    assert_eq!(
        env.journal_entries_since(start),
        &[
            undo(BankTag::Count, 256, 0, u64::from((-1_i32) as u32)),
            undo(BankTag::Dimen, 32_767, 0, u64::from((-2_i32) as u32)),
            undo(BankTag::Muskip, 300, 0, 99),
        ]
    );
}

#[test]
fn sparse_global_write_tags_absolute_register_cell_id() {
    let mut env = Env::new();
    let start = env.journal_pos();

    env.set_count_global(300, 7);

    assert_eq!(
        env.journal_entries_since(start),
        &[Entry::Undo(UndoRec::new(
            CellId::new_global(BankTag::Count, 300),
            0,
            7,
        ))]
    );
}

#[test]
fn restore_raw_routes_sparse_journal_records_without_journaling() {
    let mut env = Env::new();
    env.set_count(300, 11);
    env.bump_epoch();
    let start = env.journal_pos();
    env.set_count(300, 22);
    let rec = match env.journal_entries_since(start) {
        [Entry::Undo(rec)] => *rec,
        entries => panic!("expected one sparse undo record, got {entries:?}"),
    };
    let before_restore = env.journal_pos();

    env.restore_raw(rec.cell(), rec.old());

    assert_eq!(env.count(300), 11);
    assert_eq!(env.journal_pos(), before_restore);
    assert!(env.journal_entries_since(before_restore).is_empty());
}

#[test]
fn restore_raw_routes_dense_parameters_and_meanings() {
    let mut env = Env::new();
    let symbol = Symbol::new(9);
    let param = IntParam::new(3);
    env.set(symbol, Meaning::Relax);
    env.set_count(7, 10);
    env.set_int_param(param, 20);
    let before_restore = env.journal_pos();

    env.restore_raw(
        CellId::new(BankTag::Meaning, 9),
        Meaning::Undefined.encode(),
    );
    env.restore_raw(CellId::new(BankTag::Count, 7), 1);
    env.restore_raw(CellId::new(BankTag::IntParam, 3), 2);

    assert_eq!(env.get(symbol), Meaning::Undefined);
    assert_eq!(env.count(7), 1);
    assert_eq!(env.int_param(param), 2);
    assert_eq!(env.journal_pos(), before_restore);
}

#[test]
fn sparse_register_classes_are_independent() {
    let mut env = Env::new();

    env.set_count(300, 123);
    env.set_dimen(300, Scaled::from_raw(456));
    env.set_skip(300, GlueId::new(7));
    env.set_muskip(300, GlueId::new(8));

    assert_eq!(env.count(300), 123);
    assert_eq!(env.dimen(300), Scaled::from_raw(456));
    assert_eq!(env.skip(300), GlueId::new(7));
    assert_eq!(env.muskip(300), GlueId::new(8));
    assert!(env.overflow_counts.has_page_for(300));
    assert!(env.overflow_dimens.has_page_for(300));
    assert!(env.overflow_skips.has_page_for(300));
    assert!(env.overflow_muskips.has_page_for(300));
}

#[test]
fn nested_groups_follow_naive_oracle_three_deep() {
    let mut env = Env::new();
    let mut oracle = Oracle::new();

    oracle.set_local(1, 10);
    env.set_count(1, 10);
    assert_oracle(&env, &oracle, &[1, 2, 300]);

    env.enter_group();
    oracle.enter_group();
    oracle.set_local(1, 11);
    oracle.set_local(2, 20);
    env.set_count(1, 11);
    env.set_count(2, 20);
    assert_oracle(&env, &oracle, &[1, 2, 300]);

    env.enter_group();
    oracle.enter_group();
    oracle.set_local(1, 12);
    oracle.set_global(2, 21);
    env.set_count(1, 12);
    env.set_count_global(2, 21);
    assert_oracle(&env, &oracle, &[1, 2, 300]);

    env.enter_group();
    oracle.enter_group();
    oracle.set_local(300, 30);
    env.set_count(300, 30);
    assert_oracle(&env, &oracle, &[1, 2, 300]);

    assert_eq!(env.leave_group(), Vec::<Token>::new());
    oracle.leave_group();
    assert_oracle(&env, &oracle, &[1, 2, 300]);

    assert_eq!(env.leave_group(), Vec::<Token>::new());
    oracle.leave_group();
    assert_oracle(&env, &oracle, &[1, 2, 300]);

    assert_eq!(env.leave_group(), Vec::<Token>::new());
    oracle.leave_group();
    assert_oracle(&env, &oracle, &[1, 2, 300]);
}

#[test]
fn local_write_shadowed_by_global_same_cell_survives_group_exit() {
    let mut env = Env::new();
    let mut oracle = Oracle::new();

    env.enter_group();
    oracle.enter_group();
    env.set_count(7, 1);
    oracle.set_local(7, 1);
    env.set_count_global(7, 2);
    oracle.set_global(7, 2);

    assert_oracle(&env, &oracle, &[7]);
    assert_eq!(env.leave_group(), Vec::<Token>::new());
    oracle.leave_group();

    assert_oracle(&env, &oracle, &[7]);
    assert_eq!(env.count(7), 2);
}

#[test]
fn global_then_local_same_cell_local_wins_inside_global_after_exit() {
    let mut env = Env::new();
    let mut oracle = Oracle::new();

    env.enter_group();
    oracle.enter_group();
    env.set_count_global(9, 5);
    oracle.set_global(9, 5);
    env.set_count(9, 6);
    oracle.set_local(9, 6);

    assert_eq!(env.count(9), 6);
    assert_oracle(&env, &oracle, &[9]);

    assert_eq!(env.leave_group(), Vec::<Token>::new());
    oracle.leave_group();

    assert_eq!(env.count(9), 5);
    assert_oracle(&env, &oracle, &[9]);
}

#[test]
fn repeated_same_epoch_globals_keep_last_global_after_exit() {
    let mut env = Env::new();

    env.enter_group();
    env.set_count_global(10, 1);
    env.set_count_global(10, 2);
    env.set_count(10, 3);

    assert_eq!(env.count(10), 3);
    assert_eq!(env.leave_group(), Vec::<Token>::new());
    assert_eq!(env.count(10), 2);
}

#[test]
fn local_noop_does_not_consume_first_write_for_epoch() {
    let mut env = Env::new();
    let pos = env.checkpoint();

    env.set_count(12, 0);
    env.set_count(12, 1);
    env.rollback_to(pos);

    assert_eq!(env.count(12), 0);
}

#[test]
fn compacted_global_after_local_rolls_back_to_pre_group_value() {
    let mut env = Env::new();
    let pos = env.checkpoint();

    env.enter_group();
    env.set_count(12, 1);
    env.set_count_global(12, 0);
    assert_eq!(env.leave_group(), Vec::<Token>::new());

    assert_eq!(env.count(12), 0);
    env.rollback_to(pos);
    assert_eq!(env.count(12), 0);
}

#[test]
fn same_value_global_after_local_still_survives_group_exit() {
    let mut env = Env::new();

    env.enter_group();
    env.set_count(12, 1);
    env.set_count_global(12, 1);
    assert_eq!(env.leave_group(), Vec::<Token>::new());

    assert_eq!(env.count(12), 1);
}

#[test]
fn large_local_only_group_exit_restores_without_compaction_records() {
    let mut env = Env::new();
    let start = env.checkpoint();

    env.enter_group();
    for index in 0..1024_u16 {
        env.set_count(index, i32::from(index) + 1);
    }

    assert_eq!(env.leave_group(), Vec::<Token>::new());

    for index in 0..1024_u16 {
        assert_eq!(env.count(index), 0, "count register {index}");
    }
    assert!(env.journal_entries_since(start.journal_pos()).is_empty());
}

#[test]
fn mixed_global_local_same_cell_compacts_first_old_for_rollback() {
    let mut env = Env::new();
    env.set_count(7, 70);
    let start = env.checkpoint();

    env.enter_group();
    env.set_count(7, 71);
    env.set_count_global(7, 72);
    env.set_count(7, 73);
    env.set_count_global(7, 74);

    assert_eq!(env.leave_group(), Vec::<Token>::new());

    assert_eq!(env.count(7), 74);
    assert_eq!(
        env.journal_entries_since(start.journal_pos()),
        &[
            Entry::Undo(UndoRec::new(CellId::new_global(BankTag::Count, 7), 70, 72,)),
            Entry::Undo(UndoRec::new(CellId::new_global(BankTag::Count, 7), 73, 74,)),
        ]
    );

    env.rollback_to(start);
    assert_eq!(env.count(7), 70);
}

#[test]
fn aftergroup_payloads_are_fifo_per_group_across_nesting() {
    let mut env = Env::new();
    let one = Token::Char {
        ch: '1',
        cat: Catcode::Other,
    };
    let two = Token::Char {
        ch: '2',
        cat: Catcode::Other,
    };
    let three = Token::Char {
        ch: '3',
        cat: Catcode::Other,
    };
    let four = Token::Char {
        ch: '4',
        cat: Catcode::Other,
    };

    env.enter_group();
    env.push_aftergroup(one);
    env.enter_group();
    env.push_aftergroup(two);
    env.push_aftergroup(three);

    assert_eq!(env.leave_group(), vec![two, three]);

    env.push_aftergroup(four);
    assert_eq!(env.leave_group(), vec![one, four]);
}

#[test]
fn sparse_register_local_restores_on_group_exit() {
    let mut env = Env::new();
    let mut oracle = Oracle::new();

    env.set_count(300, 100);
    oracle.set_local(300, 100);
    env.enter_group();
    oracle.enter_group();
    env.set_count(300, 200);
    oracle.set_local(300, 200);
    assert_oracle(&env, &oracle, &[300]);

    assert_eq!(env.leave_group(), Vec::<Token>::new());
    oracle.leave_group();

    assert_oracle(&env, &oracle, &[300]);
    assert_eq!(env.count(300), 100);
}

#[test]
fn sparse_first_write_group_exit_prunes_restored_default_page() {
    let mut env = Env::new();

    env.enter_group();
    env.set_count(300, 100);
    assert_eq!(env.count(300), 100);
    assert!(env.overflow_counts.has_page_for(300));

    assert_eq!(env.leave_group(), Vec::<Token>::new());

    assert_eq!(env.count(300), 0);
    assert!(!env.overflow_counts.has_page_for(300));
}

#[test]
fn sparse_first_write_rollback_prunes_restored_default_page() {
    let mut env = Env::new();
    let pos = env.checkpoint();

    env.set_count(300, 100);
    assert_eq!(env.count(300), 100);
    assert!(env.overflow_counts.has_page_for(300));

    env.rollback_to(pos);

    assert_eq!(env.count(300), 0);
    assert!(!env.overflow_counts.has_page_for(300));
}

#[test]
fn group_exit_bumps_epoch_so_outer_undo_slice_records_rewrite() {
    let mut env = Env::new();

    env.enter_group();
    let outer_pos = env.checkpoint();
    env.enter_group();
    env.set_count(11, 1);
    assert_eq!(env.leave_group(), Vec::<Token>::new());

    // Regression for core_state.md §6 / 97a3c1d: without the group-exit epoch
    // bump, this write sees the restored cell's high stamp and skips journaling,
    // so the enclosing rollback would fail to restore the pre-inner value.
    env.set_count(11, 2);
    env.rollback_to(outer_pos);

    assert_eq!(env.count(11), 0);
}

#[test]
fn count_int_fingerprint_is_lazy_and_restores_after_local_group() {
    let mut env = Env::new();
    let initial = env.count_int_fingerprint();
    assert_eq!(env.count_int_fingerprint(), initial);

    env.enter_group();
    env.set_count(300, -17);
    env.set_int_param(IntParam::GLOBAL_DEFS, 1);
    assert_ne!(env.count_int_fingerprint(), initial);
    assert_eq!(env.leave_group(), Vec::<Token>::new());

    assert_eq!(env.count_int_fingerprint(), initial);
    env.set_count_global(3, 42);
    assert_ne!(env.count_int_fingerprint(), initial);
}

#[test]
fn paragraph_mutations_keep_only_compacted_root_survivors() {
    let mut env = Env::new();
    let checkpoint = env.begin_paragraph_mutations();

    env.enter_group();
    env.set_count(7, 99);
    env.set_int_param_global(IntParam::GLOBAL_DEFS, 2);
    assert_eq!(env.leave_group(), Vec::<Token>::new());
    env.set_count(300, -17);

    let summary = env.finish_paragraph_mutations(checkpoint);
    assert!(!summary.journal_rewound);
    assert_ne!(summary.entry_fingerprint, summary.exit_fingerprint);
    assert_eq!(
        summary.mutations,
        vec![
            crate::PureParagraphMutation::IntParam {
                param: IntParam::GLOBAL_DEFS,
                expected: 0,
                value: 2,
                global: true,
            },
            crate::PureParagraphMutation::Count {
                index: 300,
                expected: 0,
                value: -17,
                global: false,
            },
        ]
    );
    assert_eq!(env.count(7), 0, "balanced local write must not escape");
}

#[test]
fn rollback_to_restores_globals_across_group_markers() {
    let mut env = Env::new();
    let pos = env.checkpoint();

    env.enter_group();
    env.set_count_global(1, 10);
    env.enter_group();
    env.set_count_global(300, 20);
    env.set_count(2, 30);

    env.rollback_to(pos);

    assert_eq!(env.count(1), 0);
    assert_eq!(env.count(2), 0);
    assert_eq!(env.count(300), 0);
    assert!(env.journal_entries_since(pos.journal_pos()).is_empty());
}

fn undo(bank: BankTag, index: u32, old: u64, new: u64) -> Entry {
    Entry::Undo(UndoRec::new(CellId::new(bank, index), old, new))
}

#[derive(Debug)]
struct Oracle {
    scopes: Vec<AHashMap<u16, i32>>,
}

impl Oracle {
    fn new() -> Self {
        Self {
            scopes: vec![AHashMap::new()],
        }
    }

    fn enter_group(&mut self) {
        self.scopes.push(AHashMap::new());
    }

    fn leave_group(&mut self) {
        assert!(self.scopes.len() > 1, "oracle group underflow");
        self.scopes.pop();
    }

    fn set_local(&mut self, index: u16, value: i32) {
        self.scopes
            .last_mut()
            .expect("oracle always has a root scope")
            .insert(index, value);
    }

    fn set_global(&mut self, index: u16, value: i32) {
        for scope in &mut self.scopes {
            scope.insert(index, value);
        }
    }

    fn get(&self, index: u16) -> i32 {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&index).copied())
            .unwrap_or(0)
    }
}

fn assert_oracle(env: &Env, oracle: &Oracle, indices: &[u16]) {
    for &index in indices {
        assert_eq!(
            env.count(index),
            oracle.get(index),
            "count register {index}"
        );
    }
}
