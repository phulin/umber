//! Short-borrow fixed-body reads from an immutable source owner.

use super::*;

#[derive(Clone, Copy)]
struct CachedPackedChunk {
    pool_space: u32,
    key: LogicalChunkId,
    block: AdmittedDenseBlock,
    used: u32,
    epoch: u64,
}

/// One authenticated source chunk at a time for a single copy operation.
///
/// The source arena borrow excludes retirement and append while this reader
/// exists. Destination publication can still grow or remap the shared pool,
/// so the cache owns only scalar coordinates and reborrows payload on demand.
pub(crate) struct FixedPackedChunkReader<'a, T, Lane> {
    arena: &'a ForkArena<T, Lane>,
    cached: Option<CachedPackedChunk>,
}

impl<'a, T, Lane> FixedPackedChunkReader<'a, T, Lane> {
    pub(crate) const fn new(arena: &'a ForkArena<T, Lane>) -> Self {
        Self {
            arena,
            cached: None,
        }
    }

    /// Lends one exact body in a single logical packed chunk. The caller must
    /// separately authenticate the publication serial stored in its first
    /// word; every other key bound is checked here on every read.
    pub(crate) fn inspect<'p, R>(
        &mut self,
        pool: &'p ChunkPool<T>,
        list: ArenaListId<Lane>,
        expected_words: usize,
        inspect: impl FnOnce(&'p [T]) -> Option<R>,
    ) -> Option<R> {
        if list.is_empty()
            || list.space != pool.payload.logical_space()
            || list.head.raw != list.tail.raw
            || list.len() != expected_words
        {
            return None;
        }
        let end = list
            .head
            .offset
            .checked_add(u32::try_from(expected_words).ok()?)?;
        if end != list.tail.offset {
            return None;
        }
        let epoch = pool.payload.admission_epoch;
        let cached = self.cached.filter(|cached| {
            cached.pool_space == pool.payload.logical_space()
                && cached.key == list.head.raw
                && cached.epoch == epoch
                && epoch != u64::MAX
        });
        let admitted = if let Some(cached) = cached {
            cached
        } else {
            let root = self.arena.admit_owned_root(pool, list).ok()?;
            let used = pool
                .payload
                .validate_lineage(list.head.raw, self.arena.owner, self.arena.lineage)
                .ok()?
                .used;
            let admitted = CachedPackedChunk {
                pool_space: pool.payload.logical_space(),
                key: list.head.raw,
                block: root.head.block,
                used,
                epoch,
            };
            self.cached = Some(admitted);
            admitted
        };
        if end > admitted.used {
            return None;
        }
        let DenseBlockSlice::Packed(words) = pool
            .payload
            .admitted_dense_slice(admitted.block, list.head.offset..end)?
        else {
            return None;
        };
        inspect(words)
    }
}
