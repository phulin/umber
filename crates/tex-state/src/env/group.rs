use super::{Env, cell_key, checked_aftergroup_start, u32_len};
use crate::cell::{BankTag, CellId};
use crate::glue::GlueSpecRef;
use crate::journal::{
    BoxUndoRec, Entry, GlueUndoRoots, JournalPos, MacroUndoRoots, Marker, TokenUndoRoots, UndoRec,
};
use crate::macro_store::MacroDefinitionRef;
use crate::token::Token;
use crate::token_store::TokenListRef;
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
    box_root: Option<crate::node_arena::NodeListRef>,
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
            box_root: None,
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
            box_root: None,
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
            box_root: old.root(),
            box_trace_text: None,
        }
    }

    fn retaining_box(env: &Env, save_position: usize, index: u16) -> Self {
        let current = env.boxes.get(index);
        Self {
            save_position,
            cell: CellId::new(BankTag::Box, u32::from(index)),
            old: current.value(),
            trace_eligible: true,
            retaining: true,
            tracing_restores: env.int_param(crate::env::banks::IntParam::TRACING_RESTORES),
            tracing_online: env.int_param(crate::env::banks::IntParam::TRACING_ONLINE),
            escape_char: env.int_param(crate::env::banks::IntParam::ESCAPE_CHAR),
            box_root: current.root(),
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
        self.box_root = None;
        self.box_trace_text = Some(text);
    }

    pub(crate) fn box_root(&self) -> Option<&crate::node_arena::NodeListRef> {
        self.box_root.as_ref()
    }

    fn refresh_restored_shape_value(&mut self, env: &Env) {
        use crate::env::banks::TokParam;

        if self.cell.bank() != BankTag::TokParam
            || ![
                TokParam::INTER_LINE_PENALTIES_INTERNAL,
                TokParam::CLUB_PENALTIES_INTERNAL,
                TokParam::WIDOW_PENALTIES_INTERNAL,
                TokParam::DISPLAY_WIDOW_PENALTIES_INTERNAL,
                TokParam::PAR_SHAPE_INTERNAL,
            ]
            .iter()
            .any(|param| self.cell.index() == u32::from(param.raw()))
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

#[derive(Clone, Debug)]
struct OwnedWord {
    word: u64,
    token_root: Option<TokenListRef>,
    macro_root: Option<MacroDefinitionRef>,
    glue_root: Option<GlueSpecRef>,
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
#[derive(Debug)]
pub(crate) struct EnvSnapshot {
    journal_pos: JournalPos,
    box_undo_len: u32,
    aftergroup_len: u32,
    afterassignment: Option<Token>,
    group_depth: u32,
    group_boundary_len: u32,
    enclosing_group_lineage: Option<u64>,
    epoch: crate::epoch::Epoch,
    rollback_roots: Option<std::sync::Arc<super::JournalRollbackRoots>>,
    journal_baseline_serial: u64,
}

impl Clone for EnvSnapshot {
    fn clone(&self) -> Self {
        Self {
            journal_pos: self.journal_pos,
            box_undo_len: self.box_undo_len,
            aftergroup_len: self.aftergroup_len,
            afterassignment: self.afterassignment,
            group_depth: self.group_depth,
            group_boundary_len: self.group_boundary_len,
            enclosing_group_lineage: self.enclosing_group_lineage,
            epoch: self.epoch,
            rollback_roots: self
                .rollback_roots
                .as_ref()
                .map(super::JournalRollbackRoots::register),
            journal_baseline_serial: self.journal_baseline_serial,
        }
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        if let Some(roots) = &self.rollback_roots {
            roots.unregister();
        }
    }
}

impl EnvSnapshot {
    /// Returns the journal position captured by this snapshot.
    #[must_use]
    pub(crate) const fn journal_pos(&self) -> JournalPos {
        self.journal_pos
    }

    /// Returns the epoch captured by this snapshot.
    #[must_use]
    pub(crate) const fn epoch(&self) -> crate::epoch::Epoch {
        self.epoch
    }

    pub(crate) const fn journal_baseline_serial(&self) -> u64 {
        self.journal_baseline_serial
    }
}

impl Env {
    /// Returns the live TeX82 `save_ptr` represented by the typed group
    /// journal. This is diagnostic accounting only: the journal owns the
    /// rollback-coupled word projection, and §1334's high-water mark remains
    /// outside it.
    #[cfg(test)]
    pub(crate) fn canonical_save_stack_words(&self, save_group_source_lines: bool) -> usize {
        self.canonical_save_stack_projection(save_group_source_lines)
            .0
    }

    pub(crate) fn canonical_save_stack_projection(
        &self,
        save_group_source_lines: bool,
    ) -> (usize, Option<(usize, usize)>) {
        // TeX82 §§273/275 update max_save_stack before pushing the newest
        // physical record. Return both the completed live depth and that
        // record's (journal-end position, word count), so the aggregate owner
        // can reconstruct the checked depth across split typed stores.
        let (mut words, latest_push) = self.journal.canonical_save_stack_projection();
        if save_group_source_lines {
            // e-TeX [19.274] stores one source-line word before each level
            // boundary. TeX82 §273 samples `save_ptr` before the innermost
            // boundary is installed, so only its enclosing groups contribute
            // line words to this checked high-water projection.
            words = words.saturating_add(self.group_boundaries.len().saturating_sub(1));
        }
        words = words.saturating_add(self.aftergroup.len());
        (words, latest_push)
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
            rollback_roots: (self.group_depth == 0).then(|| self.journal_rollback_roots.register()),
            journal_baseline_serial: self.journal_baseline_serial,
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

    #[cfg(test)]
    pub(crate) fn last_group_marker_pos(&self) -> Option<JournalPos> {
        self.group_boundaries
            .last()
            .map(|boundary| boundary.marker_pos)
    }

    #[must_use]
    pub(crate) fn current_journal_pos(&self) -> JournalPos {
        self.journal.pos()
    }

    pub(crate) const fn journal_baseline_serial(&self) -> u64 {
        self.journal_baseline_serial
    }

    pub(crate) fn journal_retained_bytes(&self) -> usize {
        self.journal.retained_bytes()
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn journal_entry_count(&self) -> usize {
        self.journal.len()
    }

    /// Returns whether `snapshot` is an ancestor of the checked-out group stack.
    ///
    /// A rollback may unwind groups entered after capture. It may not resurrect
    /// a group that was exited, even if later groups happen to recreate the same
    /// depth and journal position.
    #[must_use]
    pub(crate) fn can_rollback_to(&self, snapshot: &EnvSnapshot) -> bool {
        if snapshot.journal_baseline_serial != self.journal_baseline_serial
            || snapshot
                .rollback_roots
                .as_ref()
                .is_some_and(|roots| !std::sync::Arc::ptr_eq(&self.journal_rollback_roots, roots))
        {
            return false;
        }
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

    /// Retargets an inherited snapshot onto an independently owned fork.
    pub(crate) fn retarget_snapshot(&self, snapshot: &EnvSnapshot) -> EnvSnapshot {
        EnvSnapshot {
            journal_pos: snapshot.journal_pos,
            box_undo_len: snapshot.box_undo_len,
            aftergroup_len: snapshot.aftergroup_len,
            afterassignment: snapshot.afterassignment,
            group_depth: snapshot.group_depth,
            group_boundary_len: snapshot.group_boundary_len,
            enclosing_group_lineage: snapshot.enclosing_group_lineage,
            epoch: snapshot.epoch,
            rollback_roots: (snapshot.group_depth == 0)
                .then(|| self.journal_rollback_roots.register()),
            journal_baseline_serial: self.journal_baseline_serial,
        }
    }

    /// Starts a fork with no rollback capabilities inherited from its source.
    pub(crate) fn reset_snapshot_roots_for_fork(&mut self) {
        self.journal_rollback_roots = std::sync::Arc::new(super::JournalRollbackRoots::default());
    }

    /// Opens one direct executor operation without registering a rollback root.
    pub(crate) fn begin_direct_operation(&mut self) -> super::DirectJournalMark {
        self.bump_epoch();
        super::DirectJournalMark {
            journal_pos: self.journal.pos(),
            lineage: self.journal_lineage,
        }
    }

    /// Returns whether this operation, rather than earlier setup, changed the
    /// current journal suffix or crossed a journal lineage boundary.
    pub(crate) fn direct_operation_changed(&self, mark: super::DirectJournalMark) -> bool {
        self.journal.pos() != mark.journal_pos || self.journal_lineage != mark.lineage
    }

    pub(crate) fn can_retire_direct_operation(&self) -> bool {
        self.group_depth == 0
            && self.journal.len() != 0
            && self.can_discard_direct_derived_history()
    }

    /// Returns whether the current journal baseline has no aggregate rollback
    /// root.
    ///
    /// Derived mutation journals may discard their suffix at this boundary
    /// even while an open TeX group keeps Env's own save stack live.
    pub(crate) fn can_discard_direct_derived_history(&self) -> bool {
        self.journal_rollback_roots.is_only(0)
    }

    /// Retires closed journal history after one successful direct operation.
    /// Retained checkpoints and open TeX groups keep their exact history.
    pub(crate) fn retire_direct_operation(&mut self) -> Option<()> {
        if !self.can_retire_direct_operation() {
            return None;
        }
        self.journal.clear_committed();
        self.journal_rollback_roots = std::sync::Arc::new(super::JournalRollbackRoots::default());
        self.journal_baseline_serial = self
            .journal_baseline_serial
            .checked_add(1)
            .expect("environment journal baseline serial exhausted");
        self.journal_lineage = self
            .journal_lineage
            .checked_add(1)
            .expect("environment journal lineage exhausted");
        Some(())
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
        self.push_aftergroup_traced(crate::token::RootedTracedTokenWord::new(
            payload,
            crate::provenance::OriginRef::unknown(),
        ));
    }

    pub(crate) fn push_aftergroup_traced(&mut self, payload: crate::token::RootedTracedTokenWord) {
        if self.group_depth != 0 {
            self.aftergroup.push(payload);
            self.journal.push_marker(Marker::Aftergroup);
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
            .map(|word| word.word().semantic_token())
            .collect()
    }

    /// Leaves the innermost group and reports whether meaning cells were
    /// restored or compacted while crossing the boundary.
    #[must_use]
    pub(crate) fn leave_group_observing_meanings(
        &mut self,
    ) -> (
        Vec<crate::token::RootedTracedTokenWord>,
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
            .map(|word| word.word().semantic_token())
            .collect())
    }

    pub(crate) fn leave_group_with_kind_observing_meanings(
        &mut self,
        expected: GroupKind,
    ) -> Result<
        (
            Vec<crate::token::RootedTracedTokenWord>,
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
        Vec<crate::token::RootedTracedTokenWord>,
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
        // TeX82 §275's `eq_define` and e-TeX [53a]'s `sa_def` save a cell only
        // on its first local assignment at the current level. Our journal
        // deliberately records additional box writes after nested box groups
        // advance the epoch, so select only entries that correspond to real
        // save-stack words for `\tracingrestores`; a global write starts a new
        // local run.
        let mut locally_saved = AHashSet::new();
        let mut traced_local_entries = AHashSet::new();
        for index in marker_index + 1..group_end {
            match self.journal.entry(index) {
                Entry::Undo(rec) => {
                    let key = cell_key(rec.cell());
                    if rec.cell().is_global() {
                        locally_saved.remove(&key);
                    } else if locally_saved.insert(key) {
                        traced_local_entries.insert(index);
                    }
                }
                Entry::BoxUndo(id) => {
                    let rec = self.journal.box_undo(id);
                    let key = cell_key(CellId::new(BankTag::Box, u32::from(rec.index())));
                    if rec.is_global() {
                        locally_saved.remove(&key);
                    } else if locally_saved.insert(key) {
                        traced_local_entries.insert(index);
                    }
                }
                Entry::Marker(_) => {}
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
                    let root = self
                        .journal
                        .token_undo_roots(index)
                        .and_then(TokenUndoRoots::old);
                    let macro_root = self
                        .journal
                        .macro_undo_roots(index)
                        .and_then(MacroUndoRoots::old);
                    let glue_root = self
                        .journal
                        .glue_undo_roots(index)
                        .and_then(GlueUndoRoots::old);
                    self.restore_raw_with_owners(
                        rec.cell(),
                        rec.old(),
                        root,
                        macro_root,
                        glue_root,
                        None,
                    );
                    if traced_local_entries.contains(&index) {
                        restores.push(RestoreRecord::restoring(self, index, rec.cell(), rec.old()));
                    }
                } else if let Entry::BoxUndo(id) = self.journal.entry(index) {
                    let rec = self.journal.box_undo(id);
                    self.boxes.restore(rec.index(), rec.old());
                    if traced_local_entries.contains(&index) {
                        restores.push(RestoreRecord::restoring_box(
                            self,
                            index,
                            rec.index(),
                            rec.old(),
                        ));
                    }
                }
            }
            self.journal.truncate_to(marker_pos);
            self.journal.truncate_box_undos(boundary.box_undo_len);
            meaning_changed
        };
        reorder_sparse_register_restores(&mut restores);
        for restore in &mut restores {
            restore.refresh_restored_shape_value(self);
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
                cell_states.entry(cell_key(rec.cell())).or_insert_with(|| {
                    GlobalCompactionState::new(OwnedWord {
                        word: rec.old(),
                        token_root: self
                            .journal
                            .token_undo_roots(index)
                            .and_then(TokenUndoRoots::old),
                        macro_root: self
                            .journal
                            .macro_undo_roots(index)
                            .and_then(MacroUndoRoots::old),
                        glue_root: self
                            .journal
                            .glue_undo_roots(index)
                            .and_then(GlueUndoRoots::old),
                    })
                });
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
                    globals.push((
                        rec,
                        self.journal.token_undo_roots(index).cloned(),
                        self.journal.macro_undo_roots(index).cloned(),
                        self.journal.glue_undo_roots(index).cloned(),
                    ));
                }
                Entry::Undo(rec) => {
                    let state = cell_states
                        .get(&cell_key(rec.cell()))
                        .expect("journal cell was indexed before group compaction");
                    if !state.has_later_global {
                        meaning_changed |= rec.cell().bank() == BankTag::Meaning;
                        let root = self
                            .journal
                            .token_undo_roots(index)
                            .and_then(TokenUndoRoots::old);
                        let macro_root = self
                            .journal
                            .macro_undo_roots(index)
                            .and_then(MacroUndoRoots::old);
                        let glue_root = self
                            .journal
                            .glue_undo_roots(index)
                            .and_then(GlueUndoRoots::old);
                        self.restore_raw_with_owners(
                            rec.cell(),
                            rec.old(),
                            root,
                            macro_root,
                            glue_root,
                            None,
                        );
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
                            if traced_local_entries.contains(&index) {
                                restores.push(RestoreRecord::restoring_box(
                                    self,
                                    index,
                                    rec.index(),
                                    rec.old(),
                                ));
                            }
                        } else if traced_local_entries.contains(&index) {
                            restores.push(RestoreRecord::retaining_box(self, index, rec.index()));
                        }
                    }
                }
                Entry::Marker(Marker::Aftergroup | Marker::Checkpoint(_)) => {}
                Entry::Marker(Marker::Group { .. }) => {
                    unreachable!("group slice starts after the marker")
                }
            }
        }

        self.journal.truncate_to(JournalPos::from_raw(marker_index));
        self.journal.truncate_box_undos(box_undo_len);
        for (rec, roots, macro_roots, glue_roots) in globals.into_iter().rev() {
            meaning_changed |= rec.cell().bank() == BankTag::Meaning;
            let new_root = roots.as_ref().and_then(TokenUndoRoots::new_value);
            let new_macro_root = macro_roots.as_ref().and_then(MacroUndoRoots::new_value);
            let new_glue_root = glue_roots.as_ref().and_then(GlueUndoRoots::new_value);
            self.restore_raw_with_owners(
                rec.cell(),
                rec.new_value(),
                new_root,
                new_macro_root,
                new_glue_root,
                None,
            );
            let key = cell_key(rec.cell());
            let state = cell_states
                .get_mut(&key)
                .expect("journal cell was indexed before group compaction");
            let old = if state.refiled {
                OwnedWord {
                    word: rec.old(),
                    token_root: roots.as_ref().and_then(TokenUndoRoots::old),
                    macro_root: macro_roots.as_ref().and_then(MacroUndoRoots::old),
                    glue_root: glue_roots.as_ref().and_then(GlueUndoRoots::old),
                }
            } else {
                state.refiled = true;
                state.first_old.clone()
            };
            let pos = self
                .journal
                .push_undo(UndoRec::new(rec.cell(), old.word, rec.new_value()));
            if matches!(rec.cell().bank(), BankTag::Toks | BankTag::TokParam) {
                self.journal
                    .attach_token_undo_roots(pos, TokenUndoRoots::new(old.token_root, new_root));
            } else if rec.cell().bank() == BankTag::Meaning {
                self.journal.attach_macro_undo_roots(
                    pos,
                    MacroUndoRoots::new(old.macro_root, new_macro_root),
                );
            } else if matches!(
                rec.cell().bank(),
                BankTag::Skip | BankTag::Muskip | BankTag::GlueParam
            ) {
                self.journal
                    .attach_glue_undo_roots(pos, GlueUndoRoots::new(old.glue_root, new_glue_root));
            }
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
                state.first_old.clone()
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
                let root = self
                    .journal
                    .token_undo_roots(index)
                    .and_then(TokenUndoRoots::old);
                let macro_root = self
                    .journal
                    .macro_undo_roots(index)
                    .and_then(MacroUndoRoots::old);
                let glue_root = self
                    .journal
                    .glue_undo_roots(index)
                    .and_then(GlueUndoRoots::old);
                self.restore_raw_with_owners(
                    rec.cell(),
                    rec.old(),
                    root,
                    macro_root,
                    glue_root,
                    None,
                );
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
