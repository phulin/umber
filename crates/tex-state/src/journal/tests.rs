use super::{BoxUndoRec, Entry, Journal, Marker, UndoRec};
use crate::cell::{BankTag, CellId};
use crate::env::box_bank::BoxSlot;
use crate::env::group::GroupKind;
use crate::ids::SnapshotId;
use crate::meaning::Meaning;

#[test]
fn push_pos_slice_and_truncate_round_trip() {
    let first = UndoRec::new(CellId::new(BankTag::Meaning, 1), 10, 20);
    let second = UndoRec::new(CellId::new_global(BankTag::Count, 2), 30, 40);
    let mut journal = Journal::new();

    let start = journal.pos();
    journal.push_undo(first);
    let after_first = journal.pos();
    journal.push_marker(Marker::Checkpoint(SnapshotId::new(7)));
    journal.push_undo(second);

    assert_eq!(
        journal.entries_since(start),
        &[
            Entry::Undo(first),
            Entry::Marker(Marker::Checkpoint(SnapshotId::new(7))),
            Entry::Undo(second),
        ]
    );
    assert_eq!(
        journal.entries_since(after_first),
        &[
            Entry::Marker(Marker::Checkpoint(SnapshotId::new(7))),
            Entry::Undo(second),
        ]
    );

    journal.truncate_to(after_first);
    assert_eq!(journal.entries_since(start), &[Entry::Undo(first)]);
    assert!(journal.entries_since(after_first).is_empty());
}

#[test]
fn journal_positions_are_ordered_by_entry_offset() {
    let mut journal = Journal::new();

    let first = journal.pos();
    journal.push_undo(UndoRec::new(CellId::new(BankTag::Dimen, 0), 1, 2));
    let second = journal.pos();

    assert!(first < second);
    assert!(journal.entries_since(first).len() > journal.entries_since(second).len());
}

#[test]
fn undo_record_accessors_preserve_fields() {
    let cell = CellId::new_global(BankTag::Box, 12);
    let rec = UndoRec::new(cell, u64::MIN, u64::MAX);

    assert_eq!(rec.cell(), cell);
    assert_eq!(rec.old(), u64::MIN);
    assert_eq!(rec.new_value(), u64::MAX);
}

#[test]
fn save_stack_projection_updates_and_rolls_back_incrementally() {
    let mut journal = Journal::new();
    journal.push_marker(Marker::Group {
        aftergroup_start: 0,
        kind: GroupKind::Simple,
    });
    assert_eq!(journal.canonical_save_stack_words(), 1);
    assert_eq!(journal.canonical_save_stack_projection().1, Some((1, 1)));

    let undefined = CellId::new(BankTag::Meaning, 1);
    journal.push_undo(UndoRec::new(
        undefined,
        Meaning::Undefined.encode(),
        Meaning::Relax.encode(),
    ));
    journal.push_undo(UndoRec::new(
        undefined,
        Meaning::Relax.encode(),
        Meaning::Undefined.encode(),
    ));
    assert_eq!(journal.canonical_save_stack_words(), 2);

    let count = CellId::new(BankTag::Count, 2);
    journal.push_undo(UndoRec::new(count, 10, 20));
    assert_eq!(journal.canonical_save_stack_words(), 4);
    assert_eq!(journal.canonical_save_stack_projection().1, Some((4, 2)));
    journal.push_undo(UndoRec::new(CellId::new_global(BankTag::Count, 2), 20, 30));
    assert_eq!(journal.canonical_save_stack_words(), 4);
    assert_eq!(
        journal.canonical_save_stack_projection().1,
        Some((4, 2)),
        "a global definition retains the already-pushed physical restore"
    );

    journal.push_box_undo(BoxUndoRec::new(
        3,
        false,
        BoxSlot::default(),
        BoxSlot::default(),
    ));
    assert_eq!(journal.canonical_save_stack_words(), 6);
    assert_eq!(journal.canonical_save_stack_projection().1, Some((6, 2)));
    assert_eq!(
        journal.testing_save_stack_projection_rolled_back_entries(),
        0
    );

    let before_nested = journal.pos();
    journal.push_marker(Marker::Group {
        aftergroup_start: 0,
        kind: GroupKind::SemiSimple,
    });
    journal.push_undo(UndoRec::new(CellId::new(BankTag::Dimen, 4), 0, 1));
    assert_eq!(journal.canonical_save_stack_words(), 9);
    assert_eq!(
        journal.testing_save_stack_projection_rolled_back_entries(),
        0
    );

    journal.truncate_to(before_nested);
    assert_eq!(journal.canonical_save_stack_words(), 6);
    assert_eq!(journal.canonical_save_stack_projection().1, Some((6, 2)));
    assert_eq!(
        journal.testing_save_stack_projection_rolled_back_entries(),
        2,
        "truncate rolls back only the removed suffix"
    );
}

#[test]
fn save_stack_projection_truncate_restores_local_eligibility_removed_by_global_assignment() {
    let mut journal = Journal::new();
    journal.push_marker(Marker::Group {
        aftergroup_start: 0,
        kind: GroupKind::Simple,
    });
    let count = CellId::new(BankTag::Count, 9);
    journal.push_undo(UndoRec::new(count, 10, 20));
    let after_local = journal.pos();
    let after_local_projection = journal.canonical_save_stack_projection();

    journal.push_undo(UndoRec::new(CellId::new_global(BankTag::Count, 9), 20, 30));
    journal.push_undo(UndoRec::new(count, 30, 40));
    assert_eq!(journal.canonical_save_stack_words(), 5);

    journal.truncate_to(after_local);
    assert_eq!(
        journal.canonical_save_stack_projection(),
        after_local_projection
    );
    journal.push_undo(UndoRec::new(count, 20, 50));
    assert_eq!(
        journal.canonical_save_stack_projection(),
        after_local_projection,
        "rolling back the global reset must restore the enclosing local run"
    );
}

#[test]
fn save_stack_projection_truncate_does_not_replay_retained_prefix() {
    let mut journal = Journal::new();
    for index in 0..4096 {
        journal.push_undo(UndoRec::new(
            CellId::new_global(BankTag::Meaning, index),
            Meaning::Undefined.encode(),
            Meaning::Relax.encode(),
        ));
    }
    let retained_prefix = journal.pos();
    journal.push_marker(Marker::Group {
        aftergroup_start: 0,
        kind: GroupKind::Simple,
    });
    journal.push_undo(UndoRec::new(CellId::new(BankTag::Dimen, 1), 0, 65_536));

    journal.truncate_to(retained_prefix);
    assert_eq!(journal.canonical_save_stack_projection(), (0, None));
    assert_eq!(
        journal.testing_save_stack_projection_rolled_back_entries(),
        2,
        "projection work is bounded by the removed suffix, not the retained prefix"
    );
}
