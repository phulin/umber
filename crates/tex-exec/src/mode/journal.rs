use super::{ModeLevelSummary, ModeList, ModeNest, ModeNestStorage};

#[cfg(test)]
use super::AlignState;

const MAX_LIVE_LEVELS: usize = 41;
// Four command-attempt frames may nest inside the retained named-boundary
// frames. The aggregate contract retains at most the prior/current boundary
// lineages, each with the editor's bounded boundary ring.
const MAX_JOURNAL_FRAMES: usize = 68;
const FIELD_COUNT: usize = 12;
const UNRECORDED: usize = usize::MAX;

const NODES: usize = 0;
#[cfg(test)]
const ALIGN_STATE: usize = 1;
const INCOMPLETE_FRACTION: usize = 2;
const DISPLAY_INTERRUPT: usize = 3;
const DISPLAY_EQ_NO: usize = 4;
const DISPLAY_ALIGNMENT: usize = 5;
const PREV_DEPTH: usize = 6;
const PREV_GRAF: usize = 7;
const PENDING_HCHARS: usize = 8;
const SPACE_FACTOR: usize = 9;
const NO_BOUNDARY: usize = 10;
const HYPHEN_CONTEXT: usize = 11;

#[derive(Clone, Copy)]
struct ListProjection {
    id: u64,
    page_region: Option<tex_state::node_region::NodeRegionId>,
    root: tex_state::node_arena::PageListId,
    list_semantic_identity_root: u64,
    component_roots: super::ModeComponentRoots,
    inverse_positions: [usize; FIELD_COUNT],
}

impl ListProjection {
    fn capture(id: u64, list: &ModeList) -> Self {
        Self {
            id,
            page_region: list.page_region,
            root: list.nodes,
            list_semantic_identity_root: list.semantic_identity_root,
            component_roots: list.component_roots,
            inverse_positions: [UNRECORDED; FIELD_COUNT],
        }
    }
}

#[derive(Clone)]
pub(super) struct PendingHRunProjection {
    first: super::PendingHChar,
    current: super::PendingHRunChar,
    insertion_index: usize,
    source_len: usize,
    script: tex_fonts::Script,
    source_identity_root: u64,
    semantic_identity_root: u64,
}

impl PendingHRunProjection {
    pub(super) fn capture(run: &super::PendingHRun) -> Self {
        Self {
            first: run.first.clone(),
            current: run.current.clone(),
            insertion_index: run.insertion_index,
            source_len: run.source.len(),
            script: run.script,
            source_identity_root: run.source_identity_root,
            semantic_identity_root: run.semantic_identity_root,
        }
    }

    fn restore(self, run: &mut super::PendingHRun) {
        run.first = self.first;
        run.current = self.current;
        run.insertion_index = self.insertion_index;
        run.source.truncate(self.source_len);
        run.script = self.script;
        run.source_identity_root = self.source_identity_root;
        run.semantic_identity_root = self.semantic_identity_root;
    }
}

struct Frame {
    generation: u64,
    id: u64,
    cursor: usize,
    projection_start: usize,
}

#[expect(
    clippy::large_enum_variant,
    reason = "rollback values stay move-only and inline; boxing a popped mode level would add a per-operation heap owner"
)]
enum Inverse {
    ListRoot {
        level_id: u64,
        old_region: Option<tex_state::node_region::NodeRegionId>,
        old: tex_state::node_arena::PageListId,
    },
    #[cfg(test)]
    AlignState {
        level_id: u64,
        old: Option<AlignState>,
    },
    IncompleteFraction {
        level_id: u64,
        old: Option<super::IncompleteFraction>,
    },
    DisplayInterrupt {
        level_id: u64,
        old: Option<super::DisplayInterrupt>,
    },
    DisplayEqNo {
        level_id: u64,
        old: Option<super::DisplayEqNo>,
    },
    DisplayAlignment {
        level_id: u64,
        old: bool,
    },
    PrevDepth {
        level_id: u64,
        old: Option<tex_state::scaled::Scaled>,
    },
    PrevGraf {
        level_id: u64,
        old: i32,
    },
    PendingHchars {
        level_id: u64,
        old: PendingHcharsRollback,
    },
    SpaceFactor {
        level_id: u64,
        old: i32,
    },
    NoBoundary {
        level_id: u64,
        old: bool,
    },
    HyphenContext {
        level_id: u64,
        old: (u8, u8, u8),
    },
    Push {
        level_id: u64,
    },
    Pop {
        level_id: u64,
        level: ModeLevelSummary,
    },
}

