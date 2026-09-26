//! Move-only partition of a semantic page list after its owning mode removes it.

use core::ops::Range;

use super::{
    PageBoxCutRange, PageBoxMigrationMetadata, PageListId, PageListSpan, PageMaterialArena,
};
use crate::fork_arena::ForkArenaError;
use crate::node_region::{
    DurableRole, GeneratedInlinePiece, NodeRegion, PageInteriorTransferLoan, RegionRoot,
    rollback_page_interior_closure, transfer_page_generated_inline_selected,
};

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
    inline_only: bool,
}

/// Direct-list chunk coordinates of a completed generated box's child root.
/// This is observation only; the consumed source token and authenticated
/// wrapper descriptor grant authority when the wrapper is later removed.
pub struct PageDirectChunkSelection {
    pub full_node_chunks: Vec<Range<usize>>,
    pub cut_chunks: Vec<PageBoxCutRange>,
    pub inline_only: bool,
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

/// One completed plain post-line body built from an exact consumed source
/// window and a fresh unique right-skip suffix. The final hpack child must
/// match this assembled root before a positive descriptor may be published.
pub struct GeneratedLineBody {
    window: ConsumedPageWindow,
    retained: PageListId,
    assembled: PageListId,
}

/// One-shot authority to bind an authenticated body selection to the exact
/// generated child root that supplied its construction receipt.
pub struct PublishedGeneratedBoxBody {
    key: super::PageBoxPositiveKey,
    final_child: PageListId,
}

impl GeneratedLineBody {
    /// A later migration or packing projection changes the child root and
    /// invalidates this exact direct-chain receipt without invalidating the
    /// still-live page list.
    pub fn matches_final_child(&self, child: PageListId) -> bool {
        self.assembled == child
    }
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
    fn replace_with_constructed_segments(self, source: PageListId) -> Option<Self> {
        if self.indexed || self.next_unclaimed != 0 {
            return None;
        }
        Some(Self {
            source,
            next_unclaimed: 0,
            chunks: Vec::new(),
            indexed: false,
        })
    }

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
    pub(crate) fn append_generated_line_body(
        &mut self,
        window: ConsumedPageWindow,
        suffix: super::FreshGeneratedSegment,
    ) -> Result<(PageListId, GeneratedLineBody), ForkArenaError> {
        let retained = self.slice_sequence(window.source(), window.selected())?;
        let retained_span = self.admit_span(retained)?;
        let assembled = self
            .append_unique_to_span(retained_span, suffix.unique)?
            .list();
        Ok((
            assembled,
            GeneratedLineBody {
                window,
                retained,
                assembled,
            },
        ))
    }

    pub(crate) fn publish_generated_line_body_descriptor(
        &mut self,
        body: GeneratedLineBody,
        final_child: PageListId,
    ) -> Result<Option<PublishedGeneratedBoxBody>, ForkArenaError> {
        if body.assembled != final_child {
            return Err(ForkArenaError::InvalidRegion);
        }
        let expected = self.slice_sequence(body.window.source(), body.window.selected())?;
        if expected != body.retained {
            return Err(ForkArenaError::InvalidRegion);
        }
        let selected = self.direct_root_chunk_selection(final_child)?;
        if !selected.inline_only {
            return Ok(None);
        }
        self.publish_generated_box_body_ranges(
            &selected.full_node_chunks,
            &[],
            &selected.cut_chunks,
            &[],
        )
        .map(|key| Some(PublishedGeneratedBoxBody { key, final_child }))
    }

