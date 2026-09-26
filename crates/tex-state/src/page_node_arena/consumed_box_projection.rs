//! Shallow record projection after one semantic unbox consumption.

use super::*;
use crate::node_record::{write_original_box_body, write_positive_box_body};
use crate::node_region::{NodeAnnexLane, NodeRegionId, PageBoxSegment};

#[derive(Clone, Copy)]
struct ProjectionOptions {
    preserve_consumed_boxes: bool,
    remove_margin_kerns: bool,
}

impl<'a> PageMaterialArena<'a> {
    /// Consumes the unboxed wrapper's child authority for one direct-record
    /// projection. Surviving nested box bodies stay at their current page
    /// coordinates; only their wrapper descriptors are rebound.
    pub fn project_consumed_box_children(
        &mut self,
        source: ConsumedBoxChildren,
        remove_margin_kerns: bool,
    ) -> Result<UniquePageList, ForkArenaError> {
        let span = self.admit_span(source.list)?;
        let mut builder = PageMaterialActiveListBuilder::vacant();
        self.open_active_list(&mut builder)?;
        let projected = self.append_reencoded_span_range(
            &mut builder,
            span,
            0..span.len(),
            true,
            true,
            remove_margin_kerns,
        );
        if let Err(error) = projected {
            self.rollback_active_list(&mut builder)?;
            return Err(error);
        }
        self.finalize_unique_active_list(&mut builder)
    }

    pub(super) fn append_reencoded_span_range(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        span: PageListSpan,
        selected: Range<usize>,
        whole_source: bool,
        preserve_consumed_boxes: bool,
        remove_margin_kerns: bool,
    ) -> Result<(), ForkArenaError> {
        if selected.start > selected.end || selected.end > span.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        self.region
            .pub_arena
            .begin_reencoded_active_list_copy(span.len(), selected.len());
        let mut selected_identity = builder
            .identity
            .as_ref()
            .map(|_| SemanticSequenceIdentity::empty());
        let mut copied = 0_usize;
        let options = ProjectionOptions {
            preserve_consumed_boxes,
            remove_margin_kerns,
        };
        if let Some(tail) = self.span_tail_chunk(span)? {
            // The source edge points backward. Stop before the selected
            // window and reverse only its cursors into semantic order. Inline
            // scratch covers short lists; longer lists grow by chunk count,
            // not by the number or size of payload records.
            let mut selected_chunks = smallvec::SmallVec::<[PageListChunkCursor; 8]>::new();
            let mut cursor = Some(tail);
            while let Some(chunk) = cursor {
                let chunk_end = chunk.inner.logical_start() + chunk.inner.len();
                if chunk_end <= selected.start {
                    break;
                }
                cursor = self.span_previous_chunk(&chunk)?;
                if chunk.inner.logical_start() < selected.end {
                    selected_chunks.push(chunk);
                }
            }
            while let Some(chunk) = selected_chunks.pop() {
                self.append_reencoded_one_chunk(
                    builder,
                    chunk,
                    &selected,
                    &mut selected_identity,
                    &mut copied,
                    options,
                )?;
            }
        }
        if !remove_margin_kerns && copied != selected.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        if selected_identity.is_some() {
            builder.identity_work.hashed_values = builder
                .identity_work
                .hashed_values
                .saturating_add(copied as u64);
            builder.identity_work.combined_summaries = builder
                .identity_work
                .combined_summaries
                .saturating_add(u64::from(whole_source));
        }
        if let (Some(identity), Some(selected_identity)) =
            (&mut builder.identity, selected_identity)
        {
            *identity = identity.concat(selected_identity);
        }
        Ok(())
    }

