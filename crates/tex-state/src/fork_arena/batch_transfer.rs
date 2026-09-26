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
            prior_tail: boundary.payload_tail,
            _lane: PhantomData,
        })
    }

    /// A construction batch starts after sealing the preceding tail. Its
    /// cancellation discards only later slots: a nested successful move may
    /// have consumed that immutable predecessor while the build was open.
    pub(crate) fn can_discard_sealed_batch_suffix(
        &self,
        pool: &ChunkPool<T>,
        batch: &BatchMark<Lane>,
        rollback: &OperationMark<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        let boundary = batch.payload_start as usize;
        let current_start = match &self.ownership {
            ForkOwnership::Accepted(_) => self.base_payload_chunks as usize,
            ForkOwnership::Forked { prefix, .. } => {
                self.base_payload_chunks as usize + prefix.payload.len()
            }
        };
        if self.active_builder
            || self.pending_batch.is_some()
            || batch.arena != self.owner
            || rollback.arena != self.owner
            || rollback.payload_chunks != batch.payload_start
            || (batch.prior_tail.is_some() && !rollback.payload_tail_sealed)
            || boundary < current_start
            || boundary > self.live_payload_len()
        {
            return Err(ForkArenaError::InvalidOperationMark);
        }
        if boundary != self.base_payload_chunks as usize
            && let Some(key) = self.live_key_at(boundary - 1)
        {
            if Some(key) != batch.prior_tail {
                return Err(ForkArenaError::InvalidOperationMark);
            }
            let meta = pool
                .payload
                .validate_lineage(key, self.owner, self.lineage)?;
            if meta.used != rollback.payload_tail_used
                || meta.sealed != rollback.payload_tail_sealed
                || meta.sequence_summary != rollback.payload_tail_summary
            {
                return Err(ForkArenaError::InvalidOperationMark);
            }
        }
        Ok(())
    }

    pub(crate) fn discard_sealed_batch_suffix(
        &mut self,
        pool: &mut ChunkPool<T>,
        batch: BatchMark<Lane>,
        rollback: OperationMark<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.can_discard_sealed_batch_suffix(pool, &batch, &rollback)?;
        self.bind_pool(pool)?;
        let discarded = self.detach_suffix(batch.payload_start as usize)?;
        for key in discarded.into_live_keys() {
            self.unindex_chunk(pool, key);
            pool.payload
                .release_lineage(key, self.owner, self.lineage)?;
            self.counters.candidate_chunks_truncated += 1;
        }
        Ok(())
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
        // Every producer settles direct child floors while publishing its
        // payload. An unfinished reservation is not a transferable batch;
        // sealing must not repair it by scanning already published values.
        for (_, key) in self.live_positions_from(mark.payload_start as usize) {
            if !pool
                .payload
                .validate_lineage(key, self.owner, self.lineage)?
                .dependency_metadata_complete
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        let boundary = self.seal_boundary(pool)?;
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

    /// Mutation-free closure preflight for a build suffix whose final tails
    /// have not yet been sealed. This lets semantic rejection preserve even
    /// lifecycle counters and sealed-capacity state.
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

    /// Chunk-only closure proof used by retained-lineage sharing and unique
    /// successor adoption. Direct child floors were folded into metadata at
    /// publication, so this never visits a node payload or follows the tree.
    pub(crate) fn preflight_shared_prefix_metadata(
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
        for (_, key) in self.live_positions_from(mark.payload_start as usize) {
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
        for (position, key) in successor.payload.iter_live_with_positions() {
            self.index_chunk(pool, key, position);
        }
        self.counters.obsolete_chunks_pruned = self
            .counters
            .obsolete_chunks_pruned
            .saturating_add(released as u64);
        self.ownership = ForkOwnership::Accepted(successor);
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
        for (_, key) in payload.iter_live_with_positions() {
            self.unindex_chunk(pool, key);
        }
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
        for (offset, key) in batch.payload.iter_live_with_positions() {
            self.index_chunk(pool, key, batch.payload_start as usize + offset);
        }
        {
            let current = self.current_chunks_mut();
            current.payload.append(batch.payload);
        }
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
            for (_, key) in batch.payload.iter_live_with_positions() {
                pool.payload.used(key, self.owner)?;
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
        for (_, key) in batch.payload.iter_live_with_positions() {
            pool.payload
                .transfer(
                    key,
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
        for (offset, key) in batch.payload.iter_live_with_positions() {
            destination.index_chunk(pool, key, payload_start + offset);
        }
        let promoted = batch.payload.len();
        {
            let current = destination.current_chunks_mut();
            current.payload.append(batch.payload);
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
            for (_, key) in batch.payload.iter_live_with_positions() {
                if !pool.payload.is_sealed(key, self.owner)? {
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
        for (_, key) in payload.iter_live_with_positions() {
            self.unindex_chunk(pool, key);
        }
        for (_, key) in payload.iter_live_with_positions() {
            pool.payload
                .transfer(
                    key,
                    self.owner,
                    self.lineage,
                    destination.owner,
                    destination.lineage,
                )
                .expect("batch payload ownership was preflighted");
        }
        let promoted = payload.len();
        let payload_start = destination.live_payload_len();
        for (offset, key) in payload.iter_live_with_positions() {
            destination.index_chunk(pool, key, payload_start + offset);
        }
        {
            let current = destination.current_chunks_mut();
            current.payload.append(payload);
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
