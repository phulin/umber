//! checkpoint lifecycle operations on the existing impl owner.

use super::*;

impl<T, Lane> ForkArena<T, Lane> {
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
    /// Restores the current transactional lineage to a sealed checkpoint
    /// while leaving the detached accepted suffix parked.
    ///
    /// This is the candidate-local counterpart of
    /// [`Self::restore_accepted_checkpoint`].  It is intentionally narrow:
    /// callers must already own the sole forked lineage and may only name a
    /// checkpoint whose whole-chunk mark belongs to this arena.  Exposing the
    /// operation lets the execution layer rewind output together with its
    /// aggregate engine checkpoint after a host resource miss; it does not
    /// create another lineage or capture a finer-grained snapshot.
    pub fn restore_current_checkpoint(
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
        // A retained prefix release rebases the arena at the floor's whole
        // chunk. The floor mark intentionally keeps its old tail identity for
        // equality/evidence, but that physical chunk has already been
        // returned to the pool; at the rebased base there is no partial tail
        // to restore or truncate.
        let at_base = mark.payload_chunks as usize == self.base_payload_chunks as usize;
        let payload_tail_used = match (at_base, mark.payload_tail) {
            (true, _) => 0,
            (false, Some(key)) => pool.payload.used(key, self.owner)?,
            (false, None) => 0,
        };
        let payload_tail_summary = match (at_base, mark.payload_tail) {
            (true, _) | (false, None) => None,
            (false, Some(key)) => pool.payload.sequence_summary(key, self.owner)?,
        };
        self.truncate_payload(
            pool,
            mark.payload_chunks as usize,
            payload_tail_used,
            !at_base && mark.payload_tail.is_some(),
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

    pub(super) fn release_set(
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
}
