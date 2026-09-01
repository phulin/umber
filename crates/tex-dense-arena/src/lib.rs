//! Safe flat-table arenas and lifetime-specific policy wrappers.

#![forbid(unsafe_code)]

use core::fmt;
use core::mem::size_of;
use core::sync::atomic::{AtomicU32, Ordering};

use tex_dense_prefix::{CapacityError, LayoutError, Superblock, VacantSlot};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests;

static NEXT_ARENA_ID: AtomicU32 = AtomicU32::new(1);

fn fresh_arena_id() -> u32 {
    NEXT_ARENA_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("dense arena id domain exhausted")
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BlockId {
    slot: u32,
    incarnation: u32,
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
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ArenaMetrics {
    pub superblocks_allocated: u64,
    pub superblocks_reused: u64,
    pub superblocks_released: u64,
    pub blocks_truncated: u64,
    pub values_constructed: u64,
    pub values_truncated: u64,
    pub direct_lookups: u64,
    pub descriptor_visits: u64,
    pub cursor_captures: u64,
    pub forked_arenas: u64,
    pub fork_tail_values_copied: u64,
    pub fork_tail_bytes_copied: u64,
    pub table_entries_copied: u64,
    pub table_bytes_copied: u64,
    pub accepted_payload_copies: u64,
    pub rejected_payload_copies: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ForkShape {
    pub accepted_blocks: usize,
    pub candidate_blocks: usize,
    pub shared_complete_blocks: usize,
    pub candidate_private_blocks: usize,
}

#[derive(Debug)]
pub enum ArenaError {
    Layout(LayoutError),
    FullBlock,
    BlockIdDomainExhausted,
    IncarnationExhausted,
    LogicalLengthOverflow,
    InvalidCursor,
    StaleBlock,
}

impl fmt::Display for ArenaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => error.fmt(formatter),
            Self::FullBlock => formatter.write_str("unexpected full superblock"),
            Self::BlockIdDomainExhausted => formatter.write_str("block id domain exhausted"),
            Self::IncarnationExhausted => formatter.write_str("block incarnation exhausted"),
            Self::LogicalLengthOverflow => formatter.write_str("logical length overflow"),
            Self::InvalidCursor => formatter.write_str("invalid or foreign cursor"),
            Self::StaleBlock => formatter.write_str("stale superblock id"),
        }
    }
}

impl std::error::Error for ArenaError {}

impl From<LayoutError> for ArenaError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<CapacityError> for ArenaError {
    fn from(_: CapacityError) -> Self {
        Self::FullBlock
    }
}

struct BlockSlot<T> {
    incarnation: u32,
    live: bool,
    block: Superblock<T>,
}

struct BlockStore<T> {
    slots: Vec<BlockSlot<T>>,
    free: Vec<u32>,
}

impl<T> BlockStore<T> {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    fn allocate(&mut self, metrics: &mut ArenaMetrics) -> Result<BlockId, ArenaError> {
        if let Some(slot) = self.free.pop() {
            let entry = self
                .slots
                .get_mut(slot as usize)
                .ok_or(ArenaError::StaleBlock)?;
            let incarnation = entry
                .incarnation
                .checked_add(1)
                .ok_or(ArenaError::IncarnationExhausted)?;
            debug_assert!(!entry.live && entry.block.is_empty());
            entry.incarnation = incarnation;
            entry.live = true;
            metrics.superblocks_reused += 1;
            return Ok(BlockId { slot, incarnation });
        }
        let slot =
            u32::try_from(self.slots.len()).map_err(|_| ArenaError::BlockIdDomainExhausted)?;
        let block = Superblock::try_new()?;
        self.slots.push(BlockSlot {
            incarnation: 1,
            live: true,
            block,
        });
        metrics.superblocks_allocated += 1;
        Ok(BlockId {
            slot,
            incarnation: 1,
        })
    }

    fn resolve(&self, id: BlockId) -> Result<&Superblock<T>, ArenaError> {
        let entry = self
            .slots
            .get(id.slot as usize)
            .ok_or(ArenaError::StaleBlock)?;
        if !entry.live || entry.incarnation != id.incarnation {
            return Err(ArenaError::StaleBlock);
        }
        Ok(&entry.block)
    }

    fn resolve_mut(&mut self, id: BlockId) -> Result<&mut Superblock<T>, ArenaError> {
        let entry = self
            .slots
            .get_mut(id.slot as usize)
            .ok_or(ArenaError::StaleBlock)?;
        if !entry.live || entry.incarnation != id.incarnation {
            return Err(ArenaError::StaleBlock);
        }
        Ok(&mut entry.block)
    }

