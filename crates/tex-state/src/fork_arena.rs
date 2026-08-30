//! Fixed-chunk storage with one accepted lineage and one transactional fork.
//!
//! Payload lives in coarse pool pages subdivided into packed logical list
//! blocks. Direct roots carry head/tail cursors and length; reverse traversal
//! follows block metadata without a descriptor lookup. Retained checkpoints
//! land on sealed whole-block boundaries, while operation marks may name the
//! private current tail's used cursor. Each block's owner-relative position
//! lives beside that block's pool metadata, so short-lived regions never
//! materialize sparse indexes up to a process-global pool slot.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::node_sequence::SemanticSequenceIdentity;

#[cfg(test)]
#[path = "fork_arena/tests.rs"]
mod tests;

const DEFAULT_CHUNK_BYTES: usize = 512;
const CHUNKS_PER_PAGE: usize = 16;

static NEXT_POOL_OWNER: AtomicU32 = AtomicU32::new(1);
static NEXT_ARENA_OWNER: AtomicU32 = AtomicU32::new(1);

/// Canonical page-material lane used by execution and borrowed typesetting.
pub enum PageMaterialLane {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[doc(hidden)]
pub struct RawChunkKey {
    slot: u32,
    generation: u32,
}

#[derive(Clone, Copy, Debug)]
struct ChunkMeta {
    generation: u32,
    arena: u32,
    // This index has the same lifetime as the chunk incarnation. Transfers
    // overwrite it in place; detach and release invalidate it in place.
    arena_position: usize,
    used: u32,
    live: bool,
    sealed: bool,
    sequence_summary: Option<SemanticSequenceIdentity>,
    previous_in_list: Option<(RawChunkKey, u32)>,
}

const UNINDEXED_ARENA_POSITION: usize = usize::MAX;

struct ChunkPage<T> {
    slots: Box<[Option<T>]>,
}

/// Stable coarse-page storage shared by typed semantic lanes.
///
/// A logical chunk is a fixed slot range inside a page allocation. Growing
/// the pool allocates one page for many chunks, so no chunk owns a heap
/// allocation and existing payload never moves.
struct ChunkStorage<T> {
    chunk_bytes: usize,
    slots_per_chunk: usize,
    pages: Vec<ChunkPage<T>>,
    chunks: Vec<ChunkMeta>,
    free: Vec<u32>,
    #[cfg(test)]
    validation_reads: core::cell::Cell<u64>,
    #[cfg(test)]
    previous_link_reads: core::cell::Cell<u64>,
    #[cfg(any(test, feature = "testing"))]
    admitted_index_resolutions: core::cell::Cell<u64>,
    #[cfg(any(test, feature = "testing"))]
    admitted_index_predecessor_steps: core::cell::Cell<u64>,
    #[cfg(any(test, feature = "testing"))]
    admitted_forward_chunk_crossings: core::cell::Cell<u64>,
}

impl<T> ChunkStorage<T> {
    /// Creates a pool whose logical chunk capacity is derived from a byte
    /// budget. At least one value fits even when `T` exceeds that budget.
    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        assert!(chunk_bytes != 0, "chunk byte budget must be nonzero");
        let slot_bytes = std::mem::size_of::<Option<T>>().max(1);
        let slots_per_chunk = (chunk_bytes / slot_bytes).max(1);
        Self {
            chunk_bytes,
            slots_per_chunk,
            pages: Vec::new(),
            chunks: Vec::new(),
            free: Vec::new(),
            #[cfg(test)]
            validation_reads: core::cell::Cell::new(0),
            #[cfg(test)]
            previous_link_reads: core::cell::Cell::new(0),
            #[cfg(any(test, feature = "testing"))]
            admitted_index_resolutions: core::cell::Cell::new(0),
            #[cfg(any(test, feature = "testing"))]
            admitted_index_predecessor_steps: core::cell::Cell::new(0),
            #[cfg(any(test, feature = "testing"))]
            admitted_forward_chunk_crossings: core::cell::Cell::new(0),
        }
    }

    #[cfg(test)]
    fn validation_reads(&self) -> u64 {
        self.validation_reads.get()
    }

    #[cfg(test)]
    fn previous_link_reads(&self) -> u64 {
        self.previous_link_reads.get()
    }

    #[cfg(any(test, feature = "testing"))]
    fn admitted_index_resolutions(&self) -> u64 {
        self.admitted_index_resolutions.get()
    }

    #[cfg(any(test, feature = "testing"))]
    fn admitted_index_predecessor_steps(&self) -> u64 {
        self.admitted_index_predecessor_steps.get()
    }

    #[cfg(any(test, feature = "testing"))]
    fn admitted_forward_chunk_crossings(&self) -> u64 {
        self.admitted_forward_chunk_crossings.get()
    }

    #[must_use]
    pub const fn chunk_byte_budget(&self) -> usize {
        self.chunk_bytes
    }

    #[must_use]
    pub const fn chunk_capacity(&self) -> usize {
        self.slots_per_chunk
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn allocated_heap_bytes(&self) -> usize {
        self.pages
            .capacity()
            .saturating_mul(std::mem::size_of::<ChunkPage<T>>())
            .saturating_add(self.pages.iter().fold(0_usize, |bytes, page| {
                bytes.saturating_add(
                    page.slots
                        .len()
                        .saturating_mul(std::mem::size_of::<Option<T>>()),
                )
            }))
            .saturating_add(
                self.chunks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ChunkMeta>()),
            )
            .saturating_add(
                self.free
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
    }

    fn add_page(&mut self) -> Result<(), ForkArenaError> {
        let page_slots = self
            .slots_per_chunk
            .checked_mul(CHUNKS_PER_PAGE)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        let start = self.chunks.len();
        let end = start
            .checked_add(CHUNKS_PER_PAGE)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        u32::try_from(end).map_err(|_| ForkArenaError::CapacityOverflow)?;
        let slots = std::iter::repeat_with(|| None)
            .take(page_slots)
            .collect::<Box<[_]>>();
        self.pages.push(ChunkPage { slots });
        self.chunks.extend((start..end).map(|_| ChunkMeta {
            generation: 1,
            arena: 0,
            arena_position: UNINDEXED_ARENA_POSITION,
            used: 0,
            live: false,
            sealed: false,
            sequence_summary: None,
            previous_in_list: None,
        }));
        self.free.extend((start..end).rev().map(|slot| slot as u32));
        Ok(())
    }

    fn allocate(&mut self, arena: u32) -> Result<RawChunkKey, ForkArenaError> {
        if self.free.is_empty() {
            self.add_page()?;
        }
        let slot = self
            .free
            .pop()
            .expect("page publication supplied free chunks");
        let meta = &mut self.chunks[slot as usize];
        debug_assert!(!meta.live && meta.used == 0);
        meta.live = true;
        meta.sealed = false;
        meta.arena = arena;
        meta.arena_position = UNINDEXED_ARENA_POSITION;
        meta.previous_in_list = None;
        Ok(RawChunkKey {
            slot,
            generation: meta.generation,
        })
    }

    fn allocate_list_block(
        &mut self,
        arena: u32,
        previous_in_list: Option<(RawChunkKey, u32)>,
    ) -> Result<RawChunkKey, ForkArenaError> {
        let key = self.allocate(arena)?;
        self.validate_mut(key, arena)?.previous_in_list = previous_in_list;
        Ok(key)
    }

    fn validate(&self, key: RawChunkKey, arena: u32) -> Result<&ChunkMeta, ForkArenaError> {
        #[cfg(test)]
        self.validation_reads
            .set(self.validation_reads.get().saturating_add(1));
        let meta = self
            .chunks
            .get(key.slot as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if key.generation != meta.generation || !meta.live || meta.arena != arena {
            return Err(ForkArenaError::InvalidChunk);
        }
        Ok(meta)
    }

    fn validate_mut(
        &mut self,
        key: RawChunkKey,
        arena: u32,
    ) -> Result<&mut ChunkMeta, ForkArenaError> {
        let meta = self
            .chunks
            .get_mut(key.slot as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if key.generation != meta.generation || !meta.live || meta.arena != arena {
            return Err(ForkArenaError::InvalidChunk);
        }
        Ok(meta)
    }

    fn slot_index(
        &self,
        key: RawChunkKey,
        offset: usize,
    ) -> Result<(usize, usize), ForkArenaError> {
        if offset >= self.slots_per_chunk {
            return Err(ForkArenaError::InvalidRange);
        }
        let slot = key.slot as usize;
        let page = slot / CHUNKS_PER_PAGE;
        let chunk = slot % CHUNKS_PER_PAGE;
        let index = chunk
            .checked_mul(self.slots_per_chunk)
            .and_then(|base| base.checked_add(offset))
            .ok_or(ForkArenaError::CapacityOverflow)?;
        Ok((page, index))
    }

    fn append(
        &mut self,
        key: RawChunkKey,
        arena: u32,
        value: T,
        item_identity: Option<u64>,
    ) -> Result<u32, ForkArenaError> {
        let used = {
            let meta = self.validate(key, arena)?;
            if meta.sealed || meta.used as usize == self.slots_per_chunk {
                return Err(ForkArenaError::ChunkSealed);
            }
            if meta.used != 0 && meta.sequence_summary.is_some() != item_identity.is_some() {
                return Err(ForkArenaError::IdentityModeMismatch);
            }
            meta.used
        };
        let (page, index) = self.slot_index(key, used as usize)?;
        debug_assert!(self.pages[page].slots[index].is_none());
        self.pages[page].slots[index] = Some(value);
        let meta = self.validate_mut(key, arena)?;
        meta.used += 1;
        if let Some(item_identity) = item_identity {
            let summary = meta
                .sequence_summary
                .get_or_insert(SemanticSequenceIdentity::empty());
            summary.push_back(item_identity);
        }
        Ok(used)
    }

    fn get(&self, key: RawChunkKey, arena: u32, offset: u32) -> Option<&T> {
        let meta = self.validate(key, arena).ok()?;
        if offset >= meta.used {
            return None;
        }
        let (page, index) = self.slot_index(key, offset as usize).ok()?;
        self.pages[page].slots[index].as_ref()
    }

    fn get_mut(&mut self, key: RawChunkKey, arena: u32, offset: u32) -> Option<&mut T> {
        let meta = self.validate(key, arena).ok()?;
        if offset >= meta.used {
            return None;
        }
        let (page, index) = self.slot_index(key, offset as usize).ok()?;
        self.pages[page].slots[index].as_mut()
    }

    fn used(&self, key: RawChunkKey, arena: u32) -> Result<u32, ForkArenaError> {
        Ok(self.validate(key, arena)?.used)
    }

    fn sequence_summary(
        &self,
        key: RawChunkKey,
        arena: u32,
    ) -> Result<Option<SemanticSequenceIdentity>, ForkArenaError> {
        Ok(self.validate(key, arena)?.sequence_summary)
    }

    fn is_sealed(&self, key: RawChunkKey, arena: u32) -> Result<bool, ForkArenaError> {
        Ok(self.validate(key, arena)?.sealed)
    }

    fn seal(&mut self, key: RawChunkKey, arena: u32) -> Result<usize, ForkArenaError> {
        let capacity = self.slots_per_chunk;
        let meta = self.validate_mut(key, arena)?;
        meta.sealed = true;
        Ok(capacity.saturating_sub(meta.used as usize))
    }

    fn truncate(
        &mut self,
        key: RawChunkKey,
        arena: u32,
        used: u32,
        sequence_summary: Option<SemanticSequenceIdentity>,
    ) -> Result<(), ForkArenaError> {
        let old_used = self.validate(key, arena)?.used;
        if used > old_used {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        if sequence_summary.is_some_and(|summary| summary.len() != used as usize)
            || (used != 0
                && self.validate(key, arena)?.sequence_summary.is_some()
                    != sequence_summary.is_some())
        {
            return Err(ForkArenaError::IdentityModeMismatch);
        }
        if used == old_used {
            return Ok(());
        }
        for offset in used..old_used {
            let (page, index) = self.slot_index(key, offset as usize)?;
            drop(self.pages[page].slots[index].take());
        }
        let capacity = self.slots_per_chunk;
        let meta = self.validate_mut(key, arena)?;
        meta.used = used;
        meta.sequence_summary = sequence_summary;
        if used as usize != capacity {
            meta.sealed = false;
        }
        Ok(())
    }

    fn release(&mut self, key: RawChunkKey, arena: u32) -> Result<usize, ForkArenaError> {
        let used = self.validate(key, arena)?.used;
        for offset in 0..used {
            let (page, index) = self.slot_index(key, offset as usize)?;
            drop(self.pages[page].slots[index].take());
        }
        let meta = self.validate_mut(key, arena)?;
        meta.live = false;
        meta.sealed = false;
        meta.arena = 0;
        meta.arena_position = UNINDEXED_ARENA_POSITION;
        meta.used = 0;
        meta.sequence_summary = None;
        meta.previous_in_list = None;
        meta.generation = meta
            .generation
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        self.free.push(key.slot);
        Ok(used as usize)
    }

    fn index_in_arena(
        &mut self,
        key: RawChunkKey,
        arena: u32,
        position: usize,
    ) -> Result<(), ForkArenaError> {
        self.validate_mut(key, arena)?.arena_position = position;
        Ok(())
    }

    fn unindex_from_arena(&mut self, key: RawChunkKey, arena: u32) {
        if let Some(meta) = self.chunks.get_mut(key.slot as usize)
            && meta.live
            && meta.generation == key.generation
            && meta.arena == arena
        {
            meta.arena_position = UNINDEXED_ARENA_POSITION;
        }
    }

    fn arena_position(&self, key: RawChunkKey, arena: u32) -> Option<usize> {
        let meta = self.chunks.get(key.slot as usize)?;
        if !meta.live || meta.generation != key.generation || meta.arena != arena {
            return None;
        }
        let position = meta.arena_position;
        (position != UNINDEXED_ARENA_POSITION).then_some(position)
    }

    /// Reads topology already admitted through an immutable arena view.
    ///
    /// The opaque root and its owner-relative endpoint positions were checked
    /// before the view was constructed. The shared `&ChunkPool` borrow then
    /// excludes release, transfer, rollback, and incarnation reuse for the
    /// lifetime of the view, so ordinary traversal need not repeat those
    /// ownership checks at every block crossing.
    fn admitted_previous_position(&self, key: RawChunkKey) -> Option<(usize, u32)> {
        let meta = self.chunks.get(key.slot as usize)?;
        debug_assert!(meta.live && meta.generation == key.generation);
        let (previous_key, end) = meta.previous_in_list?;
        let previous = self.chunks.get(previous_key.slot as usize)?;
        debug_assert!(previous.live && previous.generation == previous_key.generation);
        (previous.arena_position != UNINDEXED_ARENA_POSITION)
            .then_some((previous.arena_position, end))
    }

    fn admitted_get(&self, key: RawChunkKey, offset: u32) -> Option<&T> {
        let meta = self.chunks.get(key.slot as usize)?;
        debug_assert!(meta.live && meta.generation == key.generation);
        if offset >= meta.used {
            return None;
        }
        let (page, index) = self.slot_index(key, offset as usize).ok()?;
        self.pages.get(page)?.slots.get(index)?.as_ref()
    }

    fn admitted_slice(&self, key: RawChunkKey, range: Range<u32>) -> Option<&[Option<T>]> {
        let meta = self.chunks.get(key.slot as usize)?;
        debug_assert!(meta.live && meta.generation == key.generation);
        if range.start > range.end || range.end > meta.used {
            return None;
        }
        let (start_page, start) = self.slot_index(key, range.start as usize).ok()?;
        let end = if range.is_empty() {
            start
        } else {
            let (end_page, end) = self.slot_index(key, (range.end - 1) as usize).ok()?;
            if end_page != start_page {
                return None;
            }
            end + 1
        };
        self.pages.get(start_page)?.slots.get(start..end)
    }

    fn previous_in_list(
        &self,
        key: RawChunkKey,
        arena: u32,
    ) -> Result<Option<(RawChunkKey, u32)>, ForkArenaError> {
        #[cfg(test)]
        self.previous_link_reads
            .set(self.previous_link_reads.get().saturating_add(1));
        Ok(self.validate(key, arena)?.previous_in_list)
    }

    fn transfer(
        &mut self,
        key: RawChunkKey,
        source: u32,
        destination: u32,
    ) -> Result<(), ForkArenaError> {
        let meta = self.validate_mut(key, source)?;
        if !meta.sealed {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        meta.arena = destination;
        Ok(())
    }
}

/// The single caller-owned physical allocation pool shared by typed arenas.
///
/// Holding `&ChunkPool` permits stable direct payload borrows. Every append,
/// seal, transfer, rollback, and prune requires `&mut ChunkPool`, making the
/// physical mutation gate explicit and exclusive.
pub struct ChunkPool<T> {
    owner: u32,
    payload: ChunkStorage<T>,
    /// Empty legacy lifecycle lane retained until checkpoint marks drop their
    /// always-zero descriptor coordinates. It stores no list topology and
    /// never allocates a page.
    descriptors: ChunkStorage<()>,
}

impl<T> Default for ChunkPool<T> {
    fn default() -> Self {
        Self::with_chunk_bytes(DEFAULT_CHUNK_BYTES)
    }
}

impl<T> ChunkPool<T> {
    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        Self {
            owner: NEXT_POOL_OWNER.fetch_add(1, Ordering::Relaxed),
            payload: ChunkStorage::with_chunk_bytes(chunk_bytes),
            descriptors: ChunkStorage::with_chunk_bytes(chunk_bytes),
        }
    }

    #[must_use]
    pub const fn chunk_byte_budget(&self) -> usize {
        self.payload.chunk_byte_budget()
    }

    #[must_use]
    pub const fn chunk_capacity(&self) -> usize {
        self.payload.chunk_capacity()
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.payload.page_count()
    }

    #[must_use]
    pub const fn logical_block_metadata_bytes(&self) -> usize {
        std::mem::size_of::<ChunkMeta>()
    }

    #[must_use]
    pub fn physical_page_payload_bytes(&self) -> usize {
        self.payload
            .slots_per_chunk
            .saturating_mul(CHUNKS_PER_PAGE)
            .saturating_mul(std::mem::size_of::<Option<T>>())
    }

    #[must_use]
    pub const fn physical_page_metadata_bytes(&self) -> usize {
        CHUNKS_PER_PAGE * std::mem::size_of::<ChunkMeta>()
    }

    /// Heap capacity retained by payload and descriptor pages plus their
    /// allocation metadata. Allocator-private bookkeeping is not observable.
    #[must_use]
    pub fn allocated_heap_bytes(&self) -> usize {
        self.payload
            .allocated_heap_bytes()
            .saturating_add(self.descriptors.allocated_heap_bytes())
    }
}

/// Direct cursor into one packed logical-list block.
pub struct ChunkCursor<Lane> {
    raw: RawChunkKey,
    offset: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<Lane> Clone for ChunkCursor<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Lane> Copy for ChunkCursor<Lane> {}

impl<Lane> core::fmt::Debug for ChunkCursor<Lane> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ChunkCursor")
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl<Lane> PartialEq for ChunkCursor<Lane> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw && self.offset == other.offset
    }
}

impl<Lane> Eq for ChunkCursor<Lane> {}

impl<Lane> Hash for ChunkCursor<Lane> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
        self.offset.hash(state);
    }
}

impl<Lane> ChunkCursor<Lane> {
    const EMPTY: Self = Self {
        raw: RawChunkKey {
            slot: 0,
            generation: 0,
        },
        offset: 0,
        _lane: PhantomData,
    };

