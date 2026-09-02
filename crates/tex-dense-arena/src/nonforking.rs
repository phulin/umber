use core::sync::atomic::{AtomicU32, Ordering};

use tex_dense_prefix::{Superblock, VacantSlot};

use crate::store::BlockId;
use crate::{ArenaError, ArenaMetrics, BlockStore};

static NEXT_ARENA_ID: AtomicU32 = AtomicU32::new(1);

fn fresh_arena_id() -> u32 {
    NEXT_ARENA_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("dense arena id domain exhausted")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArenaCursor {
    arena: u32,
    len: u64,
    boundary_block: Option<BlockId>,
}

impl ArenaCursor {
    #[must_use]
    pub const fn len(self) -> u64 {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Dense nonforking storage used by the lifetime-specific wrappers below.
pub struct DenseArena<T> {
    arena_id: u32,
    store: BlockStore<T>,
    blocks: Vec<BlockId>,
    len: usize,
    metrics: ArenaMetrics,
}

impl<T> Default for DenseArena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> DenseArena<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            arena_id: fresh_arena_id(),
            store: BlockStore::new(),
            blocks: Vec::new(),
            len: 0,
            metrics: ArenaMetrics::default(),
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub const fn items_per_block() -> usize {
        Superblock::<T>::capacity()
    }

    #[must_use]
    pub fn metrics(&self) -> ArenaMetrics {
        self.metrics.merged(self.store.metrics())
    }

    fn tail_for_push(&mut self) -> Result<BlockId, ArenaError> {
        let capacity = Self::items_per_block();
        if !self.len.is_multiple_of(capacity) {
            return self
                .blocks
                .last()
                .copied()
                .ok_or(ArenaError::StalePhysicalBlock);
        }
        if let Some(id) = self.blocks.last().copied()
            && self.store.resolve(id)?.is_empty()
        {
            return Ok(id);
        }
        self.blocks.try_reserve(1)?;
        let id = self.store.allocate()?;
        self.blocks.push(id);
        Ok(id)
    }

    pub fn push_with<F>(&mut self, build: F) -> Result<usize, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        let index = self.len;
        let id = self.tail_for_push()?;
        self.store.push_with(id, build)?;
        self.len = self
            .len
            .checked_add(1)
            .ok_or(ArenaError::LogicalLengthOverflow)?;
        Ok(index)
    }

    fn coordinates(index: usize) -> (usize, usize) {
        let capacity = Self::items_per_block();
        (index / capacity, index % capacity)
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let (block_index, offset) = Self::coordinates(index);
        let id = *self.blocks.get(block_index)?;
        self.store.resolve(id).ok()?.get(offset)
    }

    #[must_use]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        let (block_index, offset) = Self::coordinates(index);
        let id = *self.blocks.get(block_index)?;
        self.store.resolve_mut(id).ok()?.get_mut(offset)
    }

    pub fn record_direct_lookup(&mut self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        self.metrics.direct_lookups += 1;
        self.get(index)
    }

    pub fn cursor(&mut self) -> ArenaCursor {
        self.metrics.cursor_captures += 1;
        let boundary_block = self
            .len
            .checked_sub(1)
            .and_then(|index| self.blocks.get(index / Self::items_per_block()))
            .copied();
        ArenaCursor {
            arena: self.arena_id,
            len: self.len as u64,
            boundary_block,
        }
    }

    fn validate_cursor(&self, cursor: ArenaCursor) -> Result<usize, ArenaError> {
        if cursor.arena != self.arena_id {
            return Err(ArenaError::InvalidCursor);
        }
        let len = usize::try_from(cursor.len).map_err(|_| ArenaError::InvalidCursor)?;
        if len > self.len {
            return Err(ArenaError::InvalidCursor);
        }
        let expected = len
            .checked_sub(1)
            .and_then(|index| self.blocks.get(index / Self::items_per_block()))
            .copied();
        if expected != cursor.boundary_block {
            return Err(ArenaError::InvalidCursor);
        }
        if let Some(id) = expected {
            let offset = (len - 1) % Self::items_per_block();
            if self.store.resolve(id)?.len() <= offset {
                return Err(ArenaError::InvalidCursor);
            }
        }
        Ok(len)
    }

    pub fn truncate(&mut self, cursor: ArenaCursor) -> Result<(), ArenaError> {
        let new_len = self.validate_cursor(cursor)?;
        self.len = new_len;
        let capacity = Self::items_per_block();
        let required_blocks = new_len.div_ceil(capacity);
        while self.blocks.len() > required_blocks {
            let id = self.blocks.pop().ok_or(ArenaError::StalePhysicalBlock)?;
            self.store.release(id)?;
        }
        if let Some(id) = self.blocks.last().copied() {
            let tail_len = new_len % capacity;
            if tail_len != 0 {
                self.store.truncate(id, tail_len)?;
            }
        }
        Ok(())
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn physical_blocks(&self) -> &[BlockId] {
        &self.blocks
    }
}

macro_rules! cursor_wrapper {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $name(ArenaCursor);
    };
}

