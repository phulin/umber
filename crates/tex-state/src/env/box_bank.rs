//! Box-register live state and its unified write-barrier bookkeeping.
//!
//! A slot's value, assignment owner, and coalescing cursor are restored as one
//! unit from the main journal. Only `value` contributes to semantic state.

use crate::epoch::Epoch;
use crate::ids::NodeListId;
use crate::journal::{BoxUndoRec, Journal, JournalPos};
use core::array;

use super::banks::{BoxWriteOutcome, DENSE_REGISTER_COUNT};

const PAGE_LEN: usize = 256;
const PAGE_COUNT: usize = 128;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum BoxOwner {
    #[default]
    Root,
    Group(u32),
}

/// Complete live state for one box register.
///
/// Invariants:
/// - value and bookkeeping are mutated and restored atomically;
/// - a live `coalesce_pos` names the matching live `BoxUndoRec`;
/// - bookkeeping is excluded from semantic hashing and format images.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoxSlot {
    value: u64,
    owner: BoxOwner,
    coalesce_epoch: Epoch,
    coalesce_pos: Option<JournalPos>,
}

impl Default for BoxSlot {
    fn default() -> Self {
        Self {
            value: NodeListId::encode_box_word(None),
            owner: BoxOwner::Root,
            coalesce_epoch: Epoch::ZERO,
            coalesce_pos: None,
        }
    }
}

impl BoxSlot {
    pub(crate) fn value(self) -> u64 {
        self.value
    }

    pub(super) fn is_owned_by(self, depth: u32) -> bool {
        self.owner == BoxOwner::Group(depth)
    }
}

#[derive(Clone, Debug)]
pub(super) struct BoxBank {
    dense: [BoxSlot; DENSE_REGISTER_COUNT],
    sparse: [Option<Box<[BoxSlot; PAGE_LEN]>>; PAGE_COUNT],
}

pub(super) struct BoxWriteContext<'a> {
    pub(super) global: bool,
    pub(super) coalesce: bool,
    pub(super) journal: &'a mut Journal,
    pub(super) epoch: Epoch,
    pub(super) group_depth: u32,
}

impl BoxBank {
    pub(super) fn new() -> Self {
        Self {
            dense: [BoxSlot::default(); DENSE_REGISTER_COUNT],
            sparse: array::from_fn(|_| None),
        }
    }

    pub(super) fn get(&self, index: u16) -> BoxSlot {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            self.dense[usize::from(index)]
        } else {
            let (page, offset) = sparse_location(index);
            self.sparse[page]
                .as_ref()
                .map_or_else(BoxSlot::default, |slots| slots[offset])
        }
    }

    fn get_mut(&mut self, index: u16) -> &mut BoxSlot {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            &mut self.dense[usize::from(index)]
        } else {
            let (page, offset) = sparse_location(index);
            &mut self.sparse[page].get_or_insert_with(|| Box::new([BoxSlot::default(); PAGE_LEN]))
                [offset]
        }
    }

    pub(super) fn write(
        &mut self,
        index: u16,
        value: Option<NodeListId>,
        ctx: BoxWriteContext<'_>,
    ) -> BoxWriteOutcome {
        let old = self.get(index);
        let value = NodeListId::encode_box_word(value);
        if old.value == value && !ctx.global {
            return BoxWriteOutcome::Unchanged;
        }

        let owner = if ctx.global || ctx.group_depth == 0 {
            BoxOwner::Root
        } else {
            BoxOwner::Group(ctx.group_depth)
        };
        let can_coalesce = !ctx.global
            && ctx.coalesce
            && old.owner == owner
            && old.coalesce_epoch == ctx.epoch
            && old.coalesce_pos.is_some();

        if can_coalesce {
            let pos = old.coalesce_pos.expect("checked above");
            let mut new = old;
            new.value = value;
            ctx.journal.replace_box_new(pos, new);
            *self.get_mut(index) = new;
            BoxWriteOutcome::Coalesced {
                displaced: old.value,
            }
        } else {
            let pos = ctx.journal.pos();
            let new = BoxSlot {
                value,
                owner,
                coalesce_epoch: ctx.epoch,
                coalesce_pos: (!ctx.global && ctx.coalesce).then_some(pos),
            };
            let (rec, actual_pos) = ctx
                .journal
                .push_box_undo(BoxUndoRec::new(index, ctx.global, old, new));
            debug_assert_eq!(pos, actual_pos);
            *self.get_mut(index) = new;
            BoxWriteOutcome::Journaled { rec, pos }
        }
    }

    pub(super) fn restore(&mut self, index: u16, slot: BoxSlot) {
        *self.get_mut(index) = slot;
        if usize::from(index) >= DENSE_REGISTER_COUNT {
            let (page, _) = sparse_location(index);
            if self.sparse[page]
                .as_ref()
                .is_some_and(|slots| slots.iter().all(|slot| *slot == BoxSlot::default()))
            {
                self.sparse[page] = None;
            }
        }
    }

    pub(super) fn restore_value(&mut self, index: u16, value: u64) {
        let mut slot = self.get(index);
        slot.value = value;
        self.restore(index, slot);
    }

    #[cfg(test)]
    pub(super) fn has_page_for(&self, index: u16) -> bool {
        let (page, _) = sparse_location(index);
        self.sparse[page].is_some()
    }

    pub(super) fn for_each_non_default_word(&self, mut f: impl FnMut(u16, u64)) {
        for (index, slot) in self.dense.iter().enumerate() {
            if slot.value != NodeListId::encode_box_word(None) {
                f(index as u16, slot.value);
            }
        }
        for (page, slots) in self.sparse.iter().enumerate() {
            let Some(slots) = slots else { continue };
            for (offset, slot) in slots.iter().enumerate() {
                if slot.value != NodeListId::encode_box_word(None) {
                    f(
                        (DENSE_REGISTER_COUNT + page * PAGE_LEN + offset) as u16,
                        slot.value,
                    );
                }
            }
        }
    }
}

fn sparse_location(index: u16) -> (usize, usize) {
    let offset = usize::from(index) - DENSE_REGISTER_COUNT;
    (offset / PAGE_LEN, offset % PAGE_LEN)
}
