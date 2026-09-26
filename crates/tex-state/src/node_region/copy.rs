//! Chunk-batched explicit copies between independently owned node regions.

use super::*;
use crate::node_record::{CopiedBoxBodyStamp, NodeAnnexCopyReader, NodeAnnexWriter};
use smallvec::SmallVec;

struct CopiedBoxEnvelope {
    index: usize,
    child_node_start: u32,
    child_node_end: u32,
    child_annex_start: u32,
    child_annex_end: u32,
}

/// Publishes one bounded group of fixed bodies while the node reservation
/// owns the disjoint node arena. The callback fills copied-box metadata before
/// the selected annex logical chunk can become sealed.
struct FixedBatchPublisher<'a> {
    pool: &'a mut ChunkPool<u32>,
    arena: &'a mut ForkArena<u32, NodeAnnexLane>,
    words: &'a mut Vec<u32>,
    region: NodeRegionId,
}

pub(super) struct CopyContext<'a> {
    pool: &'a mut ChunkPool<RegionNode>,
    annex_pool: &'a mut ChunkPool<u32>,
    source: &'a ForkArena<RegionNode, PageMaterialLane>,
    source_annex: &'a ForkArena<u32, NodeAnnexLane>,
    annex_reader: NodeAnnexCopyReader<'a>,
    destination: &'a mut ForkArena<RegionNode, PageMaterialLane>,
    destination_annex: &'a mut ForkArena<u32, NodeAnnexLane>,
    destination_region: NodeRegionId,
    stack: Vec<PageListId>,
    annex_envelopes: Vec<Option<u32>>,
    fixed_words: Vec<u32>,
    semantic_identity_enabled: bool,
}

impl FixedBatchPublisher<'_> {
    fn publish(
        &mut self,
        records: &mut [RegionNode],
        pending: &mut SmallVec<[(usize, u16); 16]>,
        batch_start: usize,
        paired_floor: &mut usize,
        reservation: Option<
            &crate::fork_arena::ConstructedRunReservation<'_, RegionNode, PageMaterialLane>,
        >,
        box_envelopes: &[CopiedBoxEnvelope],
    ) -> Result<(), ForkArenaError> {
        if pending.is_empty() {
            return Ok(());
        }
        let mut writer = NodeAnnexWriter::new(self.pool, self.arena);
        let mut offset = batch_start;
        for group in pending.chunks(16) {
            let lengths = group
                .iter()
                .map(|(_, len)| *len)
                .collect::<SmallVec<[u16; 16]>>();
            let end = offset + lengths.iter().map(|&len| usize::from(len)).sum::<usize>();
            let keys = writer.append_fixed_flat(
                &mut self.words[offset..end],
                &lengths,
                |item, annex_position, body| {
                    let index = group[item].0;
                    let Ok(box_index) =
                        box_envelopes.binary_search_by_key(&index, |envelope| envelope.index)
                    else {
                        return Ok(());
                    };
                    let envelope = &box_envelopes[box_index];
                    let node_position = reservation
                        .and_then(|reservation| reservation.position_of(index))
                        .ok_or(ForkArenaError::InvalidRange)?;
                    let annex_body = if envelope.child_annex_start == envelope.child_annex_end {
                        annex_position..annex_position
                    } else {
                        envelope.child_annex_start as usize..envelope.child_annex_end as usize
                    };
                    let stamp = CopiedBoxBodyStamp::new(
                        self.region,
                        envelope.child_node_start as usize..envelope.child_node_end as usize,
                        annex_body,
                        node_position,
                        annex_position,
                    )
                    .ok_or(ForkArenaError::InvalidRange)?;
                    stamp
                        .write_flat_body(body)
                        .ok_or(ForkArenaError::InvalidRange)
                },
            )?;
            if keys.len() != group.len() {
                return Err(ForkArenaError::InvalidRange);
            }
            for ((index, _), key) in group.iter().zip(keys) {
                records[*index] = records[*index]
                    .with_relocated_fixed_key(key)
                    .ok_or(ForkArenaError::InvalidRange)?;
            }
            offset = end;
        }
        *paired_floor = (*paired_floor).min(writer.dependency_floor().unwrap_or(usize::MAX));
        pending.clear();
        self.words.truncate(batch_start);
        Ok(())
    }
}

impl<'a> CopyContext<'a> {
    fn begin_box_body(&mut self) -> Result<u32, ForkArenaError> {
        let node = self.destination.begin_batch(self.pool)?.payload_start();
        self.annex_envelopes.push(None);
        u32::try_from(node).map_err(|_| ForkArenaError::CapacityOverflow)
    }

