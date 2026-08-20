//! Box-register live state and its unified write-barrier bookkeeping.
//!
//! A slot's value, assignment owner, and coalescing cursor are restored as one
//! unit from the main journal. Only `value` contributes to semantic state.

use crate::epoch::Epoch;
use crate::ids::NodeListId;
use crate::journal::{BoxUndoRec, Journal, JournalPos};
use crate::node_arena::NodeListRef;
use core::array;

use super::banks::{BoxWriteOutcome, DENSE_REGISTER_COUNT};

const PAGE_LEN: usize = 256;
const PAGE_COUNT: usize = 128;

/// Complete live state for one box register.
///
/// Invariants:
/// - value and bookkeeping are mutated and restored atomically;
/// - a live `coalesce_pos` names the matching live `BoxUndoRec`;
/// - bookkeeping is excluded from semantic hashing and format images.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoxSlot {
    value: u64,
    root: Option<NodeListRef>,
    owner_depth: u32,
    coalesce_epoch: Epoch,
    coalesce_pos: u32,
}

impl Default for BoxSlot {
    fn default() -> Self {
        Self {
            value: NodeListId::encode_box_word(None),
            root: None,
            owner_depth: 0,
            coalesce_epoch: Epoch::ZERO,
            coalesce_pos: 0,
        }
    }
}

impl BoxSlot {
    pub(crate) const fn value(&self) -> u64 {
        self.value
    }

    pub(crate) fn root(&self) -> Option<NodeListRef> {
        self.root.clone()
    }

    pub(crate) const fn owner_depth(&self) -> u32 {
        self.owner_depth
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
            dense: array::from_fn(|_| BoxSlot::default()),
            sparse: array::from_fn(|_| None),
        }
    }

    pub(super) fn get(&self, index: u16) -> BoxSlot {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            self.dense[usize::from(index)].clone()
        } else {
            let (page, offset) = sparse_location(index);
            self.sparse[page]
                .as_ref()
                .map_or_else(BoxSlot::default, |slots| slots[offset].clone())
        }
    }

    fn get_mut(&mut self, index: u16) -> &mut BoxSlot {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            &mut self.dense[usize::from(index)]
        } else {
            let (page, offset) = sparse_location(index);
            &mut self.sparse[page]
                .get_or_insert_with(|| Box::new(array::from_fn(|_| BoxSlot::default())))[offset]
        }
    }

    pub(super) fn write(
        &mut self,
        index: u16,
        root: Option<NodeListRef>,
        ctx: BoxWriteContext<'_>,
    ) -> BoxWriteOutcome {
        let old = self.get(index);
        let value = NodeListId::encode_box_word(root.as_ref().map(NodeListRef::id));
        if old.value == value && !ctx.global && old.owner_depth == ctx.group_depth {
            return BoxWriteOutcome::Unchanged;
        }

        let owner_depth = if ctx.global { 0 } else { ctx.group_depth };
        let can_coalesce = !ctx.global
            && ctx.coalesce
            && old.owner_depth == owner_depth
            && old.coalesce_epoch == ctx.epoch
            && old.coalesce_epoch != Epoch::ZERO;

        if can_coalesce {
            let pos = JournalPos::from_raw(old.coalesce_pos as usize);
            let mut new = old.clone();
            new.value = value;
            new.root = root;
            ctx.journal.replace_box_new(pos, new.clone());
            *self.get_mut(index) = new;
            BoxWriteOutcome::Coalesced
        } else {
            let pos = ctx.journal.pos();
            let new = BoxSlot {
                value,
                root,
                owner_depth,
                coalesce_epoch: if !ctx.global && ctx.coalesce {
                    ctx.epoch
                } else {
                    Epoch::ZERO
                },
                coalesce_pos: pos.raw(),
            };
            let actual_pos =
                ctx.journal
                    .push_box_undo(BoxUndoRec::new(index, ctx.global, old, new.clone()));
            debug_assert_eq!(pos, actual_pos);
            *self.get_mut(index) = new;
            BoxWriteOutcome::Journaled { pos }
        }
    }

    pub(super) fn write_same_level(
        &mut self,
        index: u16,
        root: Option<NodeListRef>,
        journal: &mut Journal,
    ) -> BoxWriteOutcome {
        let old = self.get(index);
        if old.owner_depth == 0 {
            return self.write(
                index,
                root,
                BoxWriteContext {
                    global: true,
                    coalesce: false,
                    journal,
                    epoch: Epoch::ZERO,
                    group_depth: 0,
                },
            );
        }
        let value = NodeListId::encode_box_word(root.as_ref().map(NodeListRef::id));
        if old.value == value {
            return BoxWriteOutcome::Unchanged;
        }
        // TeX82 §§1079/1107 mutate `box(n)` directly while preserving its
        // `eq_level`. The local assignment that established this owner also
        // remains the lifetime owner of the displaced box until its group
        // ends. Leave that undo's `new` slot intact and mutate only the live
        // slot; group exit still restores its `old` slot.
        let mut new = old;
        new.value = value;
        new.root = root;
        *self.get_mut(index) = new;
        BoxWriteOutcome::SameLevel
    }

    pub(super) fn restore(&mut self, index: u16, slot: BoxSlot) {
        *self.get_mut(index) = slot;
        if usize::from(index) >= DENSE_REGISTER_COUNT {
            let (page, _) = sparse_location(index);
            if self.sparse[page]
                .as_ref()
                .is_some_and(|slots| slots.iter().all(|slot| slot == &BoxSlot::default()))
            {
                self.sparse[page] = None;
            }
        }
    }

    pub(super) fn restore_value(&mut self, index: u16, value: u64, root: Option<NodeListRef>) {
        assert_eq!(
            NodeListId::decode_box_word(value),
            root.as_ref().map(NodeListRef::id),
            "box word and structural owner disagree"
        );
        let mut slot = self.get(index);
        slot.value = value;
        slot.root = root;
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