    fn append_reencoded_one_chunk(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        cursor: PageListChunkCursor,
        selected: &Range<usize>,
        selected_identity: &mut Option<SemanticSequenceIdentity>,
        copied: &mut usize,
        options: ProjectionOptions,
    ) -> Result<(), ForkArenaError> {
        let chunk_start = cursor.inner.logical_start();
        let chunk_end = chunk_start + cursor.inner.len();
        let start = selected.start.max(chunk_start);
        let end = selected.end.min(chunk_end);
        if start >= end {
            return Ok(());
        }
        let (dependency_floor, source_paired_dependency_floor) = self
            .region
            .pub_arena
            .admitted_chunk_dependency_floors(&self.pool.chunks, &cursor.inner)?;
        let identity_enabled = selected_identity.is_some();
        let mut local_start = start - chunk_start;
        let local_end = end - chunk_start;
        while local_start < local_end {
            if options.remove_margin_kerns {
                while local_start < local_end {
                    let (_, record) = self.region.pub_arena.admitted_chunk_value_at(
                        &self.pool.chunks,
                        &cursor.inner,
                        local_start,
                    );
                    if !is_unbox_margin_kern(record) {
                        break;
                    }
                    local_start += 1;
                }
                if local_start == local_end {
                    break;
                }
            }
            let run_end = if options.remove_margin_kerns {
                let mut run_end = local_start;
                while run_end < local_end {
                    let (_, record) = self.region.pub_arena.admitted_chunk_value_at(
                        &self.pool.chunks,
                        &cursor.inner,
                        run_end,
                    );
                    if is_unbox_margin_kern(record) {
                        break;
                    }
                    run_end += 1;
                }
                run_end
            } else {
                local_end
            };
            let source_position = cursor.owner_position();
            let region = self.region.id();
            let written = self.region.pub_arena.transform_admitted_active_list_run(
                &mut self.pool.chunks,
                &mut builder.inner,
                &cursor.inner,
                local_start..run_end,
                identity_enabled,
                |_, record, destination, destination_position| {
                    let record = *record;
                    let annex =
                        NodeAnnexView::new(&self.pool.annex_chunks, &self.region.annex_arena);
                    let item_identity =
                        identity_enabled.then(|| semantic_record_identity(&record, annex));
                    if let (Some(identity), Some(item_identity)) =
                        (selected_identity.as_mut(), item_identity)
                    {
                        identity.push_back(item_identity);
                    }
                    let metadata = options
                        .preserve_consumed_boxes
                        .then(|| {
                            box_migration_metadata_at_record(record, annex, region, source_position)
                        })
                        .flatten();
                    let (record, annex_dependency_floor) = if let Some(metadata) = metadata {
                        reencode_consumed_box_record(
                            record,
                            metadata,
                            destination_position,
                            &mut self.pool.annex_chunks,
                            &mut self.region.annex_arena,
                            region,
                        )?
                    } else {
                        record
                            .reencode_same_region(
                                &mut self.pool.annex_chunks,
                                &mut self.region.annex_arena,
                                Some,
                            )
                            .ok_or(ForkArenaError::InvalidRange)?
                    };
                    *destination = Some(record);
                    Ok(crate::fork_arena::ConstructedRunValue {
                        item_identity,
                        dependency_floor,
                        paired_dependency_floor: [
                            annex_dependency_floor,
                            source_paired_dependency_floor,
                        ]
                        .into_iter()
                        .flatten()
                        .min(),
                    })
                },
            )?;
            local_start += written;
            *copied += written;
        }
        Ok(())
    }
}