    fn resolve_pair(
        &mut self,
        source: BlockId,
        destination: BlockId,
    ) -> Result<(&Superblock<T>, &mut Superblock<T>), ArenaError> {
        if source.slot == destination.slot {
            return Err(ArenaError::StaleBlock);
        }
        let source_index = source.slot as usize;
        let destination_index = destination.slot as usize;
        let (source_entry, destination_entry) = if source_index < destination_index {
            let (left, right) = self.slots.split_at_mut(destination_index);
            (&left[source_index], &mut right[0])
        } else {
            let (left, right) = self.slots.split_at_mut(source_index);
            (&right[0], &mut left[destination_index])
        };
        if !source_entry.live
            || source_entry.incarnation != source.incarnation
            || !destination_entry.live
            || destination_entry.incarnation != destination.incarnation
        {
            return Err(ArenaError::StaleBlock);
        }
        Ok((&source_entry.block, &mut destination_entry.block))
    }

    fn release(&mut self, id: BlockId, metrics: &mut ArenaMetrics) -> Result<(), ArenaError> {
        let entry = self
            .slots
            .get_mut(id.slot as usize)
            .ok_or(ArenaError::StaleBlock)?;
        if !entry.live || entry.incarnation != id.incarnation {
            return Err(ArenaError::StaleBlock);
        }
        let removed = entry.block.len();
        entry.block.truncate(0);
        entry.live = false;
        self.free.push(id.slot);
        metrics.superblocks_released += 1;
        metrics.values_truncated += removed as u64;
        Ok(())
    }
}

/// Dense values addressed by quotient/remainder and one flat table lookup.
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
    pub const fn metrics(&self) -> ArenaMetrics {
        self.metrics
    }

    #[must_use]
    pub fn block_ids(&self) -> &[BlockId] {
        &self.blocks
    }

    #[must_use]
    pub fn is_live_block(&self, id: BlockId) -> bool {
        self.store.resolve(id).is_ok()
    }

    fn append_block(&mut self) -> Result<BlockId, ArenaError> {
        let id = self.store.allocate(&mut self.metrics)?;
        self.blocks.push(id);
        Ok(id)
    }

    fn tail_for_push(&mut self) -> Result<BlockId, ArenaError> {
        let capacity = Self::items_per_block();
        if self.len % capacity != 0 {
            return self.blocks.last().copied().ok_or(ArenaError::StaleBlock);
        }
        if let Some(id) = self.blocks.last().copied()
            && self.store.resolve(id)?.is_empty()
        {
            return Ok(id);
        }
        self.append_block()
    }

    pub fn push_with<F>(&mut self, build: F) -> Result<usize, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        let index = self.len;
        let id = self.tail_for_push()?;
        self.store.resolve_mut(id)?.push_with(build)?;
        self.len = self
            .len
            .checked_add(1)
            .ok_or(ArenaError::LogicalLengthOverflow)?;
        self.metrics.values_constructed += 1;
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
        let capacity = Self::items_per_block();
        let required_blocks = new_len.div_ceil(capacity);
        while self.blocks.len() > required_blocks {
            let id = self.blocks.pop().ok_or(ArenaError::StaleBlock)?;
            self.store.release(id, &mut self.metrics)?;
        }
        if let Some(id) = self.blocks.last().copied() {
            let tail_len = new_len % capacity;
            if tail_len != 0 {
                let old_len = self.store.resolve(id)?.len();
                self.store.resolve_mut(id)?.truncate(tail_len);
                self.metrics.values_truncated += (old_len - tail_len) as u64;
                self.metrics.blocks_truncated += 1;
            }
        }
        self.len = new_len;
        Ok(())
    }

    fn from_parts(
        arena_id: u32,
        store: BlockStore<T>,
        blocks: Vec<BlockId>,
        len: usize,
        metrics: ArenaMetrics,
    ) -> Self {
        Self {
            arena_id,
            store,
            blocks,
            len,
            metrics,
        }
    }
}

/// Generation-owned arena. Forking is available only when `T: Copy`.
pub struct GenerationArena<T>(DenseArena<T>);

impl<T> Default for GenerationArena<T> {
    fn default() -> Self {
        Self(DenseArena::new())
    }
}

impl<T> GenerationArena<T> {
    pub fn push_with<F>(&mut self, build: F) -> Result<usize, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        self.0.push_with(build)
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.0.get(index)
    }

    pub fn cursor(&mut self) -> ArenaCursor {
        self.0.cursor()
    }

    #[must_use]
    pub const fn metrics(&self) -> ArenaMetrics {
        self.0.metrics()
    }

    #[must_use]
    pub fn block_ids(&self) -> &[BlockId] {
        self.0.block_ids()
    }

    #[must_use]
    pub fn is_live_block(&self, id: BlockId) -> bool {
        self.0.is_live_block(id)
    }
}

