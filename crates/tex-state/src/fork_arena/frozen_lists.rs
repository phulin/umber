//! Pool-level index of frozen arenas whose lists other arenas may borrow.
//!
//! A frozen arena is immutable and outlives every borrower (see
//! `docs/shared_box_closures.md`). A borrowed list therefore needs no
//! ownership proof from the arena that names it: the pool recognizes it by
//! its head chunk's owner. A borrowed list adds no dependency floor: every
//! move of records between node regions logs the source region's borrows in
//! the destination instead.

use super::*;

/// Maps each frozen arena owner to its node-pool registry slot.
#[derive(Default)]
pub(crate) struct FrozenArenaIndex {
    slots: std::collections::HashMap<u32, u32, ahash::RandomState>,
}

impl<T> ChunkPool<T> {
    pub(crate) fn register_frozen_arena(&mut self, arena: u32, slot: u32) {
        let previous = self.frozen.slots.insert(arena, slot);
        assert!(previous.is_none(), "an arena freezes at most once");
    }

    pub(crate) fn unregister_frozen_arena(&mut self, arena: u32) {
        let removed = self.frozen.slots.remove(&arena);
        assert!(removed.is_some(), "only a registered frozen arena retires");
    }

    /// The registry slot of the frozen arena owning nonempty `list`, or
    /// `None` when the list is empty, unowned, or owned by a live arena.
    pub(crate) fn frozen_slot_of_list<Lane>(&self, list: ArenaListId<Lane>) -> Option<u32> {
        if list.is_empty()
            || self.frozen.slots.is_empty()
            || list.space != self.payload.logical_space()
        {
            return None;
        }
        let arena = self.payload.live_chunk_arena(list.head.raw)?;
        if list.tail.raw != list.head.raw && self.payload.live_chunk_arena(list.tail.raw)? != arena
        {
            return None;
        }
        self.frozen.slots.get(&arena).copied()
    }
}

impl<T> ChunkStorage<T> {
    fn live_chunk_arena(&self, key: LogicalChunkId) -> Option<u32> {
        let meta = self.chunks.get(key.ordinal as usize)?;
        (key.incarnation != 0 && meta.live && meta.generation == key.incarnation)
            .then_some(meta.arena)
    }
}

impl<T, Lane> ForkArena<T, Lane> {
    /// Direct dependency of a record on `list`: its head position and its
    /// tail's paired floor. A borrowed list depends on this arena's base, so
    /// only a whole-arena move can carry it without a structural copy. Its
    /// paired data lives in the frozen owner, so it has no paired floor.
    pub(super) fn list_dependency(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(usize, usize), ForkArenaError> {
        let owned = self.validate_list(pool, list).and_then(|()| {
            let head = self
                .resolved_position(pool, list.head.raw)
                .ok_or(ForkArenaError::InvalidRange)?;
            let meta = pool.payload.validate(list.tail.raw, self.owner)?;
            Ok((head, meta.paired_dependency_floor()))
        });
        match owned {
            // A borrowed list is immutable and outlives the borrower, so it
            // constrains no move of the record naming it.
            Err(_) if pool.frozen_slot_of_list(list).is_some() => Ok((usize::MAX, usize::MAX)),
            result => result,
        }
    }
}

impl<T, Lane> ForkArena<T, Lane> {
    /// Visits every live value published at or after payload `start`.
    pub(crate) fn for_each_value_from(
        &self,
        pool: &ChunkPool<T>,
        start: usize,
        mut visit: impl FnMut(&T),
    ) -> Result<(), ForkArenaError> {
        for (_, key) in self.live_positions_from(start) {
            for offset in 0..pool.payload.used(key, self.owner)? {
                visit(
                    pool.payload
                        .get(key, self.owner, offset)
                        .ok_or(ForkArenaError::InvalidChunk)?,
                );
            }
        }
        Ok(())
    }
}
