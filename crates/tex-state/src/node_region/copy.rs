//! Chunk-batched explicit copies between independently owned node regions.
//!
//! Recursion keeps all staging in context-owned stacks. Each copied list
//! opens a [`Frame`] of marks into those stacks and truncates back to them on
//! return, so a Rust call frame holds only scalars however deeply boxes nest.

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

/// One completed recursive copy with the destination dependency floors of
/// its root: the head chunk position and the tail chunk's paired annex
/// floor. `usize::MAX` means none.
#[derive(Clone, Copy)]
pub(super) struct CopiedList {
    pub(super) list: PageListId,
    pub(super) count: usize,
    node_floor: usize,
    paired_floor: usize,
}

impl CopiedList {
    const EMPTY: Self = Self {
        list: PageListId::empty(),
        count: 0,
        node_floor: usize::MAX,
        paired_floor: usize::MAX,
    };
}

/// Stack marks owned by one list copy. Records, pending fixed bodies, box
/// envelopes, and flat annex words above these marks belong to that copy.
#[derive(Clone, Copy)]
struct Frame {
    records: usize,
    pending: usize,
    envelopes: usize,
    words: usize,
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
    records: Vec<RegionNode>,
    /// Frame-relative record index and flat length of each fixed body not
    /// yet published.
    pending: Vec<(usize, u16)>,
    box_envelopes: Vec<CopiedBoxEnvelope>,
    children: Vec<PageListId>,
    semantic_identity_enabled: bool,
    /// Every node publication by this context leaves the destination tail
    /// sealed, so only the first box envelope needs an explicit boundary.
    node_tail_sealed: bool,
    /// Frozen slots of box bodies borrowed instead of copied.
    shared: Vec<u32>,
}

impl FixedBatchPublisher<'_> {
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &mut self,
        records: &mut [RegionNode],
        pending: &mut Vec<(usize, u16)>,
        pending_start: usize,
        batch_start: usize,
        paired_floor: &mut usize,
        node_position_of: impl Fn(usize) -> Option<usize>,
        box_envelopes: &[CopiedBoxEnvelope],
    ) -> Result<(), ForkArenaError> {
        if pending.len() == pending_start {
            return Ok(());
        }
        let mut writer = NodeAnnexWriter::new(self.pool, self.arena);
        let mut offset = batch_start;
        for group in pending[pending_start..].chunks(16) {
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
                    let node_position =
                        node_position_of(index).ok_or(ForkArenaError::InvalidRange)?;
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
        pending.truncate(pending_start);
        self.words.truncate(batch_start);
        Ok(())
    }
}

impl<'a> CopyContext<'a> {
    fn begin_box_body(&mut self) -> Result<u32, ForkArenaError> {
        let node = if self.node_tail_sealed {
            self.destination.next_payload_position()
        } else {
            self.node_tail_sealed = true;
            self.destination.begin_batch(self.pool)?.payload_start()
        };
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
        chunks: &'a mut ChunkPool<RegionNode>,
        annex_chunks: &'a mut ChunkPool<u32>,
        source: &'a NodeRegion<Source>,
        destination: &'a mut NodeRegion<Destination>,
        semantic_identity_enabled: bool,
    ) -> Self {
        Self {
            pool: chunks,
            annex_pool: annex_chunks,
            source: &source.pub_arena,
            source_annex: &source.annex_arena,
            annex_reader: NodeAnnexCopyReader::new(&source.annex_arena),
            destination_region: destination.id,
            destination: &mut destination.pub_arena,
            destination_annex: &mut destination.annex_arena,
            stack: Vec::new(),
            annex_envelopes: Vec::new(),
            fixed_words: Vec::new(),
            records: Vec::new(),
            pending: Vec::new(),
            box_envelopes: Vec::new(),
            children: Vec::new(),
            semantic_identity_enabled,
            node_tail_sealed: false,
            shared: Vec::new(),
        }
    }

    /// Frozen registry slots whose box bodies this copy borrowed. The caller
    /// logs them in the destination once the copy is published.
    pub(super) fn into_shared(self) -> Vec<u32> {
        self.shared
    }

    pub(super) fn copy_list(&mut self, list: PageListId) -> Result<CopiedList, ForkArenaError> {
        if list.is_empty() {
            return Ok(CopiedList::EMPTY);
        }
        if self.stack.contains(&list) {
            return Err(ForkArenaError::InvalidRegion);
        }
        let frame = Frame {
            records: self.records.len(),
            pending: self.pending.len(),
            envelopes: self.box_envelopes.len(),
            words: self.fixed_words.len(),
        };
        self.stack.push(list);
        let result = self.copy_nonempty_list(list, frame);
        self.stack.pop();
        self.records.truncate(frame.records);
        self.pending.truncate(frame.pending);
        self.box_envelopes.truncate(frame.envelopes);
        self.fixed_words.truncate(frame.words);
        result
    }

