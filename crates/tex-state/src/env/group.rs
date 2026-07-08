use super::{Env, cell_key, checked_aftergroup_start, u32_len};
use crate::journal::{Entry, JournalPos, Marker, UndoRec};
use std::collections::{HashMap, HashSet};

/// Crate-private environment rollback mark.
///
/// The public rollback boundary is `Stores`; this token exists only so that
/// `Stores` can restore all Env-owned rollback-coupled state atomically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EnvSnapshot {
    journal_pos: JournalPos,
    aftergroup_len: u32,
    group_depth: u32,
}

impl EnvSnapshot {
    /// Returns the journal position captured by this snapshot.
    #[must_use]
    pub(crate) const fn journal_pos(self) -> JournalPos {
        self.journal_pos
    }

    /// Returns the group depth captured by this snapshot.
    #[must_use]
    pub(crate) const fn group_depth(self) -> u32 {
        self.group_depth
    }
}

impl Env {
    /// Records a checkpoint position and starts a fresh epoch for later writes.
    #[must_use]
    pub(crate) fn checkpoint(&mut self) -> EnvSnapshot {
        let snapshot = EnvSnapshot {
            journal_pos: self.journal.pos(),
            aftergroup_len: u32_len(
                self.aftergroup.len(),
                "aftergroup payload list exceeds u32 entries",
            ),
            group_depth: self.group_depth,
        };
        self.epoch.bump();
        snapshot
    }

    /// Returns journal entries appended since `pos`.
    #[must_use]
    pub(crate) fn journal_entries_since(&self, pos: JournalPos) -> &[Entry] {
        self.journal.entries_since(pos)
    }

    pub(crate) fn last_group_marker_pos(&self) -> Option<JournalPos> {
        self.journal.find_last_group_marker().map(|(pos, _)| pos)
    }

    #[must_use]
    pub(crate) fn current_journal_pos(&self) -> JournalPos {
        self.journal.pos()
    }

    #[must_use]
    pub(crate) const fn group_depth(&self) -> u32 {
        self.group_depth
    }

    /// Enters a TeX group.
    pub(crate) fn enter_group(&mut self) {
        let aftergroup_start = u32_len(
            self.aftergroup.len(),
            "aftergroup payload list exceeds u32 entries",
        );
        self.journal.push_marker(Marker::Group { aftergroup_start });
        self.group_depth = self
            .group_depth
            .checked_add(1)
            .expect("group depth exceeds u32 entries");
        self.epoch.bump();
    }

    /// Pushes an opaque `\aftergroup` payload for the current group.
    pub(crate) fn push_aftergroup(&mut self, payload: u64) {
        self.aftergroup.push(payload);
    }

    /// Leaves the innermost TeX group and returns its `\aftergroup` payloads.
    ///
    /// Payloads are returned FIFO. Global assignments in the group survive by
    /// being compacted into the enclosing journal slice.
    #[must_use]
    pub(crate) fn leave_group(&mut self) -> Vec<u64> {
        let Some((marker_pos, aftergroup_start)) = self.journal.find_last_group_marker() else {
            panic!("leave_group without matching group marker");
        };
        self.group_depth = self
            .group_depth
            .checked_sub(1)
            .expect("leave_group without matching group marker");
        let marker_index = marker_pos.raw() as usize;
        let group_end = self.journal.len();
        let has_globals = (marker_index + 1..group_end).any(
            |index| matches!(self.journal.entry(index), Entry::Undo(rec) if rec.cell().is_global()),
        );

        if has_globals {
            self.leave_group_with_globals(marker_index, group_end);
        } else {
            for index in (marker_index + 1..group_end).rev() {
                if let Entry::Undo(rec) = self.journal.entry(index) {
                    self.restore_raw(rec.cell(), rec.old());
                }
            }
            self.journal.truncate_to(marker_pos);
        }

        let aftergroup_start = checked_aftergroup_start(aftergroup_start, self.aftergroup.len());
        let payloads = self.aftergroup.drain(aftergroup_start..).collect();

        // core_state.md §6 / 97a3c1d: restore leaves stamps high, so group
        // exit must start a fresh epoch or the enclosing undo slice can be
        // corrupted by a later write to the same restored cell.
        self.epoch.bump();
        payloads
    }

    fn leave_group_with_globals(&mut self, marker_index: usize, group_end: usize) {
        let mut globals = Vec::new();
        let mut globally_reassigned = HashSet::new();
        let mut first_old = HashMap::new();

        for index in marker_index + 1..group_end {
            if let Entry::Undo(rec) = self.journal.entry(index) {
                first_old
                    .entry(cell_key(rec.cell()))
                    .or_insert_with(|| rec.old());
            }
        }

        for index in (marker_index + 1..group_end).rev() {
            match self.journal.entry(index) {
                Entry::Undo(rec) if rec.cell().is_global() => {
                    globally_reassigned.insert(cell_key(rec.cell()));
                    globals.push(rec);
                }
                Entry::Undo(rec) if globally_reassigned.contains(&cell_key(rec.cell())) => {}
                Entry::Undo(rec) => self.restore_raw(rec.cell(), rec.old()),
                Entry::Marker(Marker::Checkpoint(_)) => {}
                Entry::Marker(Marker::Group { .. }) => {
                    unreachable!("group slice starts after the marker")
                }
            }
        }

        self.journal.truncate_to(JournalPos::from_raw(marker_index));
        let mut refiled_globals = HashSet::new();
        for rec in globals.into_iter().rev() {
            self.restore_raw(rec.cell(), rec.new_value());
            let key = cell_key(rec.cell());
            let old = if refiled_globals.insert(key) {
                first_old[&key]
            } else {
                rec.old()
            };
            self.journal
                .push_undo(UndoRec::new(rec.cell(), old, rec.new_value()));
        }
    }

    /// Rolls back all environment state after `snapshot`.
    pub(crate) fn rollback_to(&mut self, snapshot: EnvSnapshot) {
        let snapshot_index = snapshot.journal_pos.raw() as usize;
        let rollback_end = self.journal.len();
        for index in (snapshot_index..rollback_end).rev() {
            if let Entry::Undo(rec) = self.journal.entry(index) {
                self.restore_raw(rec.cell(), rec.old());
            }
        }
        self.journal.truncate_to(snapshot.journal_pos);
        self.group_depth = snapshot.group_depth;
        self.aftergroup.truncate(checked_aftergroup_start(
            snapshot.aftergroup_len,
            self.aftergroup.len(),
        ));
        self.epoch.bump();
    }
}
