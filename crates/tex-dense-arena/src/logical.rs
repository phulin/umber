#![allow(
    clippy::result_large_err,
    reason = "fork and settlement failures must return the unique table authority inline"
)]

use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::size_of;

use tex_dense_prefix::{Superblock, VacantSlot};

use crate::store::BlockId;
use crate::transfer::BlockRangeOwner;
use crate::{ArenaError, ArenaMetrics, BlockStore, ForkShape};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LogicalBlockId {
    space: u32,
    ordinal: u32,
    incarnation: u32,
}

impl LogicalBlockId {
    /// Reconstructs an opaque logical coordinate received from a trusted
    /// storage adapter or validated wire codec.
    ///
    /// This does not admit the coordinate. A borrowed table view still checks
    /// the space, ordinal, incarnation, physical mapping, and initialized
    /// prefix before returning a payload reference.
    #[must_use]
    pub const fn from_parts(space: u32, ordinal: u32, incarnation: u32) -> Option<Self> {
        if space == 0 || incarnation == 0 {
            return None;
        }
        Some(Self {
            space,
            ordinal,
            incarnation,
        })
    }

    #[must_use]
    pub const fn space(self) -> u32 {
        self.space
    }

    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub const fn incarnation(self) -> u32 {
        self.incarnation
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LogicalPosition {
    block: LogicalBlockId,
    offset: u32,
}

impl LogicalPosition {
    /// Reconstructs one position in an opaque logical block.
    ///
    /// Resolution through an accepted or candidate view remains the
    /// admission boundary for the position and its offset.
    #[must_use]
    pub const fn from_parts(block: LogicalBlockId, offset: u32) -> Self {
        Self { block, offset }
    }

    #[must_use]
    pub const fn block(self) -> LogicalBlockId {
        self.block
    }

    #[must_use]
    pub const fn offset(self) -> u32 {
        self.offset
    }

    /// Advances within the same logical block. Resolution still validates the
    /// resulting offset against the selected accepted or candidate prefix.
    #[must_use]
    pub fn checked_add_offset(self, additional: u32) -> Option<Self> {
        Some(Self {
            block: self.block,
            offset: self.offset.checked_add(additional)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalCursor {
    space: u32,
    order_len: u32,
    tail_len: u32,
    total_len: u64,
    boundary: Option<LogicalBlockId>,
}

impl LogicalCursor {
    #[must_use]
    pub const fn len(self) -> u64 {
        self.total_len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.total_len == 0
    }

    #[must_use]
    pub const fn logical_blocks(self) -> u32 {
        self.order_len
    }

    /// Zero denotes an exact physical-block boundary. A nonzero value is the
    /// initialized prefix copied by an interior candidate fork.
    #[must_use]
    pub const fn tail_len(self) -> u32 {
        self.tail_len
    }

    #[must_use]
    pub const fn boundary_block(self) -> Option<LogicalBlockId> {
        self.boundary
    }
}

#[derive(Clone, Copy)]
struct LogicalRow {
    incarnation: u32,
    physical: Option<BlockId>,
    initialized: u32,
}

impl LogicalRow {
    fn id(self, space: u32, ordinal: u32) -> LogicalBlockId {
        LogicalBlockId {
            space,
            ordinal,
            incarnation: self.incarnation,
        }
    }
}

struct TableState<T> {
    space: u32,
    rows: Vec<LogicalRow>,
    free: Vec<u32>,
    order: Vec<u32>,
    total_len: usize,
    force_new_block: bool,
    next_boundary_serial: u64,
    active_boundary: Option<u64>,
    metrics: ArenaMetrics,
    direct_observations: Cell<u64>,
    stale_observations: Cell<u64>,
    _payload: PhantomData<fn() -> T>,
}

impl<T> TableState<T> {
    fn new(space: u32) -> Self {
        Self {
            space,
            rows: Vec::new(),
            free: Vec::new(),
            order: Vec::new(),
            total_len: 0,
            force_new_block: false,
            next_boundary_serial: 1,
            active_boundary: None,
            metrics: ArenaMetrics::default(),
            direct_observations: Cell::new(0),
            stale_observations: Cell::new(0),
            _payload: PhantomData,
        }
    }

    fn try_clone(&self) -> Result<Self, ArenaError> {
        let mut rows = Vec::new();
        rows.try_reserve_exact(self.rows.len())?;
        rows.extend_from_slice(&self.rows);
        let mut free = Vec::new();
        free.try_reserve_exact(self.free.len())?;
        free.extend_from_slice(&self.free);
        let mut order = Vec::new();
        order.try_reserve_exact(self.order.len())?;
        order.extend_from_slice(&self.order);
        Ok(Self {
            space: self.space,
            rows,
            free,
            order,
            total_len: self.total_len,
            force_new_block: self.force_new_block,
            next_boundary_serial: self.next_boundary_serial,
            active_boundary: self.active_boundary,
            metrics: self.metrics_snapshot(),
            direct_observations: Cell::new(0),
            stale_observations: Cell::new(0),
            _payload: PhantomData,
        })
    }

    fn row(&self, id: LogicalBlockId) -> Result<&LogicalRow, ArenaError> {
        if id.space != self.space {
            self.stale_observations
                .set(self.stale_observations.get() + 1);
            return Err(ArenaError::ForeignLogicalSpace);
        }
        let Some(row) = self.rows.get(id.ordinal as usize) else {
            self.stale_observations
                .set(self.stale_observations.get() + 1);
            return Err(ArenaError::StaleLogicalBlock);
        };
        if row.physical.is_none() || row.incarnation != id.incarnation {
            self.stale_observations
                .set(self.stale_observations.get() + 1);
            return Err(ArenaError::StaleLogicalBlock);
        }
        Ok(row)
    }

    fn metrics_snapshot(&self) -> ArenaMetrics {
        let mut metrics = self.metrics;
        metrics.direct_lookups += self.direct_observations.get();
        metrics.logical_stale_rejections += self.stale_observations.get();
        metrics
    }

    fn live_order_len(&self) -> usize {
        self.order
            .last()
            .and_then(|&ordinal| self.rows.get(ordinal as usize))
            .map_or(0, |row| {
                self.order.len() - usize::from(row.initialized == 0)
            })
    }

    fn cursor(&mut self) -> LogicalCursor {
        self.metrics.cursor_captures += 1;
        let order_len = self.live_order_len();
        let boundary = order_len.checked_sub(1).map(|index| {
            let ordinal = self.order[index];
            self.rows[ordinal as usize].id(self.space, ordinal)
        });
        let tail_len = boundary.map_or(0, |id| {
            let initialized = self.rows[id.ordinal as usize].initialized;
            if initialized as usize == Superblock::<T>::capacity() {
                0
            } else {
                initialized
            }
        });
        LogicalCursor {
            space: self.space,
            order_len: u32::try_from(order_len).expect("published logical order fits u32"),
            tail_len,
            total_len: self.total_len as u64,
            boundary,
        }
    }

    fn validate_cursor(&self, cursor: LogicalCursor) -> Result<(), ArenaError> {
        if cursor.space != self.space {
            return Err(ArenaError::InvalidCursor);
        }
        let order_len = cursor.order_len as usize;
        if order_len > self.live_order_len() || cursor.total_len > self.total_len as u64 {
            return Err(ArenaError::InvalidCursor);
        }
        let expected = order_len.checked_sub(1).map(|index| {
            let ordinal = self.order[index];
            self.rows[ordinal as usize].id(self.space, ordinal)
        });
        if cursor.boundary != expected {
            return Err(ArenaError::InvalidCursor);
        }
        if let Some(id) = expected {
            let initialized = self.row(id)?.initialized;
            let valid_tail = if cursor.tail_len == 0 {
                initialized as usize == Superblock::<T>::capacity()
            } else {
                cursor.tail_len <= initialized
            };
            if !valid_tail {
                return Err(ArenaError::InvalidCursor);
            }
        } else if cursor.tail_len != 0 || cursor.total_len != 0 {
            return Err(ArenaError::InvalidCursor);
        }
        Ok(())
    }

    fn allocate_row(&mut self, store: &mut BlockStore<T>) -> Result<u32, ArenaError> {
        self.order.try_reserve(1)?;
        if let Some(&ordinal) = self.free.last() {
            let row = self
                .rows
                .get(ordinal as usize)
                .ok_or(ArenaError::StaleLogicalBlock)?;
            let incarnation = row
                .incarnation
                .checked_add(1)
                .ok_or(ArenaError::IncarnationExhausted)?;
            let physical = store.allocate()?;
            self.free.pop();
            self.rows[ordinal as usize] = LogicalRow {
                incarnation,
                physical: Some(physical),
                initialized: 0,
            };
            self.order.push(ordinal);
            self.metrics.logical_rows_reused += 1;
            return Ok(ordinal);
        }
        let ordinal =
            u32::try_from(self.rows.len()).map_err(|_| ArenaError::LogicalOrdinalExhausted)?;
        self.rows.try_reserve(1)?;
        let physical = store.allocate()?;
        self.rows.push(LogicalRow {
            incarnation: 1,
            physical: Some(physical),
            initialized: 0,
        });
        self.order.push(ordinal);
        self.metrics.logical_rows_created += 1;
        Ok(ordinal)
    }

    fn tail_for_push(&mut self, store: &mut BlockStore<T>) -> Result<u32, ArenaError> {
        let capacity = Superblock::<T>::capacity();
        if !self.force_new_block
            && let Some(&ordinal) = self.order.last()
            && (self.rows[ordinal as usize].initialized as usize) < capacity
        {
            return Ok(ordinal);
        }
        if self.force_new_block {
            if let Some(&ordinal) = self.order.last() {
                let initialized = self.rows[ordinal as usize].initialized as usize;
                self.metrics.boundary_slack_values += (capacity - initialized) as u64;
            }
            self.force_new_block = false;
        }
        self.allocate_row(store)
    }

    fn push_with<F>(
        &mut self,
        store: &mut BlockStore<T>,
        build: F,
    ) -> Result<LogicalPosition, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        let ordinal = self.tail_for_push(store)?;
        let row = self.rows[ordinal as usize];
        let physical = row.physical.ok_or(ArenaError::StalePhysicalBlock)?;
        let offset = row.initialized;
        store.push_with(physical, build)?;
        self.rows[ordinal as usize].initialized = offset
            .checked_add(1)
            .ok_or(ArenaError::LogicalLengthOverflow)?;
        self.total_len = self
            .total_len
            .checked_add(1)
            .ok_or(ArenaError::LogicalLengthOverflow)?;
        Ok(LogicalPosition {
            block: self.rows[ordinal as usize].id(self.space, ordinal),
            offset,
        })
    }

    fn resolve<'a>(
        &'a self,
        store: &'a BlockStore<T>,
        position: LogicalPosition,
    ) -> Result<&'a T, ArenaError> {
        self.direct_observations
            .set(self.direct_observations.get() + 1);
        let row = self.row(position.block)?;
        if position.offset >= row.initialized {
            self.stale_observations
                .set(self.stale_observations.get() + 1);
            return Err(ArenaError::UninitializedLogicalOffset);
        }
        store
            .resolve(row.physical.ok_or(ArenaError::StalePhysicalBlock)?)?
            .get(position.offset as usize)
            .ok_or(ArenaError::UninitializedLogicalOffset)
    }

    fn position_at_dense_index(&self, index: usize) -> Option<LogicalPosition> {
        if index >= self.total_len {
            return None;
        }
        let capacity = Superblock::<T>::capacity();
        let ordinal = *self.order.get(index / capacity)?;
        let row = self.rows[ordinal as usize];
        let offset = u32::try_from(index % capacity).ok()?;
        (offset < row.initialized).then(|| LogicalPosition {
            block: row.id(self.space, ordinal),
            offset,
        })
    }

    fn invalidate_suffix(&mut self, keep: usize) {
        let suffix: Vec<u32> = self.order.drain(keep..).collect();
        for ordinal in suffix {
            let row = &mut self.rows[ordinal as usize];
            row.physical = None;
            row.initialized = 0;
            self.free.push(ordinal);
        }
    }
}

/// One accepted pool-stable logical mapping table.
pub struct AcceptedBlockTable<T> {
    state: TableState<T>,
}

impl<T> Default for AcceptedBlockTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> AcceptedBlockTable<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: TableState::new(crate::fresh_space_id()),
        }
    }

