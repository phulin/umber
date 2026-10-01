//! Native TeX unsave. Sparse restores execute at their lazy marker, while
//! ordinary records retain their exact physical chronology.

use super::{DenseState, GroupFrame, GroupKind, GroupMismatch, JournalEntry, StateError};

impl<G> DenseState<G> {
    pub(crate) fn closing_group(&self, expected: GroupKind) -> Result<GroupFrame, StateError> {
        let frame = *self
            .group_frames()
            .last()
            .ok_or(StateError::GroupMismatch(GroupMismatch::no_group(expected)))?;
        if frame.kind() != expected {
            return Err(StateError::GroupMismatch(GroupMismatch::new(
                expected,
                frame.kind(),
            )));
        }
        Ok(frame)
    }

    #[cfg(test)]
    pub(crate) fn end_group(
        &mut self,
        expected: GroupKind,
    ) -> Result<Vec<super::GroupRestorationEntry<G>>, StateError> {
        let frame = self.closing_group(expected)?;
        let mut entries = Vec::new();
        let mut cursor = self.journal().len();
        let mut sparse = None;
        self.journal_mut().begin_unsave();
        while let Some(entry) = self.next_group_restore(frame, &mut cursor, &mut sparse) {
            if let JournalEntry::Mutation(saved) = entry {
                entries.push(self.restore_group_mutation(&saved)?);
            }
        }
        self.journal_mut().record_group_exit_with_records(frame);
        Ok(entries)
    }

    pub(crate) fn next_group_restore(
        &mut self,
        frame: GroupFrame,
        cursor: &mut usize,
        sparse: &mut Option<usize>,
    ) -> Option<JournalEntry<G>> {
        loop {
            if let Some(index) = sparse.take() {
                let entry = self.journal_mut().take_group_entry(index);
                *sparse = entry.sparse_link();
                return Some(entry);
            }
            if *cursor <= frame.journal_start as usize {
                return None;
            }
            *cursor -= 1;
            match self.journal().entry(*cursor) {
                JournalEntry::Mutation(saved) if super::is_extended_register_cell(saved.cell()) => {
                    continue;
                }
                JournalEntry::Box { saved, .. } if saved.index > 255 => continue,
                JournalEntry::SparseMarker { .. } => {
                    *sparse = self.journal().sparse_head(*cursor);
                    continue;
                }
                JournalEntry::Vacant => continue,
                _ => return Some(self.journal_mut().take_group_entry(*cursor)),
            }
        }
    }
}
