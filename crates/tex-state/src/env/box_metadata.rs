use core::array;

use crate::epoch::Epoch;
use crate::journal::JournalPos;

use super::banks::DENSE_REGISTER_COUNT;
use super::overflow::REGISTER_COUNT;

const PAGE_BITS: u16 = 8;
const PAGE_LEN: usize = 1 << PAGE_BITS;
const PAGE_COUNT: usize = (REGISTER_COUNT as usize - DENSE_REGISTER_COUNT) / PAGE_LEN;
const PAGE_MASK: u16 = (PAGE_LEN as u16) - 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct BoxAssignmentMeta {
    pub(super) local_depth: Option<u32>,
    pub(super) coalesce: Option<BoxCoalesce>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BoxCoalesce {
    pub(super) depth: u32,
    pub(super) epoch: Epoch,
    pub(super) pos: JournalPos,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BoxMetaUndo {
    index: u16,
    old: BoxAssignmentMeta,
}

/// Materialized assignment-level metadata parallel to the box register banks.
///
/// The sidecar is derived execution state, but its undo suffix is checkpointed
/// with `Env` so group exit and aggregate rollback restore it atomically with
/// the corresponding box values.
#[derive(Clone, Debug)]
pub(super) struct BoxMetadata {
    dense: [BoxAssignmentMeta; DENSE_REGISTER_COUNT],
    sparse: [Option<Box<[BoxAssignmentMeta; PAGE_LEN]>>; PAGE_COUNT],
    undo: Vec<BoxMetaUndo>,
}

impl BoxMetadata {
    pub(super) fn new() -> Self {
        Self {
            dense: [BoxAssignmentMeta::default(); DENSE_REGISTER_COUNT],
            sparse: array::from_fn(|_| None),
            undo: Vec::new(),
        }
    }

    pub(super) fn get(&self, index: u16) -> BoxAssignmentMeta {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            return self.dense[usize::from(index)];
        }
        let (page, offset) = sparse_location(index);
        self.sparse[page]
            .as_ref()
            .map_or(BoxAssignmentMeta::default(), |page| page[offset])
    }

    pub(super) fn set(&mut self, index: u16, value: BoxAssignmentMeta) {
        let old = self.get(index);
        if old == value {
            return;
        }
        self.undo.push(BoxMetaUndo { index, old });
        self.set_raw(index, value);
    }

    pub(super) fn mark(&self) -> u32 {
        u32::try_from(self.undo.len()).expect("box metadata undo log exceeds u32 entries")
    }

    pub(super) fn restore_to(&mut self, mark: u32) {
        let mark = usize::try_from(mark).expect("box metadata mark fits usize");
        assert!(mark <= self.undo.len(), "box metadata mark is past the end");
        while self.undo.len() > mark {
            let undo = self.undo.pop().expect("length checked above");
            self.set_raw(undo.index, undo.old);
        }
    }

    fn set_raw(&mut self, index: u16, value: BoxAssignmentMeta) {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            self.dense[usize::from(index)] = value;
            return;
        }
        let (page, offset) = sparse_location(index);
        if value == BoxAssignmentMeta::default() {
            let Some(sparse) = self.sparse[page].as_mut() else {
                return;
            };
            sparse[offset] = value;
            if sparse
                .iter()
                .all(|entry| *entry == BoxAssignmentMeta::default())
            {
                self.sparse[page] = None;
            }
            return;
        }
        let sparse = self.sparse[page]
            .get_or_insert_with(|| Box::new([BoxAssignmentMeta::default(); PAGE_LEN]));
        sparse[offset] = value;
    }

    #[cfg(test)]
    pub(super) fn has_sparse_page(&self, index: u16) -> bool {
        let (page, _) = sparse_location(index);
        self.sparse[page].is_some()
    }
}

fn sparse_location(index: u16) -> (usize, usize) {
    assert!(
        (DENSE_REGISTER_COUNT as u16..REGISTER_COUNT).contains(&index),
        "box register index out of sparse metadata range"
    );
    let sparse = index - DENSE_REGISTER_COUNT as u16;
    (
        usize::from(sparse >> PAGE_BITS),
        usize::from(sparse & PAGE_MASK),
    )
}
