//! Append-only journal storage for barriered environment writes.
//!
//! The journal records undo+redo words and structural markers. `Env` owns the
//! group-exit and rollback walks; this module owns positions, append, slicing,
//! truncation, and marker lookup.

use crate::cell::{BankTag, CellId};
use crate::env::box_bank::BoxSlot;
use crate::env::group::GroupKind;
use crate::ids::SnapshotId;
use crate::meaning::Meaning;
use ahash::AHashMap;

/// A journal entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Entry {
    Undo(UndoRec),
    BoxUndo(BoxUndoId),
    Marker(Marker),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoxUndoId(u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoxUndoRec {
    index: u16,
    global: bool,
    restore_depth: u32,
    old: BoxSlot,
    new: BoxSlot,
}

impl BoxUndoRec {
    pub(crate) const fn new(index: u16, global: bool, old: BoxSlot, new: BoxSlot) -> Self {
        Self {
            index,
            global,
            restore_depth: if global { 0 } else { new.owner_depth() },
            old,
            new,
        }
    }
    pub(crate) const fn new_at_depth(
        index: u16,
        restore_depth: u32,
        old: BoxSlot,
        new: BoxSlot,
    ) -> Self {
        Self {
            index,
            global: false,
            restore_depth,
            old,
            new,
        }
    }
    pub(crate) const fn index(self) -> u16 {
        self.index
    }
    pub(crate) const fn is_global(self) -> bool {
        self.global
    }
    pub(crate) const fn survives_group(self, leaving_depth: u32) -> bool {
        self.global || self.restore_depth < leaving_depth
    }
    pub(crate) const fn restore_depth(self) -> u32 {
        self.restore_depth
    }
    pub(crate) const fn old(self) -> BoxSlot {
        self.old
    }
    pub(crate) const fn new_value(self) -> BoxSlot {
        self.new
    }
    pub(crate) fn with_new_value(self, new: BoxSlot) -> Self {
        Self { new, ..self }
    }
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
    box_undos: Vec<BoxUndoRec>,
    save_stack: SaveStackProjection,
}

/// Incremental projection of TeX82 §§273--280's physical save stack.
///
/// The typed journal is the semantic owner. This derived, rollback-coupled
/// view exists only for §1334's diagnostic high-water accounting.
#[derive(Clone, Debug, Default)]
struct SaveStackProjection {
    words: usize,
    groups: Vec<AHashMap<CellId, usize>>,
    #[cfg(test)]
    rebuilds: usize,
}

impl SaveStackProjection {
    fn push_undo(&mut self, rec: UndoRec) {
        let Some(saved) = self.groups.last_mut() else {
            return;
        };
        let cell = rec.cell().without_assignment_scope();
        if rec.cell().is_global() {
            if let Some(words) = saved.remove(&cell) {
                self.words = self.words.saturating_sub(words);
            }
        } else if !saved.contains_key(&cell) {
            // TeX82 §§275--276 represents `restore_zero` in one word,
            // while `restore_old_value` occupies two.
            let words = if is_canonical_restore_zero(cell, rec.old()) {
                1
            } else {
                2
            };
            saved.insert(cell, words);
            self.words = self.words.saturating_add(words);
        }
    }

    fn push_box_undo(&mut self, rec: BoxUndoRec) {
        let Some(saved) = self.groups.last_mut() else {
            return;
        };
        let cell = CellId::new(BankTag::Box, u32::from(rec.index()));
        if rec.is_global() {
            if let Some(words) = saved.remove(&cell) {
                self.words = self.words.saturating_sub(words);
            }
        } else if !saved.contains_key(&cell) {
            saved.insert(cell, 2);
            self.words = self.words.saturating_add(2);
        }
    }

    fn push_marker(&mut self, marker: Marker) {
        if matches!(marker, Marker::Group { .. }) {
            self.words = self.words.saturating_add(1);
            self.groups.push(AHashMap::new());
        }
    }
}

fn is_canonical_restore_zero(cell: CellId, old: u64) -> bool {
    match cell.bank() {
        BankTag::Meaning => old == Meaning::Undefined.encode(),
        // TeX82 §240 initializes token-list parameters to
        // `undefined_control_sequence` at `level_zero`; the typed optional
        // codec preserves that null value as zero, distinct from a defined
        // empty list. Sections 275--276 save the null case in one word.
        BankTag::TokParam => old == 0,
        _ => false,
    }
}

impl Journal {
    /// Creates an empty journal.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.entries
            .capacity()
            .saturating_mul(std::mem::size_of::<Entry>())
            .saturating_add(
                self.box_undos
                    .capacity()
                    .saturating_mul(std::mem::size_of::<BoxUndoRec>()),
            )
            .saturating_add(
                self.save_stack
                    .groups
                    .iter()
                    .map(|group| {
                        group.capacity().saturating_mul(
                            std::mem::size_of::<CellId>() + std::mem::size_of::<usize>(),
                        )
                    })
                    .sum::<usize>(),
            )
    }

    /// Appends an undo+redo record.
    pub(crate) fn push_undo(&mut self, rec: UndoRec) -> JournalPos {
        let pos = self.pos();
        self.save_stack.push_undo(rec);
        self.entries.push(Entry::Undo(rec));
        pos
    }

    pub(crate) fn push_box_undo(&mut self, rec: BoxUndoRec) -> (BoxUndoRec, JournalPos) {
        let pos = self.pos();
        self.save_stack.push_box_undo(rec);
        let id = BoxUndoId(u32_len(
            self.box_undos.len(),
            "box undo arena exceeds u32 entries",
        ));
        self.box_undos.push(rec);
        self.entries.push(Entry::BoxUndo(id));
        (rec, pos)
    }

    pub(crate) fn box_undo(&self, id: BoxUndoId) -> BoxUndoRec {
        self.box_undos[id.0 as usize]
    }

    pub(crate) fn replace_box_new(&mut self, pos: JournalPos, new: BoxSlot) {
        let index = checked_pos(pos, self.entries.len());
        let Entry::BoxUndo(id) = self.entries[index] else {
            panic!("journal position does not name a box undo entry");
        };
        let rec = &mut self.box_undos[id.0 as usize];
        *rec = rec.with_new_value(new);
    }

    pub(crate) fn box_undo_len(&self) -> u32 {
        u32_len(self.box_undos.len(), "box undo arena exceeds u32 entries")
    }

    pub(crate) fn truncate_box_undos(&mut self, len: u32) {
        self.box_undos.truncate(len as usize);
    }

    /// Appends a structural marker.
    pub(crate) fn push_marker(&mut self, marker: Marker) {
        self.save_stack.push_marker(marker);
        self.entries.push(Entry::Marker(marker));
    }

    /// Returns the live TeX82 save-stack words represented by this journal.
    #[must_use]
    pub(crate) const fn canonical_save_stack_words(&self) -> usize {
        self.save_stack.words
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
        if len == self.entries.len() {
            return;
        }
        self.entries.truncate(len);
        self.rebuild_save_stack_projection();
    }

    fn rebuild_save_stack_projection(&mut self) {
        let mut projection = SaveStackProjection::default();
        #[cfg(test)]
        {
            projection.rebuilds = self.save_stack.rebuilds.saturating_add(1);
        }
        for entry in &self.entries {
            match *entry {
                Entry::Undo(rec) => projection.push_undo(rec),
                Entry::BoxUndo(id) => projection.push_box_undo(self.box_undos[id.0 as usize]),
                Entry::Marker(marker) => projection.push_marker(marker),
            }
        }
        self.save_stack = projection;
    }

    #[cfg(test)]
    pub(crate) const fn testing_save_stack_projection_rebuilds(&self) -> usize {
        self.save_stack.rebuilds
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
