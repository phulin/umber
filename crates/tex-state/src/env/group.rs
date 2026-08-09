use super::{Env, cell_key, checked_aftergroup_start, u32_len};
use crate::cell::{BankTag, CellId};
use crate::journal::{BoxUndoRec, Entry, JournalPos, Marker, UndoRec};
use crate::meaning::Meaning;
use crate::token::{Token, TracedTokenWord};
use ahash::AHashMap;
use ahash::AHashSet;
use smallvec::SmallVec;

pub(crate) type MutationReceipts = SmallVec<[crate::env::CellMutationReceipt; 8]>;

/// One TeX82 §283 save-stack diagnostic observed while unsaving a group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreRecord {
    save_position: usize,
    cell: CellId,
    old: u64,
    trace_eligible: bool,
    retaining: bool,
    tracing_restores: i32,
    tracing_online: i32,
    escape_char: i32,
    box_trace_text: Option<String>,
}

impl RestoreRecord {
    fn restoring(env: &Env, save_position: usize, cell: CellId, old: u64) -> Self {
        let restored = env.restored_semantic_word(cell, old);
        Self {
            save_position,
            cell,
            old: restored.word,
            trace_eligible: restored.trace_eligible,
            retaining: false,
            tracing_restores: env.int_param(crate::env::banks::IntParam::TRACING_RESTORES),
            tracing_online: env.int_param(crate::env::banks::IntParam::TRACING_ONLINE),
            escape_char: env.int_param(crate::env::banks::IntParam::ESCAPE_CHAR),
            box_trace_text: None,
        }
    }

    fn retaining(env: &Env, save_position: usize, cell: CellId) -> Self {
        let restored = env.restored_semantic_word(cell, env.semantic_word(cell));
        Self {
            save_position,
            cell,
            old: restored.word,
            trace_eligible: restored.trace_eligible,
            retaining: true,
            tracing_restores: env.int_param(crate::env::banks::IntParam::TRACING_RESTORES),
            tracing_online: env.int_param(crate::env::banks::IntParam::TRACING_ONLINE),
            escape_char: env.int_param(crate::env::banks::IntParam::ESCAPE_CHAR),
            box_trace_text: None,
        }
    }

    fn restoring_box(
        env: &Env,
        save_position: usize,
        index: u16,
        old: crate::env::box_bank::BoxSlot,
    ) -> Self {
        Self {
            save_position,
            cell: CellId::new(BankTag::Box, u32::from(index)),
            old: old.value(),
            trace_eligible: true,
            retaining: false,
            tracing_restores: env.int_param(crate::env::banks::IntParam::TRACING_RESTORES),
            tracing_online: env.int_param(crate::env::banks::IntParam::TRACING_ONLINE),
            escape_char: env.int_param(crate::env::banks::IntParam::ESCAPE_CHAR),
            box_trace_text: None,
        }
    }

    #[must_use]
    pub const fn cell(&self) -> CellId {
        self.cell
    }

    pub(crate) const fn save_position(&self) -> usize {
        self.save_position
    }

    #[must_use]
    pub const fn old(&self) -> u64 {
        self.old
    }

    #[must_use]
    pub const fn trace_eligible(&self) -> bool {
        self.trace_eligible
    }

    #[must_use]
    pub const fn is_retaining(&self) -> bool {
        self.retaining
    }

    pub const fn tracing_restores(&self) -> i32 {
        self.tracing_restores
    }

    pub const fn tracing_online(&self) -> i32 {
        self.tracing_online
    }

    pub const fn escape_char(&self) -> i32 {
        self.escape_char
    }

    pub(crate) fn capture_box_trace_text(&mut self, text: String) {
        debug_assert_eq!(self.cell.bank(), BankTag::Box);
        self.box_trace_text = Some(text);
    }

    fn refresh_restored_eqtb_value(&mut self, env: &Env) {
        if self.cell.bank() != BankTag::TokParam
            || self.cell.index() != u32::from(crate::env::banks::TokParam::PAR_SHAPE_INTERNAL.raw())
        {
            return;
        }
        // TeX82 §283 calls show_eqtb only after unsave has installed the
        // effective entry.  Global-record compaction can refile journal words
        // after the individual undo record was visited, so its raw `old`
        // word is not necessarily that final typed eqtb value.
        let restored = env.restored_semantic_word(self.cell, env.semantic_word(self.cell));
        self.old = restored.word;
        self.trace_eligible = true;
    }

    #[must_use]
    pub fn box_trace_text(&self) -> Option<&str> {
        self.box_trace_text.as_deref()
    }
}

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