    fn prepare_annex_publication(&mut self) -> Result<(), ForkArenaError> {
        if self.annex_envelopes.last().is_some_and(Option::is_none) {
            let start = self
                .destination_annex
                .begin_batch(self.annex_pool)?
                .payload_start();
            let start = u32::try_from(start).map_err(|_| ForkArenaError::CapacityOverflow)?;
            for envelope in &mut self.annex_envelopes {
                if envelope.is_none() {
                    *envelope = Some(start);
                }
            }
        }
        Ok(())
    }

    fn end_box_body(&mut self) -> Result<(u32, u32, u32), ForkArenaError> {
        let node_end = u32::try_from(self.destination.payload_position_end())
            .map_err(|_| ForkArenaError::CapacityOverflow)?;
        let annex_start = self
            .annex_envelopes
            .pop()
            .ok_or(ForkArenaError::InvalidRange)?;
        let annex_end = if annex_start.is_some() {
            self.destination_annex
                .begin_batch(self.annex_pool)?
                .payload_start()
        } else {
            self.destination_annex.payload_position_end()
        };
        let annex_end = u32::try_from(annex_end).map_err(|_| ForkArenaError::CapacityOverflow)?;
        Ok((node_end, annex_start.unwrap_or(annex_end), annex_end))
    }

    pub(super) fn new<Source, Destination>(
        pool: &'a mut NodePool,
        source: &'a NodeRegion<Source>,
        destination: &'a mut NodeRegion<Destination>,
        semantic_identity_enabled: bool,
    ) -> Self {
        Self {
            pool: &mut pool.chunks,
            annex_pool: &mut pool.annex_chunks,
            source: &source.pub_arena,
            source_annex: &source.annex_arena,
            annex_reader: NodeAnnexCopyReader::new(&source.annex_arena),
            destination_region: destination.id,
            destination: &mut destination.pub_arena,
            destination_annex: &mut destination.annex_arena,
            stack: Vec::new(),
            annex_envelopes: Vec::new(),
            fixed_words: Vec::new(),
            semantic_identity_enabled,
        }
    }

    pub(super) fn copy_list(
        &mut self,
        list: PageListId,
    ) -> Result<(PageListId, usize), ForkArenaError> {
        if list.is_empty() {
            return Ok((PageListId::empty(), 0));
        }
        if self.stack.contains(&list) {
            return Err(ForkArenaError::InvalidRegion);
        }
        let scratch_mark = self.fixed_words.len();
        self.stack.push(list);
        let result = self.copy_nonempty_list(list);
        self.stack.pop();
        self.fixed_words.truncate(scratch_mark);
        result
    }