    const fn new(raw: RawChunkKey, offset: u32) -> Self {
        Self {
            raw,
            offset,
            _lane: PhantomData,
        }
    }
}

/// Direct root of one generation-owned packed chunk chain.
///
/// The head offset is inclusive and the tail offset is exclusive. Every
/// nonempty root therefore reaches its last node without a descriptor lookup.
pub struct ArenaListId<Lane> {
    arena: u32,
    head: ChunkCursor<Lane>,
    tail: ChunkCursor<Lane>,
    len: u32,
}

/// Move-only authority to attach one whole unpublished list chain.
///
/// The coordinate is deliberately not exposed while this capability is live:
/// consuming it is the proof that the root block's predecessor is still
/// available for its one permitted write. Converting it to a shared root
/// publishes the chain and permanently gives up zero-copy concatenation.
pub struct UniqueArenaList<Lane> {
    root: ArenaListId<Lane>,
}

impl<Lane> UniqueArenaList<Lane> {
    #[must_use]
    pub(crate) const fn coordinate(&self) -> ArenaListId<Lane> {
        self.root
    }

    #[must_use]
    pub const fn publish(self) -> ArenaListId<Lane> {
        self.root
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.root.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.root.is_empty()
    }
}

impl<Lane> Clone for ArenaListId<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Lane> Copy for ArenaListId<Lane> {}
impl<Lane> core::fmt::Debug for ArenaListId<Lane> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArenaListId")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}
impl<Lane> PartialEq for ArenaListId<Lane> {
    fn eq(&self, other: &Self) -> bool {
        self.arena == other.arena
            && self.head == other.head
            && self.tail == other.tail
            && self.len == other.len
    }
}
impl<Lane> Eq for ArenaListId<Lane> {}

impl<Lane> Hash for ArenaListId<Lane> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.arena.hash(state);
        self.head.hash(state);
        self.tail.hash(state);
        self.len.hash(state);
    }
}

impl<Lane> ArenaListId<Lane> {
    #[must_use]
    pub(crate) const fn arena_identity(self) -> u32 {
        self.arena
    }

    /// Returns the owner-independent canonical empty-list coordinate.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            arena: 0,
            head: ChunkCursor::EMPTY,
            tail: ChunkCursor::EMPTY,
            len: 0,
        }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.len as usize
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub(crate) fn rebrand_arena(self, arena: u32) -> Self {
        rebrand_list(self, arena)
    }

    const fn from_root(
        arena: u32,
        head: ChunkCursor<Lane>,
        tail: ChunkCursor<Lane>,
        len: u32,
    ) -> Self {
        debug_assert!(arena != 0);
        debug_assert!(len != 0);
        debug_assert!(head.raw.generation != 0 && tail.raw.generation != 0);
        debug_assert!(
            head.raw.slot != tail.raw.slot
                || head.raw.generation != tail.raw.generation
                || head.offset < tail.offset
        );
        Self {
            arena,
            head,
            tail,
            len,
        }
    }
}

const _: () = assert!(core::mem::size_of::<ArenaListId<PageMaterialLane>>() <= 32);

#[derive(Default)]
struct ChunkSet {
    payload: Vec<RawChunkKey>,
    descriptors: Vec<RawChunkKey>,
}

enum ForkOwnership {
    Accepted(ChunkSet),
    Forked {
        prefix: ChunkSet,
        detached_prior: ChunkSet,
        current: ChunkSet,
    },
}

/// Partial local rollback point. It cannot be converted into a retained
/// checkpoint mark.
pub struct OperationMark<Lane> {
    arena: u32,
    payload_chunks: u32,
    payload_tail_used: u32,
    payload_tail_summary: Option<SemanticSequenceIdentity>,
    descriptor_chunks: u32,
    descriptor_tail_used: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<Lane> Clone for OperationMark<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Lane> Copy for OperationMark<Lane> {}

impl<Lane> core::fmt::Debug for OperationMark<Lane> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OperationMark")
            .field("arena", &self.arena)
            .field("payload_chunks", &self.payload_chunks)
            .field("payload_tail_used", &self.payload_tail_used)
            .field("descriptor_chunks", &self.descriptor_chunks)
            .field("descriptor_tail_used", &self.descriptor_tail_used)
            .finish()
    }
}

/// Consuming proof that every builder has retired and both tails are sealed.
pub struct SealedBoundary<Lane> {
    arena: u32,
    payload_chunks: u32,
    descriptor_chunks: u32,
    payload_tail: Option<RawChunkKey>,
    descriptor_tail: Option<RawChunkKey>,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

/// Opaque whole-chunk retained checkpoint coordinate.
pub struct CheckpointMark<Lane> {
    arena: u32,
    payload_chunks: u32,
    descriptor_chunks: u32,
    payload_tail: Option<RawChunkKey>,
    descriptor_tail: Option<RawChunkKey>,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<Lane> Clone for CheckpointMark<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Lane> Copy for CheckpointMark<Lane> {}
impl<Lane> PartialEq for CheckpointMark<Lane> {
    fn eq(&self, other: &Self) -> bool {
        self.arena == other.arena
            && self.payload_chunks == other.payload_chunks
            && self.descriptor_chunks == other.descriptor_chunks
            && self.payload_tail == other.payload_tail
            && self.descriptor_tail == other.descriptor_tail
    }
}
impl<Lane> Eq for CheckpointMark<Lane> {}
impl<Lane> core::fmt::Debug for CheckpointMark<Lane> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CheckpointMark")
            .field("payload_chunks", &self.payload_chunks)
            .field("descriptor_chunks", &self.descriptor_chunks)
            .finish_non_exhaustive()
    }
}

/// Exclusive whole-chunk suffix available for semantic-lane promotion.
pub struct SealedBatch<Lane> {
    arena: u32,
    serial: u64,
    payload_start: u32,
    payload_end: u32,
    descriptor_start: u32,
    descriptor_end: u32,
    lists: Vec<ArenaListId<Lane>>,
}

/// A prevalidated whole-chunk suffix temporarily loaned out of its source
/// arena. The chunk payload remains owned by the source arena until the loan
/// is either returned or committed into a destination arena.
#[allow(dead_code)] // Used by the implemented closure-transfer stage before its production carrier cutover.
pub(crate) struct DetachedBatch<Lane> {
    arena: u32,
    serial: u64,
    payload_start: u32,
    descriptor_start: u32,
    payload: Vec<RawChunkKey>,
    descriptors: Vec<RawChunkKey>,
    lists: Vec<ArenaListId<Lane>>,
    rebrand_values: u64,
}

/// Failed detached-suffix settlement returns the move-only loan unchanged.
#[allow(dead_code)] // Used by the implemented closure-transfer stage before its production carrier cutover.
pub(crate) struct DetachedBatchTransferError<Lane> {
    pub(crate) error: ForkArenaError,
    pub(crate) batch: DetachedBatch<Lane>,
}

/// Failed transfer which returns the still-exclusive sealed suffix token.
pub(crate) struct BatchTransferError<Lane> {
    pub(crate) error: ForkArenaError,
    #[cfg_attr(not(test), allow(dead_code))]
    // Test-only recovery proves failed batches remain move-only.
    pub(crate) batch: SealedBatch<Lane>,
}

impl<Lane> core::fmt::Debug for BatchTransferError<Lane> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BatchTransferError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// Payload hook used by coarse-region transfers.
///
/// A transfer validates every embedded coordinate before changing either
/// arena, then rewrites those coordinates in place while the fixed chunk
/// addresses remain stable. Implementations must visit every coordinate
/// stored by the payload value.
pub(crate) trait RegionValue<Lane> {
    fn visit_region_lists(&self, visit: &mut dyn FnMut(ArenaListId<Lane>));
    fn rebrand_region_lists(&mut self, destination_arena: u32);
}

pub struct BatchMark<Lane> {
    arena: u32,
    payload_start: u32,
    descriptor_start: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<Lane> Clone for BatchMark<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Lane> Copy for BatchMark<Lane> {}

/// Lifecycle work counters; payload copy is absent by construction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ForkArenaCounters {
    pub new_semantic_nodes: u64,
    pub source_nodes_copied: u64,
    pub partial_edge_nodes_copied: u64,
    pub overlapping_nodes_copied: u64,
    pub direct_blocks_allocated: u64,
    pub identity_nodes_hashed: u64,
    pub identity_summaries_combined: u64,
    pub chunks_sealed: u64,
    pub unused_sealed_bytes: u64,
    pub chunks_promoted: u64,
    pub candidate_chunks_truncated: u64,
    pub accepted_chunks_reattached: u64,
    pub obsolete_chunks_pruned: u64,
}

/// Exact work performed while recovering an identity from stored summaries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SequenceSummaryWork {
    pub hashed_values: u64,
    pub combined_summaries: u64,
}

/// Storage/lifecycle failures detected before mutating ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForkArenaError {
    ActiveBatch,
    ActiveBuilder,
    AlreadyForked,
    CapacityOverflow,
    ChunkSealed,
    ForeignArena,
    InvalidCheckpoint,
    InvalidChunk,
    IdentityModeMismatch,
    InvalidOperationMark,
    InvalidRange,
    InvalidRegion,
    NotForked,
    UnsealedBoundary,
    InvalidActiveListBuilder,
}

/// Move-only persistent list construction state.
///
/// The builder contains only checked coordinates and scalar tail state. It
/// never borrows or points at the arena or pool; every operation must present
/// both owners explicitly and returns before either exclusive borrow ends.
#[must_use = "an active-list builder must be finalized or explicitly rolled back"]
pub struct ActiveListBuilder<T, Lane> {
    state: ActiveListBuilderState<Lane>,
    _payload: PhantomData<fn(T) -> T>,
}

enum ActiveListBuilderState<Lane> {
    Vacant,
    Open(OpenActiveList<Lane>),
    Sealed(UniqueArenaList<Lane>),
}

struct OpenActiveList<Lane> {
    arena: u32,
    operation: OperationMark<Lane>,
    root: ArenaListId<Lane>,
}

impl<T, Lane> Default for ActiveListBuilder<T, Lane> {
    fn default() -> Self {
        Self::vacant()
    }
}

impl<T, Lane> ActiveListBuilder<T, Lane> {
    pub const fn vacant() -> Self {
        Self {
            state: ActiveListBuilderState::Vacant,
            _payload: PhantomData,
        }
    }

    #[must_use]
    pub const fn is_vacant(&self) -> bool {
        matches!(self.state, ActiveListBuilderState::Vacant)
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.state, ActiveListBuilderState::Open(_))
    }

    #[must_use]
    pub const fn is_sealed(&self) -> bool {
        matches!(self.state, ActiveListBuilderState::Sealed(_))
    }

    /// Takes the sealed coordinate and returns the builder to its vacant
    /// state without touching arena storage.
    pub fn take_sealed(&mut self) -> Result<ArenaListId<Lane>, ForkArenaError> {
        Ok(self.take_unique_sealed()?.publish())
    }

    /// Takes the sealed move capability without publishing a copyable root.
    pub fn take_unique_sealed(&mut self) -> Result<UniqueArenaList<Lane>, ForkArenaError> {
        let state = core::mem::replace(&mut self.state, ActiveListBuilderState::Vacant);
        let ActiveListBuilderState::Sealed(list) = state else {
            self.state = state;
            return Err(ForkArenaError::InvalidActiveListBuilder);
        };
        Ok(list)
    }
}