    fn copy_nonempty_list(
        &mut self,
        list: PageListId,
        frame: Frame,
    ) -> Result<CopiedList, ForkArenaError> {
        if self
            .source
            .read_single_chunk_list(self.pool, list.coordinate(), &mut self.records)?
        {
            return self.copy_short_list(list, frame);
        }
        if list.len() > self.destination.chunk_capacity(self.pool) {
            return self.copy_long_list(list, frame);
        }
        // A short list fragmented across source chunks still fits one exact
        // destination chunk. Stage each chunk reversed while following the
        // predecessor edges, then restore forward order once.
        let admitted = self.source.admit_owned_root(self.pool, list.coordinate())?;
        let mut cursor =
            self.source
                .admitted_tail_chunk_from_root(self.pool, list.coordinate(), admitted)?;
        while let Some(mut current) = cursor {
            cursor = self.source.admitted_previous_chunk(self.pool, &current)?;
            if let Some((_, source)) = self
                .source
                .admitted_remaining_chunk(self.pool, &mut current)
            {
                if let Some(packed) = source.packed_slice() {
                    self.records.extend(packed.iter().rev().copied());
                } else {
                    let start = self.records.len();
                    source.for_each(|record| self.records.push(*record));
                    self.records[start..].reverse();
                }
            }
        }
        if self.records.len() - frame.records != list.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        self.records[frame.records..].reverse();
        self.copy_short_list(list, frame)
    }

    /// Copies a list longer than one chunk, staging one source chunk at a
    /// time so scratch stays bounded by the chunk size.
    #[inline(never)]
    fn copy_long_list(
        &mut self,
        list: PageListId,
        frame: Frame,
    ) -> Result<CopiedList, ForkArenaError> {
        let admitted = self.source.admit_owned_root(self.pool, list.coordinate())?;
        let mut cursor =
            self.source
                .admitted_tail_chunk_from_root(self.pool, list.coordinate(), admitted)?;
        let mut cursors = Vec::new();
        while let Some(current) = cursor {
            cursor = self.source.admitted_previous_chunk(self.pool, &current)?;
            cursors.push(current);
        }
        let mut root = crate::fork_arena::ArenaListId::empty();
        let mut count = list.len();
        let mut computed_identity = (self.semantic_identity_enabled
            && list.semantic_identity().is_none())
        .then(SemanticSequenceIdentity::empty);
        for mut cursor in cursors.into_iter().rev() {
            self.records.truncate(frame.records);
            debug_assert_eq!(self.pending.len(), frame.pending);
            self.box_envelopes.truncate(frame.envelopes);
            if let Some((_, source)) = self.source.admitted_remaining_chunk(self.pool, &mut cursor)
            {
                if let Some(packed) = source.packed_slice() {
                    self.records.extend_from_slice(packed);
                } else {
                    source.for_each(|record| self.records.push(*record));
                }
            }
            let floors = self.relocate_records(frame, &mut count)?;
            if self.pending.len() != frame.pending {
                self.prepare_annex_publication()?;
            }
            let reservation = self.destination.reserve_constructed_list_run(
                self.pool,
                &mut root,
                self.records.len() - frame.records,
            )?;
            let mut paired_floor = floors.paired;
            FixedBatchPublisher {
                pool: self.annex_pool,
                arena: self.destination_annex,
                words: &mut self.fixed_words,
                region: self.destination_region,
            }
            .publish(
                &mut self.records[frame.records..],
                &mut self.pending,
                frame.pending,
                frame.words,
                &mut paired_floor,
                |index| reservation.position_of(index),
                &self.box_envelopes[frame.envelopes..],
            )?;
            if let Some(identity) = &mut computed_identity {
                let annex = NodeAnnexView::new(self.annex_pool, self.destination_annex);
                for record in &self.records[frame.records..] {
                    identity.push_back(record.semantic_identity(annex));
                }
            }
            reservation.publish(
                &self.records[frame.records..],
                (floors.node != usize::MAX).then_some(floors.node),
                (paired_floor != usize::MAX).then_some(paired_floor),
            )?;
        }
        self.destination.finish_constructed_list(self.pool, root)?;
        self.node_tail_sealed = true;
        let (node_floor, paired_floor) =
            self.destination
                .dependency_floors_for_region_lists(self.pool, |visit| {
                    visit(root);
                    Some(())
                })?;
        let identity = if self.semantic_identity_enabled {
            list.semantic_identity()
                .map(|hash| SemanticSequenceIdentity::from_raw(hash, list.len()))
                .or(computed_identity)
        } else {
            None
        };
        Ok(CopiedList {
            list: PageListId::from_parts(root, identity),
            count,
            node_floor: node_floor.unwrap_or(usize::MAX),
            paired_floor: paired_floor.unwrap_or(usize::MAX),
        })
    }