    #[must_use]
    pub const fn space(&self) -> u32 {
        self.state.space
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.state.total_len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.state.total_len == 0
    }

    #[must_use]
    pub const fn items_per_block() -> usize {
        Superblock::<T>::capacity()
    }

    #[must_use]
    pub const fn logical_row_bytes() -> usize {
        size_of::<LogicalRow>()
    }

    #[must_use]
    pub fn metrics(&self) -> ArenaMetrics {
        self.state.metrics_snapshot()
    }

    pub fn push_with<F>(
        &mut self,
        store: &mut BlockStore<T>,
        build: F,
    ) -> Result<LogicalPosition, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        self.state.push_with(store, build)
    }

    pub fn cursor(&mut self) -> LogicalCursor {
        self.state.cursor()
    }

    #[must_use]
    pub fn view<'a>(&'a self, store: &'a BlockStore<T>) -> AcceptedBlockView<'a, T> {
        AcceptedBlockView {
            store,
            state: &self.state,
        }
    }

    pub fn truncate(
        &mut self,
        store: &mut BlockStore<T>,
        cursor: LogicalCursor,
    ) -> Result<(), ArenaError> {
        self.state.validate_cursor(cursor)?;
        if self.state.active_boundary.is_some() {
            return Err(ArenaError::OpenBoundary);
        }
        let keep = cursor.order_len as usize;
        self.state.total_len = cursor.total_len as usize;
        let removed: Vec<u32> = self.state.order.drain(keep..).collect();
        for ordinal in removed {
            let row = &mut self.state.rows[ordinal as usize];
            let physical = row.physical.take().ok_or(ArenaError::StalePhysicalBlock)?;
            row.initialized = 0;
            store.release(physical)?;
            self.state.free.push(ordinal);
            self.state.metrics.logical_rows_released += 1;
        }
        if let Some(id) = cursor.boundary {
            let row = &mut self.state.rows[id.ordinal as usize];
            if cursor.tail_len != 0 && cursor.tail_len < row.initialized {
                let physical = row.physical.ok_or(ArenaError::StalePhysicalBlock)?;
                row.initialized = cursor.tail_len;
                store.truncate(physical, cursor.tail_len as usize)?;
            }
        }
        Ok(())
    }

    pub fn rotate_tail(&mut self) -> Result<WholeBlockBoundary<T>, ArenaError> {
        if self.state.active_boundary.is_some() {
            return Err(ArenaError::OpenBoundary);
        }
        let serial = self.state.next_boundary_serial;
        self.state.next_boundary_serial = serial
            .checked_add(1)
            .ok_or(ArenaError::BoundarySerialExhausted)?;
        self.state.active_boundary = Some(serial);
        self.state.force_new_block = true;
        self.state.metrics.boundary_rotations += 1;
        Ok(WholeBlockBoundary {
            space: self.state.space,
            serial,
            order_len: self.state.live_order_len(),
            _payload: PhantomData,
        })
    }

    pub fn seal_rotated_suffix(
        &mut self,
        boundary: WholeBlockBoundary<T>,
    ) -> Result<BlockRangeOwner<T>, (ArenaError, WholeBlockBoundary<T>)> {
        if boundary.space != self.state.space || self.state.active_boundary != Some(boundary.serial)
        {
            return Err((ArenaError::InvalidBoundary, boundary));
        }
        let suffix = &self.state.order[boundary.order_len..];
        let mut ids = Vec::new();
        if let Err(error) = ids.try_reserve_exact(suffix.len()) {
            return Err((ArenaError::from(error), boundary));
        }
        ids.extend(
            suffix
                .iter()
                .map(|&ordinal| self.state.rows[ordinal as usize].id(self.state.space, ordinal)),
        );
        self.state.active_boundary = None;
        self.state.force_new_block = false;
        Ok(BlockRangeOwner::new(self.state.space, ids))
    }

    /// Cancels a lazy rotation before any suffix block was allocated.
    pub fn cancel_rotation(
        &mut self,
        boundary: WholeBlockBoundary<T>,
    ) -> Result<(), (ArenaError, WholeBlockBoundary<T>)> {
        if boundary.space != self.state.space
            || self.state.active_boundary != Some(boundary.serial)
            || self.state.live_order_len() != boundary.order_len
        {
            return Err((ArenaError::InvalidBoundary, boundary));
        }
        self.state.active_boundary = None;
        self.state.force_new_block = false;
        Ok(())
    }

    #[must_use]
    pub fn empty_block_owner(&self) -> BlockRangeOwner<T> {
        BlockRangeOwner::new(self.state.space, Vec::new())
    }
}