/// Republishes one surviving box wrapper without moving or copying its child
/// closure. Its old wrapper and sidecars remain page-owned exclusions.
pub(super) fn reencode_consumed_box_record(
    record: PageMaterialNode,
    mut metadata: PageBoxMigrationMetadata,
    destination_node_position: usize,
    annex_pool: &mut crate::fork_arena::ChunkPool<u32>,
    annex_arena: &mut crate::fork_arena::ForkArena<u32, NodeAnnexLane>,
    region: NodeRegionId,
) -> Result<(PageMaterialNode, Option<usize>), ForkArenaError> {
    if metadata.segment.region() != region || metadata.selected_body_ranges().is_none() {
        return Err(ForkArenaError::InvalidRange);
    }
    let body = record
        .with_fixed_copy_body(NodeAnnexView::new(annex_pool, annex_arena), |body, _| {
            body.to_vec()
        })
        .ok_or(ForkArenaError::InvalidRange)?;
    let old_node_wrapper = metadata
        .segment
        .node_range()
        .end
        .checked_sub(1)
        .ok_or(ForkArenaError::InvalidRange)?;
    let old_annex_wrapper = metadata
        .segment
        .annex_range()
        .end
        .checked_sub(1)
        .ok_or(ForkArenaError::InvalidRange)?;
    if destination_node_position < old_node_wrapper {
        return Err(ForkArenaError::InvalidRange);
    }

    // The sidecar starts at a known annex slot after the current tail is
    // sealed. Its own (possibly multi-chunk) range is added by the decoder.
    annex_arena.seal_boundary(annex_pool)?;
    let sidecar_start = annex_arena.payload_position_end();
    if sidecar_start < old_annex_wrapper {
        return Err(ForkArenaError::InvalidRange);
    }
    enum PublishedDescriptor {
        Original(PageBoxMigrationKey),
        Positive(PageBoxPositiveKey),
    }
    let descriptor = if let Some(positive) = metadata.positive.take() {
        if !valid_positive_ranges(&positive.nodes, destination_node_position)
            || !valid_positive_ranges(&positive.annex, sidecar_start)
            || !valid_positive_cuts(
                &positive.node_cuts,
                &positive.nodes,
                destination_node_position,
            )
            || !valid_positive_cuts(&positive.annex_cuts, &positive.annex, sidecar_start)
        {
            return Err(ForkArenaError::InvalidRange);
        }
        PublishedDescriptor::Positive(
            NodeAnnexWriter::new(annex_pool, annex_arena).publish_box_positive_ranges(
                &positive.nodes,
                &positive.annex,
                &positive.node_cuts,
                &positive.annex_cuts,
            ),
        )
    } else {
        let mut exclusions = metadata.exclusions;
        let mut gap_node_start = old_node_wrapper;
        let mut gap_annex_start = old_annex_wrapper;
        while exclusions.last().is_some_and(|last| {
            last.node_range().end == gap_node_start && last.annex_range().end == gap_annex_start
        }) {
            let last = exclusions.pop().expect("checked final exclusion");
            gap_node_start = last.node_range().start;
            gap_annex_start = last.annex_range().start;
        }
        if gap_node_start != destination_node_position || gap_annex_start != sidecar_start {
            exclusions.push(segment_from_bounds(
                region,
                gap_node_start,
                destination_node_position,
                gap_annex_start,
                sidecar_start,
            )?);
        }
        if !crate::node_record::valid_box_exclusions(region.words(), &exclusions) {
            return Err(ForkArenaError::InvalidRange);
        }
        PublishedDescriptor::Original(
            NodeAnnexWriter::new(annex_pool, annex_arena)
                .publish_box_migration_segments(&exclusions),
        )
    };
    annex_arena.seal_boundary(annex_pool)?;

    let mut flat = vec![0_u32; body.len() + 1];
    flat[1..].copy_from_slice(&body);
    flat[29..].fill(0);
    let mut writer = NodeAnnexWriter::new(annex_pool, annex_arena);
    let mut keys = writer.append_fixed_flat(
        &mut flat,
        &[u16::try_from(body.len() + 1).map_err(|_| ForkArenaError::CapacityOverflow)?],
        |_, wrapper_annex_position, body| match descriptor {
            PublishedDescriptor::Positive(sidecar) => write_positive_box_body(
                body,
                region,
                sidecar,
                destination_node_position,
                wrapper_annex_position,
            )
            .ok_or(ForkArenaError::InvalidRange),
            PublishedDescriptor::Original(sidecar) => {
                let segment = segment_from_bounds(
                    region,
                    metadata.segment.node_range().start,
                    destination_node_position
                        .checked_add(1)
                        .ok_or(ForkArenaError::CapacityOverflow)?,
                    metadata.segment.annex_range().start,
                    wrapper_annex_position
                        .checked_add(1)
                        .ok_or(ForkArenaError::CapacityOverflow)?,
                )?;
                write_original_box_body(body, segment, Some(sidecar))
                    .ok_or(ForkArenaError::InvalidRange)
            }
        },
    )?;
    let key = keys.pop().ok_or(ForkArenaError::InvalidRange)?;
    let relocated = record
        .with_relocated_fixed_key(key)
        .ok_or(ForkArenaError::InvalidRange)?;
    Ok((relocated, Some(sidecar_start)))
}

fn segment_from_bounds(
    region: NodeRegionId,
    node_start: usize,
    node_end: usize,
    annex_start: usize,
    annex_end: usize,
) -> Result<PageBoxSegment, ForkArenaError> {
    let bounds = [
        u32::try_from(node_start).map_err(|_| ForkArenaError::CapacityOverflow)?,
        u32::try_from(node_end).map_err(|_| ForkArenaError::CapacityOverflow)?,
        u32::try_from(annex_start).map_err(|_| ForkArenaError::CapacityOverflow)?,
        u32::try_from(annex_end).map_err(|_| ForkArenaError::CapacityOverflow)?,
    ];
    PageBoxSegment::from_exclusion_bounds(region, bounds).ok_or(ForkArenaError::InvalidRange)
}
