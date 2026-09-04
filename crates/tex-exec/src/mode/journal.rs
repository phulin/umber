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
    root: tex_state::page_node_arena::PageListSpan,
    inverse_positions: [usize; FIELD_COUNT],
}

impl ListProjection {
    fn capture(id: u64, list: &ModeList) -> Self {
        Self {
            id,
            root: list.nodes,
            inverse_positions: [UNRECORDED; FIELD_COUNT],
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct PendingHRunProjection {
    insertion_index: usize,
    source_len: usize,
    script: tex_fonts::Script,
}

impl PendingHRunProjection {
    pub(super) fn capture(run: &super::PendingHRun) -> Self {
        Self {
            insertion_index: run.insertion_index,
            source_len: run.source.len(),
            script: run.script,
        }
    }

    fn restore(self, run: &mut super::PendingHRun) {
        run.insertion_index = self.insertion_index;
        run.source.truncate(self.source_len);
        run.script = self.script;
    }
}

struct Frame {
    generation: u64,
    id: u64,
    cursor: usize,
    projection_start: usize,
    rollback_retains_page_roots: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum InverseKind {
    ListRoot,
    #[cfg(test)]
    AlignState,
    IncompleteFraction,
    DisplayInterrupt,
    DisplayEqNo,
    DisplayAlignment,
    PrevDepth,
    PrevGraf,
    PendingAbsent,
    PendingProjection,
    PendingValue,
    SpaceFactor,
    NoBoundary,
    HyphenContext,
    Push,
    Pop,
}

/// One exact position in the global reverse-order stream.
///
/// Payloads larger than four bytes live inline in their type-specific journal
/// lane. This descriptor is the only value copied for scalar mutations; the
/// popped-level payload therefore cannot inflate every unrelated entry.
#[derive(Clone, Copy)]
#[repr(C)]
struct Inverse {
    level_id: u64,
    payload: u32,
    kind: InverseKind,
}

impl Inverse {
    const fn new(level_id: u64, kind: InverseKind, payload: u32) -> Self {
        Self {
            level_id,
            payload,
            kind,
        }
    }
}

struct InverseLanes {
    list_roots: Vec<tex_state::page_node_arena::PageListSpan>,
    #[cfg(test)]
    align_states: Vec<Option<AlignState>>,
    incomplete_fractions: Vec<Option<super::IncompleteFraction>>,
    display_interrupts: Vec<Option<super::DisplayInterrupt>>,
    display_eq_nos: Vec<Option<super::DisplayEqNo>>,
    prev_depths: Vec<Option<tex_state::scaled::Scaled>>,
    pending_projections: Vec<PendingHRunProjection>,
    pending_values: Vec<Option<super::PendingHRun>>,
    popped_levels: Vec<ModeLevelSummary>,
}

impl Default for InverseLanes {
    fn default() -> Self {
        // One entry per kind covers the ordinary single-operation frame
        // without a mutation-time allocation. Nested frames grow only the
        // particular typed lane they actually use.
        Self {
            list_roots: Vec::with_capacity(1),
            #[cfg(test)]
            align_states: Vec::with_capacity(1),
            incomplete_fractions: Vec::with_capacity(1),
            display_interrupts: Vec::with_capacity(1),
            display_eq_nos: Vec::with_capacity(1),
            prev_depths: Vec::with_capacity(1),
            pending_projections: Vec::with_capacity(1),
            pending_values: Vec::with_capacity(1),
            popped_levels: Vec::with_capacity(1),
        }
    }
}

impl InverseLanes {
    fn clear(&mut self) {
        self.list_roots.clear();
        #[cfg(test)]
        self.align_states.clear();
        self.incomplete_fractions.clear();
        self.display_interrupts.clear();
        self.display_eq_nos.clear();
        self.prev_depths.clear();
        self.pending_projections.clear();
        self.pending_values.clear();
        self.popped_levels.clear();
    }
}

fn push_lane<T>(lane: &mut Vec<T>, value: T) -> u32 {
    let index = u32::try_from(lane.len()).expect("mode journal lane exceeds u32");
    lane.push(value);
    index
}

fn pop_lane<T>(lane: &mut Vec<T>, index: u32) -> T {
    assert_eq!(
        usize::try_from(index).expect("u32 fits usize"),
        lane.len() - 1,
        "reverse replay consumes each compact lane in stack order"
    );
    lane.pop()
        .expect("inverse descriptor owns its lane payload")
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
    inverse_lanes: InverseLanes,
    replay_work: u64,
}

impl ModeJournal {
    pub(super) fn has_active_frame(&self) -> bool {
        !self.frames.is_empty()
    }

    pub(super) fn retains_page_node_handles(&self) -> bool {
        self.frames
            .last()
            .is_some_and(|frame| frame.rollback_retains_page_roots)
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
            inverse_lanes: InverseLanes::default(),
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
        let Self {
            inverses,
            inverse_lanes,
            ..
        } = self;
        Some(ListJournal {
            level_id: projection.id,
            inverse_positions: &mut projection.inverse_positions,
            inverses,
            inverse_lanes,
        })
    }

    pub(super) fn record_level_push(&mut self) {
        if !self.enabled {
            return;
        }
        let id = self.allocate_level_id();
        self.level_ids.push(id);
        if !self.frames.is_empty() {
            self.inverses.push(Inverse::new(id, InverseKind::Push, 0));
        }
    }

    pub(super) fn record_level_pop(&mut self, level: ModeLevelSummary) {
        if !self.enabled {
            return;
        }
        let level_id = self.level_ids.pop().expect("journal level identity exists");
        if !self.frames.is_empty() {
            let payload = push_lane(&mut self.inverse_lanes.popped_levels, level);
            self.inverses
                .push(Inverse::new(level_id, InverseKind::Pop, payload));
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

    #[cfg(test)]
    fn recorded_bytes_for_test(&self) -> usize {
        self.inverses.len() * std::mem::size_of::<Inverse>()
            + self.inverse_lanes.list_roots.len()
                * std::mem::size_of::<tex_state::page_node_arena::PageListSpan>()
            + self.inverse_lanes.align_states.len() * std::mem::size_of::<Option<AlignState>>()
            + self.inverse_lanes.incomplete_fractions.len()
                * std::mem::size_of::<Option<super::IncompleteFraction>>()
            + self.inverse_lanes.display_interrupts.len()
                * std::mem::size_of::<Option<super::DisplayInterrupt>>()
            + self.inverse_lanes.display_eq_nos.len()
                * std::mem::size_of::<Option<super::DisplayEqNo>>()
            + self.inverse_lanes.prev_depths.len()
                * std::mem::size_of::<Option<tex_state::scaled::Scaled>>()
            + self.inverse_lanes.pending_projections.len()
                * std::mem::size_of::<PendingHRunProjection>()
            + self.inverse_lanes.pending_values.len()
                * std::mem::size_of::<Option<super::PendingHRun>>()
            + self.inverse_lanes.popped_levels.len() * std::mem::size_of::<ModeLevelSummary>()
    }
}

pub(super) struct ListJournal<'a> {
    level_id: u64,
    inverse_positions: &'a mut [usize; FIELD_COUNT],
    inverses: &'a mut Vec<Inverse>,
    inverse_lanes: &'a mut InverseLanes,
}

macro_rules! push_inline_once {
    ($journal:ident, $field:expr, $kind:expr, $payload:expr) => {
        if $journal.inverse_positions[$field] == UNRECORDED {
            let position = $journal.inverses.len();
            $journal
                .inverses
                .push(Inverse::new($journal.level_id, $kind, $payload));
            $journal.inverse_positions[$field] = position;
        }
    };
}

macro_rules! push_lane_once {
    ($journal:ident, $field:expr, $kind:expr, $lane:ident, $value:expr) => {
        if $journal.inverse_positions[$field] == UNRECORDED {
            let payload = push_lane(&mut $journal.inverse_lanes.$lane, $value);
            let position = $journal.inverses.len();
            $journal
                .inverses
                .push(Inverse::new($journal.level_id, $kind, payload));
            $journal.inverse_positions[$field] = position;
        }
    };
}

impl ListJournal<'_> {
    pub(super) const fn needs_nodes(&self) -> bool {
        self.inverse_positions[NODES] == UNRECORDED
    }
    pub(super) fn record_nodes(&mut self, old: tex_state::page_node_arena::PageListSpan) {
        push_lane_once!(self, NODES, InverseKind::ListRoot, list_roots, old);
    }

    #[cfg(test)]
    pub(super) fn record_align_state(&mut self, old: Option<AlignState>) {
        push_lane_once!(
            self,
            ALIGN_STATE,
            InverseKind::AlignState,
            align_states,
            old
        );
    }

    pub(super) fn record_incomplete_fraction(&mut self, old: Option<super::IncompleteFraction>) {
        push_lane_once!(
            self,
            INCOMPLETE_FRACTION,
            InverseKind::IncompleteFraction,
            incomplete_fractions,
            old
        );
    }

    pub(super) fn record_display_interrupt(&mut self, old: Option<super::DisplayInterrupt>) {
        push_lane_once!(
            self,
            DISPLAY_INTERRUPT,
            InverseKind::DisplayInterrupt,
            display_interrupts,
            old
        );
    }

    pub(super) fn record_display_eq_no(&mut self, old: Option<super::DisplayEqNo>) {
        push_lane_once!(
            self,
            DISPLAY_EQ_NO,
            InverseKind::DisplayEqNo,
            display_eq_nos,
            old
        );
    }

    pub(super) fn record_display_alignment(&mut self, old: bool) {
        push_inline_once!(
            self,
            DISPLAY_ALIGNMENT,
            InverseKind::DisplayAlignment,
            old.into()
        );
    }

    pub(super) fn record_prev_depth(&mut self, old: Option<tex_state::scaled::Scaled>) {
        push_lane_once!(self, PREV_DEPTH, InverseKind::PrevDepth, prev_depths, old);
    }

    pub(super) fn record_prev_graf(&mut self, old: i32) {
        push_inline_once!(self, PREV_GRAF, InverseKind::PrevGraf, old as u32);
    }

    pub(super) fn record_pending_projection(&mut self, old: Option<PendingHRunProjection>) {
        if self.inverse_positions[PENDING_HCHARS] == UNRECORDED {
            let (kind, payload) = old.map_or((InverseKind::PendingAbsent, 0), |projection| {
                (
                    InverseKind::PendingProjection,
                    push_lane(&mut self.inverse_lanes.pending_projections, projection),
                )
            });
            self.inverse_positions[PENDING_HCHARS] = self.inverses.len();
            self.inverses
                .push(Inverse::new(self.level_id, kind, payload));
        }
    }

    pub(super) fn record_pending_owned(&mut self, old: &mut Option<super::PendingHRun>) {
        let position = self.inverse_positions[PENDING_HCHARS];
        if position == UNRECORDED {
            self.push_pending_value(old.take());
            return;
        }
        if self.inverses[position].kind == InverseKind::PendingProjection {
            // A projection assumes the run still exists. Record the destructive
            // transition separately so reverse replay first reinstates that run
            // and can then apply the earlier narrow projection.
            self.push_pending_value(old.take());
        }
    }

    fn push_pending_value(&mut self, old: Option<super::PendingHRun>) {
        let payload = push_lane(&mut self.inverse_lanes.pending_values, old);
        self.inverse_positions[PENDING_HCHARS] = self.inverses.len();
        self.inverses.push(Inverse::new(
            self.level_id,
            InverseKind::PendingValue,
            payload,
        ));
    }

    pub(super) fn record_space_factor(&mut self, old: i32) {
        push_inline_once!(self, SPACE_FACTOR, InverseKind::SpaceFactor, old as u32);
    }

    pub(super) fn record_no_boundary(&mut self, old: bool) {
        push_inline_once!(self, NO_BOUNDARY, InverseKind::NoBoundary, old.into());
    }

    pub(super) fn record_hyphen_context(&mut self, old: (u8, u8, u8)) {
        let payload = u32::from(old.0) | (u32::from(old.1) << 8) | (u32::from(old.2) << 16);
        push_inline_once!(self, HYPHEN_CONTEXT, InverseKind::HyphenContext, payload);
    }
}

impl ModeNestStorage {
    fn recycle_pending_sources(&mut self) {
        let scratch = &mut self.scratch;
        for value in self
            .journal
            .inverse_lanes
            .pending_values
            .drain(..)
            .flatten()
        {
            scratch.recycle_pending_source(value.source);
        }
    }

    fn recycle_live_pending_source(&mut self, value: Option<super::PendingHRun>) {
        if let Some(value) = value {
            self.scratch.recycle_pending_source(value.source);
        }
    }

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
        let mut current_frame_retains_page_roots = false;
        self.journal
            .projections
            .extend(
                self.levels
                    .iter()
                    .zip(&self.journal.level_ids)
                    .map(|(level, &id)| {
                        current_frame_retains_page_roots |= level.list.has_node_roots();
                        ListProjection::capture(id, &level.list)
                    }),
            );
        let rollback_retains_page_roots = current_frame_retains_page_roots
            || self
                .journal
                .frames
                .last()
                .is_some_and(|frame| frame.rollback_retains_page_roots);
        self.journal.frames.push(Frame {
            generation: self.journal.generation,
            id: frame_id,
            cursor,
            projection_start,
            rollback_retains_page_roots,
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

    #[cfg(test)]
    pub(super) fn journal_recorded_bytes_for_test(&self) -> usize {
        self.journal.recorded_bytes_for_test()
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
            self.recycle_pending_sources();
            self.scratch.clear();
            self.journal.inverses.clear();
            self.journal.inverse_lanes.clear();
        }
        Ok(())
    }

    pub(crate) fn rollback_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.validate_cursor(cursor)?;
        let frame = self.journal.frames.pop().expect("validated frame exists");
        while self.journal.inverses.len() > frame.cursor {
            self.journal.replay_work = self.journal.replay_work.saturating_add(1);
            let inverse = self.journal.inverses.pop().expect("cursor bounds inverses");
            self.restore_inverse(inverse);
        }
        for index in frame.projection_start..self.journal.projections.len() {
            let projection = self.journal.projections[index];
            let level = self.level_by_id_mut(projection.id);
            level.list.nodes = projection.root;
        }
        self.journal.projections.truncate(frame.projection_start);
        self.scratch.clear();
        Ok(())
    }

    /// Restores one operation-local inverse. Aggregate checkpoints retain no
    /// mode journal tail, so every descriptor and lane payload is consumed
    /// exactly once and never needs a forward-redo value.
    fn restore_inverse(&mut self, inverse: Inverse) {
        let level_id = inverse.level_id;
        match inverse.kind {
            InverseKind::ListRoot => {
                let old = pop_lane(&mut self.journal.inverse_lanes.list_roots, inverse.payload);
                self.level_by_id_mut(level_id).list.nodes = old;
            }
            #[cfg(test)]
            InverseKind::AlignState => {
                let old = pop_lane(
                    &mut self.journal.inverse_lanes.align_states,
                    inverse.payload,
                );
                self.level_by_id_mut(level_id).list.align_state = old;
            }
            InverseKind::IncompleteFraction => {
                let old = pop_lane(
                    &mut self.journal.inverse_lanes.incomplete_fractions,
                    inverse.payload,
                );
                self.level_by_id_mut(level_id).list.incomplete_fraction = old;
            }
            InverseKind::DisplayInterrupt => {
                let old = pop_lane(
                    &mut self.journal.inverse_lanes.display_interrupts,
                    inverse.payload,
                );
                self.level_by_id_mut(level_id).list.display_interrupt = old;
            }
            InverseKind::DisplayEqNo => {
                let old = pop_lane(
                    &mut self.journal.inverse_lanes.display_eq_nos,
                    inverse.payload,
                );
                self.level_by_id_mut(level_id).list.display_eq_no = old;
            }
            InverseKind::DisplayAlignment => {
                self.level_by_id_mut(level_id).list.display_alignment = inverse.payload != 0;
            }
            InverseKind::PrevDepth => {
                let old = pop_lane(&mut self.journal.inverse_lanes.prev_depths, inverse.payload);
                self.level_by_id_mut(level_id).list.prev_depth = old;
            }
            InverseKind::PrevGraf => {
                self.level_by_id_mut(level_id).list.prev_graf = inverse.payload as i32;
            }
            InverseKind::PendingAbsent => {
                let value = self.level_by_id_mut(level_id).list.pending_hchars.take();
                self.recycle_live_pending_source(value);
            }
            InverseKind::PendingProjection => {
                let projection = pop_lane(
                    &mut self.journal.inverse_lanes.pending_projections,
                    inverse.payload,
                );
                projection.restore(
                    self.level_by_id_mut(level_id)
                        .list
                        .pending_hchars
                        .as_mut()
                        .expect("destructive pending inverse reinstates projected run"),
                );
            }
            InverseKind::PendingValue => {
                let value = pop_lane(
                    &mut self.journal.inverse_lanes.pending_values,
                    inverse.payload,
                );
                let list = &mut self.level_by_id_mut(level_id).list;
                let discarded = list.pending_hchars.take();
                list.pending_hchars = value;
                self.recycle_live_pending_source(discarded);
            }
            InverseKind::SpaceFactor => {
                self.level_by_id_mut(level_id).list.space_factor = inverse.payload as i32;
            }
            InverseKind::NoBoundary => {
                self.level_by_id_mut(level_id).list.no_boundary = inverse.payload != 0;
            }
            InverseKind::HyphenContext => {
                let list = &mut self.level_by_id_mut(level_id).list;
                list.hyphen_language = inverse.payload as u8;
                list.left_hyphen_min = (inverse.payload >> 8) as u8;
                list.right_hyphen_min = (inverse.payload >> 16) as u8;
            }
            InverseKind::Push => {
                let index = self.level_index(level_id);
                self.levels.remove(index);
                self.journal.level_ids.remove(index);
            }
            InverseKind::Pop => {
                let level = pop_lane(
                    &mut self.journal.inverse_lanes.popped_levels,
                    inverse.payload,
                );
                self.levels.push(level);
                self.journal.level_ids.push(level_id);
            }
        }
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

    #[cfg(test)]
    pub(super) fn journal_recorded_bytes_for_test(&self) -> usize {
        self.storage.journal_recorded_bytes_for_test()
    }

    pub(crate) fn commit_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.storage.commit_journal(cursor)
    }

    pub(crate) fn rollback_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.storage.rollback_journal(cursor)
    }
}

#[cfg(test)]
pub(super) fn inverse_layout_for_test() -> [usize; 10] {
    [
        std::mem::size_of::<Inverse>(),
        std::mem::size_of::<tex_state::page_node_arena::PageListSpan>(),
        std::mem::size_of::<Option<AlignState>>(),
        std::mem::size_of::<Option<super::IncompleteFraction>>(),
        std::mem::size_of::<Option<super::DisplayInterrupt>>(),
        std::mem::size_of::<Option<super::DisplayEqNo>>(),
        std::mem::size_of::<Option<tex_state::scaled::Scaled>>(),
        std::mem::size_of::<PendingHRunProjection>(),
        std::mem::size_of::<Option<super::PendingHRun>>(),
        std::mem::size_of::<ModeLevelSummary>(),
    ]
}