/// One typed arena lane containing coordinates and lifecycle metadata only.
pub struct ForkArena<T, Lane> {
    owner: u32,
    pool_owner: Option<u32>,
    ownership: ForkOwnership,
    base_payload_chunks: u32,
    base_descriptor_chunks: u32,
    active_builder: bool,
    pending_batch: Option<PendingBatch>,
    next_batch_serial: u64,
    counters: ForkArenaCounters,
    _types: PhantomData<(T, Lane)>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct PendingBatch {
    serial: u64,
    payload_start: u32,
    payload_end: u32,
    descriptor_start: u32,
    descriptor_end: u32,
}

impl<T, Lane> Default for ForkArena<T, Lane> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, Lane> ForkArena<T, Lane> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            owner: NEXT_ARENA_OWNER.fetch_add(1, Ordering::Relaxed),
            pool_owner: None,
            ownership: ForkOwnership::Accepted(ChunkSet::default()),
            base_payload_chunks: 0,
            base_descriptor_chunks: 0,
            active_builder: false,
            pending_batch: None,
            next_batch_serial: 1,
            counters: ForkArenaCounters::default(),
            _types: PhantomData,
        }
    }

    /// Creates another empty typed lane for use with the caller's pool.
    #[must_use]
    pub fn empty_lane<Destination>(&self) -> ForkArena<T, Destination> {
        ForkArena {
            owner: NEXT_ARENA_OWNER.fetch_add(1, Ordering::Relaxed),
            pool_owner: None,
            ownership: ForkOwnership::Accepted(ChunkSet::default()),
            base_payload_chunks: 0,
            base_descriptor_chunks: 0,
            active_builder: false,
            pending_batch: None,
            next_batch_serial: 1,
            counters: ForkArenaCounters::default(),
            _types: PhantomData,
        }
    }

    #[must_use]
    pub const fn counters(&self) -> ForkArenaCounters {
        self.counters
    }

    #[must_use]
    pub(crate) const fn region_identity(&self) -> u32 {
        self.owner
    }

    pub(crate) fn record_source_nodes_copied(&mut self, count: usize) {
        self.counters.source_nodes_copied = self
            .counters
            .source_nodes_copied
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    }

    fn record_partial_edge_nodes_copied(&mut self, count: usize) {
        self.counters.partial_edge_nodes_copied = self
            .counters
            .partial_edge_nodes_copied
            .saturating_add(count as u64);
    }

    pub(crate) fn record_identity_work(&mut self, work: SequenceSummaryWork) {
        self.counters.identity_nodes_hashed = self
            .counters
            .identity_nodes_hashed
            .saturating_add(work.hashed_values);
        self.counters.identity_summaries_combined = self
            .counters
            .identity_summaries_combined
            .saturating_add(work.combined_summaries);
    }

    fn bind_pool(&mut self, pool: &ChunkPool<T>) -> Result<(), ForkArenaError> {
        match self.pool_owner {
            Some(owner) if owner != pool.owner => Err(ForkArenaError::InvalidChunk),
            Some(_) => Ok(()),
            None => {
                self.pool_owner = Some(pool.owner);
                Ok(())
            }
        }
    }

    fn validate_pool(&self, pool: &ChunkPool<T>) -> Result<(), ForkArenaError> {
        if self.pool_owner.is_none_or(|owner| owner == pool.owner) {
            Ok(())
        } else {
            Err(ForkArenaError::InvalidChunk)
        }
    }

    fn validate_live_chunks(&self, pool: &ChunkPool<T>) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        for position in self.base_payload_chunks as usize..self.live_payload_len() {
            let key = self
                .live_key_at(false, position)
                .ok_or(ForkArenaError::InvalidChunk)?;
            if pool.payload.validate(key, self.owner)?.arena_position != position {
                return Err(ForkArenaError::InvalidChunk);
            }
        }
        for position in self.base_descriptor_chunks as usize..self.live_descriptor_len() {
            let key = self
                .live_key_at(true, position)
                .ok_or(ForkArenaError::InvalidChunk)?;
            if pool.descriptors.validate(key, self.owner)?.arena_position != position {
                return Err(ForkArenaError::InvalidChunk);
            }
        }
        Ok(())
    }

    fn can_seal_boundary(&self, pool: &ChunkPool<T>) -> Result<(), ForkArenaError> {
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        u32::try_from(self.live_payload_len()).map_err(|_| ForkArenaError::CapacityOverflow)?;
        u32::try_from(self.live_descriptor_len()).map_err(|_| ForkArenaError::CapacityOverflow)?;
        self.validate_live_chunks(pool)
    }

    pub(crate) fn can_retire_region(&self, pool: &ChunkPool<T>) -> Result<(), ForkArenaError> {
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        self.validate_live_chunks(pool)
    }

    /// Releases every envelope after a complete read-only preflight.
    pub(crate) fn retire_region(&mut self, pool: &mut ChunkPool<T>) -> Result<(), ForkArenaError> {
        self.can_retire_region(pool)?;
        let ownership = std::mem::replace(
            &mut self.ownership,
            ForkOwnership::Accepted(ChunkSet::default()),
        );
        let sets = match ownership {
            ForkOwnership::Accepted(accepted) => vec![accepted],
            ForkOwnership::Forked {
                prefix,
                detached_prior,
                current,
            } => vec![prefix, detached_prior, current],
        };
        for set in sets {
            self.release_set(pool, set)
                .expect("region retirement was completely preflighted");
        }
        Ok(())
    }

    #[must_use]
    pub fn payload_chunk_capacity(&self, pool: &ChunkPool<T>) -> usize {
        pool.payload.chunk_capacity()
    }

    #[cfg(test)]
    pub(crate) fn live_payload_values(&self, pool: &ChunkPool<T>) -> usize {
        (0..self.live_payload_len())
            .map(|index| {
                self.live_key_at(false, index)
                    .and_then(|key| pool.payload.used(key, self.owner).ok())
                    .unwrap_or(0) as usize
            })
            .sum()
    }

    fn live_payload_len(&self) -> usize {
        self.base_payload_chunks as usize
            + match &self.ownership {
                ForkOwnership::Accepted(chunks) => chunks.payload.len(),
                ForkOwnership::Forked {
                    prefix, current, ..
                } => prefix.payload.len() + current.payload.len(),
            }
    }

    fn live_descriptor_len(&self) -> usize {
        self.base_descriptor_chunks as usize
            + match &self.ownership {
                ForkOwnership::Accepted(chunks) => chunks.descriptors.len(),
                ForkOwnership::Forked {
                    prefix, current, ..
                } => prefix.descriptors.len() + current.descriptors.len(),
            }
    }

    fn current_chunks_mut(&mut self) -> &mut ChunkSet {
        match &mut self.ownership {
            ForkOwnership::Accepted(chunks) => chunks,
            ForkOwnership::Forked { current, .. } => current,
        }
    }

    fn live_key_at(&self, descriptor: bool, index: usize) -> Option<RawChunkKey> {
        let base = if descriptor {
            self.base_descriptor_chunks as usize
        } else {
            self.base_payload_chunks as usize
        };
        let index = index.checked_sub(base)?;
        match &self.ownership {
            ForkOwnership::Accepted(chunks) => {
                if descriptor {
                    chunks.descriptors.get(index).copied()
                } else {
                    chunks.payload.get(index).copied()
                }
            }
            ForkOwnership::Forked {
                prefix, current, ..
            } => {
                let prefix_len = if descriptor {
                    prefix.descriptors.len()
                } else {
                    prefix.payload.len()
                };
                if index < prefix_len {
                    if descriptor {
                        prefix.descriptors.get(index).copied()
                    } else {
                        prefix.payload.get(index).copied()
                    }
                } else {
                    let index = index - prefix_len;
                    if descriptor {
                        current.descriptors.get(index).copied()
                    } else {
                        current.payload.get(index).copied()
                    }
                }
            }
        }
    }

    fn index_chunk(
        &self,
        pool: &mut ChunkPool<T>,
        descriptor: bool,
        key: RawChunkKey,
        position: usize,
    ) {
        let result = if descriptor {
            pool.descriptors.index_in_arena(key, self.owner, position)
        } else {
            pool.payload.index_in_arena(key, self.owner, position)
        };
        result.expect("arena index names its live owned chunk");
    }

    fn unindex_chunk(&self, pool: &mut ChunkPool<T>, descriptor: bool, key: RawChunkKey) {
        if descriptor {
            pool.descriptors.unindex_from_arena(key, self.owner);
        } else {
            pool.payload.unindex_from_arena(key, self.owner);
        }
    }

    fn resolved_position(
        &self,
        pool: &ChunkPool<T>,
        descriptor: bool,
        key: RawChunkKey,
    ) -> Option<usize> {
        if descriptor {
            pool.descriptors.arena_position(key, self.owner)
        } else {
            pool.payload.arena_position(key, self.owner)
        }
    }

    fn append_payload(
        &mut self,
        pool: &mut ChunkPool<T>,
        root: &mut ArenaListId<Lane>,
        value: T,
        item_identity: Option<u64>,
    ) -> Result<(), ForkArenaError> {
        if !root.is_empty() && root.arena != self.owner {
            return Err(ForkArenaError::ForeignArena);
        }
        let current_tail = self.current_chunks_mut().payload.last().copied();
        let reusable = (!root.is_empty())
            .then_some(root.tail.raw)
            .filter(|key| Some(*key) == current_tail)
            .filter(|key| {
                !pool.payload.is_sealed(*key, self.owner).unwrap_or(true)
                    && pool
                        .payload
                        .used(*key, self.owner)
                        .ok()
                        .is_some_and(|used| {
                            used == root.tail.offset
                                && used as usize != pool.payload.chunk_capacity()
                        })
            });
        let key = match reusable {
            Some(key) => key,
            None => {
                self.bind_pool(pool)?;
                let previous = (!root.is_empty()).then_some((root.tail.raw, root.tail.offset));
                let key = pool.payload.allocate_list_block(self.owner, previous)?;
                self.current_chunks_mut().payload.push(key);
                self.counters.direct_blocks_allocated =
                    self.counters.direct_blocks_allocated.saturating_add(1);
                let position = self.live_payload_len() - 1;
                self.index_chunk(pool, false, key, position);
                key
            }
        };
        let offset = pool.payload.append(key, self.owner, value, item_identity)?;
        if root.is_empty() {
            *root = ArenaListId::from_root(
                self.owner,
                ChunkCursor::new(key, offset),
                ChunkCursor::new(key, offset + 1),
                1,
            );
        } else {
            root.tail = ChunkCursor::new(key, offset + 1);
            root.len = root
                .len
                .checked_add(1)
                .ok_or(ForkArenaError::CapacityOverflow)?;
        }
        let became_full =
            pool.payload.used(key, self.owner)? as usize == pool.payload.chunk_capacity();
        if became_full {
            pool.payload.seal(key, self.owner)?;
            self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        }
        self.counters.new_semantic_nodes += 1;
        Ok(())
    }

    /// Clones one admitted source list directly into this arena's packed
    /// destination chunks while rewriting each newly owned value in place.
    ///
    /// The source and destination share the caller-owned pool, so a borrowed
    /// source view cannot remain live across destination mutation. This seam
    /// instead admits each source cell by its stable chunk coordinate, clones
    /// exactly that value, applies the caller's coordinate rewrite, and
    /// immediately appends it to the final list. No whole-list `Vec<T>` or
    /// second payload representation exists between the two arenas.
    pub(crate) fn clone_mapped_list_from(
        &mut self,
        pool: &mut ChunkPool<T>,
        source: &ForkArena<T, Lane>,
        list: ArenaListId<Lane>,
        mut rewrite: impl FnMut(&mut T) -> Result<Option<u64>, ForkArenaError>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError>
    where
        T: Clone,
    {
        source.validate_list(pool, list)?;
        self.bind_pool(pool)?;
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let operation = self.operation_mark(pool);
        self.active_builder = true;
        let mut root = ArenaListId::empty();
        let mut identity_mode = None;
        let copied = if list.is_empty() {
            Ok(())
        } else {
            self.clone_chunk_prefix_from(
                pool,
                source,
                list,
                list.tail.raw,
                list.tail.offset,
                &mut root,
                &mut identity_mode,
                &mut rewrite,
            )
        };
        let copied = copied.and_then(|()| {
            self.seal_direct_tail(pool, root)?;
            self.validate_list(pool, root)
        });
        self.active_builder = false;
        if let Err(error) = copied {
            self.restore_operation(pool, operation)
                .expect("direct mapped clone rollback mark belongs to its arena");
            return Err(error);
        }
        Ok(root)
    }

    #[allow(clippy::too_many_arguments)]
    fn clone_chunk_prefix_from(
        &mut self,
        pool: &mut ChunkPool<T>,
        source: &ForkArena<T, Lane>,
        list: ArenaListId<Lane>,
        key: RawChunkKey,
        end: u32,
        root: &mut ArenaListId<Lane>,
        identity_mode: &mut Option<bool>,
        rewrite: &mut impl FnMut(&mut T) -> Result<Option<u64>, ForkArenaError>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone,
    {
        let start = if key == list.head.raw {
            list.head.offset
        } else {
            let previous = pool
                .payload
                .previous_in_list(key, source.owner)?
                .ok_or(ForkArenaError::InvalidRange)?;
            self.clone_chunk_prefix_from(
                pool,
                source,
                list,
                previous.0,
                previous.1,
                root,
                identity_mode,
                rewrite,
            )?;
            0
        };
        for offset in start..end {
            let mut value = pool
                .payload
                .get(key, source.owner, offset)
                .ok_or(ForkArenaError::InvalidRange)?
                .clone();
            let item_identity = rewrite(&mut value)?;
            match *identity_mode {
                Some(enabled) if enabled != item_identity.is_some() => {
                    return Err(ForkArenaError::IdentityModeMismatch);
                }
                None => *identity_mode = Some(item_identity.is_some()),
                _ => {}
            }
            self.append_payload(pool, root, value, item_identity)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn operation_mark(&self, pool: &ChunkPool<T>) -> OperationMark<Lane> {
        let payload_tail_used = self
            .live_key_at(false, self.live_payload_len().saturating_sub(1))
            .and_then(|key| pool.payload.used(key, self.owner).ok())
            .unwrap_or(0);
        let payload_tail_summary = self
            .live_key_at(false, self.live_payload_len().saturating_sub(1))
            .and_then(|key| pool.payload.sequence_summary(key, self.owner).ok())
            .flatten();
        let descriptor_tail_used = self
            .live_key_at(true, self.live_descriptor_len().saturating_sub(1))
            .and_then(|key| pool.descriptors.used(key, self.owner).ok())
            .unwrap_or(0);
        OperationMark {
            arena: self.owner,
            payload_chunks: self.live_payload_len() as u32,
            payload_tail_used,
            payload_tail_summary,
            descriptor_chunks: self.live_descriptor_len() as u32,
            descriptor_tail_used,
            _lane: PhantomData,
        }
    }

    pub fn restore_operation(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: OperationMark<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.bind_pool(pool)?;
        if self.active_builder || self.pending_batch.is_some() || mark.arena != self.owner {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        self.truncate_lane(
            pool,
            false,
            mark.payload_chunks as usize,
            mark.payload_tail_used,
            mark.payload_tail_summary,
        )?;
        self.truncate_lane(
            pool,
            true,
            mark.descriptor_chunks as usize,
            mark.descriptor_tail_used,
            None,
        )?;
        Ok(())
    }

    fn truncate_lane(
        &mut self,
        pool: &mut ChunkPool<T>,
        descriptor: bool,
        chunks: usize,
        tail_used: u32,
        tail_summary: Option<SemanticSequenceIdentity>,
    ) -> Result<(), ForkArenaError> {
        let live_len = if descriptor {
            self.live_descriptor_len()
        } else {
            self.live_payload_len()
        };
        let base = if descriptor {
            self.base_descriptor_chunks as usize
        } else {
            self.base_payload_chunks as usize
        };
        if chunks < base || chunks > live_len {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        while if descriptor {
            self.live_descriptor_len()
        } else {
            self.live_payload_len()
        } > chunks
        {
            let key = {
                let current = self.current_chunks_mut();
                let lane = if descriptor {
                    &mut current.descriptors
                } else {
                    &mut current.payload
                };
                lane.pop().ok_or(ForkArenaError::InvalidOperationMark)?
            };
            self.unindex_chunk(pool, descriptor, key);
            if descriptor {
                pool.descriptors.release(key, self.owner)?;
            } else {
                pool.payload.release(key, self.owner)?;
            }
            self.counters.candidate_chunks_truncated += 1;
        }
        if chunks != base {
            let key = self
                .live_key_at(descriptor, chunks - 1)
                .ok_or(ForkArenaError::InvalidOperationMark)?;
            if descriptor {
                pool.descriptors
                    .truncate(key, self.owner, tail_used, None)?;
            } else {
                pool.payload
                    .truncate(key, self.owner, tail_used, tail_summary)?;
            }
        } else if tail_used != 0 {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        Ok(())
    }

    pub fn begin_builder<'a>(
        &'a mut self,
        pool: &'a mut ChunkPool<T>,
    ) -> Result<ForkArenaBuilder<'a, T, Lane>, ForkArenaError> {
        self.bind_pool(pool)?;
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let operation = self.operation_mark(pool);
        self.active_builder = true;
        Ok(ForkArenaBuilder {
            arena: self,
            pool,
            operation,
            root: ArenaListId::empty(),
            sequence_summary: None,
            finished: false,
        })
    }

    /// Opens a persistent coordinate-only builder. The builder may outlive
    /// this call, but it owns no borrow; the lane remains exclusively marked
    /// active until the builder is finalized or rolled back.
    pub fn open_active_list(
        &mut self,
        pool: &ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
    ) -> Result<(), ForkArenaError> {
        self.bind_pool(pool)?;
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        if !builder.is_vacant() {
            return Err(ForkArenaError::InvalidActiveListBuilder);
        }
        let operation = self.operation_mark(pool);
        self.active_builder = true;
        builder.state = ActiveListBuilderState::Open(OpenActiveList {
            arena: self.owner,
            operation,
            root: ArenaListId::empty(),
        });
        Ok(())
    }

    fn active_list_open_mut<'a>(
        &self,
        builder: &'a mut ActiveListBuilder<T, Lane>,
    ) -> Result<&'a mut OpenActiveList<Lane>, ForkArenaError> {
        let ActiveListBuilderState::Open(open) = &mut builder.state else {
            return Err(ForkArenaError::InvalidActiveListBuilder);
        };
        if !self.active_builder || open.arena != self.owner {
            return Err(ForkArenaError::InvalidActiveListBuilder);
        }
        Ok(open)
    }

    /// Appends one newly created semantic payload to an open active list.
    pub fn push_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        value: T,
    ) -> Result<(), ForkArenaError> {
        self.push_active_list_with_identity(pool, builder, value, None)
    }

    pub(crate) fn push_active_list_summarized(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        value: T,
        item_identity: u64,
    ) -> Result<(), ForkArenaError> {
        self.push_active_list_with_identity(pool, builder, value, Some(item_identity))
    }

    fn push_active_list_with_identity(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        value: T,
        item_identity: Option<u64>,
    ) -> Result<(), ForkArenaError> {
        let mut root = self.active_list_open_mut(builder)?.root;
        self.append_payload(pool, &mut root, value, item_identity)?;
        self.active_list_open_mut(builder)?.root = root;
        Ok(())
    }

    pub(crate) fn append_validated_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone,
    {
        self.active_list_open_mut(builder)?;
        self.validate_list(pool, list)?;
        let root = self.active_list_open_mut(builder)?.root;
        let root = self.copy_shared_then_splice(pool, root, list)?;
        self.active_list_open_mut(builder)?.root = root;
        Ok(())
    }

    /// Consumes one unpublished whole-list chain into the active suffix.
    ///
    /// Unlike [`Self::append_active_list`], this is an O(1) topology operation
    /// and performs no payload copies.
    pub fn append_unique_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        list: UniqueArenaList<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.active_list_open_mut(builder)?;
        let root = self.active_list_open_mut(builder)?.root;
        let root = self.splice_unique_direct_root(pool, root, list)?;
        self.active_list_open_mut(builder)?.root = root;
        Ok(())
    }

    /// Consumes a unique whole right chain after an already admitted shared
    /// left root. The returned admission follows directly from the two input
    /// proofs, so the ordinary append path performs no chain census.
    pub(crate) fn append_unique_to_validated_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        left: ArenaListId<Lane>,
        right: UniqueArenaList<Lane>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        self.validate_list(pool, left)?;
        self.splice_unique_direct_root(pool, left, right)
    }

    /// Reclaims move authority from a semantically consumed admitted root.
    ///
    /// The caller must have removed its owning carrier before calling this
    /// seam. The metadata check prevents reclaiming a root that has already
    /// donated its head predecessor to an earlier composition.
    pub(crate) fn reclaim_unlinked_validated_list(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<UniqueArenaList<Lane>, ForkArenaError> {
        self.validate_list(pool, list)?;
        if !list.is_empty()
            && (list.head.offset != 0
                || pool
                    .payload
                    .previous_in_list(list.head.raw, self.owner)?
                    .is_some())
        {
            return Err(ForkArenaError::InvalidRange);
        }
        Ok(UniqueArenaList { root: list })
    }

    /// Copies an existing immutable list into the active private suffix.
    ///
    /// Shared roots cannot donate their write-once head predecessor. Callers
    /// that own a whole chain should use [`Self::append_unique_active_list`].
    pub fn append_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone,
    {
        self.append_validated_active_list(pool, builder, list)
    }

    /// Copies one logical subrange into the active private suffix.
    pub fn append_active_list_range(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone,
    {
        self.append_validated_active_list_range(pool, builder, list, selected)
    }

    pub(crate) fn append_validated_active_list_range(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone,
    {
        self.active_list_open_mut(builder)?;
        self.validate_list(pool, list)?;
        if selected.start > selected.end || selected.end > list.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        if selected.is_empty() {
            return Ok(());
        }
        let selected_root = self.slice_direct_root(pool, list, selected)?;
        if selected_root.len() != list.len() {
            self.record_partial_edge_nodes_copied(selected_root.len());
        }
        let root = self.active_list_open_mut(builder)?.root;
        let root = self.copy_shared_then_splice(pool, root, selected_root)?;
        self.active_list_open_mut(builder)?.root = root;
        Ok(())
    }

    pub(crate) fn append_validated_active_list_range_summarized(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
        mut item_identity: impl FnMut(&T) -> u64,
    ) -> Result<(SemanticSequenceIdentity, SequenceSummaryWork), ForkArenaError>
    where
        T: Clone,
    {
        self.active_list_open_mut(builder)?;
        self.validate_list(pool, list)?;
        if selected.start > selected.end || selected.end > list.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        if selected.is_empty() {
            return Ok((
                SemanticSequenceIdentity::empty(),
                SequenceSummaryWork::default(),
            ));
        }
        let selected_root = self.slice_direct_root(pool, list, selected)?;
        if selected_root.len() != list.len() {
            self.record_partial_edge_nodes_copied(selected_root.len());
        }
        let root = self.active_list_open_mut(builder)?.root;
        let (root, summary, work) =
            self.copy_shared_then_splice_summarized(pool, root, selected_root, &mut item_identity)?;
        self.active_list_open_mut(builder)?.root = root;
        Ok((summary, work))
    }

    /// Finalizes the active list's direct packed-chunk root.
    pub fn finalize_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
    ) -> Result<(), ForkArenaError> {
        let list = self.active_list_open_mut(builder)?.root;
        self.seal_direct_tail(pool, list)?;
        self.validate_list(pool, list)?;
        self.active_builder = false;
        builder.state = ActiveListBuilderState::Sealed(UniqueArenaList { root: list });
        Ok(())
    }

    /// Rolls an open active list back to its partial operation mark and
    /// returns the builder to its vacant state.
    pub fn rollback_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
    ) -> Result<(), ForkArenaError> {
        let operation = self.active_list_open_mut(builder)?.operation;
        self.active_builder = false;
        builder.state = ActiveListBuilderState::Vacant;
        self.restore_operation(pool, operation)
    }

    /// Finalizes and splits an active list without reading payload values.
    pub fn finalize_and_split_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        split: usize,
        scratch: &mut Vec<()>,
    ) -> Result<(ArenaListId<Lane>, ArenaListId<Lane>), ForkArenaError> {
        self.finalize_active_list(pool, builder)?;
        let list = builder.take_sealed()?;
        if split > list.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        let left = self.slice_list(pool, list, 0..split, scratch)?;
        let right = self.slice_list(pool, list, split..list.len(), scratch)?;
        Ok((left, right))
    }

    fn cursor_at_node(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        index: usize,
    ) -> Result<ChunkCursor<Lane>, ForkArenaError> {
        if index >= list.len() || list.is_empty() {
            return Err(ForkArenaError::InvalidRange);
        }
        let mut key = list.tail.raw;
        let mut end = list.tail.offset;
        let mut remaining = list.len() - index;
        loop {
            let start = if key == list.head.raw {
                list.head.offset
            } else {
                0
            };
            if end < start {
                return Err(ForkArenaError::InvalidRange);
            }
            let available = (end - start) as usize;
            if remaining <= available {
                return Ok(ChunkCursor::new(key, end - remaining as u32));
            }
            remaining -= available;
            let previous = pool
                .payload
                .previous_in_list(key, self.owner)?
                .ok_or(ForkArenaError::InvalidRange)?;
            key = previous.0;
            end = previous.1;
        }
    }

    fn slice_direct_root(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        self.validate_list(pool, list)?;
        if selected.start > selected.end || selected.end > list.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        if selected.is_empty() {
            return Ok(ArenaListId::empty());
        }
        let head = self.cursor_at_node(pool, list, selected.start)?;
        let tail = if selected.end == list.len() {
            list.tail
        } else {
            let cursor = self.cursor_at_node(pool, list, selected.end)?;
            if cursor.offset == 0 {
                let previous = pool
                    .payload
                    .previous_in_list(cursor.raw, self.owner)?
                    .ok_or(ForkArenaError::InvalidRange)?;
                ChunkCursor::new(previous.0, previous.1)
            } else {
                cursor
            }
        };
        Ok(ArenaListId::from_root(
            self.owner,
            head,
            tail,
            u32::try_from(selected.len()).map_err(|_| ForkArenaError::CapacityOverflow)?,
        ))
    }

    fn summarize_direct_root(
        &self,
        pool: &ChunkPool<T>,
        root: ArenaListId<Lane>,
        item_identity: &mut impl FnMut(&T) -> u64,
    ) -> Result<(SemanticSequenceIdentity, SequenceSummaryWork), ForkArenaError> {
        self.validate_list(pool, root)?;
        if root.is_empty() {
            return Ok((
                SemanticSequenceIdentity::empty(),
                SequenceSummaryWork::default(),
            ));
        }
        let mut key = root.tail.raw;
        let mut end = root.tail.offset;
        let mut summary = SemanticSequenceIdentity::empty();
        let mut work = SequenceSummaryWork::default();
        loop {
            let used = pool.payload.used(key, self.owner)?;
            let start = if key == root.head.raw {
                root.head.offset
            } else {
                0
            };
            let part = if start == 0 && end == used {
                if let Some(part) = pool.payload.sequence_summary(key, self.owner)? {
                    work.combined_summaries = work.combined_summaries.saturating_add(1);
                    part
                } else {
                    let mut part = SemanticSequenceIdentity::empty();
                    for offset in start..end {
                        let value = pool
                            .payload
                            .get(key, self.owner, offset)
                            .ok_or(ForkArenaError::InvalidRange)?;
                        part.push_back(item_identity(value));
                        work.hashed_values = work.hashed_values.saturating_add(1);
                    }
                    part
                }
            } else {
                let mut part = SemanticSequenceIdentity::empty();
                for offset in start..end {
                    let value = pool
                        .payload
                        .get(key, self.owner, offset)
                        .ok_or(ForkArenaError::InvalidRange)?;
                    part.push_back(item_identity(value));
                    work.hashed_values = work.hashed_values.saturating_add(1);
                }
                part
            };
            summary = part.concat(summary);
            if key == root.head.raw {
                break;
            }
            let previous = pool
                .payload
                .previous_in_list(key, self.owner)?
                .ok_or(ForkArenaError::InvalidRange)?;
            key = previous.0;
            end = previous.1;
        }
        Ok((summary, work))
    }

    /// Splices a whole uniquely owned right chain in O(1).
    ///
    /// `right` is consumed because its head predecessor is write-once. Shared
    /// coordinates and slices must use the separately named copy path below.
    fn splice_unique_direct_root(
        &mut self,
        pool: &mut ChunkPool<T>,
        left: ArenaListId<Lane>,
        right: UniqueArenaList<Lane>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        let right = right.root;
        if (!left.is_empty() && left.arena != self.owner)
            || (!right.is_empty() && right.arena != self.owner)
        {
            return Err(ForkArenaError::ForeignArena);
        }
        if left.is_empty() {
            return Ok(right);
        }
        if right.is_empty() {
            return Ok(left);
        }
        // Once another whole block follows it, the prior private tail can no
        // longer accept payload. Sealing here also guarantees that every
        // internal block of a published direct chain is immutable.
        self.seal_direct_tail(pool, left)?;
        if right.head.offset != 0
            || pool
                .payload
                .previous_in_list(right.head.raw, self.owner)?
                .is_some()
        {
            return Err(ForkArenaError::InvalidRange);
        }
        pool.payload
            .validate_mut(right.head.raw, self.owner)?
            .previous_in_list = Some((left.tail.raw, left.tail.offset));
        let len = left
            .len
            .checked_add(right.len)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        let root = ArenaListId::from_root(self.owner, left.head, right.tail, len);
        Ok(root)
    }

    /// Copies an explicitly shared coordinate into fresh unique blocks, then
    /// consumes those blocks in one direct splice.
    fn copy_shared_then_splice(
        &mut self,
        pool: &mut ChunkPool<T>,
        left: ArenaListId<Lane>,
        right: ArenaListId<Lane>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError>
    where
        T: Clone,
    {
        let mut copy = ArenaListId::empty();
        // The chain is reverse-linked, so collect the explicit fallback in
        // its cheap direction and replay it once. This keeps counted shared
        // copying O(nodes + actual block crossings), never O(nodes*blocks).
        let reverse = self
            .list(pool, right)?
            .iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>();
        for value in reverse.into_iter().rev() {
            self.append_payload(pool, &mut copy, value, None)?;
        }
        self.counters.new_semantic_nodes = self
            .counters
            .new_semantic_nodes
            .saturating_sub(right.len as u64);
        self.record_source_nodes_copied(right.len());
        self.splice_unique_direct_root(pool, left, UniqueArenaList { root: copy })
    }

    /// Copies a shared coordinate while maintaining summaries on every new
    /// physical block. This is required when a demand-enabled active builder
    /// appends generated nodes after the copied range.
    fn copy_shared_then_splice_summarized(
        &mut self,
        pool: &mut ChunkPool<T>,
        left: ArenaListId<Lane>,
        right: ArenaListId<Lane>,
        item_identity: &mut impl FnMut(&T) -> u64,
    ) -> Result<
        (
            ArenaListId<Lane>,
            SemanticSequenceIdentity,
            SequenceSummaryWork,
        ),
        ForkArenaError,
    >
    where
        T: Clone,
    {
        let mut copy = ArenaListId::empty();
        let reverse = self
            .list(pool, right)?
            .iter()
            .rev()
            .map(|value| (value.clone(), item_identity(value)))
            .collect::<Vec<_>>();
        let mut summary = SemanticSequenceIdentity::empty();
        for (value, identity) in reverse.into_iter().rev() {
            self.append_payload(pool, &mut copy, value, Some(identity))?;
            summary.push_back(identity);
        }
        self.counters.new_semantic_nodes = self
            .counters
            .new_semantic_nodes
            .saturating_sub(right.len as u64);
        self.record_source_nodes_copied(right.len());
        let root = self.splice_unique_direct_root(pool, left, UniqueArenaList { root: copy })?;
        Ok((
            root,
            summary,
            SequenceSummaryWork {
                hashed_values: right.len as u64,
                combined_summaries: 0,
            },
        ))
    }

    fn seal_direct_tail(
        &mut self,
        pool: &mut ChunkPool<T>,
        root: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        if root.is_empty() || pool.payload.is_sealed(root.tail.raw, self.owner)? {
            return Ok(());
        }
        let unused = pool.payload.seal(root.tail.raw, self.owner)?;
        self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        self.counters.unused_sealed_bytes = self.counters.unused_sealed_bytes.saturating_add(
            u64::try_from(unused.saturating_mul(std::mem::size_of::<Option<T>>()))
                .unwrap_or(u64::MAX),
        );
        Ok(())
    }

    pub fn seal_boundary(
        &mut self,
        pool: &mut ChunkPool<T>,
    ) -> Result<SealedBoundary<Lane>, ForkArenaError> {
        self.can_seal_boundary(pool)?;
        self.bind_pool(pool)
            .expect("boundary pool binding was preflighted");
        let payload_tail = self.live_key_at(false, self.live_payload_len().saturating_sub(1));
        let descriptor_tail = self.live_key_at(true, self.live_descriptor_len().saturating_sub(1));
        let mut sealed = 0_u64;
        let mut unused_bytes = 0_u64;
        {
            if let Some(key) = payload_tail
                && !pool.payload.is_sealed(key, self.owner)?
            {
                let unused = pool.payload.seal(key, self.owner)?;
                sealed += 1;
                unused_bytes =
                    unused_bytes.saturating_add((unused * std::mem::size_of::<Option<T>>()) as u64);
            }
            if let Some(key) = descriptor_tail
                && !pool.descriptors.is_sealed(key, self.owner)?
            {
                let unused = pool.descriptors.seal(key, self.owner)?;
                sealed += 1;
                unused_bytes = unused_bytes
                    .saturating_add((unused * std::mem::size_of::<Option<()>>()) as u64);
            }
        }
        self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(sealed);
        self.counters.unused_sealed_bytes = self
            .counters
            .unused_sealed_bytes
            .saturating_add(unused_bytes);
        Ok(SealedBoundary {
            arena: self.owner,
            payload_chunks: u32::try_from(self.live_payload_len())
                .expect("payload boundary length was preflighted"),
            descriptor_chunks: u32::try_from(self.live_descriptor_len())
                .expect("descriptor boundary length was preflighted"),
            payload_tail,
            descriptor_tail,
            _lane: PhantomData,
        })
    }

    pub fn checkpoint_mark(
        &self,
        boundary: SealedBoundary<Lane>,
    ) -> Result<CheckpointMark<Lane>, ForkArenaError> {
        if boundary.arena != self.owner
            || boundary.payload_chunks as usize != self.live_payload_len()
            || boundary.descriptor_chunks as usize != self.live_descriptor_len()
        {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        Ok(CheckpointMark {
            arena: boundary.arena,
            payload_chunks: boundary.payload_chunks,
            descriptor_chunks: boundary.descriptor_chunks,
            payload_tail: boundary.payload_tail,
            descriptor_tail: boundary.descriptor_tail,
            _lane: PhantomData,
        })
    }

    pub fn validates_checkpoint(&self, mark: CheckpointMark<Lane>) -> bool {
        mark.arena == self.owner
            && mark.payload_chunks >= self.base_payload_chunks
            && mark.descriptor_chunks >= self.base_descriptor_chunks
            && mark.payload_chunks as usize <= self.live_payload_len()
            && mark.descriptor_chunks as usize <= self.live_descriptor_len()
            && (mark.payload_chunks == self.base_payload_chunks
                || mark.payload_tail
                    == mark
                        .payload_chunks
                        .checked_sub(1)
                        .and_then(|index| self.live_key_at(false, index as usize)))
            && (mark.descriptor_chunks == self.base_descriptor_chunks
                || mark.descriptor_tail
                    == mark
                        .descriptor_chunks
                        .checked_sub(1)
                        .and_then(|index| self.live_key_at(true, index as usize)))
    }

    /// Returns whole accepted prefix chunks to the pool while retaining
    /// `mark` as a logical base coordinate independent of the released keys.
    pub(crate) fn release_accepted_prefix(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: CheckpointMark<Lane>,
    ) -> Result<usize, ForkArenaError> {
        self.bind_pool(pool)?;
        if self.active_builder
            || self.pending_batch.is_some()
            || !matches!(self.ownership, ForkOwnership::Accepted(_))
            || !self.validates_checkpoint(mark)
        {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        let payload_count = (mark.payload_chunks - self.base_payload_chunks) as usize;
        let descriptor_count = (mark.descriptor_chunks - self.base_descriptor_chunks) as usize;
        let ForkOwnership::Accepted(accepted) = &mut self.ownership else {
            unreachable!()
        };
        let owner = self.owner;
        for key in accepted.payload.drain(..payload_count) {
            pool.payload.unindex_from_arena(key, owner);
            pool.payload.release(key, owner)?;
        }
        for key in accepted.descriptors.drain(..descriptor_count) {
            pool.descriptors.unindex_from_arena(key, owner);
            pool.descriptors.release(key, owner)?;
        }
        self.base_payload_chunks = mark.payload_chunks;
        self.base_descriptor_chunks = mark.descriptor_chunks;
        Ok(payload_count.saturating_add(descriptor_count))
    }

    /// Returns whether an accepted arena can detach its suffix at `mark`.
    ///
    /// This is the read-only half of checkpoint selection. Aggregate restore
    /// validates it before mutating any other owner so the later selection and
    /// settlement phases are infallible.
    pub fn can_begin_checkpoint_candidate(&self, mark: CheckpointMark<Lane>) -> bool {
        !self.active_builder
            && self.pending_batch.is_none()
            && matches!(self.ownership, ForkOwnership::Accepted(_))
            && self.validates_checkpoint(mark)
    }

    pub fn visit_checkpoint_values(
        &self,
        pool: &ChunkPool<T>,
        mark: CheckpointMark<Lane>,
        mut visit: impl FnMut(&T),
    ) -> Result<(), ForkArenaError> {
        if !self.validates_checkpoint(mark) {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        for position in self.base_payload_chunks as usize..mark.payload_chunks as usize {
            let key = self
                .live_key_at(false, position)
                .ok_or(ForkArenaError::InvalidCheckpoint)?;
            let used = pool.payload.used(key, self.owner)?;
            for offset in 0..used {
                visit(
                    pool.payload
                        .get(key, self.owner, offset)
                        .ok_or(ForkArenaError::InvalidChunk)?,
                );
            }
        }
        Ok(())
    }

    /// Visits only the accepted payload suffix after `mark`, before that
    /// suffix is detached for a candidate. Work is proportional to the exact
    /// suffix selected for the fork and never touches the unchanged prefix.
    #[doc(hidden)]
    pub fn visit_accepted_checkpoint_suffix(
        &self,
        pool: &ChunkPool<T>,
        mark: CheckpointMark<Lane>,
        mut visit: impl FnMut(&T),
    ) -> Result<(), ForkArenaError> {
        if !self.can_begin_checkpoint_candidate(mark) {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        for position in mark.payload_chunks as usize..self.live_payload_len() {
            let key = self
                .live_key_at(false, position)
                .ok_or(ForkArenaError::InvalidCheckpoint)?;
            let used = pool.payload.used(key, self.owner)?;
            for offset in 0..used {
                visit(
                    pool.payload
                        .get(key, self.owner, offset)
                        .ok_or(ForkArenaError::InvalidChunk)?,
                );
            }
        }
        Ok(())
    }

    /// Mutates the accepted suffix after `mark` in reverse sequence order.
    ///
    /// This is the reversible-journal counterpart of
    /// [`Self::visit_accepted_checkpoint_suffix`]. The arena topology remains
    /// sealed and unchanged; only move-owned journal cells are updated before
    /// the suffix is detached.
    #[doc(hidden)]
    pub fn visit_accepted_checkpoint_suffix_mut_reverse(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: CheckpointMark<Lane>,
        mut visit: impl FnMut(&mut T),
    ) -> Result<(), ForkArenaError> {
        if !self.can_begin_checkpoint_candidate(mark) {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        self.visit_live_payload_range_mut_reverse(
            pool,
            mark.payload_chunks as usize,
            self.live_payload_len(),
            &mut visit,
        )
    }

    /// Mutates the current candidate suffix after its selected prefix in
    /// reverse sequence order without touching the detached accepted suffix.
    #[doc(hidden)]
    pub fn visit_current_checkpoint_suffix_mut_reverse(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: CheckpointMark<Lane>,
        mut visit: impl FnMut(&mut T),
    ) -> Result<(), ForkArenaError> {
        if !matches!(self.ownership, ForkOwnership::Forked { .. })
            || !self.validates_checkpoint(mark)
        {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        self.visit_live_payload_range_mut_reverse(
            pool,
            mark.payload_chunks as usize,
            self.live_payload_len(),
            &mut visit,
        )
    }

    /// Visits the current candidate suffix after `mark` without touching the
    /// detached accepted suffix.
    #[doc(hidden)]
    pub fn visit_current_checkpoint_suffix(
        &self,
        pool: &ChunkPool<T>,
        mark: CheckpointMark<Lane>,
        mut visit: impl FnMut(&T),
    ) -> Result<(), ForkArenaError> {
        if !matches!(self.ownership, ForkOwnership::Forked { .. })
            || !self.validates_checkpoint(mark)
        {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        for position in mark.payload_chunks as usize..self.live_payload_len() {
            let key = self
                .live_key_at(false, position)
                .ok_or(ForkArenaError::InvalidCheckpoint)?;
            let used = pool.payload.used(key, self.owner)?;
            for offset in 0..used {
                visit(
                    pool.payload
                        .get(key, self.owner, offset)
                        .ok_or(ForkArenaError::InvalidChunk)?,
                );
            }
        }
        Ok(())
    }

    /// Mutates the detached accepted suffix in sequence order. This permits a
    /// reversible journal to redo its prior suffix immediately before arena
    /// rejection reattaches the same chunks.
    #[doc(hidden)]
    pub fn visit_detached_checkpoint_suffix_mut(
        &self,
        pool: &mut ChunkPool<T>,
        mut visit: impl FnMut(&mut T),
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        let ForkOwnership::Forked { detached_prior, .. } = &self.ownership else {
            return Err(ForkArenaError::NotForked);
        };
        for key in &detached_prior.payload {
            let used = pool.payload.used(*key, self.owner)?;
            for offset in 0..used {
                let value = pool
                    .payload
                    .get_mut(*key, self.owner, offset)
                    .ok_or(ForkArenaError::InvalidChunk)?;
                visit(value);
            }
        }
        Ok(())
    }

    fn visit_live_payload_range_mut_reverse(
        &mut self,
        pool: &mut ChunkPool<T>,
        start: usize,
        end: usize,
        visit: &mut impl FnMut(&mut T),
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        for position in (start..end).rev() {
            let key = self
                .live_key_at(false, position)
                .ok_or(ForkArenaError::InvalidCheckpoint)?;
            let used = pool.payload.used(key, self.owner)?;
            for offset in (0..used).rev() {
                let value = pool
                    .payload
                    .get_mut(key, self.owner, offset)
                    .ok_or(ForkArenaError::InvalidChunk)?;
                visit(value);
            }
        }
        Ok(())
    }

    /// Returns one cell from a lane whose publication contract seals exactly
    /// one direct payload block per record.
    #[doc(hidden)]
    pub fn sealed_single_at<'a>(
        &self,
        pool: &'a ChunkPool<T>,
        position: usize,
    ) -> Result<(ArenaListId<Lane>, CheckpointMark<Lane>, &'a T), ForkArenaError> {
        self.validate_pool(pool)?;
        let payload = self
            .live_key_at(false, position)
            .ok_or(ForkArenaError::InvalidRange)?;
        if pool.payload.used(payload, self.owner)? != 1
            || !pool.payload.is_sealed(payload, self.owner)?
        {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        let value = pool
            .payload
            .get(payload, self.owner, 0)
            .ok_or(ForkArenaError::InvalidRange)?;
        let list = ArenaListId::from_root(
            self.owner,
            ChunkCursor::new(payload, 0),
            ChunkCursor::new(payload, 1),
            1,
        );
        self.validate_list(pool, list)?;
        Ok((
            list,
            CheckpointMark {
                arena: self.owner,
                payload_chunks: (position + 1) as u32,
                descriptor_chunks: 0,
                payload_tail: Some(payload),
                descriptor_tail: None,
                _lane: PhantomData,
            },
            value,
        ))
    }

    /// Number of one-cell records in a sealed record lane.
    #[doc(hidden)]
    pub fn sealed_single_len(&self) -> Result<usize, ForkArenaError> {
        if self.active_builder {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        Ok(self.live_payload_len())
    }

    pub fn begin_checkpoint_candidate(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: CheckpointMark<Lane>,
    ) -> Result<(), ForkArenaError> {
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        if !matches!(self.ownership, ForkOwnership::Accepted(_)) {
            return Err(ForkArenaError::AlreadyForked);
        }
        if !self.validates_checkpoint(mark) {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        let ForkOwnership::Accepted(mut accepted) = std::mem::replace(
            &mut self.ownership,
            ForkOwnership::Accepted(ChunkSet::default()),
        ) else {
            unreachable!()
        };
        let detached_prior = ChunkSet {
            payload: accepted
                .payload
                .split_off((mark.payload_chunks - self.base_payload_chunks) as usize),
            descriptors: accepted
                .descriptors
                .split_off((mark.descriptor_chunks - self.base_descriptor_chunks) as usize),
        };
        for key in &detached_prior.payload {
            self.unindex_chunk(pool, false, *key);
        }
        for key in &detached_prior.descriptors {
            self.unindex_chunk(pool, true, *key);
        }
        self.ownership = ForkOwnership::Forked {
            prefix: accepted,
            detached_prior,
            current: ChunkSet::default(),
        };
        Ok(())
    }

    /// Destructively restores an accepted arena to a retained whole-chunk
    /// checkpoint and prunes the superseded accepted suffix.
    ///
    /// Callers needing reject/retry keep the arena forked and use the explicit
    /// candidate settlement methods instead. This convenience exists for the
    /// aggregate same-generation restore barrier, whose validation phase has
    /// already established [`Self::can_begin_checkpoint_candidate`].
    pub fn restore_accepted_checkpoint(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: CheckpointMark<Lane>,
    ) -> Result<(), ForkArenaError> {
        if !self.can_begin_checkpoint_candidate(mark) {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        self.begin_checkpoint_candidate(pool, mark)?;
        let boundary = self.seal_boundary(pool)?;
        self.accept_checkpoint_candidate(pool, boundary)
    }

    /// Destructively restores only the current suffix of an already-forked
    /// arena while leaving its detached accepted suffix parked. This is the
    /// candidate-local transaction rollback counterpart of
    /// [`Self::restore_accepted_checkpoint`].
    pub(crate) fn restore_current_checkpoint(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: CheckpointMark<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.bind_pool(pool)?;
        if self.active_builder
            || self.pending_batch.is_some()
            || !matches!(self.ownership, ForkOwnership::Forked { .. })
            || !self.validates_checkpoint(mark)
        {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        let payload_tail_used = match mark.payload_tail {
            Some(key) => pool.payload.used(key, self.owner)?,
            None => 0,
        };
        let payload_tail_summary = match mark.payload_tail {
            Some(key) => pool.payload.sequence_summary(key, self.owner)?,
            None => None,
        };
        let descriptor_tail_used = match mark.descriptor_tail {
            Some(key) => pool.descriptors.used(key, self.owner)?,
            None => 0,
        };
        self.truncate_lane(
            pool,
            false,
            mark.payload_chunks as usize,
            payload_tail_used,
            payload_tail_summary,
        )?;
        self.truncate_lane(
            pool,
            true,
            mark.descriptor_chunks as usize,
            descriptor_tail_used,
            None,
        )
    }

    pub fn reject_checkpoint_candidate(
        &mut self,
        pool: &mut ChunkPool<T>,
        boundary: SealedBoundary<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.bind_pool(pool)?;
        self.validate_settlement_boundary(&boundary)?;
        let ForkOwnership::Forked {
            mut prefix,
            detached_prior,
            current,
        } = std::mem::replace(
            &mut self.ownership,
            ForkOwnership::Accepted(ChunkSet::default()),
        )
        else {
            return Err(ForkArenaError::NotForked);
        };
        let released = self.release_set(pool, current)?;
        self.counters.candidate_chunks_truncated = self
            .counters
            .candidate_chunks_truncated
            .saturating_add(released as u64);
        self.counters.accepted_chunks_reattached =
            self.counters.accepted_chunks_reattached.saturating_add(
                (detached_prior.payload.len() + detached_prior.descriptors.len()) as u64,
            );
        let payload_start = self.base_payload_chunks as usize + prefix.payload.len();
        let descriptor_start = self.base_descriptor_chunks as usize + prefix.descriptors.len();
        for (offset, key) in detached_prior.payload.iter().copied().enumerate() {
            self.index_chunk(pool, false, key, payload_start + offset);
        }
        for (offset, key) in detached_prior.descriptors.iter().copied().enumerate() {
            self.index_chunk(pool, true, key, descriptor_start + offset);
        }
        prefix.payload.extend(detached_prior.payload);
        prefix.descriptors.extend(detached_prior.descriptors);
        self.ownership = ForkOwnership::Accepted(prefix);
        Ok(())
    }

    pub fn accept_checkpoint_candidate(
        &mut self,
        pool: &mut ChunkPool<T>,
        boundary: SealedBoundary<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.bind_pool(pool)?;
        self.validate_settlement_boundary(&boundary)?;
        let ForkOwnership::Forked {
            mut prefix,
            detached_prior,
            current,
        } = std::mem::replace(
            &mut self.ownership,
            ForkOwnership::Accepted(ChunkSet::default()),
        )
        else {
            return Err(ForkArenaError::NotForked);
        };
        let pruned = self.release_set(pool, detached_prior)?;
        self.counters.obsolete_chunks_pruned = self
            .counters
            .obsolete_chunks_pruned
            .saturating_add(pruned as u64);
        prefix.payload.extend(current.payload);
        prefix.descriptors.extend(current.descriptors);
        self.ownership = ForkOwnership::Accepted(prefix);
        Ok(())
    }

    fn validate_settlement_boundary(
        &self,
        boundary: &SealedBoundary<Lane>,
    ) -> Result<(), ForkArenaError> {
        if self.active_builder
            || boundary.arena != self.owner
            || boundary.payload_chunks as usize != self.live_payload_len()
            || boundary.descriptor_chunks as usize != self.live_descriptor_len()
        {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        if !matches!(self.ownership, ForkOwnership::Forked { .. }) {
            return Err(ForkArenaError::NotForked);
        }
        Ok(())
    }

    fn release_set(
        &mut self,
        pool: &mut ChunkPool<T>,
        set: ChunkSet,
    ) -> Result<usize, ForkArenaError> {
        let count = set.payload.len() + set.descriptors.len();
        for key in set.payload {
            self.unindex_chunk(pool, false, key);
            pool.payload.release(key, self.owner)?;
        }
        for key in set.descriptors {
            self.unindex_chunk(pool, true, key);
            pool.descriptors.release(key, self.owner)?;
        }
        Ok(count)
    }

    pub fn begin_batch(
        &mut self,
        pool: &mut ChunkPool<T>,
    ) -> Result<BatchMark<Lane>, ForkArenaError> {
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let boundary = self.seal_boundary(pool)?;
        Ok(BatchMark {
            arena: self.owner,
            payload_start: boundary.payload_chunks,
            descriptor_start: boundary.descriptor_chunks,
            _lane: PhantomData,
        })
    }

    pub fn seal_batch(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: BatchMark<Lane>,
        lists: Vec<ArenaListId<Lane>>,
    ) -> Result<SealedBatch<Lane>, ForkArenaError> {
        if mark.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        let boundary = self.seal_boundary(pool)?;
        for list in &lists {
            self.validate_list_in_suffix(
                pool,
                *list,
                mark.payload_start as usize,
                mark.descriptor_start as usize,
            )?;
        }
        let serial = self.next_batch_serial;
        self.next_batch_serial = serial
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        self.pending_batch = Some(PendingBatch {
            serial,
            payload_start: mark.payload_start,
            payload_end: boundary.payload_chunks,
            descriptor_start: mark.descriptor_start,
            descriptor_end: boundary.descriptor_chunks,
        });
        Ok(SealedBatch {
            arena: self.owner,
            serial,
            payload_start: mark.payload_start,
            payload_end: boundary.payload_chunks,
            descriptor_start: mark.descriptor_start,
            descriptor_end: boundary.descriptor_chunks,
            lists,
        })
    }

    /// Mutation-free closure preflight for a build suffix whose final tails
    /// have not yet been sealed. This lets semantic rejection preserve even
    /// lifecycle counters and sealed-capacity state.
    #[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
    pub(crate) fn preflight_batch_closure(
        &self,
        pool: &ChunkPool<T>,
        mark: &BatchMark<Lane>,
        lists: &[ArenaListId<Lane>],
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.validate_pool(pool)?;
        if self.active_builder || self.pending_batch.is_some() || mark.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        let payload_end = self.live_payload_len();
        let descriptor_end = self.live_descriptor_len();
        let payload = self.validate_suffix(false, mark.payload_start as usize, payload_end)?;
        self.validate_suffix(true, mark.descriptor_start as usize, descriptor_end)?;
        for list in lists {
            self.validate_list_in_suffix(
                pool,
                *list,
                mark.payload_start as usize,
                mark.descriptor_start as usize,
            )?;
        }
        for key in payload {
            let used = pool.payload.used(key, self.owner)?;
            for offset in 0..used {
                let value = pool
                    .payload
                    .get(key, self.owner, offset)
                    .ok_or(ForkArenaError::InvalidChunk)?;
                let mut valid = true;
                value.visit_region_lists(&mut |list| {
                    valid &= self
                        .validate_list_in_suffix(
                            pool,
                            list,
                            mark.payload_start as usize,
                            mark.descriptor_start as usize,
                        )
                        .is_ok();
                });
                if !valid {
                    return Err(ForkArenaError::InvalidRegion);
                }
            }
        }
        Ok(())
    }

    #[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
    pub(crate) fn cancel_batch(&mut self, batch: SealedBatch<Lane>) -> Result<(), ForkArenaError> {
        if batch.arena != self.owner
            || self.pending_batch
                != Some(PendingBatch {
                    serial: batch.serial,
                    payload_start: batch.payload_start,
                    payload_end: batch.payload_end,
                    descriptor_start: batch.descriptor_start,
                    descriptor_end: batch.descriptor_end,
                })
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.pending_batch = None;
        Ok(())
    }

    /// Detaches a prevalidated self-contained suffix without copying payload
    /// or changing chunk ownership. The returned loan is the only authority
    /// which may reattach or transfer those envelopes.
    #[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
    pub(crate) fn detach_batch(
        &mut self,
        pool: &mut ChunkPool<T>,
        batch: SealedBatch<Lane>,
    ) -> Result<DetachedBatch<Lane>, BatchTransferError<Lane>>
    where
        T: RegionValue<Lane>,
    {
        let rebrand_values = match self.preflight_self_contained_batch(pool, &batch) {
            Ok(values) => values,
            Err(error) => return Err(BatchTransferError { error, batch }),
        };
        let payload = self
            .detach_suffix(false, batch.payload_start as usize)
            .expect("self-contained payload suffix was preflighted");
        let descriptors = self
            .detach_suffix(true, batch.descriptor_start as usize)
            .expect("self-contained descriptor suffix was preflighted");
        for key in &payload {
            self.unindex_chunk(pool, false, *key);
        }
        for key in &descriptors {
            self.unindex_chunk(pool, true, *key);
        }
        Ok(DetachedBatch {
            arena: batch.arena,
            serial: batch.serial,
            payload_start: batch.payload_start,
            descriptor_start: batch.descriptor_start,
            payload,
            descriptors,
            lists: batch.lists,
            rebrand_values,
        })
    }

    /// Returns a transient transfer loan to its exact source suffix without
    /// copying or changing any payload address.
    #[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
    pub(crate) fn reattach_batch(
        &mut self,
        pool: &mut ChunkPool<T>,
        batch: DetachedBatch<Lane>,
    ) -> Result<(), DetachedBatchTransferError<Lane>> {
        let expected = PendingBatch {
            serial: batch.serial,
            payload_start: batch.payload_start,
            payload_end: batch
                .payload_start
                .saturating_add(batch.payload.len() as u32),
            descriptor_start: batch.descriptor_start,
            descriptor_end: batch
                .descriptor_start
                .saturating_add(batch.descriptors.len() as u32),
        };
        let valid = self.validate_pool(pool).and_then(|()| {
            if batch.arena != self.owner
                || self.pending_batch != Some(expected)
                || self.live_payload_len() != batch.payload_start as usize
                || self.live_descriptor_len() != batch.descriptor_start as usize
            {
                return Err(ForkArenaError::InvalidRegion);
            }
            for key in &batch.payload {
                pool.payload.used(*key, self.owner)?;
            }
            for key in &batch.descriptors {
                pool.descriptors.used(*key, self.owner)?;
            }
            Ok(())
        });
        if let Err(error) = valid {
            return Err(DetachedBatchTransferError { error, batch });
        }
        for (offset, key) in batch.payload.iter().copied().enumerate() {
            self.index_chunk(pool, false, key, batch.payload_start as usize + offset);
        }
        for (offset, key) in batch.descriptors.iter().copied().enumerate() {
            self.index_chunk(pool, true, key, batch.descriptor_start as usize + offset);
        }
        {
            let current = self.current_chunks_mut();
            current.payload.extend(batch.payload);
            current.descriptors.extend(batch.descriptors);
        }
        self.pending_batch = None;
        Ok(())
    }

    /// Commits a detached suffix into another arena. All fallible destination
    /// checks precede payload rebranding or chunk-owner mutation.
    #[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
    pub(crate) fn promote_detached_batch_into<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        batch: DetachedBatch<Lane>,
    ) -> Result<(Vec<ArenaListId<Destination>>, u64), DetachedBatchTransferError<Lane>>
    where
        T: RegionValue<Lane>,
    {
        let expected = PendingBatch {
            serial: batch.serial,
            payload_start: batch.payload_start,
            payload_end: batch
                .payload_start
                .saturating_add(batch.payload.len() as u32),
            descriptor_start: batch.descriptor_start,
            descriptor_end: batch
                .descriptor_start
                .saturating_add(batch.descriptors.len() as u32),
        };
        let preflight = self.validate_pool(pool).and_then(|()| {
            destination.validate_pool(pool)?;
            if batch.arena != self.owner
                || self.owner == destination.owner
                || self.pending_batch != Some(expected)
                || self.live_payload_len() != batch.payload_start as usize
                || self.live_descriptor_len() != batch.descriptor_start as usize
                || destination.active_builder
            {
                return Err(ForkArenaError::InvalidRegion);
            }
            if destination.pending_batch.is_some() {
                return Err(ForkArenaError::ActiveBatch);
            }
            destination.can_seal_boundary(pool)?;
            for key in &batch.payload {
                if !pool.payload.is_sealed(*key, self.owner)? {
                    return Err(ForkArenaError::UnsealedBoundary);
                }
            }
            for key in &batch.descriptors {
                if !pool.descriptors.is_sealed(*key, self.owner)? {
                    return Err(ForkArenaError::UnsealedBoundary);
                }
            }
            Ok(())
        });
        if let Err(error) = preflight {
            return Err(DetachedBatchTransferError { error, batch });
        }

        destination
            .seal_boundary(pool)
            .expect("detached destination boundary was preflighted");
        for key in &batch.payload {
            let used = pool
                .payload
                .used(*key, self.owner)
                .expect("detached payload owner was preflighted");
            for offset in 0..used {
                pool.payload
                    .get_mut(*key, self.owner, offset)
                    .expect("detached payload cell was preflighted")
                    .rebrand_region_lists(destination.owner);
            }
        }
        for key in &batch.payload {
            pool.payload
                .transfer(*key, self.owner, destination.owner)
                .expect("detached payload transfer was preflighted");
        }
        for key in &batch.descriptors {
            pool.descriptors
                .transfer(*key, self.owner, destination.owner)
                .expect("detached descriptor transfer was preflighted");
        }
        let promoted_lists = batch
            .lists
            .iter()
            .copied()
            .map(|list| rebrand_list(list, destination.owner))
            .collect::<Vec<_>>();
        let payload_start = destination.live_payload_len();
        let descriptor_start = destination.live_descriptor_len();
        for (offset, key) in batch.payload.iter().copied().enumerate() {
            destination.index_chunk(pool, false, key, payload_start + offset);
        }
        for (offset, key) in batch.descriptors.iter().copied().enumerate() {
            destination.index_chunk(pool, true, key, descriptor_start + offset);
        }
        let promoted = batch.payload.len() + batch.descriptors.len();
        {
            let current = destination.current_chunks_mut();
            current.payload.extend(batch.payload);
            current.descriptors.extend(batch.descriptors);
        }
        self.counters.chunks_promoted = self
            .counters
            .chunks_promoted
            .saturating_add(promoted as u64);
        destination.counters.chunks_promoted = destination
            .counters
            .chunks_promoted
            .saturating_add(promoted as u64);
        self.pending_batch = None;
        Ok((promoted_lists, batch.rebrand_values))
    }

    pub(crate) fn promote_batch_into<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        batch: SealedBatch<Lane>,
    ) -> Result<Vec<ArenaListId<Destination>>, BatchTransferError<Lane>>
    where
        T: RegionValue<Lane>,
    {
        if let Err(error) = self.preflight_batch_transfer(pool, destination, &batch) {
            return Err(BatchTransferError { error, batch });
        }
        let promoted_lists = batch
            .lists
            .iter()
            .copied()
            .map(|list| rebrand_list(list, destination.owner))
            .collect::<Vec<_>>();
        self.bind_pool(pool)
            .expect("batch transfer pool was preflighted");
        destination
            .bind_pool(pool)
            .expect("batch destination pool was preflighted");
        destination
            .seal_boundary(pool)
            .expect("batch destination boundary was preflighted");
        let payload = self
            .validate_suffix(
                false,
                batch.payload_start as usize,
                batch.payload_end as usize,
            )
            .expect("batch payload suffix was preflighted");
        self.validate_suffix(
            true,
            batch.descriptor_start as usize,
            batch.descriptor_end as usize,
        )
        .expect("batch descriptor suffix was preflighted");
        for key in &payload {
            let used = pool
                .payload
                .used(*key, self.owner)
                .expect("batch payload was preflighted");
            for offset in 0..used {
                pool.payload
                    .get_mut(*key, self.owner, offset)
                    .expect("batch payload cell was preflighted")
                    .rebrand_region_lists(destination.owner);
            }
        }
        let payload = self
            .detach_suffix(false, batch.payload_start as usize)
            .expect("batch payload detachment was preflighted");
        let descriptors = self
            .detach_suffix(true, batch.descriptor_start as usize)
            .expect("batch descriptor detachment was preflighted");
        for key in &payload {
            self.unindex_chunk(pool, false, *key);
        }
        for key in &descriptors {
            self.unindex_chunk(pool, true, *key);
        }
        for key in &payload {
            pool.payload
                .transfer(*key, self.owner, destination.owner)
                .expect("batch payload ownership was preflighted");
        }
        for key in &descriptors {
            pool.descriptors
                .transfer(*key, self.owner, destination.owner)
                .expect("batch descriptor ownership was preflighted");
        }
        let promoted = payload.len() + descriptors.len();
        let payload_start = destination.live_payload_len();
        let descriptor_start = destination.live_descriptor_len();
        for (offset, key) in payload.iter().copied().enumerate() {
            destination.index_chunk(pool, false, key, payload_start + offset);
        }
        for (offset, key) in descriptors.iter().copied().enumerate() {
            destination.index_chunk(pool, true, key, descriptor_start + offset);
        }
        {
            let current = destination.current_chunks_mut();
            current.payload.extend(payload);
            current.descriptors.extend(descriptors);
        }
        self.counters.chunks_promoted = self
            .counters
            .chunks_promoted
            .saturating_add(promoted as u64);
        destination.counters.chunks_promoted = destination
            .counters
            .chunks_promoted
            .saturating_add(promoted as u64);
        self.pending_batch = None;
        Ok(promoted_lists)
    }

    fn preflight_batch_transfer<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        batch: &SealedBatch<Lane>,
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.preflight_self_contained_batch(pool, batch)?;
        self.validate_pool(pool)?;
        destination.validate_pool(pool)?;
        if self.owner == destination.owner || destination.active_builder {
            return Err(ForkArenaError::InvalidRegion);
        }
        if destination.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        if batch.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        if self.pending_batch
            != Some(PendingBatch {
                serial: batch.serial,
                payload_start: batch.payload_start,
                payload_end: batch.payload_end,
                descriptor_start: batch.descriptor_start,
                descriptor_end: batch.descriptor_end,
            })
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        destination.can_seal_boundary(pool)?;
        Ok(())
    }

    fn preflight_self_contained_batch(
        &self,
        pool: &ChunkPool<T>,
        batch: &SealedBatch<Lane>,
    ) -> Result<u64, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.validate_pool(pool)?;
        if batch.arena != self.owner
            || self.pending_batch
                != Some(PendingBatch {
                    serial: batch.serial,
                    payload_start: batch.payload_start,
                    payload_end: batch.payload_end,
                    descriptor_start: batch.descriptor_start,
                    descriptor_end: batch.descriptor_end,
                })
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let payload = self.validate_suffix(
            false,
            batch.payload_start as usize,
            batch.payload_end as usize,
        )?;
        let descriptors = self.validate_suffix(
            true,
            batch.descriptor_start as usize,
            batch.descriptor_end as usize,
        )?;
        for key in &payload {
            if !pool.payload.is_sealed(*key, self.owner)? {
                return Err(ForkArenaError::UnsealedBoundary);
            }
        }
        for key in &descriptors {
            if !pool.descriptors.is_sealed(*key, self.owner)? {
                return Err(ForkArenaError::UnsealedBoundary);
            }
        }
        for list in &batch.lists {
            self.validate_list_in_suffix(
                pool,
                *list,
                batch.payload_start as usize,
                batch.descriptor_start as usize,
            )?;
        }
        let mut visited = 0_u64;
        for key in &payload {
            let used = pool.payload.used(*key, self.owner)?;
            for offset in 0..used {
                let value = pool
                    .payload
                    .get(*key, self.owner, offset)
                    .ok_or(ForkArenaError::InvalidChunk)?;
                visited = visited.saturating_add(1);
                let mut invalid = None;
                value.visit_region_lists(&mut |list| {
                    if invalid.is_none()
                        && self
                            .validate_list_in_suffix(
                                pool,
                                list,
                                batch.payload_start as usize,
                                batch.descriptor_start as usize,
                            )
                            .is_err()
                    {
                        invalid = Some(ForkArenaError::InvalidRegion);
                    }
                });
                if let Some(error) = invalid {
                    return Err(error);
                }
            }
        }
        Ok(visited)
    }

    pub(crate) fn preflight_whole_region_transfer<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        lists: &[ArenaListId<Lane>],
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        if self.active_builder || self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if !matches!(self.ownership, ForkOwnership::Accepted(_)) {
            return Err(ForkArenaError::AlreadyForked);
        }
        self.next_batch_serial
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        let boundary = SealedBatch {
            arena: self.owner,
            serial: self.next_batch_serial,
            payload_start: 0,
            payload_end: self.live_payload_len() as u32,
            descriptor_start: 0,
            descriptor_end: self.live_descriptor_len() as u32,
            lists: lists.to_vec(),
        };
        self.preflight_transfer_coordinates(pool, destination, &boundary)
    }

    fn preflight_transfer_coordinates<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        batch: &SealedBatch<Lane>,
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.validate_live_chunks(pool)?;
        destination.can_seal_boundary(pool)?;
        if self.owner == destination.owner || destination.active_builder {
            return Err(ForkArenaError::InvalidRegion);
        }
        let payload = self.validate_suffix(
            false,
            batch.payload_start as usize,
            batch.payload_end as usize,
        )?;
        self.validate_suffix(
            true,
            batch.descriptor_start as usize,
            batch.descriptor_end as usize,
        )?;
        for list in &batch.lists {
            self.validate_list_in_suffix(
                pool,
                *list,
                batch.payload_start as usize,
                batch.descriptor_start as usize,
            )?;
        }
        for key in payload {
            let used = pool.payload.used(key, self.owner)?;
            for offset in 0..used {
                let value = pool
                    .payload
                    .get(key, self.owner, offset)
                    .ok_or(ForkArenaError::InvalidChunk)?;
                let mut valid = true;
                value.visit_region_lists(&mut |list| {
                    valid &= self
                        .validate_list_in_suffix(
                            pool,
                            list,
                            batch.payload_start as usize,
                            batch.descriptor_start as usize,
                        )
                        .is_ok();
                });
                if !valid {
                    return Err(ForkArenaError::InvalidRegion);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn seal_whole_region_batch(
        &mut self,
        pool: &mut ChunkPool<T>,
        lists: Vec<ArenaListId<Lane>>,
    ) -> Result<SealedBatch<Lane>, ForkArenaError> {
        if self.active_builder || self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if !matches!(self.ownership, ForkOwnership::Accepted(_)) {
            return Err(ForkArenaError::AlreadyForked);
        }
        let boundary = self.seal_boundary(pool)?;
        let serial = self.next_batch_serial;
        self.next_batch_serial = serial
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        self.pending_batch = Some(PendingBatch {
            serial,
            payload_start: 0,
            payload_end: boundary.payload_chunks,
            descriptor_start: 0,
            descriptor_end: boundary.descriptor_chunks,
        });
        Ok(SealedBatch {
            arena: self.owner,
            serial,
            payload_start: 0,
            payload_end: boundary.payload_chunks,
            descriptor_start: 0,
            descriptor_end: boundary.descriptor_chunks,
            lists,
        })
    }

    fn validate_suffix(
        &self,
        descriptor: bool,
        start: usize,
        end: usize,
    ) -> Result<Vec<RawChunkKey>, ForkArenaError> {
        let live_len = if descriptor {
            self.live_descriptor_len()
        } else {
            self.live_payload_len()
        };
        if start > end || end != live_len {
            return Err(ForkArenaError::InvalidRegion);
        }
        (start..end)
            .map(|index| {
                self.live_key_at(descriptor, index)
                    .ok_or(ForkArenaError::InvalidRegion)
            })
            .collect()
    }

    fn detach_suffix(
        &mut self,
        descriptor: bool,
        start: usize,
    ) -> Result<Vec<RawChunkKey>, ForkArenaError> {
        let base = if descriptor {
            self.base_descriptor_chunks as usize
        } else {
            self.base_payload_chunks as usize
        };
        let prefix_len = match &self.ownership {
            ForkOwnership::Accepted(_) => base,
            ForkOwnership::Forked { prefix, .. } => {
                base + if descriptor {
                    prefix.descriptors.len()
                } else {
                    prefix.payload.len()
                }
            }
        };
        if start < prefix_len {
            return Err(ForkArenaError::InvalidRegion);
        }
        let lane = {
            let current = self.current_chunks_mut();
            if descriptor {
                &mut current.descriptors
            } else {
                &mut current.payload
            }
        };
        let local = start - prefix_len;
        if local > lane.len() {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(lane.split_off(local))
    }

    fn validate_list_in_suffix(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        payload_start: usize,
        _descriptor_start: usize,
    ) -> Result<(), ForkArenaError> {
        self.validate_list(pool, list)?;
        self.audit_direct_chain(pool, list)?;
        if list.is_empty() {
            return Ok(());
        }
        let mut key = list.tail.raw;
        loop {
            if self
                .resolved_position(pool, false, key)
                .is_none_or(|position| position < payload_start)
            {
                return Err(ForkArenaError::InvalidRegion);
            }
            if key == list.head.raw {
                break;
            }
            key = pool
                .payload
                .previous_in_list(key, self.owner)?
                .ok_or(ForkArenaError::InvalidRegion)?
                .0;
        }
        Ok(())
    }

    pub fn compose_lists(
        &mut self,
        pool: &mut ChunkPool<T>,
        lists: &[ArenaListId<Lane>],
        scratch: &mut Vec<()>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError>
    where
        T: Clone,
    {
        // This compatibility surface is intentionally implemented in terms of
        // the explicitly named shared-copy primitive. Production ownership
        // seams should consume `UniqueArenaList` instead.
        scratch.clear();
        let mut root = ArenaListId::empty();
        for list in lists.iter().copied() {
            self.validate_list(pool, list)?;
            root = if root.is_empty() {
                list
            } else {
                self.copy_shared_then_splice(pool, root, list)?
            };
        }
        self.seal_direct_tail(pool, root)?;
        Ok(root)
    }

    pub(crate) fn compose_validated_lists(
        &mut self,
        pool: &mut ChunkPool<T>,
        lists: impl IntoIterator<Item = ArenaListId<Lane>>,
        scratch: &mut Vec<()>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError>
    where
        T: Clone,
    {
        scratch.clear();
        let mut root = ArenaListId::empty();
        for list in lists {
            self.validate_list(pool, list)?;
            root = if root.is_empty() {
                list
            } else {
                self.copy_shared_then_splice(pool, root, list)?
            };
        }
        self.seal_direct_tail(pool, root)?;
        Ok(root)
    }

    /// Returns the empty canonical list for this arena lane.
    #[must_use]
    pub const fn empty_list(&self) -> ArenaListId<Lane> {
        ArenaListId::empty()
    }

    /// Selects one logical subrange without copying payload values.
    ///
    /// `scratch` is retained in the API while callers migrate away from range
    /// scratch; direct roots need no temporary topology storage.
    pub fn slice_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
        scratch: &mut Vec<()>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        scratch.clear();
        self.slice_direct_root(pool, list, selected)
    }

    pub(crate) fn slice_validated_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
        scratch: &mut Vec<()>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        self.validate_list(pool, list)?;
        scratch.clear();
        self.slice_direct_root(pool, list, selected)
    }

    pub(crate) fn slice_list_summarized(
        &mut self,
        pool: &mut ChunkPool<T>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
        scratch: &mut Vec<()>,
        mut item_identity: impl FnMut(&T) -> u64,
    ) -> Result<
        (
            ArenaListId<Lane>,
            SemanticSequenceIdentity,
            SequenceSummaryWork,
        ),
        ForkArenaError,
    > {
        scratch.clear();
        let selected_root = self.slice_direct_root(pool, list, selected)?;
        let (summary, work) =
            self.summarize_direct_root(pool, selected_root, &mut item_identity)?;
        Ok((selected_root, summary, work))
    }

    pub(crate) fn slice_validated_list_summarized(
        &mut self,
        pool: &mut ChunkPool<T>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
        scratch: &mut Vec<()>,
        mut item_identity: impl FnMut(&T) -> u64,
    ) -> Result<
        (
            ArenaListId<Lane>,
            SemanticSequenceIdentity,
            SequenceSummaryWork,
        ),
        ForkArenaError,
    > {
        self.validate_list(pool, list)?;
        scratch.clear();
        let selected_root = self.slice_direct_root(pool, list, selected)?;
        let (summary, work) =
            self.summarize_direct_root(pool, selected_root, &mut item_identity)?;
        Ok((selected_root, summary, work))
    }

    pub fn list<'a>(
        &'a self,
        pool: &'a ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<ArenaListView<'a, T, Lane>, ForkArenaError> {
        let root = self.admit_owned_root(pool, list)?;
        Ok(ArenaListView {
            arena: self,
            pool,
            list,
            root,
        })
    }

    pub(crate) fn admit_list(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.validate_list(pool, list)
    }

    /// Admits one owner-local direct root in constant time.
    ///
    /// `ArenaListId` is opaque: its ordering, chain continuity, and exact
    /// length are established only by append, slice, splice, or checked
    /// transfer. Ordinary admission therefore checks the pool/arena owners,
    /// endpoint incarnations, and endpoint offsets without replaying the
    /// accumulated predecessor chain.
    pub(crate) fn admit_owned_list(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.admit_owned_root(pool, list).map(drop)
    }

    fn admit_owned_root(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<AdmittedListRoot<Lane>, ForkArenaError> {
        self.validate_pool(pool)?;
        if list.is_empty() {
            return (list == ArenaListId::empty())
                .then_some(AdmittedListRoot {
                    head: AdmittedChunkCursor::EMPTY,
                    tail: AdmittedChunkCursor::EMPTY,
                })
                .ok_or(ForkArenaError::InvalidRange);
        }
        if list.arena != self.owner {
            return Err(ForkArenaError::ForeignArena);
        }
        let head_position = self
            .resolved_position(pool, false, list.head.raw)
            .ok_or(ForkArenaError::InvalidRange)?;
        let tail_position = self
            .resolved_position(pool, false, list.tail.raw)
            .ok_or(ForkArenaError::InvalidRange)?;
        let head = pool
            .payload
            .validate(list.head.raw, self.owner)
            .map_err(|_| ForkArenaError::InvalidRange)?;
        let tail = pool
            .payload
            .validate(list.tail.raw, self.owner)
            .map_err(|_| ForkArenaError::InvalidRange)?;
        if list.head.offset >= head.used
            || list.tail.offset == 0
            || list.tail.offset > tail.used
            || (list.head.raw == list.tail.raw && list.head.offset >= list.tail.offset)
        {
            return Err(ForkArenaError::InvalidRange);
        }
        Ok(AdmittedListRoot {
            head: AdmittedChunkCursor::new(head_position, list.head.offset),
            tail: AdmittedChunkCursor::new(tail_position, list.tail.offset),
        })
    }

    pub(crate) fn validated_list<'a>(
        &'a self,
        pool: &'a ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<ArenaListView<'a, T, Lane>, ForkArenaError> {
        let root = self.admit_owned_root(pool, list)?;
        Ok(ArenaListView {
            arena: self,
            pool,
            list,
            root,
        })
    }

    /// Mutates the sole payload cell of a one-value list without changing
    /// its sealed topology or stable coordinate.
    #[doc(hidden)]
    pub fn with_single_value_mut<R>(
        &mut self,
        pool: &mut ChunkPool<T>,
        list: ArenaListId<Lane>,
        mutate: impl FnOnce(&mut T) -> R,
    ) -> Result<R, ForkArenaError> {
        self.validate_list(pool, list)?;
        if list.len() != 1 {
            return Err(ForkArenaError::InvalidRange);
        }
        let value = pool
            .payload
            .get_mut(list.head.raw, self.owner, list.head.offset)
            .ok_or(ForkArenaError::InvalidRange)?;
        Ok(mutate(value))
    }

    fn validate_list(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.admit_owned_list(pool, list)
    }

    /// Exhaustive structural audit reserved for cold ingress and tests.
    /// Ordinary reads use [`Self::admit_owned_list`] instead.
    #[cfg(test)]
    fn audit_owned_list(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.admit_owned_list(pool, list)?;
        self.audit_direct_chain(pool, list)
    }

    fn audit_direct_chain(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        if list.is_empty() {
            return (list == ArenaListId::empty())
                .then_some(())
                .ok_or(ForkArenaError::InvalidRange);
        }
        let mut key = list.tail.raw;
        let mut end = list.tail.offset;
        let mut total = 0_usize;
        let mut crossings = 0_usize;
        loop {
            if self.resolved_position(pool, false, key).is_none() {
                return Err(ForkArenaError::InvalidRange);
            }
            let used = pool
                .payload
                .used(key, self.owner)
                .map_err(|_| ForkArenaError::InvalidRange)?;
            let start = if key == list.head.raw {
                list.head.offset
            } else {
                0
            };
            if start >= used || end > used || start >= end {
                return Err(ForkArenaError::InvalidRange);
            }
            total = total
                .checked_add((end - start) as usize)
                .ok_or(ForkArenaError::CapacityOverflow)?;
            if key == list.head.raw {
                break;
            }
            crossings += 1;
            if crossings > self.live_payload_len() {
                return Err(ForkArenaError::InvalidRange);
            }
            let previous = pool
                .payload
                .previous_in_list(key, self.owner)?
                .ok_or(ForkArenaError::InvalidRange)?;
            key = previous.0;
            end = previous.1;
        }
        if total != list.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        Ok(())
    }
}

fn rebrand_list<Source, Destination>(
    list: ArenaListId<Source>,
    arena: u32,
) -> ArenaListId<Destination> {
    if list.is_empty() {
        ArenaListId::empty()
    } else {
        ArenaListId::from_root(
            arena,
            ChunkCursor::new(list.head.raw, list.head.offset),
            ChunkCursor::new(list.tail.raw, list.tail.offset),
            list.len,
        )
    }
}

/// Exclusive operation builder. Dropping it rolls back its partial suffix.
#[must_use = "a fork-arena builder must be sealed or explicitly discarded"]
pub struct ForkArenaBuilder<'a, T, Lane> {
    arena: &'a mut ForkArena<T, Lane>,
    pool: &'a mut ChunkPool<T>,
    operation: OperationMark<Lane>,
    root: ArenaListId<Lane>,
    sequence_summary: Option<SemanticSequenceIdentity>,
    finished: bool,
}

impl<T, Lane> ForkArenaBuilder<'_, T, Lane> {
    pub fn push(&mut self, value: T) -> Result<(), ForkArenaError> {
        self.push_with_identity(value, None)
    }

    #[cfg(test)]
    pub(crate) fn push_summarized(
        &mut self,
        value: T,
        item_identity: u64,
    ) -> Result<(), ForkArenaError> {
        self.push_with_identity(value, Some(item_identity))
    }

    fn push_with_identity(
        &mut self,
        value: T,
        item_identity: Option<u64>,
    ) -> Result<(), ForkArenaError> {
        self.arena
            .append_payload(self.pool, &mut self.root, value, item_identity)?;
        match (&mut self.sequence_summary, item_identity) {
            (Some(summary), Some(item_identity)) => summary.push_back(item_identity),
            (None, Some(item_identity)) if self.root.len == 1 => {
                self.sequence_summary = Some(SemanticSequenceIdentity::from_raw(item_identity, 1));
            }
            (None, None) => {}
            _ => return Err(ForkArenaError::IdentityModeMismatch),
        }
        Ok(())
    }

    pub fn seal_unique(mut self) -> Result<UniqueArenaList<Lane>, ForkArenaError> {
        self.arena.seal_direct_tail(self.pool, self.root)?;
        self.arena.validate_list(self.pool, self.root)?;
        let list = self.root;
        self.arena.active_builder = false;
        self.finished = true;
        Ok(UniqueArenaList { root: list })
    }

    /// Seals and publishes a copyable shared root.
    pub fn seal(self) -> Result<ArenaListId<Lane>, ForkArenaError> {
        Ok(self.seal_unique()?.publish())
    }

    pub fn discard(mut self) -> Result<(), ForkArenaError> {
        self.arena.active_builder = false;
        self.finished = true;
        self.arena.restore_operation(self.pool, self.operation)
    }
}

impl<T, Lane> Drop for ForkArenaBuilder<'_, T, Lane> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.arena.active_builder = false;
        self.arena
            .restore_operation(self.pool, self.operation)
            .expect("builder rollback mark belongs to its arena");
    }
}

/// Borrowed indexed view of one canonical arena list.
pub struct ArenaListView<'a, T, Lane> {
    arena: &'a ForkArena<T, Lane>,
    pool: &'a ChunkPool<T>,
    list: ArenaListId<Lane>,
    root: AdmittedListRoot<Lane>,
}

/// Owner-relative cursor valid for one immutable admitted view.
///
/// It deliberately carries no pool-global slot or incarnation. Those are
/// checked once at root admission and resolved from the arena's sole chunk
/// ownership lane while the immutable pool borrow prevents lifecycle change.
struct AdmittedChunkCursor<Lane> {
    position: usize,
    offset: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<Lane> Clone for AdmittedChunkCursor<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Lane> Copy for AdmittedChunkCursor<Lane> {}

impl<Lane> AdmittedChunkCursor<Lane> {
    const EMPTY: Self = Self {
        position: 0,
        offset: 0,
        _lane: PhantomData,
    };

    const fn new(position: usize, offset: u32) -> Self {
        Self {
            position,
            offset,
            _lane: PhantomData,
        }
    }
}

struct AdmittedListRoot<Lane> {
    head: AdmittedChunkCursor<Lane>,
    tail: AdmittedChunkCursor<Lane>,
}

impl<Lane> Clone for AdmittedListRoot<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Lane> Copy for AdmittedListRoot<Lane> {}

/// One borrowed contiguous payload run from a direct list chain.
///
/// Chunk cells are admitted once before this value is constructed. Iteration
/// therefore reads the payload slice directly, without repeating arena-owner,
/// incarnation, or logical-index resolution for each value.
#[derive(Clone, Copy)]
pub struct ArenaChunkSlice<'a, T> {
    cells: &'a [Option<T>],
}

impl<'a, T> ArenaChunkSlice<'a, T> {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.cells.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn iter(self) -> impl DoubleEndedIterator<Item = &'a T> + ExactSizeIterator + 'a {
        self.cells
            .iter()
            .map(|cell| cell.as_ref().expect("admitted chunk range is initialized"))
    }
}

impl<T, Lane> Clone for ArenaListView<'_, T, Lane> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, Lane> Copy for ArenaListView<'_, T, Lane> {}

impl<T, Lane> core::fmt::Debug for ArenaListView<'_, T, Lane> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArenaListView")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl<'a, T, Lane> ArenaListView<'a, T, Lane> {
    #[must_use]
    pub const fn nodes(self) -> Self {
        self
    }
    #[must_use]
    pub const fn len(&self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    #[cfg(feature = "testing")]
    pub(crate) fn traversal_counters(&self) -> (u64, u64, u64) {
        (
            self.pool.payload.admitted_index_resolutions(),
            self.pool.payload.admitted_index_predecessor_steps(),
            self.pool.payload.admitted_forward_chunk_crossings(),
        )
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&'a T> {
        let cursor = self.cursor_at_node(index)?;
        self.get_cursor(cursor)
    }

    pub fn iter(&self) -> ArenaListIter<'a, T, Lane> {
        self.iter_from(0)
    }

    /// Iterates from one logical position while retaining the admitted
    /// owner-relative cursor within each packed block.
    ///
    /// The initial position and actual block crossings use indexed
    /// resolution. Nodes within a block advance the cursor directly.
    pub fn iter_from(&self, start: usize) -> ArenaListIter<'a, T, Lane> {
        let front = start.min(self.len());
        let front_span = if front < self.len() {
            self.cursor_span_at_node(front)
        } else {
            None
        };
        let (front_cursor, front_block_end) =
            front_span.map_or((None, 0), |(cursor, end)| (Some(cursor), end));
        ArenaListIter {
            view: *self,
            front,
            back: self.len(),
            front_cursor,
            front_block_end,
            back_cursor: (front < self.len()).then_some(self.root.tail),
            forward_chunk_crossings: 0,
            reverse_chunk_crossings: 0,
        }
    }

    fn key_at(&self, cursor: AdmittedChunkCursor<Lane>) -> Option<RawChunkKey> {
        self.arena.live_key_at(false, cursor.position)
    }

    fn get_cursor(&self, cursor: AdmittedChunkCursor<Lane>) -> Option<&'a T> {
        let key = self.key_at(cursor)?;
        self.pool.payload.admitted_get(key, cursor.offset)
    }

    fn previous_cursor(
        &self,
        cursor: AdmittedChunkCursor<Lane>,
    ) -> Option<AdmittedChunkCursor<Lane>> {
        let key = self.key_at(cursor)?;
        let (position, end) = self.pool.payload.admitted_previous_position(key)?;
        Some(AdmittedChunkCursor::new(position, end))
    }

    fn cursor_at_node(&self, index: usize) -> Option<AdmittedChunkCursor<Lane>> {
        self.cursor_span_at_node(index).map(|(cursor, _)| cursor)
    }

    fn cursor_span_at_node(&self, index: usize) -> Option<(AdmittedChunkCursor<Lane>, u32)> {
        if index >= self.len() || self.is_empty() {
            return None;
        }
        #[cfg(any(test, feature = "testing"))]
        self.pool.payload.admitted_index_resolutions.set(
            self.pool
                .payload
                .admitted_index_resolutions
                .get()
                .saturating_add(1),
        );
        let mut cursor = self.root.tail;
        let mut remaining = self.len() - index;
        loop {
            let start = if cursor.position == self.root.head.position {
                self.root.head.offset
            } else {
                0
            };
            if cursor.offset < start {
                return None;
            }
            let available = (cursor.offset - start) as usize;
            if remaining <= available {
                let block_end = cursor.offset;
                cursor.offset -= remaining as u32;
                return Some((cursor, block_end));
            }
            remaining -= available;
            #[cfg(any(test, feature = "testing"))]
            self.pool.payload.admitted_index_predecessor_steps.set(
                self.pool
                    .payload
                    .admitted_index_predecessor_steps
                    .get()
                    .saturating_add(1),
            );
            cursor = self.previous_cursor(cursor)?;
        }
    }

    /// Visits the authoritative chunk slices in logical list order.
    ///
    /// The root was admitted in constant time when this view was created.
    /// This walk follows the sole persistent predecessor chain once and calls
    /// `visit` while that walk unwinds, so forward consumers perform no
    /// per-value index resolution and allocate no traversal sidecar.
    pub fn visit_chunks(&self, mut visit: impl FnMut(ArenaChunkSlice<'a, T>)) {
        if self.is_empty() {
            return;
        }
        self.visit_chunk_prefix(self.root.tail, &mut visit)
            .expect("an admitted direct list remains valid during its immutable borrow");
    }

    /// Visits one logical range in forward order by walking the sole
    /// predecessor chain once.
    ///
    /// This is the zero-allocation boundary for long forward walks. The Rust
    /// call stack retains the predecessor path until callbacks run in logical
    /// order, so no successor metadata, traversal cache, or second topology is
    /// required. The callback may stop the walk early with [`ControlFlow`].
    pub fn try_for_each_range<B>(
        &self,
        selected: Range<usize>,
        mut visit: impl FnMut(usize, &'a T) -> core::ops::ControlFlow<B>,
    ) -> core::ops::ControlFlow<B> {
        assert!(
            selected.start <= selected.end && selected.end <= self.len(),
            "arena-list traversal range must be in bounds"
        );
        if selected.is_empty() {
            return core::ops::ControlFlow::Continue(());
        }
        self.visit_range_chunk_prefix(self.root.tail, self.len(), &selected, &mut visit)
            .expect("an admitted direct list remains valid during its immutable borrow")
    }

    fn visit_range_chunk_prefix<B>(
        &self,
        cursor: AdmittedChunkCursor<Lane>,
        logical_end: usize,
        selected: &Range<usize>,
        visit: &mut impl FnMut(usize, &'a T) -> core::ops::ControlFlow<B>,
    ) -> Result<core::ops::ControlFlow<B>, ForkArenaError> {
        let start_offset = if cursor.position == self.root.head.position {
            self.root.head.offset
        } else {
            0
        };
        if cursor.offset < start_offset {
            return Err(ForkArenaError::InvalidRange);
        }
        let logical_start = logical_end
            .checked_sub((cursor.offset - start_offset) as usize)
            .ok_or(ForkArenaError::InvalidRange)?;
        if selected.start < logical_start {
            let previous = self
                .previous_cursor(cursor)
                .ok_or(ForkArenaError::InvalidRange)?;
            #[cfg(any(test, feature = "testing"))]
            self.pool.payload.admitted_forward_chunk_crossings.set(
                self.pool
                    .payload
                    .admitted_forward_chunk_crossings
                    .get()
                    .saturating_add(1),
            );
            if let core::ops::ControlFlow::Break(value) =
                self.visit_range_chunk_prefix(previous, logical_start, selected, visit)?
            {
                return Ok(core::ops::ControlFlow::Break(value));
            }
        }
        let selected_start = selected.start.max(logical_start);
        let selected_end = selected.end.min(logical_end);
        if selected_start < selected_end {
            let key = self.key_at(cursor).ok_or(ForkArenaError::InvalidRange)?;
            let first = start_offset
                .checked_add(
                    u32::try_from(selected_start - logical_start)
                        .map_err(|_| ForkArenaError::CapacityOverflow)?,
                )
                .ok_or(ForkArenaError::CapacityOverflow)?;
            let last = start_offset
                .checked_add(
                    u32::try_from(selected_end - logical_start)
                        .map_err(|_| ForkArenaError::CapacityOverflow)?,
                )
                .ok_or(ForkArenaError::CapacityOverflow)?;
            let cells = self
                .pool
                .payload
                .admitted_slice(key, first..last)
                .ok_or(ForkArenaError::InvalidRange)?;
            for (offset, cell) in cells.iter().enumerate() {
                let value = cell.as_ref().ok_or(ForkArenaError::InvalidRange)?;
                if let core::ops::ControlFlow::Break(value) = visit(selected_start + offset, value)
                {
                    return Ok(core::ops::ControlFlow::Break(value));
                }
            }
        }
        Ok(core::ops::ControlFlow::Continue(()))
    }

    fn visit_chunk_prefix(
        &self,
        cursor: AdmittedChunkCursor<Lane>,
        visit: &mut impl FnMut(ArenaChunkSlice<'a, T>),
    ) -> Result<(), ForkArenaError> {
        let start = if cursor.position == self.root.head.position {
            self.root.head.offset
        } else {
            let previous = self
                .previous_cursor(cursor)
                .ok_or(ForkArenaError::InvalidRange)?;
            self.visit_chunk_prefix(previous, visit)?;
            0
        };
        let key = self.key_at(cursor).ok_or(ForkArenaError::InvalidRange)?;
        let cells = self
            .pool
            .payload
            .admitted_slice(key, start..cursor.offset)
            .ok_or(ForkArenaError::InvalidRange)?;
        visit(ArenaChunkSlice { cells });
        Ok(())
    }

    /// Visits every value in logical order through direct chunk slices.
    pub fn for_each(&self, mut visit: impl FnMut(&'a T)) {
        let _: core::ops::ControlFlow<core::convert::Infallible> =
            self.try_for_each_range(0..self.len(), |_, value| {
                visit(value);
                core::ops::ControlFlow::Continue(())
            });
    }
}

pub struct ArenaListIter<'arena, T, Lane> {
    view: ArenaListView<'arena, T, Lane>,
    front: usize,
    back: usize,
    front_cursor: Option<AdmittedChunkCursor<Lane>>,
    front_block_end: u32,
    back_cursor: Option<AdmittedChunkCursor<Lane>>,
    forward_chunk_crossings: usize,
    reverse_chunk_crossings: usize,
}

impl<T, Lane> ArenaListIter<'_, T, Lane> {
    /// Compatibility observation proving reverse traversal performs no
    /// descriptor visits.
    #[must_use]
    pub const fn reverse_descriptor_visits(&self) -> usize {
        0
    }

    /// Number of actual packed-block boundaries crossed by reverse traversal.
    #[must_use]
    pub const fn reverse_chunk_crossings(&self) -> usize {
        self.reverse_chunk_crossings
    }

    /// Number of packed-block boundaries crossed by forward traversal.
    #[must_use]
    pub const fn forward_chunk_crossings(&self) -> usize {
        self.forward_chunk_crossings
    }
}

impl<'arena, T, Lane> Iterator for ArenaListIter<'arena, T, Lane> {
    type Item = &'arena T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let cursor = self.front_cursor?;
        let value = self.view.get_cursor(cursor)?;
        self.front += 1;
        if self.front == self.back {
            self.front_cursor = None;
        } else {
            let next_offset = cursor.offset.checked_add(1)?;
            if next_offset < self.front_block_end {
                self.front_cursor = Some(AdmittedChunkCursor::new(cursor.position, next_offset));
            } else {
                let (cursor, end) = self.view.cursor_span_at_node(self.front)?;
                self.front_cursor = Some(cursor);
                self.front_block_end = end;
                self.forward_chunk_crossings = self.forward_chunk_crossings.saturating_add(1);
            }
        }
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}

impl<'arena, T, Lane> DoubleEndedIterator for ArenaListIter<'arena, T, Lane> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        let mut cursor = self.back_cursor?;
        let block_start = if cursor.position == self.view.root.head.position {
            self.view.root.head.offset
        } else {
            0
        };
        if cursor.offset == block_start {
            cursor = self.view.previous_cursor(cursor)?;
            self.reverse_chunk_crossings = self.reverse_chunk_crossings.saturating_add(1);
        }
        cursor.offset = cursor.offset.checked_sub(1)?;
        self.back_cursor = Some(cursor);
        self.view.get_cursor(cursor)
    }
}

impl<T, Lane> ExactSizeIterator for ArenaListIter<'_, T, Lane> {}