/// Detached identity of one live TeX save-stack boundary.
///
/// e-TeX 2.6 [49.1292] traverses these records for `\showgroups` without
/// changing the live save stack. Keeping the entry line beside the boundary
/// makes that observation independent of the journal representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupFrame {
    kind: GroupKind,
    entered_line: u32,
    lineage: u64,
}

impl GroupFrame {
    #[must_use]
    pub const fn kind(self) -> GroupKind {
        self.kind
    }

    #[must_use]
    pub const fn entered_line(self) -> u32 {
        self.entered_line
    }

    #[must_use]
    pub const fn lineage(self) -> u64 {
        self.lineage
    }
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
    entered_line: u32,
    lineage: u64,
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

    /// e-TeX 2.6 [49.1293]'s `print_group` group-kind name, shared by
    /// `\showgroups` and `\tracinggroups`'s `{entering ...}`/`{leaving ...}`
    /// display.
    #[must_use]
    pub const fn group_text(self) -> &'static str {
        match self {
            Self::Simple => "simple group",
            Self::HBox => "hbox group",
            Self::AdjustedHBox => "adjusted hbox group",
            Self::VBox => "vbox group",
            Self::VTop => "vtop group",
            Self::Align => "align group",
            Self::NoAlign => "no align group",
            Self::Output => "output group",
            Self::Math => "math group",
            Self::Disc => "disc group",
            Self::Insert => "insert group",
            Self::VCenter => "vcenter group",
            Self::MathChoice => "math choice group",
            Self::SemiSimple => "semi simple group",
            Self::MathShift => "math shift group",
            Self::MathLeft => "math left group",
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
    enclosing_group_lineage: Option<u64>,
    epoch: crate::epoch::Epoch,
}

impl EnvSnapshot {
    /// Returns the journal position captured by this snapshot.
    #[must_use]
    pub(crate) const fn journal_pos(self) -> JournalPos {
        self.journal_pos
    }

    /// Returns the epoch captured by this snapshot.
    #[must_use]
    pub(crate) const fn epoch(self) -> crate::epoch::Epoch {
        self.epoch
    }
}

impl Env {
    /// Reconstructs the live TeX82 `save_ptr` represented by the typed group
    /// journal. This is diagnostic accounting only: journal storage remains
    /// the semantic owner, and §1334's high-water mark is kept outside it.
    pub(crate) fn canonical_save_stack_words(&self) -> usize {
        let mut words = 0_usize;
        let mut locally_saved = Vec::<AHashSet<CellId>>::new();
        for index in 0..self.journal.len() {
            match self.journal.entry(index) {
                Entry::Marker(Marker::Group { .. }) => {
                    words = words.saturating_add(1);
                    locally_saved.push(AHashSet::new());
                }
                Entry::Undo(rec) => {
                    let Some(saved) = locally_saved.last_mut() else {
                        continue;
                    };
                    let cell = rec.cell().without_assignment_scope();
                    if rec.cell().is_global() {
                        saved.remove(&cell);
                    } else if saved.insert(cell) {
                        // TeX82 §§275--276 represents `restore_zero` in one
                        // word, while `restore_old_value` occupies two.
                        let restore_words = if cell.bank() == BankTag::Meaning
                            && rec.old() == Meaning::Undefined.encode()
                        {
                            1
                        } else {
                            2
                        };
                        words = words.saturating_add(restore_words);
                    }
                }
                Entry::BoxUndo(id) => {
                    let Some(saved) = locally_saved.last_mut() else {
                        continue;
                    };
                    let rec = self.journal.box_undo(id);
                    let cell = CellId::new(BankTag::Box, u32::from(rec.index()));
                    if rec.is_global() {
                        saved.remove(&cell);
                    } else if saved.insert(cell) {
                        words = words.saturating_add(2);
                    }
                }
                Entry::Marker(Marker::Checkpoint(_)) => {}
            }
        }
        words.saturating_add(self.aftergroup.len())
    }

    /// Opens a journal slice whose first write cannot coalesce with work that
    /// preceded the mark.
    pub(crate) fn begin_journal_region(&mut self) -> super::JournalRegionMark {
        self.bump_epoch();
        super::JournalRegionMark {
            journal_pos: self.journal.pos(),
            lineage: self.journal_lineage,
        }
    }