enum PendingHcharsRollback {
    Absent,
    Projection(PendingHRunProjection),
    Value(Option<super::PendingHRun>),
}

impl Inverse {
    /// Restores one operation-local inverse. Aggregate checkpoints retain no
    /// mode journal tail, so an inverse is consumed exactly once and never
    /// needs a forward-redo payload.
    fn restore(self, storage: &mut ModeNestStorage) {
        match self {
            Self::ListRoot {
                level_id,
                old_region,
                old,
            } => {
                let list = &mut storage.level_by_id_mut(level_id).list;
                list.page_region = old_region;
                list.nodes = old;
            }
            #[cfg(test)]
            Self::AlignState { level_id, old } => {
                storage.level_by_id_mut(level_id).list.align_state = old;
            }
            Self::IncompleteFraction { level_id, old } => {
                storage.level_by_id_mut(level_id).list.incomplete_fraction = old;
            }
            Self::DisplayInterrupt { level_id, old } => {
                storage.level_by_id_mut(level_id).list.display_interrupt = old;
            }
            Self::DisplayEqNo { level_id, old } => {
                storage.level_by_id_mut(level_id).list.display_eq_no = old;
            }
            Self::DisplayAlignment { level_id, old } => {
                storage.level_by_id_mut(level_id).list.display_alignment = old;
            }
            Self::PrevDepth { level_id, old } => {
                storage.level_by_id_mut(level_id).list.prev_depth = old;
            }
            Self::PrevGraf { level_id, old } => {
                storage.level_by_id_mut(level_id).list.prev_graf = old;
            }
            Self::PendingHchars { level_id, old } => {
                let pending = &mut storage.level_by_id_mut(level_id).list.pending_hchars;
                match old {
                    PendingHcharsRollback::Absent => *pending = None,
                    PendingHcharsRollback::Projection(projection) => projection.restore(
                        pending
                            .as_mut()
                            .expect("projected pending run remains in place"),
                    ),
                    PendingHcharsRollback::Value(value) => *pending = value,
                }
            }
            Self::SpaceFactor { level_id, old } => {
                storage.level_by_id_mut(level_id).list.space_factor = old;
            }
            Self::NoBoundary { level_id, old } => {
                storage.level_by_id_mut(level_id).list.no_boundary = old;
            }
            Self::HyphenContext { level_id, old } => {
                let list = &mut storage.level_by_id_mut(level_id).list;
                (
                    list.hyphen_language,
                    list.left_hyphen_min,
                    list.right_hyphen_min,
                ) = old;
            }
            Self::Push { level_id } => {
                let index = storage.level_index(level_id);
                storage.levels.remove(index);
                storage.journal.level_ids.remove(index);
            }
            Self::Pop { level_id, level } => {
                storage.levels.push(level);
                storage.journal.level_ids.push(level_id);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Cursor {
    pub(super) generation: u64,
    pub(super) frame_id: u64,
    pub(super) cursor: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CursorError {
    Disabled,
    NotInnermost,
    WrongGeneration,
}

pub(super) struct ModeJournal {
    enabled: bool,
    generation: u64,
    next_level_id: u64,
    next_frame_id: u64,
    level_ids: Vec<u64>,
    frames: Vec<Frame>,
    projections: Vec<ListProjection>,
    inverses: Vec<Inverse>,
    replay_work: u64,
}

impl ModeJournal {
    pub(super) fn has_active_frame(&self) -> bool {
        !self.frames.is_empty()
    }

    pub(super) fn enabled(level_count: usize) -> Self {
        Self::with_capacities(
            level_count,
            MAX_JOURNAL_FRAMES,
            MAX_LIVE_LEVELS * MAX_JOURNAL_FRAMES,
        )
    }

    fn with_capacities(
        level_count: usize,
        frame_capacity: usize,
        projection_capacity: usize,
    ) -> Self {
        let mut level_ids = Vec::with_capacity(MAX_LIVE_LEVELS);
        level_ids.extend(1..=level_count as u64);
        Self {
            enabled: true,
            generation: 1,
            next_level_id: level_count as u64 + 1,
            next_frame_id: 1,
            level_ids,
            frames: Vec::with_capacity(frame_capacity),
            projections: Vec::with_capacity(projection_capacity),
            inverses: Vec::with_capacity(32),
            replay_work: 0,
        }
    }

    pub(super) fn list(&mut self, index: usize) -> Option<ListJournal<'_>> {
        if !self.enabled {
            return None;
        }
        let frame = self.frames.last()?;
        let projection = self.projections.get_mut(frame.projection_start + index)?;
        if projection.id != self.level_ids[index] {
            return None;
        }
        Some(ListJournal {
            level_id: projection.id,
            inverse_positions: &mut projection.inverse_positions,
            inverses: &mut self.inverses,
        })
    }

    pub(super) fn record_level_push(&mut self) {
        if !self.enabled {
            return;
        }
        let id = self.allocate_level_id();
        self.level_ids.push(id);
        if !self.frames.is_empty() {
            self.inverses.push(Inverse::Push { level_id: id });
        }
    }

    pub(super) fn record_level_pop(&mut self, level: ModeLevelSummary) {
        if !self.enabled {
            return;
        }
        let level_id = self.level_ids.pop().expect("journal level identity exists");
        if !self.frames.is_empty() {
            self.inverses.push(Inverse::Pop { level_id, level });
        }
    }

    fn allocate_level_id(&mut self) -> u64 {
        let id = self.next_level_id;
        self.next_level_id = self
            .next_level_id
            .checked_add(1)
            .expect("mode journal id overflow");
        id
    }
}

pub(super) struct ListJournal<'a> {
    level_id: u64,
    inverse_positions: &'a mut [usize; FIELD_COUNT],
    inverses: &'a mut Vec<Inverse>,
}

/// Writes the concrete inverse variant at its producing call site.
///
/// This deliberately remains a macro: passing [`Inverse`] through a generic
/// helper would transfer the enum's maximum-sized variant even when the
/// mutation needs only a scalar payload.
macro_rules! push_inverse_once {
    ($journal:ident, $field:expr, $inverse:expr) => {
        if $journal.inverse_positions[$field] == UNRECORDED {
            let position = $journal.inverses.len();
            $journal.inverses.push($inverse);
            $journal.inverse_positions[$field] = position;
        }
    };
}

impl ListJournal<'_> {
    pub(super) const fn needs_nodes(&self) -> bool {
        self.inverse_positions[NODES] == UNRECORDED
    }
    pub(super) fn record_nodes(
        &mut self,
        old_region: Option<tex_state::node_region::NodeRegionId>,
        old: tex_state::node_arena::PageListId,
    ) {
        if self.inverse_positions[NODES] == UNRECORDED {
            self.inverse_positions[NODES] = self.inverses.len();
            self.inverses.push(Inverse::ListRoot {
                level_id: self.level_id,
                old_region,
                old,
            });
        }
    }

    #[cfg(test)]
    pub(super) fn record_align_state(&mut self, old: Option<AlignState>) {
        push_inverse_once!(
            self,
            ALIGN_STATE,
            Inverse::AlignState {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_incomplete_fraction(&mut self, old: Option<super::IncompleteFraction>) {
        push_inverse_once!(
            self,
            INCOMPLETE_FRACTION,
            Inverse::IncompleteFraction {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_display_interrupt(&mut self, old: Option<super::DisplayInterrupt>) {
        push_inverse_once!(
            self,
            DISPLAY_INTERRUPT,
            Inverse::DisplayInterrupt {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_display_eq_no(&mut self, old: Option<super::DisplayEqNo>) {
        push_inverse_once!(
            self,
            DISPLAY_EQ_NO,
            Inverse::DisplayEqNo {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_display_alignment(&mut self, old: bool) {
        push_inverse_once!(
            self,
            DISPLAY_ALIGNMENT,
            Inverse::DisplayAlignment {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_prev_depth(&mut self, old: Option<tex_state::scaled::Scaled>) {
        push_inverse_once!(
            self,
            PREV_DEPTH,
            Inverse::PrevDepth {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_prev_graf(&mut self, old: i32) {
        push_inverse_once!(
            self,
            PREV_GRAF,
            Inverse::PrevGraf {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_pending_projection(&mut self, old: Option<PendingHRunProjection>) {
        if self.inverse_positions[PENDING_HCHARS] == UNRECORDED {
            self.inverse_positions[PENDING_HCHARS] = self.inverses.len();
            self.inverses.push(Inverse::PendingHchars {
                level_id: self.level_id,
                old: old.map_or(
                    PendingHcharsRollback::Absent,
                    PendingHcharsRollback::Projection,
                ),
            });
        }
    }

    pub(super) fn record_pending_owned(&mut self, mut old: Option<super::PendingHRun>) {
        let position = self.inverse_positions[PENDING_HCHARS];
        if position == UNRECORDED {
            self.inverse_positions[PENDING_HCHARS] = self.inverses.len();
            self.inverses.push(Inverse::PendingHchars {
                level_id: self.level_id,
                old: PendingHcharsRollback::Value(old),
            });
            return;
        }
        let Inverse::PendingHchars { old: rollback, .. } = &mut self.inverses[position] else {
            unreachable!("pending-hchar field records its own inverse variant")
        };
        if let PendingHcharsRollback::Projection(projection) = rollback {
            if let Some(run) = &mut old {
                projection.clone().restore(run);
            }
            *rollback = PendingHcharsRollback::Value(old);
        }
    }

    pub(super) fn record_pending_value(&mut self, old: Option<&super::PendingHRun>) {
        let position = self.inverse_positions[PENDING_HCHARS];
        if position == UNRECORDED {
            self.inverse_positions[PENDING_HCHARS] = self.inverses.len();
            self.inverses.push(Inverse::PendingHchars {
                level_id: self.level_id,
                old: PendingHcharsRollback::Value(old.cloned()),
            });
            return;
        }
        let Inverse::PendingHchars { old: rollback, .. } = &mut self.inverses[position] else {
            unreachable!("pending-hchar field records its own inverse variant")
        };
        if let PendingHcharsRollback::Projection(projection) = rollback {
            let mut value = old.cloned();
            if let Some(run) = &mut value {
                projection.clone().restore(run);
            }
            *rollback = PendingHcharsRollback::Value(value);
        }
    }

    pub(super) fn record_space_factor(&mut self, old: i32) {
        push_inverse_once!(
            self,
            SPACE_FACTOR,
            Inverse::SpaceFactor {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_no_boundary(&mut self, old: bool) {
        push_inverse_once!(
            self,
            NO_BOUNDARY,
            Inverse::NoBoundary {
                level_id: self.level_id,
                old,
            }
        );
    }

    pub(super) fn record_hyphen_context(&mut self, old: (u8, u8, u8)) {
        push_inverse_once!(
            self,
            HYPHEN_CONTEXT,
            Inverse::HyphenContext {
                level_id: self.level_id,
                old,
            }
        );
    }
}

impl ModeNestStorage {
    #[cfg(test)]
    pub(super) fn reset_journal_for_test(&mut self) {
        assert!(self.journal.frames.is_empty());
        self.journal.generation = self.journal.generation.wrapping_add(1);
        self.journal.level_ids.clear();
        for _ in 0..self.levels.len() {
            let id = self.journal.allocate_level_id();
            self.journal.level_ids.push(id);
        }
    }

    pub(crate) fn begin_journal(&mut self) -> Cursor {
        assert!(self.journal.enabled);
        let cursor = self.journal.inverses.len();
        let frame_id = self.journal.next_frame_id;
        self.journal.next_frame_id = self
            .journal
            .next_frame_id
            .checked_add(1)
            .expect("mode journal frame identity overflow");
        let projection_start = self.journal.projections.len();
        self.journal.projections.extend(
            self.levels
                .iter()
                .zip(&self.journal.level_ids)
                .map(|(level, &id)| ListProjection::capture(id, &level.list)),
        );
        self.journal.frames.push(Frame {
            generation: self.journal.generation,
            id: frame_id,
            cursor,
            projection_start,
        });
        Cursor {
            generation: self.journal.generation,
            frame_id,
            cursor,
        }
    }

    #[cfg(test)]
    pub(super) fn journal_inverse_len_for_test(&self) -> usize {
        self.journal.inverses.len()
    }

    #[cfg(feature = "profiling")]
    pub(super) const fn replay_work(&self) -> u64 {
        self.journal.replay_work
    }

    pub(crate) fn commit_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.validate_cursor(cursor)?;
        let frame = self.journal.frames.pop().expect("validated frame exists");
        self.journal.projections.truncate(frame.projection_start);
        if self.journal.frames.is_empty() {
            self.journal.inverses.clear();
        }
        Ok(())
    }

    pub(crate) fn rollback_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.validate_cursor(cursor)?;
        let frame = self.journal.frames.pop().expect("validated frame exists");
        while self.journal.inverses.len() > frame.cursor {
            self.journal.replay_work = self.journal.replay_work.saturating_add(1);
            let inverse = self.journal.inverses.pop().expect("cursor bounds inverses");
            inverse.restore(self);
        }
        for index in frame.projection_start..self.journal.projections.len() {
            let projection = self.journal.projections[index];
            let level = self.level_by_id_mut(projection.id);
            level.list.page_region = projection.page_region;
            level.list.nodes = projection.root;
            level.list.semantic_identity_root = projection.list_semantic_identity_root;
            level.list.component_roots = projection.component_roots;
        }
        self.journal.projections.truncate(frame.projection_start);
        Ok(())
    }

    fn validate_cursor(&self, cursor: Cursor) -> Result<(), CursorError> {
        if !self.journal.enabled {
            return Err(CursorError::Disabled);
        }
        let Some(frame) = self.journal.frames.last() else {
            return Err(CursorError::NotInnermost);
        };
        if cursor.generation != self.journal.generation || cursor.generation != frame.generation {
            return Err(CursorError::WrongGeneration);
        }
        if cursor.frame_id != frame.id || cursor.cursor != frame.cursor {
            return Err(CursorError::NotInnermost);
        }
        Ok(())
    }

    fn level_index(&self, id: u64) -> usize {
        self.journal
            .level_ids
            .iter()
            .position(|&candidate| candidate == id)
            .expect("journal inverse level identity remains live")
    }

    fn level_by_id_mut(&mut self, id: u64) -> &mut ModeLevelSummary {
        let index = self.level_index(id);
        &mut self.levels[index]
    }
}

impl ModeNest {
    #[cfg(test)]
    pub(super) fn reset_journal_for_test(&mut self) {
        self.storage.reset_journal_for_test();
    }

    pub(crate) fn begin_journal(&mut self) -> Cursor {
        self.storage.begin_journal()
    }

    #[cfg(test)]
    pub(super) fn journal_inverse_len_for_test(&self) -> usize {
        self.storage.journal_inverse_len_for_test()
    }

    pub(crate) fn commit_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.storage.commit_journal(cursor)
    }

    pub(crate) fn rollback_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.storage.rollback_journal(cursor)
    }
}
