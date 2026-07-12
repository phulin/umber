//! Append-only journal storage for barriered environment writes.
//!
//! The journal records undo+redo words and structural markers. `Env` owns the
//! group-exit and rollback walks; this module owns positions, append, slicing,
//! truncation, and marker lookup.

use crate::cell::CellId;
use crate::env::group::GroupKind;
use crate::ids::SnapshotId;

/// A journal entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Entry {
    Undo(UndoRec),
    Marker(Marker),
}

/// A barrier undo+redo record for one environment cell.
///
/// The write barrier records only the first write to a cell in each epoch.
/// With undo+redo records, that means `new` is the value from the first
/// barrier hit and can be stale if the same cell is written again before the
/// epoch advances. M1 accepts that behavior: rollback uses `old`, while later
/// forward-replay consumers must re-derive final values from live cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UndoRec {
    cell: CellId,
    old: u64,
    new: u64,
}

impl UndoRec {
    /// Creates a journal record for `cell`, replacing `old` with `new`.
    #[must_use]
    pub(crate) const fn new(cell: CellId, old: u64, new: u64) -> Self {
        Self { cell, old, new }
    }

    /// Returns the recorded cell id.
    #[must_use]
    pub(crate) const fn cell(self) -> CellId {
        self.cell
    }

    /// Returns the value to restore when walking the journal backward.
    #[must_use]
    pub(crate) const fn old(self) -> u64 {
        self.old
    }

    /// Returns the value written by the barrier.
    #[must_use]
    pub(crate) const fn new_value(self) -> u64 {
        self.new
    }

    #[must_use]
    pub(crate) const fn with_new_value(self, new: u64) -> Self {
        Self { new, ..self }
    }
}

/// Structural journal markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Marker {
    Group {
        aftergroup_start: u32,
        kind: GroupKind,
    },
    #[allow(dead_code)]
    Checkpoint(SnapshotId),
}

/// A stable position between journal entries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct JournalPos(u32);

impl JournalPos {
    /// Creates a journal position from a previously validated entry offset.
    #[must_use]
    pub(crate) fn from_raw(raw: usize) -> Self {
        JournalPos(u32_len(raw, "journal exceeds u32 entries"))
    }

    /// Returns the raw entry offset.
    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

/// Append/truncate journal storage.
#[derive(Clone, Debug, Default)]
pub(crate) struct Journal {
    entries: Vec<Entry>,
}

impl Journal {
    /// Creates an empty journal.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Appends an undo+redo record.
    pub(crate) fn push_undo(&mut self, rec: UndoRec) -> JournalPos {
        let pos = self.pos();
        self.entries.push(Entry::Undo(rec));
        pos
    }

    /// Replaces the forward value of an existing undo entry.
    pub(crate) fn replace_undo_new_value(&mut self, pos: JournalPos, new: u64) {
        let index = checked_pos(pos, self.entries.len());
        let Some(entry) = self.entries.get_mut(index) else {
            panic!("journal position does not name an undo entry");
        };
        let Entry::Undo(rec) = entry else {
            panic!("journal position does not name an undo entry");
        };
        *rec = rec.with_new_value(new);
    }

    /// Appends a structural marker.
    pub(crate) fn push_marker(&mut self, marker: Marker) {
        self.entries.push(Entry::Marker(marker));
    }

    /// Returns the current end position.
    #[must_use]
    pub(crate) fn pos(&self) -> JournalPos {
        JournalPos(u32_len(self.entries.len(), "journal exceeds u32 entries"))
    }

    /// Returns the number of entries currently held by the journal.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns one journal entry by absolute entry offset.
    #[must_use]
    pub(crate) fn entry(&self, index: usize) -> Entry {
        self.entries[index]
    }

    /// Returns entries appended since `pos`.
    #[must_use]
    pub(crate) fn entries_since(&self, pos: JournalPos) -> &[Entry] {
        let start = checked_pos(pos, self.entries.len());
        &self.entries[start..]
    }

    /// Truncates the journal to `pos`.
    pub(crate) fn truncate_to(&mut self, pos: JournalPos) {
        let len = checked_pos(pos, self.entries.len());
        self.entries.truncate(len);
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
mod tests;
