use core::cell::Cell;

use tex_dense_prefix::{Superblock, VacantSlot};

use crate::{ArenaError, ArenaMetrics};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BlockId {
    slot: u32,
    incarnation: u32,
}

struct BlockSlot<T> {
    incarnation: u32,
    live: bool,
    block: Superblock<T>,
}

/// Caller-owned typed physical storage.
///
/// Physical identities never leave this crate. Callers address values only
/// through logical tables and their borrowed views.
pub struct BlockStore<T> {
    slots: Vec<BlockSlot<T>>,
    free: Vec<u32>,
    metrics: ArenaMetrics,
    stale_observations: Cell<u64>,
}

impl<T> Default for BlockStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> BlockStore<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            metrics: ArenaMetrics {
                superblocks_allocated: 0,
                superblocks_reused: 0,
                superblocks_released: 0,
                blocks_truncated: 0,
                values_constructed: 0,
                values_truncated: 0,
                direct_lookups: 0,
                descriptor_visits: 0,
                cursor_captures: 0,
                logical_rows_created: 0,
                logical_rows_reused: 0,
                logical_rows_released: 0,
                logical_stale_rejections: 0,
                physical_stale_rejections: 0,
                forked_arenas: 0,
                fork_tail_values_copied: 0,
                fork_tail_bytes_copied: 0,
                table_entries_copied: 0,
                table_live_entries_copied: 0,
                table_vacant_entries_copied: 0,
                table_bytes_copied: 0,
                accepted_payload_copies: 0,
                rejected_payload_copies: 0,
                boundary_rotations: 0,
                boundary_slack_values: 0,
                block_ranges_detached: 0,
                block_ranges_prepared: 0,
                block_ranges_transferred: 0,
                block_ranges_rolled_back: 0,
            },
            stale_observations: Cell::new(0),
        }
    }

    #[must_use]
    pub fn metrics(&self) -> ArenaMetrics {
        let mut metrics = self.metrics;
        metrics.physical_stale_rejections += self.stale_observations.get();
        metrics
    }

    #[must_use]
    pub fn live_blocks(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    #[must_use]
    pub fn reusable_blocks(&self) -> usize {
        self.free.len()
    }

    pub(crate) fn allocate(&mut self) -> Result<BlockId, ArenaError> {
        if let Some(&slot) = self.free.last() {
            let entry = self
                .slots
                .get_mut(slot as usize)
                .ok_or(ArenaError::StalePhysicalBlock)?;
            let incarnation = entry
                .incarnation
                .checked_add(1)
                .ok_or(ArenaError::IncarnationExhausted)?;
            debug_assert!(!entry.live && entry.block.is_empty());
            self.free.pop();
            entry.incarnation = incarnation;
            entry.live = true;
            self.metrics.superblocks_reused += 1;
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
        self.metrics.superblocks_allocated += 1;
        Ok(BlockId {
            slot,
            incarnation: 1,
        })
    }

    pub(crate) fn resolve(&self, id: BlockId) -> Result<&Superblock<T>, ArenaError> {
        let Some(entry) = self.slots.get(id.slot as usize) else {
            self.stale_observations
                .set(self.stale_observations.get() + 1);
            return Err(ArenaError::StalePhysicalBlock);
        };
        if !entry.live || entry.incarnation != id.incarnation {
            self.stale_observations
                .set(self.stale_observations.get() + 1);
            return Err(ArenaError::StalePhysicalBlock);
        }
        Ok(&entry.block)
    }

    pub(crate) fn resolve_mut(&mut self, id: BlockId) -> Result<&mut Superblock<T>, ArenaError> {
        let entry = self
            .slots
            .get_mut(id.slot as usize)
            .ok_or(ArenaError::StalePhysicalBlock)?;
        if !entry.live || entry.incarnation != id.incarnation {
            return Err(ArenaError::StalePhysicalBlock);
        }
        Ok(&mut entry.block)
    }

    pub(crate) fn push_with<F>(&mut self, id: BlockId, build: F) -> Result<(), ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        self.resolve_mut(id)?.push_with(build)?;
        self.metrics.values_constructed += 1;
        Ok(())
    }

    pub(crate) fn copy_prefix(&mut self, source: BlockId, len: usize) -> Result<BlockId, ArenaError>
    where
        T: Copy,
    {
        let destination = self.allocate()?;
        let result = self.copy_into(source, destination, len);
        if let Err(error) = result {
            let _ = self.release(destination);
            return Err(error);
        }
        self.metrics.values_constructed += len as u64;
        Ok(destination)
    }

    fn copy_into(
        &mut self,
        source: BlockId,
        destination: BlockId,
        len: usize,
    ) -> Result<(), ArenaError>
    where
        T: Copy,
    {
        if source.slot == destination.slot {
            return Err(ArenaError::StalePhysicalBlock);
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
            || len > source_entry.block.len()
        {
            return Err(ArenaError::StalePhysicalBlock);
        }
        destination_entry
            .block
            .extend_copy_from_slice(&source_entry.block.initialized()[..len])?;
        Ok(())
    }

    pub(crate) fn truncate(&mut self, id: BlockId, len: usize) -> Result<(), ArenaError> {
        let old_len = self.resolve(id)?.len();
        if len > old_len {
            return Err(ArenaError::InvalidCursor);
        }
        if len < old_len {
            self.resolve_mut(id)?.truncate(len);
            self.metrics.blocks_truncated += 1;
            self.metrics.values_truncated += (old_len - len) as u64;
        }
        Ok(())
    }

    pub(crate) fn release(&mut self, id: BlockId) -> Result<(), ArenaError> {
        let entry = self
            .slots
            .get_mut(id.slot as usize)
            .ok_or(ArenaError::StalePhysicalBlock)?;
        if !entry.live || entry.incarnation != id.incarnation {
            self.stale_observations
                .set(self.stale_observations.get() + 1);
            return Err(ArenaError::StalePhysicalBlock);
        }
        let removed = entry.block.len();
        entry.live = false;
        entry.block.truncate(0);
        self.free.push(id.slot);
        self.metrics.superblocks_released += 1;
        self.metrics.values_truncated += removed as u64;
        Ok(())
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn force_incarnation_exhaustion(&mut self, id: BlockId) {
        self.slots[id.slot as usize].incarnation = u32::MAX;
    }
}