impl<T: Copy> GenerationArena<T> {
    pub fn fork(self, checkpoint: ArenaCursor) -> Result<GenerationFork<T>, ArenaError> {
        let checkpoint_len = self.0.validate_cursor(checkpoint)?;
        let capacity = DenseArena::<T>::items_per_block();
        let shared_complete = checkpoint_len / capacity;
        let tail_len = checkpoint_len % capacity;
        let DenseArena {
            arena_id,
            store,
            blocks: accepted_blocks,
            len: accepted_len,
            mut metrics,
        } = self.0;
        let mut candidate_blocks = Vec::with_capacity(shared_complete + usize::from(tail_len > 0));
        candidate_blocks.extend_from_slice(&accepted_blocks[..shared_complete]);
        metrics.forked_arenas += 1;
        metrics.table_entries_copied += shared_complete as u64;
        metrics.table_bytes_copied += (shared_complete * size_of::<BlockId>()) as u64;
        let mut fork = GenerationFork {
            arena_id,
            store,
            accepted_blocks,
            accepted_len,
            candidate_blocks,
            candidate_len: shared_complete * capacity,
            shared_complete,
            metrics,
        };
        if tail_len > 0 {
            let source_id = fork.accepted_blocks[shared_complete];
            let destination_id = fork.store.allocate(&mut fork.metrics)?;
            fork.candidate_blocks.push(destination_id);
            let (source, destination) = fork.store.resolve_pair(source_id, destination_id)?;
            destination.extend_copy_from_slice(&source.initialized()[..tail_len])?;
            fork.candidate_len += tail_len;
            fork.metrics.values_constructed += tail_len as u64;
            fork.metrics.fork_tail_values_copied += tail_len as u64;
            fork.metrics.fork_tail_bytes_copied += (tail_len * size_of::<T>()) as u64;
        }
        Ok(fork)
    }
}

/// Sole owner of accepted and candidate views while one fork is live.
pub struct GenerationFork<T: Copy> {
    arena_id: u32,
    store: BlockStore<T>,
    accepted_blocks: Vec<BlockId>,
    accepted_len: usize,
    candidate_blocks: Vec<BlockId>,
    candidate_len: usize,
    shared_complete: usize,
    metrics: ArenaMetrics,
}

impl<T: Copy> GenerationFork<T> {
    fn table_get<'a>(
        store: &'a BlockStore<T>,
        table: &[BlockId],
        len: usize,
        index: usize,
    ) -> Option<&'a T> {
        if index >= len {
            return None;
        }
        let capacity = Superblock::<T>::capacity();
        let id = *table.get(index / capacity)?;
        store.resolve(id).ok()?.get(index % capacity)
    }

    #[must_use]
    pub fn candidate_get(&self, index: usize) -> Option<&T> {
        Self::table_get(
            &self.store,
            &self.candidate_blocks,
            self.candidate_len,
            index,
        )
    }

    fn push_copy(&mut self, value: T, fork_tail: bool) -> Result<usize, ArenaError> {
        let capacity = Superblock::<T>::capacity();
        if self.candidate_len % capacity == 0 {
            let id = self.store.allocate(&mut self.metrics)?;
            self.candidate_blocks.push(id);
        }
        let id = *self.candidate_blocks.last().ok_or(ArenaError::StaleBlock)?;
        self.store
            .resolve_mut(id)?
            .push_with(|slot| slot.insert(value))?;
        let index = self.candidate_len;
        self.candidate_len += 1;
        self.metrics.values_constructed += 1;
        if fork_tail {
            self.metrics.fork_tail_values_copied += 1;
            self.metrics.fork_tail_bytes_copied += size_of::<T>() as u64;
        }
        Ok(index)
    }

    pub fn candidate_push(&mut self, value: T) -> Result<usize, ArenaError> {
        self.push_copy(value, false)
    }

    #[must_use]
    pub fn shape(&self) -> ForkShape {
        ForkShape {
            accepted_blocks: self.accepted_blocks.len(),
            candidate_blocks: self.candidate_blocks.len(),
            shared_complete_blocks: self.shared_complete,
            candidate_private_blocks: self.candidate_blocks.len() - self.shared_complete,
        }
    }

    #[must_use]
    pub const fn metrics(&self) -> ArenaMetrics {
        self.metrics
    }

    pub fn accept(mut self) -> Result<GenerationArena<T>, ArenaError> {
        for id in self.accepted_blocks.drain(self.shared_complete..) {
            self.store.release(id, &mut self.metrics)?;
        }
        Ok(GenerationArena(DenseArena::from_parts(
            self.arena_id,
            self.store,
            self.candidate_blocks,
            self.candidate_len,
            self.metrics,
        )))
    }

    pub fn reject(mut self) -> Result<GenerationArena<T>, ArenaError> {
        for id in self.candidate_blocks.drain(self.shared_complete..) {
            self.store.release(id, &mut self.metrics)?;
        }
        Ok(GenerationArena(DenseArena::from_parts(
            self.arena_id,
            self.store,
            self.accepted_blocks,
            self.accepted_len,
            self.metrics,
        )))
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

/// Group-local storage: mark, restore meanings externally, then truncate.
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

/// Reusable page-attempt scratch; completion moves the whole owner.
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
}

/// Save/checkpoint journal storage. Capturing a mark only records a cursor.
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

/// Candidate output is private and is moved on commit or dropped on rejection.
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
}
