//! Derived chunk endpoints for consuming a predecessor-linked list in order.

use super::{ArenaListId, ChunkCursor, ChunkPool, ForkArena, ForkArenaError};
use smallvec::SmallVec;

/// Reusable traversal scratch, never a semantic root or an admission proof.
/// The actual list head advances; only the remaining chunk ends are retained.
pub(crate) struct ArenaListFront<Lane> {
    expected: ArenaListId<Lane>,
    ends: SmallVec<[ChunkCursor<Lane>; 8]>,
}

impl<Lane> Default for ArenaListFront<Lane> {
    fn default() -> Self {
        Self {
            expected: ArenaListId::empty(),
            ends: SmallVec::new(),
        }
    }
}

impl<Lane> ArenaListFront<Lane> {
    fn bind<T>(
        &mut self,
        arena: &ForkArena<T, Lane>,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        arena.validate_list(pool, list)?;
        if self.expected == list {
            return Ok(());
        }
        self.ends.clear();
        self.expected = ArenaListId::empty();
        if list.is_empty() {
            return Ok(());
        }
        let mut end = list.tail;
        let mut remaining = list.len();
        loop {
            let start = if end.raw == list.head.raw {
                list.head.offset
            } else {
                0
            };
            let len = end
                .offset
                .checked_sub(start)
                .ok_or(ForkArenaError::InvalidRange)? as usize;
            if len == 0 {
                return Err(ForkArenaError::InvalidRange);
            }
            remaining = remaining
                .checked_sub(len)
                .ok_or(ForkArenaError::InvalidRange)?;
            self.ends.push(end);
            if end.raw == list.head.raw {
                if remaining != 0 {
                    return Err(ForkArenaError::InvalidRange);
                }
                break;
            }
            let (key, offset) = pool
                .payload
                .previous_in_list(end.raw, arena.owner)?
                .ok_or(ForkArenaError::InvalidRange)?;
            #[cfg(any(test, feature = "testing"))]
            arena.pool_forward_chunk_crossing(pool);
            end = ChunkCursor::new(key, offset);
        }
        self.expected = list;
        Ok(())
    }

    fn next_head(&self, list: ArenaListId<Lane>) -> Result<ChunkCursor<Lane>, ForkArenaError> {
        let end = self.ends.last().ok_or(ForkArenaError::InvalidRange)?;
        let next = list
            .head
            .offset
            .checked_add(1)
            .ok_or(ForkArenaError::InvalidRange)?;
        if next < end.offset {
            Ok(ChunkCursor::new(list.head.raw, next))
        } else if next == end.offset {
            let next = self
                .ends
                .get(
                    self.ends
                        .len()
                        .checked_sub(2)
                        .ok_or(ForkArenaError::InvalidRange)?,
                )
                .ok_or(ForkArenaError::InvalidRange)?;
            Ok(ChunkCursor::new(next.raw, 0))
        } else {
            Err(ForkArenaError::InvalidRange)
        }
    }

    pub(crate) fn second<T>(
        &mut self,
        arena: &ForkArena<T, Lane>,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        if list.len() < 2 {
            return Ok(ArenaListId::empty());
        }
        self.bind(arena, pool, list)?;
        let head = self.next_head(list)?;
        Ok(ArenaListId::from_root(
            list.space,
            head,
            ChunkCursor::new(head.raw, head.offset + 1),
            1,
        ))
    }

    pub(crate) fn take<T>(
        &mut self,
        arena: &ForkArena<T, Lane>,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(ArenaListId<Lane>, ArenaListId<Lane>), ForkArenaError> {
        self.bind(arena, pool, list)?;
        if list.is_empty() {
            return Ok((list, list));
        }
        let end = ChunkCursor::new(list.head.raw, list.head.offset + 1);
        let removed = ArenaListId::from_root(list.space, list.head, end, 1);
        let remaining = if list.len() == 1 {
            self.ends.clear();
            ArenaListId::empty()
        } else {
            let head = self.next_head(list)?;
            if head.raw != list.head.raw {
                self.ends.pop();
            }
            ArenaListId::from_root(list.space, head, list.tail, list.len - 1)
        };
        self.expected = remaining;
        Ok((removed, remaining))
    }
}
