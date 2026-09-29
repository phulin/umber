//! Fused ownership moves for closed payload suffixes.
//!
//! Moving a closed suffix between arenas used to run one metadata pass per
//! concern: suffix and closure preflight, paired-floor preflight, sealing
//! validation, detachment, unindexing, owner transfer, reindexing, and one
//! pass for each floor rebase. A suffix move is instead one mutation-free
//! proof over the chunk metadata, followed by one pass that rewrites each
//! chunk's owner, lineage position, and floors in a single metadata write.

use super::*;

/// How one moved suffix rewrites its chunk dependency floors.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SuffixFloorRebase {
    /// Rebase node floors from the source suffix start onto the destination
    /// start. Otherwise node floors are carried unchanged.
    pub(crate) nodes: bool,
    /// Paired-lane floors move from `source` to `destination` coordinates.
    pub(crate) paired: Option<PairedFloorRebase>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PairedFloorRebase {
    pub(crate) source: usize,
    pub(crate) destination: usize,
}

impl<T> ChunkStorage<T> {
    /// Rebrands one exclusively owned sealed chunk to `destination` at
    /// `position` and rewrites its floors. The caller has proved ownership,
    /// exclusivity, sealing, and floor bounds for the whole suffix.
    #[allow(clippy::too_many_arguments)]
    fn move_proved_chunk(
        &mut self,
        key: LogicalChunkId,
        source: (u32, u32),
        destination: (u32, u32),
        position: usize,
        node_floor: Option<(usize, usize)>,
        paired: Option<PairedFloorRebase>,
    ) {
        let meta = &mut self.chunks[key.ordinal as usize];
        debug_assert!(meta.live && meta.generation == key.incarnation && meta.sealed);
        debug_assert_eq!(meta.arena, source.0);
        meta.arena = destination.0;
        let entry = meta
            .lineages
            .iter_mut()
            .find(|entry| entry.id == source.1)
            .expect("suffix move proved the source lineage");
        entry.id = destination.1;
        entry.position = position;
        if let Some((source_start, destination_start)) = node_floor
            && meta.dependency_floor != usize::MAX
        {
            meta.dependency_floor = destination_start + (meta.dependency_floor - source_start);
        }
        if let Some(paired) = paired
            && meta.paired_dependency_floor != usize::MAX
        {
            meta.paired_dependency_floor =
                paired.destination + (meta.paired_dependency_floor - paired.source);
        }
    }
}

impl<T, Lane> ForkArena<T, Lane> {
    /// Mutation-free proof that the suffix from `payload_start` is closed,
    /// exclusively owned, and movable into `destination`.
    ///
    /// Every declared root must lie wholly in the suffix, and every chunk's
    /// complete dependency floor must lie at or after `payload_start`. With
    /// `paired_start`, every paired floor must lie at or after it. Only the
    /// live tail may remain unsealed; the move seals it.
    pub(crate) fn preflight_suffix_move<Destination>(
        &self,
        pool: &ChunkPool<T>,
        destination: &ForkArena<T, Destination>,
        payload_start: usize,
        roots: &[ArenaListId<Lane>],
        paired_start: Option<usize>,
    ) -> Result<(), ForkArenaError>
    where
        T: RegionValue<Lane>,
    {
        self.validate_pool(pool)?;
        destination.validate_pool(pool)?;
        if self.active_builder || self.pending_batch.is_some() {
            return Err(ForkArenaError::InvalidRegion);
        }
        if self.owner == destination.owner || destination.active_builder {
            return Err(ForkArenaError::InvalidRegion);
        }
        if destination.pending_batch.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let prefix_len = match &self.ownership {
            ForkOwnership::Accepted(_) => self.base_payload_chunks as usize,
            ForkOwnership::Forked { prefix, .. } => {
                self.base_payload_chunks as usize + prefix.payload.len()
            }
        };
        let end = self.live_payload_len();
        if payload_start < prefix_len || payload_start > end {
            return Err(ForkArenaError::InvalidRegion);
        }
        u32::try_from(end).map_err(|_| ForkArenaError::CapacityOverflow)?;
        self.validate_live_chunks(pool)?;
        destination.can_seal_boundary(pool)?;
        destination
            .live_payload_len()
            .checked_add(end - payload_start)
            .filter(|end| *end <= u32::MAX as usize)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        for root in roots {
            self.validate_list_in_suffix(pool, *root, payload_start)?;
        }
        if T::HAS_INLINE_REGION_LISTS {
            return Err(ForkArenaError::InvalidRegion);
        }
        let tail = self.live_tail_position();
        for (position, key) in self.live_positions_from(payload_start) {
            let meta = pool.payload.validate(key, self.owner)?;
            let owners = meta.lineages.iter().filter(|entry| entry.id != 0).count();
            if owners != 1 || meta.lineages.iter().all(|entry| entry.id != self.lineage) {
                return Err(ForkArenaError::ChunkShared);
            }
            if !meta.sealed && Some(position) != tail {
                return Err(ForkArenaError::UnsealedBoundary);
            }
            if !meta.dependency_metadata_complete
                || meta.dependency_floor < payload_start
                || paired_start.is_some_and(|start| meta.paired_dependency_floor < start)
            {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        Ok(())
    }

    /// Seals and moves the suffix proved by [`Self::preflight_suffix_move`]
    /// into `destination`, returning the destination start position.
    ///
    /// The caller must have run that preflight with the same arguments under
    /// the same exclusive borrows; this pass performs no fallible checks.
    pub(crate) fn move_proved_suffix_into<Destination>(
        &mut self,
        pool: &mut ChunkPool<T>,
        destination: &mut ForkArena<T, Destination>,
        payload_start: usize,
        floors: SuffixFloorRebase,
    ) -> usize {
        self.bind_pool(pool)
            .expect("suffix move source pool was preflighted");
        destination
            .bind_pool(pool)
            .expect("suffix move destination pool was preflighted");
        self.seal_boundary(pool)
            .expect("suffix move source boundary was preflighted");
        destination
            .seal_boundary(pool)
            .expect("suffix move destination boundary was preflighted");
        let payload = self
            .detach_suffix(payload_start)
            .expect("suffix move range was preflighted");
        let destination_start = destination.live_payload_len();
        let node_floor = floors.nodes.then_some((payload_start, destination_start));
        let storage = &mut pool.payload;
        for (offset, key) in payload.iter_live_with_positions() {
            storage.move_proved_chunk(
                key,
                (self.owner, self.lineage),
                (destination.owner, destination.lineage),
                destination_start + offset,
                node_floor,
                floors.paired,
            );
        }
        storage.admission_epoch = storage.admission_epoch.saturating_add(1);
        let promoted = payload.len() as u64;
        destination.current_chunks_mut().payload.append(payload);
        self.counters.chunks_promoted = self.counters.chunks_promoted.saturating_add(promoted);
        destination.counters.chunks_promoted = destination
            .counters
            .chunks_promoted
            .saturating_add(promoted);
        destination_start
    }
}