    pub fn stamp_published_generated_box_body(
        &mut self,
        root: PageListId,
        publication: PublishedGeneratedBoxBody,
    ) -> Result<PageBoxMigrationMetadata, ForkArenaError> {
        let child = match self
            .node_cursor(root)?
            .get(0)
            .ok_or(ForkArenaError::InvalidRange)?
        {
            crate::node_view::NodeView::HList(boxed) | crate::node_view::NodeView::VList(boxed) => {
                boxed.children
            }
            _ => return Err(ForkArenaError::InvalidRange),
        };
        if child != publication.final_child {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.stamp_generated_box_body(root, publication.key)
    }

    /// Consumes actual fresh unique output segments and the prior semantic
    /// owner together. No copied coordinate or local span can mint successor
    /// move authority. The constructed direct chain is the next tape source.
    pub(crate) fn finish_generated_source_segments(
        &mut self,
        old: Option<ConsumedPageSource>,
        segments: Vec<super::FreshGeneratedSegment>,
    ) -> Result<(PageListId, Option<ConsumedPageSource>), ForkArenaError> {
        let mut whole = PageListSpan::empty();
        for segment in segments {
            whole = self.append_unique_to_span(whole, segment.unique)?;
        }
        let root = whole.list();
        let successor = old.and_then(|old| old.replace_with_constructed_segments(root));
        Ok((root, successor))
    }

    /// Validates that an authenticated positive descriptor selects exactly
    /// the consumed wrapper's final inline direct chain. This is read-only;
    /// cut projection and paired ownership change happen in a later prepared
    /// transfer, after the caller removes that exact wrapper.
    pub(crate) fn preflight_generated_inline_box(
        &self,
        wrapper: PageListId,
        metadata: &PageBoxMigrationMetadata,
    ) -> Result<PageListId, ForkArenaError> {
        if self.box_migration_metadata(wrapper).as_ref() != Some(metadata) {
            return Err(ForkArenaError::InvalidRegion);
        }
        let positive = metadata
            .positive
            .as_ref()
            .ok_or(ForkArenaError::InvalidRange)?;
        if !positive.annex.is_empty() || !positive.annex_cuts.is_empty() {
            return Err(ForkArenaError::InvalidRegion);
        }
        let child = match self
            .node_cursor(wrapper)?
            .get(0)
            .ok_or(ForkArenaError::InvalidRange)?
        {
            crate::node_view::NodeView::HList(boxed) | crate::node_view::NodeView::VList(boxed) => {
                boxed.children
            }
            _ => return Err(ForkArenaError::InvalidRange),
        };
        let direct = self.direct_root_chunk_selection(child)?;
        if !direct.inline_only
            || direct.full_node_chunks != positive.nodes
            || direct.cut_chunks != positive.node_cuts
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.region
            .pub_arena
            .preflight_consumed_inline_floors(&self.pool.chunks, &positive.nodes)?;
        for cut in &positive.node_cuts {
            self.region.pub_arena.read_consumed_cut_values(
                &self.pool.chunks,
                cut.chunk_position,
                cut.local.clone(),
            )?;
        }
        Ok(child)
    }

    /// The final child chain supplies direct order; the descriptor supplies
    /// authenticated complete chunks and bounded cut records. A single
    /// prepared transfer projects cuts, fills holes with complete chunks,
    /// and journals every changed predecessor and dependency floor.
    pub(crate) fn finish_generated_inline_box(
        &mut self,
        wrapper_root: PageListId,
        metadata: &PageBoxMigrationMetadata,
        reset_shift: bool,
        destination: &mut NodeRegion<DurableRole>,
    ) -> Result<(RegionRoot<DurableRole>, PageInteriorTransferLoan), ForkArenaError> {
        let child = self.preflight_generated_inline_box(wrapper_root, metadata)?;
        let positive = metadata
            .positive
            .as_ref()
            .ok_or(ForkArenaError::InvalidRange)?;
        let mut wrapper = self
            .node_cursor(wrapper_root)?
            .first()
            .ok_or(ForkArenaError::InvalidRange)?
            .to_owned();
        {
            let boxed = match &mut wrapper {
                crate::node::Node::HList(boxed) | crate::node::Node::VList(boxed) => boxed,
                _ => return Err(ForkArenaError::InvalidRange),
            };
            if boxed
                .diagnostic_children
                .is_some_and(|diagnostic| diagnostic != child)
            {
                return Err(ForkArenaError::InvalidRegion);
            }
            if reset_shift {
                boxed.shift = crate::scaled::Scaled::from_raw(0);
            }
        }
        let indexed = self.direct_chunk_index(child, false)?;
        if indexed
            .windows(2)
            .any(|pair| pair[0].position >= pair[1].position)
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let mut pieces = Vec::new();
        let mut index = 0;
        while index < indexed.len() {
            if indexed[index].whole {
                let start = indexed[index].records.start;
                let mut end = indexed[index].records.end;
                index += 1;
                while index < indexed.len() && indexed[index].whole {
                    end = indexed[index].records.end;
                    index += 1;
                }
                pieces.push(GeneratedInlinePiece::Full(
                    self.slice_sequence(child, start..end)?,
                ));
            } else {
                pieces.push(GeneratedInlinePiece::Cut(PageBoxCutRange {
                    chunk_position: indexed[index].position,
                    local: indexed[index].local.clone(),
                }));
                index += 1;
            }
        }
        let (coordinate, loan) = transfer_page_generated_inline_selected(
            self.pool,
            self.region,
            &pieces,
            &positive.nodes,
            *self.semantic_identity_enabled,
            destination,
        )?;
        let boxed = match &mut wrapper {
            crate::node::Node::HList(boxed) | crate::node::Node::VList(boxed) => boxed,
            _ => unreachable!("preflighted generated wrapper"),
        };
        boxed.children = child.with_coordinate(coordinate);
        if boxed.diagnostic_children == Some(child) {
            boxed.diagnostic_children = Some(boxed.children);
        }
        let projected =
            destination.publish_box_wrapper(self.pool, wrapper, wrapper_root.sequence_identity());
        match projected {
            Ok(root) => Ok((root, loan)),
            Err(error) => {
                rollback_page_interior_closure(self.pool, self.region, destination, loan)
                    .expect("failed cut projection returns its empty loan");
                Err(error)
            }
        }
    }

    fn direct_chunk_index(
        &self,
        source: PageListId,
        inspect_inline: bool,
    ) -> Result<Vec<ConsumedSourceChunk>, ForkArenaError> {
        let span = self.admit_span(source)?;
        let mut chunks = Vec::new();
        let mut cursor = self.span_tail_chunk(span)?;
        while let Some(chunk) = cursor {
            let whole = self
                .region
                .pub_arena
                .admitted_consumed_chunk_is_whole(&self.pool.chunks, &chunk.inner)?;
            let inline_only = !inspect_inline
                || (0..chunk.len()).all(|offset| {
                    self.region
                        .pub_arena
                        .admitted_chunk_value_at(&self.pool.chunks, &chunk.inner, offset)
                        .1
                        .is_inline_leaf()
                });
            chunks.push(ConsumedSourceChunk {
                records: chunk.logical_start()..chunk.logical_start() + chunk.len(),
                position: chunk.owner_position(),
                local: self
                    .region
                    .pub_arena
                    .admitted_consumed_chunk_local_range(&self.pool.chunks, &chunk.inner)?,
                whole,
                inline_only,
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
            inline_only: true,
        };
        for chunk in self.direct_chunk_index(root, true)? {
            selection.inline_only &= chunk.inline_only;
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
                selection.cut_chunks.push(PageBoxCutRange {
                    chunk_position: chunk.position,
                    local: chunk.local,
                });
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
        source.chunks = self.direct_chunk_index(source.source, false)?;
        source.indexed = true;
        Ok(())
    }
}
