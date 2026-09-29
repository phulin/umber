//! Reversible direct-chain boundary changes for consumed source windows.

use super::*;

/// Restores a consumed window's original head edge after a rejected loan.
pub(crate) struct ConsumedHeadEdgeLoan<Lane> {
    owner: u32,
    lineage: u32,
    head: LogicalChunkId,
    previous: Option<(LogicalChunkId, u32)>,
    paired_floor: usize,
    detached_floor: usize,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

/// Exact old paired floors for complete inline-only chunks. A consumed
/// window can inherit an annex floor from an excluded predecessor even when
/// none of its own direct records uses the annex lane.
pub(crate) struct ConsumedInlineFloorLoan<Lane> {
    owner: u32,
    lineage: u32,
    floors: Vec<(LogicalChunkId, usize, usize)>,
    _lane: PhantomData<fn(Lane) -> Lane>,
}

impl<T, Lane> ForkArena<T, Lane> {
    /// Read-only exclusive/sealed proof used by generated-box eligibility.
    pub(crate) fn preflight_consumed_inline_floors(
        &self,
        pool: &ChunkPool<T>,
        ranges: &[Range<usize>],
    ) -> Result<(), ForkArenaError> {
        self.validate_pool(pool)?;
        let mut previous_end = 0;
        for range in ranges {
            if range.start < previous_end || range.start >= range.end {
                return Err(ForkArenaError::InvalidRange);
            }
            for position in range.clone() {
                let key = self
                    .live_key_at(position)
                    .ok_or(ForkArenaError::InvalidRange)?;
                let meta = pool
                    .payload
                    .validate_lineage(key, self.owner, self.lineage)?;
                if !meta.sealed
                    || !meta.dependency_metadata_complete
                    || meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1
                {
                    return Err(ForkArenaError::InvalidRegion);
                }
            }
            previous_end = range.end;
        }
        Ok(())
    }

    /// Clears inherited paired floors on caller-proved complete inline-only
    /// chunks. The typed node layer first verifies every selected direct
    /// record has no annex dependency; this method records every old scalar
    /// so a rejected operation restores exact source metadata.
    pub(crate) fn detach_consumed_inline_floors(
        &mut self,
        pool: &mut ChunkPool<T>,
        ranges: &[Range<usize>],
    ) -> Result<ConsumedInlineFloorLoan<Lane>, ForkArenaError> {
        self.preflight_consumed_inline_floors(pool, ranges)?;
        let mut floors = Vec::new();
        for range in ranges {
            if range.start >= range.end {
                return Err(ForkArenaError::InvalidRange);
            }
            for position in range.clone() {
                let key = self
                    .live_key_at(position)
                    .ok_or(ForkArenaError::InvalidRange)?;
                let meta =
                    pool.payload
                        .validate_exclusive_lineage_mut(key, self.owner, self.lineage)?;
                if !meta.sealed || !meta.dependency_metadata_complete {
                    return Err(ForkArenaError::InvalidRegion);
                }
                floors.push((key, meta.dependency_floor(), meta.paired_dependency_floor()));
            }
        }
        for &(key, _, _) in &floors {
            let meta =
                pool.payload
                    .validate_exclusive_lineage_mut(key, self.owner, self.lineage)?;
            meta.set_dependency_floor(usize::MAX);
            meta.set_paired_dependency_floor(usize::MAX);
        }
        Ok(ConsumedInlineFloorLoan {
            owner: self.owner,
            lineage: self.lineage,
            floors,
            _lane: PhantomData,
        })
    }

