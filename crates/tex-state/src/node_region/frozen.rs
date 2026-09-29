//! Frozen durable regions whose box bodies other regions borrow.
//!
//! See `docs/shared_box_closures.md`. The pool owns every frozen region and
//! counts its shares. Each region logs the frozen regions it may name; every
//! log entry holds one share, and retiring the region releases them. A
//! frozen region whose last share is released retires in turn, releasing its
//! own log through the same worklist.

use super::*;

pub(super) struct FrozenSlot {
    region: Option<NodeRegion<DurableRole>>,
    shares: u32,
}

#[derive(Default)]
pub(super) struct FrozenRegistry {
    slots: Vec<FrozenSlot>,
    free: Vec<u32>,
}

/// Observations of shared box bodies. `frozen - retired` frozen regions are
/// live.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrozenRegionCounters {
    pub frozen: u64,
    pub retired: u64,
    pub shared_bodies: u64,
    pub materialized_lists: u64,
}

impl FrozenRegistry {
    pub(super) fn region(&self, slot: u32) -> Option<&NodeRegion<DurableRole>> {
        self.slots.get(slot as usize)?.region.as_ref()
    }
}

impl NodePool {
    /// Moves an exclusive, quiescent durable region into the registry, where
    /// it stays immutable until its last share is released. The caller must
    /// log the first share before any other pool operation.
    #[allow(clippy::result_large_err)] // Failure returns the exclusive owner.
    pub(crate) fn freeze_region(
        &mut self,
        region: NodeRegion<DurableRole>,
    ) -> Result<u32, (ForkArenaError, NodeRegion<DurableRole>)> {
        if let Err(error) = self
            .validate_region(&region)
            .and_then(|()| region.pub_arena.can_retire_region(&self.chunks))
            .and_then(|()| region.annex_arena.can_retire_region(&self.annex_chunks))
        {
            return Err((error, region));
        }
        let arena = region.pub_arena.region_identity();
        let slot = if let Some(slot) = self.frozen.free.pop() {
            let entry = &mut self.frozen.slots[slot as usize];
            debug_assert!(entry.region.is_none() && entry.shares == 0);
            entry.region = Some(region);
            slot
        } else {
            let slot = u32::try_from(self.frozen.slots.len()).expect("frozen slots fit u32");
            self.frozen.slots.push(FrozenSlot {
                region: Some(region),
                shares: 0,
            });
            slot
        };
        self.chunks.register_frozen_arena(arena, slot);
        self.frozen_counters.frozen = self.frozen_counters.frozen.saturating_add(1);
        Ok(slot)
    }

    /// The frozen region owning borrowed `list`, if any.
    pub(crate) fn frozen_region_of(&self, list: PageListId) -> Option<&NodeRegion<DurableRole>> {
        let slot = self.chunks.frozen_slot_of_list(list.coordinate())?;
        self.frozen.slots.get(slot as usize)?.region.as_ref()
    }

    pub(crate) fn frozen_slot_of(&self, list: PageListId) -> Option<u32> {
        self.chunks.frozen_slot_of_list(list.coordinate())
    }

