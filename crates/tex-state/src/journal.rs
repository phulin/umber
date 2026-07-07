//! Append-only journal storage for barriered environment writes.
//!
//! The journal records undo+redo words and structural markers. Group exit and
//! rollback semantics are implemented by later Env tasks; this module only
//! owns positions, append, slicing, truncation, and marker lookup.

use crate::cell::CellId;
use crate::ids::SnapshotId;

/// A journal entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Entry {
    Undo(UndoRec),
    Marker(Marker),
}

/// A barrier undo+redo record for one environment cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UndoRec {
    cell: CellId,
    old: u64,
    new: u64,
}

impl UndoRec {
    /// Creates a journal record for `cell`, replacing `old` with `new`.
    #[must_use]
    pub const fn new(cell: CellId, old: u64, new: u64) -> Self {
        Self { cell, old, new }
    }

    /// Returns the recorded cell id.
    #[must_use]
    pub const fn cell(self) -> CellId {
        self.cell
    }

    /// Returns the value to restore when walking the journal backward.
    #[must_use]
    pub const fn old(self) -> u64 {
        self.old
    }

    /// Returns the value written by the barrier.
    #[must_use]
    pub const fn new_value(self) -> u64 {
        self.new
    }
}

/// Structural journal markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Marker {
    Group { aftergroup_start: u32 },
    Checkpoint(SnapshotId),
}

/// A stable position between journal entries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JournalPos(u32);

impl JournalPos {
    /// Returns the raw entry offset.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Append/truncate journal storage.
#[derive(Clone, Debug, Default)]
pub struct Journal {
    entries: Vec<Entry>,
}

impl Journal {
    /// Creates an empty journal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an undo+redo record.
    pub fn push_undo(&mut self, rec: UndoRec) {
        self.entries.push(Entry::Undo(rec));
    }

    /// Appends a structural marker.
    pub fn push_marker(&mut self, marker: Marker) {
        self.entries.push(Entry::Marker(marker));
    }

    /// Returns the current end position.
    #[must_use]
    pub fn pos(&self) -> JournalPos {
        JournalPos(u32_len(self.entries.len(), "journal exceeds u32 entries"))
    }

    /// Returns entries appended since `pos`.
    #[must_use]
    pub fn entries_since(&self, pos: JournalPos) -> &[Entry] {
        let start = checked_pos(pos, self.entries.len());
        &self.entries[start..]
    }

    /// Truncates the journal to `pos`.
    pub fn truncate_to(&mut self, pos: JournalPos) {
        let len = checked_pos(pos, self.entries.len());
        self.entries.truncate(len);
    }

    /// Finds the last group marker, skipping checkpoint markers.
    #[must_use]
    pub fn find_last_group_marker(&self) -> Option<(JournalPos, u32)> {
        for (index, entry) in self.entries.iter().enumerate().rev() {
            if let Entry::Marker(Marker::Group { aftergroup_start }) = entry {
                return Some((
                    JournalPos(u32_len(index, "journal exceeds u32 entries")),
                    *aftergroup_start,
                ));
            }
        }
        None
    }
}

fn checked_pos(pos: JournalPos, len: usize) -> usize {
    let index = pos.raw() as usize;
    assert!(index <= len, "journal position is past the end");
    index
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, Journal, JournalPos, Marker, UndoRec};
    use crate::cell::{BankTag, CellId};
    use crate::ids::SnapshotId;

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
    fn marker_search_skips_checkpoint_markers() {
        let mut journal = Journal::new();
        journal.push_marker(Marker::Group {
            aftergroup_start: 3,
        });
        journal.push_undo(UndoRec::new(CellId::new(BankTag::Toks, 4), 5, 6));
        journal.push_marker(Marker::Checkpoint(SnapshotId::new(99)));

        let found = journal.find_last_group_marker();

        assert_eq!(found, Some((JournalPos(0), 3)));
    }

    #[test]
    fn marker_search_finds_latest_group_marker() {
        let mut journal = Journal::new();
        journal.push_marker(Marker::Group {
            aftergroup_start: 1,
        });
        journal.push_marker(Marker::Checkpoint(SnapshotId::new(2)));
        journal.push_marker(Marker::Group {
            aftergroup_start: 8,
        });

        assert_eq!(journal.find_last_group_marker(), Some((JournalPos(2), 8)));
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
}
