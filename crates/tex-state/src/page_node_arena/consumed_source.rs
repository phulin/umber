//! Move-only partition of a semantic page list after its owning mode removes it.

use core::ops::Range;

use super::{PageListId, PageMaterialArena};
use crate::fork_arena::{ConsumedHeadEdgeLoan, ForkArenaError, PageMaterialLane};
use crate::node_record::NodeAnnexView;

/// The single semantic owner of a consumed paragraph or alignment source.
///
/// This token is created only at a caller's `take_nodes` boundary. It records
/// which source indices have already been assigned to generated outputs; an
/// ordinary [`PageListId`] remains a coordinate and grants no move authority.
pub struct ConsumedPageSource {
    source: PageListId,
    next_unclaimed: usize,
}

/// One disjoint direct-record window from a consumed semantic source.
///
/// The window remains move-only so a later projection cannot claim the same
/// source records twice. Nested children need their own ownership proof.
pub struct ConsumedPageWindow {
    source: PageListId,
    selected: Range<usize>,
}

impl ConsumedPageSource {
    /// The caller must have removed `source` from its sole semantic list owner.
    /// Diagnostic projections and historical owners remain separate inputs.
    pub fn from_removed_root(source: PageListId) -> Self {
        Self {
            source,
            next_unclaimed: 0,
        }
    }

    pub const fn source(&self) -> PageListId {
        self.source
    }

    /// Partitions in source order. Skipped discardable records stay page-owned.
    pub fn partition_window(&mut self, selected: Range<usize>) -> Option<ConsumedPageWindow> {
        if selected.start < self.next_unclaimed
            || selected.start > selected.end
            || selected.end > self.source.len()
        {
            return None;
        }
        self.next_unclaimed = selected.end;
        Some(ConsumedPageWindow {
            source: self.source,
            selected,
        })
    }
}

impl ConsumedPageWindow {
    pub const fn source(&self) -> PageListId {
        self.source
    }

    pub fn selected(&self) -> Range<usize> {
        self.selected.clone()
    }
}

impl PageMaterialArena<'_> {
    /// Detaches the first full chunk of a consumed source window. The paired
    /// floor is recomputed from this chunk's own annex keys, excluding the
    /// predecessor floor inherited from its old source chain.
    pub(crate) fn detach_consumed_source_head(
        &mut self,
        root: PageListId,
    ) -> Result<ConsumedHeadEdgeLoan<PageMaterialLane>, ForkArenaError> {
        let range = self
            .region
            .pub_arena
            .owner_relative_list_block_range(&self.pool.chunks, root.coordinate())?;
        let annex = NodeAnnexView::new(&self.pool.annex_chunks, &self.region.annex_arena);
        let mut intrinsic_floor = usize::MAX;
        self.region.pub_arena.visit_interval_values(
            &self.pool.chunks,
            &[range.start..range.start + 1],
            |record| {
                record
                    .visit_annex_block_ranges(annex, |range| {
                        intrinsic_floor = intrinsic_floor.min(range.start);
                    })
                    .ok_or(ForkArenaError::InvalidRange)
            },
        )?;
        self.region.pub_arena.detach_consumed_head_edge(
            &mut self.pool.chunks,
            root.coordinate(),
            intrinsic_floor,
        )
    }

    pub(crate) fn restore_consumed_source_head(
        &mut self,
        loan: ConsumedHeadEdgeLoan<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .restore_consumed_head_edge(&mut self.pool.chunks, loan)
    }
}