    /// Resolves a borrowed list through its frozen owner.
    pub(crate) fn borrowed_cursor(
        &self,
        list: PageListId,
    ) -> Option<crate::node_view::NodeCursor<'_>> {
        let region = self.frozen_region_of(list)?;
        let view = region
            .pub_arena
            .list(&self.chunks, list.coordinate())
            .ok()?;
        let annex = NodeAnnexView::new(&self.annex_chunks, &region.annex_arena);
        Some(crate::node_view::NodeCursor::fork_arena(view, annex))
    }

    /// Records that `region` may name lists of frozen region `slot`.
    pub(crate) fn log_borrow<Role>(&mut self, region: &mut NodeRegion<Role>, slot: u32) {
        if region.borrows.contains(&slot) {
            return;
        }
        let entry = &mut self.frozen.slots[slot as usize];
        debug_assert!(entry.region.is_some());
        entry.shares = entry.shares.checked_add(1).expect("frozen share count");
        region.borrows.push(slot);
    }

    /// Makes `destination` log every frozen region `source` may name. Used
    /// when records move from `source` without a floor proof excluding
    /// borrowed coordinates.
    pub(crate) fn inherit_borrows<Source, Destination>(
        &mut self,
        source: &NodeRegion<Source>,
        destination: &mut NodeRegion<Destination>,
    ) {
        for &slot in &source.borrows {
            self.log_borrow(destination, slot);
        }
    }

    /// Every frozen region named by a child list of a record published at
    /// or after node position `start` of `region`.
    pub(super) fn frozen_slots_named_from<Role>(
        &self,
        region: &NodeRegion<Role>,
        start: usize,
    ) -> Result<Vec<u32>, ForkArenaError> {
        let annex = NodeAnnexView::new(&self.annex_chunks, &region.annex_arena);
        let mut named = Vec::new();
        let mut valid = true;
        region
            .pub_arena
            .for_each_value_from(&self.chunks, start, |record| {
                if record.is_inline_leaf() {
                    return;
                }
                valid &= record
                    .visit_node_lists(annex, |list| {
                        if let Some(slot) = self.chunks.frozen_slot_of_list(list.coordinate())
                            && !named.contains(&slot)
                        {
                            named.push(slot);
                        }
                    })
                    .is_some();
            })?;
        valid.then_some(named).ok_or(ForkArenaError::InvalidRange)
    }

    pub(super) fn release_borrows(&mut self, mut work: Vec<u32>) {
        while let Some(slot) = work.pop() {
            let entry = &mut self.frozen.slots[slot as usize];
            entry.shares = entry
                .shares
                .checked_sub(1)
                .expect("released frozen share was held");
            if entry.shares != 0 {
                continue;
            }
            let mut region = entry.region.take().expect("shared frozen region is live");
            self.chunks
                .unregister_frozen_arena(region.pub_arena.region_identity());
            self.retire_region_storage(&mut region)
                .expect("frozen region retirement was preflighted at freezing");
            work.append(&mut region.borrows);
            self.frozen.free.push(slot);
            self.frozen_counters.retired = self.frozen_counters.retired.saturating_add(1);
        }
    }

    #[must_use]
    pub const fn frozen_region_counters(&self) -> FrozenRegionCounters {
        self.frozen_counters
    }
}

impl NodePool {
    /// Splits durable closure C whose root is one box into a frozen region F
    /// (all of C) and a fresh one-record closure C' whose root box borrows
    /// F's body. TeX cannot observe the split. Declines, returning C
    /// unchanged, when C is not a single unshared box or cannot be moved.
    #[allow(clippy::result_large_err)] // Failure returns the exclusive owner.
    pub(crate) fn freeze_closure_body(
        &mut self,
        closure: OwnedNodeClosure<DurableRole>,
        semantic_identity_enabled: bool,
    ) -> Result<OwnedNodeClosure<DurableRole>, OwnedNodeClosure<DurableRole>> {
        if !self.can_freeze_closure_body(&closure) {
            return Err(closure);
        }
        let Ok(mut replacement) = self.start_region::<DurableRole>() else {
            return Err(closure);
        };
        let root = closure.root.list;
        let slot = match self.freeze_region(closure.region) {
            Ok(slot) => slot,
            Err((_, region)) => {
                self.retire_region(replacement)
                    .unwrap_or_else(|_| unreachable!("fresh replacement retires"));
                return Err(OwnedNodeClosure {
                    region,
                    root: closure.root,
                    #[cfg(feature = "profiling")]
                    profiled_fresh_recursive_copy: closure.profiled_fresh_recursive_copy,
                });
            }
        };
        let copied =
            super::copy_borrowed_list_into(self, root, &mut replacement, semantic_identity_enabled)
                .expect("a frozen single-box root copies into a fresh region");
        // The copy logged F's body share; the source slot itself holds no
        // further share, so the replacement's log keeps F alive.
        debug_assert!(replacement.borrows.contains(&slot));
        Ok(replacement
            .into_closure(self, copied)
            .unwrap_or_else(|_| unreachable!("fresh replacement admits its copied root")))
    }

    fn can_freeze_closure_body(&self, closure: &OwnedNodeClosure<DurableRole>) -> bool {
        // Small closures copy faster than they split, and sharing them only
        // trades a deep copy for a shallow one plus registry traffic.
        const MIN_FROZEN_CHUNKS: usize = 8;
        let region = &closure.region;
        if region.pub_arena.live_payload_interval().len() < MIN_FROZEN_CHUNKS
            || region.pub_arena.is_forked()
            || region.annex_arena.is_forked()
            || closure.root.list.len() != 1
        {
            return false;
        }
        let Some(root) = closure
            .list(self)
            .ok()
            .and_then(|cursor| cursor.first().map(|node| node.to_owned()))
        else {
            return false;
        };
        matches!(
            root,
            Node::HList(boxed) | Node::VList(boxed)
                if !boxed.children.is_empty() && boxed.diagnostic_children.is_none()
        )
    }
}
