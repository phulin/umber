//! Packed logical-chunk physical extent compaction and rollback regrowth.

use super::*;

impl<T> ChunkStorage<T> {
    /// Returns the unused physical suffix of a sealed packed logical chunk.
    /// The logical key and its physical base remain unchanged, so already
    /// admitted reads of the initialized prefix still name the same values.
    pub(super) fn compact_packed_tail(&mut self, key: LogicalChunkId) {
        if self.layout != ChunkStorageLayout::PackedCopy {
            return;
        }
        let row = self.logical_rows[key.ordinal as usize];
        let physical = DenseBlockKey {
            slot: row.physical_slot,
            incarnation: row.physical_incarnation,
        };
        if self.tail_block != Some(physical) {
            return;
        }
        let used = self.chunks[key.ordinal as usize].used;
        if used >= row.physical_capacity {
            return;
        }
        let block = &mut self.blocks[physical.slot as usize];
        if row.physical_base as usize + row.physical_capacity as usize != block.payload().len() {
            return;
        }
        block
            .payload_mut()
            .truncate(row.physical_base as usize + used as usize);
        self.logical_rows[key.ordinal as usize].physical_capacity = used;
    }

    /// Releases one physical extent. Only a full-width extent is reusable as
    /// another open logical chunk; a short interior hole stays with its block.
    pub(super) fn release_dense_extent(
        &mut self,
        physical: DenseBlockKey,
        base: u32,
        extent: u32,
    ) -> Result<(), ForkArenaError> {
        let full_width = extent as usize == self.slots_per_chunk;
        let is_tail = self.tail_block == Some(physical);
        let block = self.dense_block_mut(physical)?;
        block.live_chunks = block
            .live_chunks
            .checked_sub(1)
            .ok_or(ForkArenaError::InvalidChunk)?;
        if block.live_chunks == 0 {
            block.payload_mut().truncate(0);
            self.free_blocks.push(physical.slot);
            if is_tail {
                self.tail_block = None;
            }
            #[cfg(feature = "profiling")]
            self.record_node_pool_storage(NodePoolStorageEvent::Release);
        } else if full_width {
            self.free_ranges.push((physical, base));
        } else if is_tail && base as usize + extent as usize == block.payload().len() {
            block.payload_mut().truncate(base as usize);
        }
        Ok(())
    }

    /// Copies a packed extent to a new full-width reservation when rollback
    /// must reopen a sealed tail that later chunks have followed.
    fn copy_packed_extent(
        &mut self,
        source: (DenseBlockKey, u32),
        destination: (DenseBlockKey, u32),
        used: usize,
    ) -> Result<(), ForkArenaError> {
        let copy = self.packed_copier.ok_or(ForkArenaError::InvalidChunk)?;
        let source_page = source.0.slot as usize;
        let destination_page = destination.0.slot as usize;
        let source_start = source.1 as usize;
        let destination_start = destination.1 as usize;
        if source_page == destination_page {
            let DenseBlockPayload::Packed(block) = self.blocks[source_page].payload_mut() else {
                return Err(ForkArenaError::InvalidChunk);
            };
            let cells = block.initialized_mut();
            if source_start + used <= destination_start {
                let (before, after) = cells.split_at_mut(destination_start);
                copy(
                    &before[source_start..source_start + used],
                    &mut after[..used],
                );
            } else if destination_start + used <= source_start {
                let (before, after) = cells.split_at_mut(source_start);
                copy(
                    &after[..used],
                    &mut before[destination_start..destination_start + used],
                );
            } else {
                return Err(ForkArenaError::InvalidRange);
            }
        } else if source_page < destination_page {
            let (before, after) = self.blocks.split_at_mut(destination_page);
            let DenseBlockPayload::Packed(source_block) = before[source_page].payload() else {
                return Err(ForkArenaError::InvalidChunk);
            };
            let DenseBlockPayload::Packed(destination_block) = after[0].payload_mut() else {
                return Err(ForkArenaError::InvalidChunk);
            };
            copy(
                &source_block.initialized()[source_start..source_start + used],
                &mut destination_block.initialized_mut()
                    [destination_start..destination_start + used],
            );
        } else {
            let (before, after) = self.blocks.split_at_mut(source_page);
            let DenseBlockPayload::Packed(destination_block) =
                before[destination_page].payload_mut()
            else {
                return Err(ForkArenaError::InvalidChunk);
            };
            let DenseBlockPayload::Packed(source_block) = after[0].payload() else {
                return Err(ForkArenaError::InvalidChunk);
            };
            copy(
                &source_block.initialized()[source_start..source_start + used],
                &mut destination_block.initialized_mut()
                    [destination_start..destination_start + used],
            );
        }
        Ok(())
    }

    pub(super) fn restore_full_packed_extent(
        &mut self,
        key: LogicalChunkId,
    ) -> Result<(), ForkArenaError> {
        let row = self.logical_rows[key.ordinal as usize];
        if self.layout != ChunkStorageLayout::PackedCopy
            || row.physical_capacity as usize == self.slots_per_chunk
        {
            return Ok(());
        }
        let old_physical = DenseBlockKey {
            slot: row.physical_slot,
            incarnation: row.physical_incarnation,
        };
        let missing = self.slots_per_chunk - row.physical_capacity as usize;
        let block_capacity = Superblock::<T>::capacity();
        if self.tail_block == Some(old_physical)
            && self.dense_block(old_physical)?.payload().len()
                == row.physical_base as usize + row.physical_capacity as usize
            && self.dense_block(old_physical)?.payload().len() + missing <= block_capacity
        {
            let initializer = self
                .packed_initializer
                .ok_or(ForkArenaError::InvalidChunk)?;
            let DenseBlockPayload::Packed(payload) =
                self.dense_block_mut(old_physical)?.payload_mut()
            else {
                return Err(ForkArenaError::InvalidChunk);
            };
            initializer(payload, missing)?;
            self.logical_rows[key.ordinal as usize].physical_capacity = self.slots_per_chunk as u32;
            return Ok(());
        }
        let next_epoch = self.admission_epoch.saturating_add(1);
        let replacement = self.allocate_dense_range()?;
        let used = self.chunks[key.ordinal as usize].used as usize;
        if let Err(error) =
            self.copy_packed_extent((old_physical, row.physical_base), replacement, used)
        {
            self.release_dense_extent(replacement.0, replacement.1, self.slots_per_chunk as u32)?;
            return Err(error);
        }
        let current = &mut self.logical_rows[key.ordinal as usize];
        current.physical_slot = replacement.0.slot;
        current.physical_incarnation = replacement.0.incarnation;
        current.physical_base = replacement.1;
        current.physical_capacity = self.slots_per_chunk as u32;
        self.admission_epoch = next_epoch;
        self.release_dense_extent(old_physical, row.physical_base, row.physical_capacity)?;
        Ok(())
    }
}