    fn copy_nonempty_list(
        &mut self,
        list: PageListId,
    ) -> Result<(PageListId, usize), ForkArenaError> {
        let admitted = self.source.admit_owned_root(self.pool, list.coordinate())?;
        let mut cursors = SmallVec::<[AdmittedListChunkCursor<PageMaterialLane>; 2]>::new();
        let mut cursor =
            self.source
                .admitted_tail_chunk_from_root(self.pool, list.coordinate(), admitted)?;
        while let Some(current) = cursor {
            cursor = self.source.admitted_previous_chunk(self.pool, &current)?;
            cursors.push(current);
        }
        let mut root = crate::fork_arena::ArenaListId::empty();
        let mut count = list.len();
        let mut computed_identity = (self.semantic_identity_enabled
            && list.semantic_identity().is_none())
        .then(SemanticSequenceIdentity::empty);
        let mut records = SmallVec::<[RegionNode; 16]>::new();
        let mut pending = SmallVec::<[(usize, u16); 16]>::new();
        let batch_start = self.fixed_words.len();
        for mut cursor in cursors.into_iter().rev() {
            records.clear();
            debug_assert!(pending.is_empty());
            if let Some((_, source)) = self.source.admitted_remaining_chunk(self.pool, &mut cursor)
            {
                if let Some(packed) = source.packed_slice() {
                    records.extend_from_slice(packed);
                } else {
                    source.for_each(|record| records.push(*record));
                }
            }
            let mut dependency_floor = usize::MAX;
            let mut paired_floor = usize::MAX;
            let mut defer_fixed_publication = false;
            let mut box_envelopes = SmallVec::<[CopiedBoxEnvelope; 4]>::new();
            for index in 0..records.len() {
                let record = &mut records[index];
                if record.is_inline_leaf() {
                    continue;
                }
                if record.has_fixed_copy_payload() {
                    let (body_start, body_len, fields) = record
                        .with_cached_fixed_copy_body(
                            &mut self.annex_reader,
                            self.annex_pool,
                            |body, fields| {
                                let start = self.fixed_words.len();
                                self.fixed_words.push(0);
                                self.fixed_words.extend_from_slice(body);
                                (start + 1, body.len(), fields)
                            },
                        )
                        .ok_or(ForkArenaError::InvalidRange)?;
                    if matches!(
                        record.kind(),
                        Some(crate::node::NodeKind::HList | crate::node::NodeKind::VList)
                    ) {
                        self.fixed_words[body_start + 28..body_start + body_len].fill(0);
                    }
                    let is_box = matches!(
                        record.kind(),
                        Some(crate::node::NodeKind::HList | crate::node::NodeKind::VList)
                    );
                    let mut source_children = [PageListId::empty(); 4];
                    let mut has_nonempty_child = false;
                    for (child_index, &offset) in fields.offsets().iter().enumerate() {
                        let start = body_start + usize::from(offset);
                        let source_child = PageListId::from_words(
                            self.fixed_words[start..start + 10]
                                .try_into()
                                .map_err(|_| ForkArenaError::InvalidRange)?,
                        )
                        .ok_or(ForkArenaError::InvalidRange)?;
                        has_nonempty_child |= !source_child.is_empty();
                        source_children[child_index] = source_child;
                    }
                    let child_start = if is_box && has_nonempty_child {
                        defer_fixed_publication = true;
                        Some(self.begin_box_body()?)
                    } else {
                        None
                    };
                    let mut copied_children = [PageListId::empty(); 4];
                    for (child_index, &source_child) in source_children
                        .iter()
                        .take(fields.offsets().len())
                        .enumerate()
                    {
                        let (copied, child_count) = if source_child.is_empty() {
                            (PageListId::empty(), 0)
                        } else {
                            self.copy_list(source_child)?
                        };
                        copied_children[child_index] = copied;
                        count = count.saturating_add(child_count);
                    }
                    if let Some(child_node_start) = child_start {
                        let (child_node_end, child_annex_start, child_annex_end) =
                            self.end_box_body()?;
                        box_envelopes.push(CopiedBoxEnvelope {
                            index,
                            child_node_start,
                            child_node_end,
                            child_annex_start,
                            child_annex_end,
                        });
                    }
                    if has_nonempty_child {
                        let (child_floor, child_annex_floor) = self
                            .destination
                            .dependency_floors_for_region_lists(self.pool, |visit| {
                                for (child_index, &offset) in fields.offsets().iter().enumerate() {
                                    let copied = copied_children[child_index];
                                    visit(copied.coordinate());
                                    let start = body_start + usize::from(offset);
                                    self.fixed_words[start..start + 10]
                                        .copy_from_slice(&copied.words());
                                }
                                Some(())
                            })?;
                        dependency_floor = dependency_floor.min(child_floor.unwrap_or(usize::MAX));
                        paired_floor = paired_floor.min(child_annex_floor.unwrap_or(usize::MAX));
                    }
                    pending.push((index, (body_len + 1) as u16));
                    if !defer_fixed_publication && pending.len() == 16 {
                        self.prepare_annex_publication()?;
                        FixedBatchPublisher {
                            pool: self.annex_pool,
                            arena: self.destination_annex,
                            words: &mut self.fixed_words,
                            region: self.destination_region,
                        }
                        .publish(
                            &mut records,
                            &mut pending,
                            batch_start,
                            &mut paired_floor,
                            None,
                            &box_envelopes,
                        )?;
                    }
                    continue;
                }
                let mut children = SmallVec::<[PageListId; 4]>::new();
                record
                    .visit_node_lists(
                        NodeAnnexView::new(self.annex_pool, self.source_annex),
                        |child| children.push(child),
                    )
                    .ok_or(ForkArenaError::InvalidRange)?;
                for child in &mut children {
                    let (copied, child_count) = self.copy_list(*child)?;
                    *child = copied;
                    count = count.saturating_add(child_count);
                }
                let copied_children = children;
                let mut children = copied_children.iter().copied();
                let mut reencoded = None;
                self.prepare_annex_publication()?;
                let (child_floor, child_annex_floor) = self
                    .destination
                    .dependency_floors_for_region_lists(self.pool, |visit| {
                        reencoded = record.reencode_between_regions(
                            self.annex_pool,
                            &mut self.annex_reader,
                            self.destination_annex,
                            |_| {
                                let child = children.next()?;
                                visit(child.coordinate());
                                Some(child)
                            },
                        );
                        reencoded.is_some().then_some(())
                    })?;
                if children.next().is_some() {
                    return Err(ForkArenaError::InvalidRegion);
                }
                let (relocated, annex_floor) = reencoded.ok_or(ForkArenaError::InvalidRange)?;
                *record = relocated;
                paired_floor = paired_floor.min(annex_floor.unwrap_or(usize::MAX));
                dependency_floor = dependency_floor.min(child_floor.unwrap_or(usize::MAX));
                paired_floor = paired_floor.min(child_annex_floor.unwrap_or(usize::MAX));
            }
            if !pending.is_empty() {
                self.prepare_annex_publication()?;
            }
            let reservation = self.destination.reserve_constructed_list_run(
                self.pool,
                &mut root,
                records.len(),
            )?;
            FixedBatchPublisher {
                pool: self.annex_pool,
                arena: self.destination_annex,
                words: &mut self.fixed_words,
                region: self.destination_region,
            }
            .publish(
                &mut records,
                &mut pending,
                batch_start,
                &mut paired_floor,
                Some(&reservation),
                &box_envelopes,
            )?;
            if let Some(identity) = &mut computed_identity {
                let annex = NodeAnnexView::new(self.annex_pool, self.destination_annex);
                for record in &records {
                    identity.push_back(record.semantic_identity(annex));
                }
            }
            reservation.publish(
                &records,
                (dependency_floor != usize::MAX).then_some(dependency_floor),
                (paired_floor != usize::MAX).then_some(paired_floor),
            )?;
        }
        self.destination.finish_constructed_list(self.pool, root)?;
        let identity = if self.semantic_identity_enabled {
            list.semantic_identity()
                .map(|hash| SemanticSequenceIdentity::from_raw(hash, list.len()))
                .or(computed_identity)
        } else {
            None
        };
        Ok((PageListId::from_parts(root, identity), count))
    }
}

