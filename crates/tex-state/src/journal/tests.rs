use super::{Entry, Journal, Marker, UndoRec};
use crate::cell::{BankTag, CellId};
use crate::ids::SnapshotId;
use std::mem::size_of;

#[test]
fn widened_cell_ids_do_not_grow_journal_records() {
    assert_eq!(size_of::<CellId>(), 8);
    assert_eq!(size_of::<UndoRec>(), 24);
    assert_eq!(size_of::<Entry>(), 32);
}

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
    assert_eq!(first.raw(), 0);
    assert_eq!(second.raw(), 1);
}

#[test]
fn undo_record_accessors_preserve_fields() {
    let cell = CellId::new_global(BankTag::Box, 12);
    let rec = UndoRec::new(cell, u64::MIN, u64::MAX);

    assert_eq!(rec.cell(), cell);
    assert_eq!(rec.old(), u64::MIN);
    assert_eq!(rec.new_value(), u64::MAX);
}
