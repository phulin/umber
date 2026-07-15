use super::{Env, cell_key, checked_aftergroup_start, u32_len};
use crate::cell::BankTag;
use crate::journal::{BoxUndoRec, Entry, JournalPos, Marker, UndoRec};
use crate::token::Token;
use ahash::AHashMap;

#[derive(Clone, Copy, Debug)]
struct GlobalCompactionState<T> {
    first_old: T,
    has_later_global: bool,
    refiled: bool,
}

impl<T> GlobalCompactionState<T> {
    fn new(first_old: T) -> Self {
        Self {
            first_old,
            has_later_global: false,
            refiled: false,
        }
    }
}

/// TeX group boundary kind tracked on state-layer group markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupKind {
    /// A `{` ... `}` group.
    Simple,
    HBox,
    AdjustedHBox,
    VBox,
    VTop,
    /// A `\begingroup` ... `\endgroup` group.
    SemiSimple,
    /// A `$` ... `$` or `$$` ... `$$` math-shift group.
    MathShift,
    /// TeX's per-entry `align_group`, replaced after every alignment cell.
    Align,
    NoAlign,
    Output,
    Math,
    Disc,
    Insert,
    VCenter,
    MathChoice,
    MathLeft,
}

/// Cached location and payload metadata for one live journal group marker.
///
/// This stack is rollback-coupled to the journal and makes current-group
/// queries independent of the number of writes made inside the group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GroupBoundary {
    marker_pos: JournalPos,
    box_undo_len: u32,
    aftergroup_start: u32,
    kind: GroupKind,
}

