use super::{Env, SEGMENT_LEN};
use crate::cell::{BankTag, CellId};
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::ids::{GlueId, NodeListId, TokenListId};
use crate::interner::Symbol;
use crate::journal::{Entry, UndoRec};
use crate::meaning::Meaning;
use crate::scaled::Scaled;
use std::collections::HashMap;

#[test]
fn default_get_before_any_set_is_undefined() {
    let env = Env::new();

    assert_eq!(env.get(Symbol::new(10)), Meaning::Undefined);
}

#[test]
fn first_write_per_epoch_coalesces_and_keeps_first_new_value() {
    let mut env = Env::new();
    let symbol = Symbol::new(3);
    let start = env.journal_pos();

    env.set(symbol, Meaning::Relax);
    env.set(symbol, Meaning::CharGiven('x'));

    assert_eq!(env.get(symbol), Meaning::CharGiven('x'));
    assert_eq!(
        env.journal_entries_since(start),
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

    assert_eq!(
        env.journal_entries_since(start),
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
    let cells_ptr = env.meaning_cells[0].as_ptr();
    let stamps_ptr = env.meaning_stamps[0].as_ptr();

    env.set(second_segment, Meaning::CharGiven('z'));

    assert_eq!(env.meaning_cells[0].as_ptr(), cells_ptr);
    assert_eq!(env.meaning_stamps[0].as_ptr(), stamps_ptr);
    assert_eq!(env.get(first), Meaning::Relax);
    assert_eq!(env.get(second_segment), Meaning::CharGiven('z'));
}

#[test]
fn dense_register_typed_api_round_trips_boundary_and_signed_values() {
    let mut env = Env::new();

    env.set_count(255, i32::MIN);
    env.set_dimen(255, Scaled::MIN);
    env.set_skip(255, GlueId::new(u32::MAX));
    env.set_toks(255, TokenListId::new(u32::MAX - 1));
    env.set_box_reg(255, NodeListId::new(u32::MAX - 2));

    assert_eq!(env.count(255), i32::MIN);
    assert_eq!(env.dimen(255), Scaled::MIN);
    assert_eq!(env.skip(255), GlueId::new(u32::MAX));
    assert_eq!(env.toks(255), TokenListId::new(u32::MAX - 1));
    assert_eq!(env.box_reg(255), NodeListId::new(u32::MAX - 2));
}

#[test]
fn dense_register_journal_records_use_bank_tags_and_encoded_words() {
    let mut env = Env::new();
    let start = env.journal_pos();

    env.set_count(1, -1);
    env.set_dimen(2, Scaled::from_raw(-2));
    env.set_skip(3, GlueId::new(33));
    env.set_toks(4, TokenListId::new(44));
    env.set_box_reg(5, NodeListId::new(55));

    assert_eq!(
        env.journal_entries_since(start),
        &[
            undo(BankTag::Count, 1, 0, u64::from((-1_i32) as u32)),
            undo(BankTag::Dimen, 2, 0, u64::from((-2_i32) as u32)),
            undo(BankTag::Skip, 3, 0, 33),
            undo(BankTag::Toks, 4, 0, 44),
            undo(BankTag::Box, 5, 0, 55),
        ]
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

    assert_eq!(
        env.journal_entries_since(start),
        &[
            undo(BankTag::Count, 256, 0, u64::from((-1_i32) as u32)),
            undo(BankTag::Dimen, 32_767, 0, u64::from((-2_i32) as u32)),
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

    assert_eq!(env.count(300), 123);
    assert_eq!(env.dimen(300), Scaled::from_raw(456));
    assert!(env.overflow_counts.has_page_for(300));
    assert!(env.overflow_dimens.has_page_for(300));
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

    assert_eq!(env.leave_group(), Vec::<u64>::new());
    oracle.leave_group();
    assert_oracle(&env, &oracle, &[1, 2, 300]);

    assert_eq!(env.leave_group(), Vec::<u64>::new());
    oracle.leave_group();
    assert_oracle(&env, &oracle, &[1, 2, 300]);

    assert_eq!(env.leave_group(), Vec::<u64>::new());
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
    assert_eq!(env.leave_group(), Vec::<u64>::new());
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

    assert_eq!(env.leave_group(), Vec::<u64>::new());
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
    assert_eq!(env.leave_group(), Vec::<u64>::new());
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
    assert_eq!(env.leave_group(), Vec::<u64>::new());

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
    assert_eq!(env.leave_group(), Vec::<u64>::new());

    assert_eq!(env.count(12), 1);
}

#[test]
fn aftergroup_payloads_are_fifo_per_group_across_nesting() {
    let mut env = Env::new();

    env.enter_group();
    env.push_aftergroup(1);
    env.enter_group();
    env.push_aftergroup(2);
    env.push_aftergroup(3);

    assert_eq!(env.leave_group(), vec![2, 3]);

    env.push_aftergroup(4);
    assert_eq!(env.leave_group(), vec![1, 4]);
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

    assert_eq!(env.leave_group(), Vec::<u64>::new());
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

    assert_eq!(env.leave_group(), Vec::<u64>::new());

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
    assert_eq!(env.leave_group(), Vec::<u64>::new());

    // Regression for core_state.md §6 / 97a3c1d: without the group-exit epoch
    // bump, this write sees the restored cell's high stamp and skips journaling,
    // so the enclosing rollback would fail to restore the pre-inner value.
    env.set_count(11, 2);
    env.rollback_to(outer_pos);

    assert_eq!(env.count(11), 0);
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
    scopes: Vec<HashMap<u16, i32>>,
}

impl Oracle {
    fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    fn enter_group(&mut self) {
        self.scopes.push(HashMap::new());
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
