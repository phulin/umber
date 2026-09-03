//! Fixed-chunk storage with one accepted lineage and one transactional fork.
//!
//! Payload lives in coarse pool pages subdivided into packed logical list
//! blocks. Direct roots carry head/tail cursors and length; reverse traversal
//! follows block metadata directly. Retained checkpoints
//! land on sealed whole-block boundaries, while operation marks may name the
//! private current tail's used cursor. Each block's owner-relative position
//! lives beside that block's pool metadata, so short-lived regions never
//! materialize sparse indexes up to a process-global pool slot.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering};
use tex_dense_arena::{AcceptedBlockTable, LogicalBlockId as DenseLogicalBlockId, LogicalPosition};
use tex_dense_prefix::Superblock;

use crate::node_sequence::SemanticSequenceIdentity;

#[cfg(test)]
#[path = "fork_arena/tests.rs"]
mod tests;

const DEFAULT_CHUNK_BYTES: usize = 512;
static NEXT_POOL_OWNER: AtomicU32 = AtomicU32::new(1);
static NEXT_ARENA_OWNER: AtomicU32 = AtomicU32::new(1);
static NEXT_ARENA_LINEAGE: AtomicU32 = AtomicU32::new(1);

/// Canonical page-material lane used by execution and borrowed typesetting.
pub enum PageMaterialLane {}

/// Private identity of one exact 64 KiB initialized-prefix payload block.
///
/// Page-list topology never stores or exposes this value. Logical rows retain
/// the pool-stable coordinate while this key independently rejects reuse of a
/// physical block slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DenseBlockKey {
    slot: u32,
    incarnation: u32,
}

/// Compact block component of a pool-stable logical position.
///
/// The enclosing list stores the coordinate-space id once, allowing both
/// endpoints plus the list length to remain 32 bytes. Resolution reconstructs
/// the canonical `tex_dense_arena::LogicalPosition` before consulting the
/// selected storage view.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct LogicalChunkId {
    ordinal: u32,
    incarnation: u32,
}

impl LogicalChunkId {
    fn block(self, space: u32) -> Result<DenseLogicalBlockId, ForkArenaError> {
        DenseLogicalBlockId::from_parts(space, self.ordinal, self.incarnation)
            .ok_or(ForkArenaError::InvalidChunk)
    }
}

#[derive(Clone, Copy, Debug)]
struct LogicalChunkRow {
    incarnation: u32,
    physical_slot: u32,
    physical_incarnation: u32,
    physical_base: u32,
}

#[derive(Clone, Copy, Debug)]
struct ChunkMeta {
    generation: u32,
    arena: u32,
    // At most two page lineages may admit one immutable sealed chunk. Each
    // lineage owns its own logical position, so independently appended tails
    // cannot admit one another merely because their coordinates share an
    // arena family.
    lineages: [ChunkLineage; 2],
    used: u32,
    live: bool,
    sealed: bool,
    sequence_summary: Option<SemanticSequenceIdentity>,
    previous_in_list: Option<LogicalPosition>,
    /// Lowest owner-relative payload-chunk position named by any child list
    /// stored in this chunk. `usize::MAX` means no child coordinate.
    dependency_floor: usize,
    dependency_metadata_complete: bool,
    paired_dependency_floor: usize,
}

#[derive(Clone, Copy, Debug)]
struct ChunkLineage {
    id: u32,
    position: usize,
}

const VACANT_CHUNK_LINEAGE: ChunkLineage = ChunkLineage {
    id: 0,
    position: usize::MAX,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum ChunkStorageLayout {
    OptionalSlots,
    PackedCopy,
}

#[derive(Clone, Copy)]
pub(crate) enum NodePoolStorageClass {
    Node,
    Annex,
}

#[cfg(feature = "profiling")]
#[derive(Clone, Copy)]
pub(crate) enum NodePoolStorageEvent {
    FreshAllocation,
    ReuseAllocation,
    Release,
    DropStorage,
}

enum DenseBlockPayload<T> {
    Optional(Superblock<Option<T>>),
    Packed(Superblock<T>),
}

impl<T> DenseBlockPayload<T> {
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        match self {
            Self::Optional(block) => block.len(),
            Self::Packed(block) => block.len(),
        }
    }

    fn value(&self, index: usize) -> Option<&T> {
        match self {
            Self::Optional(block) => block.get(index)?.as_ref(),
            Self::Packed(block) => block.get(index),
        }
    }

    fn value_mut(&mut self, index: usize) -> Option<&mut T> {
        match self {
            Self::Optional(block) => block.get_mut(index)?.as_mut(),
            Self::Packed(block) => block.get_mut(index),
        }
    }

    fn optional_slot(&mut self, index: usize) -> Option<&mut Option<T>> {
        match self {
            Self::Optional(block) => block.get_mut(index),
            Self::Packed(_) => None,
        }
    }

    fn insert(&mut self, index: usize, value: T) -> Result<(), ForkArenaError> {
        match self {
            Self::Optional(block) => {
                let slot = block.get_mut(index).ok_or(ForkArenaError::InvalidRange)?;
                if slot.is_some() {
                    return Err(ForkArenaError::InvalidRange);
                }
                *slot = Some(value);
                Ok(())
            }
            Self::Packed(block) => {
                if block.len() != index {
                    return Err(ForkArenaError::InvalidRange);
                }
                block
                    .push_with(|slot| slot.insert(value))
                    .map(|_| ())
                    .map_err(|_| ForkArenaError::CapacityOverflow)
            }
        }
    }

    /// Initializes the next slot of an already-admitted destination block.
    /// Admission proved both vacancy and initialized-prefix position, so a
    /// failure here is an internal capability-construction defect rather than
    /// recoverable input.
    fn initialize_admitted(&mut self, index: usize, value: T) {
        match self {
            Self::Optional(block) => {
                let slot = block
                    .get_mut(index)
                    .expect("admitted optional destination is in bounds");
                debug_assert!(slot.is_none());
                *slot = Some(value);
            }
            Self::Packed(block) => {
                debug_assert_eq!(block.len(), index);
                block
                    .push_with(|slot| slot.insert(value))
                    .expect("admitted destination retains exact-block capacity");
            }
        }
    }

    fn truncate(&mut self, len: usize) {
        match self {
            Self::Optional(block) => block.truncate(len),
            Self::Packed(block) => block.truncate(len),
        }
    }
}

#[derive(Clone, Copy)]
enum DenseBlockSlice<'a, T> {
    Optional(&'a [Option<T>]),
    Packed(&'a [T]),
}

impl<'a, T> DenseBlockSlice<'a, T> {
    fn iter(self) -> DenseBlockIter<'a, T> {
        match self {
            Self::Optional(cells) => DenseBlockIter::Optional(cells.iter()),
            Self::Packed(cells) => DenseBlockIter::Packed(cells.iter()),
        }
    }
}

enum DenseBlockIter<'a, T> {
    Optional(core::slice::Iter<'a, Option<T>>),
    Packed(core::slice::Iter<'a, T>),
}

impl<'a, T> Iterator for DenseBlockIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Optional(iter) => iter
                .next()
                .map(|cell| cell.as_ref().expect("admitted chunk range is initialized")),
            Self::Packed(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<T> DoubleEndedIterator for DenseBlockIter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Optional(iter) => iter
                .next_back()
                .map(|cell| cell.as_ref().expect("admitted chunk range is initialized")),
            Self::Packed(iter) => iter.next_back(),
        }
    }
}

impl<T> ExactSizeIterator for DenseBlockIter<'_, T> {
    fn len(&self) -> usize {
        match self {
            Self::Optional(iter) => iter.len(),
            Self::Packed(iter) => iter.len(),
        }
    }
}

struct DenseBlock<T> {
    incarnation: u32,
    live_chunks: u32,
    block: DenseBlockPayload<T>,
}

impl<T> DenseBlock<T> {
    fn payload(&self) -> &DenseBlockPayload<T> {
        &self.block
    }

    fn payload_mut(&mut self) -> &mut DenseBlockPayload<T> {
        &mut self.block
    }
}

/// Exact-superblock storage shared by typed semantic lanes.
///
/// Logical list chunks are cursor ranges packed into the initialized prefix
/// of an exact 64 KiB block. The semantic chunk boundary therefore neither
/// allocates nor moves payload, and releasing a chunk never writes a vacancy
/// representation across its resident values.
struct ChunkStorage<T> {
    layout: ChunkStorageLayout,
    logical_space: u32,
    logical_rows: Vec<LogicalChunkRow>,
    logical_free: Vec<u32>,
    chunk_bytes: usize,
    slots_per_chunk: usize,
    blocks: Vec<DenseBlock<T>>,
    free_blocks: Vec<u32>,
    free_ranges: Vec<(DenseBlockKey, u32)>,
    tail_block: Option<DenseBlockKey>,
    chunks: Vec<ChunkMeta>,
    #[cfg(feature = "profiling")]
    node_pool_storage_class: Option<NodePoolStorageClass>,
    #[cfg(test)]
    validation_reads: core::cell::Cell<u64>,
    #[cfg(test)]
    arena_position_reads: core::cell::Cell<u64>,
    #[cfg(test)]
    previous_link_reads: core::cell::Cell<u64>,
    #[cfg(any(test, feature = "testing"))]
    admitted_index_resolutions: core::cell::Cell<u64>,
    #[cfg(any(test, feature = "testing"))]
    admitted_index_predecessor_steps: core::cell::Cell<u64>,
    #[cfg(any(test, feature = "testing"))]
    admitted_forward_chunk_crossings: core::cell::Cell<u64>,
}

/// Profiling-only occupancy of one exact-superblock storage lane.
#[cfg(feature = "profiling")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ChunkStorageLayoutCensus {
    pub(crate) live_blocks: u64,
    pub(crate) used_records: u64,
    pub(crate) stranded_records: u64,
    pub(crate) partial_blocks: u64,
    pub(crate) physically_shared_blocks: u64,
}

/// One admitted final payload slot and the scalar facts needed to publish it.
struct ReservedChunkSlot<'a, T> {
    slot: &'a mut Option<T>,
    offset: u32,
    became_full: bool,
}

struct ReservedChunkPosition {
    page: usize,
    index: usize,
    offset: u32,
    became_full: bool,
}

/// Direct indices for one logical chunk already admitted against its physical
/// block incarnation.
///
/// This is deliberately a pair of stable vector coordinates, not a pointer.
/// The immutable pool borrow held by a list view excludes block recycling or
/// relocation while the coordinate is in use.
#[derive(Clone, Copy)]
struct AdmittedDenseBlock {
    page: u32,
    base: u32,
}

/// Exclusive admitted payload and metadata borrows for one contiguous append
/// run inside a physical block.
///
/// Allocation or a logical chunk transition rebuilds this capability. Values
/// within the run initialize the held typed block and advance its held chunk
/// metadata directly; neither vector is indexed again until the capability is
/// dropped at the block boundary.
struct AdmittedAppendRun<'a, T> {
    #[allow(dead_code)] // Used by the opt-in iterator-shaped benchmark adapter.
    key: LogicalChunkId,
    payload: &'a mut DenseBlockPayload<T>,
    meta: &'a mut ChunkMeta,
    index: usize,
    offset: u32,
    end: u32,
}

/// Coordinate-only continuation retained by builders across individual
/// `push` calls. Bulk publication uses [`AdmittedAppendRun`] instead.
struct AdmittedAppendBlock {
    key: LogicalChunkId,
    page: usize,
    index: usize,
    offset: u32,
}

impl<T> AdmittedAppendRun<'_, T> {
    #[allow(dead_code)] // Used by the opt-in iterator-shaped benchmark adapter.
    fn push(&mut self, value: T) {
        assert!(self.offset < self.end, "admitted append run has capacity");
        debug_assert!(self.meta.live && self.meta.generation == self.key.incarnation);
        debug_assert_eq!(self.meta.used, self.offset);
        debug_assert!(!self.meta.sealed && self.meta.sequence_summary.is_none());

        self.payload.initialize_admitted(self.index, value);
        self.index += 1;
        self.offset += 1;
        self.meta.used = self.offset;
        self.meta.sealed = self.offset == self.end;
        self.meta.dependency_metadata_complete = false;
    }

    #[allow(dead_code)] // Used by the opt-in iterator-shaped benchmark adapter.
    fn has_capacity(&self) -> bool {
        self.offset < self.end
    }

    fn is_full(&self) -> bool {
        self.offset == self.end
    }

    /// Copies one header value and as much of the following borrowed body as
    /// fits, then publishes the initialized prefix once for the complete run.
    fn extend_copy_parts(&mut self, first: Option<T>, body: &[T]) -> usize
    where
        T: Copy,
    {
        let available = (self.end - self.offset) as usize;
        let count = available.min(body.len().saturating_add(usize::from(first.is_some())));
        if count == 0 {
            return 0;
        }
        let header_count = usize::from(first.is_some());
        let body_count = count - header_count;
        match self.payload {
            DenseBlockPayload::Optional(block) => {
                let cells = &mut block.initialized_mut()[self.index..self.index + count];
                let mut destination = cells.iter_mut();
                if let Some(first) = first {
                    let slot = destination.next().expect("admitted header destination");
                    debug_assert!(slot.is_none());
                    *slot = Some(first);
                }
                for (slot, value) in destination.zip(&body[..body_count]) {
                    debug_assert!(slot.is_none());
                    *slot = Some(*value);
                }
            }
            DenseBlockPayload::Packed(block) => {
                if let Some(first) = first {
                    block
                        .extend_copy_from_slice(core::slice::from_ref(&first))
                        .expect("admitted header retains exact-block capacity");
                }
                block
                    .extend_copy_from_slice(&body[..body_count])
                    .expect("admitted body retains exact-block capacity");
            }
        }
        self.index += count;
        self.offset += count as u32;
        self.meta.used = self.offset;
        self.meta.sealed = self.offset == self.end;
        self.meta.dependency_metadata_complete = false;
        count
    }
}

struct CloneReservation {
    item_identity: Option<u64>,
    dependency_floor: Option<usize>,
    dependency_metadata_complete: bool,
    source_arena: u32,
    source_key: LogicalChunkId,
    source_offset: u32,
}

#[allow(dead_code)]
struct ReservationCompletion {
    placeholder_identity: Option<u64>,
    item_identity: Option<u64>,
    dependency_floor: Option<usize>,
}

/// Metadata produced while constructing one final value in an admitted run.
pub(crate) struct ConstructedRunValue {
    pub(crate) item_identity: Option<u64>,
    pub(crate) dependency_floor: Option<usize>,
    pub(crate) paired_dependency_floor: Option<usize>,
}

impl<T> ChunkStorage<T> {
    fn reserve_optional_run(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        count: usize,
        identity_enabled: bool,
    ) -> Result<(u32, Option<SemanticSequenceIdentity>), ForkArenaError> {
        if count == 0 {
            return Err(ForkArenaError::InvalidRange);
        }
        let meta = self.validate_lineage(key, arena, lineage)?;
        if meta.sealed || meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1 {
            return Err(ForkArenaError::ChunkShared);
        }
        if meta.used != 0 && meta.sequence_summary.is_some() != identity_enabled {
            return Err(ForkArenaError::IdentityModeMismatch);
        }
        let start = meta.used;
        let end = start
            .checked_add(u32::try_from(count).map_err(|_| ForkArenaError::CapacityOverflow)?)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        if end as usize > self.slots_per_chunk {
            return Err(ForkArenaError::CapacityOverflow);
        }
        let block = self
            .admit_dense_block(key)
            .ok_or(ForkArenaError::InvalidChunk)?;
        let first = block.base as usize + start as usize;
        let last = block.base as usize + end as usize;
        let DenseBlockPayload::Optional(payload) = self.blocks[block.page as usize].payload()
        else {
            return Err(ForkArenaError::InvalidRange);
        };
        if payload.initialized()[first..last]
            .iter()
            .any(Option::is_some)
        {
            return Err(ForkArenaError::InvalidRange);
        }
        let capacity = self.slots_per_chunk;
        let meta = self.validate_exclusive_lineage_mut(key, arena, lineage)?;
        let prefix_summary = meta.sequence_summary;
        meta.used = end;
        meta.sealed = end as usize == capacity;
        meta.dependency_metadata_complete = false;
        if identity_enabled {
            let summary = meta
                .sequence_summary
                .get_or_insert(SemanticSequenceIdentity::empty());
            for _ in 0..count {
                summary.push_back(0);
            }
        }
        Ok((start, prefix_summary))
    }

