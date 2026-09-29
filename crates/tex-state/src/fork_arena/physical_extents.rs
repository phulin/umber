//! Physical extent compaction and exclusive rollback regrowth for logical chunks.

use super::*;

impl<T> ChunkStorage<T> {
    /// Returns the unused physical suffix of a sealed logical chunk.
    /// The logical key and its physical base remain unchanged, so already
    /// admitted reads of the initialized prefix still name the same values.
    pub(super) fn compact_sealed_tail(&mut self, key: LogicalChunkId) {
        let row = self.chunks[key.ordinal as usize];
        let physical = row.physical();
        if self.tail_block != Some(physical) {
            return;
        }
        let used = row.used;
        let capacity = row.physical_capacity(self.slots_per_chunk);
        if used >= capacity {
            return;
        }
        let block = &mut self.blocks[physical.slot as usize];
        if row.physical_base as usize + capacity as usize != block.payload().len() {
            return;
        }
        if let DenseBlockPayload::Optional(payload) = block.payload() {
            debug_assert!(
                payload.initialized()[row.physical_base as usize + used as usize..]
                    .iter()
                    .all(Option::is_none)
            );
        }
        block
            .payload_mut()
            .truncate(row.physical_base as usize + used as usize);
        self.chunks[key.ordinal as usize].physical_compact = true;
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

    /// Moves initialized optional values into a distinct, vacant reservation.
    /// Every bound and vacancy is checked before the first `take`, making the
    /// move infallible and preserving exactly-once drop for non-Copy values.
    fn move_optional_extent(
        &mut self,
        source: (DenseBlockKey, u32),
        destination: (DenseBlockKey, u32),
        used: usize,
    ) -> Result<(), ForkArenaError> {
        fn move_cells<T>(
            source: &mut [Option<T>],
            destination: &mut [Option<T>],
        ) -> Result<(), ForkArenaError> {
            if source.len() != destination.len()
                || source.iter().any(Option::is_none)
                || destination.iter().any(Option::is_some)
            {
                return Err(ForkArenaError::InvalidChunk);
            }
            for (from, to) in source.iter_mut().zip(destination) {
                *to = from.take();
            }
            Ok(())
        }

        self.dense_block(source.0)?;
        self.dense_block(destination.0)?;
        let source_page = source.0.slot as usize;
        let destination_page = destination.0.slot as usize;
        let source_start = source.1 as usize;
        let destination_start = destination.1 as usize;
        if source_page == destination_page {
            let DenseBlockPayload::Optional(block) = self.blocks[source_page].payload_mut() else {
                return Err(ForkArenaError::InvalidChunk);
            };
            let cells = block.initialized_mut();
            if source_start + used <= destination_start {
                let (before, after) = cells.split_at_mut(destination_start);
                move_cells(
                    before
                        .get_mut(source_start..source_start + used)
                        .ok_or(ForkArenaError::InvalidRange)?,
                    after.get_mut(..used).ok_or(ForkArenaError::InvalidRange)?,
                )
            } else if destination_start + used <= source_start {
                let (before, after) = cells.split_at_mut(source_start);
                move_cells(
                    after.get_mut(..used).ok_or(ForkArenaError::InvalidRange)?,
                    before
                        .get_mut(destination_start..destination_start + used)
                        .ok_or(ForkArenaError::InvalidRange)?,
                )
            } else {
                Err(ForkArenaError::InvalidRange)
            }
        } else if source_page < destination_page {
            let (before, after) = self.blocks.split_at_mut(destination_page);
            let DenseBlockPayload::Optional(source_block) = before[source_page].payload_mut()
            else {
                return Err(ForkArenaError::InvalidChunk);
            };
            let DenseBlockPayload::Optional(destination_block) = after[0].payload_mut() else {
                return Err(ForkArenaError::InvalidChunk);
            };
            move_cells(
                source_block
                    .initialized_mut()
                    .get_mut(source_start..source_start + used)
                    .ok_or(ForkArenaError::InvalidRange)?,
                destination_block
                    .initialized_mut()
                    .get_mut(destination_start..destination_start + used)
                    .ok_or(ForkArenaError::InvalidRange)?,
            )
        } else {
            let (before, after) = self.blocks.split_at_mut(source_page);
            let DenseBlockPayload::Optional(destination_block) =
                before[destination_page].payload_mut()
            else {
                return Err(ForkArenaError::InvalidChunk);
            };
            let DenseBlockPayload::Optional(source_block) = after[0].payload_mut() else {
                return Err(ForkArenaError::InvalidChunk);
            };
            move_cells(
                source_block
                    .initialized_mut()
                    .get_mut(source_start..source_start + used)
                    .ok_or(ForkArenaError::InvalidRange)?,
                destination_block
                    .initialized_mut()
                    .get_mut(destination_start..destination_start + used)
                    .ok_or(ForkArenaError::InvalidRange)?,
            )
        }
    }

    pub(super) fn restore_full_extent(
        &mut self,
        key: LogicalChunkId,
    ) -> Result<(), ForkArenaError> {
        let row = self.chunks[key.ordinal as usize];
        let capacity = row.physical_capacity(self.slots_per_chunk);
        if capacity as usize == self.slots_per_chunk {
            // A compact extent that is exactly full needs no regrowth.
            self.chunks[key.ordinal as usize].physical_compact = false;
            return Ok(());
        }
        let old_physical = row.physical();
        let missing = self.slots_per_chunk - capacity as usize;
        let block_capacity = match self.layout {
            ChunkStorageLayout::OptionalSlots => Superblock::<Option<T>>::capacity(),
            ChunkStorageLayout::PackedCopy => Superblock::<T>::capacity(),
        };
        if self.tail_block == Some(old_physical)
            && self.dense_block(old_physical)?.payload().len()
                == row.physical_base as usize + capacity as usize
            && self.dense_block(old_physical)?.payload().len() + missing <= block_capacity
        {
            let initializer = self.packed_initializer;
            match self.dense_block_mut(old_physical)?.payload_mut() {
                DenseBlockPayload::Optional(payload) => payload
                    .extend_with(missing, || None)
                    .map_err(|_| ForkArenaError::CapacityOverflow)?,
                DenseBlockPayload::Packed(payload) => {
                    initializer.ok_or(ForkArenaError::InvalidChunk)?(payload, missing)?
                }
            }
            self.chunks[key.ordinal as usize].physical_compact = false;
            return Ok(());
        }
        let next_epoch = self.admission_epoch.saturating_add(1);
        let replacement = self.allocate_dense_range()?;
        let used = self.chunks[key.ordinal as usize].used as usize;
        let relocated = match self.layout {
            ChunkStorageLayout::OptionalSlots => {
                self.move_optional_extent((old_physical, row.physical_base), replacement, used)
            }
            ChunkStorageLayout::PackedCopy => {
                self.copy_packed_extent((old_physical, row.physical_base), replacement, used)
            }
        };
        if let Err(error) = relocated {
            self.release_dense_extent(replacement.0, replacement.1, self.slots_per_chunk as u32)?;
            return Err(error);
        }
        let current = &mut self.chunks[key.ordinal as usize];
        current.physical_slot = replacement.0.slot;
        current.physical_incarnation = replacement.0.incarnation;
        current.physical_base = replacement.1;
        current.physical_compact = false;
        self.admission_epoch = next_epoch;
        self.release_dense_extent(old_physical, row.physical_base, capacity)?;
        Ok(())
    }
}
