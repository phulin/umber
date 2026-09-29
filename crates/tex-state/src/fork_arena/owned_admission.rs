//! Fused admission of one owned logical chunk.
//!
//! Root admission used to resolve a chunk's logical row once per check: the
//! compact-position round trip, the arena-position lookup, owner validation,
//! and physical block admission each repeated the row and metadata reads.
//! This module performs the same checks with one read of each table.

use super::*;

/// One owned logical chunk admitted for reads under a shared pool borrow.
#[derive(Clone, Copy)]
pub(super) struct AdmittedOwnedChunk {
    /// Owner-relative arena position in the admitting lineage.
    pub(super) position: usize,
    /// Initialized logical length.
    pub(super) used: u32,
    pub(super) block: AdmittedDenseBlock,
}

impl<T> ChunkStorage<T> {
    /// Admits `key` as a live chunk owned by `arena` and visible in
    /// `lineage`.
    ///
    /// This accepts exactly what a compact-position round trip, an
    /// arena-position lookup, owner validation, and dense-block admission
    /// accept together: a nonzero incarnation that matches both the logical
    /// row and the live metadata, an owning arena, a lineage position, and a
    /// physical extent that lies inside its block and covers the initialized
    /// length.
    pub(super) fn admit_owned_chunk(
        &self,
        key: LogicalChunkId,
        arena: u32,
        lineage: u32,
    ) -> Option<AdmittedOwnedChunk> {
        #[cfg(test)]
        {
            self.arena_position_reads
                .set(self.arena_position_reads.get().saturating_add(1));
            self.validation_reads
                .set(self.validation_reads.get().saturating_add(1));
        }
        if key.incarnation == 0 {
            return None;
        }
        let meta = self.chunks.get(key.ordinal as usize)?;
        if !meta.live
            || meta.generation != key.incarnation
            || meta.arena != arena
            || meta.physical_slot == u32::MAX
        {
            return None;
        }
        let position = meta
            .lineages
            .iter()
            .find(|entry| entry.id == lineage)?
            .position();
        if position == usize::MAX {
            return None;
        }
        let capacity = meta.physical_capacity(self.slots_per_chunk);
        let end = meta.physical_base.checked_add(capacity)?;
        let block = self.blocks.get(meta.physical_slot as usize)?;
        if block.incarnation != meta.physical_incarnation
            || end as usize > block.payload().len()
            || meta.used > capacity
        {
            return None;
        }
        Some(AdmittedOwnedChunk {
            position,
            used: meta.used,
            block: AdmittedDenseBlock {
                page: meta.physical_slot,
                base: meta.physical_base,
            },
        })
    }

    /// Follows one predecessor link of a chunk inside an admitted view.
    ///
    /// Root admission checked ownership and the shared pool borrow excludes
    /// lifecycle change, so the walk trusts the link topology and resolves no
    /// physical block.
    pub(super) fn admitted_previous_link(
        &self,
        key: LogicalChunkId,
    ) -> Option<(LogicalChunkId, u32)> {
        let meta = self.chunks.get(key.ordinal as usize)?;
        debug_assert!(meta.live && meta.generation == key.incarnation);
        meta.previous_in_list()
    }

    /// Resolves the arena position and physical block of a chunk reached
    /// through an admitted view's predecessor links.
    pub(super) fn admitted_chunk_coordinate(
        &self,
        key: LogicalChunkId,
        lineage: u32,
    ) -> Option<(usize, AdmittedDenseBlock)> {
        let meta = self.chunks.get(key.ordinal as usize)?;
        debug_assert!(meta.live && meta.generation == key.incarnation);
        let position = meta
            .lineages
            .iter()
            .find(|entry| entry.id == lineage)?
            .position();
        if position == usize::MAX {
            return None;
        }
        Some((position, self.admit_dense_block(key)?))
    }
}
