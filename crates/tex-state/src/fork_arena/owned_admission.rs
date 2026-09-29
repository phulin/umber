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
        let ordinal = key.ordinal as usize;
        let row = self.logical_rows.get(ordinal)?;
        if row.incarnation != key.incarnation || row.physical_slot == u32::MAX {
            return None;
        }
        let meta = self.chunks.get(ordinal)?;
        if !meta.live || meta.generation != key.incarnation || meta.arena != arena {
            return None;
        }
        let position = meta
            .lineages
            .iter()
            .find(|entry| entry.id == lineage)?
            .position;
        if position == usize::MAX {
            return None;
        }
        let end = row.physical_base.checked_add(row.physical_capacity)?;
        let block = self.blocks.get(row.physical_slot as usize)?;
        if block.incarnation != row.physical_incarnation
            || end as usize > block.payload().len()
            || meta.used > row.physical_capacity
        {
            return None;
        }
        Some(AdmittedOwnedChunk {
            position,
            used: meta.used,
            block: AdmittedDenseBlock {
                page: row.physical_slot,
                base: row.physical_base,
            },
        })
    }
}
