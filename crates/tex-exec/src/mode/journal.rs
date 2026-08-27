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
    node_len: usize,
    physical_node_len: usize,
    page_node_root_count: usize,
    semantic_identity: tex_state::node_sequence::SemanticSequenceIdentity,
    list_semantic_identity_root: u64,
    component_roots: super::ModeComponentRoots,
    inverse_positions: [usize; FIELD_COUNT],
}

impl ListProjection {
    fn capture(id: u64, list: &ModeList) -> Self {
        Self {
            id,
            node_len: list.nodes().len(),
            physical_node_len: list.physical_nodes().len(),
            page_node_root_count: list.sequence.page_node_root_count(),
            semantic_identity: list.sequence.semantic_identity(),
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
    source_suffix: Option<Vec<super::PendingHChar>>,
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
            source_suffix: None,
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

    fn swap(&mut self, run: &mut super::PendingHRun) {
        std::mem::swap(&mut self.first, &mut run.first);
        std::mem::swap(&mut self.current, &mut run.current);
        std::mem::swap(&mut self.insertion_index, &mut run.insertion_index);
        std::mem::swap(&mut self.script, &mut run.script);
        std::mem::swap(&mut self.source_identity_root, &mut run.source_identity_root);
        std::mem::swap(&mut self.semantic_identity_root, &mut run.semantic_identity_root);
        if let Some(mut suffix) = self.source_suffix.take() {
            debug_assert_eq!(run.source.len(), self.source_len);
            run.source.append(&mut suffix);
        } else {
            self.source_suffix = Some(run.source.split_off(self.source_len));
        }
    }
}

struct Frame {
    generation: u64,
    id: u64,
    cursor: usize,
    projection_start: usize,
}

struct AcceptedFrame {
    frame: Option<Frame>,
    projections: Vec<ListProjection>,
    inverses: Vec<Inverse>,
    node_tails: Vec<(
        u64,
        tex_state::node_sequence::NodeSequenceAcceptedTail,
        u64,
        super::ModeComponentRoots,
    )>,
}

pub(super) struct AcceptedModeTail {
    frames: Vec<AcceptedFrame>,
}

#[expect(
    clippy::large_enum_variant,
    reason = "rollback values stay move-only and inline; boxing a popped mode level would add a per-operation heap owner"
)]
enum Inverse {
    Nodes {
        level_id: u64,
        old: tex_state::node_sequence::NodeSequence,
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
    /// Exchanges the live value with the value retained by this entry. The
    /// same operation therefore performs accepted rewind and rejection redo.
    fn swap(&mut self, storage: &mut ModeNestStorage) {
        match self {
            Self::Nodes { level_id, old } => {
                std::mem::swap(&mut storage.level_by_id_mut(*level_id).list.sequence, old);
            }
            #[cfg(test)]
            Self::AlignState { level_id, old } => {
                std::mem::swap(
                    &mut storage.level_by_id_mut(*level_id).list.align_state,
                    old,
                );
            }
            Self::IncompleteFraction { level_id, old } => {
                std::mem::swap(
                    &mut storage.level_by_id_mut(*level_id).list.incomplete_fraction,
                    old,
                );
            }
            Self::DisplayInterrupt { level_id, old } => {
                std::mem::swap(
                    &mut storage.level_by_id_mut(*level_id).list.display_interrupt,
                    old,
                );
            }
            Self::DisplayEqNo { level_id, old } => {
                std::mem::swap(
                    &mut storage.level_by_id_mut(*level_id).list.display_eq_no,
                    old,
                );
            }
            Self::DisplayAlignment { level_id, old } => {
                std::mem::swap(
                    &mut storage.level_by_id_mut(*level_id).list.display_alignment,
                    old,
                );
            }
            Self::PrevDepth { level_id, old } => {
                std::mem::swap(&mut storage.level_by_id_mut(*level_id).list.prev_depth, old);
            }
            Self::PrevGraf { level_id, old } => {
                std::mem::swap(&mut storage.level_by_id_mut(*level_id).list.prev_graf, old);
            }
            Self::PendingHchars { level_id, old } => {
                let pending = &mut storage.level_by_id_mut(*level_id).list.pending_hchars;
                match old {
                    PendingHcharsRollback::Absent => {
                        *old = PendingHcharsRollback::Value(pending.take());
                    }
                    PendingHcharsRollback::Projection(projection) => projection.swap(
                        pending
                            .as_mut()
                            .expect("projected pending run remains in place"),
                    ),
                    PendingHcharsRollback::Value(value) => std::mem::swap(pending, value),
                }
            }
            Self::SpaceFactor { level_id, old } => {
                std::mem::swap(
                    &mut storage.level_by_id_mut(*level_id).list.space_factor,
                    old,
                );
            }
            Self::NoBoundary { level_id, old } => {
                std::mem::swap(
                    &mut storage.level_by_id_mut(*level_id).list.no_boundary,
                    old,
                );
            }
            Self::HyphenContext { level_id, old } => {
                let list = &mut storage.level_by_id_mut(*level_id).list;
                let current = (
                    list.hyphen_language,
                    list.left_hyphen_min,
                    list.right_hyphen_min,
                );
                (
                    list.hyphen_language,
                    list.left_hyphen_min,
                    list.right_hyphen_min,
                ) = *old;
                *old = current;
            }
            Self::Push { level_id } => {
                let id = *level_id;
                let index = storage.level_index(id);
                let level = storage.levels.remove(index);
                storage.journal.level_ids.remove(index);
                *self = Self::Pop {
                    level_id: id,
                    level,
                };
            }
            Self::Pop { level_id, .. } => {
                let id = *level_id;
                let Self::Pop { level, .. } = std::mem::replace(self, Self::Push { level_id: id })
                else {
                    unreachable!()
                };
                storage.levels.push(level);
                storage.journal.level_ids.push(id);
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

    pub(super) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.level_ids
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(
                self.frames
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Frame>()),
            )
            .saturating_add(
                self.projections
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ListProjection>()),
            )
            .saturating_add(
                self.inverses
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Inverse>()),
            )
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
    pub(super) fn record_nodes(&mut self, old: &tex_state::node_sequence::NodeSequence) {
        if self.inverse_positions[NODES] == UNRECORDED {
            self.inverse_positions[NODES] = self.inverses.len();
            self.inverses.push(Inverse::Nodes {
                level_id: self.level_id,
                old: old.clone(),
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
    pub(super) fn validates_checkpoint_cursor(&self, cursor: Cursor) -> bool {
        cursor.generation == self.journal.generation
            && cursor.cursor <= self.journal.inverses.len()
            && (self.journal.inverses.len() == cursor.cursor
                || self
                    .journal
                    .frames
                    .iter()
                    .any(|frame| frame.cursor == cursor.cursor))
    }

    pub(super) fn restore_checkpoint_cursor(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        if !self.validates_checkpoint_cursor(cursor) {
            return Err(CursorError::WrongGeneration);
        }
        if !self
            .journal
            .frames
            .iter()
            .any(|frame| frame.cursor == cursor.cursor)
        {
            debug_assert_eq!(self.journal.inverses.len(), cursor.cursor);
            return Ok(());
        }
        loop {
            let frame = self
                .journal
                .frames
                .last()
                .expect("validated checkpoint frame remains present");
            let active = Cursor {
                generation: frame.generation,
                frame_id: frame.id,
                cursor: frame.cursor,
            };
            self.rollback_journal(active)?;
            if active.cursor == cursor.cursor {
                break;
            }
        }
        let _replacement = self.begin_journal();
        Ok(())
    }

    /// Rewinds the accepted owner to `cursor` while retaining every displaced
    /// value exactly once for either rejection redo or acceptance pruning.
    pub(super) fn begin_checkpoint_candidate(
        &mut self,
        cursor: Cursor,
    ) -> Result<(AcceptedModeTail, Cursor), CursorError> {
        if !self.validates_checkpoint_cursor(cursor) {
            return Err(CursorError::WrongGeneration);
        }
        let mut accepted = AcceptedModeTail { frames: Vec::new() };
        loop {
            let is_selected = self
                .journal
                .frames
                .last()
                .is_some_and(|frame| frame.id == cursor.frame_id);
            let frame = if is_selected {
                None
            } else {
                Some(
                    self.journal
                        .frames
                        .pop()
                        .ok_or(CursorError::WrongGeneration)?,
                )
            };
            let (frame_cursor, projection_start) = frame.as_ref().map_or_else(
                || {
                    let selected = self
                        .journal
                        .frames
                        .last()
                        .expect("selected checkpoint frame exists");
                    (selected.cursor, selected.projection_start)
                },
                |frame| (frame.cursor, frame.projection_start),
            );
            let mut inverses = self.journal.inverses.split_off(frame_cursor);
            for inverse in inverses.iter_mut().rev() {
                self.journal.replay_work = self.journal.replay_work.saturating_add(1);
                inverse.swap(self);
            }
            let projections = if is_selected {
                let projections = self.journal.projections[projection_start..].to_vec();
                for projection in &mut self.journal.projections[projection_start..] {
                    projection.inverse_positions = [UNRECORDED; FIELD_COUNT];
                }
                projections
            } else {
                self.journal.projections.drain(projection_start..).collect()
            };
            let mut node_tails = Vec::with_capacity(projections.len());
            for projection in &projections {
                let level = self.level_by_id_mut(projection.id);
                let accepted_list_root = level.list.semantic_identity_root;
                let accepted_components = level.list.component_roots;
                let tail = level.list.sequence.split_accepted_tail(
                        projection.node_len,
                        projection.physical_node_len,
                        projection.page_node_root_count,
                        projection.semantic_identity,
                    );
                level.list.semantic_identity_root = projection.list_semantic_identity_root;
                level.list.component_roots = projection.component_roots;
                node_tails.push((
                    projection.id,
                    tail,
                    accepted_list_root,
                    accepted_components,
                ));
            }
            accepted.frames.push(AcceptedFrame {
                frame,
                projections,
                inverses,
                node_tails,
            });
            if is_selected {
                break;
            }
        }
        let candidate = self.begin_journal();
        Ok((accepted, candidate))
    }

    /// Undoes the live candidate suffix, then redoes the accepted frames in
    /// their original order and restores their journal coordinates.
    pub(super) fn reject_checkpoint_candidate(
        &mut self,
        candidate: Cursor,
        mut accepted: AcceptedModeTail,
    ) -> Result<(), CursorError> {
        while self
            .journal
            .frames
            .last()
            .is_some_and(|frame| frame.id != candidate.frame_id)
        {
            let frame = self
                .journal
                .frames
                .last()
                .expect("checked candidate descendant exists");
            let descendant = Cursor {
                generation: frame.generation,
                frame_id: frame.id,
                cursor: frame.cursor,
            };
            self.rollback_journal(descendant)?;
        }
        self.rollback_journal(candidate)?;
        for mut accepted_frame in accepted.frames.drain(..).rev() {
            for (level_id, tail, list_root, components) in
                accepted_frame.node_tails.drain(..)
            {
                let level = self.level_by_id_mut(level_id);
                level.list.sequence.restore_accepted_tail(tail);
                level.list.semantic_identity_root = list_root;
                level.list.component_roots = components;
            }
            for inverse in &mut accepted_frame.inverses {
                inverse.swap(self);
            }
            self.journal.inverses.append(&mut accepted_frame.inverses);
            if let Some(frame) = accepted_frame.frame {
                debug_assert_eq!(self.journal.projections.len(), frame.projection_start);
                self.journal
                    .projections
                    .append(&mut accepted_frame.projections);
                self.journal.frames.push(frame);
            } else {
                let selected = self
                    .journal
                    .frames
                    .last()
                    .expect("selected checkpoint frame remains installed");
                debug_assert_eq!(
                    self.journal.projections.len() - selected.projection_start,
                    accepted_frame.projections.len()
                );
                self.journal.projections[selected.projection_start..]
                    .copy_from_slice(&accepted_frame.projections);
            }
        }
        Ok(())
    }

    /// Promotes the live candidate while retaining any named marks published
    /// beneath it. Only the hidden aggregate frame and its duplicate list
    /// projections are removed.
    pub(super) fn accept_checkpoint_candidate(
        &mut self,
        candidate: Cursor,
    ) -> Result<(), CursorError> {
        let index = self
            .journal
            .frames
            .iter()
            .position(|frame| {
                frame.generation == candidate.generation && frame.id == candidate.frame_id
            })
            .ok_or(CursorError::WrongGeneration)?;
        let frame = self.journal.frames.remove(index);
        if frame.cursor != candidate.cursor {
            return Err(CursorError::WrongGeneration);
        }
        let projection_end = self
            .journal
            .frames
            .get(index)
            .map_or(self.journal.projections.len(), |next| next.projection_start);
        let removed = projection_end - frame.projection_start;
        self.journal
            .projections
            .drain(frame.projection_start..projection_end);
        for descendant in &mut self.journal.frames[index..] {
            descendant.projection_start -= removed;
        }
        Ok(())
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
            let mut inverse = self.journal.inverses.pop().expect("cursor bounds inverses");
            inverse.swap(self);
        }
        for index in frame.projection_start..self.journal.projections.len() {
            let projection = self.journal.projections[index];
            let level = self.level_by_id_mut(projection.id);
            level.list.sequence.restore_checkpoint_lengths(
                projection.node_len,
                projection.physical_node_len,
                projection.page_node_root_count,
                projection.semantic_identity,
            );
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
        self.storage.borrow_mut().reset_journal_for_test();
    }

    pub(crate) fn begin_journal(&mut self) -> Cursor {
        self.storage.borrow_mut().begin_journal()
    }

    #[cfg(test)]
    pub(super) fn journal_inverse_len_for_test(&self) -> usize {
        self.storage.borrow().journal_inverse_len_for_test()
    }

    pub(crate) fn commit_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.storage.borrow_mut().commit_journal(cursor)
    }

    pub(crate) fn rollback_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.storage.borrow_mut().rollback_journal(cursor)
    }
}