impl GroupKind {
    #[must_use]
    pub const fn start_text(self) -> &'static str {
        match self {
            Self::Simple => "{",
            Self::HBox
            | Self::AdjustedHBox
            | Self::VBox
            | Self::VTop
            | Self::NoAlign
            | Self::Output
            | Self::Math
            | Self::Disc
            | Self::Insert
            | Self::VCenter
            | Self::MathChoice
            | Self::MathLeft => "{",
            Self::SemiSimple => "\\begingroup",
            Self::MathShift => "$",
            Self::Align => "an alignment entry",
        }
    }

    #[must_use]
    pub const fn end_text(self) -> &'static str {
        match self {
            Self::Simple => "}",
            Self::HBox
            | Self::AdjustedHBox
            | Self::VBox
            | Self::VTop
            | Self::NoAlign
            | Self::Output
            | Self::Math
            | Self::Disc
            | Self::Insert
            | Self::VCenter
            | Self::MathChoice
            | Self::MathLeft => "}",
            Self::SemiSimple => "\\endgroup",
            Self::MathShift => "$",
            Self::Align => "\\cr",
        }
    }

    #[must_use]
    pub const fn etex_code(self) -> i32 {
        match self {
            Self::Simple => 1,
            Self::HBox => 2,
            Self::AdjustedHBox => 3,
            Self::VBox => 4,
            Self::VTop => 5,
            Self::Align => 6,
            Self::NoAlign => 7,
            Self::Output => 8,
            Self::Math => 9,
            Self::Disc => 10,
            Self::Insert => 11,
            Self::VCenter => 12,
            Self::MathChoice => 13,
            Self::SemiSimple => 14,
            Self::MathShift => 15,
            Self::MathLeft => 16,
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
    box_undo_len: u32,
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
        let snapshot = EnvSnapshot {
            journal_pos: self.journal.pos(),
            box_undo_len: self.journal.box_undo_len(),
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

    pub(crate) fn box_undo(&self, id: crate::journal::BoxUndoId) -> BoxUndoRec {
        self.journal.box_undo(id)
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

    pub(crate) fn journal_retained_bytes(&self) -> usize {
        self.journal.retained_bytes()
    }

    #[must_use]
    pub(crate) const fn group_depth(&self) -> u32 {
        self.group_depth
    }

    #[must_use]
    pub(crate) fn innermost_group_kind(&self) -> Option<GroupKind> {
        self.group_boundaries.last().map(|boundary| boundary.kind)
    }

    #[must_use]
    pub(crate) fn group_kinds(&self) -> impl DoubleEndedIterator<Item = GroupKind> + '_ {
        self.group_boundaries.iter().map(|boundary| boundary.kind)
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
        let box_undo_len = self.journal.box_undo_len();
        self.journal.push_marker(Marker::Group {
            aftergroup_start,
            kind,
        });
        self.group_boundaries.push(GroupBoundary {
            marker_pos,
            box_undo_len,
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
    #[cfg(test)]
    pub(crate) fn leave_group(&mut self) -> Vec<Token> {
        self.leave_group_unchecked().0
    }

    /// Leaves the innermost group and reports whether meaning cells were
    /// restored or compacted while crossing the boundary.
    #[must_use]
    pub(crate) fn leave_group_observing_meanings(
        &mut self,
    ) -> (Vec<Token>, bool, Vec<crate::cell::CellId>) {
        self.leave_group_unchecked()
    }

    /// Leaves the innermost TeX group if it matches the requested boundary kind.
    #[cfg(test)]
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
        Ok(self.leave_group_unchecked().0)
    }

    pub(crate) fn leave_group_with_kind_observing_meanings(
        &mut self,
        expected: GroupKind,
    ) -> Result<(Vec<Token>, bool, Vec<crate::cell::CellId>), GroupMismatch> {
        let Some(actual) = self.innermost_group_kind() else {
            return Err(GroupMismatch::new_no_group(expected));
        };
        if actual != expected {
            return Err(GroupMismatch::new(expected, actual));
        }
        Ok(self.leave_group_unchecked())
    }

    fn leave_group_unchecked(&mut self) -> (Vec<Token>, bool, Vec<crate::cell::CellId>) {
        let Some(boundary) = self.group_boundaries.pop() else {
            panic!("leave_group without matching group marker");
        };
        let marker_pos = boundary.marker_pos;
        let aftergroup_start = boundary.aftergroup_start;
        let leaving_depth = self.group_depth;
        self.group_depth = self
            .group_depth
            .checked_sub(1)
            .expect("leave_group without matching group marker");
        let marker_index = marker_pos.raw() as usize;
        let group_end = self.journal.len();
        let mut changed_cells = (marker_index + 1..group_end)
            .filter_map(|index| match self.journal.entry(index) {
                Entry::Undo(rec) => Some(rec.cell()),
                Entry::BoxUndo(id) => Some(crate::cell::CellId::new(
                    BankTag::Box,
                    u32::from(self.journal.box_undo(id).index()),
                )),
                Entry::Marker(_) => None,
            })
            .collect::<Vec<_>>();
        changed_cells.sort_unstable();
        changed_cells.dedup();
        let has_globals =
            (marker_index + 1..group_end).any(|index| match self.journal.entry(index) {
                Entry::Undo(rec) => rec.cell().is_global(),
                Entry::BoxUndo(id) => self.journal.box_undo(id).survives_group(leaving_depth),
                Entry::Marker(_) => false,
            });

        let meaning_changed = if has_globals {
            self.leave_group_with_globals(
                marker_index,
                group_end,
                boundary.box_undo_len,
                leaving_depth,
            )
        } else {
            let mut meaning_changed = false;
            for index in (marker_index + 1..group_end).rev() {
                if let Entry::Undo(rec) = self.journal.entry(index) {
                    meaning_changed |= rec.cell().bank() == BankTag::Meaning;
                    self.restore_raw(rec.cell(), rec.old());
                } else if let Entry::BoxUndo(id) = self.journal.entry(index) {
                    let rec = self.journal.box_undo(id);
                    self.boxes.restore(rec.index(), rec.old());
                }
            }
            self.journal.truncate_to(marker_pos);
            self.journal.truncate_box_undos(boundary.box_undo_len);
            meaning_changed
        };

        let aftergroup_start = checked_aftergroup_start(aftergroup_start, self.aftergroup.len());
        let payloads = self.aftergroup.drain(aftergroup_start..).collect();

        // core_state.md §6 / 97a3c1d: restore leaves stamps high, so group
        // exit must start a fresh epoch or the enclosing undo slice can be
        // corrupted by a later write to the same restored cell.
        self.epoch.bump();
        (payloads, meaning_changed, changed_cells)
    }

    fn leave_group_with_globals(
        &mut self,
        marker_index: usize,
        group_end: usize,
        box_undo_len: u32,
        leaving_depth: u32,
    ) -> bool {
        let mut globals = Vec::new();
        let mut box_globals = Vec::new();
        let mut cell_states = AHashMap::new();
        let mut box_states = AHashMap::new();

        for index in marker_index + 1..group_end {
            if let Entry::Undo(rec) = self.journal.entry(index) {
                cell_states
                    .entry(cell_key(rec.cell()))
                    .or_insert_with(|| GlobalCompactionState::new(rec.old()));
            } else if let Entry::BoxUndo(id) = self.journal.entry(index) {
                let rec = self.journal.box_undo(id);
                box_states
                    .entry(rec.index())
                    .or_insert_with(|| GlobalCompactionState::new(rec.old()));
            }
        }

        let mut meaning_changed = false;
        for index in (marker_index + 1..group_end).rev() {
            match self.journal.entry(index) {
                Entry::Undo(rec) if rec.cell().is_global() => {
                    cell_states
                        .get_mut(&cell_key(rec.cell()))
                        .expect("journal cell was indexed before group compaction")
                        .has_later_global = true;
                    globals.push(rec);
                }
                Entry::Undo(rec) => {
                    let state = cell_states
                        .get(&cell_key(rec.cell()))
                        .expect("journal cell was indexed before group compaction");
                    if !state.has_later_global {
                        meaning_changed |= rec.cell().bank() == BankTag::Meaning;
                        self.restore_raw(rec.cell(), rec.old());
                    }
                }
                Entry::BoxUndo(id) => {
                    let rec = self.journal.box_undo(id);
                    if rec.survives_group(leaving_depth) {
                        box_states
                            .get_mut(&rec.index())
                            .expect("box undo was indexed before group compaction")
                            .has_later_global = true;
                        box_globals.push(rec);
                    } else {
                        let state = box_states
                            .get(&rec.index())
                            .expect("box undo was indexed before group compaction");
                        if !state.has_later_global {
                            self.boxes.restore(rec.index(), rec.old());
                        }
                    }
                }
                Entry::Marker(Marker::Checkpoint(_)) => {}
                Entry::Marker(Marker::Group { .. }) => {
                    unreachable!("group slice starts after the marker")
                }
            }
        }

        self.journal.truncate_to(JournalPos::from_raw(marker_index));
        self.journal.truncate_box_undos(box_undo_len);
        for rec in globals.into_iter().rev() {
            meaning_changed |= rec.cell().bank() == BankTag::Meaning;
            self.restore_raw(rec.cell(), rec.new_value());
            let key = cell_key(rec.cell());
            let state = cell_states
                .get_mut(&key)
                .expect("journal cell was indexed before group compaction");
            let old = if state.refiled {
                rec.old()
            } else {
                state.refiled = true;
                state.first_old
            };
            self.journal
                .push_undo(UndoRec::new(rec.cell(), old, rec.new_value()));
        }
        for rec in box_globals.into_iter().rev() {
            self.boxes.restore(rec.index(), rec.new_value());
            let state = box_states
                .get_mut(&rec.index())
                .expect("box undo was indexed before group compaction");
            let old = if state.refiled {
                rec.old()
            } else {
                state.refiled = true;
                state.first_old
            };
            self.journal.push_box_undo(if rec.is_global() {
                BoxUndoRec::new(rec.index(), true, old, rec.new_value())
            } else {
                BoxUndoRec::new_at_depth(rec.index(), rec.restore_depth(), old, rec.new_value())
            });
        }
        meaning_changed
    }

    /// Rolls back all environment state after `snapshot`.
    pub(crate) fn rollback_to(&mut self, snapshot: EnvSnapshot) {
        let snapshot_index = snapshot.journal_pos.raw() as usize;
        let rollback_end = self.journal.len();
        for index in (snapshot_index..rollback_end).rev() {
            if let Entry::Undo(rec) = self.journal.entry(index) {
                self.restore_raw(rec.cell(), rec.old());
            } else if let Entry::BoxUndo(id) = self.journal.entry(index) {
                let rec = self.journal.box_undo(id);
                self.boxes.restore(rec.index(), rec.old());
            }
        }
        self.journal.truncate_to(snapshot.journal_pos);
        self.journal.truncate_box_undos(snapshot.box_undo_len);
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
