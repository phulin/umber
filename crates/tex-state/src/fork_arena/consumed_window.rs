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

impl<T, Lane> ForkArena<T, Lane> {
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
        let old_paired_floor = meta.paired_dependency_floor;
        let old_previous = pool.payload.previous_in_list(head, self.owner)?;
        pool.payload
            .set_previous_in_list(head, self.owner, self.lineage, None)?;
        let meta = pool
            .payload
            .validate_exclusive_lineage_mut(head, self.owner, self.lineage)?;
        meta.paired_dependency_floor = intrinsic_paired_floor;
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
        if meta.paired_dependency_floor != loan.detached_floor
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
        meta.paired_dependency_floor = loan.paired_floor;
        Ok(())
    }

    /// Links an already transferred head to its private projected prefix.
    /// The source edge loan remains live for rollback and is not consumed.
    pub(crate) fn bind_consumed_head_to_prefix<SourceLane>(
        &mut self,
        pool: &mut ChunkPool<T>,
        loan: &ConsumedHeadEdgeLoan<SourceLane>,
        prefix: ArenaListId<Lane>,
    ) -> Result<(), ForkArenaError> {
        self.validate_list(pool, prefix)?;
        if prefix.is_empty()
            || pool
                .payload
                .previous_in_list(loan.head, self.owner)?
                .is_some()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let head =
            pool.payload
                .validate_exclusive_lineage_mut(loan.head, self.owner, self.lineage)?;
        if head.paired_dependency_floor != loan.detached_floor {
            return Err(ForkArenaError::InvalidRegion);
        }
        pool.payload.set_previous_in_list(
            loan.head,
            self.owner,
            self.lineage,
            Some((prefix.tail.raw, prefix.tail.offset)),
        )
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
        head.paired_dependency_floor = loan.detached_floor;
        Ok(())
    }
}