cursor_wrapper!(GroupMark);
cursor_wrapper!(AttemptMark);
cursor_wrapper!(JournalMark);

/// Group-local storage; it deliberately has no generation-fork API.
pub struct GroupStorage<T>(DenseArena<T>);

impl<T> Default for GroupStorage<T> {
    fn default() -> Self {
        Self(DenseArena::new())
    }
}

impl<T> GroupStorage<T> {
    pub fn push_with<F>(&mut self, build: F) -> Result<usize, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        self.0.push_with(build)
    }

    pub fn enter_group(&mut self) -> GroupMark {
        GroupMark(self.0.cursor())
    }

    pub fn leave_group(&mut self, mark: GroupMark) -> Result<(), ArenaError> {
        self.0.truncate(mark.0)
    }
}

pub struct PageAttemptScratch<T>(DenseArena<T>);
pub struct CompletedScratch<T>(DenseArena<T>);

impl<T> Default for PageAttemptScratch<T> {
    fn default() -> Self {
        Self(DenseArena::new())
    }
}

impl<T> PageAttemptScratch<T> {
    pub fn push_with<F>(&mut self, build: F) -> Result<usize, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        self.0.push_with(build)
    }

    pub fn begin_attempt(&mut self) -> AttemptMark {
        AttemptMark(self.0.cursor())
    }

    pub fn rewind(&mut self, mark: AttemptMark) -> Result<(), ArenaError> {
        self.0.truncate(mark.0)
    }

    #[must_use]
    pub fn finish(self) -> CompletedScratch<T> {
        CompletedScratch(self.0)
    }
}

impl<T> CompletedScratch<T> {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub struct CheckpointJournal<T>(DenseArena<T>);

impl<T> Default for CheckpointJournal<T> {
    fn default() -> Self {
        Self(DenseArena::new())
    }
}

impl<T> CheckpointJournal<T> {
    pub fn push_with<F>(&mut self, build: F) -> Result<usize, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        self.0.push_with(build)
    }

    pub fn save(&mut self) -> JournalMark {
        JournalMark(self.0.cursor())
    }

    pub fn restore(&mut self, mark: JournalMark) -> Result<(), ArenaError> {
        self.0.truncate(mark.0)
    }
}

pub struct SpeculativeOutput<T>(DenseArena<T>);
pub struct CommittedOutput<T>(DenseArena<T>);

impl<T> Default for SpeculativeOutput<T> {
    fn default() -> Self {
        Self(DenseArena::new())
    }
}

impl<T> SpeculativeOutput<T> {
    pub fn push_with<F>(&mut self, build: F) -> Result<usize, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        self.0.push_with(build)
    }

    #[must_use]
    pub fn commit(self) -> CommittedOutput<T> {
        CommittedOutput(self.0)
    }

    pub fn reject(self) {}
}

impl<T> CommittedOutput<T> {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