    fn with_optional_source_destination<R>(
        &mut self,
        source: AdmittedDenseBlock,
        source_range: Range<u32>,
        destination: LogicalChunkId,
        destination_range: Range<u32>,
        build: impl FnOnce(&[Option<T>], &mut [Option<T>]) -> R,
    ) -> Result<R, ForkArenaError> {
        let destination = self
            .admit_dense_block(destination)
            .ok_or(ForkArenaError::InvalidChunk)?;
        let source_start = source.base as usize + source_range.start as usize;
        let source_end = source.base as usize + source_range.end as usize;
        let destination_start = destination.base as usize + destination_range.start as usize;
        let destination_end = destination.base as usize + destination_range.end as usize;
        let source_page = source.page as usize;
        let destination_page = destination.page as usize;
        if source_page == destination_page {
            let DenseBlockPayload::Optional(block) = self.blocks[source_page].payload_mut() else {
                return Err(ForkArenaError::InvalidRange);
            };
            let cells = block.initialized_mut();
            if source_end <= destination_start {
                let (before, after) = cells.split_at_mut(destination_start);
                return Ok(build(
                    &before[source_start..source_end],
                    &mut after[..destination_end - destination_start],
                ));
            }
            if destination_end <= source_start {
                let (before, after) = cells.split_at_mut(source_start);
                return Ok(build(
                    &after[..source_end - source_start],
                    &mut before[destination_start..destination_end],
                ));
            }
            return Err(ForkArenaError::InvalidRange);
        }
        if source_page < destination_page {
            let (before, after) = self.blocks.split_at_mut(destination_page);
            let DenseBlockPayload::Optional(source_block) = before[source_page].payload() else {
                return Err(ForkArenaError::InvalidRange);
            };
            let DenseBlockPayload::Optional(destination_block) = after[0].payload_mut() else {
                return Err(ForkArenaError::InvalidRange);
            };
            return Ok(build(
                &source_block.initialized()[source_start..source_end],
                &mut destination_block.initialized_mut()[destination_start..destination_end],
            ));
        }
        let (before, after) = self.blocks.split_at_mut(source_page);
        let DenseBlockPayload::Optional(destination_block) = before[destination_page].payload_mut()
        else {
            return Err(ForkArenaError::InvalidRange);
        };
        let DenseBlockPayload::Optional(source_block) = after[0].payload() else {
            return Err(ForkArenaError::InvalidRange);
        };
        Ok(build(
            &source_block.initialized()[source_start..source_end],
            &mut destination_block.initialized_mut()[destination_start..destination_end],
        ))
    }
    /// Creates a pool whose logical chunk capacity is derived from a byte
    /// budget. At least one value fits even when `T` exceeds that budget.
    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        Self::with_layout(chunk_bytes, ChunkStorageLayout::OptionalSlots)
    }

    #[cfg(any(test, feature = "testing"))]
    fn with_packed_chunk_bytes(chunk_bytes: usize) -> Self
    where
        T: Copy,
    {
        Self::with_layout(chunk_bytes, ChunkStorageLayout::PackedCopy)
    }

    fn with_layout(chunk_bytes: usize, layout: ChunkStorageLayout) -> Self {
        assert!(chunk_bytes != 0, "chunk byte budget must be nonzero");
        let slot_bytes = match layout {
            ChunkStorageLayout::OptionalSlots => std::mem::size_of::<Option<T>>().max(1),
            ChunkStorageLayout::PackedCopy => std::mem::size_of::<T>().max(1),
        };
        let slots_per_chunk = (chunk_bytes / slot_bytes).max(1);
        Self {
            layout,
            // Reuse the workspace's canonical space allocator and coordinate
            // vocabulary. Payload resolution remains in this transitional
            // adapter until the NodeRecord table cutover.
            logical_space: AcceptedBlockTable::<u8>::new().space(),
            logical_rows: Vec::new(),
            logical_free: Vec::new(),
            chunk_bytes,
            slots_per_chunk,
            blocks: Vec::new(),
            free_blocks: Vec::new(),
            free_ranges: Vec::new(),
            tail_block: None,
            chunks: Vec::new(),
            #[cfg(feature = "profiling")]
            node_pool_storage_class: None,
            #[cfg(test)]
            validation_reads: core::cell::Cell::new(0),
            #[cfg(test)]
            arena_position_reads: core::cell::Cell::new(0),
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

    fn with_node_pool_layout(
        chunk_bytes: usize,
        layout: ChunkStorageLayout,
        class: NodePoolStorageClass,
    ) -> Self {
        #[cfg(feature = "profiling")]
        {
            let mut storage = Self::with_layout(chunk_bytes, layout);
            storage.node_pool_storage_class = Some(class);
            storage
        }
        #[cfg(not(feature = "profiling"))]
        {
            let _ = class;
            Self::with_layout(chunk_bytes, layout)
        }
    }

    fn allocate_logical(
        &mut self,
        physical: DenseBlockKey,
        physical_base: u32,
    ) -> Result<LogicalChunkId, ForkArenaError> {
        if let Some(&ordinal) = self.logical_free.last() {
            let row = self
                .logical_rows
                .get_mut(ordinal as usize)
                .ok_or(ForkArenaError::InvalidChunk)?;
            let incarnation = row
                .incarnation
                .checked_add(1)
                .ok_or(ForkArenaError::CapacityOverflow)?;
            self.logical_free.pop();
            *row = LogicalChunkRow {
                incarnation,
                physical_slot: physical.slot,
                physical_incarnation: physical.incarnation,
                physical_base,
            };
            return Ok(LogicalChunkId {
                ordinal,
                incarnation,
            });
        }
        let ordinal =
            u32::try_from(self.logical_rows.len()).map_err(|_| ForkArenaError::CapacityOverflow)?;
        self.logical_rows.push(LogicalChunkRow {
            incarnation: 1,
            physical_slot: physical.slot,
            physical_incarnation: physical.incarnation,
            physical_base,
        });
        Ok(LogicalChunkId {
            ordinal,
            incarnation: 1,
        })
    }

    fn mapping(&self, key: LogicalChunkId) -> Result<(DenseBlockKey, u32), ForkArenaError> {
        let row = self
            .logical_rows
            .get(key.ordinal as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if row.incarnation != key.incarnation {
            return Err(ForkArenaError::InvalidChunk);
        }
        if row.physical_slot == u32::MAX {
            return Err(ForkArenaError::InvalidChunk);
        }
        Ok((
            DenseBlockKey {
                slot: row.physical_slot,
                incarnation: row.physical_incarnation,
            },
            row.physical_base,
        ))
    }

    fn physical(&self, key: LogicalChunkId) -> Result<DenseBlockKey, ForkArenaError> {
        self.mapping(key).map(|mapping| mapping.0)
    }

    const fn logical_space(&self) -> u32 {
        self.logical_space
    }

    fn logical_position(
        &self,
        key: LogicalChunkId,
        offset: u32,
    ) -> Result<LogicalPosition, ForkArenaError> {
        self.physical(key)?;
        Ok(LogicalPosition::from_parts(
            key.block(self.logical_space)?,
            offset,
        ))
    }

    fn compact_position(
        &self,
        position: LogicalPosition,
    ) -> Result<(LogicalChunkId, u32), ForkArenaError> {
        let block = position.block();
        if block.space() != self.logical_space {
            return Err(ForkArenaError::InvalidChunk);
        }
        let key = LogicalChunkId {
            ordinal: block.ordinal(),
            incarnation: block.incarnation(),
        };
        self.physical(key)?;
        Ok((key, position.offset()))
    }

    fn release_logical(&mut self, key: LogicalChunkId) -> Result<DenseBlockKey, ForkArenaError> {
        let physical = self.physical(key)?;
        let row = &mut self.logical_rows[key.ordinal as usize];
        row.physical_slot = u32::MAX;
        row.physical_incarnation = 0;
        row.physical_base = 0;
        self.logical_free.push(key.ordinal);
        Ok(physical)
    }

    #[cfg(test)]
    fn validation_reads(&self) -> u64 {
        self.validation_reads.get()
    }

    #[cfg(test)]
    fn arena_position_reads(&self) -> u64 {
        self.arena_position_reads.get()
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

    const fn resident_slot_bytes(&self) -> usize {
        match self.layout {
            ChunkStorageLayout::OptionalSlots => std::mem::size_of::<Option<T>>(),
            ChunkStorageLayout::PackedCopy => std::mem::size_of::<T>(),
        }
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.blocks.len()
    }

    #[cfg(any(test, feature = "profiling"))]
    fn live_page_count(&self) -> usize {
        self.blocks.len().saturating_sub(self.free_blocks.len())
    }

    #[cfg(feature = "profiling")]
    fn vacant_page_count(&self) -> usize {
        self.free_blocks.len()
    }

    #[cfg(any(test, feature = "profiling"))]
    fn live_page_payload_bytes(&self) -> usize {
        self.live_page_count()
            .saturating_mul(tex_dense_prefix::SUPERBLOCK_BYTES)
    }

    #[cfg(any(test, feature = "profiling"))]
    fn vacant_page_payload_bytes(&self) -> usize {
        self.vacant_page_payload_block_count()
            .saturating_mul(tex_dense_prefix::SUPERBLOCK_BYTES)
    }

    fn vacant_page_payload_block_count(&self) -> usize {
        self.free_blocks.len()
    }

    #[cfg(feature = "profiling")]
    fn profiling_physical_token(&self, key: LogicalChunkId) -> Option<u64> {
        let (physical, _) = self.mapping(key).ok()?;
        Some((u64::from(physical.slot) << 32) | u64::from(physical.incarnation))
    }

    #[cfg(feature = "profiling")]
    fn profiling_live_physical_tokens(&self) -> Vec<u64> {
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.live_chunks != 0)
            .filter_map(|(slot, block)| {
                let slot = u32::try_from(slot).ok()?;
                Some((u64::from(slot) << 32) | u64::from(block.incarnation))
            })
            .collect()
    }

    #[cfg(feature = "profiling")]
    fn profiling_layout_census(&self) -> ChunkStorageLayoutCensus {
        use std::collections::BTreeMap;

        let mut used_by_block = BTreeMap::<u64, u64>::new();
        for (ordinal, chunk) in self.chunks.iter().enumerate() {
            if !chunk.live {
                continue;
            }
            let Some(row) = self.logical_rows.get(ordinal) else {
                continue;
            };
            let token = (u64::from(row.physical_slot) << 32) | u64::from(row.physical_incarnation);
            let used = used_by_block.entry(token).or_default();
            *used = used.saturating_add(u64::from(chunk.used));
        }
        let live_blocks = self.live_page_count() as u64;
        let used_records = used_by_block.values().copied().sum::<u64>();
        let records_per_block = match self.layout {
            ChunkStorageLayout::OptionalSlots => Superblock::<Option<T>>::capacity(),
            ChunkStorageLayout::PackedCopy => Superblock::<T>::capacity(),
        } as u64;
        ChunkStorageLayoutCensus {
            live_blocks,
            used_records,
            stranded_records: live_blocks
                .saturating_mul(records_per_block)
                .saturating_sub(used_records),
            partial_blocks: used_by_block
                .values()
                .filter(|&&used| used < records_per_block)
                .count() as u64,
            physically_shared_blocks: self
                .blocks
                .iter()
                .filter(|block| block.live_chunks > 1)
                .count() as u64,
        }
    }

    #[cfg(feature = "profiling")]
    fn record_node_pool_storage(&self, event: NodePoolStorageEvent) {
        if let Some(class) = self.node_pool_storage_class {
            crate::measurement::record_node_pool_storage(
                class,
                self.live_page_count(),
                self.vacant_page_count(),
                self.live_page_payload_bytes(),
                self.vacant_page_payload_bytes(),
                event,
            );
        }
    }

    #[cfg(test)]
    fn live_chunk_count(&self) -> usize {
        self.chunks.iter().filter(|chunk| chunk.live).count()
    }

    fn allocated_heap_bytes(&self) -> usize {
        self.logical_rows
            .capacity()
            .saturating_mul(std::mem::size_of::<LogicalChunkRow>())
            .saturating_add(
                self.logical_free
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(
                self.blocks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<DenseBlock<T>>())
                    .saturating_add(
                        self.blocks
                            .len()
                            .saturating_mul(tex_dense_prefix::SUPERBLOCK_BYTES),
                    )
                    .saturating_add(
                        self.chunks
                            .capacity()
                            .saturating_mul(std::mem::size_of::<ChunkMeta>()),
                    )
                    .saturating_add(
                        self.free_blocks
                            .capacity()
                            .saturating_mul(std::mem::size_of::<u32>()),
                    )
                    .saturating_add(
                        self.free_ranges
                            .capacity()
                            .saturating_mul(std::mem::size_of::<(DenseBlockKey, u32)>()),
                    ),
            )
    }

    fn allocate_dense_block(&mut self) -> Result<DenseBlockKey, ForkArenaError> {
        if let Some(&slot) = self.free_blocks.last() {
            let entry = self
                .blocks
                .get_mut(slot as usize)
                .ok_or(ForkArenaError::InvalidChunk)?;
            let incarnation = entry
                .incarnation
                .checked_add(1)
                .ok_or(ForkArenaError::CapacityOverflow)?;
            debug_assert_eq!(entry.live_chunks, 0);
            debug_assert!(entry.block.is_empty());
            self.free_blocks.pop();
            entry.incarnation = incarnation;
            #[cfg(feature = "profiling")]
            self.record_node_pool_storage(NodePoolStorageEvent::ReuseAllocation);
            return Ok(DenseBlockKey { slot, incarnation });
        }
        let slot =
            u32::try_from(self.blocks.len()).map_err(|_| ForkArenaError::CapacityOverflow)?;
        let block = self.allocate_dense_block_payload()?;
        self.blocks.push(DenseBlock {
            incarnation: 1,
            live_chunks: 0,
            block,
        });
        #[cfg(feature = "profiling")]
        self.record_node_pool_storage(NodePoolStorageEvent::FreshAllocation);
        Ok(DenseBlockKey {
            slot,
            incarnation: 1,
        })
    }

    fn allocate_dense_block_payload(&self) -> Result<DenseBlockPayload<T>, ForkArenaError> {
        Ok(match self.layout {
            ChunkStorageLayout::OptionalSlots => DenseBlockPayload::Optional(
                Superblock::try_new().map_err(|_| ForkArenaError::CapacityOverflow)?,
            ),
            ChunkStorageLayout::PackedCopy => DenseBlockPayload::Packed(
                Superblock::try_new().map_err(|_| ForkArenaError::CapacityOverflow)?,
            ),
        })
    }

    fn dense_block(&self, key: DenseBlockKey) -> Result<&DenseBlock<T>, ForkArenaError> {
        let block = self
            .blocks
            .get(key.slot as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if block.incarnation != key.incarnation {
            return Err(ForkArenaError::InvalidChunk);
        }
        Ok(block)
    }

    fn dense_block_mut(
        &mut self,
        key: DenseBlockKey,
    ) -> Result<&mut DenseBlock<T>, ForkArenaError> {
        let block = self
            .blocks
            .get_mut(key.slot as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if block.incarnation != key.incarnation {
            return Err(ForkArenaError::InvalidChunk);
        }
        Ok(block)
    }

    fn allocate_dense_range(&mut self) -> Result<(DenseBlockKey, u32), ForkArenaError> {
        if self.layout == ChunkStorageLayout::PackedCopy {
            let key = self.allocate_dense_block()?;
            self.dense_block_mut(key)?.live_chunks = 1;
            return Ok((key, 0));
        }
        while let Some((key, base)) = self.free_ranges.pop() {
            if let Ok(block) = self.dense_block_mut(key) {
                block.live_chunks = block
                    .live_chunks
                    .checked_add(1)
                    .ok_or(ForkArenaError::CapacityOverflow)?;
                return Ok((key, base));
            }
        }
        let capacity = Superblock::<Option<T>>::capacity();
        let tail = self.tail_block.filter(|key| {
            self.dense_block(*key).is_ok_and(|block| {
                block.payload().len().saturating_add(self.slots_per_chunk) <= capacity
            })
        });
        let key = match tail {
            Some(key) => key,
            None => {
                let key = self.allocate_dense_block()?;
                self.tail_block = Some(key);
                key
            }
        };
        let base = u32::try_from(self.dense_block(key)?.payload().len())
            .map_err(|_| ForkArenaError::CapacityOverflow)?;
        let slots_per_chunk = self.slots_per_chunk;
        let block = self.dense_block_mut(key)?;
        for _ in 0..slots_per_chunk {
            let DenseBlockPayload::Optional(payload) = block.payload_mut() else {
                return Err(ForkArenaError::InvalidChunk);
            };
            payload
                .push_with(|slot| slot.insert(None))
                .map_err(|_| ForkArenaError::CapacityOverflow)?;
        }
        block.live_chunks += 1;
        Ok((key, base))
    }

    fn allocate(&mut self, arena: u32, lineage: u32) -> Result<LogicalChunkId, ForkArenaError> {
        let (physical, physical_base) = self.allocate_dense_range()?;
        let key = self.allocate_logical(physical, physical_base)?;
        let meta = ChunkMeta {
            generation: key.incarnation,
            arena,
            lineages: [
                ChunkLineage {
                    id: lineage,
                    position: usize::MAX,
                },
                VACANT_CHUNK_LINEAGE,
            ],
            used: 0,
            live: true,
            sealed: false,
            sequence_summary: None,
            previous_in_list: None,
            dependency_floor: usize::MAX,
            dependency_metadata_complete: true,
            paired_dependency_floor: usize::MAX,
        };
        if key.ordinal as usize == self.chunks.len() {
            self.chunks.push(meta);
        } else {
            let destination = self
                .chunks
                .get_mut(key.ordinal as usize)
                .ok_or(ForkArenaError::InvalidChunk)?;
            *destination = meta;
        }
        Ok(key)
    }

    fn allocate_list_block(
        &mut self,
        arena: u32,
        lineage: u32,
        previous_in_list: Option<(LogicalChunkId, u32)>,
    ) -> Result<LogicalChunkId, ForkArenaError> {
        let key = self.allocate(arena, lineage)?;
        self.set_previous_in_list(key, arena, lineage, previous_in_list)?;
        Ok(key)
    }

    fn validate(&self, key: LogicalChunkId, arena: u32) -> Result<&ChunkMeta, ForkArenaError> {
        #[cfg(test)]
        self.validation_reads
            .set(self.validation_reads.get().saturating_add(1));
        let meta = self
            .chunks
            .get(key.ordinal as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if key.incarnation != meta.generation || !meta.live || meta.arena != arena {
            return Err(ForkArenaError::InvalidChunk);
        }
        Ok(meta)
    }

    fn validate_mut(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
    ) -> Result<&mut ChunkMeta, ForkArenaError> {
        let meta = self
            .chunks
            .get_mut(key.ordinal as usize)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if key.incarnation != meta.generation || !meta.live || meta.arena != arena {
            return Err(ForkArenaError::InvalidChunk);
        }
        Ok(meta)
    }

    fn validate_lineage(
        &self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
    ) -> Result<&ChunkMeta, ForkArenaError> {
        let meta = self.validate(key, arena)?;
        meta.lineages
            .iter()
            .any(|entry| entry.id == lineage)
            .then_some(meta)
            .ok_or(ForkArenaError::InvalidChunk)
    }

    fn validate_exclusive_lineage_mut(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
    ) -> Result<&mut ChunkMeta, ForkArenaError> {
        let meta = self.validate_mut(key, arena)?;
        if meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1
            || meta.lineages.iter().all(|entry| entry.id != lineage)
        {
            return Err(ForkArenaError::ChunkShared);
        }
        Ok(meta)
    }

    fn share_with_lineage(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        source_lineage: u32,
        destination_lineage: u32,
        destination_position: usize,
    ) -> Result<(), ForkArenaError> {
        let meta = self.validate_mut(key, arena)?;
        if !meta.sealed
            || meta.lineages.iter().all(|entry| entry.id != source_lineage)
            || meta
                .lineages
                .iter()
                .any(|entry| entry.id == destination_lineage)
        {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        let vacant = meta
            .lineages
            .iter_mut()
            .find(|entry| entry.id == 0)
            .ok_or(ForkArenaError::TooManyLineages)?;
        *vacant = ChunkLineage {
            id: destination_lineage,
            position: destination_position,
        };
        Ok(())
    }

    fn set_previous_in_list(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        previous: Option<(LogicalChunkId, u32)>,
    ) -> Result<(), ForkArenaError> {
        let paired_prefix_floor = previous
            .map(|(key, _)| {
                self.validate(key, arena)
                    .map(|meta| meta.paired_dependency_floor)
            })
            .transpose()?;
        let previous = previous
            .map(|(key, offset)| self.logical_position(key, offset))
            .transpose()?;
        let meta = self.validate_exclusive_lineage_mut(key, arena, lineage)?;
        meta.previous_in_list = previous;
        if let Some(paired_prefix_floor) = paired_prefix_floor {
            meta.paired_dependency_floor = meta.paired_dependency_floor.min(paired_prefix_floor);
        }
        Ok(())
    }

    fn slot_index(
        &self,
        key: LogicalChunkId,
        offset: usize,
    ) -> Result<(usize, usize), ForkArenaError> {
        if offset >= self.slots_per_chunk {
            return Err(ForkArenaError::InvalidRange);
        }
        let (physical, base) = self.mapping(key)?;
        let index = (base as usize)
            .checked_add(offset)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        Ok((physical.slot as usize, index))
    }

    /// Admits and publishes one vacant final slot before returning its place.
    ///
    /// The caller must fill the returned slot immediately. This private seam
    /// lets the semantic facade perform the sole payload write without
    /// carrying `T` through generic arena return values or callbacks.
    fn reserve(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        item_identity: Option<u64>,
        dependency_floor: Option<usize>,
        dependency_metadata_complete: bool,
    ) -> Result<ReservedChunkSlot<'_, T>, ForkArenaError> {
        let ReservedChunkPosition {
            page,
            index,
            offset,
            became_full,
        } = self.reserve_position(
            key,
            arena,
            lineage,
            item_identity,
            dependency_floor,
            dependency_metadata_complete,
        )?;
        Ok(ReservedChunkSlot {
            slot: self.blocks[page]
                .payload_mut()
                .optional_slot(index)
                .expect("vacancy reservation requires optional-slot storage"),
            offset,
            became_full,
        })
    }

    fn reserve_position(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        item_identity: Option<u64>,
        dependency_floor: Option<usize>,
        dependency_metadata_complete: bool,
    ) -> Result<ReservedChunkPosition, ForkArenaError> {
        let used = {
            let meta = self.validate_lineage(key, arena, lineage)?;
            if meta.sealed || meta.used as usize == self.slots_per_chunk {
                return Err(ForkArenaError::ChunkSealed);
            }
            if meta.used != 0 && meta.sequence_summary.is_some() != item_identity.is_some() {
                return Err(ForkArenaError::IdentityModeMismatch);
            }
            meta.used
        };
        let (page, index) = self.slot_index(key, used as usize)?;
        let physical = self.physical(key)?;
        let block = self.dense_block_mut(physical)?;
        match block.payload() {
            DenseBlockPayload::Optional(payload) if payload.get(index).is_none() => {
                return Err(ForkArenaError::InvalidRange);
            }
            DenseBlockPayload::Packed(payload) if payload.len() != index => {
                return Err(ForkArenaError::InvalidRange);
            }
            _ => {}
        }
        let became_full = used as usize + 1 == self.slots_per_chunk;
        let meta = self.validate_exclusive_lineage_mut(key, arena, lineage)?;
        meta.used += 1;
        meta.sealed = became_full;
        if let Some(item_identity) = item_identity {
            let summary = meta
                .sequence_summary
                .get_or_insert(SemanticSequenceIdentity::empty());
            summary.push_back(item_identity);
        }
        if let Some(dependency_floor) = dependency_floor {
            meta.dependency_floor = meta.dependency_floor.min(dependency_floor);
        }
        meta.dependency_metadata_complete &= dependency_metadata_complete;
        Ok(ReservedChunkPosition {
            page,
            index,
            offset: used,
            became_full,
        })
    }

    /// Checks and exclusively admits one untracked contiguous append run.
    fn admit_untracked_append_run(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
    ) -> Result<AdmittedAppendRun<'_, T>, ForkArenaError> {
        let meta = self.validate_lineage(key, arena, lineage)?;
        if meta.sealed || meta.used as usize == self.slots_per_chunk {
            return Err(ForkArenaError::ChunkSealed);
        }
        if meta.used != 0 && meta.sequence_summary.is_some() {
            return Err(ForkArenaError::IdentityModeMismatch);
        }
        if meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1 {
            return Err(ForkArenaError::ChunkShared);
        }
        let block = self
            .admit_dense_block(key)
            .ok_or(ForkArenaError::InvalidChunk)?;
        let index = block
            .base
            .checked_add(meta.used)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        match self.blocks[block.page as usize].payload() {
            DenseBlockPayload::Optional(payload) if payload.get(index as usize).is_none() => {
                return Err(ForkArenaError::InvalidRange);
            }
            DenseBlockPayload::Packed(payload) if payload.len() != index as usize => {
                return Err(ForkArenaError::InvalidRange);
            }
            _ => {}
        }
        let page = block.page as usize;
        let offset = meta.used;
        let end =
            u32::try_from(self.slots_per_chunk).map_err(|_| ForkArenaError::CapacityOverflow)?;
        let meta = &mut self.chunks[key.ordinal as usize];
        let payload = self.blocks[page].payload_mut();
        Ok(AdmittedAppendRun {
            key,
            payload,
            meta,
            index: index as usize,
            offset,
            end,
        })
    }

    fn admit_untracked_append_block(
        &self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
    ) -> Result<AdmittedAppendBlock, ForkArenaError> {
        let meta = self.validate_lineage(key, arena, lineage)?;
        if meta.sealed || meta.used as usize == self.slots_per_chunk {
            return Err(ForkArenaError::ChunkSealed);
        }
        if meta.used != 0 && meta.sequence_summary.is_some() {
            return Err(ForkArenaError::IdentityModeMismatch);
        }
        if meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1 {
            return Err(ForkArenaError::ChunkShared);
        }
        let block = self
            .admit_dense_block(key)
            .ok_or(ForkArenaError::InvalidChunk)?;
        let index = block
            .base
            .checked_add(meta.used)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        Ok(AdmittedAppendBlock {
            key,
            page: block.page as usize,
            index: index as usize,
            offset: meta.used,
        })
    }

    fn append_admitted_untracked(
        &mut self,
        cursor: &mut AdmittedAppendBlock,
        value: T,
    ) -> (u32, bool) {
        let offset = cursor.offset;
        let next_offset = offset + 1;
        let became_full = next_offset as usize == self.slots_per_chunk;
        self.blocks[cursor.page]
            .payload_mut()
            .initialize_admitted(cursor.index, value);

        let meta = &mut self.chunks[cursor.key.ordinal as usize];
        debug_assert!(meta.live && meta.generation == cursor.key.incarnation);
        debug_assert_eq!(meta.used, offset);
        debug_assert!(!meta.sealed && meta.sequence_summary.is_none());
        meta.used = next_offset;
        meta.sealed = became_full;
        meta.dependency_metadata_complete = false;

        cursor.index += 1;
        cursor.offset = next_offset;
        (offset, became_full)
    }

    /// Clones one admitted source cell into one distinct final reserved cell.
    fn reserve_clone_from(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        reservation: CloneReservation,
    ) -> Result<(u32, bool), ForkArenaError>
    where
        T: Clone,
    {
        let CloneReservation {
            item_identity,
            dependency_floor,
            dependency_metadata_complete,
            source_arena,
            source_key,
            source_offset,
        } = reservation;
        if key == source_key {
            return Err(ForkArenaError::InvalidRange);
        }
        let source_meta = self.validate(source_key, source_arena)?;
        if source_offset >= source_meta.used {
            return Err(ForkArenaError::InvalidRange);
        }
        let paired_dependency_floor = source_meta.paired_dependency_floor;
        let (source_page, source_index) = self.slot_index(source_key, source_offset as usize)?;
        let cloned = self.blocks[source_page]
            .payload()
            .value(source_index)
            .ok_or(ForkArenaError::InvalidRange)?
            .clone();
        let ReservedChunkPosition {
            page,
            index,
            offset,
            became_full,
        } = self.reserve_position(
            key,
            arena,
            lineage,
            item_identity,
            dependency_floor,
            dependency_metadata_complete,
        )?;
        self.blocks[page].payload_mut().insert(index, cloned)?;
        let destination = self.validate_exclusive_lineage_mut(key, arena, lineage)?;
        destination.paired_dependency_floor = destination
            .paired_dependency_floor
            .min(paired_dependency_floor);
        Ok((offset, became_full))
    }

    fn get(&self, key: LogicalChunkId, arena: u32, offset: u32) -> Option<&T> {
        let meta = self.validate(key, arena).ok()?;
        if offset >= meta.used {
            return None;
        }
        let (page, index) = self.slot_index(key, offset as usize).ok()?;
        self.blocks[page].payload().value(index)
    }

    fn get_mut(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        offset: u32,
    ) -> Option<&mut T> {
        let meta = self
            .validate_exclusive_lineage_mut(key, arena, lineage)
            .ok()?;
        if offset >= meta.used {
            return None;
        }
        let (page, index) = self.slot_index(key, offset as usize).ok()?;
        self.blocks[page].payload_mut().value_mut(index)
    }

    #[allow(dead_code)]
    fn complete_reservation(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        offset: u32,
        completion: ReservationCompletion,
    ) -> Result<(), ForkArenaError> {
        let ReservationCompletion {
            placeholder_identity,
            item_identity,
            dependency_floor,
        } = completion;
        let meta = self.validate_exclusive_lineage_mut(key, arena, lineage)?;
        if offset.checked_add(1) != Some(meta.used)
            || placeholder_identity.is_some() != item_identity.is_some()
        {
            return Err(ForkArenaError::InvalidRange);
        }
        if let (Some(placeholder), Some(identity), Some(summary)) = (
            placeholder_identity,
            item_identity,
            meta.sequence_summary.as_mut(),
        ) {
            summary.replace(summary.len() - 1, placeholder, identity);
        }
        if let Some(dependency_floor) = dependency_floor {
            meta.dependency_floor = meta.dependency_floor.min(dependency_floor);
        }
        meta.dependency_metadata_complete = true;
        Ok(())
    }

    fn used(&self, key: LogicalChunkId, arena: u32) -> Result<u32, ForkArenaError> {
        Ok(self.validate(key, arena)?.used)
    }

    fn sequence_summary(
        &self,
        key: LogicalChunkId,
        arena: u32,
    ) -> Result<Option<SemanticSequenceIdentity>, ForkArenaError> {
        Ok(self.validate(key, arena)?.sequence_summary)
    }

    fn is_sealed(&self, key: LogicalChunkId, arena: u32) -> Result<bool, ForkArenaError> {
        Ok(self.validate(key, arena)?.sealed)
    }

    fn seal(&mut self, key: LogicalChunkId, arena: u32) -> Result<usize, ForkArenaError> {
        let capacity = self.slots_per_chunk;
        let meta = self.validate_mut(key, arena)?;
        meta.sealed = true;
        Ok(capacity.saturating_sub(meta.used as usize))
    }

    /// Seals a tail carried by an exclusive construction capability.
    ///
    /// The capability was minted only after owner/generation/lineage
    /// admission and the exclusive borrow prevents lifecycle mutation until
    /// consumption. Those facts are debug-audited here instead of becoming a
    /// second fallible publication pass.
    fn seal_constructed_tail(&mut self, key: LogicalChunkId, arena: u32, lineage: u32) -> usize {
        let capacity = self.slots_per_chunk;
        let meta = &mut self.chunks[key.ordinal as usize];
        debug_assert!(meta.live && meta.generation == key.incarnation);
        debug_assert_eq!(meta.arena, arena);
        debug_assert!(meta.lineages.iter().any(|entry| entry.id == lineage));
        meta.sealed = true;
        capacity.saturating_sub(meta.used as usize)
    }

    fn truncate(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        used: u32,
        sequence_summary: Option<SemanticSequenceIdentity>,
    ) -> Result<(), ForkArenaError> {
        let old_used = self
            .validate_exclusive_lineage_mut(key, arena, lineage)?
            .used;
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
        let (physical, base) = self.mapping(key)?;
        match self.dense_block_mut(physical)?.payload_mut() {
            DenseBlockPayload::Optional(block) => {
                for offset in used..old_used {
                    let index = base as usize + offset as usize;
                    drop(block.get_mut(index).and_then(Option::take));
                }
            }
            DenseBlockPayload::Packed(block) => {
                debug_assert_eq!(base, 0);
                block.truncate(used as usize);
            }
        }
        let capacity = self.slots_per_chunk;
        let meta = self.validate_exclusive_lineage_mut(key, arena, lineage)?;
        meta.used = used;
        meta.sequence_summary = sequence_summary;
        if used as usize != capacity {
            meta.sealed = false;
        }
        Ok(())
    }

    fn restore_sealed(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        sealed: bool,
    ) -> Result<(), ForkArenaError> {
        let capacity = self.slots_per_chunk;
        let meta = self.validate_exclusive_lineage_mut(key, arena, lineage)?;
        if meta.used as usize == capacity && !sealed {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        meta.sealed = sealed;
        Ok(())
    }

    #[cfg(test)]
    fn release(&mut self, key: LogicalChunkId, arena: u32) -> Result<usize, ForkArenaError> {
        self.release_lineage(key, arena, self.validate(key, arena)?.lineages[0].id)
    }

    fn release_lineage(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
    ) -> Result<usize, ForkArenaError> {
        let (physical, physical_base) = self.mapping(key)?;
        let used = self.validate(key, arena)?.used;
        let meta = self.validate_mut(key, arena)?;
        let Some(index) = meta.lineages.iter().position(|entry| entry.id == lineage) else {
            return Err(ForkArenaError::InvalidChunk);
        };
        if meta
            .lineages
            .iter()
            .enumerate()
            .any(|(other, entry)| other != index && entry.id != 0)
        {
            meta.lineages[index] = VACANT_CHUNK_LINEAGE;
            return Ok(0);
        }
        match self.dense_block_mut(physical)?.payload_mut() {
            DenseBlockPayload::Optional(block) => {
                for offset in 0..used {
                    let index = physical_base as usize + offset as usize;
                    drop(block.get_mut(index).and_then(Option::take));
                }
            }
            DenseBlockPayload::Packed(block) => {
                debug_assert_eq!(physical_base, 0);
                block.truncate(0);
            }
        }
        let meta = self.validate_mut(key, arena)?;
        meta.live = false;
        meta.sealed = false;
        meta.arena = 0;
        meta.lineages = [VACANT_CHUNK_LINEAGE; 2];
        meta.used = 0;
        meta.sequence_summary = None;
        meta.previous_in_list = None;
        let block = self.dense_block_mut(physical)?;
        block.live_chunks = block
            .live_chunks
            .checked_sub(1)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if block.live_chunks == 0 {
            block.payload_mut().truncate(0);
            self.free_ranges.retain(|(key, _)| *key != physical);
            self.free_blocks.push(physical.slot);
            if self.tail_block == Some(physical) {
                self.tail_block = None;
            }
            #[cfg(feature = "profiling")]
            self.record_node_pool_storage(NodePoolStorageEvent::Release);
        } else if self.layout == ChunkStorageLayout::OptionalSlots {
            self.free_ranges.push((physical, physical_base));
        }
        self.release_logical(key)?;
        Ok(used as usize)
    }

    fn index_in_arena(
        &mut self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
        position: usize,
    ) -> Result<(), ForkArenaError> {
        let meta = self.validate_mut(key, arena)?;
        let entry = meta
            .lineages
            .iter_mut()
            .find(|entry| entry.id == lineage)
            .ok_or(ForkArenaError::InvalidChunk)?;
        entry.position = position;
        Ok(())
    }

    fn unindex_from_arena(&mut self, key: LogicalChunkId, arena: u32, lineage: u32) {
        if let Some(meta) = self.chunks.get_mut(key.ordinal as usize)
            && meta.live
            && meta.generation == key.incarnation
            && meta.arena == arena
            && let Some(entry) = meta.lineages.iter_mut().find(|entry| entry.id == lineage)
        {
            entry.position = usize::MAX;
        }
    }

    fn arena_position(&self, key: LogicalChunkId, arena: u32, lineage: u32) -> Option<usize> {
        #[cfg(test)]
        self.arena_position_reads
            .set(self.arena_position_reads.get().saturating_add(1));
        self.physical(key).ok()?;
        let meta = self.chunks.get(key.ordinal as usize)?;
        if !meta.live || meta.generation != key.incarnation || meta.arena != arena {
            return None;
        }
        let position = meta
            .lineages
            .iter()
            .find(|entry| entry.id == lineage)?
            .position;
        (position != usize::MAX).then_some(position)
    }

    /// Reads topology already admitted through an immutable arena view.
    ///
    /// The opaque root and its owner-relative endpoint positions were checked
    /// before the view was constructed. The shared `&ChunkPool` borrow then
    /// excludes release, transfer, rollback, and incarnation reuse for the
    /// lifetime of the view, so ordinary traversal need not repeat those
    /// ownership checks at every block crossing.
    /// Resolves the predecessor incarnation and its admitted arena position.
    fn admitted_previous_coordinate(
        &self,
        key: LogicalChunkId,
        lineage: u32,
    ) -> Option<(LogicalChunkId, usize, u32)> {
        self.physical(key).ok()?;
        let meta = self.chunks.get(key.ordinal as usize)?;
        debug_assert!(meta.live && meta.generation == key.incarnation);
        let (previous_key, end) = self.compact_position(meta.previous_in_list?).ok()?;
        self.physical(previous_key).ok()?;
        let previous = self.chunks.get(previous_key.ordinal as usize)?;
        debug_assert!(previous.live && previous.generation == previous_key.incarnation);
        let position = previous
            .lineages
            .iter()
            .find(|entry| entry.id == lineage)?
            .position;
        if position == usize::MAX {
            return None;
        }
        Some((previous_key, position, end))
    }

    /// Resolves one logical chunk to its typed physical block once at an
    /// admission or chunk-transition boundary.
    fn admit_dense_block(&self, key: LogicalChunkId) -> Option<AdmittedDenseBlock> {
        let (physical, base) = self.mapping(key).ok()?;
        self.dense_block(physical).ok()?;
        base.checked_add(u32::try_from(self.slots_per_chunk).ok()?)?;
        Some(AdmittedDenseBlock {
            page: physical.slot,
            base,
        })
    }

    /// Reads from an already-admitted typed block. All coordinate ownership
    /// and incarnation checks happened before `block` was constructed.
    fn admitted_dense_value(&self, block: AdmittedDenseBlock, offset: u32) -> Option<&T> {
        let index = block.base.checked_add(offset)?;
        self.blocks
            .get(block.page as usize)?
            .payload()
            .value(index as usize)
    }

    /// Reads through a capability whose block and initialized range were
    /// admitted already. Safe indexing remains the only executable guard.
    fn admitted_capability_value(&self, block: AdmittedDenseBlock, offset: u32) -> &T {
        let index = block.base as usize + offset as usize;
        self.blocks[block.page as usize]
            .payload()
            .value(index)
            .expect("admitted cursor remains inside the initialized prefix")
    }

    /// Borrows one range from a physical block admitted at a list boundary.
    fn admitted_dense_slice(
        &self,
        block: AdmittedDenseBlock,
        range: Range<u32>,
    ) -> Option<DenseBlockSlice<'_, T>> {
        if range.start > range.end {
            return None;
        }
        let start = block.base.checked_add(range.start)? as usize;
        let end = block.base.checked_add(range.end)? as usize;
        match self.blocks.get(block.page as usize)?.payload() {
            DenseBlockPayload::Optional(payload) => Some(DenseBlockSlice::Optional(
                payload.initialized().get(start..end)?,
            )),
            DenseBlockPayload::Packed(payload) => Some(DenseBlockSlice::Packed(
                payload.initialized().get(start..end)?,
            )),
        }
    }

    fn admitted_slice(
        &self,
        key: LogicalChunkId,
        range: Range<u32>,
    ) -> Option<DenseBlockSlice<'_, T>> {
        self.physical(key).ok()?;
        let meta = self.chunks.get(key.ordinal as usize)?;
        debug_assert!(meta.live && meta.generation == key.incarnation);
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
        match self.blocks.get(start_page)?.payload() {
            DenseBlockPayload::Optional(block) => Some(DenseBlockSlice::Optional(
                block.initialized().get(start..end)?,
            )),
            DenseBlockPayload::Packed(block) => Some(DenseBlockSlice::Packed(
                block.initialized().get(start..end)?,
            )),
        }
    }

    fn previous_in_list(
        &self,
        key: LogicalChunkId,
        arena: u32,
    ) -> Result<Option<(LogicalChunkId, u32)>, ForkArenaError> {
        #[cfg(test)]
        self.previous_link_reads
            .set(self.previous_link_reads.get().saturating_add(1));
        self.validate(key, arena)?
            .previous_in_list
            .map(|position| self.compact_position(position))
            .transpose()
    }

    fn transfer(
        &mut self,
        key: LogicalChunkId,
        source: u32,
        source_lineage: u32,
        destination: u32,
        destination_lineage: u32,
    ) -> Result<(), ForkArenaError> {
        let meta = self.validate_exclusive_lineage_mut(key, source, source_lineage)?;
        if !meta.sealed {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        meta.arena = destination;
        meta.lineages
            .iter_mut()
            .find(|entry| entry.id == source_lineage)
            .expect("exclusive lineage validation retained its source")
            .id = destination_lineage;
        Ok(())
    }
}

#[cfg(feature = "profiling")]
impl<T> Drop for ChunkStorage<T> {
    fn drop(&mut self) {
        self.record_node_pool_storage(NodePoolStorageEvent::DropStorage);
    }
}

/// Storage-agnostic borrowed resolver selected by the semantic fork owner.
///
/// Both variants use the same transitional owned-Node adapter today. The
/// compact cutover replaces only these constructors with
/// `AcceptedBlockView<NodeRecord>` and `CandidateBlockView<NodeRecord>`; list
/// coordinates, predecessors, and traversal code remain unchanged.
enum LogicalBlockView<'a, T> {
    Accepted(&'a ChunkStorage<T>),
    Candidate(&'a ChunkStorage<T>),
}

impl<T> Clone for LogicalBlockView<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for LogicalBlockView<'_, T> {}

impl<'a, T> LogicalBlockView<'a, T> {
    fn storage(self) -> &'a ChunkStorage<T> {
        match self {
            Self::Accepted(storage) | Self::Candidate(storage) => storage,
        }
    }

    fn slice(self, first: LogicalPosition, end: u32) -> Option<DenseBlockSlice<'a, T>> {
        let storage = self.storage();
        let (key, start) = storage.compact_position(first).ok()?;
        storage.admitted_slice(key, start..end)
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
    next_publication_serial: u32,
}

impl<T> Default for ChunkPool<T> {
    fn default() -> Self {
        Self::with_chunk_bytes(DEFAULT_CHUNK_BYTES)
    }
}

impl<T> ChunkPool<T> {
    #[must_use]
    pub(crate) const fn logical_space(&self) -> u32 {
        self.payload.logical_space()
    }

    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        Self {
            owner: NEXT_POOL_OWNER.fetch_add(1, Ordering::Relaxed),
            payload: ChunkStorage::with_chunk_bytes(chunk_bytes),
            next_publication_serial: 1,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn with_packed_chunk_bytes(chunk_bytes: usize) -> Self
    where
        T: Copy,
    {
        Self {
            owner: NEXT_POOL_OWNER.fetch_add(1, Ordering::Relaxed),
            payload: ChunkStorage::with_packed_chunk_bytes(chunk_bytes),
            next_publication_serial: 1,
        }
    }

    #[must_use]
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn testing_with_packed_chunk_bytes(chunk_bytes: usize) -> Self
    where
        T: Copy,
    {
        Self {
            owner: NEXT_POOL_OWNER.fetch_add(1, Ordering::Relaxed),
            payload: ChunkStorage::with_packed_chunk_bytes(chunk_bytes),
            next_publication_serial: 1,
        }
    }

    pub(crate) fn with_node_pool_chunk_bytes(
        chunk_bytes: usize,
        class: NodePoolStorageClass,
    ) -> Self {
        Self {
            owner: NEXT_POOL_OWNER.fetch_add(1, Ordering::Relaxed),
            payload: ChunkStorage::with_node_pool_layout(
                chunk_bytes,
                ChunkStorageLayout::OptionalSlots,
                class,
            ),
            next_publication_serial: 1,
        }
    }

    pub(crate) fn with_node_pool_packed_chunk_bytes(
        chunk_bytes: usize,
        class: NodePoolStorageClass,
    ) -> Self
    where
        T: Copy,
    {
        Self {
            owner: NEXT_POOL_OWNER.fetch_add(1, Ordering::Relaxed),
            payload: ChunkStorage::with_node_pool_layout(
                chunk_bytes,
                ChunkStorageLayout::PackedCopy,
                class,
            ),
            next_publication_serial: 1,
        }
    }

    pub(crate) fn next_publication_serial(&mut self) -> u32 {
        let serial = self.next_publication_serial;
        self.next_publication_serial = serial
            .checked_add(1)
            .expect("chunk-pool publication serial exhausted");
        serial
    }

    #[must_use]
    pub const fn chunk_byte_budget(&self) -> usize {
        self.payload.chunk_byte_budget()
    }

    #[must_use]
    pub const fn chunk_capacity(&self) -> usize {
        self.payload.chunk_capacity()
    }

    #[cfg(test)]
    pub(crate) const fn resident_payload_slot_bytes(&self) -> usize {
        self.payload.resident_slot_bytes()
    }

    #[cfg(test)]
    pub(crate) const fn has_packed_payload(&self) -> bool {
        matches!(self.payload.layout, ChunkStorageLayout::PackedCopy)
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.payload.page_count()
    }

    #[cfg(test)]
    pub(crate) fn live_payload_chunk_count(&self) -> usize {
        self.payload.live_chunk_count()
    }

    #[must_use]
    pub const fn logical_block_metadata_bytes(&self) -> usize {
        std::mem::size_of::<ChunkMeta>()
    }

    /// Transitional logical-to-physical adapter row size. This row disappears
    /// when `AcceptedBlockTable<NodeRecord>` becomes the production resolver.
    #[must_use]
    pub const fn logical_mapping_row_bytes(&self) -> usize {
        std::mem::size_of::<LogicalChunkRow>()
    }

    #[must_use]
    pub fn physical_page_payload_bytes(&self) -> usize {
        tex_dense_prefix::SUPERBLOCK_BYTES
    }

    #[must_use]
    pub const fn physical_page_metadata_bytes(&self) -> usize {
        std::mem::size_of::<DenseBlock<T>>()
    }

    /// Heap capacity retained by payload pages and their allocation metadata.
    /// Allocator-private bookkeeping is not observable.
    #[must_use]
    pub fn allocated_heap_bytes(&self) -> usize {
        self.payload.allocated_heap_bytes()
    }

    /// Heap capacity retained by live logical owners plus stable pool
    /// metadata. Warm vacant payload backing is reusable capacity, not a
    /// checkpoint owner charge.
    #[must_use]
    pub(crate) fn live_owner_heap_bytes(&self) -> usize {
        self.payload.allocated_heap_bytes().saturating_sub(
            self.payload
                .vacant_page_payload_block_count()
                .saturating_mul(tex_dense_prefix::SUPERBLOCK_BYTES),
        )
    }

    #[cfg(test)]
    pub(crate) fn live_page_count(&self) -> usize {
        self.payload.live_page_count()
    }

    #[cfg(test)]
    pub(crate) fn vacant_page_payload_block_count(&self) -> usize {
        self.payload.vacant_page_payload_block_count()
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn profiling_live_physical_tokens(&self) -> Vec<u64> {
        self.payload.profiling_live_physical_tokens()
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn profiling_layout_census(&self) -> ChunkStorageLayoutCensus {
        self.payload.profiling_layout_census()
    }
}

/// Direct cursor into one packed logical-list block.
pub struct ChunkCursor<Lane> {
    raw: LogicalChunkId,
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
        raw: LogicalChunkId {
            ordinal: 0,
            incarnation: 0,
        },
        offset: 0,
        _lane: PhantomData,
    };

    const fn new(raw: LogicalChunkId, offset: u32) -> Self {
        Self {
            raw,
            offset,
            _lane: PhantomData,
        }
    }

    fn logical_position(self, space: u32) -> Result<LogicalPosition, ForkArenaError> {
        Ok(LogicalPosition::from_parts(
            self.raw.block(space)?,
            self.offset,
        ))
    }
}

/// Direct root of one generation-owned packed chunk chain.
///
/// The head offset is inclusive and the tail offset is exclusive. Every
/// nonempty root therefore reaches its last node without an auxiliary lookup.
pub struct ArenaListId<Lane> {
    /// Pool-stable logical coordinate space, not a semantic arena owner.
    space: u32,
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
        self.space == other.space
            && self.head == other.head
            && self.tail == other.tail
            && self.len == other.len
    }
}
impl<Lane> Eq for ArenaListId<Lane> {}

impl<Lane> Hash for ArenaListId<Lane> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.space.hash(state);
        self.head.hash(state);
        self.tail.hash(state);
        self.len.hash(state);
    }
}

impl<Lane> ArenaListId<Lane> {
    fn head_position(self) -> Result<LogicalPosition, ForkArenaError> {
        self.head.logical_position(self.space)
    }

    fn tail_position(self) -> Result<LogicalPosition, ForkArenaError> {
        self.tail.logical_position(self.space)
    }

    #[allow(dead_code)] // Used by the nonresident compact-node codec until its atomic cutover.
    pub(crate) const fn words(self) -> [u32; 8] {
        [
            self.space,
            self.head.raw.ordinal,
            self.head.raw.incarnation,
            self.head.offset,
            self.tail.raw.ordinal,
            self.tail.raw.incarnation,
            self.tail.offset,
            self.len,
        ]
    }

    #[allow(dead_code)] // Used by the nonresident compact-node codec until its atomic cutover.
    pub(crate) const fn from_words(words: [u32; 8]) -> Option<Self> {
        if words[7] == 0 {
            return if words[0] == 0
                && words[1] == 0
                && words[2] == 0
                && words[3] == 0
                && words[4] == 0
                && words[5] == 0
                && words[6] == 0
            {
                Some(Self::empty())
            } else {
                None
            };
        }
        if words[0] == 0 || words[2] == 0 || words[5] == 0 {
            return None;
        }
        if words[1] == words[4] && words[2] == words[5] && words[3] >= words[6] {
            return None;
        }
        Some(Self {
            space: words[0],
            head: ChunkCursor::new(
                LogicalChunkId {
                    ordinal: words[1],
                    incarnation: words[2],
                },
                words[3],
            ),
            tail: ChunkCursor::new(
                LogicalChunkId {
                    ordinal: words[4],
                    incarnation: words[5],
                },
                words[6],
            ),
            len: words[7],
        })
    }

    /// Returns the owner-independent canonical empty-list coordinate.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            space: 0,
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

    const fn from_root(
        space: u32,
        head: ChunkCursor<Lane>,
        tail: ChunkCursor<Lane>,
        len: u32,
    ) -> Self {
        debug_assert!(space != 0);
        debug_assert!(len != 0);
        debug_assert!(head.raw.incarnation != 0 && tail.raw.incarnation != 0);
        debug_assert!(
            head.raw.ordinal != tail.raw.ordinal
                || head.raw.incarnation != tail.raw.incarnation
                || head.offset < tail.offset
        );
        Self {
            space,
            head,
            tail,
            len,
        }
    }
}

const _: () = assert!(core::mem::size_of::<ArenaListId<PageMaterialLane>>() <= 32);

#[derive(Clone, Default)]
struct ChunkSet {
    payload: Vec<LogicalChunkId>,
}

/// Constant-size authentication for one arena lane's indexed live suffix.
///
/// `end` is the owner-relative position after the last live chunk. `tail` is
/// absent when every position below `end` has already been released into the
/// logical base. Module-private structural mutations update this record at
/// the same point as the authoritative chunk vectors and pool indexes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LiveChunkFrontier {
    end: usize,
    tail: Option<LogicalChunkId>,
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
    payload_tail_sealed: bool,
    payload_tail_summary: Option<SemanticSequenceIdentity>,
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
            .field("payload_tail_sealed", &self.payload_tail_sealed)
            .finish()
    }
}

/// Consuming proof that every builder has retired and both tails are sealed.
pub struct SealedBoundary<Lane> {
    arena: u32,
    payload_chunks: u32,
    payload_tail: Option<LogicalChunkId>,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

/// Opaque whole-chunk retained checkpoint coordinate.
pub struct CheckpointMark<Lane> {
    arena: u32,
    payload_chunks: u32,
    payload_tail: Option<LogicalChunkId>,
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
            && self.payload_tail == other.payload_tail
    }
}
impl<Lane> Eq for CheckpointMark<Lane> {}
impl<Lane> core::fmt::Debug for CheckpointMark<Lane> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CheckpointMark")
            .field("payload_chunks", &self.payload_chunks)
            .finish_non_exhaustive()
    }
}

