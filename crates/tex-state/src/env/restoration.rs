//! TeX82 unsave and e-TeX's shared sparse restore chain.

use super::durable_boxes::{BoxUnsave, DurableBoxState};
use super::{
    BankError, DenseState, GroupFrame, GroupKind, GroupMismatch, GroupRestorationCell,
    GroupRestorationEntry, GroupRestorationValue, GroupRestorations, JournalEntry, Mutation,
    StateError, is_extended_register_cell,
};
use crate::page_node_arena::PageMaterialArena;

fn scalar_key(index: usize) -> u64 {
    ((index as u64) << 32) | u64::from(u32::MAX)
}

impl<G> DenseState<G> {
    fn closing_group(&self, expected: GroupKind) -> Result<GroupFrame, StateError> {
        let frame = *self
            .groups
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
    ) -> Result<GroupRestorations<G>, StateError> {
        self.unsave::<false>(self.closing_group(expected)?, None)
    }

    pub(crate) fn end_group_with_boxes(
        &mut self,
        expected: GroupKind,
        owners: &mut DurableBoxState,
        arena: &mut PageMaterialArena,
    ) -> Result<GroupRestorations<G>, StateError> {
        let frame = self.closing_group(expected)?;
        let mut boxes = owners.begin_unsave(arena, frame.level)?;
        // Ordinary groups never enter the mixed-store walk.
        let result = if boxes.len() == 0 {
            self.unsave::<false>(frame, None)
        } else {
            self.unsave::<true>(frame, Some(&mut boxes))
        };
        boxes.finish();
        result
    }

    fn unsave<const BOXES: bool>(
        &mut self,
        frame: GroupFrame,
        mut boxes: Option<&mut BoxUnsave<'_, '_>>,
    ) -> Result<GroupRestorations<G>, StateError> {
        let end = self.journal().len();
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(
                self.journal().group_entry_count(frame)
                    + boxes.as_ref().map_or(0, |boxes| boxes.len()),
            )
            .map_err(|_| StateError::Bank(BankError::AllocationFailed))?;
        let mut boundary = self
            .journal()
            .group_sparse_start(frame)
            .map(scalar_key)
            .into_iter()
            .chain(boxes.as_ref().and_then(|boxes| boxes.sparse_boundary()))
            .min();
        let mut sparse = self.journal_mut().take_sparse_scratch();
        sparse.clear();
        let result = (|| -> Result<(), StateError> {
            for index in (frame.journal_start as usize..end).rev() {
                if BOXES {
                    self.restore_boxes_above(
                        scalar_key(index),
                        boxes.as_deref_mut().expect("box unsave"),
                        &mut boundary,
                        &mut sparse,
                        &mut entries,
                    )?;
                }
                if boundary.is_some_and(|key| key > scalar_key(index)) {
                    self.restore_sparse(&mut sparse, boxes.as_deref_mut(), &mut entries)?;
                    boundary = None;
                }
                if let JournalEntry::Mutation(saved) = self.journal().entry(index)
                    && saved.saved_at() == Some(frame.level)
                {
                    if is_extended_register_cell(saved.cell()) {
                        sparse.push((index, saved));
                    } else {
                        self.restore_group_mutation(saved, &mut entries)?;
                    }
                }
                if boundary == Some(scalar_key(index)) {
                    self.restore_sparse(&mut sparse, boxes.as_deref_mut(), &mut entries)?;
                    boundary = None;
                }
            }
            if BOXES {
                self.restore_boxes_above(
                    0,
                    boxes.as_deref_mut().expect("box unsave"),
                    &mut boundary,
                    &mut sparse,
                    &mut entries,
                )?;
            }
            self.restore_sparse(&mut sparse, boxes.as_deref_mut(), &mut entries)?;
            Ok(())
        })();
        self.journal_mut().return_sparse_scratch(sparse);
        result?;
        self.groups.pop();
        self.journal_mut().record_group_exit_with_records(frame);
        Ok(GroupRestorations { frame, entries })
    }

    fn restore_boxes_above(
        &mut self,
        limit: u64,
        boxes: &mut BoxUnsave<'_, '_>,
        boundary: &mut Option<u64>,
        sparse: &mut Vec<(usize, Mutation<G>)>,
        entries: &mut Vec<GroupRestorationEntry<G>>,
    ) -> Result<(), StateError> {
        while let Some(key) = boxes.next_key(false).filter(|key| *key > limit) {
            if boundary.is_some_and(|boundary| boundary > key) {
                self.restore_sparse(sparse, Some(boxes), entries)?;
                *boundary = None;
            }
            self.restore_box(boxes, false, entries)?;
        }
        Ok(())
    }

    fn restore_sparse(
        &mut self,
        sparse: &mut Vec<(usize, Mutation<G>)>,
        mut boxes: Option<&mut BoxUnsave<'_, '_>>,
        entries: &mut Vec<GroupRestorationEntry<G>>,
    ) -> Result<(), StateError> {
        for (index, saved) in sparse.drain(..) {
            if let Some(boxes) = boxes.as_deref_mut() {
                while boxes
                    .next_key(true)
                    .is_some_and(|key| key > scalar_key(index))
                {
                    self.restore_box(boxes, true, entries)?;
                }
            }
            self.restore_group_mutation(saved, entries)?;
        }
        if let Some(boxes) = boxes {
            while boxes.next_key(true).is_some() {
                self.restore_box(boxes, true, entries)?;
            }
        }
        Ok(())
    }

    fn restore_box(
        &mut self,
        boxes: &mut BoxUnsave<'_, '_>,
        sparse: bool,
        entries: &mut Vec<GroupRestorationEntry<G>>,
    ) -> Result<(), StateError> {
        let restored = boxes.restore(sparse)?;
        entries.push(GroupRestorationEntry {
            cell: GroupRestorationCell::BoxRegister(restored.index),
            saved: GroupRestorationValue::NodeList(restored.saved),
            live: GroupRestorationValue::NodeList(restored.live),
            outcome: restored.outcome,
            trace: self.group_restoration_trace_state()?,
        });
        Ok(())
    }
}