impl<T: Copy> AcceptedBlockTable<T> {
    pub fn fork(
        self,
        store: &mut BlockStore<T>,
        checkpoint: LogicalCursor,
    ) -> Result<AcceptedCandidateTables<T>, (ArenaError, Self)> {
        if let Err(error) = self.state.validate_cursor(checkpoint) {
            return Err((error, self));
        }
        if self.state.active_boundary.is_some() {
            return Err((ArenaError::OpenBoundary, self));
        }
        let mut candidate = match self.state.try_clone() {
            Ok(candidate) => candidate,
            Err(error) => return Err((error, self)),
        };
        let table_entries = candidate.rows.len();
        let live_entries = candidate
            .rows
            .iter()
            .filter(|row| row.physical.is_some())
            .count();
        candidate.metrics.forked_arenas += 1;
        candidate.metrics.table_entries_copied += table_entries as u64;
        candidate.metrics.table_live_entries_copied += live_entries as u64;
        candidate.metrics.table_vacant_entries_copied += (table_entries - live_entries) as u64;
        candidate.metrics.table_bytes_copied += (table_entries * size_of::<LogicalRow>()) as u64;
        let candidate_rows = checkpoint.order_len as usize;
        candidate.invalidate_suffix(candidate_rows);
        candidate.total_len = checkpoint.total_len as usize;
        candidate.force_new_block = false;
        let shared_complete = candidate_rows - usize::from(checkpoint.tail_len > 0);
        if checkpoint.tail_len > 0 {
            let ordinal = candidate.order[candidate_rows - 1];
            let source = self.state.rows[ordinal as usize]
                .physical
                .expect("validated accepted row has a physical block");
            let destination = match store.copy_prefix(source, checkpoint.tail_len as usize) {
                Ok(destination) => destination,
                Err(error) => return Err((error, self)),
            };
            candidate.rows[ordinal as usize].physical = Some(destination);
            candidate.rows[ordinal as usize].initialized = checkpoint.tail_len;
            candidate.metrics.fork_tail_values_copied += checkpoint.tail_len as u64;
            candidate.metrics.fork_tail_bytes_copied +=
                u64::from(checkpoint.tail_len) * size_of::<T>() as u64;
        }
        Ok(AcceptedCandidateTables {
            accepted: self.state,
            candidate,
            shared_complete,
        })
    }
}

