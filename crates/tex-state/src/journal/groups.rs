//! The one contiguous TeX save stack. Boundaries carry only scalar metadata;
//! actual before-values and aftergroup words live inline in this same stack.

use super::Mutation;
use crate::GroupFrame;
use crate::env::durable_boxes::DurableMutation;
use crate::token::TracedTokenWord;

/// One native slot, packed with a zero niche for an absent chain link.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SaveIndex(core::num::NonZeroU32);
impl SaveIndex {
    pub(super) fn new(index: usize) -> Self {
        Self(
            core::num::NonZeroU32::new(
                u32::try_from(index)
                    .expect("save stack fits u32")
                    .checked_add(1)
                    .expect("save stack index has a zero niche"),
            )
            .expect("encoded save index is nonzero"),
        )
    }
    pub(super) fn get(self) -> usize {
        (self.0.get() - 1) as usize
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GroupBoundary {
    pub(super) frame: GroupFrame,
    pub(super) previous: Option<SaveIndex>,
    pub(super) sparse_marker: Option<SaveIndex>,
    pub(super) definition_frontier: u32,
}

pub(crate) enum JournalEntry<G> {
    Mutation(Mutation<G>),
    GroupEnter(GroupBoundary),
    Box {
        saved: DurableMutation,
        sparse_link: Option<SaveIndex>,
    },
    AfterGroup(TracedTokenWord),
    SparseMarker {
        head: Option<SaveIndex>,
    },
    Vacant,
}

impl<G> JournalEntry<G> {
    pub(super) fn set_sparse_link(&mut self, link: Option<usize>) {
        match self {
            Self::Mutation(saved) => saved.sparse_link = link.map(SaveIndex::new),
            Self::Box { sparse_link, .. } => *sparse_link = link.map(SaveIndex::new),
            _ => unreachable!("only register restores enter the sparse chain"),
        }
    }
    pub(crate) fn sparse_link(&self) -> Option<usize> {
        match self {
            Self::Mutation(saved) => saved.sparse_link.map(SaveIndex::get),
            Self::Box { sparse_link, .. } => sparse_link.map(SaveIndex::get),
            _ => None,
        }
    }
}

pub(super) struct GroupSaveStack<G> {
    pub(super) entries: Vec<JournalEntry<G>>,
    pub(super) top: Option<usize>,
    pub(super) depth: usize,
    pub(super) unsaving: bool,
}

impl<G> Default for GroupSaveStack<G> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            top: None,
            depth: 0,
            unsaving: false,
        }
    }
}
impl<G> GroupSaveStack<G> {
    pub(super) fn is_empty(&self) -> bool {
        self.depth == 0
    }
    pub(super) fn append(&mut self, entry: JournalEntry<G>) {
        self.entries.push(entry);
    }
    pub(super) fn truncate(&mut self, len: usize) {
        self.entries.truncate(len);
    }
    pub(super) fn entry(&self, index: usize) -> Option<&JournalEntry<G>> {
        self.entries.get(index)
    }
    pub(super) fn boundary(&self, index: usize) -> &GroupBoundary {
        match &self.entries[index] {
            JournalEntry::GroupEnter(boundary) => boundary,
            _ => unreachable!("group boundary exists"),
        }
    }
    pub(super) fn boundary_mut(&mut self, index: usize) -> &mut GroupBoundary {
        match &mut self.entries[index] {
            JournalEntry::GroupEnter(boundary) => boundary,
            _ => unreachable!("group boundary exists"),
        }
    }
    pub(super) fn sparse_head(&self, index: usize) -> Option<usize> {
        match &self.entries[index] {
            JournalEntry::SparseMarker { head } => head.map(SaveIndex::get),
            _ => unreachable!("sparse marker exists"),
        }
    }
    pub(super) fn set_sparse_head(&mut self, index: usize, value: Option<usize>) {
        match &mut self.entries[index] {
            JournalEntry::SparseMarker { head } => *head = value.map(SaveIndex::new),
            _ => unreachable!("sparse marker exists"),
        }
    }
    pub(super) fn frames(&self) -> GroupFrames<'_, G> {
        GroupFrames { stack: self }
    }
}

/// Borrowed diagnostic view of native boundaries; no second group topology.
#[derive(Clone, Copy)]
pub struct GroupFrames<'a, G> {
    stack: &'a GroupSaveStack<G>,
}
impl<'a, G> GroupFrames<'a, G> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.stack.depth
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stack.depth == 0
    }
    #[must_use]
    pub fn last(&self) -> Option<&'a GroupFrame> {
        self.stack
            .top
            .map(|index| &self.stack.boundary(index).frame)
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &'a GroupFrame> + use<'a, G> {
        self.stack.entries.iter().filter_map(|entry| match entry {
            JournalEntry::GroupEnter(boundary) => Some(&boundary.frame),
            _ => None,
        })
    }
    #[must_use]
    pub fn to_vec(&self) -> Vec<GroupFrame> {
        self.iter().copied().collect()
    }
}
impl<G> core::ops::Index<usize> for GroupFrames<'_, G> {
    type Output = GroupFrame;
    fn index(&self, index: usize) -> &GroupFrame {
        self.iter().nth(index).expect("group frame exists")
    }
}
pub struct GroupFramesIter<'a, G> {
    entries: core::slice::Iter<'a, JournalEntry<G>>,
}
impl<'a, G> Iterator for GroupFramesIter<'a, G> {
    type Item = &'a GroupFrame;
    fn next(&mut self) -> Option<Self::Item> {
        self.entries.find_map(|entry| match entry {
            JournalEntry::GroupEnter(boundary) => Some(&boundary.frame),
            _ => None,
        })
    }
}
impl<G> DoubleEndedIterator for GroupFramesIter<'_, G> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.by_ref().rev().find_map(|entry| match entry {
            JournalEntry::GroupEnter(boundary) => Some(&boundary.frame),
            _ => None,
        })
    }
}
impl<'a, G> IntoIterator for GroupFrames<'a, G> {
    type Item = &'a GroupFrame;
    type IntoIter = GroupFramesIter<'a, G>;
    fn into_iter(self) -> Self::IntoIter {
        GroupFramesIter {
            entries: self.stack.entries.iter(),
        }
    }
}