/// Exclusive whole-chunk suffix available for semantic-lane promotion.
pub struct SealedBatch<Lane> {
    arena: u32,
    serial: u64,
    payload_start: u32,
    payload_end: u32,
    lists: Vec<ArenaListId<Lane>>,
}

/// Fixed-size whole-region transfer receipt.
///
/// The semantic boundary has either one declared root (the node lane) or no
/// declared root (its paired annex lane). The arena and pool retain all block
/// tables and traversal metadata; this receipt carries only scalar frontiers
/// and the optional list coordinate needed by the caller.
pub(crate) struct WholeRegionBatch<Lane> {
    arena: u32,
    serial: u64,
    payload_end: u32,
    root: Option<ArenaListId<Lane>>,
}

/// A prevalidated whole-chunk suffix temporarily loaned out of its source
/// arena. The chunk payload remains owned by the source arena until the loan
/// is either returned or committed into a destination arena.
#[allow(dead_code)] // Used by the implemented closure-transfer stage before its production carrier cutover.
pub(crate) struct DetachedBatch<Lane> {
    arena: u32,
    serial: u64,
    payload_start: u32,
    payload: Vec<LogicalChunkId>,
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
#[doc(hidden)]
pub trait RegionValue<Lane> {
    fn visit_region_lists(&self, visit: &mut dyn FnMut(ArenaListId<Lane>));
    fn rebrand_region_lists(&mut self, destination_arena: u32);
}

pub struct BatchMark<Lane> {
    arena: u32,
    payload_start: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<Lane> Clone for BatchMark<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Lane> Copy for BatchMark<Lane> {}

impl<Lane> BatchMark<Lane> {
    pub(crate) const fn payload_start(&self) -> usize {
        self.payload_start as usize
    }
}

impl<Lane> DetachedBatch<Lane> {
    pub(crate) const fn payload_start(&self) -> usize {
        self.payload_start as usize
    }
}

/// Lifecycle work counters; payload copy is absent by construction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ForkArenaCounters {
    pub new_semantic_nodes: u64,
    /// Complete payload values handed across the ordinary append boundary.
    pub whole_payload_moves: u64,
    /// Complete payload values handed across an explicit copy boundary.
    pub whole_payload_copies: u64,
    /// Payloads cloned from an admitted coordinate into their final slot.
    pub resident_payload_clones: u64,
    /// Values initialized through a borrow of their final resident slot.
    pub destination_values_constructed: u64,
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
    pub sealed_prefix_chunks_shared: u64,
    pub rootless_suffix_chunks_released: u64,
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
    ChunkShared,
    ForeignArena,
    InvalidCheckpoint,
    InvalidChunk,
    IdentityModeMismatch,
    InvalidOperationMark,
    InvalidRange,
    InvalidRegion,
    NotForked,
    TooManyLineages,
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
    lineage: u32,
    pool_owner: Option<u32>,
    ownership: ForkOwnership,
    base_payload_chunks: u32,
    payload_frontier: LiveChunkFrontier,
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
}

impl<T, Lane> Default for ForkArena<T, Lane> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, Lane> ForkArena<T, Lane> {
    fn logical_payload_view<'a>(&self, pool: &'a ChunkPool<T>) -> LogicalBlockView<'a, T> {
        match self.ownership {
            ForkOwnership::Accepted(_) => LogicalBlockView::Accepted(&pool.payload),
            ForkOwnership::Forked { .. } => LogicalBlockView::Candidate(&pool.payload),
        }
    }

    #[must_use]
    pub fn new() -> Self {
        let owner = NEXT_ARENA_OWNER.fetch_add(1, Ordering::Relaxed);
        Self {
            owner,
            lineage: NEXT_ARENA_LINEAGE.fetch_add(1, Ordering::Relaxed),
            pool_owner: None,
            ownership: ForkOwnership::Accepted(ChunkSet::default()),
            base_payload_chunks: 0,
            payload_frontier: LiveChunkFrontier::default(),
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
            lineage: NEXT_ARENA_LINEAGE.fetch_add(1, Ordering::Relaxed),
            pool_owner: None,
            ownership: ForkOwnership::Accepted(ChunkSet::default()),
            base_payload_chunks: 0,
            payload_frontier: LiveChunkFrontier::default(),
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

    fn computed_live_chunk_frontier(&self) -> LiveChunkFrontier {
        let end = self.live_payload_len();
        let base = self.base_payload_chunks as usize;
        let tail = (end != base).then(|| self.live_key_at(end - 1)).flatten();
        LiveChunkFrontier { end, tail }
    }

    fn refresh_live_chunk_frontiers(&mut self) {
        self.payload_frontier = self.computed_live_chunk_frontier();
    }

    /// Authenticates the complete live chunk suffix from its maintained tail.
    ///
    /// The ordered key vectors and pool indexes are mutated only by this
    /// module. Verifying their constant-size end record plus the tail chunk's
    /// incarnation, arena, lineage, and owner-relative position therefore
    /// admits the maintained prefix invariant without replaying every chunk.
    fn validate_live_chunks(&self, pool: &ChunkPool<T>) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        self.validate_live_chunk_frontier(pool, self.payload_frontier)
    }

    fn validate_live_chunk_frontier(
        &self,
        pool: &ChunkPool<T>,
        frontier: LiveChunkFrontier,
    ) -> Result<(), ForkArenaError> {
        if frontier != self.computed_live_chunk_frontier() {
            return Err(ForkArenaError::InvalidChunk);
        }
        let Some(key) = frontier.tail else {
            let base = self.base_payload_chunks as usize;
            return (frontier.end == base)
                .then_some(())
                .ok_or(ForkArenaError::InvalidChunk);
        };
        let position = frontier
            .end
            .checked_sub(1)
            .ok_or(ForkArenaError::InvalidChunk)?;
        let actual = pool.payload.arena_position(key, self.owner, self.lineage);
        (actual == Some(position))
            .then_some(())
            .ok_or(ForkArenaError::InvalidChunk)
    }

    pub(crate) fn can_seal_boundary(&self, pool: &ChunkPool<T>) -> Result<(), ForkArenaError> {
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        u32::try_from(self.live_payload_len()).map_err(|_| ForkArenaError::CapacityOverflow)?;
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

    /// Preflights a second arena lineage over this arena's sealed prefix.
    ///
    /// Every chunk carries exactly two bounded lineage slots. The destination
    /// receives its own chunk list and positions; payload and predecessor
    /// metadata remain single-copy and immutable.
    pub(crate) fn can_share_sealed_prefix(
        &self,
        pool: &ChunkPool<T>,
        mark: &BatchMark<Lane>,
        lists: &[ArenaListId<Lane>],
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.can_seal_boundary(pool)?;
        if self.base_payload_chunks != 0 || !matches!(self.ownership, ForkOwnership::Accepted(_)) {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.preflight_shared_prefix_metadata(pool, mark, lists)?;
        let ForkOwnership::Accepted(chunks) = &self.ownership else {
            unreachable!()
        };
        let payload_start = mark.payload_start as usize;
        for key in &chunks.payload[payload_start..] {
            let meta = pool
                .payload
                .validate_lineage(*key, self.owner, self.lineage)?;
            if meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1 {
                return Err(ForkArenaError::TooManyLineages);
            }
        }
        Ok(())
    }

    /// Creates the sole second lineage over this arena's immutable chunks.
    /// The returned arena appends only to fresh private chunks.
    pub(crate) fn share_sealed_prefix(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: BatchMark<Lane>,
        lists: &[ArenaListId<Lane>],
    ) -> Result<Self, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.can_share_sealed_prefix(pool, &mark, lists)?;
        self.seal_boundary(pool)
            .expect("shared-prefix boundary was completely preflighted");
        let ForkOwnership::Accepted(chunks) = &self.ownership else {
            unreachable!()
        };
        let shared = ChunkSet {
            payload: chunks.payload[mark.payload_start as usize..].to_vec(),
        };
        let lineage = NEXT_ARENA_LINEAGE.fetch_add(1, Ordering::Relaxed);
        for (position, key) in shared.payload.iter().copied().enumerate() {
            pool.payload
                .share_with_lineage(key, self.owner, self.lineage, lineage, position)
                .expect("shared payload lineage was completely preflighted");
        }
        let count = shared.payload.len() as u64;
        self.counters.sealed_prefix_chunks_shared = self
            .counters
            .sealed_prefix_chunks_shared
            .saturating_add(count);
        let counters = self.counters;
        let mut shared = Self {
            owner: self.owner,
            lineage,
            pool_owner: self.pool_owner,
            ownership: ForkOwnership::Accepted(shared),
            base_payload_chunks: 0,
            payload_frontier: LiveChunkFrontier::default(),
            active_builder: false,
            pending_batch: None,
            next_batch_serial: 1,
            counters,
            _types: PhantomData,
        };
        shared.refresh_live_chunk_frontiers();
        Ok(shared)
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
        self.refresh_live_chunk_frontiers();
        Ok(())
    }

    #[must_use]
    pub fn payload_chunk_capacity(&self, pool: &ChunkPool<T>) -> usize {
        pool.payload.chunk_capacity()
    }

    pub(crate) fn live_payload_chunks(&self) -> usize {
        self.live_payload_len()
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn profiling_current_physical_tokens(&self, pool: &ChunkPool<T>) -> Vec<u64> {
        let sets = match &self.ownership {
            ForkOwnership::Accepted(accepted) => [&accepted.payload[..], &[][..]],
            ForkOwnership::Forked {
                prefix, current, ..
            } => [&prefix.payload[..], &current.payload[..]],
        };
        sets.into_iter()
            .flatten()
            .filter_map(|key| pool.payload.profiling_physical_token(*key))
            .collect()
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn profiling_prior_physical_tokens(&self, pool: &ChunkPool<T>) -> Vec<u64> {
        let ForkOwnership::Forked {
            prefix,
            detached_prior,
            ..
        } = &self.ownership
        else {
            return Vec::new();
        };
        prefix
            .payload
            .iter()
            .chain(&detached_prior.payload)
            .filter_map(|key| pool.payload.profiling_physical_token(*key))
            .collect()
    }

    pub(crate) fn rebase_paired_dependency_suffix(
        &mut self,
        pool: &mut ChunkPool<T>,
        payload_start: usize,
        source_paired_start: usize,
        destination_paired_start: usize,
    ) -> Result<(), ForkArenaError> {
        for position in payload_start..self.live_payload_len() {
            let key = self
                .live_key_at(position)
                .ok_or(ForkArenaError::InvalidChunk)?;
            let meta =
                pool.payload
                    .validate_exclusive_lineage_mut(key, self.owner, self.lineage)?;
            if meta.paired_dependency_floor == usize::MAX {
                continue;
            }
            let relative = meta
                .paired_dependency_floor
                .checked_sub(source_paired_start)
                .ok_or(ForkArenaError::InvalidRegion)?;
            meta.paired_dependency_floor = destination_paired_start
                .checked_add(relative)
                .ok_or(ForkArenaError::CapacityOverflow)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn live_payload_values(&self, pool: &ChunkPool<T>) -> usize {
        (0..self.live_payload_len())
            .map(|index| {
                self.live_key_at(index)
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

    fn current_chunks_mut(&mut self) -> &mut ChunkSet {
        match &mut self.ownership {
            ForkOwnership::Accepted(chunks) => chunks,
            ForkOwnership::Forked { current, .. } => current,
        }
    }

    fn live_key_at(&self, index: usize) -> Option<LogicalChunkId> {
        let base = self.base_payload_chunks as usize;
        let index = index.checked_sub(base)?;
        match &self.ownership {
            ForkOwnership::Accepted(chunks) => chunks.payload.get(index).copied(),
            ForkOwnership::Forked {
                prefix, current, ..
            } => {
                let prefix_len = prefix.payload.len();
                if index < prefix_len {
                    prefix.payload.get(index).copied()
                } else {
                    let index = index - prefix_len;
                    current.payload.get(index).copied()
                }
            }
        }
    }

    fn index_chunk(&self, pool: &mut ChunkPool<T>, key: LogicalChunkId, position: usize) {
        let result = pool
            .payload
            .index_in_arena(key, self.owner, self.lineage, position);
        result.expect("arena index names its live owned chunk");
    }

    fn unindex_chunk(&self, pool: &mut ChunkPool<T>, key: LogicalChunkId) {
        pool.payload
            .unindex_from_arena(key, self.owner, self.lineage);
    }

    fn resolved_position(&self, pool: &ChunkPool<T>, key: LogicalChunkId) -> Option<usize> {
        pool.payload.arena_position(key, self.owner, self.lineage)
    }

    /// Reserves the one final resident payload slot and publishes its root.
    #[cfg(test)]
    fn reserve_payload_slot<'pool>(
        &mut self,
        pool: &'pool mut ChunkPool<T>,
        root: &mut ArenaListId<Lane>,
        item_identity: Option<u64>,
    ) -> Result<&'pool mut Option<T>, ForkArenaError> {
        self.reserve_payload_slot_with_dependency(pool, root, item_identity, None, false)
    }

    fn reserve_payload_slot_with_dependency<'pool>(
        &mut self,
        pool: &'pool mut ChunkPool<T>,
        root: &mut ArenaListId<Lane>,
        item_identity: Option<u64>,
        dependency_floor: Option<usize>,
        dependency_metadata_complete: bool,
    ) -> Result<&'pool mut Option<T>, ForkArenaError> {
        let key = self.payload_reservation_target(pool, root)?;
        let logical_space = pool.payload.logical_space();
        let ReservedChunkSlot {
            slot,
            offset,
            became_full,
        } = pool.payload.reserve(
            key,
            self.owner,
            self.lineage,
            item_identity,
            dependency_floor,
            dependency_metadata_complete,
        )?;
        self.complete_payload_reservation(root, key, offset, became_full, logical_space)?;
        Ok(slot)
    }

    fn append_payload_value_with_dependency(
        &mut self,
        pool: &mut ChunkPool<T>,
        root: &mut ArenaListId<Lane>,
        value: T,
        item_identity: Option<u64>,
        dependency_floor: Option<usize>,
        dependency_metadata_complete: bool,
    ) -> Result<(), ForkArenaError> {
        let key = self.payload_reservation_target(pool, root)?;
        let logical_space = pool.payload.logical_space();
        let ReservedChunkPosition {
            page,
            index,
            offset,
            became_full,
        } = pool.payload.reserve_position(
            key,
            self.owner,
            self.lineage,
            item_identity,
            dependency_floor,
            dependency_metadata_complete,
        )?;
        pool.payload.blocks[page]
            .payload_mut()
            .insert(index, value)?;
        self.complete_payload_reservation(root, key, offset, became_full, logical_space)
    }

    fn payload_reservation_target(
        &mut self,
        pool: &mut ChunkPool<T>,
        root: &ArenaListId<Lane>,
    ) -> Result<LogicalChunkId, ForkArenaError> {
        if !root.is_empty() && root.space != pool.payload.logical_space() {
            return Err(ForkArenaError::ForeignArena);
        }
        let current_tail = self.current_chunks_mut().payload.last().copied();
        let reusable = (pool.payload.layout == ChunkStorageLayout::PackedCopy || !root.is_empty())
            .then_some(current_tail)
            .flatten()
            .filter(|key| {
                !pool.payload.is_sealed(*key, self.owner).unwrap_or(true)
                    && pool
                        .payload
                        .used(*key, self.owner)
                        .ok()
                        .is_some_and(|used| {
                            (root.is_empty() || used == root.tail.offset)
                                && used as usize != pool.payload.chunk_capacity()
                        })
            });
        let key = match reusable {
            Some(key) => key,
            None => {
                self.bind_pool(pool)?;
                let previous = (!root.is_empty()).then_some((root.tail.raw, root.tail.offset));
                let key = pool
                    .payload
                    .allocate_list_block(self.owner, self.lineage, previous)?;
                self.current_chunks_mut().payload.push(key);
                self.counters.direct_blocks_allocated =
                    self.counters.direct_blocks_allocated.saturating_add(1);
                let position = self.live_payload_len() - 1;
                self.index_chunk(pool, key, position);
                self.refresh_live_chunk_frontiers();
                key
            }
        };
        Ok(key)
    }

    fn complete_payload_reservation(
        &mut self,
        root: &mut ArenaListId<Lane>,
        key: LogicalChunkId,
        offset: u32,
        became_full: bool,
        logical_space: u32,
    ) -> Result<(), ForkArenaError> {
        if root.is_empty() {
            *root = ArenaListId::from_root(
                logical_space,
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
        if became_full {
            self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        }
        self.counters.new_semantic_nodes += 1;
        Ok(())
    }

    /// Publishes one value appended through an admitted block continuation.
    /// The continuation is discarded before the `u32` list length is
    /// exhausted, so only scalar root and counter advancement remains.
    fn complete_admitted_payload_reservation(
        &mut self,
        root: &mut ArenaListId<Lane>,
        key: LogicalChunkId,
        offset: u32,
        became_full: bool,
        logical_space: u32,
    ) {
        if root.is_empty() {
            *root = ArenaListId::from_root(
                logical_space,
                ChunkCursor::new(key, offset),
                ChunkCursor::new(key, offset + 1),
                1,
            );
        } else {
            root.tail = ChunkCursor::new(key, offset + 1);
            root.len += 1;
        }
        if became_full {
            self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        }
        self.counters.new_semantic_nodes += 1;
    }

    /// Publishes the scalar root and counters for one already-initialized
    /// contiguous physical-block run.
    fn complete_admitted_payload_run(
        &mut self,
        root: &mut ArenaListId<Lane>,
        key: LogicalChunkId,
        start: u32,
        count: u32,
        became_full: bool,
        logical_space: u32,
    ) {
        debug_assert!(count != 0);
        let end = start + count;
        if root.is_empty() {
            *root = ArenaListId::from_root(
                logical_space,
                ChunkCursor::new(key, start),
                ChunkCursor::new(key, end),
                count,
            );
        } else {
            root.tail = ChunkCursor::new(key, end);
            root.len += count;
        }
        if became_full {
            self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        }
        self.counters.new_semantic_nodes += u64::from(count);
    }

    /// Clones one same-arena source cell directly into its final packed slot.
    fn append_payload_clone_from_coordinate(
        &mut self,
        pool: &mut ChunkPool<T>,
        root: &mut ArenaListId<Lane>,
        source_key: LogicalChunkId,
        source_offset: u32,
        item_identity: Option<u64>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone + RegionValue<Lane>,
    {
        let dependency_floor = {
            let source = pool
                .payload
                .get(source_key, self.owner, source_offset)
                .ok_or(ForkArenaError::InvalidRange)?;
            self.region_value_dependency_floor(pool, source)?
        };
        let key = self.payload_reservation_target(pool, root)?;
        let (offset, became_full) = pool.payload.reserve_clone_from(
            key,
            self.owner,
            self.lineage,
            CloneReservation {
                item_identity,
                dependency_floor,
                dependency_metadata_complete: true,
                source_arena: self.owner,
                source_key,
                source_offset,
            },
        )?;
        self.complete_payload_reservation(
            root,
            key,
            offset,
            became_full,
            pool.payload.logical_space(),
        )?;
        self.counters.whole_payload_copies = self.counters.whole_payload_copies.saturating_add(1);
        self.counters.resident_payload_clones =
            self.counters.resident_payload_clones.saturating_add(1);
        Ok(())
    }

    /// Clones one cross-arena source cell into its final slot, rewrites that
    /// resident value, then completes identity and child dependency metadata.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    fn append_payload_mapped_clone_from_coordinate(
        &mut self,
        pool: &mut ChunkPool<T>,
        root: &mut ArenaListId<Lane>,
        source_arena: u32,
        source_key: LogicalChunkId,
        source_offset: u32,
        identity_enabled: bool,
        rewrite: &mut impl FnMut(&mut T) -> Result<Option<u64>, ForkArenaError>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone + RegionValue<Lane>,
    {
        let placeholder_identity = identity_enabled.then_some(0);
        let key = self.payload_reservation_target(pool, root)?;
        let (offset, became_full) = pool.payload.reserve_clone_from(
            key,
            self.owner,
            self.lineage,
            CloneReservation {
                item_identity: placeholder_identity,
                dependency_floor: None,
                dependency_metadata_complete: false,
                source_arena,
                source_key,
                source_offset,
            },
        )?;
        self.complete_payload_reservation(
            root,
            key,
            offset,
            became_full,
            pool.payload.logical_space(),
        )?;
        self.counters.whole_payload_copies = self.counters.whole_payload_copies.saturating_add(1);
        self.counters.resident_payload_clones =
            self.counters.resident_payload_clones.saturating_add(1);

        let item_identity = {
            let value = pool
                .payload
                .get_mut(key, self.owner, self.lineage, offset)
                .ok_or(ForkArenaError::InvalidRange)?;
            rewrite(value)?
        };
        if item_identity.is_some() != identity_enabled {
            return Err(ForkArenaError::IdentityModeMismatch);
        }
        let dependency_floor = {
            let value = pool
                .payload
                .get(key, self.owner, offset)
                .ok_or(ForkArenaError::InvalidRange)?;
            self.region_value_dependency_floor(pool, value)?
        };
        pool.payload.complete_reservation(
            key,
            self.owner,
            self.lineage,
            offset,
            ReservationCompletion {
                placeholder_identity,
                item_identity,
                dependency_floor,
            },
        )?;
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
    #[cfg(test)]
    pub(crate) fn clone_mapped_list_from(
        &mut self,
        pool: &mut ChunkPool<T>,
        source: &ForkArena<T, Lane>,
        list: ArenaListId<Lane>,
        identity_enabled: bool,
        mut rewrite: impl FnMut(&mut T) -> Result<Option<u64>, ForkArenaError>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError>
    where
        T: Clone + RegionValue<Lane>,
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
                identity_enabled,
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
    #[cfg(test)]
    fn clone_chunk_prefix_from(
        &mut self,
        pool: &mut ChunkPool<T>,
        source: &ForkArena<T, Lane>,
        list: ArenaListId<Lane>,
        key: LogicalChunkId,
        end: u32,
        root: &mut ArenaListId<Lane>,
        identity_enabled: bool,
        rewrite: &mut impl FnMut(&mut T) -> Result<Option<u64>, ForkArenaError>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone + RegionValue<Lane>,
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
                identity_enabled,
                rewrite,
            )?;
            0
        };
        for offset in start..end {
            self.append_payload_mapped_clone_from_coordinate(
                pool,
                root,
                source.owner,
                key,
                offset,
                identity_enabled,
                rewrite,
            )?;
        }
        Ok(())
    }

    #[must_use]
    pub fn operation_mark(&self, pool: &ChunkPool<T>) -> OperationMark<Lane> {
        let payload_tail_used = self
            .live_key_at(self.live_payload_len().saturating_sub(1))
            .and_then(|key| pool.payload.used(key, self.owner).ok())
            .unwrap_or(0);
        let payload_tail_summary = self
            .live_key_at(self.live_payload_len().saturating_sub(1))
            .and_then(|key| pool.payload.sequence_summary(key, self.owner).ok())
            .flatten();
        let payload_tail_sealed = self
            .live_key_at(self.live_payload_len().saturating_sub(1))
            .and_then(|key| pool.payload.is_sealed(key, self.owner).ok())
            .unwrap_or(false);
        OperationMark {
            arena: self.owner,
            payload_chunks: self.live_payload_len() as u32,
            payload_tail_used,
            payload_tail_sealed,
            payload_tail_summary,
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
        self.truncate_payload(
            pool,
            mark.payload_chunks as usize,
            mark.payload_tail_used,
            mark.payload_tail_sealed,
            mark.payload_tail_summary,
        )
    }

    fn truncate_payload(
        &mut self,
        pool: &mut ChunkPool<T>,
        chunks: usize,
        tail_used: u32,
        tail_sealed: bool,
        tail_summary: Option<SemanticSequenceIdentity>,
    ) -> Result<(), ForkArenaError> {
        let live_len = self.live_payload_len();
        let base = self.base_payload_chunks as usize;
        if chunks < base || chunks > live_len {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        while self.live_payload_len() > chunks {
            let key = {
                let current = self.current_chunks_mut();
                current
                    .payload
                    .pop()
                    .ok_or(ForkArenaError::InvalidOperationMark)?
            };
            self.unindex_chunk(pool, key);
            pool.payload
                .release_lineage(key, self.owner, self.lineage)?;
            self.counters.candidate_chunks_truncated += 1;
        }
        if chunks != base {
            let key = self
                .live_key_at(chunks - 1)
                .ok_or(ForkArenaError::InvalidOperationMark)?;
            pool.payload
                .truncate(key, self.owner, self.lineage, tail_used, tail_summary)?;
            pool.payload
                .restore_sealed(key, self.owner, self.lineage, tail_sealed)?;
        } else if tail_used != 0 {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        self.refresh_live_chunk_frontiers();
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
            append_block: None,
            #[cfg(test)]
            sequence_summary: None,
            finished: false,
        })
    }

    /// Appends one independently addressed list without sealing its physical
    /// tail, allowing subsequent small typed records to share the same exact
    /// initialized-prefix block. Region boundaries seal that tail before any
    /// fork, transfer, or retained checkpoint can observe it.
    #[allow(dead_code)] // Retained for the opt-in generic arena benchmark adapter.
    pub(crate) fn append_unsealed_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        values: impl IntoIterator<Item = T>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        self.bind_pool(pool)?;
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let operation = self.operation_mark(pool);
        self.append_unsealed_list_from_mark(pool, values, operation)
    }

    #[allow(dead_code)] // Retained for the opt-in generic arena benchmark adapter.
    fn append_unsealed_list_from_mark(
        &mut self,
        pool: &mut ChunkPool<T>,
        values: impl IntoIterator<Item = T>,
        operation: OperationMark<Lane>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        let mut root = ArenaListId::empty();
        let logical_space = pool.payload.logical_space();
        let mut values = values.into_iter();
        while let Some(first) = values.next() {
            let appended = (|| {
                if root.len == u32::MAX {
                    return Err(ForkArenaError::CapacityOverflow);
                }
                let key = self.payload_reservation_target(pool, &root)?;
                let (start, count, became_full) = {
                    let mut run =
                        pool.payload
                            .admit_untracked_append_run(key, self.owner, self.lineage)?;
                    let start = run.offset;
                    run.push(first);
                    let mut count = 1_u32;
                    while run.has_capacity() && root.len < u32::MAX - count {
                        let Some(value) = values.next() else {
                            break;
                        };
                        run.push(value);
                        count += 1;
                    }
                    (start, count, run.is_full())
                };
                self.complete_admitted_payload_run(
                    &mut root,
                    key,
                    start,
                    count,
                    became_full,
                    logical_space,
                );
                Ok::<(), ForkArenaError>(())
            })();
            if let Err(error) = appended {
                self.restore_operation(pool, operation)
                    .expect("unsealed list append restores its own operation mark");
                return Err(error);
            }
        }
        Ok(root)
    }

    /// Publishes one copied header followed by one borrowed `Copy` body.
    ///
    /// Each physical destination run is admitted once, filled directly from
    /// the caller's final source slices, and settled once. No iterator or
    /// element-sized arena publication capability survives inside the run.
    pub(crate) fn append_unsealed_copy_parts(
        &mut self,
        pool: &mut ChunkPool<T>,
        header: T,
        body: &[T],
    ) -> Result<ArenaListId<Lane>, ForkArenaError>
    where
        T: Copy,
    {
        self.bind_pool(pool)?;
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let operation = self.operation_mark(pool);
        self.append_unsealed_copy_parts_from_mark(pool, header, body, operation)
    }

    fn append_unsealed_copy_parts_from_mark(
        &mut self,
        pool: &mut ChunkPool<T>,
        header_value: T,
        body: &[T],
        operation: OperationMark<Lane>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError>
    where
        T: Copy,
    {
        let logical_space = pool.payload.logical_space();
        let mut root = ArenaListId::empty();
        let mut header = Some(header_value);
        let mut body_start = 0_usize;
        while header.is_some() || body_start < body.len() {
            let appended = (|| {
                let remaining = body.len() - body_start + usize::from(header.is_some());
                let root_capacity = (u32::MAX - root.len) as usize;
                if root_capacity == 0 {
                    return Err(ForkArenaError::CapacityOverflow);
                }
                let key = self.payload_reservation_target(pool, &root)?;
                let (start, count, became_full) = {
                    let mut run =
                        pool.payload
                            .admit_untracked_append_run(key, self.owner, self.lineage)?;
                    let start = run.offset;
                    let limit = remaining.min(root_capacity);
                    let body_end = body_start + limit.saturating_sub(usize::from(header.is_some()));
                    let count = run.extend_copy_parts(header, &body[body_start..body_end]);
                    (start, count as u32, run.is_full())
                };
                if count == 0 {
                    return Err(ForkArenaError::CapacityOverflow);
                }
                if header.take().is_some() {
                    body_start += count as usize - 1;
                } else {
                    body_start += count as usize;
                }
                self.complete_admitted_payload_run(
                    &mut root,
                    key,
                    start,
                    count,
                    became_full,
                    logical_space,
                );
                Ok(())
            })();
            if let Err(error) = appended {
                self.restore_operation(pool, operation)
                    .expect("copy-parts append restores its own operation mark");
                return Err(error);
            }
        }
        Ok(root)
    }

    /// Fixed-record counterpart of [`Self::append_unsealed_copy_parts`].
    /// Rotation happens before the one admitted destination run so the record
    /// remains wholly inside one logical chunk.
    pub(crate) fn append_unsealed_fixed_copy_parts(
        &mut self,
        pool: &mut ChunkPool<T>,
        header: T,
        body: &[T],
    ) -> Result<ArenaListId<Lane>, ForkArenaError>
    where
        T: Copy,
    {
        self.bind_pool(pool)?;
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let needed = body
            .len()
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        if needed > pool.payload.chunk_capacity() {
            return Err(ForkArenaError::CapacityOverflow);
        }
        let operation = self.operation_mark(pool);
        if let Some(key) = self.live_key_at(self.live_payload_len().saturating_sub(1)) {
            let used = pool.payload.used(key, self.owner)? as usize;
            if !pool.payload.is_sealed(key, self.owner)?
                && pool.payload.chunk_capacity().saturating_sub(used) < needed
            {
                let unused = pool.payload.seal(key, self.owner)?;
                self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
                self.counters.unused_sealed_bytes = self
                    .counters
                    .unused_sealed_bytes
                    .saturating_add((unused * pool.payload.resident_slot_bytes()) as u64);
            }
        }
        self.append_unsealed_copy_parts_from_mark(pool, header, body, operation)
    }

    /// Focused performance-gate access to ordinary unsealed packed-list
    /// publication. Production codecs remain the only non-testing caller.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn testing_append_unsealed_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        values: impl IntoIterator<Item = T>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        self.append_unsealed_list(pool, values)
    }

    /// Appends one independently addressed fixed record wholly within one
    /// logical chunk, rotating a short tail before publication when needed.
    #[allow(dead_code)] // Compatibility helper for non-slice test payloads.
    pub(crate) fn append_unsealed_fixed_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        values: impl IntoIterator<Item = T>,
    ) -> Result<ArenaListId<Lane>, ForkArenaError> {
        self.bind_pool(pool)?;
        if self.active_builder {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let values = values.into_iter();
        let (needed, upper) = values.size_hint();
        if upper != Some(needed) {
            return Err(ForkArenaError::InvalidRange);
        }
        if needed > pool.payload.chunk_capacity() {
            return Err(ForkArenaError::CapacityOverflow);
        }
        let operation = self.operation_mark(pool);
        if let Some(key) = self.live_key_at(self.live_payload_len().saturating_sub(1)) {
            let used = pool.payload.used(key, self.owner)? as usize;
            if !pool.payload.is_sealed(key, self.owner)?
                && pool.payload.chunk_capacity().saturating_sub(used) < needed
            {
                let unused = pool.payload.seal(key, self.owner)?;
                self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
                self.counters.unused_sealed_bytes = self
                    .counters
                    .unused_sealed_bytes
                    .saturating_add((unused * pool.payload.resident_slot_bytes()) as u64);
            }
        }
        self.append_unsealed_list_from_mark(pool, values, operation)
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
        self.counters.whole_payload_moves = self.counters.whole_payload_moves.saturating_add(1);
        let mut root = self.active_list_open_mut(builder)?.root;
        self.append_payload_value_with_dependency(pool, &mut root, value, None, None, false)?;
        self.active_list_open_mut(builder)?.root = root;
        Ok(())
    }

    pub(crate) fn owner_relative_head_position(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<usize, ForkArenaError> {
        self.validate_list(pool, list)?;
        self.resolved_position(pool, list.head.raw)
            .ok_or(ForkArenaError::InvalidRange)
    }

    pub(crate) fn preflight_paired_dependency_floor(
        &self,
        pool: &ChunkPool<T>,
        lists: &[ArenaListId<Lane>],
        paired_start: usize,
    ) -> Result<(), ForkArenaError> {
        for list in lists {
            self.validate_list(pool, *list)?;
            if list.is_empty() {
                continue;
            }
            let floor = pool
                .payload
                .validate(list.tail.raw, self.owner)?
                .paired_dependency_floor;
            if floor < paired_start {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    /// Reserves and initializes one generated region value in its final slot.
    ///
    /// The initializer receives the resident `Option<T>` directly. Identity,
    /// child-region metadata, and any context-paired dependency floor are
    /// derived only after that slot contains the final value. A rejected child
    /// coordinate truncates the one unpublished reservation and restores the
    /// active root exactly.
    pub(crate) fn construct_region_value_active_list<Context, Observation, Dependencies>(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        identity_enabled: bool,
        context: &mut Context,
        initialize: impl FnOnce(&mut Option<T>, &mut Context),
        inspect: impl FnOnce(
            &T,
            &Context,
        )
            -> Result<(u64, Observation, Dependencies, Option<usize>), ForkArenaError>,
    ) -> Result<(Option<u64>, Observation), ForkArenaError>
    where
        Dependencies: RegionValue<Lane>,
    {
        let operation = self.operation_mark(pool);
        let previous_root = self.active_list_open_mut(builder)?.root;
        let placeholder_identity = identity_enabled.then_some(0);
        let mut root = previous_root;
        {
            let slot = match self.reserve_payload_slot_with_dependency(
                pool,
                &mut root,
                placeholder_identity,
                None,
                false,
            ) {
                Ok(slot) => slot,
                Err(error) => {
                    self.active_builder = false;
                    let restored = self.restore_operation(pool, operation);
                    self.active_builder = true;
                    restored.expect("failed destination reservation restores its exact suffix");
                    return Err(error);
                }
            };
            assert!(
                slot.is_none(),
                "reserved construction destination is vacant"
            );
            initialize(slot, context);
            assert!(
                slot.is_some(),
                "node initializer fills its reserved destination"
            );
        }
        self.active_list_open_mut(builder)?.root = root;

        let key = root.tail.raw;
        let offset = root.tail.offset - 1;
        let completed = (|| {
            let value = pool
                .payload
                .get(key, self.owner, offset)
                .ok_or(ForkArenaError::InvalidRange)?;
            let (identity, observation, dependencies, context_paired_dependency_floor) =
                inspect(value, context)?;
            let dependency_floor = self.region_value_dependency_floor(pool, &dependencies)?;
            let paired_dependency_floor = [
                self.paired_dependency_floor_for(pool, &dependencies)?,
                context_paired_dependency_floor,
            ]
            .into_iter()
            .flatten()
            .min();
            let item_identity = identity_enabled.then_some(identity);
            pool.payload.complete_reservation(
                key,
                self.owner,
                self.lineage,
                offset,
                ReservationCompletion {
                    placeholder_identity,
                    item_identity,
                    dependency_floor,
                },
            )?;
            if let Some(paired_dependency_floor) = paired_dependency_floor {
                let meta =
                    pool.payload
                        .validate_exclusive_lineage_mut(key, self.owner, self.lineage)?;
                meta.paired_dependency_floor =
                    meta.paired_dependency_floor.min(paired_dependency_floor);
            }
            Ok((item_identity, observation))
        })();
        if completed.is_err() {
            self.active_list_open_mut(builder)?.root = previous_root;
            self.active_builder = false;
            let restored = self.restore_operation(pool, operation);
            self.active_builder = true;
            restored.expect("failed destination construction restores its exact suffix");
            self.counters.new_semantic_nodes = self.counters.new_semantic_nodes.saturating_sub(1);
        }
        if completed.is_ok() {
            self.counters.destination_values_constructed = self
                .counters
                .destination_values_constructed
                .saturating_add(1);
        }
        completed
    }

    /// Returns the admitted final place for one newly created payload.
    #[cfg(test)]
    pub(crate) fn reserve_active_list_slot<'pool>(
        &mut self,
        pool: &'pool mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        item_identity: Option<u64>,
    ) -> Result<&'pool mut Option<T>, ForkArenaError> {
        let mut root = self.active_list_open_mut(builder)?.root;
        let slot = self.reserve_payload_slot(pool, &mut root, item_identity)?;
        self.active_list_open_mut(builder)?.root = root;
        Ok(slot)
    }

    /// Reserves one page-node slot while folding its bounded direct child
    /// coordinates into chunk dependency metadata. This visits only fields of
    /// the value being published; later lineage sharing never walks payload.
    #[cfg(test)]
    pub(crate) fn reserve_region_value_active_list_slot<'pool>(
        &mut self,
        pool: &'pool mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        value: &impl RegionValue<Lane>,
        item_identity: Option<u64>,
    ) -> Result<&'pool mut Option<T>, ForkArenaError> {
        let dependency_floor = self.region_value_dependency_floor(pool, value)?;
        let mut root = self.active_list_open_mut(builder)?.root;
        let slot = self.reserve_payload_slot_with_dependency(
            pool,
            &mut root,
            item_identity,
            dependency_floor,
            true,
        )?;
        self.active_list_open_mut(builder)?.root = root;
        Ok(slot)
    }

    fn region_value_dependency_floor<U: RegionValue<Lane>>(
        &self,
        pool: &ChunkPool<T>,
        value: &U,
    ) -> Result<Option<usize>, ForkArenaError> {
        let mut dependency_floor = usize::MAX;
        let mut valid = true;
        value.visit_region_lists(&mut |list| {
            if list.is_empty() {
                return;
            }
            if self.validate_list(pool, list).is_err() {
                valid = false;
                return;
            }
            let Some(head) = self.resolved_position(pool, list.head.raw) else {
                valid = false;
                return;
            };
            dependency_floor = dependency_floor.min(head);
        });
        if !valid {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok((dependency_floor != usize::MAX).then_some(dependency_floor))
    }

    pub(crate) fn paired_dependency_floor_for(
        &self,
        pool: &ChunkPool<T>,
        value: &impl RegionValue<Lane>,
    ) -> Result<Option<usize>, ForkArenaError> {
        let mut paired_dependency_floor = usize::MAX;
        let mut valid = true;
        value.visit_region_lists(&mut |list| {
            if list.is_empty() {
                return;
            }
            match self
                .validate_list(pool, list)
                .and_then(|()| pool.payload.validate(list.tail.raw, self.owner))
            {
                Ok(meta) => {
                    paired_dependency_floor =
                        paired_dependency_floor.min(meta.paired_dependency_floor);
                }
                Err(_) => valid = false,
            }
        });
        if !valid {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok((paired_dependency_floor != usize::MAX).then_some(paired_dependency_floor))
    }

    pub(crate) fn dependency_floors_for_region_lists(
        &self,
        pool: &ChunkPool<T>,
        visit_lists: impl FnOnce(&mut dyn FnMut(ArenaListId<Lane>)) -> Option<()>,
    ) -> Result<(Option<usize>, Option<usize>), ForkArenaError> {
        let mut dependency_floor = usize::MAX;
        let mut paired_dependency_floor = usize::MAX;
        let mut valid = true;
        let decoded = visit_lists(&mut |list| {
            if list.is_empty() || !valid {
                return;
            }
            match self.validate_list(pool, list).and_then(|()| {
                let head = self
                    .resolved_position(pool, list.head.raw)
                    .ok_or(ForkArenaError::InvalidRange)?;
                let meta = pool.payload.validate(list.tail.raw, self.owner)?;
                Ok((head, meta.paired_dependency_floor))
            }) {
                Ok((head, paired)) => {
                    dependency_floor = dependency_floor.min(head);
                    paired_dependency_floor = paired_dependency_floor.min(paired);
                }
                Err(_) => valid = false,
            }
        });
        if decoded.is_none() || !valid {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok((
            (dependency_floor != usize::MAX).then_some(dependency_floor),
            (paired_dependency_floor != usize::MAX).then_some(paired_dependency_floor),
        ))
    }

    pub(crate) fn paired_dependency_floor_for_list(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<Option<usize>, ForkArenaError> {
        self.validate_list(pool, list)?;
        if list.is_empty() {
            return Ok(None);
        }
        let floor = pool
            .payload
            .validate(list.tail.raw, self.owner)?
            .paired_dependency_floor;
        Ok((floor != usize::MAX).then_some(floor))
    }

    pub(crate) fn begin_reencoded_active_list_copy(
        &mut self,
        source_len: usize,
        selected_len: usize,
    ) {
        if selected_len != source_len {
            self.record_partial_edge_nodes_copied(selected_len);
        }
    }

    #[allow(dead_code)] // Compatibility helper for scalar construction tests.
    pub(crate) fn append_reencoded_active_list_copy_value(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        value: T,
        item_identity: Option<u64>,
        dependency_floor: Option<usize>,
        paired_dependency_floor: Option<usize>,
    ) -> Result<(), ForkArenaError> {
        let mut root = self.active_list_open_mut(builder)?.root;
        self.append_payload_value_with_dependency(
            pool,
            &mut root,
            value,
            item_identity,
            dependency_floor,
            true,
        )?;
        if let Some(paired_dependency_floor) = paired_dependency_floor {
            let meta = pool.payload.validate_exclusive_lineage_mut(
                root.tail.raw,
                self.owner,
                self.lineage,
            )?;
            meta.paired_dependency_floor =
                meta.paired_dependency_floor.min(paired_dependency_floor);
        }
        self.active_list_open_mut(builder)?.root = root;
        self.counters.whole_payload_copies = self.counters.whole_payload_copies.saturating_add(1);
        self.counters.resident_payload_clones =
            self.counters.resident_payload_clones.saturating_add(1);
        self.counters.new_semantic_nodes = self.counters.new_semantic_nodes.saturating_sub(1);
        self.record_source_nodes_copied(1);
        Ok(())
    }

    /// Transforms one admitted source subspan into one exact final destination
    /// run. Source and destination may occupy distinct ranges of the same
    /// physical optional-slot block; the storage layer lends disjoint slices
    /// in that case. Payload and chunk metadata settle once after the run.
    pub(crate) fn transform_admitted_active_list_run(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        source: &AdmittedListChunkCursor<Lane>,
        selected: Range<usize>,
        identity_enabled: bool,
        mut transform: impl FnMut(
            &Self,
            &T,
            &mut Option<T>,
        ) -> Result<ConstructedRunValue, ForkArenaError>,
    ) -> Result<usize, ForkArenaError> {
        if selected.start > selected.end || selected.end > source.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        if selected.is_empty() {
            return Ok(0);
        }
        let operation = self.operation_mark(pool);
        let previous_root = self.active_list_open_mut(builder)?.root;
        let mut root = previous_root;
        let key = self.payload_reservation_target(pool, &root)?;
        let used = pool.payload.used(key, self.owner)? as usize;
        let count = selected
            .len()
            .min(pool.payload.chunk_capacity().saturating_sub(used))
            .min((u32::MAX - root.len) as usize);
        if count == 0 {
            return Err(ForkArenaError::CapacityOverflow);
        }
        let (destination_start, prefix_summary) = pool.payload.reserve_optional_run(
            key,
            self.owner,
            self.lineage,
            count,
            identity_enabled,
        )?;
        let source_start = source.start
            + u32::try_from(selected.start).map_err(|_| ForkArenaError::CapacityOverflow)?;
        let source_end =
            source_start + u32::try_from(count).map_err(|_| ForkArenaError::CapacityOverflow)?;
        let destination_end = destination_start + count as u32;
        let mut run_summary = identity_enabled.then(SemanticSequenceIdentity::empty);
        let mut dependency_floor = usize::MAX;
        let mut paired_dependency_floor = usize::MAX;
        let transformed = pool.payload.with_optional_source_destination(
            source.block,
            source_start..source_end,
            key,
            destination_start..destination_end,
            |source, destination| {
                for (source, destination) in source.iter().zip(destination) {
                    let source = source.as_ref().expect("admitted source run is initialized");
                    let metadata = transform(&*self, source, destination)?;
                    if destination.is_none() {
                        return Err(ForkArenaError::InvalidRange);
                    }
                    match (&mut run_summary, metadata.item_identity) {
                        (Some(summary), Some(identity)) => summary.push_back(identity),
                        (None, None) => {}
                        _ => return Err(ForkArenaError::IdentityModeMismatch),
                    }
                    if let Some(floor) = metadata.dependency_floor {
                        dependency_floor = dependency_floor.min(floor);
                    }
                    if let Some(floor) = metadata.paired_dependency_floor {
                        paired_dependency_floor = paired_dependency_floor.min(floor);
                    }
                }
                Ok(())
            },
        );
        if let Err(error) = transformed {
            self.active_builder = false;
            let restored = self.restore_operation(pool, operation);
            self.active_builder = true;
            restored.expect("failed admitted run restores its exact destination suffix");
            return Err(error);
        }
        {
            let meta =
                pool.payload
                    .validate_exclusive_lineage_mut(key, self.owner, self.lineage)?;
            meta.sequence_summary = match (prefix_summary, run_summary) {
                (Some(prefix), Some(run)) => Some(prefix.concat(run)),
                (None, Some(run)) => Some(run),
                (None, None) => None,
                (Some(_), None) => return Err(ForkArenaError::IdentityModeMismatch),
            };
            meta.dependency_floor = meta.dependency_floor.min(dependency_floor);
            meta.paired_dependency_floor =
                meta.paired_dependency_floor.min(paired_dependency_floor);
            meta.dependency_metadata_complete = true;
        }
        self.complete_admitted_payload_run(
            &mut root,
            key,
            destination_start,
            count as u32,
            destination_end as usize == pool.payload.chunk_capacity(),
            pool.payload.logical_space(),
        );
        self.active_list_open_mut(builder)?.root = root;
        self.counters.whole_payload_copies = self
            .counters
            .whole_payload_copies
            .saturating_add(count as u64);
        self.counters.resident_payload_clones = self
            .counters
            .resident_payload_clones
            .saturating_add(count as u64);
        self.counters.new_semantic_nodes = self
            .counters
            .new_semantic_nodes
            .saturating_sub(count as u64);
        self.record_source_nodes_copied(count);
        Ok(count)
    }

    pub(crate) fn append_constructed_active_list_value(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
        value: T,
        dependency_floor: Option<usize>,
        paired_dependency_floor: Option<usize>,
    ) -> Result<(), ForkArenaError> {
        let mut root = self.active_list_open_mut(builder)?.root;
        self.append_payload_value_with_dependency(
            pool,
            &mut root,
            value,
            None,
            dependency_floor,
            true,
        )?;
        if let Some(paired_dependency_floor) = paired_dependency_floor {
            let meta = pool.payload.validate_exclusive_lineage_mut(
                root.tail.raw,
                self.owner,
                self.lineage,
            )?;
            meta.paired_dependency_floor =
                meta.paired_dependency_floor.min(paired_dependency_floor);
        }
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
        T: Clone + RegionValue<Lane>,
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
        T: Clone + RegionValue<Lane>,
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
        T: Clone + RegionValue<Lane>,
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
        T: Clone + RegionValue<Lane>,
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
        T: Clone + RegionValue<Lane>,
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
            self.copy_shared_then_splice_with_identity(pool, root, selected_root, &mut |value| {
                Some(item_identity(value))
            })?;
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
        self.finish_constructed_tail(pool, list);
        self.active_builder = false;
        builder.state = ActiveListBuilderState::Sealed(UniqueArenaList { root: list });
        Ok(())
    }

    /// Consumes the builder's sole open-owner capability and returns the
    /// unpublished whole-list capability in one step.
    ///
    /// Persistent callers keep the vacant shell for reuse, but the internal
    /// `OpenActiveList` value is moved out exactly once. The root was formed
    /// only by admitted appends, so finish seals its final prefix without
    /// rechecking the root, predecessor chain, generation, or incarnation.
    pub(crate) fn finish_active_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        builder: &mut ActiveListBuilder<T, Lane>,
    ) -> UniqueArenaList<Lane> {
        let state = core::mem::replace(&mut builder.state, ActiveListBuilderState::Vacant);
        let ActiveListBuilderState::Open(open) = state else {
            unreachable!("active-list admission returned the open owner")
        };
        debug_assert!(self.active_builder);
        debug_assert_eq!(open.arena, self.owner);
        self.finish_constructed_tail(pool, open.root);
        self.active_builder = false;
        UniqueArenaList { root: open.root }
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
            list.space,
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
        if (!left.is_empty() && left.space != pool.payload.logical_space())
            || (!right.is_empty() && right.space != pool.payload.logical_space())
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
        pool.payload.set_previous_in_list(
            right.head.raw,
            self.owner,
            self.lineage,
            Some((left.tail.raw, left.tail.offset)),
        )?;
        let left_floor = pool
            .payload
            .validate(left.tail.raw, self.owner)?
            .paired_dependency_floor;
        let right_tail = pool.payload.validate_exclusive_lineage_mut(
            right.tail.raw,
            self.owner,
            self.lineage,
        )?;
        right_tail.paired_dependency_floor = right_tail.paired_dependency_floor.min(left_floor);
        let len = left
            .len
            .checked_add(right.len)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        let root = ArenaListId::from_root(left.space, left.head, right.tail, len);
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
        T: Clone + RegionValue<Lane>,
    {
        let (root, _, _) =
            self.copy_shared_then_splice_with_identity(pool, left, right, &mut |_| None)?;
        Ok(root)
    }

    fn copy_shared_then_splice_with_identity(
        &mut self,
        pool: &mut ChunkPool<T>,
        left: ArenaListId<Lane>,
        right: ArenaListId<Lane>,
        item_identity: &mut impl FnMut(&T) -> Option<u64>,
    ) -> Result<
        (
            ArenaListId<Lane>,
            SemanticSequenceIdentity,
            SequenceSummaryWork,
        ),
        ForkArenaError,
    >
    where
        T: Clone + RegionValue<Lane>,
    {
        self.validate_list(pool, right)?;
        let mut copy = ArenaListId::empty();
        let mut summary = SemanticSequenceIdentity::empty();
        let mut work = SequenceSummaryWork::default();
        let mut identity_mode = None;
        if !right.is_empty() {
            self.copy_shared_chunk_prefix(
                pool,
                right,
                right.tail.raw,
                right.tail.offset,
                &mut copy,
                &mut summary,
                &mut work,
                &mut identity_mode,
                item_identity,
            )?;
        }
        self.counters.new_semantic_nodes = self
            .counters
            .new_semantic_nodes
            .saturating_sub(right.len as u64);
        self.record_source_nodes_copied(right.len());
        let root = self.splice_unique_direct_root(pool, left, UniqueArenaList { root: copy })?;
        Ok((root, summary, work))
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_shared_chunk_prefix(
        &mut self,
        pool: &mut ChunkPool<T>,
        list: ArenaListId<Lane>,
        key: LogicalChunkId,
        end: u32,
        copy: &mut ArenaListId<Lane>,
        summary: &mut SemanticSequenceIdentity,
        work: &mut SequenceSummaryWork,
        identity_mode: &mut Option<bool>,
        item_identity: &mut impl FnMut(&T) -> Option<u64>,
    ) -> Result<(), ForkArenaError>
    where
        T: Clone + RegionValue<Lane>,
    {
        let start = if key == list.head.raw {
            list.head.offset
        } else {
            let previous = pool
                .payload
                .previous_in_list(key, self.owner)?
                .ok_or(ForkArenaError::InvalidRange)?;
            self.copy_shared_chunk_prefix(
                pool,
                list,
                previous.0,
                previous.1,
                copy,
                summary,
                work,
                identity_mode,
                item_identity,
            )?;
            0
        };
        for offset in start..end {
            let identity = {
                let source = pool
                    .payload
                    .get(key, self.owner, offset)
                    .ok_or(ForkArenaError::InvalidRange)?;
                item_identity(source)
            };
            match *identity_mode {
                Some(enabled) if enabled != identity.is_some() => {
                    return Err(ForkArenaError::IdentityModeMismatch);
                }
                None => *identity_mode = Some(identity.is_some()),
                _ => {}
            }
            self.append_payload_clone_from_coordinate(pool, copy, key, offset, identity)?;
            if let Some(identity) = identity {
                summary.push_back(identity);
                work.hashed_values = work.hashed_values.saturating_add(1);
            }
        }
        Ok(())
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
            u64::try_from(unused.saturating_mul(pool.payload.resident_slot_bytes()))
                .unwrap_or(u64::MAX),
        );
        Ok(())
    }

    /// Consumes a statically exclusive construction tail without
    /// revalidating its owner, generation, range, or predecessor chain.
    fn finish_constructed_tail(&mut self, pool: &mut ChunkPool<T>, root: ArenaListId<Lane>) {
        if root.is_empty() {
            return;
        }
        let key = root.tail.raw;
        let meta = &pool.payload.chunks[key.ordinal as usize];
        debug_assert_eq!(meta.used, root.tail.offset);
        if meta.sealed {
            return;
        }
        let unused = pool
            .payload
            .seal_constructed_tail(key, self.owner, self.lineage);
        self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        self.counters.unused_sealed_bytes = self.counters.unused_sealed_bytes.saturating_add(
            u64::try_from(unused.saturating_mul(pool.payload.resident_slot_bytes()))
                .unwrap_or(u64::MAX),
        );
    }

    pub fn seal_boundary(
        &mut self,
        pool: &mut ChunkPool<T>,
    ) -> Result<SealedBoundary<Lane>, ForkArenaError> {
        self.can_seal_boundary(pool)?;
        self.bind_pool(pool)
            .expect("boundary pool binding was preflighted");
        let payload_tail = self.live_key_at(self.live_payload_len().saturating_sub(1));
        let mut sealed = 0_u64;
        let mut unused_bytes = 0_u64;
        {
            if let Some(key) = payload_tail
                && !pool.payload.is_sealed(key, self.owner)?
            {
                let unused = pool.payload.seal(key, self.owner)?;
                sealed += 1;
                unused_bytes = unused_bytes
                    .saturating_add((unused * pool.payload.resident_slot_bytes()) as u64);
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
            payload_tail,
            _lane: PhantomData,
        })
    }

    pub fn checkpoint_mark(
        &self,
        boundary: SealedBoundary<Lane>,
    ) -> Result<CheckpointMark<Lane>, ForkArenaError> {
        if boundary.arena != self.owner
            || boundary.payload_chunks as usize != self.live_payload_len()
        {
            return Err(ForkArenaError::UnsealedBoundary);
        }
        Ok(CheckpointMark {
            arena: boundary.arena,
            payload_chunks: boundary.payload_chunks,
            payload_tail: boundary.payload_tail,
            _lane: PhantomData,
        })
    }

    pub fn validates_checkpoint(&self, mark: CheckpointMark<Lane>) -> bool {
        mark.arena == self.owner
            && mark.payload_chunks >= self.base_payload_chunks
            && mark.payload_chunks as usize <= self.live_payload_len()
            && (mark.payload_chunks == self.base_payload_chunks
                || mark.payload_tail
                    == mark
                        .payload_chunks
                        .checked_sub(1)
                        .and_then(|index| self.live_key_at(index as usize)))
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
        let ForkOwnership::Accepted(accepted) = &mut self.ownership else {
            unreachable!()
        };
        let owner = self.owner;
        for key in accepted.payload.drain(..payload_count) {
            pool.payload.unindex_from_arena(key, owner, self.lineage);
            pool.payload.release_lineage(key, owner, self.lineage)?;
        }
        self.base_payload_chunks = mark.payload_chunks;
        self.refresh_live_chunk_frontiers();
        Ok(payload_count)
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
                .live_key_at(position)
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
                .live_key_at(position)
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
                .live_key_at(position)
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
                    .get_mut(*key, self.owner, self.lineage, offset)
                    .ok_or(ForkArenaError::InvalidChunk)?;
                visit(value);
            }
        }
        Ok(())
    }

    /// Visits the accepted detached prefix from the fork point through one
    /// pre-fork checkpoint. The checkpoint is meaningful only while the sole
    /// candidate transaction keeps that detached suffix parked.
    #[doc(hidden)]
    pub fn visit_detached_checkpoint_prefix(
        &self,
        pool: &ChunkPool<T>,
        mark: CheckpointMark<Lane>,
        mut visit: impl FnMut(&T),
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        let ForkOwnership::Forked {
            prefix,
            detached_prior,
            ..
        } = &self.ownership
        else {
            return Err(ForkArenaError::NotForked);
        };
        let prefix_payload = self
            .base_payload_chunks
            .saturating_add(prefix.payload.len() as u32);
        let detached_end = prefix_payload.saturating_add(detached_prior.payload.len() as u32);
        if mark.arena != self.owner
            || mark.payload_chunks < prefix_payload
            || mark.payload_chunks > detached_end
        {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        let count = (mark.payload_chunks - prefix_payload) as usize;
        for key in detached_prior.payload.iter().take(count) {
            let used = pool.payload.used(*key, self.owner)?;
            for offset in 0..used {
                visit(
                    pool.payload
                        .get(*key, self.owner, offset)
                        .ok_or(ForkArenaError::InvalidChunk)?,
                );
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
                .live_key_at(position)
                .ok_or(ForkArenaError::InvalidCheckpoint)?;
            let used = pool.payload.used(key, self.owner)?;
            for offset in (0..used).rev() {
                let value = pool
                    .payload
                    .get_mut(key, self.owner, self.lineage, offset)
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
            .live_key_at(position)
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
            pool.payload.logical_space(),
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
                payload_tail: Some(payload),
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
        };
        for key in &detached_prior.payload {
            self.unindex_chunk(pool, *key);
        }
        self.ownership = ForkOwnership::Forked {
            prefix: accepted,
            detached_prior,
            current: ChunkSet::default(),
        };
        self.refresh_live_chunk_frontiers();
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
        self.truncate_payload(
            pool,
            mark.payload_chunks as usize,
            payload_tail_used,
            mark.payload_tail.is_some(),
            payload_tail_summary,
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
        self.counters.accepted_chunks_reattached = self
            .counters
            .accepted_chunks_reattached
            .saturating_add(detached_prior.payload.len() as u64);
        let payload_start = self.base_payload_chunks as usize + prefix.payload.len();
        for (offset, key) in detached_prior.payload.iter().copied().enumerate() {
            self.index_chunk(pool, key, payload_start + offset);
        }
        prefix.payload.extend(detached_prior.payload);
        self.ownership = ForkOwnership::Accepted(prefix);
        self.refresh_live_chunk_frontiers();
        Ok(())
    }

    pub(crate) fn can_settle_checkpoint_candidate(
        &self,
        boundary: &SealedBoundary<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.validate_settlement_boundary(boundary)
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
        self.ownership = ForkOwnership::Accepted(prefix);
        self.refresh_live_chunk_frontiers();
        Ok(())
    }

    /// Releases whole current-lineage chunks above the newest retained mark.
    ///
    /// `boundary` proves that no builder or partial tail can still publish
    /// into the suffix. An accepted arena releases only the chunks after
    /// `retained`; a forked arena additionally preserves its selected prefix
    /// and parked accepted suffix. Shared chunks lose only this arena's
    /// bounded lineage slot and remain live for their other owner.
    pub(crate) fn release_rootless_current_suffix(
        &mut self,
        pool: &mut ChunkPool<T>,
        boundary: SealedBoundary<Lane>,
        retained: Option<CheckpointMark<Lane>>,
    ) -> Result<usize, ForkArenaError> {
        self.bind_pool(pool)?;
        if self.active_builder
            || self.pending_batch.is_some()
            || boundary.arena != self.owner
            || boundary.payload_chunks as usize != self.live_payload_len()
            || retained.is_some_and(|mark| !self.validates_checkpoint(mark))
        {
            return Err(ForkArenaError::UnsealedBoundary);
        }

        let payload_origin = match &self.ownership {
            ForkOwnership::Accepted(_) => self.base_payload_chunks as usize,
            ForkOwnership::Forked { prefix, .. } => {
                self.base_payload_chunks as usize + prefix.payload.len()
            }
        };
        let payload_floor = retained
            .map_or(payload_origin, |mark| mark.payload_chunks as usize)
            .checked_sub(payload_origin)
            .ok_or(ForkArenaError::InvalidCheckpoint)?;
        let current = self.current_chunks_mut();
        if payload_floor > current.payload.len() {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        let released = ChunkSet {
            payload: current.payload.split_off(payload_floor),
        };
        self.refresh_live_chunk_frontiers();
        let count = self.release_set(pool, released)?;
        self.counters.rootless_suffix_chunks_released = self
            .counters
            .rootless_suffix_chunks_released
            .saturating_add(count as u64);
        Ok(count)
    }

    fn validate_settlement_boundary(
        &self,
        boundary: &SealedBoundary<Lane>,
    ) -> Result<(), ForkArenaError> {
        if self.active_builder
            || boundary.arena != self.owner
            || boundary.payload_chunks as usize != self.live_payload_len()
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
        let count = set.payload.len();
        for key in set.payload {
            self.unindex_chunk(pool, key);
            pool.payload
                .release_lineage(key, self.owner, self.lineage)?;
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
            _lane: PhantomData,
        })
    }

    pub fn seal_batch(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: BatchMark<Lane>,
        lists: Vec<ArenaListId<Lane>>,
    ) -> Result<SealedBatch<Lane>, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        if mark.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        let boundary = self.seal_boundary(pool)?;
        self.complete_legacy_suffix_dependencies(
            pool,
            mark.payload_start as usize,
            boundary.payload_chunks as usize,
        )?;
        for list in &lists {
            self.validate_list_in_suffix(pool, *list, mark.payload_start as usize)?;
        }
        let serial = self.next_batch_serial;
        self.next_batch_serial = serial
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        self.pending_batch = Some(PendingBatch {
            serial,
            payload_start: mark.payload_start,
            payload_end: boundary.payload_chunks,
        });
        Ok(SealedBatch {
            arena: self.owner,
            serial,
            payload_start: mark.payload_start,
            payload_end: boundary.payload_chunks,
            lists,
        })
    }

    /// Completes metadata for the old generic value-returning builder.
    ///
    /// Production page nodes publish dependency floors beside their final
    /// resident slot and never enter this compatibility path. Generic arena
    /// tests may still use `ForkArenaBuilder::push`; scanning that freshly
    /// sealed construction suffix once keeps transfer and lookup metadata-only
    /// without introducing another node representation.
    fn complete_legacy_suffix_dependencies(
        &mut self,
        pool: &mut ChunkPool<T>,
        start: usize,
        end: usize,
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        for position in start..end {
            let key = self
                .live_key_at(position)
                .ok_or(ForkArenaError::InvalidChunk)?;
            if pool
                .payload
                .validate_lineage(key, self.owner, self.lineage)?
                .dependency_metadata_complete
            {
                continue;
            }
            let used = pool.payload.used(key, self.owner)?;
            let mut dependency_floor = None;
            for offset in 0..used {
                let value = pool
                    .payload
                    .get(key, self.owner, offset)
                    .ok_or(ForkArenaError::InvalidChunk)?;
                if let Some(floor) = self.region_value_dependency_floor(pool, value)? {
                    dependency_floor =
                        Some(dependency_floor.map_or(floor, |old: usize| old.min(floor)));
                }
            }
            let meta =
                pool.payload
                    .validate_exclusive_lineage_mut(key, self.owner, self.lineage)?;
            if let Some(floor) = dependency_floor {
                meta.dependency_floor = meta.dependency_floor.min(floor);
            }
            meta.dependency_metadata_complete = true;
        }
        Ok(())
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
        let payload = self.validate_suffix(mark.payload_start as usize, payload_end)?;
        for list in lists {
            self.validate_list_in_suffix(pool, *list, mark.payload_start as usize)?;
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
                        .validate_list_in_suffix(pool, list, mark.payload_start as usize)
                        .is_ok();
                });
                if !valid {
                    return Err(ForkArenaError::InvalidRegion);
                }
            }
        }
        Ok(())
    }

    /// Whether one validated owner root is wholly resident in the
    /// construction suffix opened at `mark`.
    pub(crate) fn list_is_in_batch_suffix(
        &self,
        pool: &ChunkPool<T>,
        mark: &BatchMark<Lane>,
        list: ArenaListId<Lane>,
    ) -> Result<bool, ForkArenaError> {
        self.validate_pool(pool)?;
        if mark.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.validate_list(pool, list)?;
        if list.is_empty() {
            return Ok(false);
        }
        match self.validate_list_in_suffix(pool, list, mark.payload_start as usize) {
            Ok(()) => Ok(true),
            Err(ForkArenaError::InvalidRegion) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Chunk-only closure proof used by retained-lineage sharing. Direct child
    /// floors were folded into metadata at publication, so this never visits
    /// a node payload or follows the node tree.
    fn preflight_shared_prefix_metadata(
        &self,
        pool: &ChunkPool<T>,
        mark: &BatchMark<Lane>,
        lists: &[ArenaListId<Lane>],
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        if self.active_builder || self.pending_batch.is_some() || mark.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        let payload_end = self.live_payload_len();
        self.validate_suffix(mark.payload_start as usize, payload_end)?;
        for list in lists {
            self.validate_list_endpoints_in_suffix(pool, *list, mark.payload_start as usize)?;
        }
        for position in mark.payload_start as usize..payload_end {
            let key = self
                .live_key_at(position)
                .ok_or(ForkArenaError::InvalidChunk)?;
            let meta = pool
                .payload
                .validate_lineage(key, self.owner, self.lineage)?;
            if !meta.dependency_metadata_complete
                || meta.dependency_floor < mark.payload_start as usize
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    /// Proves that every declared successor root and nested child belongs to
    /// the construction suffix opened at `mark`.
    ///
    /// Unlike batch promotion, unique-successor adoption keeps this arena's
    /// identity. The proof therefore needs no destination, relocation map, or
    /// coordinate rewrite; it only excludes references into the predecessor
    /// prefix before that prefix is released.
    pub(crate) fn preflight_unique_successor_adoption(
        &self,
        pool: &ChunkPool<T>,
        mark: &BatchMark<Lane>,
        lists: &[ArenaListId<Lane>],
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        if !matches!(self.ownership, ForkOwnership::Accepted(_)) {
            return Err(ForkArenaError::AlreadyForked);
        }
        self.validate_live_chunks(pool)?;
        self.preflight_batch_closure(pool, mark, lists)
    }

    /// Releases one consumed predecessor prefix and keeps its construction
    /// suffix as the sole semantic successor.
    ///
    /// Chunk keys, payload addresses, arena identity, sequence summaries, and
    /// the unsealed partial tail stay unchanged. Ownership work is one release
    /// for each predecessor chunk and one index update for each adopted chunk;
    /// no payload is copied or rebranded.
    pub(crate) fn adopt_unique_successor_suffix(
        &mut self,
        pool: &mut ChunkPool<T>,
        mark: BatchMark<Lane>,
        lists: &[ArenaListId<Lane>],
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.preflight_unique_successor_adoption(pool, &mark, lists)?;
        let payload_start = (mark.payload_start - self.base_payload_chunks) as usize;
        let ForkOwnership::Accepted(mut accepted) = std::mem::replace(
            &mut self.ownership,
            ForkOwnership::Accepted(ChunkSet::default()),
        ) else {
            unreachable!("unique-successor adoption preflighted accepted ownership")
        };
        let successor = ChunkSet {
            payload: accepted.payload.split_off(payload_start),
        };
        let released = self.release_set(pool, accepted)?;
        self.base_payload_chunks = 0;
        for (position, key) in successor.payload.iter().copied().enumerate() {
            self.index_chunk(pool, key, position);
        }
        self.counters.obsolete_chunks_pruned = self
            .counters
            .obsolete_chunks_pruned
            .saturating_add(released as u64);
        self.ownership = ForkOwnership::Accepted(successor);
        self.refresh_live_chunk_frontiers();
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
            .detach_suffix(batch.payload_start as usize)
            .expect("self-contained payload suffix was preflighted");
        for key in &payload {
            self.unindex_chunk(pool, *key);
        }
        self.refresh_live_chunk_frontiers();
        Ok(DetachedBatch {
            arena: batch.arena,
            serial: batch.serial,
            payload_start: batch.payload_start,
            payload,
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
        if let Err(error) = self.can_reattach_batch(pool, &batch) {
            return Err(DetachedBatchTransferError { error, batch });
        }
        for (offset, key) in batch.payload.iter().copied().enumerate() {
            self.index_chunk(pool, key, batch.payload_start as usize + offset);
        }
        {
            let current = self.current_chunks_mut();
            current.payload.extend(batch.payload);
        }
        self.refresh_live_chunk_frontiers();
        self.pending_batch = None;
        Ok(())
    }

    pub(crate) fn can_reattach_batch(
        &self,
        pool: &ChunkPool<T>,
        batch: &DetachedBatch<Lane>,
    ) -> Result<(), ForkArenaError> {
        let expected = PendingBatch {
            serial: batch.serial,
            payload_start: batch.payload_start,
            payload_end: batch
                .payload_start
                .saturating_add(batch.payload.len() as u32),
        };
        self.validate_pool(pool).and_then(|()| {
            if batch.arena != self.owner
                || self.pending_batch != Some(expected)
                || self.live_payload_len() != batch.payload_start as usize
            {
                return Err(ForkArenaError::InvalidRegion);
            }
            for key in &batch.payload {
                pool.payload.used(*key, self.owner)?;
            }
            Ok(())
        })
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
        if let Err(error) = self.can_promote_detached_batch_into(pool, destination, &batch) {
            return Err(DetachedBatchTransferError { error, batch });
        }
        destination
            .seal_boundary(pool)
            .expect("detached destination boundary was preflighted");
        for key in &batch.payload {
            pool.payload
                .transfer(
                    *key,
                    self.owner,
                    self.lineage,
                    destination.owner,
                    destination.lineage,
                )
                .expect("detached payload transfer was preflighted");
        }
        let promoted_lists = batch
            .lists
            .iter()
            .copied()
            .map(|list| rebrand_list(list, destination.owner))
            .collect::<Vec<_>>();
        let payload_start = destination.live_payload_len();
        for (offset, key) in batch.payload.iter().copied().enumerate() {
            destination.index_chunk(pool, key, payload_start + offset);
        }
        let promoted = batch.payload.len();
        {
            let current = destination.current_chunks_mut();
            current.payload.extend(batch.payload);
        }
        destination.refresh_live_chunk_frontiers();
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

    pub(crate) fn can_promote_detached_batch_into<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        batch: &DetachedBatch<Lane>,
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        let expected = PendingBatch {
            serial: batch.serial,
            payload_start: batch.payload_start,
            payload_end: batch
                .payload_start
                .saturating_add(batch.payload.len() as u32),
        };
        self.validate_pool(pool).and_then(|()| {
            destination.validate_pool(pool)?;
            if batch.arena != self.owner
                || self.owner == destination.owner
                || self.pending_batch != Some(expected)
                || self.live_payload_len() != batch.payload_start as usize
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
            Ok(())
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
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
        self.validate_suffix(batch.payload_start as usize, batch.payload_end as usize)
            .expect("batch payload suffix was preflighted");
        let payload = self
            .detach_suffix(batch.payload_start as usize)
            .expect("batch payload detachment was preflighted");
        for key in &payload {
            self.unindex_chunk(pool, *key);
        }
        for key in &payload {
            pool.payload
                .transfer(
                    *key,
                    self.owner,
                    self.lineage,
                    destination.owner,
                    destination.lineage,
                )
                .expect("batch payload ownership was preflighted");
        }
        let promoted = payload.len();
        let payload_start = destination.live_payload_len();
        for (offset, key) in payload.iter().copied().enumerate() {
            destination.index_chunk(pool, key, payload_start + offset);
        }
        {
            let current = destination.current_chunks_mut();
            current.payload.extend(payload);
        }
        destination.refresh_live_chunk_frontiers();
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

    #[cfg_attr(not(test), allow(dead_code))]
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
                })
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let payload =
            self.validate_suffix(batch.payload_start as usize, batch.payload_end as usize)?;
        for key in &payload {
            if !pool.payload.is_sealed(*key, self.owner)? {
                return Err(ForkArenaError::UnsealedBoundary);
            }
        }
        for list in &batch.lists {
            self.validate_list_in_suffix(pool, *list, batch.payload_start as usize)?;
        }
        for key in &payload {
            let meta = pool
                .payload
                .validate_lineage(*key, self.owner, self.lineage)?;
            if !meta.dependency_metadata_complete
                || meta.dependency_floor < batch.payload_start as usize
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        // Coordinates are pool-stable, so successful transfer rewrites and
        // scans zero resident values.
        Ok(0)
    }

    pub(crate) fn preflight_whole_region_transfer<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        root: Option<ArenaListId<Lane>>,
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
        self.preflight_whole_transfer_coordinates(pool, destination, root)
    }

    fn preflight_whole_transfer_coordinates<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        root: Option<ArenaListId<Lane>>,
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.validate_live_chunks(pool)?;
        destination.can_seal_boundary(pool)?;
        if self.owner == destination.owner || destination.active_builder {
            return Err(ForkArenaError::InvalidRegion);
        }
        if let Some(root) = root {
            self.validate_list_in_suffix(pool, root, 0)?;
        }
        for position in 0..self.live_payload_len() {
            let key = self
                .live_key_at(position)
                .ok_or(ForkArenaError::InvalidRegion)?;
            let used = pool.payload.used(key, self.owner)?;
            for offset in 0..used {
                let value = pool
                    .payload
                    .get(key, self.owner, offset)
                    .ok_or(ForkArenaError::InvalidChunk)?;
                let mut valid = true;
                value.visit_region_lists(&mut |list| {
                    valid &= self.validate_list_in_suffix(pool, list, 0).is_ok();
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
        root: Option<ArenaListId<Lane>>,
    ) -> Result<WholeRegionBatch<Lane>, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        if self.active_builder || self.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBuilder);
        }
        if !matches!(self.ownership, ForkOwnership::Accepted(_)) {
            return Err(ForkArenaError::AlreadyForked);
        }
        let boundary = self.seal_boundary(pool)?;
        self.complete_legacy_suffix_dependencies(pool, 0, boundary.payload_chunks as usize)?;
        let serial = self.next_batch_serial;
        self.next_batch_serial = serial
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        self.pending_batch = Some(PendingBatch {
            serial,
            payload_start: 0,
            payload_end: boundary.payload_chunks,
        });
        Ok(WholeRegionBatch {
            arena: self.owner,
            serial,
            payload_end: boundary.payload_chunks,
            root,
        })
    }

    pub(crate) fn promote_whole_region_into<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        batch: WholeRegionBatch<Lane>,
    ) -> Result<Option<ArenaListId<Destination>>, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        if batch.arena != self.owner
            || self.pending_batch
                != Some(PendingBatch {
                    serial: batch.serial,
                    payload_start: 0,
                    payload_end: batch.payload_end,
                })
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.preflight_whole_transfer_coordinates(pool, destination, batch.root)?;
        self.bind_pool(pool)
            .expect("whole-region source pool was preflighted");
        destination
            .bind_pool(pool)
            .expect("whole-region destination pool was preflighted");
        destination
            .seal_boundary(pool)
            .expect("whole-region destination boundary was preflighted");
        let payload = self
            .detach_suffix(0)
            .expect("whole-region source suffix was preflighted");
        for key in &payload {
            self.unindex_chunk(pool, *key);
        }
        for key in &payload {
            pool.payload
                .transfer(
                    *key,
                    self.owner,
                    self.lineage,
                    destination.owner,
                    destination.lineage,
                )
                .expect("whole-region payload ownership was preflighted");
        }
        let promoted = payload.len();
        let payload_start = destination.live_payload_len();
        for (offset, key) in payload.iter().copied().enumerate() {
            destination.index_chunk(pool, key, payload_start + offset);
        }
        destination.current_chunks_mut().payload.extend(payload);
        destination.refresh_live_chunk_frontiers();
        self.counters.chunks_promoted = self
            .counters
            .chunks_promoted
            .saturating_add(promoted as u64);
        destination.counters.chunks_promoted = destination
            .counters
            .chunks_promoted
            .saturating_add(promoted as u64);
        self.pending_batch = None;
        Ok(batch.root.map(|root| rebrand_list(root, destination.owner)))
    }

    fn validate_suffix(
        &self,
        start: usize,
        end: usize,
    ) -> Result<Vec<LogicalChunkId>, ForkArenaError> {
        let live_len = self.live_payload_len();
        if start > end || end != live_len {
            return Err(ForkArenaError::InvalidRegion);
        }
        (start..end)
            .map(|index| self.live_key_at(index).ok_or(ForkArenaError::InvalidRegion))
            .collect()
    }

    fn detach_suffix(&mut self, start: usize) -> Result<Vec<LogicalChunkId>, ForkArenaError> {
        let base = self.base_payload_chunks as usize;
        let prefix_len = match &self.ownership {
            ForkOwnership::Accepted(_) => base,
            ForkOwnership::Forked { prefix, .. } => base + prefix.payload.len(),
        };
        if start < prefix_len {
            return Err(ForkArenaError::InvalidRegion);
        }
        let lane = &mut self.current_chunks_mut().payload;
        let local = start - prefix_len;
        if local > lane.len() {
            return Err(ForkArenaError::InvalidRegion);
        }
        let detached = lane.split_off(local);
        self.refresh_live_chunk_frontiers();
        Ok(detached)
    }

    fn validate_list_in_suffix(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        payload_start: usize,
    ) -> Result<(), ForkArenaError> {
        self.validate_list(pool, list)?;
        self.audit_direct_chain(pool, list)?;
        if list.is_empty() {
            return Ok(());
        }
        let mut key = list.tail.raw;
        loop {
            if self
                .resolved_position(pool, key)
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

    fn validate_list_endpoints_in_suffix(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        payload_start: usize,
    ) -> Result<(), ForkArenaError> {
        self.validate_list(pool, list)?;
        if list.is_empty() {
            return Ok(());
        }
        let head = self
            .resolved_position(pool, list.head.raw)
            .ok_or(ForkArenaError::InvalidRegion)?;
        let tail = self
            .resolved_position(pool, list.tail.raw)
            .ok_or(ForkArenaError::InvalidRegion)?;
        if head < payload_start || tail < payload_start {
            return Err(ForkArenaError::InvalidRegion);
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
        T: Clone + RegionValue<Lane>,
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
            payload: self.logical_payload_view(pool),
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

    pub(crate) fn admit_owned_root(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<AdmittedListRoot<Lane>, ForkArenaError> {
        self.validate_pool(pool)?;
        if list.is_empty() {
            return (list == ArenaListId::empty())
                .then_some(AdmittedListRoot::EMPTY)
                .ok_or(ForkArenaError::InvalidRange);
        }
        if list.space != pool.payload.logical_space() {
            return Err(ForkArenaError::ForeignArena);
        }
        let (head_key, head_offset) = pool
            .payload
            .compact_position(list.head_position()?)
            .map_err(|_| ForkArenaError::InvalidRange)?;
        let (tail_key, tail_offset) = pool
            .payload
            .compact_position(list.tail_position()?)
            .map_err(|_| ForkArenaError::InvalidRange)?;
        if head_key != list.head.raw
            || head_offset != list.head.offset
            || tail_key != list.tail.raw
            || tail_offset != list.tail.offset
        {
            return Err(ForkArenaError::InvalidRange);
        }
        let head_position = self
            .resolved_position(pool, list.head.raw)
            .ok_or(ForkArenaError::InvalidRange)?;
        let tail_position = self
            .resolved_position(pool, list.tail.raw)
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
        let head_block = pool
            .payload
            .admit_dense_block(list.head.raw)
            .ok_or(ForkArenaError::InvalidRange)?;
        let tail_block = if list.head.raw == list.tail.raw {
            head_block
        } else {
            pool.payload
                .admit_dense_block(list.tail.raw)
                .ok_or(ForkArenaError::InvalidRange)?
        };
        Ok(AdmittedListRoot {
            owner: self.owner,
            head: AdmittedChunkCursor::new(
                u32::try_from(head_position).map_err(|_| ForkArenaError::CapacityOverflow)?,
                head_block,
                list.head.offset,
            ),
            tail: AdmittedChunkCursor::new(
                u32::try_from(tail_position).map_err(|_| ForkArenaError::CapacityOverflow)?,
                tail_block,
                list.tail.offset,
            ),
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
            payload: self.logical_payload_view(pool),
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
            .get_mut(list.head.raw, self.owner, self.lineage, list.head.offset)
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
            if self.resolved_position(pool, key).is_none() {
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
    _arena: u32,
) -> ArenaListId<Destination> {
    if list.is_empty() {
        ArenaListId::empty()
    } else {
        ArenaListId::from_root(
            list.space,
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
    append_block: Option<AdmittedAppendBlock>,
    #[cfg(test)]
    sequence_summary: Option<SemanticSequenceIdentity>,
    finished: bool,
}

impl<T, Lane> ForkArenaBuilder<'_, T, Lane> {
    #[cfg(test)]
    fn validation_reads(&self) -> u64 {
        self.pool.payload.validation_reads()
    }

    pub fn push(&mut self, value: T) -> Result<(), ForkArenaError> {
        if self.append_block.is_none() {
            if self.root.len == u32::MAX {
                return Err(ForkArenaError::CapacityOverflow);
            }
            let key = self
                .arena
                .payload_reservation_target(self.pool, &self.root)?;
            self.append_block = Some(self.pool.payload.admit_untracked_append_block(
                key,
                self.arena.owner,
                self.arena.lineage,
            )?);
        }
        let append = self
            .append_block
            .as_mut()
            .expect("append block is admitted before construction");
        let key = append.key;
        let (offset, became_full) = self.pool.payload.append_admitted_untracked(append, value);
        self.arena.complete_admitted_payload_reservation(
            &mut self.root,
            key,
            offset,
            became_full,
            self.pool.payload.logical_space(),
        );
        if became_full || self.root.len == u32::MAX {
            self.append_block = None;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn push_with_dependencies(
        &mut self,
        value: T,
        source: &impl RegionValue<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.append_block = None;
        let dependency_floor = self
            .arena
            .region_value_dependency_floor(self.pool, source)?;
        self.arena.append_payload_value_with_dependency(
            self.pool,
            &mut self.root,
            value,
            None,
            dependency_floor,
            true,
        )
    }

    #[cfg(test)]
    pub(crate) fn paired_dependency_floor_for(
        &self,
        source: &impl RegionValue<Lane>,
    ) -> Result<Option<usize>, ForkArenaError> {
        self.arena.paired_dependency_floor_for(self.pool, source)
    }

    #[cfg(test)]
    pub(crate) fn record_paired_dependency(
        &mut self,
        paired_dependency_floor: Option<usize>,
    ) -> Result<(), ForkArenaError> {
        let Some(paired_dependency_floor) = paired_dependency_floor else {
            return Ok(());
        };
        if self.root.is_empty() {
            return Err(ForkArenaError::InvalidRange);
        }
        let meta = self.pool.payload.validate_exclusive_lineage_mut(
            self.root.tail.raw,
            self.arena.owner,
            self.arena.lineage,
        )?;
        meta.paired_dependency_floor = meta.paired_dependency_floor.min(paired_dependency_floor);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn push_summarized(
        &mut self,
        value: T,
        item_identity: u64,
    ) -> Result<(), ForkArenaError> {
        self.push_with_identity(value, Some(item_identity))
    }

    #[cfg(test)]
    fn push_with_identity(
        &mut self,
        value: T,
        item_identity: Option<u64>,
    ) -> Result<(), ForkArenaError> {
        self.append_block = None;
        self.arena.append_payload_value_with_dependency(
            self.pool,
            &mut self.root,
            value,
            item_identity,
            None,
            false,
        )?;
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

    /// Finishes the list by consuming its exclusive construction owner.
    ///
    /// Every fallible allocation and dynamic coordinate check happened while
    /// appending. The borrow itself proves the arena/pool pair cannot change,
    /// and the move consumes the sole predecessor/publication authority, so
    /// finish is infallible.
    pub fn finish_unique(mut self) -> UniqueArenaList<Lane> {
        self.arena.finish_constructed_tail(self.pool, self.root);
        let list = self.root;
        self.arena.active_builder = false;
        self.finished = true;
        UniqueArenaList { root: list }
    }

    /// Finishes and publishes the copyable root exactly once.
    pub fn finish(self) -> ArenaListId<Lane> {
        self.finish_unique().publish()
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
    payload: LogicalBlockView<'a, T>,
}

/// Owner-relative cursor valid for one immutable admitted view.
///
/// It deliberately carries no pool-global slot or incarnation. Those are
/// checked once at root admission and resolved from the arena's sole chunk
/// ownership lane while the immutable pool borrow prevents lifecycle change.
struct AdmittedChunkCursor<Lane> {
    position: u32,
    block: AdmittedDenseBlock,
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
        block: AdmittedDenseBlock { page: 0, base: 0 },
        offset: 0,
        _lane: PhantomData,
    };

    const fn new(position: u32, block: AdmittedDenseBlock, offset: u32) -> Self {
        Self {
            position,
            block,
            offset,
            _lane: PhantomData,
        }
    }
}

pub(crate) struct AdmittedListRoot<Lane> {
    owner: u32,
    head: AdmittedChunkCursor<Lane>,
    tail: AdmittedChunkCursor<Lane>,
}

impl<Lane> Clone for AdmittedListRoot<Lane> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Lane> Copy for AdmittedListRoot<Lane> {}

impl<Lane> AdmittedListRoot<Lane> {
    pub(crate) const EMPTY: Self = Self {
        owner: 0,
        head: AdmittedChunkCursor::EMPTY,
        tail: AdmittedChunkCursor::EMPTY,
    };
}

/// One borrowed contiguous payload run from a direct list chain.
///
/// Chunk cells are admitted once before this value is constructed. Iteration
/// therefore reads the payload slice directly, without repeating arena-owner,
/// incarnation, or logical-index resolution for each value.
#[derive(Clone, Copy)]
pub struct ArenaChunkSlice<'a, T> {
    cells: DenseBlockSlice<'a, T>,
}

/// Coordinate-only continuation for one admitted packed-list chunk.
///
/// Unlike [`ArenaListIter`], this cursor retains no arena borrow. A caller may
/// therefore keep the coordinate on its Rust stack while an operation appends
/// to the same arena, then reacquire a short immutable borrow for the next
/// source node. The admitted source root remains authoritative: no successor
/// table, copied index, or second node representation is retained here.
pub(crate) struct AdmittedListChunkCursor<Lane> {
    key: LogicalChunkId,
    block: AdmittedDenseBlock,
    position: usize,
    start: u32,
    end: u32,
    next: u32,
    logical_start: usize,
    head_position: usize,
    head_offset: u32,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<Lane> AdmittedListChunkCursor<Lane> {
    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    #[must_use]
    pub(crate) const fn logical_start(&self) -> usize {
        self.logical_start
    }
}

impl<'a, T> ArenaChunkSlice<'a, T> {
    #[must_use]
    pub const fn len(&self) -> usize {
        match self.cells {
            DenseBlockSlice::Optional(cells) => cells.len(),
            DenseBlockSlice::Packed(cells) => cells.len(),
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(self) -> impl DoubleEndedIterator<Item = &'a T> + ExactSizeIterator + 'a {
        self.cells.iter()
    }

    /// Visits the already-admitted initialized cells without rebuilding an
    /// element-stepping arena iterator.
    pub fn for_each(self, mut visit: impl FnMut(&'a T)) {
        match self.cells {
            DenseBlockSlice::Optional(cells) => {
                for cell in cells {
                    visit(cell.as_ref().expect("admitted chunk range is initialized"));
                }
            }
            DenseBlockSlice::Packed(cells) => {
                for cell in cells {
                    visit(cell);
                }
            }
        }
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

    #[cfg(any(test, feature = "testing"))]
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

    /// Returns the first value directly from the admitted head coordinate.
    #[must_use]
    pub fn first(&self) -> Option<&'a T> {
        if self.is_empty() {
            None
        } else {
            self.get_cursor(self.root.head)
        }
    }

    /// Returns the last value directly from the admitted tail coordinate.
    #[must_use]
    pub fn last(&self) -> Option<&'a T> {
        if self.is_empty() {
            return None;
        }
        let mut cursor = self.root.tail;
        cursor.offset = cursor.offset.checked_sub(1)?;
        self.get_cursor(cursor)
    }

    /// Builds a compatibility iterator for mixed or reverse traversal.
    ///
    /// Long forward consumers must use [`Self::for_each`] or
    /// [`Self::try_for_each_range`], which retain the predecessor walk on the
    /// Rust stack and cross each packed block once. The iterator cannot retain
    /// that continuation between `next` calls without a second topology, so
    /// each forward block boundary performs logical-index resolution.
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
        let front_chunk =
            front_span.and_then(|(cursor, end)| self.chunk_iter(cursor, cursor.offset..end));
        let back_chunk = (front < self.len()).then(|| {
            let tail = self.root.tail;
            let start = if tail.position == self.root.head.position {
                self.root.head.offset
            } else {
                0
            };
            self.chunk_iter(tail, start..tail.offset)
                .expect("admitted tail range remains initialized")
        });
        ArenaListIter {
            view: *self,
            front,
            back: self.len(),
            front_chunk,
            back_chunk,
            forward_chunk_crossings: 0,
            reverse_chunk_crossings: 0,
        }
    }

    fn chunk_iter(
        &self,
        cursor: AdmittedChunkCursor<Lane>,
        range: Range<u32>,
    ) -> Option<DenseBlockIter<'a, T>> {
        self.payload
            .storage()
            .admitted_dense_slice(cursor.block, range)
            .map(DenseBlockSlice::iter)
    }

    fn get_cursor(&self, cursor: AdmittedChunkCursor<Lane>) -> Option<&'a T> {
        self.payload
            .storage()
            .admitted_dense_value(cursor.block, cursor.offset)
    }

    fn previous_cursor(
        &self,
        cursor: AdmittedChunkCursor<Lane>,
    ) -> Option<AdmittedChunkCursor<Lane>> {
        let key = self.arena.live_key_at(cursor.position as usize)?;
        let (key, position, end) = self
            .pool
            .payload
            .admitted_previous_coordinate(key, self.arena.lineage)?;
        let block = self.pool.payload.admit_dense_block(key)?;
        Some(AdmittedChunkCursor::new(
            u32::try_from(position).ok()?,
            block,
            end,
        ))
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

    /// Visits one logical range through the authoritative initialized slices.
    pub fn for_each_range(&self, selected: Range<usize>, mut visit: impl FnMut(usize, &'a T)) {
        let _: core::ops::ControlFlow<core::convert::Infallible> =
            self.try_for_each_range(selected, |index, value| {
                visit(index, value);
                core::ops::ControlFlow::Continue(())
            });
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
            let key = self
                .arena
                .live_key_at(cursor.position as usize)
                .ok_or(ForkArenaError::InvalidRange)?;
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
            let first = ChunkCursor::<Lane>::new(key, first).logical_position(self.list.space)?;
            let cells = self
                .payload
                .slice(first, last)
                .ok_or(ForkArenaError::InvalidRange)?;
            for (offset, value) in cells.iter().enumerate() {
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
        let key = self
            .arena
            .live_key_at(cursor.position as usize)
            .ok_or(ForkArenaError::InvalidRange)?;
        let start = ChunkCursor::<Lane>::new(key, start).logical_position(self.list.space)?;
        let cells = self
            .payload
            .slice(start, cursor.offset)
            .ok_or(ForkArenaError::InvalidRange)?;
        visit(ArenaChunkSlice { cells });
        Ok(())
    }

    /// Visits every value in logical order through direct chunk slices.
    pub fn for_each(&self, mut visit: impl FnMut(&'a T)) {
        self.for_each_range(0..self.len(), |_, value| visit(value));
    }
}

impl<T, Lane> ForkArena<T, Lane> {
    /// Reconstitutes a borrowed view from a root admitted at the span
    /// boundary. The immutable pool borrow keeps that owner-relative proof
    /// live for the complete view traversal.
    pub(crate) fn admitted_view<'a>(
        &'a self,
        pool: &'a ChunkPool<T>,
        list: ArenaListId<Lane>,
        root: AdmittedListRoot<Lane>,
    ) -> Result<ArenaListView<'a, T, Lane>, ForkArenaError> {
        self.validate_admitted_root(pool, list, root)?;
        Ok(ArenaListView {
            arena: self,
            pool,
            list,
            root,
            payload: self.logical_payload_view(pool),
        })
    }

    /// Rechecks only the region and endpoint incarnations carried by an
    /// admitted root. It deliberately does not re-resolve compact positions,
    /// payload blocks, or offsets.
    fn validate_admitted_root(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        root: AdmittedListRoot<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        if list.is_empty() {
            return (list == ArenaListId::empty() && root.owner == 0)
                .then_some(())
                .ok_or(ForkArenaError::InvalidRange);
        }
        if root.owner != self.owner
            || list.space != pool.payload.logical_space()
            || self.live_key_at(root.head.position as usize) != Some(list.head.raw)
            || self.live_key_at(root.tail.position as usize) != Some(list.tail.raw)
        {
            return Err(ForkArenaError::InvalidRange);
        }
        Ok(())
    }

    /// Admits a list once and returns its tail packed-chunk continuation.
    #[allow(dead_code)] // Retained by direct ForkArena controls; page spans carry the proof.
    pub(crate) fn admitted_tail_chunk(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
    ) -> Result<Option<AdmittedListChunkCursor<Lane>>, ForkArenaError> {
        let root = self.admit_owned_root(pool, list)?;
        self.admitted_tail_chunk_from_root(pool, list, root)
    }

    /// Starts traversal from the direct root resolved by an earlier integer
    /// admission. The owner/incarnation/range proof is carried, not replayed.
    pub(crate) fn admitted_tail_chunk_from_root(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        root: AdmittedListRoot<Lane>,
    ) -> Result<Option<AdmittedListChunkCursor<Lane>>, ForkArenaError> {
        self.validate_admitted_root(pool, list, root)?;
        if list.is_empty() {
            return Ok(None);
        }
        let start = if root.tail.position == root.head.position {
            root.head.offset
        } else {
            0
        };
        let chunk_len = (root.tail.offset - start) as usize;
        Ok(Some(AdmittedListChunkCursor {
            key: list.tail.raw,
            block: root.tail.block,
            position: root.tail.position as usize,
            start,
            end: root.tail.offset,
            next: start,
            logical_start: list.len() - chunk_len,
            head_position: root.head.position as usize,
            head_offset: root.head.offset,
            _lane: PhantomData,
        }))
    }

    /// Follows the sole predecessor edge without resolving a logical index.
    pub(crate) fn admitted_previous_chunk(
        &self,
        pool: &ChunkPool<T>,
        cursor: &AdmittedListChunkCursor<Lane>,
    ) -> Result<Option<AdmittedListChunkCursor<Lane>>, ForkArenaError> {
        if cursor.logical_start == 0 {
            return Ok(None);
        }
        if self.live_key_at(cursor.position) != Some(cursor.key) {
            return Err(ForkArenaError::InvalidRange);
        }
        let (key, position, end) = pool
            .payload
            .admitted_previous_coordinate(cursor.key, self.lineage)
            .ok_or(ForkArenaError::InvalidRange)?;
        let block = pool
            .payload
            .admit_dense_block(key)
            .ok_or(ForkArenaError::InvalidRange)?;
        let start = if position == cursor.head_position {
            cursor.head_offset
        } else {
            0
        };
        let len = usize::try_from(end.checked_sub(start).ok_or(ForkArenaError::InvalidRange)?)
            .map_err(|_| ForkArenaError::CapacityOverflow)?;
        let logical_start = cursor
            .logical_start
            .checked_sub(len)
            .ok_or(ForkArenaError::InvalidRange)?;
        #[cfg(any(test, feature = "testing"))]
        self.pool_forward_chunk_crossing(pool);
        Ok(Some(AdmittedListChunkCursor {
            key,
            block,
            position,
            start,
            end,
            next: start,
            logical_start,
            head_position: cursor.head_position,
            head_offset: cursor.head_offset,
            _lane: PhantomData,
        }))
    }

    #[cfg(any(test, feature = "testing"))]
    fn pool_forward_chunk_crossing(&self, pool: &ChunkPool<T>) {
        pool.payload.admitted_forward_chunk_crossings.set(
            pool.payload
                .admitted_forward_chunk_crossings
                .get()
                .saturating_add(1),
        );
    }

    /// Resolves a caller-proven position inside an admitted packed chunk.
    pub(crate) fn admitted_chunk_value_at<'a>(
        &'a self,
        pool: &'a ChunkPool<T>,
        cursor: &AdmittedListChunkCursor<Lane>,
        offset: usize,
    ) -> (usize, &'a T) {
        debug_assert!(offset < cursor.len());
        let offset = cursor.start + offset as u32;
        let value = pool.payload.admitted_capability_value(cursor.block, offset);
        (
            cursor.logical_start + (offset - cursor.start) as usize,
            value,
        )
    }

    /// Advances one admitted chunk cursor. Successful reads are exactly one
    /// block-table lookup, one payload index, and scalar cursor increments.
    pub(crate) fn admitted_next_chunk_value<'a>(
        &'a self,
        pool: &'a ChunkPool<T>,
        cursor: &mut AdmittedListChunkCursor<Lane>,
    ) -> Option<(usize, &'a T)> {
        if cursor.next == cursor.end {
            return None;
        }
        let offset = cursor.next;
        let logical = cursor.logical_start + (offset - cursor.start) as usize;
        let value = pool.payload.admitted_capability_value(cursor.block, offset);
        cursor.next += 1;
        Some((logical, value))
    }

    pub(crate) fn admitted_chunk_dependency_floors(
        &self,
        pool: &ChunkPool<T>,
        cursor: &AdmittedListChunkCursor<Lane>,
    ) -> Result<(Option<usize>, Option<usize>), ForkArenaError> {
        if self.live_key_at(cursor.position) != Some(cursor.key) {
            return Err(ForkArenaError::InvalidRange);
        }
        let meta = pool.payload.validate(cursor.key, self.owner)?;
        if !meta.dependency_metadata_complete {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok((
            (meta.dependency_floor != usize::MAX).then_some(meta.dependency_floor),
            (meta.paired_dependency_floor != usize::MAX).then_some(meta.paired_dependency_floor),
        ))
    }

    /// Takes the complete remaining initialized slice from one admitted chunk
    /// and settles its scalar cursor once.
    pub(crate) fn admitted_remaining_chunk<'a>(
        &'a self,
        pool: &'a ChunkPool<T>,
        cursor: &mut AdmittedListChunkCursor<Lane>,
    ) -> Option<(usize, ArenaChunkSlice<'a, T>)> {
        if cursor.next == cursor.end {
            return None;
        }
        let logical = cursor.logical_start + (cursor.next - cursor.start) as usize;
        let cells = pool
            .payload
            .admitted_dense_slice(cursor.block, cursor.next..cursor.end)?;
        cursor.next = cursor.end;
        Some((logical, ArenaChunkSlice { cells }))
    }
}

#[cfg(feature = "testing")]
impl<Lane> ForkArena<u32, Lane> {
    /// Runs the mutation-compatible admitted chunk path used by page-node
    /// consumers and returns a checksum suitable for focused performance
    /// gates.
    #[doc(hidden)]
    pub fn testing_admitted_chunk_checksum(
        &self,
        pool: &ChunkPool<u32>,
        list: ArenaListId<Lane>,
    ) -> Result<u64, ForkArenaError> {
        let mut chunk = self.admitted_tail_chunk(pool, list)?;
        let mut checksum = 0_u64;
        while let Some(mut current) = chunk {
            while let Some((_, value)) = self.admitted_next_chunk_value(pool, &mut current) {
                checksum = checksum.wrapping_add(u64::from(*value));
            }
            chunk = self.admitted_previous_chunk(pool, &current)?;
        }
        Ok(checksum)
    }
}

pub struct ArenaListIter<'arena, T, Lane> {
    view: ArenaListView<'arena, T, Lane>,
    front: usize,
    back: usize,
    front_chunk: Option<DenseBlockIter<'arena, T>>,
    back_chunk: Option<DenseBlockIter<'arena, T>>,
    forward_chunk_crossings: usize,
    reverse_chunk_crossings: usize,
}

impl<T, Lane> ArenaListIter<'_, T, Lane> {
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
        let value = self.front_chunk.as_mut()?.next()?;
        self.front += 1;
        if self.front == self.back {
            self.front_chunk = None;
        } else if self.front_chunk.as_ref()?.len() == 0 {
            let (cursor, end) = self.view.cursor_span_at_node(self.front)?;
            self.front_chunk = self.view.chunk_iter(cursor, cursor.offset..end);
            self.forward_chunk_crossings = self.forward_chunk_crossings.saturating_add(1);
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
        if self.back_chunk.as_ref()?.len() == 0 {
            let (cursor, end) = self.view.cursor_span_at_node(self.back - 1)?;
            let start = if cursor.position == self.view.root.head.position {
                self.view.root.head.offset
            } else {
                0
            };
            self.back_chunk = self.view.chunk_iter(cursor, start..end);
            self.reverse_chunk_crossings = self.reverse_chunk_crossings.saturating_add(1);
        }
        let value = self.back_chunk.as_mut()?.next_back()?;
        self.back -= 1;
        Some(value)
    }
}

impl<T, Lane> ExactSizeIterator for ArenaListIter<'_, T, Lane> {}