/// Move-only proof that the next suffix begins on a fresh physical block.
pub struct WholeBlockBoundary<T> {
    space: u32,
    serial: u64,
    order_len: usize,
    _payload: PhantomData<fn() -> T>,
}

pub struct AcceptedBlockView<'a, T> {
    store: &'a BlockStore<T>,
    state: &'a TableState<T>,
}

impl<'a, T> AcceptedBlockView<'a, T> {
    pub fn get(&self, position: LogicalPosition) -> Result<&'a T, ArenaError> {
        self.state.resolve(self.store, position)
    }
}

pub struct CandidateBlockView<'a, T> {
    store: &'a BlockStore<T>,
    state: &'a TableState<T>,
}

impl<'a, T> CandidateBlockView<'a, T> {
    pub fn get(&self, position: LogicalPosition) -> Result<&'a T, ArenaError> {
        self.state.resolve(self.store, position)
    }
}

/// Sole owner of one accepted mapping and one candidate mapping.
pub struct AcceptedCandidateTables<T: Copy> {
    accepted: TableState<T>,
    candidate: TableState<T>,
    shared_complete: usize,
}

impl<T: Copy> AcceptedCandidateTables<T> {
    #[must_use]
    pub fn views<'a>(
        &'a self,
        store: &'a BlockStore<T>,
    ) -> (AcceptedBlockView<'a, T>, CandidateBlockView<'a, T>) {
        (
            AcceptedBlockView {
                store,
                state: &self.accepted,
            },
            CandidateBlockView {
                store,
                state: &self.candidate,
            },
        )
    }

    pub fn candidate_push_with<F>(
        &mut self,
        store: &mut BlockStore<T>,
        build: F,
    ) -> Result<LogicalPosition, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        self.candidate.push_with(store, build)
    }

    #[must_use]
    pub fn shape(&self) -> ForkShape {
        ForkShape {
            accepted_blocks: self.accepted.order.len(),
            candidate_blocks: self.candidate.order.len(),
            shared_complete_blocks: self.shared_complete,
            candidate_private_blocks: self.candidate.order.len() - self.shared_complete,
        }
    }

    #[must_use]
    pub fn metrics(&self) -> ArenaMetrics {
        self.candidate.metrics_snapshot()
    }

    pub fn accept(
        self,
        store: &mut BlockStore<T>,
    ) -> Result<AcceptedBlockTable<T>, (ArenaError, Self)> {
        for &ordinal in &self.accepted.order[self.shared_complete..] {
            let Some(physical) = self.accepted.rows[ordinal as usize].physical else {
                return Err((ArenaError::StalePhysicalBlock, self));
            };
            if store.resolve(physical).is_err() {
                return Err((ArenaError::StalePhysicalBlock, self));
            }
        }
        for &ordinal in &self.accepted.order[self.shared_complete..] {
            let physical = self.accepted.rows[ordinal as usize]
                .physical
                .expect("preflighted accepted physical block");
            store
                .release(physical)
                .expect("preflight makes acceptance release infallible");
        }
        Ok(AcceptedBlockTable {
            state: self.candidate,
        })
    }

    pub fn reject(
        mut self,
        store: &mut BlockStore<T>,
    ) -> Result<AcceptedBlockTable<T>, (ArenaError, Self)> {
        for &ordinal in &self.candidate.order[self.shared_complete..] {
            let Some(physical) = self.candidate.rows[ordinal as usize].physical else {
                return Err((ArenaError::StalePhysicalBlock, self));
            };
            if store.resolve(physical).is_err() {
                return Err((ArenaError::StalePhysicalBlock, self));
            }
        }
        for &ordinal in &self.candidate.order[self.shared_complete..] {
            let physical = self.candidate.rows[ordinal as usize]
                .physical
                .expect("preflighted candidate physical block");
            store
                .release(physical)
                .expect("preflight makes rejection release infallible");
        }
        self.accepted.metrics = self.candidate.metrics_snapshot();
        self.accepted.direct_observations.set(0);
        self.accepted.stale_observations.set(0);
        Ok(AcceptedBlockTable {
            state: self.accepted,
        })
    }

    pub(crate) fn candidate_position_at(&self, index: usize) -> Option<LogicalPosition> {
        self.candidate.position_at_dense_index(index)
    }

    pub(crate) const fn candidate_len(&self) -> usize {
        self.candidate.total_len
    }
}

impl<T> AcceptedBlockTable<T> {
    pub(crate) fn position_at_dense_index(&self, index: usize) -> Option<LogicalPosition> {
        self.state.position_at_dense_index(index)
    }
}
