//! Exact interior moves into vacant logical slots between projected cuts.

use super::*;

pub(crate) struct TransferredHoleIntervals<Lane> {
    source: u32,
    destination: u32,
    ranges: Vec<Range<usize>>,
    live_counts: Vec<usize>,
    source_start: usize,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<T, Lane> ForkArena<T, Lane> {
    fn hole_source_start(&self) -> usize {
        let base = self.base_payload_chunks as usize;
        match &self.ownership {
            ForkOwnership::Accepted(_) => base,
            ForkOwnership::Forked { prefix, .. } => base + prefix.payload.len(),
        }
    }

    /// Checks every selected source key and the corresponding destination
    /// vacancy before either lane changes ownership. A caller must first
    /// detach any predecessor outside the selected set.
    pub(crate) fn preflight_interior_holes<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        ranges: &[Range<usize>],
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.can_seal_boundary(pool)?;
        destination.can_seal_boundary(pool)?;
        if self.owner == destination.owner
            || self.pending_batch.is_some()
            || !matches!(destination.ownership, ForkOwnership::Accepted(_))
            || destination.base_payload_chunks != 0
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let mut previous_end = self.hole_source_start();
        for range in ranges {
            if range.start < previous_end
                || range.start >= range.end
                || range.end > self.live_payload_len()
                || range.end > destination.live_payload_len()
            {
                return Err(ForkArenaError::InvalidRegion);
            }
            u32::try_from(range.end).map_err(|_| ForkArenaError::CapacityOverflow)?;
            for position in range.clone() {
                let key = self
                    .live_key_at(position)
                    .ok_or(ForkArenaError::InvalidRegion)?;
                if destination.live_key_at(position).is_some() {
                    return Err(ForkArenaError::InvalidRegion);
                }
                let meta = pool
                    .payload
                    .validate_lineage(key, self.owner, self.lineage)?;
                if !meta.sealed
                    || !meta.dependency_metadata_complete
                    || meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1
                {
                    return Err(ForkArenaError::InvalidRegion);
                }
                if let Some((previous, _)) = pool.payload.previous_in_list(key, self.owner)? {
                    let predecessor = self
                        .resolved_position(pool, previous)
                        .ok_or(ForkArenaError::InvalidRegion)?;
                    let index = ranges.partition_point(|range| range.end <= predecessor);
                    if !ranges
                        .get(index)
                        .is_some_and(|range| range.contains(&predecessor))
                    {
                        return Err(ForkArenaError::InvalidRegion);
                    }
                }
            }
            previous_end = range.end;
        }
        Ok(())
    }

    pub(crate) fn transfer_interior_into_holes<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        ranges: &[Range<usize>],
    ) -> Result<TransferredHoleIntervals<Lane>, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.preflight_interior_holes(pool, destination, ranges)?;
        self.bind_pool(pool)?;
        destination.bind_pool(pool)?;
        let source_start = self.hole_source_start();
        let base = ranges.first().map_or(0, |range| range.start);
        let relative = ranges
            .iter()
            .map(|range| range.start - source_start..range.end - source_start)
            .collect::<Vec<_>>();
        let (moved, live_counts) = self
            .current_chunks_mut()
            .payload
            .take_selected_ranges(&relative);
        for (offset, key) in moved.iter_live_with_positions() {
            self.unindex_chunk(pool, key);
            pool.payload
                .transfer(
                    key,
                    self.owner,
                    self.lineage,
                    destination.owner,
                    destination.lineage,
                )
                .expect("hole transfer preflight proved exclusive selected key");
            destination.index_chunk(pool, key, base + offset);
        }
        if !ranges.is_empty() {
            destination
                .current_chunks_mut()
                .payload
                .restore_sparse_overlay(base, moved);
        }
        Ok(TransferredHoleIntervals {
            source: self.owner,
            destination: destination.owner,
            ranges: ranges.to_vec(),
            live_counts,
            source_start,
            _lane: PhantomData,
        })
    }

    pub(crate) fn preflight_rollback_interior_holes<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        loan: &TransferredHoleIntervals<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.can_seal_boundary(pool)?;
        destination.can_seal_boundary(pool)?;
        if loan.source != self.owner
            || loan.destination != destination.owner
            || self.hole_source_start() != loan.source_start
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        for (range, expected) in loan.ranges.iter().zip(&loan.live_counts) {
            let mut actual = 0;
            for position in range.clone() {
                if self.live_key_at(position).is_some() {
                    return Err(ForkArenaError::InvalidRegion);
                }
                let key = destination
                    .live_key_at(position)
                    .ok_or(ForkArenaError::InvalidRegion)?;
                pool.payload
                    .validate_lineage(key, destination.owner, destination.lineage)?;
                actual += 1;
            }
            if actual != *expected {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    pub(crate) fn rollback_interior_holes<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        loan: TransferredHoleIntervals<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.preflight_rollback_interior_holes(pool, destination, &loan)?;
        let base = loan.ranges.first().map_or(0, |range| range.start);
        let relative = loan
            .ranges
            .iter()
            .map(|range| range.start..range.end)
            .collect::<Vec<_>>();
        let (moved, _) = destination
            .current_chunks_mut()
            .payload
            .take_selected_ranges(&relative);
        for (offset, key) in moved.iter_live_with_positions() {
            destination.unindex_chunk(pool, key);
            pool.payload.transfer(
                key,
                destination.owner,
                destination.lineage,
                self.owner,
                self.lineage,
            )?;
            self.index_chunk(pool, key, base + offset);
        }
        if !loan.ranges.is_empty() {
            self.current_chunks_mut()
                .payload
                .restore_sparse_overlay(base - loan.source_start, moved);
        }
        Ok(())
    }
}