    /// Copies a list of at most one chunk, staged in this frame's records,
    /// into one fresh, already sealed destination chunk.
    fn copy_short_list(
        &mut self,
        list: PageListId,
        frame: Frame,
    ) -> Result<CopiedList, ForkArenaError> {
        let mut count = list.len();
        let floors = self.relocate_records(frame, &mut count)?;
        let mut paired_floor = floors.paired;
        if self.pending.len() != frame.pending {
            self.prepare_annex_publication()?;
            let node_position = self.destination.next_payload_position();
            FixedBatchPublisher {
                pool: self.annex_pool,
                arena: self.destination_annex,
                words: &mut self.fixed_words,
                region: self.destination_region,
            }
            .publish(
                &mut self.records[frame.records..],
                &mut self.pending,
                frame.pending,
                frame.words,
                &mut paired_floor,
                |_| Some(node_position),
                &self.box_envelopes[frame.envelopes..],
            )?;
        }
        let node_floor = self.destination.next_payload_position();
        let root = self.destination.publish_sealed_chunk_list(
            self.pool,
            &self.records[frame.records..],
            (floors.node != usize::MAX).then_some(floors.node),
            (paired_floor != usize::MAX).then_some(paired_floor),
        )?;
        self.node_tail_sealed = true;
        let identity = self
            .semantic_identity_enabled
            .then(|| self.short_list_identity(list, frame));
        Ok(CopiedList {
            list: PageListId::from_parts(root, identity),
            count,
            node_floor,
            paired_floor,
        })
    }

    fn short_list_identity(&self, list: PageListId, frame: Frame) -> SemanticSequenceIdentity {
        list.semantic_identity()
            .map(|hash| SemanticSequenceIdentity::from_raw(hash, list.len()))
            .unwrap_or_else(|| {
                let annex = NodeAnnexView::new(self.annex_pool, self.destination_annex);
                let mut identity = SemanticSequenceIdentity::empty();
                for record in &self.records[frame.records..] {
                    identity.push_back(record.semantic_identity(annex));
                }
                identity
            })
    }

    /// Copies every child closure named by the frame's staged records and
    /// relocates them in place. Fixed bodies stay pending for the caller's
    /// publication once node positions are known.
    fn relocate_records(
        &mut self,
        frame: Frame,
        count: &mut usize,
    ) -> Result<RelocatedFloors, ForkArenaError> {
        let mut floors = RelocatedFloors {
            node: usize::MAX,
            paired: usize::MAX,
        };
        let mut defer_fixed_publication = false;
        for index in 0..self.records.len() - frame.records {
            let record = self.records[frame.records + index];
            if record.is_inline_leaf() {
                continue;
            }
            if record.has_fixed_copy_payload() {
                self.relocate_fixed_record(
                    frame,
                    index,
                    record,
                    count,
                    &mut floors,
                    &mut defer_fixed_publication,
                )?;
            } else {
                self.relocate_variable_record(frame.records + index, record, count, &mut floors)?;
            }
        }
        Ok(floors)
    }

    fn relocate_fixed_record(
        &mut self,
        frame: Frame,
        index: usize,
        record: RegionNode,
        count: &mut usize,
        floors: &mut RelocatedFloors,
        defer_fixed_publication: &mut bool,
    ) -> Result<(), ForkArenaError> {
        let (body_start, body_len, fields) = record
            .with_cached_fixed_copy_body(&mut self.annex_reader, self.annex_pool, |body, fields| {
                let start = self.fixed_words.len();
                self.fixed_words.push(0);
                self.fixed_words.extend_from_slice(body);
                (start + 1, body.len(), fields)
            })
            .ok_or(ForkArenaError::InvalidRange)?;
        let is_box = matches!(
            record.kind(),
            Some(crate::node::NodeKind::HList | crate::node::NodeKind::VList)
        );
        if is_box {
            self.fixed_words[body_start + 28..body_start + body_len].fill(0);
        }
        let mut has_nonempty_child = false;
        for &offset in fields.offsets() {
            has_nonempty_child |= !self
                .fixed_child(body_start + usize::from(offset))?
                .is_empty();
        }
        if is_box && has_nonempty_child && self.share_box_body(body_start)? {
            // The borrowed body keeps its coordinate, and the copied wrapper
            // carries no body stamp: no interval move may claim the body.
            has_nonempty_child = false;
        }
        let child_start = if is_box && has_nonempty_child {
            *defer_fixed_publication = true;
            Some(self.begin_box_body()?)
        } else {
            None
        };
        if has_nonempty_child {
            for &offset in fields.offsets() {
                let start = body_start + usize::from(offset);
                let copied = self.copy_list(self.fixed_child(start)?)?;
                *count = count.saturating_add(copied.count);
                floors.node = floors.node.min(copied.node_floor);
                floors.paired = floors.paired.min(copied.paired_floor);
                self.fixed_words[start..start + 10].copy_from_slice(&copied.list.words());
            }
        }
        if let Some(child_node_start) = child_start {
            let (child_node_end, child_annex_start, child_annex_end) = self.end_box_body()?;
            self.box_envelopes.push(CopiedBoxEnvelope {
                index,
                child_node_start,
                child_node_end,
                child_annex_start,
                child_annex_end,
            });
        }
        self.pending.push((index, (body_len + 1) as u16));
        if !*defer_fixed_publication && self.pending.len() - frame.pending == 16 {
            self.prepare_annex_publication()?;
            FixedBatchPublisher {
                pool: self.annex_pool,
                arena: self.destination_annex,
                words: &mut self.fixed_words,
                region: self.destination_region,
            }
            .publish(
                &mut self.records[frame.records..],
                &mut self.pending,
                frame.pending,
                frame.words,
                &mut floors.paired,
                |_| None,
                &self.box_envelopes[frame.envelopes..],
            )?;
        }
        Ok(())
    }