    /// Returns the canonical cells written since `mark`.
    ///
    /// Checkpoints, group exits, and rollback advance the environment epoch.
    /// Rejecting such a region keeps destructive journal compaction from
    /// silently producing an incomplete write footprint.
    pub(crate) fn journal_region_cells(
        &self,
        mark: super::JournalRegionMark,
    ) -> Result<Vec<CellId>, super::JournalRegionInvalidated> {
        if self.journal_lineage != mark.lineage || mark.journal_pos > self.journal.pos() {
            return Err(super::JournalRegionInvalidated);
        }
        let mut cells = self
            .journal
            .entries_since(mark.journal_pos)
            .iter()
            .filter_map(|entry| match *entry {
                Entry::Undo(rec) => Some(rec.cell().without_assignment_scope()),
                Entry::BoxUndo(id) => Some(CellId::new(
                    BankTag::Box,
                    u32::from(self.journal.box_undo(id).index()),
                )),
                Entry::Marker(_) => None,
            })
            .collect::<Vec<_>>();
        cells.sort_unstable();
        cells.dedup();
        Ok(cells)
    }

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
            enclosing_group_lineage: self
                .group_boundaries
                .last()
                .map(|boundary| boundary.lineage),
            epoch: self.epoch,
        };
        self.epoch.bump();
        self.journal_lineage = self
            .journal_lineage
            .checked_add(1)
            .expect("environment journal lineage exhausted");
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

    /// Returns whether `snapshot` is an ancestor of the checked-out group stack.
    ///
    /// A rollback may unwind groups entered after capture. It may not resurrect
    /// a group that was exited, even if later groups happen to recreate the same
    /// depth and journal position.
    #[must_use]
    pub(crate) fn can_rollback_to(&self, snapshot: EnvSnapshot) -> bool {
        let Ok(boundary_len) = usize::try_from(snapshot.group_boundary_len) else {
            return false;
        };
        if boundary_len > self.group_boundaries.len() {
            return false;
        }
        match snapshot.enclosing_group_lineage {
            Some(lineage) => self
                .group_boundaries
                .get(boundary_len.saturating_sub(1))
                .is_some_and(|boundary| boundary.lineage == lineage),
            None => boundary_len == 0,
        }
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

    #[must_use]
    pub(crate) fn group_frames(&self) -> impl DoubleEndedIterator<Item = GroupFrame> + '_ {
        self.group_boundaries.iter().map(|boundary| GroupFrame {
            kind: boundary.kind,
            entered_line: boundary.entered_line,
            lineage: boundary.lineage,
        })
    }

    /// Enters a TeX group.
    pub(crate) fn enter_group(&mut self) {
        self.enter_group_with_kind(GroupKind::Simple);
    }

    /// Enters a TeX group with an explicit boundary kind.
    pub(crate) fn enter_group_with_kind(&mut self, kind: GroupKind) {
        self.enter_group_with_kind_at_line(kind, 0);
    }

    /// Enters a TeX group and records e-TeX's `saved(-1)` source line.
    pub(crate) fn enter_group_with_kind_at_line(&mut self, kind: GroupKind, entered_line: u32) {
        let aftergroup_start = u32_len(
            self.aftergroup.len(),
            "aftergroup payload list exceeds u32 entries",
        );
        let marker_pos = self.journal.pos();
        let box_undo_len = self.journal.box_undo_len();
        let lineage = self.next_group_lineage;
        self.next_group_lineage = self
            .next_group_lineage
            .checked_add(1)
            .expect("group lineage exceeds u64 entries");
        self.journal.push_marker(Marker::Group {
            aftergroup_start,
            kind,
        });
        self.group_boundaries.push(GroupBoundary {
            marker_pos,
            box_undo_len,
            aftergroup_start,
            kind,
            entered_line,
            lineage,
        });
        self.group_depth = self
            .group_depth
            .checked_add(1)
            .expect("group depth exceeds u32 entries");
        self.epoch.bump();
    }

    /// Pushes an opaque `\aftergroup` payload for the current group.
    pub(crate) fn push_aftergroup(&mut self, payload: Token) {
        self.push_aftergroup_traced(TracedTokenWord::pack(
            payload,
            crate::token::OriginId::UNKNOWN,
        ));
    }

    pub(crate) fn push_aftergroup_traced(&mut self, payload: TracedTokenWord) {
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
        self.leave_group_unchecked()
            .0
            .into_iter()
            .map(TracedTokenWord::semantic_token)
            .collect()
    }

