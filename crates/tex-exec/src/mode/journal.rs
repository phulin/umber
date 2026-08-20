use tex_state::node::Node;

use super::{AlignState, ModeLevelSummary, ModeList, ModeNest};

#[derive(Clone)]
struct ListProjection {
    id: u64,
    node_len: usize,
    physical_node_len: usize,
    scalars: ModeListScalars,
}

#[derive(Clone)]
struct ModeListScalars {
    align_state: Option<AlignState>,
    incomplete_fraction: Option<super::IncompleteFraction>,
    display_interrupt: Option<super::DisplayInterrupt>,
    display_eq_no: Option<super::DisplayEqNo>,
    display_alignment: bool,
    prev_depth: Option<tex_state::scaled::Scaled>,
    prev_graf: i32,
    pending_hchars: Option<super::PendingHRun>,
    space_factor: i32,
    no_boundary: bool,
    hyphen_language: u8,
    left_hyphen_min: u8,
    right_hyphen_min: u8,
}

impl ModeListScalars {
    fn capture(list: &ModeList) -> Self {
        Self {
            align_state: list.align_state.clone(),
            incomplete_fraction: list.incomplete_fraction.clone(),
            display_interrupt: list.display_interrupt.clone(),
            display_eq_no: list.display_eq_no.clone(),
            display_alignment: list.display_alignment,
            prev_depth: list.prev_depth,
            prev_graf: list.prev_graf,
            pending_hchars: list.pending_hchars.clone(),
            space_factor: list.space_factor,
            no_boundary: list.no_boundary,
            hyphen_language: list.hyphen_language,
            left_hyphen_min: list.left_hyphen_min,
            right_hyphen_min: list.right_hyphen_min,
        }
    }

    fn restore(self, list: &mut ModeList) {
        list.align_state = self.align_state;
        list.incomplete_fraction = self.incomplete_fraction;
        list.display_interrupt = self.display_interrupt;
        list.display_eq_no = self.display_eq_no;
        list.display_alignment = self.display_alignment;
        list.prev_depth = self.prev_depth;
        list.prev_graf = self.prev_graf;
        list.pending_hchars = self.pending_hchars;
        list.space_factor = self.space_factor;
        list.no_boundary = self.no_boundary;
        list.hyphen_language = self.hyphen_language;
        list.left_hyphen_min = self.left_hyphen_min;
        list.right_hyphen_min = self.right_hyphen_min;
    }
}

struct Frame {
    generation: u64,
    id: u64,
    cursor: usize,
    lists: Vec<ListProjection>,
}

enum Inverse {
    Node {
        level_id: u64,
        index: usize,
        old: Node,
    },
    Nodes {
        level_id: u64,
        old: tex_state::node_sequence::NodeSequence,
    },
    Push {
        level_id: u64,
    },
    Pop {
        level_id: u64,
        level: Box<ModeLevelSummary>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Cursor {
    generation: u64,
    frame_id: u64,
    cursor: usize,
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
    inverses: Vec<Inverse>,
}

impl ModeJournal {
    pub(super) fn enabled(level_count: usize) -> Self {
        let level_ids = (1..=level_count as u64).collect();
        Self {
            enabled: true,
            generation: 1,
            next_level_id: level_count as u64 + 1,
            next_frame_id: 1,
            level_ids,
            frames: Vec::new(),
            inverses: Vec::new(),
        }
    }

    pub(super) fn list(&mut self, index: usize) -> Option<ListJournal<'_>> {
        (self.enabled && !self.frames.is_empty()).then(|| ListJournal {
            level_id: self.level_ids[index],
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
            self.inverses.push(Inverse::Pop {
                level_id,
                level: Box::new(level),
            });
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
    inverses: &'a mut Vec<Inverse>,
}

impl ListJournal<'_> {
    pub(super) fn record_node(&mut self, index: usize, old: Node) {
        self.inverses.push(Inverse::Node {
            level_id: self.level_id,
            index,
            old,
        });
    }

    pub(super) fn record_nodes(&mut self, old: tex_state::node_sequence::NodeSequence) {
        self.inverses.push(Inverse::Nodes {
            level_id: self.level_id,
            old,
        });
    }
}

impl ModeNest {
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
        let lists = self
            .levels
            .iter()
            .zip(&self.journal.level_ids)
            .map(|(level, &id)| ListProjection {
                id,
                node_len: level.list.nodes().len(),
                physical_node_len: level.list.physical_nodes().len(),
                scalars: ModeListScalars::capture(&level.list),
            })
            .collect();
        self.journal.frames.push(Frame {
            generation: self.journal.generation,
            id: frame_id,
            cursor,
            lists,
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

    pub(crate) fn commit_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.validate_cursor(cursor)?;
        self.journal.frames.pop();
        if self.journal.frames.is_empty() {
            self.journal.inverses.clear();
        }
        Ok(())
    }

    pub(crate) fn rollback_journal(&mut self, cursor: Cursor) -> Result<(), CursorError> {
        self.validate_cursor(cursor)?;
        let frame = self.journal.frames.pop().expect("validated frame exists");
        let inverses = self.journal.inverses.split_off(frame.cursor);
        for inverse in inverses.into_iter().rev() {
            match inverse {
                Inverse::Node {
                    level_id,
                    index,
                    old,
                } => {
                    let level = self.level_by_id_mut(level_id);
                    level
                        .list
                        .sequence
                        .mutate_semantic(|nodes| nodes[index] = old);
                }
                Inverse::Nodes { level_id, old } => {
                    self.level_by_id_mut(level_id).list.sequence = old;
                }
                Inverse::Push { level_id } => {
                    let index = self.level_index(level_id);
                    self.levels.remove(index);
                    self.journal.level_ids.remove(index);
                }
                Inverse::Pop { level_id, level } => {
                    self.levels.push(*level);
                    self.journal.level_ids.push(level_id);
                }
            }
        }
        for projection in frame.lists {
            let level = self.level_by_id_mut(projection.id);
            level
                .list
                .sequence
                .truncate(projection.node_len, projection.physical_node_len);
            projection.scalars.restore(&mut level.list);
        }
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
