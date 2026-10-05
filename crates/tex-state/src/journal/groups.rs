//! The native group stack: compact actual boundaries and one chronological
//! sequence of demanded before-values, aftergroup words and sparse markers.

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
    pub(super) sparse_marker: Option<SaveIndex>,
}

pub(crate) enum JournalEntry<G> {
    Mutation(Mutation<G>),
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
    pub(super) roots: crate::definition_arena::DefinitionRoots,
    pub(super) boundaries: Vec<GroupBoundary>,
    pub(super) unsaving: bool,
}

impl<G> GroupSaveStack<G> {
    pub(super) fn new(roots: crate::definition_arena::DefinitionRoots) -> Self {
        Self {
            entries: Vec::new(),
            roots,
            boundaries: Vec::new(),
            unsaving: false,
        }
    }
}
impl<G> GroupSaveStack<G> {
    pub(super) fn is_empty(&self) -> bool {
        self.boundaries.is_empty()
    }
    pub(super) fn append(&mut self, entry: JournalEntry<G>) {
        if let JournalEntry::Mutation(saved) = &entry {
            self.roots.acquire_word(&saved.before);
        }
        self.entries.push(entry);
    }
    pub(super) fn truncate(&mut self, len: usize) {
        for entry in self.entries.drain(len..) {
            if let JournalEntry::Mutation(saved) = entry {
                self.roots.release_word(&saved.before);
            }
        }
    }
    pub(super) fn entry(&self, index: usize) -> Option<&JournalEntry<G>> {
        self.entries.get(index)
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

/// Borrowed diagnostic view of the actual compact boundary stack.
#[derive(Clone, Copy)]
pub struct GroupFrames<'a, G> {
    stack: &'a GroupSaveStack<G>,
}
impl<'a, G> GroupFrames<'a, G> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.stack.boundaries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stack.boundaries.is_empty()
    }
    #[must_use]
    pub fn last(&self) -> Option<&'a GroupFrame> {
        self.stack.boundaries.last().map(|boundary| &boundary.frame)
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &'a GroupFrame> + use<'a, G> {
        self.stack.boundaries.iter().map(|boundary| &boundary.frame)
    }
    #[must_use]
    pub fn to_vec(&self) -> Vec<GroupFrame> {
        self.iter().copied().collect()
    }
}
impl<G> core::ops::Index<usize> for GroupFrames<'_, G> {
    type Output = GroupFrame;
    fn index(&self, index: usize) -> &GroupFrame {
        &self.stack.boundaries[index].frame
    }
}
pub struct GroupFramesIter<'a> {
    boundaries: core::slice::Iter<'a, GroupBoundary>,
}
impl<'a> Iterator for GroupFramesIter<'a> {
    type Item = &'a GroupFrame;
    fn next(&mut self) -> Option<Self::Item> {
        self.boundaries.next().map(|boundary| &boundary.frame)
    }
}
impl DoubleEndedIterator for GroupFramesIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.boundaries.next_back().map(|boundary| &boundary.frame)
    }
}
impl<'a, G> IntoIterator for GroupFrames<'a, G> {
    type Item = &'a GroupFrame;
    type IntoIter = GroupFramesIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        GroupFramesIter {
            boundaries: self.stack.boundaries.iter(),
        }
    }
}

impl<G> Drop for GroupSaveStack<G> {
    fn drop(&mut self) {
        self.truncate(0);
    }
}