    pub(crate) fn restore_consumed_inline_floors(
        &mut self,
        pool: &mut ChunkPool<T>,
        loan: ConsumedInlineFloorLoan<Lane>,
    ) -> Result<(), ForkArenaError> {
        if loan.owner != self.owner || loan.lineage != self.lineage {
            return Err(ForkArenaError::InvalidRegion);
        }
        for &(key, _, _) in &loan.floors {
            let meta =
                pool.payload
                    .validate_exclusive_lineage_mut(key, self.owner, self.lineage)?;
            if meta.dependency_floor() != usize::MAX || meta.paired_dependency_floor() != usize::MAX
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        for (key, old_dependency, old_paired) in loan.floors {
            let meta =
                pool.payload
                    .validate_exclusive_lineage_mut(key, self.owner, self.lineage)?;
            meta.set_dependency_floor(old_dependency);
            meta.set_paired_dependency_floor(old_paired);
        }
        Ok(())
    }

    /// Checks the destination-owned moved chunks before rollback begins.
    /// Their scalar floors must still be the isolated values recorded by the
    /// prepared inline loan; return to the source then only changes owner.
    pub(crate) fn preflight_consumed_inline_floor_inverse<SourceLane>(
        &self,
        pool: &ChunkPool<T>,
        loan: &ConsumedInlineFloorLoan<SourceLane>,
    ) -> Result<(), ForkArenaError> {
        for &(key, _, _) in &loan.floors {
            let meta = pool
                .payload
                .validate_lineage(key, self.owner, self.lineage)?;
            if meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1
                || meta.dependency_floor() != usize::MAX
                || meta.paired_dependency_floor() != usize::MAX
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    pub(crate) fn preflight_consumed_head_inverse<SourceLane>(
        &self,
        pool: &ChunkPool<T>,
        loan: &ConsumedHeadEdgeLoan<SourceLane>,
        prefix: Option<ArenaListId<Lane>>,
    ) -> Result<(), ForkArenaError> {
        if let Some(prefix) = prefix {
            self.validate_list(pool, prefix)?;
        }
        let meta = pool
            .payload
            .validate_lineage(loan.head, self.owner, self.lineage)?;
        if meta.lineages.iter().filter(|entry| entry.id != 0).count() != 1
            || meta.paired_dependency_floor() != loan.detached_floor
            || pool.payload.previous_in_list(loan.head, self.owner)?
                != prefix.map(|prefix| (prefix.tail.raw, prefix.tail.offset))
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(())
    }

    /// Reads only the selected local records of one authenticated chunk.
    /// The returned fixed-size values are shallow; child lists remain keys
    /// and must be mapped by the typed destination publisher.
    pub(crate) fn read_consumed_cut_values(
        &self,
        pool: &ChunkPool<T>,
        position: usize,
        local: Range<usize>,
    ) -> Result<Vec<T>, ForkArenaError>
    where
        T: Copy,
    {
        self.validate_pool(pool)?;
        let key = self
            .live_key_at(position)
            .ok_or(ForkArenaError::InvalidRange)?;
        let meta = pool
            .payload
            .validate_lineage(key, self.owner, self.lineage)?;
        let used = pool.payload.used(key, self.owner)? as usize;
        if !meta.sealed || local.start >= local.end || local.end > used {
            return Err(ForkArenaError::InvalidRange);
        }
        (local.start..local.end)
            .map(|offset| {
                pool.payload
                    .get(key, self.owner, offset as u32)
                    .copied()
                    .ok_or(ForkArenaError::InvalidChunk)
            })
            .collect()
    }

    /// Physical offsets addressed by this admitted direct-list cursor.
    pub(crate) fn admitted_consumed_chunk_local_range(
        &self,
        pool: &ChunkPool<T>,
        cursor: &AdmittedListChunkCursor<Lane>,
    ) -> Result<Range<usize>, ForkArenaError> {
        if self.live_key_at(cursor.position) != Some(cursor.key) {
            return Err(ForkArenaError::InvalidRange);
        }
        let used = pool.payload.used(cursor.key, self.owner)?;
        if cursor.start > cursor.end || cursor.end > used {
            return Err(ForkArenaError::InvalidRange);
        }
        Ok(cursor.start as usize..cursor.end as usize)
    }

    /// A selected list cursor owns this entire physical logical chunk only
    /// when it starts at offset zero and ends at the initialized chunk end.
    /// A list-level range covering the cursor is insufficient: its head or
    /// tail may still share the chunk with an older sibling list.
    pub(crate) fn admitted_consumed_chunk_is_whole(
        &self,
        pool: &ChunkPool<T>,
        cursor: &AdmittedListChunkCursor<Lane>,
    ) -> Result<bool, ForkArenaError> {
        if self.live_key_at(cursor.position) != Some(cursor.key) {
            return Err(ForkArenaError::InvalidRange);
        }
        Ok(cursor.start == 0 && cursor.end == pool.payload.used(cursor.key, self.owner)?)
    }

    /// Clears the predecessor of a whole consumed head chunk. The caller
    /// supplies its exact intrinsic annex floor, computed from this chunk's
    /// direct records; the old aggregate floor may include an earlier source
    /// chunk and cannot safely be reused after transfer.
    pub(crate) fn detach_consumed_head_edge(
        &mut self,
        pool: &mut ChunkPool<T>,
        window: ArenaListId<Lane>,
        intrinsic_paired_floor: usize,
    ) -> Result<ConsumedHeadEdgeLoan<Lane>, ForkArenaError> {
        self.validate_list(pool, window)?;
        if window.is_empty() || window.head.offset != 0 {
            return Err(ForkArenaError::InvalidRange);
        }
        let head = window.head.raw;
        let meta = pool
            .payload
            .validate_exclusive_lineage_mut(head, self.owner, self.lineage)?;
        if !meta.sealed {
            return Err(ForkArenaError::InvalidRegion);
        }
        let old_paired_floor = meta.paired_dependency_floor();
        let old_previous = pool.payload.previous_in_list(head, self.owner)?;
        pool.payload
            .set_previous_in_list(head, self.owner, self.lineage, None)?;
        let meta = pool
            .payload
            .validate_exclusive_lineage_mut(head, self.owner, self.lineage)?;
        meta.set_paired_dependency_floor(intrinsic_paired_floor);
        Ok(ConsumedHeadEdgeLoan {
            owner: self.owner,
            lineage: self.lineage,
            head,
            previous: old_previous,
            paired_floor: old_paired_floor,
            detached_floor: intrinsic_paired_floor,
            _lane: PhantomData,
        })
    }

    /// Returns the original predecessor and floor after the transferred
    /// window's chunks have been returned to their source owner.
    pub(crate) fn restore_consumed_head_edge(
        &mut self,
        pool: &mut ChunkPool<T>,
        loan: ConsumedHeadEdgeLoan<Lane>,
    ) -> Result<(), ForkArenaError> {
        if loan.owner != self.owner || loan.lineage != self.lineage {
            return Err(ForkArenaError::InvalidRegion);
        }
        let meta =
            pool.payload
                .validate_exclusive_lineage_mut(loan.head, self.owner, self.lineage)?;
        if meta.paired_dependency_floor() != loan.detached_floor
            || pool
                .payload
                .previous_in_list(loan.head, self.owner)?
                .is_some()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        pool.payload
            .set_previous_in_list(loan.head, self.owner, self.lineage, loan.previous)?;
        let meta =
            pool.payload
                .validate_exclusive_lineage_mut(loan.head, self.owner, self.lineage)?;
        meta.set_paired_dependency_floor(loan.paired_floor);
        Ok(())
    }

    /// Reverses the private prefix link before the selected chunks return to
    /// the source owner, restoring the detached head's exact intrinsic floor.
    pub(crate) fn unbind_consumed_head_from_prefix<SourceLane>(
        &mut self,
        pool: &mut ChunkPool<T>,
        loan: &ConsumedHeadEdgeLoan<SourceLane>,
        prefix: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.validate_list(pool, prefix)?;
        if pool.payload.previous_in_list(loan.head, self.owner)?
            != Some((prefix.tail.raw, prefix.tail.offset))
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        pool.payload
            .set_previous_in_list(loan.head, self.owner, self.lineage, None)?;
        let head =
            pool.payload
                .validate_exclusive_lineage_mut(loan.head, self.owner, self.lineage)?;
        head.set_paired_dependency_floor(loan.detached_floor);
        Ok(())
    }
}
