//! Move-only partition of a semantic page list after its owning mode removes it.

use core::ops::Range;

use super::{PageListId, PageListSpan, PageMaterialArena};
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
    chunks: Vec<ConsumedSourceChunk>,
    indexed: bool,
}

struct ConsumedSourceChunk {
    records: Range<usize>,
    position: usize,
    local: Range<usize>,
    whole: bool,
}

/// Direct-list chunk coordinates of a completed generated box's child root.
/// This is observation only; the consumed source token and authenticated
/// wrapper descriptor grant authority when the wrapper is later removed.
pub struct PageDirectChunkSelection {
    pub full_node_chunks: Vec<Range<usize>>,
    pub cut_chunks: Vec<(usize, Range<usize>)>,
}

/// One disjoint direct-record window from a consumed semantic source.
///
/// The window remains move-only so a later projection cannot claim the same
/// source records twice. Nested children need their own ownership proof.
pub struct ConsumedPageWindow {
    source: PageListId,
    selected: Range<usize>,
    plan: ConsumedWindowChunkPlan,
}

/// Exact direct-record geometry of one consumed source window. A full chunk
/// may be loaned only after nested dependencies and diagnostic aliases are
/// separately accounted for. Cut ranges are logical record offsets in the
/// source list and require a bounded shallow projection at consumption.
pub struct ConsumedWindowChunkPlan {
    pub full_node_chunks: Vec<Range<usize>>,
    pub cut_records: Vec<Range<usize>>,
}

impl ConsumedPageSource {
    pub const fn source(&self) -> PageListId {
        self.source
    }

    /// Partitions in source order. Skipped discardable records stay page-owned.
    pub fn partition_window(&mut self, selected: Range<usize>) -> Option<ConsumedPageWindow> {
        if !self.indexed
            || selected.start < self.next_unclaimed
            || selected.start > selected.end
            || selected.end > self.source.len()
        {
            return None;
        }
        let mut plan = ConsumedWindowChunkPlan {
            full_node_chunks: Vec::new(),
            cut_records: Vec::new(),
        };
        let first = self
            .chunks
            .partition_point(|chunk| chunk.records.end <= selected.start);
        for chunk in self.chunks[first..]
            .iter()
            .take_while(|chunk| chunk.records.start < selected.end)
        {
            let records =
                selected.start.max(chunk.records.start)..selected.end.min(chunk.records.end);
            if chunk.whole && records == chunk.records {
                if let Some(last) = plan.full_node_chunks.last_mut()
                    && last.end == chunk.position
                {
                    last.end += 1;
                    continue;
                }
                plan.full_node_chunks
                    .push(chunk.position..chunk.position + 1);
            } else {
                plan.cut_records.push(records);
            }
        }
        self.next_unclaimed = selected.end;
        Some(ConsumedPageWindow {
            source: self.source,
            selected,
            plan,
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

    pub const fn plan(&self) -> &ConsumedWindowChunkPlan {
        &self.plan
    }
}

impl PageMaterialArena<'_> {
    fn direct_chunk_index(
        &self,
        source: PageListId,
    ) -> Result<Vec<ConsumedSourceChunk>, ForkArenaError> {
        let span = self.admit_span(source)?;
        let mut chunks = Vec::new();
        let mut cursor = self.span_tail_chunk(span)?;
        while let Some(chunk) = cursor {
            let whole = self
                .region
                .pub_arena
                .admitted_consumed_chunk_is_whole(&self.pool.chunks, &chunk.inner)?;
            chunks.push(ConsumedSourceChunk {
                records: chunk.logical_start()..chunk.logical_start() + chunk.len(),
                position: chunk.owner_position(),
                local: self
                    .region
                    .pub_arena
                    .admitted_consumed_chunk_local_range(&self.pool.chunks, &chunk.inner)?,
                whole,
            });
            cursor = self.span_previous_chunk(&chunk)?;
        }
        chunks.reverse();
        Ok(chunks)
    }

    /// Captures only the direct child-chain geometry. The caller invokes this
    /// at box publication after materialization has determined the final
    /// child root; no child list or annex closure is recursively inspected.
    pub fn direct_root_chunk_selection(
        &self,
        root: PageListId,
    ) -> Result<PageDirectChunkSelection, ForkArenaError> {
        let mut selection = PageDirectChunkSelection {
            full_node_chunks: Vec::new(),
            cut_chunks: Vec::new(),
        };
        for chunk in self.direct_chunk_index(root)? {
            if chunk.whole {
                if let Some(last) = selection.full_node_chunks.last_mut()
                    && last.end == chunk.position
                {
                    last.end += 1;
                    continue;
                }
                selection
                    .full_node_chunks
                    .push(chunk.position..chunk.position + 1);
            } else {
                selection.cut_chunks.push((chunk.position, chunk.local));
            }
        }
        Ok(selection)
    }

    /// Removes the sole semantic source from the actual mode-list slot.
    /// Geometry stays lazy because math finishing, glue normalization, or
    /// hyphenation may replace the source before line materialization.
    pub(crate) fn take_generated_mode_source(
        &self,
        slot: &mut PageListSpan,
    ) -> Result<ConsumedPageSource, ForkArenaError> {
        let source = slot.list();
        self.admit_span(source)?;
        let removed = core::mem::take(slot);
        debug_assert_eq!(removed.list(), source);
        Ok(ConsumedPageSource {
            source,
            next_unclaimed: 0,
            chunks: Vec::new(),
            indexed: false,
        })
    }

    /// Builds one transient direct-chunk index after the final semantic tape
    /// is known to retain this consumed root. It is discarded with the source
    /// token after paragraph or alignment materialization.
    pub(crate) fn index_consumed_source(
        &self,
        source: &mut ConsumedPageSource,
    ) -> Result<(), ForkArenaError> {
        if source.indexed {
            return Err(ForkArenaError::InvalidRegion);
        }
        source.chunks = self.direct_chunk_index(source.source)?;
        source.indexed = true;
        Ok(())
    }

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
