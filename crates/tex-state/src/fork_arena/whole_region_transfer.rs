//! Interval-root and suffix validation shared by closure transfers.

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

    pub(super) fn validate_suffix(
        &self,
        start: usize,
        end: usize,
    ) -> Result<Vec<LogicalChunkId>, ForkArenaError> {
        let live_len = self.live_payload_len();
        if start > end || end != live_len {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(self
            .live_positions_from(start)
            .map(|(_, key)| key)
            .collect())
    }

    pub(super) fn detach_suffix(&mut self, start: usize) -> Result<SparseChunks, ForkArenaError> {
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
        // Admission proves the endpoints; the predecessor walk below proves
        // suffix residence and exits at the first earlier chunk. The
        // exhaustive range audit stays reserved for cold ingress, so a
        // negative answer for a long list costs one step, not a full walk.
        self.validate_list(pool, list)?;
        if list.is_empty() {
            return Ok(());
        }
        let bound = self.live_payload_len().saturating_sub(payload_start);
        let mut key = list.tail.raw;
        let mut crossings = 0_usize;
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
            crossings += 1;
            if crossings >= bound {
                return Err(ForkArenaError::InvalidRegion);
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
