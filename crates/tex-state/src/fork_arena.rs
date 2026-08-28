//! Fixed-chunk storage with one accepted lineage and one transactional fork.
//!
//! Payload lives in coarse pool pages. Arenas own only stable chunk keys and
//! canonical, non-recursive range-list descriptors. Retained checkpoints land
//! on sealed whole-chunk boundaries; operation marks may additionally name a
//! partially used tail.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(test)]
#[path = "fork_arena/tests.rs"]
mod tests;

const DEFAULT_CHUNK_BYTES: usize = 4 * 1024;
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
struct ChunkStorage<T> {
    chunk_bytes: usize,
    slots_per_chunk: usize,
    pages: Vec<ChunkPage<T>>,
    chunks: Vec<ChunkMeta>,
    free: Vec<u32>,
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
        Ok(RawChunkKey {
            slot,
            generation: meta.generation,
        })
    }

    fn validate(&self, key: RawChunkKey, arena: u32) -> Result<&ChunkMeta, ForkArenaError> {
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

    fn append(&mut self, key: RawChunkKey, arena: u32, value: T) -> Result<u32, ForkArenaError> {
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

    fn get(&self, key: RawChunkKey, arena: u32, offset: u32) -> Option<&T> {
        let meta = self.validate(key, arena).ok()?;
        if offset >= meta.used {
            return None;
        }
        let (page, index) = self.slot_index(key, offset as usize).ok()?;
        self.pages[page].slots[index].as_ref()
    }

    fn used(&self, key: RawChunkKey, arena: u32) -> Result<u32, ForkArenaError> {
        Ok(self.validate(key, arena)?.used)
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

    fn truncate(&mut self, key: RawChunkKey, arena: u32, used: u32) -> Result<(), ForkArenaError> {
        let old_used = self.validate(key, arena)?.used;
        if used > old_used {
            return Err(ForkArenaError::InvalidOperationMark);
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

/// The single caller-owned physical allocation pool shared by typed arenas.
///
/// Holding `&ChunkPool` permits stable direct payload borrows. Every append,
/// seal, transfer, rollback, and prune requires `&mut ChunkPool`, making the
/// physical mutation gate explicit and exclusive.
pub struct ChunkPool<T> {
    owner: u32,
    payload: ChunkStorage<T>,
    descriptors: ChunkStorage<RangeEntry>,
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
}

/// A generation-checked chunk coordinate branded for one semantic lane.
pub struct ChunkId<Lane> {
    arena: u32,
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
    arena: u32,
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
    const fn empty(arena: u32) -> Self {
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

/// Fixed coordinate of one canonical arena-owned nonrecursive list record.
///
/// Every nonempty list, including a single direct range, publishes one or
/// more consecutive `RangeEntry` records. The coordinate therefore stays
/// compact while the arena record remains the sole list topology.
pub struct ArenaListId<Lane> {
    arena: u32,
    first: RawChunkKey,
    start: u32,
    count: u32,
    len: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
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
            && self.first == other.first
            && self.start == other.start
            && self.count == other.count
            && self.len == other.len
    }
}
impl<Lane> Eq for ArenaListId<Lane> {}

impl<Lane> Hash for ArenaListId<Lane> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.arena.hash(state);
        self.first.hash(state);
        self.start.hash(state);
        self.count.hash(state);
        self.len.hash(state);
    }
}

impl<Lane> ArenaListId<Lane> {
    /// Returns the owner-independent canonical empty-list coordinate.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            arena: 0,
            first: RawChunkKey {
                slot: 0,
                generation: 0,
            },
            start: 0,
            count: 0,
            len: 0,
            _lane: PhantomData,
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

    const fn from_record(arena: u32, first: RawChunkKey, start: u32, count: u32, len: u32) -> Self {
        Self {
            arena,
            first,
            start,
            count,
            len,
            _lane: PhantomData,
        }
    }
}

const _: () = assert!(core::mem::size_of::<ArenaListId<PageMaterialLane>>() <= 24);

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

pub struct BatchMark<Lane> {
    arena: u32,
    payload_start: u32,
    descriptor_start: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

/// Lifecycle work counters; payload copy is absent by construction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ForkArenaCounters {
    pub new_semantic_nodes: u64,
    pub source_nodes_copied: u64,
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
    InvalidCheckpoint,
    InvalidChunk,
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
    Sealed(ArenaListId<Lane>),
}

struct OpenActiveList<Lane> {
    arena: u32,
    operation: OperationMark<Lane>,
    pending: Option<ArenaRange<Lane>>,
    pending_extendable: bool,
    descriptor_first: Option<(RawChunkKey, u32)>,
    descriptor_count: u32,
    len: u32,
}

impl<T, Lane> Default for ActiveListBuilder<T, Lane> {
    fn default() -> Self {
        Self::vacant()
    }
}

impl<T, Lane> ActiveListBuilder<T, Lane> {
    #[must_use]
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
        let ActiveListBuilderState::Sealed(list) = self.state else {
            return Err(ForkArenaError::InvalidActiveListBuilder);
        };
        self.state = ActiveListBuilderState::Vacant;
        Ok(list)
    }
}

/// One typed arena lane containing coordinates and lifecycle metadata only.
pub struct ForkArena<T, Lane> {
    owner: u32,
    pool_owner: Option<u32>,
    ownership: ForkOwnership,
    active_builder: bool,
    pending_batch: Option<PendingBatch>,
    next_batch_serial: u64,
    payload_resolver: Vec<(RawChunkKey, usize)>,
    descriptor_resolver: Vec<(RawChunkKey, usize)>,
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
            active_builder: false,
            pending_batch: None,
            next_batch_serial: 1,
            payload_resolver: Vec::new(),
            descriptor_resolver: Vec::new(),
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
            active_builder: false,
            pending_batch: None,
            next_batch_serial: 1,
            payload_resolver: Vec::new(),
            descriptor_resolver: Vec::new(),
            counters: ForkArenaCounters::default(),
            _types: PhantomData,
        }
    }

    #[must_use]
    pub const fn counters(&self) -> ForkArenaCounters {
        self.counters
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

    fn allocate_chunk(
        &mut self,
        pool: &mut ChunkPool<T>,
        descriptor: bool,
    ) -> Result<RawChunkKey, ForkArenaError> {
        self.bind_pool(pool)?;
        let key = if descriptor {
            pool.descriptors.allocate(self.owner)?
        } else {
            pool.payload.allocate(self.owner)?
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

    fn append_payload(
        &mut self,
        pool: &mut ChunkPool<T>,
        value: T,
    ) -> Result<(RawChunkKey, u32), ForkArenaError> {
        let last = self
            .current_chunks_mut()
            .payload
            .last()
            .copied()
            .filter(|key| {
                !pool.payload.is_sealed(*key, self.owner).unwrap_or(true)
                    && pool
                        .payload
                        .used(*key, self.owner)
                        .ok()
                        .is_some_and(|used| used as usize != pool.payload.chunk_capacity())
            });
        let key = match last {
            Some(key) => key,
            None => self.allocate_chunk(pool, false)?,
        };
        let offset = pool.payload.append(key, self.owner, value)?;
        let became_full =
            pool.payload.used(key, self.owner)? as usize == pool.payload.chunk_capacity();
        if became_full {
            pool.payload.seal(key, self.owner)?;
            self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        }
        self.counters.new_semantic_nodes += 1;
        Ok((key, offset))
    }

    fn append_descriptor(
        &mut self,
        pool: &mut ChunkPool<T>,
        entry: RangeEntry,
    ) -> Result<(RawChunkKey, u32), ForkArenaError> {
        let last = self
            .current_chunks_mut()
            .descriptors
            .last()
            .copied()
            .filter(|key| {
                !pool.descriptors.is_sealed(*key, self.owner).unwrap_or(true)
                    && pool
                        .descriptors
                        .used(*key, self.owner)
                        .ok()
                        .is_some_and(|used| used as usize != pool.descriptors.chunk_capacity())
            });
        let key = match last {
            Some(key) => key,
            None => self.allocate_chunk(pool, true)?,
        };
        let offset = pool.descriptors.append(key, self.owner, entry)?;
        let became_full =
            pool.descriptors.used(key, self.owner)? as usize == pool.descriptors.chunk_capacity();
        if became_full {
            pool.descriptors.seal(key, self.owner)?;
            self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        }
        Ok((key, offset))
    }

    #[must_use]
    pub fn operation_mark(&self, pool: &ChunkPool<T>) -> OperationMark<Lane> {
        let payload_tail_used = self
            .live_key_at(false, self.live_payload_len().saturating_sub(1))
            .and_then(|key| pool.payload.used(key, self.owner).ok())
            .unwrap_or(0);
        let descriptor_tail_used = self
            .live_key_at(true, self.live_descriptor_len().saturating_sub(1))
            .and_then(|key| pool.descriptors.used(key, self.owner).ok())
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
        )?;
        self.truncate_lane(
            pool,
            true,
            mark.descriptor_chunks as usize,
            mark.descriptor_tail_used,
        )?;
        self.rebuild_resolvers();
        Ok(())
    }

    fn truncate_lane(
        &mut self,
        pool: &mut ChunkPool<T>,
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
            if descriptor {
                pool.descriptors.release(key, self.owner)?;
            } else {
                pool.payload.release(key, self.owner)?;
            }
            self.counters.candidate_chunks_truncated += 1;
        }
        if chunks != 0 {
            let key = self
                .live_key_at(descriptor, chunks - 1)
                .ok_or(ForkArenaError::InvalidOperationMark)?;
            if descriptor {
                pool.descriptors.truncate(key, self.owner, tail_used)?;
            } else {
                pool.payload.truncate(key, self.owner, tail_used)?;
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
            first: None,
            len: 0,
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
            pending: None,
            pending_extendable: false,
            descriptor_first: None,
            descriptor_count: 0,
            len: 0,
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

    fn flush_active_list_pending(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
    ) -> Result<(), ForkArenaError> {
        let (pending, cumulative_end) = {
            let open = self.active_list_open_mut(builder)?;
            (open.pending.take(), open.len)
        };
        let Some(range) = pending else {
            return Ok(());
        };
        let raw = RawRange {
            first: range.first.ok_or(ForkArenaError::InvalidRange)?.raw,
            start: range.start,
            len: range.len,
        };
        let coordinate = self.append_descriptor(
            pool,
            RangeEntry {
                range: raw,
                cumulative_end,
            },
        )?;
        let open = self.active_list_open_mut(builder)?;
        open.descriptor_first.get_or_insert(coordinate);
        open.descriptor_count = open
            .descriptor_count
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        open.pending_extendable = false;
        Ok(())
    }

    /// Appends one newly created semantic payload to an open active list.
    pub fn push_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        value: T,
    ) -> Result<(), ForkArenaError> {
        let should_flush = {
            let open = self.active_list_open_mut(builder)?;
            open.pending.is_some() && !open.pending_extendable
        };
        if should_flush {
            self.flush_active_list_pending(pool, builder)?;
        }
        let (first, start) = self.append_payload(pool, value)?;
        let open = self.active_list_open_mut(builder)?;
        open.len = open
            .len
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        if open.pending_extendable {
            let pending = open
                .pending
                .as_mut()
                .expect("extendable active-list tail has a range");
            pending.len = pending
                .len
                .checked_add(1)
                .ok_or(ForkArenaError::CapacityOverflow)?;
        } else {
            open.pending = Some(ArenaRange {
                arena: self.owner,
                first: Some(ChunkId {
                    arena: self.owner,
                    raw: first,
                    _lane: PhantomData,
                }),
                start,
                len: 1,
            });
            open.pending_extendable = true;
        }
        Ok(())
    }

    fn append_active_range(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        range: ArenaRange<Lane>,
    ) -> Result<(), ForkArenaError> {
        if range.is_empty() {
            return Ok(());
        }
        self.validate_range(pool, range)?;
        if self.active_list_open_mut(builder)?.pending.is_some() {
            self.flush_active_list_pending(pool, builder)?;
        }
        let open = self.active_list_open_mut(builder)?;
        open.len = open
            .len
            .checked_add(range.len)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        open.pending = Some(range);
        open.pending_extendable = false;
        Ok(())
    }

    /// Appends an existing immutable list by coordinates only.
    pub fn append_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.active_list_open_mut(builder)?;
        self.validate_list(pool, list)?;
        for index in 0..list.count {
            let raw = self
                .descriptor_entry(pool, list.first, list.start, index)?
                .range;
            self.append_active_range(
                pool,
                builder,
                ArenaRange {
                    arena: self.owner,
                    first: Some(ChunkId {
                        arena: self.owner,
                        raw: raw.first,
                        _lane: PhantomData,
                    }),
                    start: raw.start,
                    len: raw.len,
                },
            )?;
        }
        Ok(())
    }

    /// Finalizes the active list into the canonical direct-range or flat
    /// range-sequence coordinate. Payload is never materialized.
    pub fn finalize_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
    ) -> Result<(), ForkArenaError> {
        let (descriptor_count, pending, len) = {
            let open = self.active_list_open_mut(builder)?;
            (open.descriptor_count, open.pending, open.len)
        };
        if descriptor_count == 0 && pending.is_none() {
            self.active_builder = false;
            builder.state = ActiveListBuilderState::Sealed(ArenaListId::empty());
            return Ok(());
        }
        self.flush_active_list_pending(pool, builder)?;
        let open = self.active_list_open_mut(builder)?;
        let (first, start) = open
            .descriptor_first
            .ok_or(ForkArenaError::InvalidActiveListBuilder)?;
        let list = ArenaListId::from_record(self.owner, first, start, open.descriptor_count, len);
        self.validate_list(pool, list)?;
        self.active_builder = false;
        builder.state = ActiveListBuilderState::Sealed(list);
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
        scratch: &mut Vec<ArenaRange<Lane>>,
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

    pub fn seal_boundary(
        &mut self,
        pool: &mut ChunkPool<T>,
    ) -> Result<SealedBoundary<Lane>, ForkArenaError> {
        self.bind_pool(pool)?;
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

    pub fn validates_checkpoint(&self, mark: CheckpointMark<Lane>) -> bool {
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

    pub fn visit_checkpoint_values(
        &self,
        pool: &ChunkPool<T>,
        mark: CheckpointMark<Lane>,
        mut visit: impl FnMut(&T),
    ) -> Result<(), ForkArenaError> {
        if !self.validates_checkpoint(mark) {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        for position in 0..mark.payload_chunks as usize {
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
        prefix.payload.extend(detached_prior.payload);
        prefix.descriptors.extend(detached_prior.descriptors);
        self.ownership = ForkOwnership::Accepted(prefix);
        self.rebuild_resolvers();
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

    fn release_set(
        &mut self,
        pool: &mut ChunkPool<T>,
        set: ChunkSet,
    ) -> Result<usize, ForkArenaError> {
        let count = set.payload.len() + set.descriptors.len();
        for key in set.payload {
            pool.payload.release(key, self.owner)?;
        }
        for key in set.descriptors {
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

    pub fn promote_batch_into<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        batch: SealedBatch<Lane>,
    ) -> Result<Vec<ArenaListId<Destination>>, ForkArenaError> {
        self.bind_pool(pool)?;
        destination.bind_pool(pool)?;
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
        destination.seal_boundary(pool)?;
        let payload = self.detach_suffix(false, batch.payload_start as usize)?;
        let descriptors = self.detach_suffix(true, batch.descriptor_start as usize)?;
        let promoted_lists = batch
            .lists
            .into_iter()
            .map(|list| rebrand_list(list, destination.owner))
            .collect::<Vec<_>>();
        for key in &payload {
            pool.payload.transfer(*key, self.owner, destination.owner)?;
        }
        for key in &descriptors {
            pool.descriptors
                .transfer(*key, self.owner, destination.owner)?;
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
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        payload_start: usize,
        descriptor_start: usize,
    ) -> Result<(), ForkArenaError> {
        self.validate_list(pool, list)?;
        if list.is_empty() {
            return Ok(());
        }
        if self
            .resolved_position(true, list.first)
            .is_none_or(|position| position < descriptor_start)
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        for index in 0..list.count {
            let range = self
                .descriptor_entry(pool, list.first, list.start, index)?
                .range;
            if self
                .resolved_position(false, range.first)
                .is_none_or(|position| position < payload_start)
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    pub fn compose_lists(
        &mut self,
        pool: &mut ChunkPool<T>,
        lists: &[ArenaListId<Lane>],
        scratch: &mut Vec<ArenaRange<Lane>>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        scratch.clear();
        for list in lists.iter().copied() {
            self.validate_list(pool, list)?;
            self.append_list_ranges(pool, list, scratch)?;
        }
        self.compose_ranges(pool, scratch)
    }

    /// Returns the empty canonical list for this arena lane.
    #[must_use]
    pub const fn empty_list(&self) -> ArenaListId<Lane> {
        ArenaListId::empty()
    }

    /// Selects one logical subrange without copying payload values.
    ///
    /// `scratch` is caller-owned scalar descriptor storage and is cleared
    /// before use. A discontiguous result is recorded once in the arena's
    /// canonical descriptor lane.
    pub fn slice_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        list: ArenaListId<Lane>,
        selected: Range<usize>,
        scratch: &mut Vec<ArenaRange<Lane>>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        self.validate_list(pool, list)?;
        if selected.start > selected.end || selected.end > list.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        scratch.clear();
        if selected.is_empty() {
            return Ok(self.empty_list());
        }
        let mut prior_end = 0_usize;
        for index in 0..list.count {
            let entry = self.descriptor_entry(pool, list.first, list.start, index)?;
            let entry_end = entry.cumulative_end as usize;
            let overlap_start = selected.start.max(prior_end);
            let overlap_end = selected.end.min(entry_end);
            if overlap_start < overlap_end {
                let range = ArenaRange {
                    arena: self.owner,
                    first: Some(ChunkId {
                        arena: self.owner,
                        raw: entry.range.first,
                        _lane: PhantomData,
                    }),
                    start: entry.range.start,
                    len: entry.range.len,
                };
                scratch.push(self.slice_range(
                    pool,
                    range,
                    overlap_start - prior_end,
                    overlap_end - overlap_start,
                )?);
            }
            prior_end = entry_end;
            if prior_end >= selected.end {
                break;
            }
        }
        self.compose_ranges(pool, scratch)
    }

    fn compose_ranges(
        &mut self,
        pool: &mut ChunkPool<T>,
        scratch: &[ArenaRange<Lane>],
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        if scratch.is_empty() {
            return Ok(self.empty_list());
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
            let (key, offset) = self.append_descriptor(
                pool,
                RangeEntry {
                    range: raw,
                    cumulative_end: cumulative,
                },
            )?;
            if first.is_none() {
                first = Some(key);
                start = offset;
            }
        }
        Ok(ArenaListId::from_record(
            self.owner,
            first.expect("nonempty range sequence has a first descriptor"),
            start,
            u32::try_from(scratch.len()).map_err(|_| ForkArenaError::CapacityOverflow)?,
            cumulative,
        ))
    }

    fn slice_range(
        &self,
        pool: &ChunkPool<T>,
        range: ArenaRange<Lane>,
        start: usize,
        len: usize,
    ) -> Result<ArenaRange<Lane>, ForkArenaError> {
        if start.checked_add(len).is_none_or(|end| end > range.len()) {
            return Err(ForkArenaError::InvalidRange);
        }
        if len == 0 {
            return Ok(ArenaRange::empty(self.owner));
        }
        let first = range.first.ok_or(ForkArenaError::InvalidRange)?;
        let capacity = pool.payload.chunk_capacity();
        let absolute = range.start as usize + start;
        let first_position = self
            .resolved_position(false, first.raw)
            .ok_or(ForkArenaError::InvalidRange)?;
        let raw = self
            .live_key_at(false, first_position + absolute / capacity)
            .ok_or(ForkArenaError::InvalidRange)?;
        Ok(ArenaRange {
            arena: self.owner,
            first: Some(ChunkId {
                arena: self.owner,
                raw,
                _lane: PhantomData,
            }),
            start: (absolute % capacity) as u32,
            len: u32::try_from(len).map_err(|_| ForkArenaError::CapacityOverflow)?,
        })
    }

    pub fn list<'a>(
        &'a self,
        pool: &'a ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<ArenaListView<'a, T, Lane>, ForkArenaError> {
        self.validate_list(pool, list)?;
        Ok(ArenaListView {
            arena: self,
            pool,
            list,
        })
    }

    fn validate_list(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        if list.is_empty() {
            return (list == ArenaListId::empty())
                .then_some(())
                .ok_or(ForkArenaError::InvalidRange);
        }
        if list.arena != self.owner || list.count == 0 {
            return Err(ForkArenaError::ForeignArena);
        }
        let mut cumulative_end = 0;
        for index in 0..list.count {
            let entry = self.descriptor_entry(pool, list.first, list.start, index)?;
            cumulative_end = entry.cumulative_end;
            self.validate_raw_range(pool, entry.range)?;
        }
        if cumulative_end != list.len {
            return Err(ForkArenaError::InvalidRange);
        }
        Ok(())
    }

    fn validate_range(
        &self,
        pool: &ChunkPool<T>,
        range: ArenaRange<Lane>,
    ) -> Result<(), ForkArenaError> {
        if range.len == 0 && range.first.is_none() {
            return Ok(());
        }
        if range.arena != self.owner {
            return Err(ForkArenaError::ForeignArena);
        }
        if range.len == 0 {
            return Err(ForkArenaError::InvalidRange);
        }
        let first = range.first.ok_or(ForkArenaError::InvalidRange)?;
        if first.arena != self.owner {
            return Err(ForkArenaError::ForeignArena);
        }
        self.validate_raw_range(
            pool,
            RawRange {
                first: first.raw,
                start: range.start,
                len: range.len,
            },
        )
    }

    fn validate_raw_range(
        &self,
        pool: &ChunkPool<T>,
        range: RawRange,
    ) -> Result<(), ForkArenaError> {
        let capacity = pool.payload.chunk_capacity();
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
            let used = pool.payload.used(key, self.owner)? as usize;
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

    fn descriptor_entry(
        &self,
        pool: &ChunkPool<T>,
        first: RawChunkKey,
        start: u32,
        index: u32,
    ) -> Result<RangeEntry, ForkArenaError> {
        let capacity = pool.descriptors.chunk_capacity();
        let first_position = self
            .resolved_position(true, first)
            .ok_or(ForkArenaError::InvalidRange)?;
        let absolute = start as usize + index as usize;
        let key = self
            .live_key_at(true, first_position + absolute / capacity)
            .ok_or(ForkArenaError::InvalidRange)?;
        pool.descriptors
            .get(key, self.owner, (absolute % capacity) as u32)
            .copied()
            .ok_or(ForkArenaError::InvalidRange)
    }

    fn append_list_ranges(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        scratch: &mut Vec<ArenaRange<Lane>>,
    ) -> Result<(), ForkArenaError> {
        for index in 0..list.count {
            let raw = self
                .descriptor_entry(pool, list.first, list.start, index)?
                .range;
            scratch.push(ArenaRange {
                arena: self.owner,
                first: Some(ChunkId {
                    arena: self.owner,
                    raw: raw.first,
                    _lane: PhantomData,
                }),
                start: raw.start,
                len: raw.len,
            });
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
        ArenaListId::from_record(arena, list.first, list.start, list.count, list.len)
    }
}

/// Exclusive operation builder. Dropping it rolls back its partial suffix.
#[must_use = "a fork-arena builder must be sealed or explicitly discarded"]
pub struct ForkArenaBuilder<'a, T, Lane> {
    arena: &'a mut ForkArena<T, Lane>,
    pool: &'a mut ChunkPool<T>,
    operation: OperationMark<Lane>,
    first: Option<(RawChunkKey, u32)>,
    len: u32,
    finished: bool,
}

impl<T, Lane> ForkArenaBuilder<'_, T, Lane> {
    pub fn push(&mut self, value: T) -> Result<(), ForkArenaError> {
        let coordinate = self.arena.append_payload(self.pool, value)?;
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
        let list = if range.is_empty() {
            ArenaListId::empty()
        } else {
            self.arena
                .compose_ranges(self.pool, core::slice::from_ref(&range))?
        };
        self.arena.active_builder = false;
        self.finished = true;
        Ok(list)
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

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&'a T> {
        if index >= self.len() {
            return None;
        }
        let mut low = 0_u32;
        let mut high = self.list.count;
        while low < high {
            let middle = low + (high - low) / 2;
            let entry = self.descriptor_at(self.list.first, self.list.start, middle)?;
            if index < entry.cumulative_end as usize {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        let entry = self.descriptor_at(self.list.first, self.list.start, low)?;
        let prior = if low == 0 {
            0
        } else {
            self.descriptor_at(self.list.first, self.list.start, low - 1)?
                .cumulative_end as usize
        };
        let range = ArenaRange {
            arena: self.arena.owner,
            first: Some(ChunkId {
                arena: self.arena.owner,
                raw: entry.range.first,
                _lane: PhantomData,
            }),
            start: entry.range.start,
            len: entry.range.len,
        };
        let local = index - prior;
        self.range_get(range, local)
    }

    fn descriptor_at(&self, first: RawChunkKey, start: u32, index: u32) -> Option<RangeEntry> {
        let capacity = self.pool.descriptors.chunk_capacity();
        let absolute = start as usize + index as usize;
        let chunk_delta = absolute / capacity;
        let offset = absolute % capacity;
        let first_position = self.arena.resolved_position(true, first)?;
        let key = self.arena.live_key_at(true, first_position + chunk_delta)?;
        self.pool
            .descriptors
            .get(key, self.arena.owner, offset as u32)
            .copied()
    }

    fn range_get(&self, range: ArenaRange<Lane>, index: usize) -> Option<&'a T> {
        let first = range.first?;
        let capacity = self.pool.payload.chunk_capacity();
        let absolute = range.start as usize + index;
        let chunk_delta = absolute / capacity;
        let offset = absolute % capacity;
        let first_position = self.arena.resolved_position(false, first.raw)?;
        let key = self
            .arena
            .live_key_at(false, first_position + chunk_delta)?;
        self.pool.payload.get(key, self.arena.owner, offset as u32)
    }

    pub fn iter(&self) -> ArenaListIter<'_, 'a, T, Lane> {
        ArenaListIter {
            view: self,
            front: 0,
            back: self.len(),
        }
    }
}

pub struct ArenaListIter<'view, 'arena, T, Lane> {
    view: &'view ArenaListView<'arena, T, Lane>,
    front: usize,
    back: usize,
}

impl<'view, 'arena, T, Lane> Iterator for ArenaListIter<'view, 'arena, T, Lane> {
    type Item = &'arena T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let value = self.view.get(self.front)?;
        self.front += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}

impl<'view, 'arena, T, Lane> DoubleEndedIterator for ArenaListIter<'view, 'arena, T, Lane> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.view.get(self.back)
    }
}

impl<T, Lane> ExactSizeIterator for ArenaListIter<'_, '_, T, Lane> {}
