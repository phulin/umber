use super::{Env, cell_key, checked_aftergroup_start, u32_len};
use crate::journal::{Entry, JournalPos, Marker, UndoRec};
use crate::token::Token;
use std::collections::{HashMap, HashSet};

/// TeX group boundary kind tracked on state-layer group markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupKind {
    /// A `{` ... `}` group.
    Simple,
    /// A box-packaging group whose closing brace is owned by its scanner.
    Box,
    /// A `\begingroup` ... `\endgroup` group.
    SemiSimple,
    /// A `$` ... `$` or `$$` ... `$$` math-shift group.
    MathShift,
    /// TeX's per-entry `align_group`, replaced after every alignment cell.
    Align,
}

/// Cached location and payload metadata for one live journal group marker.
///
/// This stack is rollback-coupled to the journal and makes current-group
/// queries independent of the number of writes made inside the group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GroupBoundary {
    marker_pos: JournalPos,
    aftergroup_start: u32,
    kind: GroupKind,
}

impl GroupKind {
    #[must_use]
    pub const fn start_text(self) -> &'static str {
        match self {
            Self::Simple => "{",
            Self::Box => "{",
            Self::SemiSimple => "\\begingroup",
            Self::MathShift => "$",
            Self::Align => "an alignment entry",
        }
    }

    #[must_use]
    pub const fn end_text(self) -> &'static str {
        match self {
            Self::Simple => "}",
            Self::Box => "}",
            Self::SemiSimple => "\\endgroup",
            Self::MathShift => "$",
            Self::Align => "\\cr",
        }
    }
}

/// Group-boundary mismatch detected before any state rollback is performed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupMismatch {
    expected: GroupKind,
    actual: GroupKind,
}

impl GroupMismatch {
    pub(crate) const fn new(expected: GroupKind, actual: GroupKind) -> Self {
        Self { expected, actual }
    }

    pub(crate) const fn new_no_group(expected: GroupKind) -> Self {
        Self {
            expected,
            actual: expected,
        }
    }

    #[must_use]
    pub const fn expected(self) -> GroupKind {
        self.expected
    }

    #[must_use]
    pub const fn actual(self) -> GroupKind {
        self.actual
    }
}

/// Crate-private environment rollback mark.
///
/// The public rollback boundary is `Universe`; this token exists only so that
/// the aggregate owner can restore all Env-owned rollback-coupled state
/// atomically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EnvSnapshot {
    journal_pos: JournalPos,
    aftergroup_len: u32,
    afterassignment: Option<Token>,
    group_depth: u32,
    group_boundary_len: u32,
    epoch: crate::epoch::Epoch,
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

    /// Returns the epoch captured by this snapshot.
    #[must_use]
    pub(crate) const fn epoch(self) -> crate::epoch::Epoch {
        self.epoch
    }
}

impl Env {
    /// Records a checkpoint position and starts a fresh epoch for later writes.
    #[must_use]
    pub(crate) fn checkpoint(&mut self) -> EnvSnapshot {
        self.box_journal_positions.clear();
        let snapshot = EnvSnapshot {
            journal_pos: self.journal.pos(),
            aftergroup_len: u32_len(
                self.aftergroup.len(),
                "aftergroup payload list exceeds u32 entries",
            ),
            afterassignment: self.afterassignment,
            group_depth: self.group_depth,
            group_boundary_len: u32_len(
                self.group_boundaries.len(),
                "group boundary stack exceeds u32 entries",
            ),
            epoch: self.epoch,
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
        self.group_boundaries
            .last()
            .map(|boundary| boundary.marker_pos)
    }

    #[must_use]
    pub(crate) fn current_journal_pos(&self) -> JournalPos {
        self.journal.pos()
    }

    #[must_use]
    pub(crate) const fn group_depth(&self) -> u32 {
        self.group_depth
    }

    #[must_use]
    pub(crate) fn innermost_group_kind(&self) -> Option<GroupKind> {
        self.group_boundaries.last().map(|boundary| boundary.kind)
    }

    /// Enters a TeX group.
    pub(crate) fn enter_group(&mut self) {
        self.enter_group_with_kind(GroupKind::Simple);
    }

    /// Enters a TeX group with an explicit boundary kind.
    pub(crate) fn enter_group_with_kind(&mut self, kind: GroupKind) {
        let aftergroup_start = u32_len(
            self.aftergroup.len(),
            "aftergroup payload list exceeds u32 entries",
        );
        let marker_pos = self.journal.pos();
        self.journal.push_marker(Marker::Group {
            aftergroup_start,
            kind,
        });
        self.group_boundaries.push(GroupBoundary {
            marker_pos,
            aftergroup_start,
            kind,
        });
        self.group_depth = self
            .group_depth
            .checked_add(1)
            .expect("group depth exceeds u32 entries");
        self.epoch.bump();
    }

    /// Pushes an opaque `\aftergroup` payload for the current group.
    pub(crate) fn push_aftergroup(&mut self, payload: Token) {
        if self.group_depth != 0 {
            self.aftergroup.push(payload);
        }
    }

    /// Stores the token to replay after the next assignment.
    pub(crate) fn set_afterassignment(&mut self, token: Token) {
        self.afterassignment = Some(token);
    }

    /// Takes and clears the token to replay after the next assignment.
    pub(crate) fn take_afterassignment(&mut self) -> Option<Token> {
        self.afterassignment.take()
    }

    /// Leaves the innermost TeX group and returns its `\aftergroup` payloads.
    ///
    /// Payloads are returned FIFO. Global assignments in the group survive by
    /// being compacted into the enclosing journal slice.
    #[must_use]
    pub(crate) fn leave_group(&mut self) -> Vec<Token> {
        self.leave_group_unchecked()
    }

    /// Leaves the innermost TeX group if it matches the requested boundary kind.
    pub(crate) fn leave_group_with_kind(
        &mut self,
        expected: GroupKind,
    ) -> Result<Vec<Token>, GroupMismatch> {
        let Some(actual) = self.innermost_group_kind() else {
            return Err(GroupMismatch::new_no_group(expected));
        };
        if actual != expected {
            return Err(GroupMismatch::new(expected, actual));
        }
        Ok(self.leave_group_unchecked())
    }

    fn leave_group_unchecked(&mut self) -> Vec<Token> {
        let Some(boundary) = self.group_boundaries.pop() else {
            panic!("leave_group without matching group marker");
        };
        let marker_pos = boundary.marker_pos;
        let aftergroup_start = boundary.aftergroup_start;
        let exiting_depth = self.group_depth;
        self.box_journal_positions
            .retain(|&(_, depth), _| depth != exiting_depth);
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
        self.box_journal_positions.clear();
        let snapshot_index = snapshot.journal_pos.raw() as usize;
        let rollback_end = self.journal.len();
        for index in (snapshot_index..rollback_end).rev() {
            if let Entry::Undo(rec) = self.journal.entry(index) {
                self.restore_raw(rec.cell(), rec.old());
            }
        }
        self.journal.truncate_to(snapshot.journal_pos);
        self.group_boundaries.truncate(
            snapshot
                .group_boundary_len
                .try_into()
                .expect("group boundary length fits usize"),
        );
        self.group_depth = snapshot.group_depth;
        self.aftergroup.truncate(checked_aftergroup_start(
            snapshot.aftergroup_len,
            self.aftergroup.len(),
        ));
        self.afterassignment = snapshot.afterassignment;
        self.epoch.bump();
    }
}