    /// Leaves the innermost group and reports whether meaning cells were
    /// restored or compacted while crossing the boundary.
    #[must_use]
    pub(crate) fn leave_group_observing_meanings(
        &mut self,
    ) -> (
        Vec<TracedTokenWord>,
        bool,
        MutationReceipts,
        Vec<RestoreRecord>,
    ) {
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
        Ok(self
            .leave_group_unchecked()
            .0
            .into_iter()
            .map(TracedTokenWord::semantic_token)
            .collect())
    }

    pub(crate) fn leave_group_with_kind_observing_meanings(
        &mut self,
        expected: GroupKind,
    ) -> Result<
        (
            Vec<TracedTokenWord>,
            bool,
            MutationReceipts,
            Vec<RestoreRecord>,
        ),
        GroupMismatch,
    > {
        let Some(actual) = self.innermost_group_kind() else {
            return Err(GroupMismatch::new_no_group(expected));
        };
        if actual != expected {
            return Err(GroupMismatch::new(expected, actual));
        }
        Ok(self.leave_group_unchecked())
    }

    fn leave_group_unchecked(
        &mut self,
    ) -> (
        Vec<TracedTokenWord>,
        bool,
        MutationReceipts,
        Vec<RestoreRecord>,
    ) {
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
        let mut candidate_cells = (marker_index + 1..group_end)
            .filter_map(|index| match self.journal.entry(index) {
                Entry::Undo(rec) => Some(rec.cell()),
                Entry::BoxUndo(id) => Some(crate::cell::CellId::new(
                    BankTag::Box,
                    u32::from(self.journal.box_undo(id).index()),
                )),
                Entry::Marker(_) => None,
            })
            .map(CellId::without_assignment_scope)
            .collect::<SmallVec<[CellId; 8]>>();
        candidate_cells.sort_unstable();
        candidate_cells.dedup();
        let before_words = candidate_cells
            .iter()
            .copied()
            .map(|cell| (cell, self.semantic_word(cell)))
            .collect::<AHashMap<_, _>>();
        let retained_cells = (marker_index + 1..group_end)
            .filter_map(|index| match self.journal.entry(index) {
                Entry::Undo(rec) if rec.cell().is_global() => {
                    Some(rec.cell().without_assignment_scope())
                }
                Entry::BoxUndo(id) if self.journal.box_undo(id).survives_group(leaving_depth) => {
                    Some(CellId::new(
                        BankTag::Box,
                        u32::from(self.journal.box_undo(id).index()),
                    ))
                }
                Entry::Undo(_) | Entry::BoxUndo(_) | Entry::Marker(_) => None,
            })
            .collect::<AHashSet<_>>();
        let has_globals =
            (marker_index + 1..group_end).any(|index| match self.journal.entry(index) {
                Entry::Undo(rec) => rec.cell().is_global(),
                Entry::BoxUndo(id) => self.journal.box_undo(id).survives_group(leaving_depth),
                Entry::Marker(_) => false,
            });
        let mut restores = Vec::new();
        // TeX's `eq_define` saves a cell only on its first local assignment
        // at the current level. Our journal deliberately records every write,
        // so select the entries that correspond to real save-stack words for
        // `\tracingrestores`; a global write starts a new local run.
        let mut locally_saved = AHashSet::new();
        let mut traced_local_entries = AHashSet::new();
        for index in marker_index + 1..group_end {
            let Entry::Undo(rec) = self.journal.entry(index) else {
                continue;
            };
            let key = cell_key(rec.cell());
            if rec.cell().is_global() {
                locally_saved.remove(&key);
            } else if locally_saved.insert(key) {
                traced_local_entries.insert(index);
            }
        }
        let meaning_changed = if has_globals {
            self.leave_group_with_globals(
                marker_index,
                group_end,
                boundary.box_undo_len,
                leaving_depth,
                &traced_local_entries,
                &mut restores,
            )
        } else {
            let mut meaning_changed = false;
            for index in (marker_index + 1..group_end).rev() {
                if let Entry::Undo(rec) = self.journal.entry(index) {
                    meaning_changed |= rec.cell().bank() == BankTag::Meaning;
                    self.restore_raw(rec.cell(), rec.old());
                    if traced_local_entries.contains(&index) {
                        restores.push(RestoreRecord::restoring(self, index, rec.cell(), rec.old()));
                    }
                } else if let Entry::BoxUndo(id) = self.journal.entry(index) {
                    let rec = self.journal.box_undo(id);
                    self.boxes.restore(rec.index(), rec.old());
                    restores.push(RestoreRecord::restoring_box(
                        self,
                        index,
                        rec.index(),
                        rec.old(),
                    ));
                }
            }
            self.journal.truncate_to(marker_pos);
            self.journal.truncate_box_undos(boundary.box_undo_len);
            meaning_changed
        };
        let mut traced_boxes = SmallVec::<[u32; 8]>::new();
        restores.retain(|record| {
            if record.cell().bank() != BankTag::Box {
                return true;
            }
            let index = record.cell().index();
            if traced_boxes.contains(&index) {
                false
            } else {
                traced_boxes.push(index);
                true
            }
        });
        reorder_sparse_register_restores(&mut restores);
        for restore in &mut restores {
            restore.refresh_restored_eqtb_value(self);
        }

        let aftergroup_start = checked_aftergroup_start(aftergroup_start, self.aftergroup.len());
        let payloads = self.aftergroup.drain(aftergroup_start..).collect();

        // core_state.md §6 / 97a3c1d: restore leaves stamps high, so group
        // exit must start a fresh epoch or the enclosing undo slice can be
        // corrupted by a later write to the same restored cell.
        self.epoch.bump();
        self.journal_lineage = self
            .journal_lineage
            .checked_add(1)
            .expect("environment journal lineage exhausted");
        let receipts = candidate_cells
            .into_iter()
            .map(|cell| {
                crate::env::CellMutationReceipt::restore(
                    cell,
                    before_words[&cell],
                    self.semantic_word(cell),
                    retained_cells.contains(&cell),
                )
            })
            .collect();
        (payloads, meaning_changed, receipts, restores)
    }

