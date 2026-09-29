//! Single-chunk list reads and exact sealed-chunk publication.
//!
//! Recursive explicit copies are dominated by short lists: a copied box
//! usually names a child list that fits in one logical chunk. These
//! primitives admit such a source root once and publish its relocated copy
//! as one already sealed logical chunk whose physical extent is exactly its
//! length. They produce the same final state as reserving a full-width
//! chunk, appending the run, sealing it, and returning the unused physical
//! suffix, without the intermediate reservation plan or re-admission.

use super::*;

impl<T> ChunkStorage<T> {
    /// Appends `values` as one new sealed logical chunk.
    ///
    /// The physical extent is exactly `values.len()` slots at the end of the
    /// current tail superblock, as a sealed-and-compacted tail would leave
    /// it. Rollback that reopens this chunk recovers a full-width extent
    /// through the ordinary short-extent path.
    fn allocate_sealed_exact(
        &mut self,
        arena: u32,
        lineage: u32,
        values: &[T],
        dependency_floor: usize,
        paired_dependency_floor: usize,
    ) -> Result<LogicalChunkId, ForkArenaError>
    where
        T: Copy,
    {
        let len = values.len();
        if len == 0 || len > self.slots_per_chunk {
            return Err(ForkArenaError::InvalidRange);
        }
        let capacity = match self.layout {
            ChunkStorageLayout::OptionalSlots => Superblock::<Option<T>>::capacity(),
            ChunkStorageLayout::PackedCopy => Superblock::<T>::capacity(),
        };
        let tail = self.tail_block.filter(|key| {
            self.dense_block(*key)
                .is_ok_and(|block| block.payload().len() + len <= capacity)
        });
        let physical = match tail {
            Some(key) => key,
            None => {
                let key = self.allocate_dense_block()?;
                self.tail_block = Some(key);
                key
            }
        };
        let base = u32::try_from(self.dense_block(physical)?.payload().len())
            .map_err(|_| ForkArenaError::CapacityOverflow)?;
        let key = self.allocate_logical(physical, base)?;
        let block = self.dense_block_mut(physical)?;
        match block.payload_mut() {
            DenseBlockPayload::Optional(payload) => {
                let mut values = values.iter();
                payload
                    .extend_with(len, || values.next().copied())
                    .map_err(|_| ForkArenaError::CapacityOverflow)?;
            }
            DenseBlockPayload::Packed(payload) => payload
                .extend_copy_from_slice(values)
                .map_err(|_| ForkArenaError::CapacityOverflow)?,
        }
        block.live_chunks += 1;
        self.logical_rows[key.ordinal as usize].physical_capacity = len as u32;
        let meta = ChunkMeta::fresh(
            key.incarnation,
            arena,
            lineage,
            len as u32,
            true,
            dependency_floor,
            paired_dependency_floor,
        );
        if key.ordinal as usize == self.chunks.len() {
            self.chunks.push(meta);
        } else {
            *self
                .chunks
                .get_mut(key.ordinal as usize)
                .ok_or(ForkArenaError::InvalidChunk)? = meta;
        }
        Ok(key)
    }
}

impl<T, Lane> ForkArena<T, Lane> {
    /// Copies an owned root that lies inside one logical chunk into `out`.
    ///
    /// Returns `Ok(false)` without reading when the root spans several
    /// chunks. The admission checks are those of
    /// [`Self::admit_owned_root`] for its coinciding endpoints.
    pub(crate) fn read_single_chunk_list(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        out: &mut Vec<T>,
    ) -> Result<bool, ForkArenaError>
    where
        T: Copy,
    {
        if list.is_empty() || list.head.raw != list.tail.raw {
            return Ok(false);
        }
        self.validate_pool(pool)?;
        if list.space != pool.payload.logical_space() {
            return Err(ForkArenaError::ForeignArena);
        }
        let chunk = pool
            .payload
            .admit_owned_chunk(list.head.raw, self.owner, self.lineage)
            .ok_or(ForkArenaError::InvalidRange)?;
        if list.head.offset >= list.tail.offset
            || list.tail.offset > chunk.used
            || list.tail.offset - list.head.offset != list.len
        {
            return Err(ForkArenaError::InvalidRange);
        }
        let block = chunk.block;
        match pool
            .payload
            .admitted_dense_slice(block, list.head.offset..list.tail.offset)
            .ok_or(ForkArenaError::InvalidRange)?
        {
            DenseBlockSlice::Optional(cells) => {
                for cell in cells {
                    out.push(cell.ok_or(ForkArenaError::InvalidRange)?);
                }
            }
            DenseBlockSlice::Packed(cells) => out.extend_from_slice(cells),
        }
        Ok(true)
    }

    /// Records per logical chunk in this arena's pool.
    pub(crate) fn chunk_capacity(&self, pool: &ChunkPool<T>) -> usize {
        pool.payload.slots_per_chunk
    }

    /// Owner-relative position that the next published chunk will occupy.
    pub(crate) fn next_payload_position(&self) -> usize {
        self.live_payload_len()
    }

    /// Publishes a fresh private list of at most one chunk as a sealed
    /// exact-extent logical chunk and returns its root.
    ///
    /// The chunk takes [`Self::next_payload_position`]. The caller's paired
    /// operation marks cover it exactly as they cover a reserved run.
    pub(crate) fn publish_sealed_chunk_list(
        &mut self,
        pool: &mut ChunkPool<T>,
        values: &[T],
        dependency_floor: Option<usize>,
        paired_dependency_floor: Option<usize>,
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
        let key = pool.payload.allocate_sealed_exact(
            self.owner,
            self.lineage,
            values,
            dependency_floor.unwrap_or(usize::MAX),
            paired_dependency_floor.unwrap_or(usize::MAX),
        )?;
        self.current_chunks_mut().payload.push(key);
        let position = self.live_payload_len() - 1;
        self.index_chunk(pool, key, position);
        let len = values.len() as u32;
        let unused = pool.payload.slots_per_chunk - values.len();
        self.counters.direct_blocks_allocated =
            self.counters.direct_blocks_allocated.saturating_add(1);
        self.counters.chunks_sealed = self.counters.chunks_sealed.saturating_add(1);
        self.counters.unused_sealed_bytes = self.counters.unused_sealed_bytes.saturating_add(
            u64::try_from(unused.saturating_mul(pool.payload.resident_slot_bytes()))
                .unwrap_or(u64::MAX),
        );
        self.counters.new_semantic_nodes += u64::from(len);
        Ok(ArenaListId::from_root(
            pool.payload.logical_space(),
            ChunkCursor::new(key, 0),
            ChunkCursor::new(key, len),
            len,
        ))
    }
}
