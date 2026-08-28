//! Fixed-chunk storage with one accepted lineage and one transactional fork.
//!
//! Payload lives in coarse pool pages. Arenas own only stable chunk keys and
//! canonical, non-recursive range-list descriptors. Retained checkpoints land
//! on sealed whole-chunk boundaries; operation marks may additionally name a
//! partially used tail.

use core::marker::PhantomData;
use std::cell::{Ref, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
#[path = "fork_arena/tests.rs"]
mod tests;

const DEFAULT_CHUNK_BYTES: usize = 4 * 1024;
const CHUNKS_PER_PAGE: usize = 16;

static NEXT_POOL_OWNER: AtomicU64 = AtomicU64::new(1);
static NEXT_ARENA_OWNER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[doc(hidden)]
pub struct RawChunkKey {
    pool: u64,
    slot: u32,
    generation: u32,
}

#[derive(Clone, Copy, Debug)]
struct ChunkMeta {
    generation: u32,
    arena: u64,
    used: u32,
    live: bool,
    sealed: bool,
}

struct ChunkPage<T> {
    slots: Box<[Option<T>]>,
}

/// Stable coarse-page storage shared by typed semantic lanes.
///
/// A logical chunk is a fixed slot range inside a page allocation. Growing
/// the pool allocates one page for many chunks, so no chunk owns a heap
/// allocation and existing payload never moves.
pub struct ChunkPool<T> {
    owner: u64,
    chunk_bytes: usize,
    slots_per_chunk: usize,
    pages: Vec<ChunkPage<T>>,
    chunks: Vec<ChunkMeta>,
    free: Vec<u32>,
}

impl<T> Default for ChunkPool<T> {
    fn default() -> Self {
        Self::with_chunk_bytes(DEFAULT_CHUNK_BYTES)
    }
}

impl<T> ChunkPool<T> {
    /// Creates a pool whose logical chunk capacity is derived from a byte
    /// budget. At least one value fits even when `T` exceeds that budget.
    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        assert!(chunk_bytes != 0, "chunk byte budget must be nonzero");
        let slot_bytes = std::mem::size_of::<Option<T>>().max(1);
        let slots_per_chunk = (chunk_bytes / slot_bytes).max(1);
        Self {
            owner: NEXT_POOL_OWNER.fetch_add(1, Ordering::Relaxed),
            chunk_bytes,
            slots_per_chunk,
            pages: Vec::new(),
            chunks: Vec::new(),
            free: Vec::new(),
        }
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
            used: 0,
            live: false,
            sealed: false,
        }));
        self.free.extend((start..end).rev().map(|slot| slot as u32));
        Ok(())
    }

    fn allocate(&mut self, arena: u64) -> Result<RawChunkKey, ForkArenaError> {
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
        Ok(RawChunkKey {
            pool: self.owner,
            slot,
            generation: meta.generation,
        })
    }

    fn validate(&self, key: RawChunkKey, arena: u64) -> Result<&ChunkMeta, ForkArenaError> {
        let meta = self
            .chunks
            .get(key.slot as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if key.pool != self.owner
            || key.generation != meta.generation
            || !meta.live
            || meta.arena != arena
        {
            return Err(ForkArenaError::InvalidChunk);
        }
        Ok(meta)
    }

    fn validate_mut(
        &mut self,
        key: RawChunkKey,
        arena: u64,
    ) -> Result<&mut ChunkMeta, ForkArenaError> {
        let owner = self.owner;
        let meta = self
            .chunks
            .get_mut(key.slot as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if key.pool != owner
            || key.generation != meta.generation
            || !meta.live
            || meta.arena != arena
        {
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

    fn append(&mut self, key: RawChunkKey, arena: u64, value: T) -> Result<u32, ForkArenaError> {
        let used = {
            let meta = self.validate(key, arena)?;
            if meta.sealed || meta.used as usize == self.slots_per_chunk {
                return Err(ForkArenaError::ChunkSealed);
            }
            meta.used
        };
        let (page, index) = self.slot_index(key, used as usize)?;
        debug_assert!(self.pages[page].slots[index].is_none());
        self.pages[page].slots[index] = Some(value);
        self.validate_mut(key, arena)?.used += 1;
        Ok(used)
    }

    fn get(&self, key: RawChunkKey, arena: u64, offset: u32) -> Option<&T> {
        let meta = self.validate(key, arena).ok()?;
        if offset >= meta.used {
            return None;
        }
        let (page, index) = self.slot_index(key, offset as usize).ok()?;
        self.pages[page].slots[index].as_ref()
    }

    fn used(&self, key: RawChunkKey, arena: u64) -> Result<u32, ForkArenaError> {
        Ok(self.validate(key, arena)?.used)
    }

    fn is_sealed(&self, key: RawChunkKey, arena: u64) -> Result<bool, ForkArenaError> {
        Ok(self.validate(key, arena)?.sealed)
    }

    fn seal(&mut self, key: RawChunkKey, arena: u64) -> Result<usize, ForkArenaError> {
        let capacity = self.slots_per_chunk;
        let meta = self.validate_mut(key, arena)?;
        meta.sealed = true;
        Ok(capacity.saturating_sub(meta.used as usize))
    }

    fn truncate(&mut self, key: RawChunkKey, arena: u64, used: u32) -> Result<(), ForkArenaError> {
        let old_used = self.validate(key, arena)?.used;
        if used > old_used || (used != old_used && self.validate(key, arena)?.sealed) {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        if used == old_used {
            return Ok(());
        }
        for offset in used..old_used {
            let (page, index) = self.slot_index(key, offset as usize)?;
            drop(self.pages[page].slots[index].take());
        }
        self.validate_mut(key, arena)?.used = used;
        Ok(())
    }

    fn release(&mut self, key: RawChunkKey, arena: u64) -> Result<usize, ForkArenaError> {
        let used = self.validate(key, arena)?.used;
        for offset in 0..used {
            let (page, index) = self.slot_index(key, offset as usize)?;
            drop(self.pages[page].slots[index].take());
        }
        let meta = self.validate_mut(key, arena)?;
        meta.live = false;
        meta.sealed = false;
        meta.arena = 0;
        meta.used = 0;
        meta.generation = meta
            .generation
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        self.free.push(key.slot);
        Ok(used as usize)
    }

    fn transfer(
        &mut self,
        key: RawChunkKey,
        source: u64,
        destination: u64,
    ) -> Result<(), ForkArenaError> {
        let meta = self.validate_mut(key, source)?;
        if !meta.sealed {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        meta.arena = destination;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RawRange {
    first: RawChunkKey,
    start: u32,
    len: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RangeEntry {
    range: RawRange,
    cumulative_end: u32,
}

struct ForkPools<T> {
    payload: ChunkPool<T>,
    descriptors: ChunkPool<RangeEntry>,
}

/// A generation-checked chunk coordinate branded for one semantic lane.
pub struct ChunkId<Lane> {
    arena: u64,
    raw: RawChunkKey,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<Lane> Clone for ChunkId<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Lane> Copy for ChunkId<Lane> {}
impl<Lane> core::fmt::Debug for ChunkId<Lane> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ChunkId(..)")
    }
}
impl<Lane> PartialEq for ChunkId<Lane> {
    fn eq(&self, other: &Self) -> bool {
        self.arena == other.arena && self.raw == other.raw
    }
}
impl<Lane> Eq for ChunkId<Lane> {}

/// One contiguous span across successive chunks of an arena lane.
pub struct ArenaRange<Lane> {
    arena: u64,
    first: Option<ChunkId<Lane>>,
    start: u32,
    len: u32,
}

impl<Lane> Clone for ArenaRange<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Lane> Copy for ArenaRange<Lane> {}
impl<Lane> core::fmt::Debug for ArenaRange<Lane> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArenaRange")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}
impl<Lane> PartialEq for ArenaRange<Lane> {
    fn eq(&self, other: &Self) -> bool {
        self.arena == other.arena
            && self.first == other.first
            && self.start == other.start
            && self.len == other.len
    }
}
impl<Lane> Eq for ArenaRange<Lane> {}

impl<Lane> ArenaRange<Lane> {
    const fn empty(arena: u64) -> Self {
        Self {
            arena,
            first: None,
            start: 0,
            len: 0,
        }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.len as usize
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// The sole list coordinate: either one range or one flat arena-owned range
/// sequence. Sequence entries never name another list.
pub enum ArenaListId<Lane> {
    Range(ArenaRange<Lane>),
    Sequence {
        arena: u64,
        first: RawChunkKey,
        start: u32,
        count: u32,
        len: u32,
        _lane: PhantomData<fn(Lane) -> Lane>,
    },
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
        match (*self, *other) {
            (Self::Range(left), Self::Range(right)) => left == right,
            (
                Self::Sequence {
                    arena: la,
                    first: lf,
                    start: ls,
                    count: lc,
                    len: ll,
                    ..
                },
                Self::Sequence {
                    arena: ra,
                    first: rf,
                    start: rs,
                    count: rc,
                    len: rl,
                    ..
                },
            ) => la == ra && lf == rf && ls == rs && lc == rc && ll == rl,
            _ => false,
        }
    }
}
impl<Lane> Eq for ArenaListId<Lane> {}

impl<Lane> ArenaListId<Lane> {
    #[must_use]
    pub const fn len(self) -> usize {
        match self {
            Self::Range(range) => range.len(),
            Self::Sequence { len, .. } => len as usize,
        }
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }
}

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
    arena: u64,
    payload_chunks: u32,
    payload_tail_used: u32,
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

/// Consuming proof that every builder has retired and both tails are sealed.
pub struct SealedBoundary<Lane> {
    arena: u64,
    payload_chunks: u32,
    descriptor_chunks: u32,
    payload_tail: Option<RawChunkKey>,
    descriptor_tail: Option<RawChunkKey>,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

/// Opaque whole-chunk retained checkpoint coordinate.
pub struct CheckpointMark<Lane> {
    arena: u64,
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
    arena: u64,
    serial: u64,
    payload_start: u32,
    payload_end: u32,
    descriptor_start: u32,
    descriptor_end: u32,
    lists: Vec<ArenaListId<Lane>>,
}

pub struct BatchMark<Lane> {
    arena: u64,
    payload_start: u32,
    descriptor_start: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

/// Lifecycle work counters; payload copy is absent by construction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ForkArenaCounters {
    pub payload_values_appended: u64,
    pub payload_values_copied: u64,
    pub chunks_sealed: u64,
    pub unused_sealed_bytes: u64,
    pub chunks_promoted: u64,
    pub candidate_chunks_truncated: u64,
    pub accepted_chunks_reattached: u64,
    pub obsolete_chunks_pruned: u64,
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
    ForeignPool,
    InvalidCheckpoint,
    InvalidChunk,
    InvalidOperationMark,
    InvalidRange,
    InvalidRegion,
    NotForked,
    UnsealedBoundary,
}

/// One typed arena lane over stable shared chunk pools.
pub struct ForkArena<T, Lane> {
    owner: u64,
    pools: Rc<RefCell<ForkPools<T>>>,
    ownership: ForkOwnership,
    active_builder: bool,
    pending_batch: Option<PendingBatch>,
    next_batch_serial: u64,
    payload_resolver: Vec<(RawChunkKey, usize)>,
    descriptor_resolver: Vec<(RawChunkKey, usize)>,
    counters: ForkArenaCounters,
    _lane: PhantomData<fn(Lane) -> Lane>,
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
        Self::with_chunk_bytes(DEFAULT_CHUNK_BYTES)
    }

    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        let pools = ForkPools {
            payload: ChunkPool::with_chunk_bytes(chunk_bytes),
            descriptors: ChunkPool::with_chunk_bytes(chunk_bytes),
        };
        Self {
            owner: NEXT_ARENA_OWNER.fetch_add(1, Ordering::Relaxed),
            pools: Rc::new(RefCell::new(pools)),
            ownership: ForkOwnership::Accepted(ChunkSet::default()),
            active_builder: false,
            pending_batch: None,
            next_batch_serial: 1,
            payload_resolver: Vec::new(),
            descriptor_resolver: Vec::new(),
            counters: ForkArenaCounters::default(),
            _lane: PhantomData,
        }
    }

    /// Creates another typed lane over the same coarse pools.
    #[must_use]
    pub fn empty_lane<Destination>(&self) -> ForkArena<T, Destination> {
        ForkArena {
            owner: NEXT_ARENA_OWNER.fetch_add(1, Ordering::Relaxed),
            pools: Rc::clone(&self.pools),
            ownership: ForkOwnership::Accepted(ChunkSet::default()),
            active_builder: false,
            pending_batch: None,
            next_batch_serial: 1,
            payload_resolver: Vec::new(),
            descriptor_resolver: Vec::new(),
            counters: ForkArenaCounters::default(),
            _lane: PhantomData,
        }
    }

    #[must_use]
    pub const fn counters(&self) -> ForkArenaCounters {
        self.counters
    }

    #[must_use]
    pub fn payload_chunk_capacity(&self) -> usize {
        self.pools.borrow().payload.chunk_capacity()
    }

    fn live_payload_len(&self) -> usize {
        match &self.ownership {
            ForkOwnership::Accepted(chunks) => chunks.payload.len(),
            ForkOwnership::Forked {
                prefix, current, ..
            } => prefix.payload.len() + current.payload.len(),
        }
    }

    fn live_descriptor_len(&self) -> usize {
        match &self.ownership {
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

    fn rebuild_resolvers(&mut self) {
        self.payload_resolver.clear();
        self.descriptor_resolver.clear();
        for index in 0..self.live_payload_len() {
            self.payload_resolver.push((
                self.live_key_at(false, index).expect("live payload key"),
                index,
            ));
        }
        for index in 0..self.live_descriptor_len() {
            self.descriptor_resolver.push((
                self.live_key_at(true, index).expect("live descriptor key"),
                index,
            ));
        }
        self.payload_resolver.sort_unstable_by_key(|entry| entry.0);
        self.descriptor_resolver
            .sort_unstable_by_key(|entry| entry.0);
    }

    fn resolved_position(&self, descriptor: bool, key: RawChunkKey) -> Option<usize> {
        let resolver = if descriptor {
            &self.descriptor_resolver
        } else {
            &self.payload_resolver
        };
        resolver
            .binary_search_by_key(&key, |entry| entry.0)
            .ok()
            .map(|index| resolver[index].1)
    }

    fn allocate_chunk(&mut self, descriptor: bool) -> Result<RawChunkKey, ForkArenaError> {
        let key = {
            let mut pools = self.pools.borrow_mut();
            if descriptor {
                pools.descriptors.allocate(self.owner)?
            } else {
                pools.payload.allocate(self.owner)?
            }
        };
        let chunks = self.current_chunks_mut();
        if descriptor {
            chunks.descriptors.push(key);
        } else {
            chunks.payload.push(key);
        }
        self.rebuild_resolvers();
        Ok(key)
    }

    fn append_payload(&mut self, value: T) -> Result<(RawChunkKey, u32), ForkArenaError> {
        let last = self
            .current_chunks_mut()
            .payload
            .last()
            .copied()
            .filter(|key| {
                let pools = self.pools.borrow();
                !pools.payload.is_sealed(*key, self.owner).unwrap_or(true)
                    && pools
                        .payload
                        .used(*key, self.owner)
                        .ok()
                        .is_some_and(|used| used as usize != pools.payload.chunk_capacity())
            });
        let key = match last {
            Some(key) => key,
            None => self.allocate_chunk(false)?,
        };
        let offset = self
            .pools
            .borrow_mut()
            .payload
            .append(key, self.owner, value)?;
        let became_full = {
            let pools = self.pools.borrow();
            pools.payload.used(key, self.owner)? as usize == pools.payload.chunk_capacity()
        };
        if became_full {
            self.pools.borrow_mut().payload.seal(key, self.owner)?;
            self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        }
        self.counters.payload_values_appended += 1;
        Ok((key, offset))
    }

    fn append_descriptor(
        &mut self,
        entry: RangeEntry,
    ) -> Result<(RawChunkKey, u32), ForkArenaError> {
        let last = self
            .current_chunks_mut()
            .descriptors
            .last()
            .copied()
            .filter(|key| {
                let pools = self.pools.borrow();
                !pools
                    .descriptors
                    .is_sealed(*key, self.owner)
                    .unwrap_or(true)
                    && pools
                        .descriptors
                        .used(*key, self.owner)
                        .ok()
                        .is_some_and(|used| used as usize != pools.descriptors.chunk_capacity())
            });
        let key = match last {
            Some(key) => key,
            None => self.allocate_chunk(true)?,
        };
        let offset = self
            .pools
            .borrow_mut()
            .descriptors
            .append(key, self.owner, entry)?;
        let became_full = {
            let pools = self.pools.borrow();
            pools.descriptors.used(key, self.owner)? as usize == pools.descriptors.chunk_capacity()
        };
        if became_full {
            self.pools.borrow_mut().descriptors.seal(key, self.owner)?;
            self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        }
        Ok((key, offset))
    }

    #[must_use]
    pub fn operation_mark(&self) -> OperationMark<Lane> {
        let pools = self.pools.borrow();
        let payload_tail_used = self
            .live_key_at(false, self.live_payload_len().saturating_sub(1))
            .and_then(|key| pools.payload.used(key, self.owner).ok())
            .unwrap_or(0);
        let descriptor_tail_used = self
            .live_key_at(true, self.live_descriptor_len().saturating_sub(1))
            .and_then(|key| pools.descriptors.used(key, self.owner).ok())
            .unwrap_or(0);
        OperationMark {
            arena: self.owner,
            payload_chunks: self.live_payload_len() as u32,
            payload_tail_used,
            descriptor_chunks: self.live_descriptor_len() as u32,
            descriptor_tail_used,
            _lane: PhantomData,
        }
    }

    pub fn restore_operation(&mut self, mark: OperationMark<Lane>) -> Result<(), ForkArenaError> {
        if self.active_builder || self.pending_batch.is_some() || mark.arena != self.owner {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        self.truncate_lane(false, mark.payload_chunks as usize, mark.payload_tail_used)?;
        self.truncate_lane(
            true,
            mark.descriptor_chunks as usize,
            mark.descriptor_tail_used,
        )?;
        self.rebuild_resolvers();
        Ok(())
    }

    fn truncate_lane(
        &mut self,
        descriptor: bool,
        chunks: usize,
        tail_used: u32,
    ) -> Result<(), ForkArenaError> {
        let live_len = if descriptor {
            self.live_descriptor_len()
        } else {
            self.live_payload_len()
        };
        if chunks > live_len {
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
            let mut pools = self.pools.borrow_mut();
            if descriptor {
                pools.descriptors.release(key, self.owner)?;
            } else {
                pools.payload.release(key, self.owner)?;
            }
            self.counters.candidate_chunks_truncated += 1;
        }
        if chunks != 0 {
            let key = self
                .live_key_at(descriptor, chunks - 1)
                .ok_or(ForkArenaError::InvalidOperationMark)?;
            let mut pools = self.pools.borrow_mut();
            if descriptor {
                pools.descriptors.truncate(key, self.owner, tail_used)?;
            } else {
                pools.payload.truncate(key, self.owner, tail_used)?;
            }
        } else if tail_used != 0 {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        Ok(())
    }

    pub fn begin_builder(&mut self) -> Result<ForkArenaBuilder<'_, T, Lane>, ForkArenaError> {
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let operation = self.operation_mark();
        self.active_builder = true;
        Ok(ForkArenaBuilder {
            arena: self,
            operation,
            first: None,
            len: 0,
            finished: false,
        })
    }

    pub fn seal_boundary(&mut self) -> Result<SealedBoundary<Lane>, ForkArenaError> {
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let payload_tail = self.live_key_at(false, self.live_payload_len().saturating_sub(1));
        let descriptor_tail = self.live_key_at(true, self.live_descriptor_len().saturating_sub(1));
        let mut sealed = 0_u64;
        let mut unused_bytes = 0_u64;
        {
            let mut pools = self.pools.borrow_mut();
            if let Some(key) = payload_tail
                && !pools.payload.is_sealed(key, self.owner)?
            {
                let unused = pools.payload.seal(key, self.owner)?;
                sealed += 1;
                unused_bytes =
                    unused_bytes.saturating_add((unused * std::mem::size_of::<Option<T>>()) as u64);
            }
            if let Some(key) = descriptor_tail
                && !pools.descriptors.is_sealed(key, self.owner)?
            {
                let unused = pools.descriptors.seal(key, self.owner)?;
                sealed += 1;
                unused_bytes = unused_bytes
                    .saturating_add((unused * std::mem::size_of::<Option<RangeEntry>>()) as u64);
            }
        }
        self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(sealed);
        self.counters.unused_sealed_bytes = self
            .counters
            .unused_sealed_bytes
            .saturating_add(unused_bytes);
        Ok(SealedBoundary {
            arena: self.owner,
            payload_chunks: self.live_payload_len() as u32,
            descriptor_chunks: self.live_descriptor_len() as u32,
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

    fn validates_checkpoint(&self, mark: CheckpointMark<Lane>) -> bool {
        mark.arena == self.owner
            && mark.payload_chunks as usize <= self.live_payload_len()
            && mark.descriptor_chunks as usize <= self.live_descriptor_len()
            && mark.payload_tail
                == mark
                    .payload_chunks
                    .checked_sub(1)
                    .and_then(|index| self.live_key_at(false, index as usize))
            && mark.descriptor_tail
                == mark
                    .descriptor_chunks
                    .checked_sub(1)
                    .and_then(|index| self.live_key_at(true, index as usize))
    }

    pub fn begin_checkpoint_candidate(
        &mut self,
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
            payload: accepted.payload.split_off(mark.payload_chunks as usize),
            descriptors: accepted
                .descriptors
                .split_off(mark.descriptor_chunks as usize),
        };
        self.ownership = ForkOwnership::Forked {
            prefix: accepted,
            detached_prior,
            current: ChunkSet::default(),
        };
        self.rebuild_resolvers();
        Ok(())
    }

    pub fn reject_checkpoint_candidate(
        &mut self,
        boundary: SealedBoundary<Lane>,
    ) -> Result<(), ForkArenaError> {
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
        let released = self.release_set(current)?;
        self.counters.candidate_chunks_truncated = self
            .counters
            .candidate_chunks_truncated
            .saturating_add(released as u64);
        self.counters.accepted_chunks_reattached =
            self.counters.accepted_chunks_reattached.saturating_add(
                (detached_prior.payload.len() + detached_prior.descriptors.len()) as u64,
            );
        prefix.payload.extend(detached_prior.payload);
        prefix.descriptors.extend(detached_prior.descriptors);
        self.ownership = ForkOwnership::Accepted(prefix);
        self.rebuild_resolvers();
        Ok(())
    }

    pub fn accept_checkpoint_candidate(
        &mut self,
        boundary: SealedBoundary<Lane>,
    ) -> Result<(), ForkArenaError> {
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
        let pruned = self.release_set(detached_prior)?;
        self.counters.obsolete_chunks_pruned = self
            .counters
            .obsolete_chunks_pruned
            .saturating_add(pruned as u64);
        prefix.payload.extend(current.payload);
        prefix.descriptors.extend(current.descriptors);
        self.ownership = ForkOwnership::Accepted(prefix);
        self.rebuild_resolvers();
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

    fn release_set(&mut self, set: ChunkSet) -> Result<usize, ForkArenaError> {
        let count = set.payload.len() + set.descriptors.len();
        let mut pools = self.pools.borrow_mut();
        for key in set.payload {
            pools.payload.release(key, self.owner)?;
        }
        for key in set.descriptors {
            pools.descriptors.release(key, self.owner)?;
        }
        Ok(count)
    }

    pub fn begin_batch(&mut self) -> Result<BatchMark<Lane>, ForkArenaError> {
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let boundary = self.seal_boundary()?;
        Ok(BatchMark {
            arena: self.owner,
            payload_start: boundary.payload_chunks,
            descriptor_start: boundary.descriptor_chunks,
            _lane: PhantomData,
        })
    }

    pub fn seal_batch(
        &mut self,
        mark: BatchMark<Lane>,
        lists: Vec<ArenaListId<Lane>>,
    ) -> Result<SealedBatch<Lane>, ForkArenaError> {
        if mark.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        let boundary = self.seal_boundary()?;
        for list in &lists {
            self.validate_list_in_suffix(
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

    pub fn promote_batch_into<Destination>(
        &mut self,
        destination: &mut ForkArena<T, Destination>,
        batch: SealedBatch<Lane>,
    ) -> Result<Vec<ArenaListId<Destination>>, ForkArenaError> {
        if batch.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        if !Rc::ptr_eq(&self.pools, &destination.pools) {
            return Err(ForkArenaError::ForeignPool);
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
        {
            let pools = self.pools.borrow();
            for key in &payload {
                if !pools.payload.is_sealed(*key, self.owner)? {
                    return Err(ForkArenaError::UnsealedBoundary);
                }
            }
            for key in &descriptors {
                if !pools.descriptors.is_sealed(*key, self.owner)? {
                    return Err(ForkArenaError::UnsealedBoundary);
                }
            }
        }
        destination.seal_boundary()?;
        let payload = self.detach_suffix(false, batch.payload_start as usize)?;
        let descriptors = self.detach_suffix(true, batch.descriptor_start as usize)?;
        let promoted_lists = batch
            .lists
            .into_iter()
            .map(|list| rebrand_list(list, destination.owner))
            .collect::<Vec<_>>();
        {
            let mut pools = self.pools.borrow_mut();
            for key in &payload {
                pools
                    .payload
                    .transfer(*key, self.owner, destination.owner)?;
            }
            for key in &descriptors {
                pools
                    .descriptors
                    .transfer(*key, self.owner, destination.owner)?;
            }
        }
        let promoted = payload.len() + descriptors.len();
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
        self.rebuild_resolvers();
        destination.rebuild_resolvers();
        self.pending_batch = None;
        Ok(promoted_lists)
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
        let prefix_len = match &self.ownership {
            ForkOwnership::Accepted(_) => 0,
            ForkOwnership::Forked { prefix, .. } => {
                if descriptor {
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
        list: ArenaListId<Lane>,
        payload_start: usize,
        descriptor_start: usize,
    ) -> Result<(), ForkArenaError> {
        self.validate_list(list)?;
        match list {
            ArenaListId::Range(range) => {
                if let Some(first) = range.first
                    && self
                        .resolved_position(false, first.raw)
                        .is_none_or(|position| position < payload_start)
                {
                    return Err(ForkArenaError::InvalidRegion);
                }
            }
            ArenaListId::Sequence { first, .. } => {
                if self
                    .resolved_position(true, first)
                    .is_none_or(|position| position < descriptor_start)
                {
                    return Err(ForkArenaError::InvalidRegion);
                }
                for range in self.list_ranges(list)? {
                    if self
                        .resolved_position(false, range.first)
                        .is_none_or(|position| position < payload_start)
                    {
                        return Err(ForkArenaError::InvalidRegion);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn compose_lists(
        &mut self,
        lists: &[ArenaListId<Lane>],
        scratch: &mut Vec<ArenaRange<Lane>>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        scratch.clear();
        for list in lists.iter().copied() {
            self.validate_list(list)?;
            match list {
                ArenaListId::Range(range) if !range.is_empty() => scratch.push(range),
                ArenaListId::Range(_) => {}
                ArenaListId::Sequence { .. } => {
                    scratch.extend(self.list_ranges(list)?.into_iter().map(|raw| ArenaRange {
                        arena: self.owner,
                        first: Some(ChunkId {
                            arena: self.owner,
                            raw: raw.first,
                            _lane: PhantomData,
                        }),
                        start: raw.start,
                        len: raw.len,
                    }));
                }
            }
        }
        if scratch.is_empty() {
            return Ok(ArenaListId::Range(ArenaRange::empty(self.owner)));
        }
        if scratch.len() == 1 {
            return Ok(ArenaListId::Range(scratch[0]));
        }
        let mut cumulative = 0_u32;
        let mut first = None;
        let mut start = 0_u32;
        for range in scratch.iter().copied() {
            cumulative = cumulative
                .checked_add(range.len)
                .ok_or(ForkArenaError::CapacityOverflow)?;
            let raw = RawRange {
                first: range.first.ok_or(ForkArenaError::InvalidRange)?.raw,
                start: range.start,
                len: range.len,
            };
            let (key, offset) = self.append_descriptor(RangeEntry {
                range: raw,
                cumulative_end: cumulative,
            })?;
            if first.is_none() {
                first = Some(key);
                start = offset;
            }
        }
        Ok(ArenaListId::Sequence {
            arena: self.owner,
            first: first.expect("nonempty range sequence has a first descriptor"),
            start,
            count: scratch.len() as u32,
            len: cumulative,
            _lane: PhantomData,
        })
    }

    pub fn list(
        &self,
        list: ArenaListId<Lane>,
    ) -> Result<ArenaListView<'_, T, Lane>, ForkArenaError> {
        self.validate_list(list)?;
        Ok(ArenaListView {
            arena: self,
            pools: self.pools.borrow(),
            list,
        })
    }

    fn validate_list(&self, list: ArenaListId<Lane>) -> Result<(), ForkArenaError> {
        match list {
            ArenaListId::Range(range) => self.validate_range(range),
            ArenaListId::Sequence {
                arena,
                first,
                start,
                count,
                len,
                ..
            } => {
                if arena != self.owner {
                    return Err(ForkArenaError::ForeignArena);
                }
                let ranges = self.descriptor_span(first, start, count)?;
                if ranges.last().map_or(0, |entry| entry.cumulative_end) != len {
                    return Err(ForkArenaError::InvalidRange);
                }
                for entry in ranges {
                    self.validate_raw_range(entry.range)?;
                }
                Ok(())
            }
        }
    }

    fn validate_range(&self, range: ArenaRange<Lane>) -> Result<(), ForkArenaError> {
        if range.arena != self.owner {
            return Err(ForkArenaError::ForeignArena);
        }
        if range.len == 0 {
            return if range.first.is_none() {
                Ok(())
            } else {
                Err(ForkArenaError::InvalidRange)
            };
        }
        let first = range.first.ok_or(ForkArenaError::InvalidRange)?;
        if first.arena != self.owner {
            return Err(ForkArenaError::ForeignArena);
        }
        self.validate_raw_range(RawRange {
            first: first.raw,
            start: range.start,
            len: range.len,
        })
    }

    fn validate_raw_range(&self, range: RawRange) -> Result<(), ForkArenaError> {
        let pools = self.pools.borrow();
        let capacity = pools.payload.chunk_capacity();
        let first = self
            .resolved_position(false, range.first)
            .ok_or(ForkArenaError::InvalidRange)?;
        if range.start as usize >= capacity {
            return Err(ForkArenaError::InvalidRange);
        }
        let mut remaining = range.len as usize;
        let mut position = first;
        let mut offset = range.start as usize;
        while remaining != 0 {
            let key = self
                .live_key_at(false, position)
                .ok_or(ForkArenaError::InvalidRange)?;
            let used = pools.payload.used(key, self.owner)? as usize;
            if offset >= used {
                return Err(ForkArenaError::InvalidRange);
            }
            let available = used - offset;
            let consumed = available.min(remaining);
            remaining -= consumed;
            position += 1;
            offset = 0;
            if remaining != 0 && used != capacity {
                return Err(ForkArenaError::InvalidRange);
            }
        }
        Ok(())
    }

    fn descriptor_span(
        &self,
        first: RawChunkKey,
        start: u32,
        count: u32,
    ) -> Result<Vec<RangeEntry>, ForkArenaError> {
        let pools = self.pools.borrow();
        let capacity = pools.descriptors.chunk_capacity();
        let mut position = self
            .resolved_position(true, first)
            .ok_or(ForkArenaError::InvalidRange)?;
        let mut offset = start as usize;
        let mut remaining = count as usize;
        let mut out = Vec::with_capacity(remaining);
        while remaining != 0 {
            let key = self
                .live_key_at(true, position)
                .ok_or(ForkArenaError::InvalidRange)?;
            let used = pools.descriptors.used(key, self.owner)? as usize;
            while offset < used && remaining != 0 {
                out.push(
                    *pools
                        .descriptors
                        .get(key, self.owner, offset as u32)
                        .ok_or(ForkArenaError::InvalidRange)?,
                );
                offset += 1;
                remaining -= 1;
            }
            if remaining != 0 && used != capacity {
                return Err(ForkArenaError::InvalidRange);
            }
            position += 1;
            offset = 0;
        }
        Ok(out)
    }

    fn list_ranges(&self, list: ArenaListId<Lane>) -> Result<Vec<RawRange>, ForkArenaError> {
        match list {
            ArenaListId::Range(range) => Ok(range
                .first
                .map(|first| RawRange {
                    first: first.raw,
                    start: range.start,
                    len: range.len,
                })
                .into_iter()
                .collect()),
            ArenaListId::Sequence {
                first,
                start,
                count,
                ..
            } => Ok(self
                .descriptor_span(first, start, count)?
                .into_iter()
                .map(|entry| entry.range)
                .collect()),
        }
    }
}

fn rebrand_list<Source, Destination>(
    list: ArenaListId<Source>,
    arena: u64,
) -> ArenaListId<Destination> {
    match list {
        ArenaListId::Range(range) => ArenaListId::Range(ArenaRange {
            arena,
            first: range.first.map(|first| ChunkId {
                arena,
                raw: first.raw,
                _lane: PhantomData,
            }),
            start: range.start,
            len: range.len,
        }),
        ArenaListId::Sequence {
            first,
            start,
            count,
            len,
            ..
        } => ArenaListId::Sequence {
            arena,
            first,
            start,
            count,
            len,
            _lane: PhantomData,
        },
    }
}

/// Exclusive operation builder. Dropping it rolls back its partial suffix.
#[must_use = "a fork-arena builder must be sealed or explicitly discarded"]
pub struct ForkArenaBuilder<'a, T, Lane> {
    arena: &'a mut ForkArena<T, Lane>,
    operation: OperationMark<Lane>,
    first: Option<(RawChunkKey, u32)>,
    len: u32,
    finished: bool,
}

impl<T, Lane> ForkArenaBuilder<'_, T, Lane> {
    pub fn push(&mut self, value: T) -> Result<(), ForkArenaError> {
        let coordinate = self.arena.append_payload(value)?;
        self.first.get_or_insert(coordinate);
        self.len = self
            .len
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        Ok(())
    }

    pub fn seal(mut self) -> Result<ArenaListId<Lane>, ForkArenaError> {
        let range = match self.first {
            Some((first, start)) => ArenaRange {
                arena: self.arena.owner,
                first: Some(ChunkId {
                    arena: self.arena.owner,
                    raw: first,
                    _lane: PhantomData,
                }),
                start,
                len: self.len,
            },
            None => ArenaRange::empty(self.arena.owner),
        };
        self.arena.active_builder = false;
        self.finished = true;
        Ok(ArenaListId::Range(range))
    }

    pub fn discard(mut self) -> Result<(), ForkArenaError> {
        self.arena.active_builder = false;
        self.finished = true;
        self.arena.restore_operation(self.operation)
    }
}

impl<T, Lane> Drop for ForkArenaBuilder<'_, T, Lane> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.arena.active_builder = false;
        self.arena
            .restore_operation(self.operation)
            .expect("builder rollback mark belongs to its arena");
    }
}

/// Borrowed indexed view of one canonical arena list.
pub struct ArenaListView<'a, T, Lane> {
    arena: &'a ForkArena<T, Lane>,
    pools: Ref<'a, ForkPools<T>>,
    list: ArenaListId<Lane>,
}

impl<T, Lane> core::fmt::Debug for ArenaListView<'_, T, Lane> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArenaListView")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl<T, Lane> ArenaListView<'_, T, Lane> {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len() {
            return None;
        }
        let (range, local) = match self.list {
            ArenaListId::Range(range) => (range, index),
            ArenaListId::Sequence {
                first,
                start,
                count,
                ..
            } => {
                let mut low = 0_u32;
                let mut high = count;
                while low < high {
                    let middle = low + (high - low) / 2;
                    let entry = self.descriptor_at(first, start, middle)?;
                    if index < entry.cumulative_end as usize {
                        high = middle;
                    } else {
                        low = middle + 1;
                    }
                }
                let entry = self.descriptor_at(first, start, low)?;
                let prior = if low == 0 {
                    0
                } else {
                    self.descriptor_at(first, start, low - 1)?.cumulative_end as usize
                };
                (
                    ArenaRange {
                        arena: self.arena.owner,
                        first: Some(ChunkId {
                            arena: self.arena.owner,
                            raw: entry.range.first,
                            _lane: PhantomData,
                        }),
                        start: entry.range.start,
                        len: entry.range.len,
                    },
                    index - prior,
                )
            }
        };
        self.range_get(range, local)
    }

    fn descriptor_at(&self, first: RawChunkKey, start: u32, index: u32) -> Option<RangeEntry> {
        let capacity = self.pools.descriptors.chunk_capacity();
        let absolute = start as usize + index as usize;
        let chunk_delta = absolute / capacity;
        let offset = absolute % capacity;
        let first_position = self.arena.resolved_position(true, first)?;
        let key = self.arena.live_key_at(true, first_position + chunk_delta)?;
        self.pools
            .descriptors
            .get(key, self.arena.owner, offset as u32)
            .copied()
    }

    fn range_get(&self, range: ArenaRange<Lane>, index: usize) -> Option<&T> {
        let first = range.first?;
        let capacity = self.pools.payload.chunk_capacity();
        let absolute = range.start as usize + index;
        let chunk_delta = absolute / capacity;
        let offset = absolute % capacity;
        let first_position = self.arena.resolved_position(false, first.raw)?;
        let key = self
            .arena
            .live_key_at(false, first_position + chunk_delta)?;
        self.pools.payload.get(key, self.arena.owner, offset as u32)
    }

    pub fn iter(&self) -> ArenaListIter<'_, '_, T, Lane> {
        ArenaListIter {
            view: self,
            position: 0,
        }
    }
}

pub struct ArenaListIter<'view, 'arena, T, Lane> {
    view: &'view ArenaListView<'arena, T, Lane>,
    position: usize,
}

impl<'view, 'arena, T, Lane> Iterator for ArenaListIter<'view, 'arena, T, Lane> {
    type Item = &'view T;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.view.get(self.position)?;
        self.position += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.view.len().saturating_sub(self.position);
        (remaining, Some(remaining))
    }
}

impl<T, Lane> ExactSizeIterator for ArenaListIter<'_, '_, T, Lane> {}