    fn leave_group_with_globals(
        &mut self,
        marker_index: usize,
        group_end: usize,
        box_undo_len: u32,
        leaving_depth: u32,
        traced_local_entries: &AHashSet<usize>,
        restores: &mut Vec<RestoreRecord>,
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
                        if traced_local_entries.contains(&index) {
                            restores.push(RestoreRecord::restoring(
                                self,
                                index,
                                rec.cell(),
                                rec.old(),
                            ));
                        }
                    } else if traced_local_entries.contains(&index) {
                        restores.push(RestoreRecord::retaining(self, index, rec.cell()));
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
                            restores.push(RestoreRecord::restoring_box(
                                self,
                                index,
                                rec.index(),
                                rec.old(),
                            ));
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
    pub(crate) fn rollback_to(&mut self, snapshot: EnvSnapshot) -> MutationReceipts {
        let snapshot_index = snapshot.journal_pos.raw() as usize;
        let rollback_end = self.journal.len();
        let mut candidate_cells = (snapshot_index..rollback_end)
            .filter_map(|index| match self.journal.entry(index) {
                Entry::Undo(rec) => Some(rec.cell().without_assignment_scope()),
                Entry::BoxUndo(id) => Some(CellId::new(
                    BankTag::Box,
                    u32::from(self.journal.box_undo(id).index()),
                )),
                Entry::Marker(_) => None,
            })
            .collect::<SmallVec<[CellId; 8]>>();
        candidate_cells.sort_unstable();
        candidate_cells.dedup();
        let before_words = candidate_cells
            .iter()
            .copied()
            .map(|cell| (cell, self.semantic_word(cell)))
            .collect::<AHashMap<_, _>>();
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
        self.journal_lineage = self
            .journal_lineage
            .checked_add(1)
            .expect("environment journal lineage exhausted");
        candidate_cells
            .into_iter()
            .map(|cell| {
                crate::env::CellMutationReceipt::restore(
                    cell,
                    before_words[&cell],
                    self.semantic_word(cell),
                    false,
                )
            })
            .collect()
    }
}

/// e-TeX [49.1221--1224] represents all extended registers through one
/// save-stack sparse-array boundary. TeX82 §283 therefore restores ordinary
/// entries above that boundary first, then drains extended-register entries
/// together in reverse assignment order.
fn reorder_sparse_register_restores(restores: &mut Vec<RestoreRecord>) {
    fn is_sparse_register(record: &RestoreRecord) -> bool {
        matches!(
            record.cell().bank(),
            BankTag::Count | BankTag::Dimen | BankTag::Skip | BankTag::Muskip | BankTag::Toks
        ) && record.cell().index() > 255
    }

    let Some(last_sparse) = restores.iter().rposition(is_sparse_register) else {
        return;
    };
    let insertion = restores[..last_sparse]
        .iter()
        .filter(|record| !is_sparse_register(record))
        .count();
    let mut sparse = Vec::new();
    restores.retain(|record| {
        if is_sparse_register(record) {
            sparse.push(record.clone());
            false
        } else {
            true
        }
    });
    restores.splice(insertion..insertion, sparse);
}
