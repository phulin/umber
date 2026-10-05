//! Actual construction boundaries, cancellation, sharing, and adoption.

use super::*;

impl<T, Lane> ForkArena<T, Lane> {
    pub fn begin_batch(
        &mut self,
        pool: &mut ChunkPool<T>,
    ) -> Result<BatchMark<Lane>, ForkArenaError> {
        self.can_seal_boundary(pool)?;
        Ok(self.begin_proved_batch(pool))
    }

    /// Opens a batch after paired preflight, without re-admitting its boundary.
    pub(crate) fn begin_proved_batch(&mut self, pool: &mut ChunkPool<T>) -> BatchMark<Lane> {
        let boundary = self.seal_proved_boundary(pool);
        BatchMark {
            arena: self.owner,
            payload_start: boundary.payload_chunks,
            prior_tail: boundary.payload_tail,
            _lane: PhantomData,
        }
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
            ForkOwnership::Forked(forked) => {
                self.base_payload_chunks as usize + forked.prefix.payload.len()
            }
        };
        if self.active_builder
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
                || pool.payload.summary(key) != rollback.payload_tail_summary
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
        matches!(self.ownership, ForkOwnership::Forked(_))
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
        if self.active_builder || mark.arena != self.owner {
            return Err(ForkArenaError::InvalidRegion);
        }
        let payload_end = self.live_payload_len();
        let payload = self.validate_suffix(mark.payload_start as usize, payload_end)?;
        for list in lists {
            self.validate_list_in_suffix(pool, *list, mark.payload_start as usize)?;
        }
        // Leaf lanes store no inline list coordinate, so the root checks
        // above are the whole closure proof.
        if !T::HAS_INLINE_REGION_LISTS {
            return Ok(());
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
        if self.active_builder || mark.arena != self.owner {
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
                || meta.dependency_floor() < mark.payload_start as usize
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
}
