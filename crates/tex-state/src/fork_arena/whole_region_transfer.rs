//! Whole-region closure and transfer on the existing typed arena.

use super::*;

impl<T, Lane> ForkArena<T, Lane> {
    pub(crate) fn preflight_interval_root(
        &self,
        pool: &ChunkPool<T>,
        root: ArenaListId<Lane>,
        start: usize,
        end: usize,
    ) -> Result<(), ForkArenaError> {
        self.validate_list_in_suffix(pool, root, start)?;
        if root.is_empty() {
            return Err(ForkArenaError::InvalidRange);
        }
        let tail = self
            .resolved_position(pool, root.tail.raw)
            .ok_or(ForkArenaError::InvalidRange)?;
        (tail < end)
            .then_some(())
            .ok_or(ForkArenaError::InvalidRegion)
    }

    pub(crate) fn preflight_paired_interval_floor(
        &self,
        pool: &ChunkPool<T>,
        start: usize,
        end: usize,
        paired_start: usize,
    ) -> Result<(), ForkArenaError> {
        for position in start..end {
            let key = self
                .live_key_at(position)
                .ok_or(ForkArenaError::InvalidRegion)?;
            let meta = pool
                .payload
                .validate_lineage(key, self.owner, self.lineage)?;
            if meta.paired_dependency_floor < paired_start {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    pub(crate) fn preflight_interior_interval<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        start: usize,
        end: usize,
    ) -> Result<(), ForkArenaError> {
        self.can_seal_boundary(pool)?;
        destination.can_seal_boundary(pool)?;
        if self.owner == destination.owner
            || self.pending_batch.is_some()
            || !matches!(self.ownership, ForkOwnership::Accepted(_))
            || !matches!(destination.ownership, ForkOwnership::Accepted(_))
            || destination.live_payload_len() != 0
            || start < self.base_payload_chunks as usize
            || start > end
            || end > self.live_payload_len()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let (start, end) = (u32::try_from(start), u32::try_from(end));
        let (Ok(start), Ok(end)) = (start, end) else {
            return Err(ForkArenaError::CapacityOverflow);
        };
        for position in start as usize..end as usize {
            let key = self
                .live_key_at(position)
                .ok_or(ForkArenaError::InvalidRegion)?;
            let meta = pool
                .payload
                .validate_lineage(key, self.owner, self.lineage)?;
            if !meta.sealed
                || !meta.dependency_metadata_complete
                || meta.dependency_floor < start as usize
                || meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    /// Transfers whole interior logical chunks into an empty independent
    /// owner. Their original positions remain stable in both the source's
    /// vacant slots and the destination, so no resident value is rewritten.
    pub(crate) fn transfer_interior_interval<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        start: usize,
        end: usize,
    ) -> Result<TransferredInterval<Lane>, ForkArenaError> {
        self.preflight_interior_interval(pool, destination, start, end)?;
        destination.bind_pool(pool)?;
        let base = self.base_payload_chunks as usize;
        let ForkOwnership::Accepted(source) = &mut self.ownership else {
            unreachable!("interior source was preflighted as accepted");
        };
        let moved = source.payload[start - base..end - base].to_vec();
        source.payload[start - base..end - base].fill(VACANT_LOGICAL_CHUNK);
        destination.base_payload_chunks = start as u32;
        for (offset, key) in moved.iter().copied().enumerate() {
            self.unindex_chunk(pool, key);
            pool.payload
                .transfer(
                    key,
                    self.owner,
                    self.lineage,
                    destination.owner,
                    destination.lineage,
                )
                .expect("interior chunk transfer was preflighted");
            destination.index_chunk(pool, key, start + offset);
        }
        destination.current_chunks_mut().payload.extend(moved);
        Ok(TransferredInterval {
            source: self.owner,
            destination: destination.owner,
            start: start as u32,
            end: end as u32,
            _lane: PhantomData,
        })
    }

    /// Reverses one interior transfer before the source operation restores
    /// its earlier roots. Any destination suffix created around the selected
    /// interval is discarded first.
    pub(crate) fn rollback_interior_interval<Source>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Source>,
        loan: TransferredInterval<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.can_seal_boundary(pool)?;
        destination.can_seal_boundary(pool)?;
        let (start, end) = (loan.start as usize, loan.end as usize);
        if self.owner != loan.source
            || destination.owner != loan.destination
            || !matches!(self.ownership, ForkOwnership::Accepted(_))
            || !matches!(destination.ownership, ForkOwnership::Accepted(_))
            || destination.base_payload_chunks != loan.start
            || destination.live_payload_len() < end
            || end > self.live_payload_len()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        for position in start..end {
            if self.live_key_at(position).is_some() || destination.live_key_at(position).is_none() {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        let extra = destination.detach_suffix(end)?;
        for key in extra {
            if key != VACANT_LOGICAL_CHUNK {
                destination.unindex_chunk(pool, key);
                pool.payload
                    .release_lineage(key, destination.owner, destination.lineage)?;
            }
        }
        let selected = destination.detach_suffix(start)?;
        let base = self.base_payload_chunks as usize;
        for (offset, key) in selected.into_iter().enumerate() {
            destination.unindex_chunk(pool, key);
            pool.payload.transfer(
                key,
                destination.owner,
                destination.lineage,
                self.owner,
                self.lineage,
            )?;
            self.index_chunk(pool, key, start + offset);
            let ForkOwnership::Accepted(source) = &mut self.ownership else {
                unreachable!("interior source was preflighted as accepted");
            };
            source.payload[start + offset - base] = key;
        }
        destination.base_payload_chunks = 0;
        Ok(())
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
        for position in self.base_payload_chunks as usize..self.live_payload_len() {
            let Some(key) = self.live_key_at(position) else {
                continue;
            };
            let meta = pool
                .payload
                .validate_lineage(key, self.owner, self.lineage)?;
            if !meta.dependency_metadata_complete
                || meta.dependency_floor < self.base_payload_chunks as usize
            {
                return Err(ForkArenaError::InvalidRegion);
            }
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
            .detach_suffix(self.base_payload_chunks as usize)
            .expect("whole-region source suffix was preflighted");
        for key in &payload {
            if *key != VACANT_LOGICAL_CHUNK {
                self.unindex_chunk(pool, *key);
            }
        }
        for key in &payload {
            if *key != VACANT_LOGICAL_CHUNK {
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
        }
        let promoted = payload.len();
        let payload_start = destination.live_payload_len();
        for (offset, key) in payload.iter().copied().enumerate() {
            if key != VACANT_LOGICAL_CHUNK {
                destination.index_chunk(pool, key, payload_start + offset);
            }
        }
        destination.current_chunks_mut().payload.extend(payload);
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

    pub(super) fn validate_suffix(
        &self,
        start: usize,
        end: usize,
    ) -> Result<Vec<LogicalChunkId>, ForkArenaError> {
        let live_len = self.live_payload_len();
        if start > end || end != live_len {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok((start..end)
            .filter_map(|index| self.live_key_at(index))
            .collect())
    }

    pub(super) fn detach_suffix(
        &mut self,
        start: usize,
    ) -> Result<Vec<LogicalChunkId>, ForkArenaError> {
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
        Ok(detached)
    }

    pub(super) fn validate_list_in_suffix(
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

    pub(super) fn validate_list_endpoints_in_suffix(
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
}
