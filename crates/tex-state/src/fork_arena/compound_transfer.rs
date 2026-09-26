//! Exact disjoint interval loans for isolated box construction segments.

use super::*;

fn selected(ranges: &[Range<usize>], position: usize) -> bool {
    let next = ranges.partition_point(|range| range.end <= position);
    ranges
        .get(next)
        .is_some_and(|range| range.start <= position)
}

impl<T, Lane> ForkArena<T, Lane> {
    fn current_payload_start(&self) -> usize {
        let base = self.base_payload_chunks as usize;
        match &self.ownership {
            ForkOwnership::Accepted(_) => base,
            ForkOwnership::Forked { prefix, .. } => base + prefix.payload.len(),
        }
    }

    fn validate_list_endpoints_in_intervals(
        &self,
        pool: &ChunkPool<T>,
        list: ArenaListId<Lane>,
        ranges: &[Range<usize>],
    ) -> Result<(), ForkArenaError> {
        self.validate_list(pool, list)?;
        if list.is_empty() {
            return Ok(());
        }
        for key in [list.head.raw, list.tail.raw] {
            let position = self
                .resolved_position(pool, key)
                .ok_or(ForkArenaError::InvalidRegion)?;
            if !selected(ranges, position) {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    /// Preflights only the selected chunks and their direct dependencies.
    /// No page-root census or recursive tree walk grants transfer authority:
    /// the caller must have consumed the semantic box owner before invoking
    /// the move. Excluded ranges keep their page ownership and coordinates.
    /// Every selected chunk's direct predecessor is checked once; child lists
    /// need only authenticated endpoints because that predecessor proof keeps
    /// every intermediate chunk inside the selected set.
    pub(crate) fn preflight_interior_intervals<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        ranges: &[Range<usize>],
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.preflight_interior_intervals_inner(pool, destination, ranges, false)
    }

    fn preflight_interior_intervals_inner<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        ranges: &[Range<usize>],
        destination_has_prefix: bool,
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.can_seal_boundary(pool)?;
        destination.can_seal_boundary(pool)?;
        let selected_start = ranges.first().map_or(0, |range| range.start);
        if self.owner == destination.owner
            || self.pending_batch.is_some()
            || !matches!(destination.ownership, ForkOwnership::Accepted(_))
            || if destination_has_prefix {
                destination.base_payload_chunks != 0
                    || destination.live_payload_len() != selected_start
            } else {
                destination.live_payload_len() != 0
            }
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let mut previous_end = self.current_payload_start();
        for range in ranges {
            if range.start < previous_end
                || range.start >= range.end
                || range.end > self.live_payload_len()
            {
                return Err(ForkArenaError::InvalidRegion);
            }
            u32::try_from(range.end).map_err(|_| ForkArenaError::CapacityOverflow)?;
            previous_end = range.end;
        }
        for range in ranges {
            for (_, key) in self
                .live_positions_from(range.start)
                .take_while(|(position, _)| *position < range.end)
            {
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
                    if !selected(ranges, predecessor) {
                        return Err(ForkArenaError::InvalidRegion);
                    }
                }
                if T::HAS_INLINE_REGION_LISTS {
                    let used = pool.payload.used(key, self.owner)?;
                    for offset in 0..used {
                        let value = pool
                            .payload
                            .get(key, self.owner, offset)
                            .ok_or(ForkArenaError::InvalidChunk)?;
                        let mut dependency_error = None;
                        value.visit_region_lists(&mut |list| {
                            if dependency_error.is_none() {
                                dependency_error = self
                                    .validate_list_endpoints_in_intervals(pool, list, ranges)
                                    .err();
                            }
                        });
                        if let Some(error) = dependency_error {
                            return Err(error);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Lets the typed paired-region preflight inspect selected direct records
    /// without materializing or recursively traversing their child closures.
    pub(crate) fn visit_interval_values(
        &self,
        pool: &ChunkPool<T>,
        ranges: &[Range<usize>],
        mut visit: impl FnMut(&T) -> Result<(), ForkArenaError>,
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        for range in ranges {
            for (_, key) in self
                .live_positions_from(range.start)
                .take_while(|(position, _)| *position < range.end)
            {
                let used = pool.payload.used(key, self.owner)?;
                for offset in 0..used {
                    visit(
                        pool.payload
                            .get(key, self.owner, offset)
                            .ok_or(ForkArenaError::InvalidChunk)?,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Moves exactly the selected intervals. Vacant logical spans in the
    /// destination retain the excluded chunks' old coordinates without
    /// owning or copying any of their payload.
    pub(crate) fn transfer_interior_intervals<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        ranges: &[Range<usize>],
    ) -> Result<TransferredIntervals<Lane>, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.transfer_interior_intervals_inner(pool, destination, ranges, false)
    }

    /// Appends exact source intervals after a private destination boundary
    /// projection. The destination has already reserved vacant positions up
    /// to the first source interval, so moved chunks keep their original
    /// logical positions and their direct dependency floors remain valid.
    pub(crate) fn transfer_interior_intervals_after_prefix<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        ranges: &[Range<usize>],
    ) -> Result<TransferredIntervals<Lane>, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.transfer_interior_intervals_inner(pool, destination, ranges, true)
    }

    fn transfer_interior_intervals_inner<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        ranges: &[Range<usize>],
        destination_has_prefix: bool,
    ) -> Result<TransferredIntervals<Lane>, ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.preflight_interior_intervals_inner(pool, destination, ranges, destination_has_prefix)?;
        self.bind_pool(pool)?;
        destination.bind_pool(pool)?;
        let base = ranges.first().map_or(0, |range| range.start);
        let end = ranges.last().map_or(0, |range| range.end);
        if !destination_has_prefix {
            destination.base_payload_chunks = base as u32;
        }
        let source_base = self.current_payload_start();
        let relative_ranges = ranges
            .iter()
            .map(|range| range.start - source_base..range.end - source_base)
            .collect::<Vec<_>>();
        let (moved, live_counts) = self
            .current_chunks_mut()
            .payload
            .take_selected_ranges(&relative_ranges);
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
                .expect("compound chunk ownership was preflighted");
            destination.index_chunk(pool, key, base + offset);
        }
        destination.current_chunks_mut().payload.append(moved);
        Ok(TransferredIntervals {
            source: self.owner,
            destination: destination.owner,
            ranges: ranges
                .iter()
                .map(|range| range.start as u32..range.end as u32)
                .collect(),
            live_counts: live_counts.into_iter().map(|count| count as u32).collect(),
            source_current_start: source_base as u32,
            base: base as u32,
            end: end as u32,
            destination_has_prefix,
            _lane: PhantomData,
        })
    }

    /// Adds a single compressed vacant interval after copied cut chunks.
    pub(crate) fn reserve_vacant_prefix_until(
        &mut self,
        pool: &ChunkPool<T>,
        position: usize,
    ) -> Result<(), ForkArenaError> {
        self.can_seal_boundary(pool)?;
        if self.base_payload_chunks != 0
            || !matches!(self.ownership, ForkOwnership::Accepted(_))
            || position < self.live_payload_len()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        u32::try_from(position).map_err(|_| ForkArenaError::CapacityOverflow)?;
        self.current_chunks_mut().payload.extend_vacant_to(position);
        Ok(())
    }

    /// A paired-region owner checks both lanes with this read-only proof
    /// before reversing either lane, so a bad second receipt cannot leave a
    /// half-restored box.
    pub(crate) fn preflight_rollback_interior_intervals<Source>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Source>,
        loan: &TransferredIntervals<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.can_seal_boundary(pool)?;
        destination.can_seal_boundary(pool)?;
        let base = loan.base as usize;
        let end = loan.end as usize;
        if self.owner != loan.source
            || destination.owner != loan.destination
            || !matches!(destination.ownership, ForkOwnership::Accepted(_))
            || self.current_payload_start() != loan.source_current_start as usize
            || destination.base_payload_chunks
                != if loan.destination_has_prefix {
                    0
                } else {
                    loan.base
                }
            || destination.live_payload_len() < end
            || end > self.live_payload_len()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        for (range, expected_live) in loan.ranges.iter().zip(&loan.live_counts) {
            let start = range.start as usize;
            let end = range.end as usize;
            if self
                .live_positions_from(start)
                .next()
                .is_some_and(|(position, _)| position < end)
            {
                return Err(ForkArenaError::InvalidRegion);
            }
            let mut actual_live = 0;
            for (position, key) in destination
                .live_positions_from(start)
                .take_while(|(position, _)| *position < end)
            {
                let meta =
                    pool.payload
                        .validate_lineage(key, destination.owner, destination.lineage)?;
                if !meta.sealed
                    || meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1
                    || pool
                        .payload
                        .arena_position(key, destination.owner, destination.lineage)
                        != Some(position)
                {
                    return Err(ForkArenaError::InvalidRegion);
                }
                actual_live += 1;
            }
            if actual_live != *expected_live as usize {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        let ranges = loan
            .ranges
            .iter()
            .map(|range| range.start as usize..range.end as usize)
            .collect::<Vec<_>>();
        for (position, _) in destination
            .live_positions_from(base)
            .take_while(|(position, _)| *position < end)
        {
            if !selected(&ranges, position) {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        for (position, key) in destination.live_positions_from(end) {
            pool.payload.mapping(key)?;
            pool.payload
                .validate_lineage(key, destination.owner, destination.lineage)?;
            if pool
                .payload
                .arena_position(key, destination.owner, destination.lineage)
                != Some(position)
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    /// Returns every selected chunk to its exact page slot, after dropping
    /// any destination construction suffix added around the loan.
    pub(crate) fn rollback_interior_intervals<Source>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Source>,
        loan: TransferredIntervals<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.preflight_rollback_interior_intervals(pool, destination, &loan)?;
        let base = loan.base as usize;
        let end = loan.end as usize;
        let extra = destination.detach_suffix(end)?;
        for key in extra.into_live_keys() {
            destination.unindex_chunk(pool, key);
            pool.payload
                .release_lineage(key, destination.owner, destination.lineage)?;
        }
        let selected = destination.detach_suffix(base)?;
        let source_base = self.current_payload_start();
        for (offset, key) in selected.iter_live_with_positions() {
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
                .restore_sparse_overlay(base - source_base, selected);
        }
        if !loan.destination_has_prefix {
            destination.base_payload_chunks = 0;
        }
        Ok(())
    }
}
