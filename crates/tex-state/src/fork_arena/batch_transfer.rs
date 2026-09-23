//! batch transfer operations on the existing impl owner.

use super::*;

impl<T, Lane> ForkArena<T, Lane> {
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

    pub(crate) fn is_forked(&self) -> bool {
        matches!(self.ownership, ForkOwnership::Forked { .. })
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
    pub(super) fn complete_legacy_suffix_dependencies(
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
    pub(super) fn preflight_shared_prefix_metadata(
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
}