/// Synthetic shapes for the explicit, opt-in node-copy timing tier.
#[cfg(any(feature = "profiling", feature = "testing"))]
#[derive(Clone, Copy, Debug)]
pub enum ExplicitCopyShape {
    Inline,
    FixedAnnex,
    Nested,
    VariableSpan,
}

/// Source and destination owners for the opt-in explicit-copy timing tier.
/// Construction and rollback are separate from the measured copy method.
#[cfg(any(feature = "profiling", feature = "testing"))]
pub struct ExplicitCopyHarness {
    pool: NodePool,
    source: NodeRegion<PageRole>,
    root: RegionRoot<PageRole>,
    destination: NodeRegion<DurableRole>,
    node_mark: crate::fork_arena::OperationMark<PageMaterialLane>,
    annex_mark: crate::fork_arena::OperationMark<NodeAnnexLane>,
    nodes: usize,
    copied_nodes: usize,
}

#[cfg(any(feature = "profiling", feature = "testing"))]
impl ExplicitCopyHarness {
    pub fn new(shape: ExplicitCopyShape, nodes: usize) -> Self {
        use crate::glue::Order;
        use crate::node::{BoxLr, BoxNode, BoxNodeFields, Sign, Whatsit};
        use crate::scaled::{GlueSetRatio, Scaled};

        assert!(nodes > 0);
        let mut pool = NodePool::new();
        let mut source = pool.start_region::<PageRole>().expect("source region");
        let child = if matches!(shape, ExplicitCopyShape::Nested) {
            source
                .publish_owned(&mut pool, [Node::Penalty(7)])
                .expect("shared source child")
                .list
        } else {
            PageListId::empty()
        };
        let make_box = || {
            Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children: child,
            }))
        };
        let root = source
            .publish_owned(
                &mut pool,
                (0..nodes).map(|index| match shape {
                    ExplicitCopyShape::Inline => Node::Penalty(index as i32),
                    ExplicitCopyShape::FixedAnnex | ExplicitCopyShape::Nested => make_box(),
                    ExplicitCopyShape::VariableSpan => Node::Whatsit(Whatsit::Special {
                        class: "copy-profile".into(),
                        payload: vec![index as u8; 128],
                    }),
                }),
            )
            .expect("source root");
        let destination = pool
            .start_region::<DurableRole>()
            .expect("destination region");
        let node_mark = destination.pub_arena.operation_mark(&pool.chunks);
        let annex_mark = destination.annex_arena.operation_mark(&pool.annex_chunks);
        let copied_nodes = nodes
            * if matches!(shape, ExplicitCopyShape::Nested) {
                2
            } else {
                1
            };
        Self {
            pool,
            source,
            root,
            destination,
            node_mark,
            annex_mark,
            nodes,
            copied_nodes,
        }
    }

    /// Runs one exact copy; the caller must subsequently restore this harness.
    pub fn copy_once(&mut self) -> usize {
        let before = self.destination.pub_arena.counters().source_nodes_copied;
        let copied = copy_region_root_into(
            &mut self.pool,
            &self.source,
            self.root,
            &mut self.destination,
            false,
        )
        .expect("profile copy");
        assert_eq!(copied.list.len(), self.nodes);
        assert_eq!(
            self.destination.pub_arena.counters().source_nodes_copied - before,
            self.copied_nodes as u64
        );
        self.copied_nodes
    }

    pub fn restore(&mut self) {
        self.destination
            .pub_arena
            .restore_operation(&mut self.pool.chunks, self.node_mark)
            .expect("restore measured node suffix");
        self.destination
            .annex_arena
            .restore_operation(&mut self.pool.annex_chunks, self.annex_mark)
            .expect("restore measured annex suffix");
    }
}