    /// Borrows a box body owned by a frozen region instead of copying it.
    /// Only box bodies are shared, so every other child list kind stays
    /// exclusively owned by the region that names it. A body is shared only
    /// whole: every nonempty child list of the box must be frozen.
    fn share_box_body(&mut self, body_start: usize) -> Result<bool, ForkArenaError> {
        let mut slots = [None; 2];
        for (slot, offset) in slots
            .iter_mut()
            .zip([BOX_CHILDREN_OFFSET, BOX_DIAGNOSTIC_CHILDREN_OFFSET])
        {
            let list = self.fixed_child(body_start + offset)?;
            if list.is_empty() {
                continue;
            }
            let Some(frozen) = self.pool.frozen_slot_of_list(list.coordinate()) else {
                return Ok(false);
            };
            *slot = Some(frozen);
        }
        for slot in slots.into_iter().flatten() {
            if !self.shared.contains(&slot) {
                self.shared.push(slot);
            }
        }
        Ok(true)
    }

    fn fixed_child(&self, start: usize) -> Result<PageListId, ForkArenaError> {
        PageListId::from_words(
            self.fixed_words[start..start + 10]
                .try_into()
                .map_err(|_| ForkArenaError::InvalidRange)?,
        )
        .ok_or(ForkArenaError::InvalidRange)
    }

    fn relocate_variable_record(
        &mut self,
        slot: usize,
        record: RegionNode,
        count: &mut usize,
        floors: &mut RelocatedFloors,
    ) -> Result<(), ForkArenaError> {
        let children = self.children.len();
        record
            .visit_node_lists(
                NodeAnnexView::new(self.annex_pool, self.source_annex),
                |child| self.children.push(child),
            )
            .ok_or(ForkArenaError::InvalidRange)?;
        for child in children..self.children.len() {
            let copied = self.copy_list(self.children[child])?;
            *count = count.saturating_add(copied.count);
            floors.node = floors.node.min(copied.node_floor);
            floors.paired = floors.paired.min(copied.paired_floor);
            self.children[child] = copied.list;
        }
        self.prepare_annex_publication()?;
        let mut copied = self.children[children..].iter().copied();
        let relocated = record.reencode_between_regions(
            self.annex_pool,
            &mut self.annex_reader,
            self.destination_annex,
            |_| copied.next(),
        );
        let exhausted = copied.next().is_none();
        self.children.truncate(children);
        let (relocated, annex_floor) = relocated.ok_or(ForkArenaError::InvalidRange)?;
        if !exhausted {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.records[slot] = relocated;
        floors.paired = floors.paired.min(annex_floor.unwrap_or(usize::MAX));
        Ok(())
    }
}

/// Fixed-body word offsets of an H/V box's primary and diagnostic child
/// lists (see `encode_box_payload`).
const BOX_CHILDREN_OFFSET: usize = 7;
const BOX_DIAGNOSTIC_CHILDREN_OFFSET: usize = 17;

/// Dependency floors accumulated while relocating one staged chunk;
/// `usize::MAX` means no dependency.
struct RelocatedFloors {
    node: usize,
    paired: usize,
}

#[cfg(any(feature = "profiling", feature = "testing"))]
mod harness;
#[cfg(any(feature = "profiling", feature = "testing"))]
pub use harness::{ExplicitCopyHarness, ExplicitCopyShape};
